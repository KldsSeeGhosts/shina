use super::*;

impl State {
    pub(crate) fn machine_identity(&self, machine: &str) -> String {
        if machine == "local" {
            self.machines
                .views
                .get("local")
                .and_then(|v| v["serverId"].as_str())
                .map(str::to_owned)
                .or_else(|| {
                    std::fs::read_to_string(self.data_dir.join("server-id"))
                        .ok()
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_else(|| "local".into())
        } else {
            machine.to_owned()
        }
    }

    pub(crate) fn linked_environment_ids_for_target(&self) -> std::collections::HashSet<String> {
        let mut set: std::collections::HashSet<String> =
            self.peers.iter().map(|p| p.server_id.clone()).collect();
        if let Some(client) = &self.client {
            for id in client.linked.keys() {
                set.insert(id.clone());
            }
        }
        set.insert(self.machine_identity(&self.machines.selected));
        set.insert(self.machines.selected.clone());
        set
    }

    pub fn has_pending_context_imports(&self, machine: &str, thread_id: usize) -> bool {
        self.pending_context_imports
            .iter()
            .any(|(m, tid, _)| m == machine && *tid == thread_id)
    }

    pub fn copy_composer_selection(&mut self, cut: bool) -> Option<rune::ClipboardContent> {
        let t = self.current();
        if t.draft.anchor == t.draft.focus || t.draft.is_secure() {
            return None;
        }
        let (start, end) = t.draft.range();
        let selected_markdown = crate::composer_rich::markdown_range(&t.draft, start..end);
        let selected_html = if crate::composer_source::has_shown_source(&t.draft) {
            let (mut collapsed, map) = crate::composer_source::collapsed_copy(&t.draft);
            let (cs, ce) = if let Some(map) = map {
                let s = map.map(start, rune::MapBias::After);
                let e = map.map(end, rune::MapBias::Before).max(s);
                (s, e)
            } else {
                (start, end)
            };
            if cs < ce {
                collapsed.set_selection(cs, ce);
                collapsed.selected_html()
            } else {
                t.draft.selected_html()
            }
        } else {
            t.draft.selected_html()
        };
        let refs = crate::composer_context::collect_composer_context_references(&selected_markdown);
        let mut wanted: std::collections::HashSet<String> =
            refs.into_iter().map(|r| r.context_id).collect();
        for rec in &t.draft_context {
            if wanted.contains(&rec.context_id)
                && let crate::composer_context::ComposerContextData::PreviewAnnotation {
                    screenshot_context_id: Some(shot_id),
                    ..
                } = &rec.data
            {
                wanted.insert(shot_id.clone());
            }
        }
        let mut records = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut copied_files = std::collections::HashMap::new();
        for rec in &t.draft_context {
            if wanted.contains(&rec.context_id) && seen.insert(rec.context_id.clone()) {
                if let crate::composer_context::ComposerContextData::Image { attachment_id, .. }
                | crate::composer_context::ComposerContextData::File { attachment_id, .. } =
                    &rec.data
                    && let Some(att) = t.draft_images.iter().find(|a| a.id == *attachment_id)
                {
                    copied_files.insert(attachment_id.clone(), att.clone());
                }
                records.push(rec.clone());
            }
        }
        for att in &t.draft_images {
            let kind = if att.is_image() { "image" } else { "file" };
            let cid = crate::composer_context::to_kind_scoped_composer_context_id(kind, &att.id);
            if wanted.contains(&cid) && seen.insert(cid) {
                copied_files.insert(att.id.clone(), att.clone());
                records.push(crate::composer_context::attachment_context_record(att));
            }
        }
        let staged = if self.current().answer_intake {
            (|| -> Result<(), String> {
                let files = std::mem::take(&mut copied_files);
                for (id, file) in files {
                    let mut single = std::collections::HashMap::from([(id, file)]);
                    attachments::stage_clipboard_files(&mut records, &mut single)?;
                    copied_files.extend(single);
                }
                Ok(())
            })()
        } else {
            attachments::stage_clipboard_files(&mut records, &mut copied_files)
        };
        if let Err(error) = staged {
            self.current_mut().attachment_error = Some(error);
            return None;
        }
        let t = self.current();
        let source_env = self.machine_identity(&self.machines.selected);
        let content = if !records.is_empty() {
            let fragment = crate::composer_context::ComposerContextClipboardFragment {
                version: 1,
                source: crate::composer_context::ClipboardSource {
                    environment_id: source_env.clone(),
                    thread_id: Some(t.id.to_string()),
                    message_id: None,
                },
                records,
            };
            if let Some(encoded) =
                crate::composer_context::encode_composer_context_fragment(&fragment)
            {
                crate::composer_context::remember_copied_composer_attachments(
                    &source_env,
                    copied_files,
                );
                let html = crate::composer_context::encode_composer_context_clipboard_html(
                    &selected_markdown,
                    &encoded,
                    (!selected_html.is_empty()).then_some(selected_html.as_str()),
                );
                rune::ClipboardContent::new(selected_markdown).with_html(html)
            } else if !selected_html.is_empty() {
                rune::ClipboardContent::new(selected_markdown).with_html(selected_html)
            } else {
                rune::ClipboardContent::new(selected_markdown)
            }
        } else if !selected_html.is_empty() {
            rune::ClipboardContent::new(selected_markdown).with_html(selected_html)
        } else {
            rune::ClipboardContent::new(selected_markdown)
        };
        if cut {
            let t = self.current_mut();
            t.draft.undo_group(|e| {
                e.set_selection(start, end);
                e.insert("");
            });
            t.reconcile_draft_context();
            t.typing_guard.on_type(std::time::Instant::now());
            t.update_submission_error();
            self.ensure_composer_commands();
            self.checkpoint();
        }
        Some(content)
    }

    pub fn paste_composer_content(
        &mut self,
        content: &rune::ClipboardContent,
        bypass_fold: bool,
    ) -> bool {
        if !self.current().answer_intake
            && let Some(scope) = self.current().question_scope()
        {
            if !self.question_attachments_allowed(self.active) {
                self.current_mut().attachment_error =
                    Some("This question cannot accept attachments.".into());
                return true;
            }
            return self
                .with_question_draft(self.active, scope, |s| {
                    s.paste_composer_content(content, bypass_fold)
                })
                .unwrap_or(true);
        }
        let has_image = content.image().is_some();
        if let Some(image) = content.image() {
            self.attach_image_with_options("Pasted image.png", image.png(), bypass_fold);
        }
        let claim_files = content.files().iter().any(|path| {
            path.is_dir()
                || crate::attachment_intake::claims_paste(
                    &path.to_string_lossy(),
                    crate::attachment_intake::native_mime(&path.to_string_lossy()),
                    content.text(),
                )
        });
        if claim_files {
            self.attach_paths(content.files());
        }
        if !content.text().is_empty() && !has_image && !claim_files {
            if self.current().answer_intake
                && self
                    .current()
                    .native
                    .questionnaire
                    .as_ref()
                    .is_some_and(|f| f.questions[f.index].initial_answer.is_some())
            {
                self.current_mut().draft.insert(content.text());
                return true;
            }
            if !bypass_fold && self.fold_paste(content.text()) {
                return true;
            }
            if self.current().draft.active_block() == rune::Block::Code {
                crate::composer_paste::insert(
                    self,
                    &content.text().replace("\r\n", "\n").replace('\r', "\n"),
                );
                return true;
            }
            let target_env = self.machine_identity(&self.machines.selected);
            let linked_envs = self.linked_environment_ids_for_target();
            let (rewritten_text, outcome) = crate::composer_context::import_pasted_composer_text(
                content.text(),
                content.html(),
                &target_env,
                &linked_envs,
                &self.current().draft_context,
                &self.current().draft_images,
            );
            if self.current().answer_intake
                && outcome
                    .imported_records
                    .iter()
                    .any(|r| !matches!(r.kind(), "thread" | "file" | "image"))
            {
                self.current_mut().attachment_error =
                    Some("Answer or dismiss the question first, then add it to chat.".into());
                return true;
            }
            for rec in outcome.imported_records {
                let t = self.current_mut();
                if let Some(existing) = t
                    .draft_context
                    .iter_mut()
                    .find(|r| r.context_id == rec.context_id)
                {
                    *existing = rec;
                } else {
                    t.draft_context.push(rec);
                }
            }
            crate::composer_source::collapse_shown_markdown_source(&mut self.current_mut().draft);
            let t = self.current_mut();
            t.composer_source.on_paste(&t.draft);
            crate::composer_paste::insert(self, &rewritten_text);
            let target_machine = self.machines.selected.clone();
            let target_thread_id = self.current().id;
            for pending in outcome.pending_attachments {
                self.resolve_pending_attachment_import(&target_machine, target_thread_id, pending);
            }
            return true;
        }
        (has_image || !content.files().is_empty()) && content.text().is_empty()
    }

    pub(super) fn resolve_pending_attachment_import(
        &mut self,
        target_machine: &str,
        target_thread_id: usize,
        pending: crate::composer_context::PendingAttachmentImport,
    ) {
        if self.current().answer_intake
            && self.current().question_reserved_count() >= attachments::MAX_ATTACHMENTS
        {
            self.current_mut().attachment_error = Some(format!(
                "You can attach up to {} files per message.",
                attachments::MAX_ATTACHMENTS
            ));
            return;
        }
        let (attachment_id, name, mime_type, kind) = match &pending.record.data {
            crate::composer_context::ComposerContextData::Image {
                attachment_id,
                name,
                mime_type,
                ..
            } => (
                attachment_id.clone(),
                name.clone(),
                mime_type.clone(),
                "image",
            ),
            crate::composer_context::ComposerContextData::File {
                attachment_id,
                name,
                mime_type,
                ..
            } => (
                attachment_id.clone(),
                name.clone(),
                mime_type.clone(),
                "file",
            ),
            _ => return,
        };
        let fail_msg = |reason: &str| {
            format!(
                "Couldn't bring {name} into this message. {reason} Remove the chip or attach the file again."
            )
        };
        let reserved = self
            .pending_context_imports
            .iter()
            .filter(|(machine, id, _)| machine == target_machine && *id == target_thread_id)
            .count();
        let attached = if target_machine == self.machines.selected {
            self.threads.iter().find(|t| t.id == target_thread_id)
        } else {
            self.machines
                .threads
                .get(target_machine)
                .and_then(|threads| threads.iter().find(|t| t.id == target_thread_id))
        }
        .map_or(0, |thread| thread.draft_images.len());
        if attached + reserved >= attachments::MAX_ATTACHMENTS {
            self.current_mut().attachment_error =
                Some(fail_msg("The attachment limit has been reached."));
            return;
        }
        let local_bytes: Option<Vec<u8>> = if let Some(copied) =
            crate::composer_context::copied_composer_attachment(
                &pending.source_environment_id,
                &attachment_id,
            ) {
            if copied.is_image() {
                attachments::read(&copied).ok()
            } else {
                attachments::transfer_file_bytes(&copied).ok()
            }
        } else if (pending.source_environment_id == self.machine_identity("local")
            || pending.source_environment_id == "local")
            && (kind == "image" && attachments::safe_id(&attachment_id)
                || kind == "file" && attachments::safe_file_id(&attachment_id))
        {
            attachments::directory()
                .map(|d| d.join(&attachment_id))
                .and_then(|path| {
                    use std::io::Read;
                    let maximum = if kind == "file" {
                        attachments::MAX_FILE_BYTES
                    } else {
                        attachments::MAX_IMAGE_BYTES
                    };
                    let mut bytes = Vec::new();
                    std::fs::File::open(path)
                        .ok()?
                        .take(maximum + 1)
                        .read_to_end(&mut bytes)
                        .ok()?;
                    (bytes.len() as u64 <= maximum).then_some(bytes)
                })
        } else {
            None
        };
        if let Some(bytes) = local_bytes {
            self.apply_imported_attachment_bytes(
                target_machine,
                target_thread_id,
                &pending.new_context_id,
                &name,
                &mime_type,
                kind,
                Ok(bytes),
            );
            return;
        }
        if pending.source_environment_id == self.machine_identity("local")
            || pending.source_environment_id == "local"
        {
            self.current_mut().attachment_error =
                Some(fail_msg("The original attachment is no longer available."));
            return;
        }
        let link = self
            .client
            .as_ref()
            .and_then(|c| c.linked.get(&pending.source_environment_id))
            .and_then(|c| c.remote.clone());
        let Some(link) = link else {
            self.current_mut().attachment_error =
                Some(fail_msg("The environment it came from is not connected."));
            return;
        };
        if !self.machine_connected(&pending.source_environment_id) {
            self.current_mut().attachment_error = Some(fail_msg(
                "The environment it came from is not reachable right now.",
            ));
            return;
        }
        if kind == "file"
            && !self
                .client
                .as_ref()
                .and_then(|c| c.linked.get(&pending.source_environment_id))
                .is_some_and(|c| {
                    c.info.lock().expect("connection info")["capabilities"]
                        .as_array()
                        .is_some_and(|caps| {
                            caps.iter()
                                .any(|cap| cap == crate::protocol::FILE_ATTACHMENTS)
                        })
                })
        {
            self.current_mut().attachment_error =
                Some(fail_msg(crate::protocol::FILE_ATTACHMENTS_UPDATE_REQUIRED));
            return;
        }
        if !(kind == "image" && attachments::safe_id(&attachment_id)
            || kind == "file" && attachments::safe_file_id(&attachment_id))
        {
            self.current_mut().attachment_error =
                Some(fail_msg("The original attachment is no longer available."));
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = self.wake.clone();
        let target_machine = target_machine.to_owned();
        let new_context_id = pending.new_context_id.clone();
        let question_scope = self
            .threads
            .iter()
            .find(|t| t.id == target_thread_id)
            .and_then(|t| t.answer_scope.clone());
        if let Some(scope) = &question_scope
            && let Some(t) = self.threads.iter_mut().find(|t| t.id == target_thread_id)
        {
            t.question_drafts.entry(scope.clone()).or_default().pending += 1;
        }
        self.pending_context_imports.push((
            target_machine.clone(),
            target_thread_id,
            new_context_id.clone(),
        ));
        self.context_import_rxs.push(rx);
        std::thread::spawn(move || {
            let maximum = if kind == "file" {
                attachments::MAX_FILE_BYTES
            } else {
                attachments::MAX_IMAGE_BYTES
            };
            let result = fetch_linked_blob(&link, &attachment_id, maximum);
            let _ = tx.send(ContextAssetDownload {
                machine: target_machine,
                thread_id: target_thread_id,
                new_context_id,
                name,
                mime_type,
                kind,
                result,
                question_scope,
            });
            wake();
        });
    }

    pub fn poll_context_imports(&mut self) -> bool {
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.context_import_rxs.len());
        let rxs = std::mem::take(&mut self.context_import_rxs);
        for rx in rxs {
            match rx.try_recv() {
                Ok(download) => {
                    changed = true;
                    self.pending_context_imports.retain(|(m, tid, cid)| {
                        !(m == &download.machine
                            && *tid == download.thread_id
                            && cid == &download.new_context_id)
                    });
                    if let Some(scope) = download.question_scope {
                        if download.machine == self.machines.selected
                            && let Some(index) =
                                self.threads.iter().position(|t| t.id == download.thread_id)
                        {
                            if let Some(draft) = self.threads[index].question_drafts.get_mut(&scope)
                            {
                                draft.pending = draft.pending.saturating_sub(1);
                            }
                            self.with_question_draft(index, scope, |s| {
                                s.apply_imported_attachment_bytes(
                                    &download.machine,
                                    download.thread_id,
                                    &download.new_context_id,
                                    &download.name,
                                    &download.mime_type,
                                    download.kind,
                                    download.result,
                                )
                            });
                        } else if let Some(t) = self
                            .machines
                            .threads
                            .get_mut(&download.machine)
                            .and_then(|ts| ts.iter_mut().find(|t| t.id == download.thread_id))
                            && let Some(draft) = t.question_drafts.get_mut(&scope)
                        {
                            draft.pending = draft.pending.saturating_sub(1);
                        }
                    } else {
                        self.apply_imported_attachment_bytes(
                            &download.machine,
                            download.thread_id,
                            &download.new_context_id,
                            &download.name,
                            &download.mime_type,
                            download.kind,
                            download.result,
                        );
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => changed = true,
            }
        }
        self.context_import_rxs = remaining;
        changed
    }

    pub(super) fn apply_imported_attachment_bytes(
        &mut self,
        target_machine: &str,
        target_thread_id: usize,
        new_context_id: &str,
        name: &str,
        mime_type: &str,
        kind: &str,
        result: Result<Vec<u8>, String>,
    ) {
        let pending: std::collections::HashSet<_> = self
            .pending_context_imports
            .iter()
            .filter(|(machine, thread, _)| machine == target_machine && *thread == target_thread_id)
            .map(|(_, _, context)| context.clone())
            .collect();
        let threads = if target_machine == self.machines.selected {
            Some(&mut self.threads)
        } else {
            self.machines.threads.get_mut(target_machine)
        };
        let Some(t) =
            threads.and_then(|threads| threads.iter_mut().find(|t| t.id == target_thread_id))
        else {
            return;
        };
        if !crate::composer_tokens::collect_inline_context_ids_from_edit(&t.draft)
            .contains(new_context_id)
        {
            return;
        }
        // A successful import resolves only its own chip. Keep an earlier
        // error while a different attachment chip remains unresolved and no
        // download is pending for it. Completion order must not hide failure.
        let previous_error = t.attachment_error.clone().filter(|_| {
            t.draft.pills().any(|(_, href)| {
                crate::composer_context::parse_composer_context_href(href).is_some_and(
                    |(kind, context)| {
                        matches!(kind.as_str(), "image" | "file")
                            && context != new_context_id
                            && !pending.contains(&context)
                            && !t
                                .draft_context
                                .iter()
                                .any(|record| record.context_id == context)
                    },
                )
            })
        });
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(reason) => {
                t.attachment_error = Some(format!(
                    "Couldn't bring {name} into this message. {reason} Remove the chip or attach the file again."
                ));
                return;
            }
        };
        if (if t.answer_intake {
            t.question_reserved_count()
        } else {
            t.draft_images.len()
        }) >= attachments::MAX_ATTACHMENTS
        {
            t.attachment_error = Some(format!(
                "Couldn't bring {name} into this message. The attachment limit has been reached. Remove the chip or attach the file again."
            ));
            return;
        }
        let attachment = if kind == "image" {
            match attachments::attach_bytes(name, bytes) {
                Ok(att) => att,
                Err(error) => {
                    t.attachment_error = Some(error);
                    return;
                }
            }
        } else {
            let dir = match attachments::directory() {
                Some(d) => d,
                None => {
                    t.attachment_error = Some("The attachment directory is unavailable.".into());
                    return;
                }
            };
            match attachments::store_uploaded_file(&dir, name, mime_type, &bytes) {
                Ok(att) => att,
                Err(error) => {
                    t.attachment_error = Some(error);
                    return;
                }
            }
        };
        if !t.draft_images.iter().any(|a| a.id == attachment.id) {
            if let Err(error) = t.add_attachment(attachment.clone()) {
                t.attachment_error = Some(format!(
                    "Couldn't bring {name} into this message. {error} Remove the chip or attach the file again."
                ));
                return;
            }
        }
        let mut rec = crate::composer_context::attachment_context_record(&attachment);
        rec.context_id = new_context_id.to_owned();
        if let Some(existing) = t
            .draft_context
            .iter_mut()
            .find(|r| r.context_id == rec.context_id)
        {
            *existing = rec;
        } else {
            t.draft_context.push(rec);
        }
        // Also clear a resolved error when the imported bytes deduplicate to
        // an existing attachment and add_attachment did not run.
        t.attachment_error = previous_error;
    }
    pub(super) fn attachment_guard(&self, index: usize) -> Option<String> {
        if self.demo {
            Some("Open a real workspace before attaching files.".into())
        } else if self.threads[index].archived() {
            Some("Restore this thread before attaching files.".into())
        } else if !self.threads[index].answer_intake
            && !self.threads[index].request_held_by_typing_guard()
            && self.threads[index]
                .request
                .as_ref()
                .is_some_and(|r| r["method"] != "confirm")
        {
            Some("This question cannot accept attachments.".into())
        } else if self.threads[index].status.busy() {
            if self.threads[index].native.operating() {
                Some("Wait for the current operation before attaching files.".into())
            } else {
                None
            }
        } else if self.threads[index].configuring {
            Some("Wait for the model change to finish before attaching files.".into())
        } else {
            None
        }
    }
    /// Attach one or more picked or dropped files. A failure leaves the text draft untouched.
    pub fn attach_paths(&mut self, paths: &[PathBuf]) {
        self.attach_paths_to(self.current().id, paths);
    }
    pub fn fold_paste(&mut self, text: &str) -> bool {
        let exceeds = crate::composer_paste::would_exceed_limit(&self.current().draft, text);
        if text.is_empty() || !crate::composer_paste::should_fold(text) && !exceeds {
            return false;
        }
        let can_stage = !(self.current().machine != "local" && self.client.is_none())
            && self.pasted_text_staging_supported()
            && self.file_attachment_staging_supported_on(&self.machines.selected)
            && self.attachment_guard(self.active).is_none()
            && (if self.current().answer_intake {
                self.current().question_reserved_count()
            } else {
                self.current().draft_images.len()
            }) + if self.current().answer_intake {
                0
            } else {
                self.attachment_uploads
                    .preparing_count(&self.machines.selected, self.current().id)
            } < attachments::MAX_ATTACHMENTS;
        let maximum = self
            .file_attachment_staging_limit_on(&self.machines.selected)
            .unwrap_or(attachments::MAX_FILE_BYTES);
        if !can_stage || text.len() as u64 > maximum {
            if !exceeds {
                return false;
            }
            let (title, description) = if can_stage {
                (
                    "Pasted text is too large to attach",
                    "Reduce the clipboard contents or save a smaller excerpt as a file.",
                )
            } else {
                (
                    "Pasted text is too large for this message",
                    "Remove some text or an attachment, then paste again.",
                )
            };
            self.current_mut().attachment_error = Some(format!("{title}\n{description}"));
            self.push_toast(
                ToastKind::Error,
                title,
                Some(description.into()),
                None,
                Some(6000),
            );
            return true;
        }
        let target = (self.machines.selected.clone(), self.current().id);
        if !self
            .pasted_text_names
            .as_ref()
            .is_some_and(|(machine, thread, _)| (machine, *thread) == (&target.0, target.1))
        {
            self.pasted_text_names = Some((target.0, target.1, Default::default()));
        }
        let names: Vec<_> = self
            .current()
            .draft_images
            .iter()
            .map(|a| a.name.clone())
            .collect();
        let reserved = &mut self.pasted_text_names.as_mut().unwrap().2;
        reserved.extend(names);
        let name = crate::composer_paste::next_name(reserved.iter().map(String::as_str));
        reserved.insert(name.clone());
        match crate::composer_paste::stage_named(text, &name) {
            Ok(attachment) => {
                if let Some(notice) = &mut self.undo {
                    notice.owns_text_key = false;
                }
                let t = self.current_mut();
                t.draft_images.push(attachment.clone());
                t.insert_attachment_chips(&[attachment], false);
                self.push_toast(
                    ToastKind::Info,
                    format!("Large paste attached as {name}"),
                    Some(format!(
                        "{} · Use {} to keep a large paste inline.",
                        if text.len().div_ceil(1024) >= 1024 {
                            format!("{:.1} MB", text.len() as f64 / 1048576.0)
                        } else {
                            format!("{} KB", text.len().div_ceil(1024).max(1))
                        },
                        if cfg!(target_os = "macos") {
                            "⌘⇧V"
                        } else {
                            "Ctrl+Shift+V"
                        }
                    )),
                    None,
                    Some(6000),
                );
                self.checkpoint();
                true
            }
            Err(error) => {
                self.pasted_text_names.as_mut().unwrap().2.remove(&name);
                self.current_mut().attachment_error = Some(error);
                exceeds
            }
        }
    }
    pub fn attach_paths_to(&mut self, thread: usize, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let Some(index) = self.threads.iter().position(|t| t.id == thread) else {
            return;
        };
        if !self.threads[index].answer_intake
            && let Some(scope) = self.threads[index].question_scope()
        {
            if !self.question_attachments_allowed(index) {
                self.threads[index].attachment_error =
                    Some("This question cannot accept attachments.".into());
                return;
            }
            self.with_question_draft(index, scope, |s| s.attach_paths_to(thread, paths));
            return;
        }
        if let Some(error) = self.attachment_guard(index) {
            self.threads[index].attachment_error = Some(error);
            return;
        }
        self.threads[index].attachment_error = None;
        let uploads_before = self.attachment_uploads.entries.len();
        let mut errors = Vec::new();
        let mut attached = Vec::new();
        let mut inserted_folder = false;
        for path in paths {
            if path.is_dir() {
                let local = self.machines.selected == "local"
                    || self
                        .client
                        .as_ref()
                        .and_then(|c| c.linked.get(&self.machines.selected))
                        .and_then(|c| c.remote.as_ref())
                        .is_some_and(|link| {
                            url::Url::parse(&format!("http://{}", link.host))
                                .ok()
                                .and_then(|u| u.host_str().map(str::to_owned))
                                .is_some_and(|host| {
                                    host == "localhost"
                                        || host
                                            .parse::<std::net::IpAddr>()
                                            .is_ok_and(|ip| ip.is_loopback())
                                })
                        });
                if !local {
                    errors.push(
                        "Folders can only be referenced on a server running on this computer"
                            .into(),
                    );
                    continue;
                }
                match path.canonicalize() {
                    Ok(path) => {
                        let label = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        let token = crate::composer_tokens::Token::File {
                            path,
                            directory: true,
                            source: None,
                        };
                        let draft = &mut self.threads[index].draft;
                        let end = draft.text.len();
                        let boundary = draft
                            .text
                            .chars()
                            .next_back()
                            .is_some_and(|c| !c.is_whitespace());
                        draft.undo_group(|e| {
                            e.set_selection(end, end);
                            if boundary {
                                e.insert(" ");
                            }
                            let start = e.focus;
                            e.insert(&label);
                            e.replace_with_pill(start..e.focus, token.id(), &label);
                            e.insert(" ");
                        });
                        inserted_folder = true;
                    }
                    Err(_) => errors.push(format!(
                        "Couldn't get the path of \"{}\"\nType the folder path with @ instead.",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )),
                }
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let file_kind = crate::attachment_intake::classify(
                &name,
                crate::attachment_intake::native_mime(&name),
            );
            if file_kind == crate::attachment_intake::Kind::File
                && let Some(maximum) =
                    self.file_attachment_staging_limit_on(&self.machines.selected)
                && std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > maximum)
            {
                errors.push(crate::protocol::file_attachment_too_large(&name, maximum));
                continue;
            }
            if crate::attachment_intake::classify(
                &name,
                crate::attachment_intake::native_mime(&name),
            ) == crate::attachment_intake::Kind::File
                && !self.file_attachment_staging_supported_on(&self.machines.selected)
            {
                errors.push("This server does not support file attachments.".into());
                continue;
            }
            let replacement = self.threads[index].draft_images.iter().position(|a| {
                a.matches_reattach(
                    &name,
                    &crate::attachment_intake::file_mime(
                        &name,
                        crate::attachment_intake::native_mime(&name),
                    ),
                    std::fs::metadata(path).map_or(0, |m| m.len()),
                )
            });
            if replacement.is_none()
                && (if self.threads[index].answer_intake {
                    self.threads[index].question_reserved_count()
                } else {
                    self.threads[index].draft_images.len()
                }) >= attachments::MAX_ATTACHMENTS
            {
                errors.push(format!(
                    "You can attach up to {} files per message.",
                    attachments::MAX_ATTACHMENTS
                ));
                break;
            }
            // Question drafts retain main's request-scoped, send-time upload path.
            // The ordinary tray queue owns thread drafts, not question answer scopes.
            if self.background.handle.is_some() && !self.threads[index].answer_intake {
                self.enqueue_attachment_path(index, path.clone());
                continue;
            }
            match attachments::attach_file(path) {
                Ok(mut image) => {
                    if let Some(slot) = replacement {
                        image = match attachments::stage_transfer_file(&image) {
                            Ok(image) => image,
                            Err(error) => {
                                errors.push(error);
                                continue;
                            }
                        };
                        image.asset_id = Some(image.id.clone());
                        image.id = self.threads[index].draft_images[slot].id.clone();
                        self.threads[index].draft_images[slot] = image;
                    } else {
                        match self.threads[index].add_attachment(image.clone()) {
                            Ok(()) => attached.push(image),
                            Err(error) => errors.push(error),
                        }
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        if !errors.is_empty() {
            self.threads[index].attachment_error = errors.pop();
        }
        if !attached.is_empty() {
            self.threads[index].insert_attachment_chips(&attached, false);
        }
        if index == self.active
            && (!attached.is_empty()
                || inserted_folder
                || self.attachment_uploads.entries.len() > uploads_before)
        {
            self.restore_composer_focus(crate::ui::composer_ui::FocusReason::AttachmentInserted);
        }
        self.checkpoint();
    }
    /// Attach a pasted PNG image, inserting an inline chip when contextual.
    #[cfg(test)]
    pub fn attach_image(&mut self, name: &str, bytes: &[u8]) {
        self.attach_image_with_options(name, bytes, false);
    }
    pub fn attach_image_with_options(&mut self, name: &str, bytes: &[u8], skip_inline_chip: bool) {
        if !self.current().answer_intake
            && let Some(scope) = self.current().question_scope()
        {
            if !self.question_attachments_allowed(self.active) {
                self.current_mut().attachment_error =
                    Some("This question cannot accept attachments.".into());
                return;
            }
            self.with_question_draft(self.active, scope, |s| {
                s.attach_image_with_options(name, bytes, skip_inline_chip)
            });
            return;
        }
        if let Some(error) = self.attachment_guard(self.active) {
            self.current_mut().attachment_error = Some(error);
            return;
        }
        if self.background.handle.is_some() && !self.current().answer_intake {
            self.current_mut().attachment_error = None;
            self.enqueue_attachment_image(
                self.active,
                name.into(),
                bytes.to_vec(),
                skip_inline_chip,
            );
            self.checkpoint();
            return;
        }
        let prepared = crate::attachment_intake::prepare(
            name,
            crate::attachment_intake::native_mime(name),
            bytes.to_vec(),
            attachments::MAX_IMAGE_BYTES as usize,
        )
        .map_err(|failure| failure.message(name))
        .and_then(|(name, bytes)| attachments::attach_bytes(&name, bytes));
        match prepared {
            Ok(image) => {
                let result = self.current_mut().add_attachment(image.clone());
                self.current_mut().attachment_error = result.as_ref().err().cloned();
                if result.is_ok() {
                    self.current_mut()
                        .insert_attachment_chips(&[image], skip_inline_chip);
                    self.restore_composer_focus(
                        crate::ui::composer_ui::FocusReason::AttachmentInserted,
                    );
                }
            }
            Err(error) => self.current_mut().attachment_error = Some(error),
        }
        self.checkpoint();
    }
    pub fn request_remove_attachment(&mut self, id: &str) {
        if !self.current().answer_intake
            && let Some(scope) = self.current().question_scope()
        {
            self.answer_remove_scope = Some(scope.clone());
            self.with_question_draft(self.active, scope, |s| s.request_remove_attachment(id));
            return;
        }
        if !self.current().answer_intake
            && self.attachment_uploads.entries.iter().any(|entry| {
                entry.id == id && matches!(entry.phase, crate::attachment_uploads::Phase::Preparing)
            })
        {
            self.remove_attachment(id);
            return;
        }
        let t = self.current();
        if let Some(attachment) = t.draft_images.iter().find(|a| a.id == id && a.is_image()) {
            let ctx_id = crate::composer_context::to_kind_scoped_composer_context_id(
                "image",
                &attachment.id,
            );
            let referenced = crate::composer_tokens::collect_inline_context_ids_from_edit(&t.draft)
                .contains(&ctx_id);
            if referenced {
                self.confirm_remove_attachment = Some((t.id, id.to_owned()));
                return;
            }
        }
        self.remove_attachment(id);
    }
    pub fn confirm_remove_attachment_dialog(&mut self) {
        if let Some(scope) = self.answer_remove_scope.clone()
            && !self.current().answer_intake
        {
            if self.current().question_scope().as_ref() != Some(&scope) {
                self.confirm_remove_attachment = None;
                self.answer_remove_scope = None;
                return;
            }
            self.with_question_draft(self.active, scope, |s| s.confirm_remove_attachment_dialog());
            self.answer_remove_scope = None;
            return;
        }
        let Some((thread_id, id)) = self.confirm_remove_attachment.take() else {
            return;
        };
        let Some(t) = self.threads.iter_mut().find(|t| t.id == thread_id) else {
            return;
        };
        let ctx_id = crate::composer_context::to_kind_scoped_composer_context_id("image", &id);
        let pill_href = crate::composer_context::format_composer_context_href("image", &ctx_id);
        let ranges: Vec<_> = t
            .draft
            .pills()
            .filter(|(_, pill)| *pill == pill_href)
            .map(|(range, _)| range)
            .collect();
        if !ranges.is_empty() {
            t.draft.undo_group(|e| {
                for range in ranges.into_iter().rev() {
                    let mut start = range.start;
                    let mut end = range.end;
                    if e.text[end..].starts_with(' ') {
                        end += 1;
                    } else if e.text[..start].ends_with(' ') {
                        start -= 1;
                    }
                    e.set_selection(start, end);
                    e.insert("");
                    if start >= e.text.len() {
                        let trimmed_len = e.text.trim_end().len();
                        if trimmed_len < e.text.len() {
                            e.set_selection(trimmed_len, e.text.len());
                            e.insert("");
                        }
                    }
                }
            });
        }
        t.draft_context.retain(|r| r.context_id != ctx_id);
        t.retained_context.records.remove(&ctx_id);
        t.remove_attachment(&id);
        if !t.answer_intake {
            self.attachment_uploads
                .remove(&self.machines.selected, thread_id, &id);
        }
        self.checkpoint();
    }
    pub fn remove_attachment(&mut self, id: &str) {
        if !self.current().answer_intake
            && let Some(scope) = self.current().question_scope()
        {
            self.with_question_draft(self.active, scope, |s| s.remove_attachment(id));
            return;
        }
        let thread = self.current().id;
        if !self.current().answer_intake {
            self.attachment_uploads
                .remove(&self.machines.selected, thread, id);
        }
        self.current_mut().remove_attachment(id);
        self.checkpoint();
    }
    pub(crate) fn push_toast(
        &mut self,
        kind: ToastKind,
        title: impl Into<String>,
        description: Option<String>,
        action: Option<(String, ToastAction)>,
        timeout_ms: Option<u64>,
    ) {
        let title = title.into();
        if title == "Not connected: message not sent" {
            self.toasts.retain(|t| t.title != title);
        }
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        let (action_label, action) = match action {
            Some((label, action)) => (Some(label), Some(action)),
            None => (None, None),
        };
        let expires_at = timeout_ms.map(|ms| lifecycle::now().saturating_add(ms));
        self.toasts.push(ToastNotice {
            id,
            kind,
            title,
            description,
            action_label,
            action,
            expires_at,
        });
        while self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    pub fn activate_toast(&mut self, id: u64) {
        let Some(pos) = self.toasts.iter().position(|t| t.id == id) else {
            return;
        };
        let toast = self.toasts.remove(pos);
        let Some(action) = toast.action else {
            return;
        };
        match action {
            ToastAction::ReviewCaptainPromotion {
                server,
                thread_id,
                profile,
                restore,
            } => {
                if self.machines.selected != server {
                    self.select_machine(&server);
                }
                if self.machines.selected != server {
                    return;
                }
                match self.begin_captain_promotion(&server, thread_id) {
                    Ok(()) => {
                        let current_restore = self
                            .captain_editor
                            .as_ref()
                            .and_then(|editor| editor.promotion.as_ref())
                            .and_then(|promotion| promotion.restore);
                        if current_restore != restore {
                            let candidate = self
                                .captain_editor
                                .as_ref()
                                .and_then(|editor| editor.promotion.as_ref())
                                .and_then(|promotion| promotion.restore_candidate);
                            if restore.is_none() || candidate == restore {
                                if let Err(error) =
                                    self.set_captain_promotion_restore(restore.is_some())
                                    && let Some(editor) = self.captain_editor.as_mut()
                                {
                                    editor.error = Some(error);
                                }
                            } else if let Some(editor) = self.captain_editor.as_mut() {
                                editor.error = Some("The former captain changed. Review the profile before confirming.".into());
                            }
                        }
                        if restore.is_none()
                            && let Some(editor) = self.captain_editor.as_mut()
                            && editor
                                .promotion
                                .as_ref()
                                .is_some_and(|promotion| promotion.restore.is_none())
                        {
                            editor.name.replace_text(profile.name);
                            editor.role.replace_text(profile.role);
                            editor.instructions.replace_text(profile.instructions);
                            editor.portrait = profile.portrait;
                        }
                    }
                    Err(error) => self.push_toast(
                        ToastKind::Error,
                        "Could not reopen promotion",
                        Some(error),
                        None,
                        Some(6000),
                    ),
                }
            }
            ToastAction::OpenThread { server, thread_id } => {
                if self.client.is_some() {
                    if server != self.machines.selected {
                        self.select_machine(&server);
                    }
                    if self.machines.selected == server
                        && let Some(index) = self.threads.iter().position(|t| t.id == thread_id)
                    {
                        self.show_thread(index);
                    }
                } else if let Some(index) = self.threads.iter().position(|t| t.id == thread_id) {
                    self.show_thread(index);
                }
            }
        }
    }

    pub(crate) fn expire_toasts(&mut self) -> bool {
        let at = lifecycle::now();
        let before = self.toasts.len();
        self.toasts
            .retain(|toast| toast.expires_at.is_none_or(|exp| exp > at));
        self.toasts.len() != before
    }

    pub(crate) fn thread_workspace_path(&self, thread_id: usize) -> Option<String> {
        let thread = self.threads.iter().find(|t| t.id == thread_id)?;
        thread.project?;
        if self.client.is_some() && thread.machine != "local" {
            return self.client_views["directories"][thread.id.to_string()]
                .as_str()
                .map(str::to_owned);
        }
        self.project(thread.project)
            .filter(|project| project.machine == thread.machine)
            .map(|project| project.path.to_string_lossy().into_owned())
    }

    pub(crate) fn can_write_project_file(&self, thread: &Thread) -> bool {
        !self.demo
            && self.thread_workspace_path(thread.id).is_some()
            && self.client.as_ref().is_none_or(|client| {
                client.supports_capability(&thread.machine, crate::protocol::PROJECT_WRITE_FILE)
            })
    }

    pub(crate) fn is_plan_copied(&self, card_id: &str) -> bool {
        self.plan_copied
            .as_ref()
            .is_some_and(|(id, expires)| id == card_id && *expires > lifecycle::now())
    }

    pub(crate) fn mark_plan_copied(&mut self, card_id: String, tx: &mut Tx) {
        self.plan_copied = Some((card_id, lifecycle::now().saturating_add(2_000)));
        tx.wake_after(Duration::from_secs(2));
    }

    pub(crate) fn toggle_plan_expanded(&mut self, card_id: &str) {
        if !self.plan_expanded.remove(card_id) {
            self.plan_expanded.insert(card_id.to_owned());
        }
    }

    pub(crate) fn open_plan_save_dialog(&mut self, thread_id: usize, plan_markdown: &str) {
        let Some(thread) = self.threads.iter().find(|t| t.id == thread_id) else {
            return;
        };
        if !self.can_write_project_file(thread) {
            return;
        }
        let Some(cwd) = self.thread_workspace_path(thread_id) else {
            return;
        };
        let machine = thread.machine.clone();
        let initial_path = self.plan_save_last_path.clone().unwrap_or_else(|| {
            crate::plan_export::build_proposed_plan_markdown_filename(plan_markdown)
        });
        let mut path_edit = TextEdit::default();
        path_edit.replace_text(&initial_path);
        path_edit.focused = true;
        self.plan_save_dialog = Some(PlanSaveDialog {
            machine,
            cwd,
            contents: crate::plan_export::normalize_plan_markdown_for_export(plan_markdown),
            path_edit,
            saving: false,
            request_id: None,
        });
    }

    pub(crate) fn close_plan_save_dialog(&mut self) {
        if !self.plan_save_dialog.as_ref().is_some_and(|d| d.saving) {
            self.plan_save_dialog = None;
        }
    }

    pub(crate) fn submit_plan_save(&mut self) {
        let Some(dialog) = self.plan_save_dialog.as_mut() else {
            return;
        };
        if dialog.saving {
            return;
        }
        let raw_path = dialog.path_edit.text.clone();
        let relative_path = raw_path.trim().to_owned();
        if relative_path.is_empty() {
            self.push_toast(
                ToastKind::Warning,
                "Enter a workspace path",
                None,
                None,
                Some(5_000),
            );
            return;
        }
        self.plan_save_last_path = Some(raw_path);
        self.next_plan_save_request_id += 1;
        let req_id = self.next_plan_save_request_id;
        dialog.saving = true;
        dialog.request_id = Some(req_id);
        let machine = dialog.machine.clone();
        let cwd = dialog.cwd.clone();
        let contents = dialog.contents.clone();
        if let Some(client) = &self.client {
            let target = if machine == "local" {
                Some(client)
            } else {
                client.linked.get(&machine)
            };
            let Some(target) = target else {
                self.apply_plan_save_result(req_id, Err("Machine unavailable.".into()));
                return;
            };
            if !target.has_capability(crate::protocol::PROJECT_WRITE_FILE) {
                self.apply_plan_save_result(
                    req_id,
                    Err("This server needs an update to save plans to the workspace.".into()),
                );
                return;
            }
            target.call_rpc(
                format!("write-plan:{req_id}"),
                "project.writeFile",
                json!({
                    "cwd": cwd,
                    "relativePath": relative_path,
                    "contents": contents,
                }),
            );
        } else if machine != "local" {
            self.apply_plan_save_result(req_id, Err("Machine unavailable.".into()));
        } else {
            let result =
                crate::projects::write_project_file(&crate::protocol::ProjectWriteFileInput {
                    cwd,
                    relative_path,
                    contents,
                });
            self.apply_plan_save_result(req_id, result);
        }
    }

    pub(crate) fn apply_plan_save_result(
        &mut self,
        req_id: u64,
        result: Result<crate::protocol::ProjectWriteFileResult, String>,
    ) {
        let Some(dialog) = self.plan_save_dialog.as_mut() else {
            return;
        };
        if dialog.request_id != Some(req_id) {
            return;
        }
        match result {
            Ok(written) => {
                self.plan_save_dialog = None;
                self.push_toast(
                    ToastKind::Success,
                    "Plan saved to workspace",
                    Some(written.relative_path),
                    None,
                    Some(5_000),
                );
            }
            Err(error) => {
                dialog.saving = false;
                dialog.request_id = None;
                self.push_toast(
                    ToastKind::Error,
                    "Could not save plan",
                    Some(error),
                    None,
                    Some(5_000),
                );
            }
        }
    }

    pub(crate) fn notify_background_started(&mut self, server: &str, thread_id: usize) {
        self.push_toast(
            ToastKind::Success,
            "Started in background",
            None,
            Some((
                "Open".into(),
                ToastAction::OpenThread {
                    server: server.to_owned(),
                    thread_id,
                },
            )),
            Some(5_000),
        );
    }

    pub(crate) fn notify_background_failure(
        &mut self,
        server: &str,
        thread_id: usize,
        error: String,
    ) {
        self.toasts.retain(|toast| {
            !(toast.title == "Started in background"
                && toast.action.as_ref().is_some_and(|action| {
                    matches!(
                        action,
                        ToastAction::OpenThread {
                            server: s,
                            thread_id: tid
                        } if s == server && *tid == thread_id
                    )
                }))
        });
        self.push_toast(
            ToastKind::Error,
            "Background task failed",
            Some(error),
            Some((
                "Open draft".into(),
                ToastAction::OpenThread {
                    server: server.to_owned(),
                    thread_id,
                },
            )),
            Some(5_000),
        );
    }
}
