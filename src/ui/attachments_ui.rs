use super::*;

pub(super) fn image_preview(
    s: &State,
    p: Palette,
    image: &Attachment,
    width: f32,
    height: f32,
    key: String,
) -> El {
    if !image.is_image()
        && (image.mime.starts_with("video/")
            || crate::attachment_intake::native_mime(&image.name).starts_with("video/"))
        && let Some(path) = crate::attachments::path_for(image).filter(|p| p.is_file())
    {
        // KSG ComposerVideoThumbnail and prepareVideoFirstFrame. Keep a paused,
        // muted native poster alive between frames; evict only least-recently used posters.
        let clock = s.attachment_poster_clock.get().wrapping_add(1);
        s.attachment_poster_clock.set(clock);
        let mut posters = s.attachment_video_posters.borrow_mut();
        let (player, accessed) = posters.entry(image.id.clone()).or_insert_with(|| {
            (
                rune::Player::file(attachment_preview::player_path(&path)),
                clock,
            )
        });
        *accessed = clock;
        let player = player.clone();
        if player.time() == 0.0
            && let Some(duration) = player.duration().filter(|d| *d > 0.0)
        {
            player.seek(0.1_f64.min(duration / 2.0));
        }
        if posters.len() > 256
            && let Some(oldest) = posters
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone())
        {
            posters.remove(&oldest);
        }
        return el()
            .key(key)
            .size(width, height)
            .radius(6.0)
            .clip_rounded(6.0)
            .video(&player, rune::SurfaceFit::Fill)
            .child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .center()
                    .child(icon("play", Color::WHITE)),
            );
    }
    let mut preview = el()
        .key(key)
        .size(width, height)
        .radius(6.0)
        .clip_rounded(6.0)
        .border(0.5, p.line);
    if let Some(path) = crate::attachments::preview_path_for(image) {
        let handle = s.attachment_cache.file(path, width.max(height), 2.0);
        // Headless fixtures have no window waker to rebuild after asynchronous decode.
        if rune::is_headless() {
            let _ = handle.wait();
        }
        // Cover and crop, as `object-cover` does, so square tiles never stretch a screenshot.
        preview = preview.pixels_fit(handle.pixels(), rune::SurfaceFit::Fill);
    } else {
        preview = preview
            .center()
            .fill(p.selected)
            .child(if image.is_image() {
                icon("image", p.muted)
            } else {
                label(&image.name, TextStyle::new(10.0, 400.0), p.muted)
                    .wrap_anywhere()
                    .align(rune::Align::Center)
                    .padding(4.0)
            });
    }
    preview
}

pub(super) fn image_size(image: &Attachment, maximum: f32) -> (f32, f32) {
    let ratio = image.width.max(1) as f32 / image.height.max(1) as f32;
    if ratio >= 1.0 {
        (maximum, (maximum / ratio).max(40.0))
    } else {
        ((maximum * ratio).max(40.0), maximum)
    }
}

/// A draft image as a bare 64 px thumbnail with a close button in its corner, ported from
/// KSG Code's `ComposerAttachmentStrip`. The file name lives in the tooltip and the label.
pub(super) const ATTACHMENT_THUMB: f32 = 64.0;
pub(super) fn composer_attachment_thumbnail(
    s: &State,
    p: Palette,
    image: &Attachment,
    width: f32,
    height: f32,
    key: String,
) -> El {
    let inherited = s
        .attachment_uploads
        .entries
        .iter()
        .find(|entry| {
            entry.machine == s.machines.selected
                && entry.thread == s.current().id
                && entry.id == image.id
        })
        .and_then(|entry| entry.thumbnail(&s.attachment_cache));
    if let Some(thumbnail) = inherited {
        if rune::is_headless() {
            let _ = thumbnail.wait();
        }
        if thumbnail.is_ready() && thumbnail.error().is_none() {
            return el()
                .key(key)
                .size(width, height)
                .pixels_fit(thumbnail.pixels(), rune::SurfaceFit::Fill);
        }
    }
    image_preview(s, p, image, width, height, key)
}

pub(super) fn attachment_chip(s: &State, p: Palette, image: &Attachment) -> El {
    let id = image.id.clone();
    let preview_id = id.clone();
    let mut tile = el()
        .key(format!("attachment-{}", image.id))
        .role("group")
        .label(format!(
            "Attached {}: {}",
            if image.is_image() { "image" } else { "file" },
            image.name
        ))
        .help(image.name.clone())
        .size(ATTACHMENT_THUMB, ATTACHMENT_THUMB)
        .child(
            composer_attachment_thumbnail(
                s,
                p,
                image,
                ATTACHMENT_THUMB,
                ATTACHMENT_THUMB,
                format!("attachment-thumb-{id}"),
            )
            .radius(6.0)
            .clip_rounded(6.0)
            .role("button")
            .label(format!("Preview {}", image.name))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| {
                s.preview_attachments(s.current().draft_images.clone(), &preview_id, true);
            }),
        )
        .child(
            el().key(format!("remove-attachment-{id}"))
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!("Remove {}", image.name))
                .focusable(true)
                .pin(None, Some(4.0), Some(4.0), None)
                .size(18.0, 18.0)
                .center()
                .radius(6.0)
                .fill(Color::BLACK.alpha(0.6))
                .hover(Color::BLACK.alpha(0.8))
                .on_press(move |s: &mut State| s.request_remove_attachment(&id))
                .child(icon("close", Color::WHITE).size(12.0, 12.0)),
        );
    if let Some(status) = attachment_upload_status(s, p, &image.id) {
        tile = tile.child(status);
    }
    tile
}

// Ported from KSG Code's ComposerAttachmentMediaRow and ComposerAttachmentUploadProgress.
pub(super) fn attachment_upload_status(s: &State, p: Palette, id: &str) -> Option<El> {
    use crate::attachment_uploads::Phase;
    let phase = s
        .attachment_uploads
        .phase(&s.machines.selected, s.current().id, id)?;
    let entry = s
        .attachment_uploads
        .entries
        .iter()
        .find(|entry| entry.id == id)?;
    if !entry.image
        && !s
            .current()
            .draft_images
            .iter()
            .any(|a| a.id == id && attachment_is_video(a))
    {
        return None;
    }
    let elapsed = match phase {
        Phase::Uploading { since, .. } => since.elapsed(),
        _ => Duration::ZERO,
    };
    if let Phase::Failed(reason) = phase {
        let retry_id = id.to_owned();
        return Some(
            row()
                .key(format!("attachment-status-{id}"))
                .pin(Some(4.0), None, None, Some(4.0))
                .child(
                    icon_button(
                        &format!("retry-attachment-{id}"),
                        &format!("Retry upload for {}", entry.name),
                        "reload",
                        p,
                        move |s| s.retry_attachment_upload(&retry_id),
                    )
                    .help(reason)
                    .fill(p.card)
                    .size(18.0, 18.0),
                ),
        );
    }
    phase.label(elapsed).map(|status| {
        row()
            .key(format!("attachment-status-{id}"))
            .pin(Some(0.0), Some(0.0), None, Some(0.0))
            .px(4.0)
            .justify_center()
            .fill(p.card)
            .child(meta_text(status, p.ink))
    })
}

// Ported from KSG Code's preparingComposerImages tiles in ComposerAttachmentMediaRow.
pub(super) fn preparing_attachment_chip(
    s: &State,
    p: Palette,
    entry: &crate::attachment_uploads::Entry,
) -> El {
    let id = entry.id.clone();
    let thumbnail = entry.thumbnail(&s.attachment_cache).and_then(|thumbnail| {
        if rune::is_headless() {
            let _ = thumbnail.wait();
        }
        (thumbnail.is_ready() && thumbnail.error().is_none()).then_some(thumbnail)
    });
    let body = if let Some(thumbnail) = thumbnail {
        el().size(64.0, 64.0)
            .pixels_fit(thumbnail.pixels(), rune::SurfaceFit::Fill)
    } else {
        icon("image", p.muted)
    };
    let mut tile = el()
        .key(format!("attachment-{id}"))
        .size(64.0, 64.0)
        .radius(6.0)
        .border(0.5, p.line)
        .fill(p.selected)
        .center()
        .label(format!("Preparing {}", entry.name))
        .child(body)
        .child(
            icon_button(
                &format!("remove-attachment-{id}"),
                &format!("Remove {}", entry.name),
                "close",
                p,
                move |s| s.remove_attachment(&id),
            )
            .pin(None, Some(4.0), Some(4.0), None)
            .size(18.0, 18.0)
            .fill(p.card),
        );
    if let Some(status) = attachment_upload_status(s, p, &entry.id) {
        tile = tile.child(status);
    }
    tile
}

// Ported from KSG Code's ComposerAttachmentFileList. Rune's editable pill has no
// suffix/adornment slot, so pending inline files use the reference implementation's file row.
pub(super) fn attachment_upload_file_rows(s: &State, p: Palette) -> Option<(El, Option<String>)> {
    use crate::attachment_uploads::Phase;
    let mut rows = col()
        .keeps_text_selection()
        .on_key(composer_ui::control_key)
        .w_full()
        .gap(4.0)
        .px(12.0)
        .pt(8.0);
    let mut any = false;
    let mut first_focusable: Option<String> = None;
    let inline = inline_file_attachment_ids(s.current());
    for file in s.current().draft_images.iter().filter(|a| {
        !a.is_image() && !attachment_is_video(a) && !a.needs_reattach() && !inline.contains(&a.id)
    }) {
        if s.attachment_uploads
            .phase(&s.machines.selected, s.current().id, &file.id)
            .is_some_and(|phase| !matches!(phase, Phase::Ready | Phase::Legacy))
        {
            continue;
        }
        any = true;
        let id = file.id.clone();
        first_focusable.get_or_insert_with(|| format!("attachment-thumb-{id}"));
        let preview_id = id.clone();
        let size = if file.bytes.div_ceil(1024).max(1) >= 1024 {
            format!("{:.1} MB", file.bytes as f64 / 1048576.0)
        } else {
            format!("{} KB", file.bytes.div_ceil(1024).max(1))
        };
        rows = rows.child(
            row()
                .key(format!("attachment-{id}"))
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .py(4.0)
                .center_y()
                .child(icon("file", p.muted))
                .child(
                    label(&file.name, BODY, p.ink)
                        .key(format!("attachment-thumb-{id}"))
                        .role("button")
                        .label(format!("Preview {}", file.name))
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .truncate()
                        .w(0.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .on_press(move |s: &mut State| {
                            s.preview_attachments(
                                s.current().draft_images.clone(),
                                &preview_id,
                                true,
                            )
                        }),
                )
                .child(meta_text(size, p.muted))
                .child(icon_button(
                    &format!("remove-attachment-{id}"),
                    &format!("Remove {}", file.name),
                    "close",
                    p,
                    move |s| s.remove_attachment(&id),
                )),
        );
    }
    // KSG ComposerAttachmentFileList retains restored files with no bytes as markers.
    for file in s
        .current()
        .draft_images
        .iter()
        .filter(|a| a.needs_reattach())
    {
        any = true;
        let id = file.id.clone();
        first_focusable.get_or_insert_with(|| format!("remove-missing-attachment-{id}"));
        let can_attach = s.file_attachment_staging_supported_on(&s.machines.selected)
            && file.bytes <= crate::attachments::MAX_FILE_BYTES;
        rows = rows.child(
            el().key(format!("attachment-{id}")).w_full().child(
                row()
                    .key(format!("attachment-needs-reattach-{id}"))
                    .w_full()
                    .min_w(0.0)
                    .gap(8.0)
                    .py(4.0)
                    .center_y()
                    .label(format!("{} needs reattach", file.name))
                    .child(icon("file", p.muted))
                    .child(
                        label(&file.name, BODY, p.ink)
                            .truncate()
                            .w(0.0)
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(meta_text(
                        if can_attach {
                            "Attach again"
                        } else {
                            "Remove to send"
                        },
                        p.muted,
                    ))
                    .child(
                        el().key(format!("remove-attachment-{id}"))
                            .child(icon_button(
                                &format!("remove-missing-attachment-{id}"),
                                &format!("Remove {}", file.name),
                                "close",
                                p,
                                move |s| s.remove_attachment(&id),
                            )),
                    ),
            ),
        );
    }
    for entry in s.attachment_uploads.entries.iter().filter(|entry| {
        entry.machine == s.machines.selected
            && entry.thread == s.current().id
            && !entry.image
            && !s
                .current()
                .draft_images
                .iter()
                .any(|a| a.id == entry.id && attachment_is_video(a))
            && !s
                .current()
                .draft_images
                .iter()
                .any(|a| a.id == entry.id && a.needs_reattach())
            && !matches!(entry.phase, Phase::Ready | Phase::Legacy)
    }) {
        any = true;
        let elapsed = match entry.phase {
            Phase::Uploading { since, .. } => since.elapsed(),
            _ => Duration::ZERO,
        };
        let status = entry.phase.label(elapsed).unwrap_or_default();
        let mut file = row()
            .key(format!("attachment-upload-file-{}", entry.id))
            .w_full()
            .min_w(0.0)
            .gap(8.0)
            .py(4.0)
            .center_y()
            .child(icon("file", p.muted))
            .child(
                label(&entry.name, BODY, p.ink)
                    .truncate()
                    .w(0.0)
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(meta_text(status, p.muted).key(format!("attachment-status-{}", entry.id)));
        if let Phase::Failed(reason) = &entry.phase {
            let id = entry.id.clone();
            first_focusable.get_or_insert_with(|| format!("retry-attachment-{id}"));
            file = file.child(
                icon_button(
                    &format!("retry-attachment-{id}"),
                    &format!("Retry upload for {}", entry.name),
                    "reload",
                    p,
                    move |s| s.retry_attachment_upload(&id),
                )
                .help(reason),
            );
        }
        let id = entry.id.clone();
        first_focusable.get_or_insert_with(|| format!("remove-pending-attachment-{id}"));
        file = file.child(icon_button(
            &format!("remove-pending-attachment-{id}"),
            &format!("Remove {}", entry.name),
            "close",
            p,
            move |s| s.remove_attachment(&id),
        ));
        rows = rows.child(file);
    }
    any.then_some((rows, first_focusable))
}

pub(super) fn inline_file_attachment_ids(t: &Thread) -> std::collections::HashSet<String> {
    let inline_ids = crate::composer_tokens::collect_inline_context_ids_from_edit(&t.draft);
    let mut ids: std::collections::HashSet<String> = t
        .draft_context
        .iter()
        .filter(|r| inline_ids.contains(&r.context_id))
        .filter_map(|r| match &r.data {
            crate::composer_context::ComposerContextData::File { attachment_id, .. } => {
                Some(attachment_id.clone())
            }
            _ => None,
        })
        .collect();
    for attachment in &t.draft_images {
        if inline_ids.contains(
            &crate::composer_context::to_kind_scoped_composer_context_id("file", &attachment.id),
        ) {
            ids.insert(attachment.id.clone());
        }
    }
    ids
}

pub(super) fn attachment_is_video(a: &Attachment) -> bool {
    !a.is_image()
        && (a.mime.starts_with("video/")
            || crate::attachment_intake::file_mime(&a.name, &a.mime).starts_with("video/"))
}

pub(crate) fn tray_attachments(t: &Thread) -> Vec<&Attachment> {
    // KSG keeps previewable videos in the media strip even when they have an inline chip.
    t.draft_images
        .iter()
        .filter(|a| a.is_image() || attachment_is_video(a))
        .collect()
}

pub(super) fn remove_attachment_dialog(s: &State, p: Palette) -> El {
    let name = s
        .confirm_remove_attachment
        .as_ref()
        .and_then(|(tid, id)| s.threads.iter().find(|t| t.id == *tid).map(|t| (t, id)))
        .and_then(|(t, id)| t.draft_images.iter().find(|a| a.id == *id))
        .map_or("this image", |a| a.name.as_str());
    let card = col()
        .key("remove-attachment-dialog")
        .role("dialog")
        .label("Remove attachment")
        .w(380.0)
        .max_w(540.0)
        .padding(16.0)
        .gap(12.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(label("Remove attachment", DIALOG_TITLE, p.ink))
        .child(
            label(
                format!(
                    "Remove {name} from the message?\nIt is referenced in your text; removing it also removes every reference."
                ),
                BODY,
                p.ink,
            )
            .wrap(),
        )
        .child(
            row()
                .gap(8.0)
                .child(spacer())
                .child(secondary_button(
                    "cancel-remove-attachment",
                    "Cancel",
                    p,
                    |s| {
                        s.confirm_remove_attachment = None;
                    },
                ))
                .child(button(
                    "confirm-remove-attachment",
                    "Remove",
                    p,
                    |s| s.confirm_remove_attachment_dialog(),
                )),
        );
    el().key("remove-attachment-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .padding(16.0)
        .fill(Color::BLACK.alpha(0.3))
        .modal(false, |s: &mut State| {
            s.confirm_remove_attachment = None;
        })
        .child(card)
}

pub(super) fn message_images(s: &State, p: Palette, images: &[Attachment], key: &str) -> El {
    let mut strip = row()
        .justify_end()
        .items_end()
        .flex_wrap()
        .gap(6.0)
        .max_w(620.0);
    for (index, image) in images.iter().enumerate() {
        let items = images.to_vec();
        let id = image.id.clone();
        let (width, height) = image_size(image, if images.len() == 1 { 220.0 } else { 132.0 });
        strip = strip.child(
            image_preview(s, p, image, width, height, format!("{key}-image-{index}"))
                .role("button")
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| s.preview_attachments(items.clone(), &id, false))
                .label(format!(
                    "Attached {}: {}",
                    if image.is_image() { "image" } else { "file" },
                    image.name
                )),
        );
    }
    strip
}

pub(super) fn attach_control(locked: bool, p: Palette) -> El {
    let mut control = el()
        .key("attach-image")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label("Attach files or folders. Shift-click to choose a folder.")
        .help("Attach files. Shift-click for a folder.")
        .size(24.0, 24.0)
        .center()
        .radius(6.0)
        .opacity(if locked { 0.65 } else { 1.0 })
        .hover(p.selected)
        .disabled(locked)
        .child(icon("paperclip", p.muted));
    if !locked {
        control = control
            .focusable(true)
            .on_press_tx(|s: &mut State, tx: &mut Tx| pick_attachment(s, tx, tx.mods().shift));
    }
    control
}

// Ported from KSG Code's ComposerAttachButton in ComposerAttachmentStrip.tsx.
pub(super) fn pick_attachment(s: &mut State, tx: &mut Tx, folder: bool) {
    let thread = s.current().id;
    let machine = s.machines.selected.clone();
    let question = s.current().question_scope();
    let options = rune::FileDialogOptions {
        title: if folder {
            "Attach a folder"
        } else {
            "Attach files"
        }
        .into(),
        ..Default::default()
    };
    let callback = move |s: &mut State, result, _: &mut Tx| {
        if !crate::attachment_picker::target_accepts(s, &machine, thread, question.as_deref()) {
            return;
        }
        match result {
            Ok(Some(path)) => s.attach_paths_on_to(&machine, thread, &[path]),
            Ok(None) => {}
            Err(error) => {
                s.set_attachment_target_error(
                    &machine,
                    thread,
                    format!("Could not open the attachment picker: {error}"),
                );
            }
        }
        s.checkpoint();
    };
    if folder {
        tx.open_folder(options, callback);
    } else {
        crate::attachment_picker::open(s, thread, options);
    }
}
