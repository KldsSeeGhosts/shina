//! Composer prompt stash, ported from KSG Code's `useComposerStash.ts`,
//! `promptStashStore.ts`, `ComposerStashBadge.tsx` and `ComposerStashMenu.tsx`.
//!
//! A stash entry keeps the rich draft document, attachment references, context records
//! and the reply target. Entries are stored newest-first in `composer-stash.json` next
//! to `ui-state.json`; attachments stay by reference so the file never carries payloads.
//! The queue is global, as KSG's single localStorage queue is: one list shared by every
//! thread, project and machine this client drives.

use super::*;

use crate::attachments::Attachment;
use crate::composer_context::{ComposerContextRecord, RetainedContextPayloads};
use crate::composer_notices::{Notice, Variant};
use serde_json::{Value, json};
use std::collections::HashMap;

/// KSG's `MAX_STASH_ENTRIES`.
pub const MAX_ENTRIES: usize = 20;
const FILE: &str = "composer-stash.json";
pub struct Entry {
    pub id: String,
    /// Unix seconds, from `crate::projects::now()`, for the menu's relative age.
    pub created: u64,
    /// The draft it came from; the reply record only restores onto that same thread.
    pub machine: String,
    pub thread: usize,
    /// Rich draft: plain text, editor HTML, attachments and visible context records.
    pub snapshot: crate::composer_rich::Snapshot,
    pub reply_to: Option<ComposerContextRecord>,
    /// Payloads retained for pills the draft can re-reference (KSG's records map).
    pub retained: RetainedContextPayloads,
    /// Attachment names lost between save and load, KSG's dropped/unreadable lists.
    pub missing: Vec<String>,
}

#[derive(Default)]
pub struct Stash {
    /// Newest first, as KSG prepends each entry.
    pub entries: Vec<Entry>,
}

fn path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(FILE)
}

impl Stash {
    fn to_value(&self) -> Value {
        json!({
            "version": 1,
            "entries": self.entries.iter().map(entry_to_value).collect::<Vec<_>>(),
        })
    }
}

fn entry_to_value(entry: &Entry) -> Value {
    json!({
        "id": entry.id,
        "created": entry.created,
        "machine": entry.machine,
        "thread": entry.thread,
        "text": entry.snapshot.text,
        "html": entry.snapshot.html,
        "attachments": entry.snapshot.attachments.iter().map(Attachment::to_value).collect::<Vec<_>>(),
        "context": entry.snapshot.context.iter().filter_map(|r| serde_json::to_value(r).ok()).collect::<Vec<_>>(),
        "replyTo": entry.reply_to.as_ref().and_then(|r| serde_json::to_value(r).ok()),
        "retained": {
            "records": entry.retained.records.iter().filter_map(|(k, r)| serde_json::to_value(r).ok().map(|v| (k.clone(), v))).collect::<serde_json::Map<_, _>>(),
            "files": entry.retained.files.iter().map(|(k, a)| (k.clone(), a.to_value())).collect::<serde_json::Map<_, _>>(),
        },
        "missing": entry.missing,
    })
}

fn records_from(value: &Value) -> HashMap<String, ComposerContextRecord> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| {
                    serde_json::from_value::<ComposerContextRecord>(v.clone())
                        .ok()
                        .map(|r| (k.clone(), r))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn files_from(value: &Value) -> HashMap<String, Attachment> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| Attachment::from_value(v).map(|a| (k.clone(), a)))
                .collect()
        })
        .unwrap_or_default()
}

fn entry_from_value(value: &Value) -> Option<Entry> {
    let mut missing = Vec::new();
    let mut attachments = Vec::new();
    for a in value["attachments"].as_array().into_iter().flatten() {
        match Attachment::from_value(a) {
            Some(attachment) => attachments.push(attachment),
            None => {
                // The blob is gone. Keep the name so restore can say what was lost,
                // as KSG tracks dropped/unreadable image names.
                missing.push(a["name"].as_str().unwrap_or("attachment").to_owned());
            }
        }
    }
    let context: Vec<ComposerContextRecord> = value["context"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| serde_json::from_value::<ComposerContextRecord>(v.clone()).ok())
        .collect();
    missing.extend(
        value["missing"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned)),
    );
    Some(Entry {
        id: value["id"].as_str()?.to_owned(),
        created: value["created"].as_u64().unwrap_or(0),
        machine: value["machine"].as_str().unwrap_or("local").to_owned(),
        thread: value["thread"].as_u64().unwrap_or(0) as usize,
        snapshot: crate::composer_rich::Snapshot {
            text: value["text"].as_str()?.to_owned(),
            html: value["html"].as_str().unwrap_or_default().to_owned(),
            attachments,
            context,
        },
        reply_to: value
            .get("replyTo")
            .filter(|v| !v.is_null())
            .and_then(|v| serde_json::from_value::<ComposerContextRecord>(v.clone()).ok()),
        retained: RetainedContextPayloads {
            records: records_from(&value["retained"]["records"]),
            files: files_from(&value["retained"]["files"]),
        },
        missing,
    })
}

/// Read the stash file. A corrupt file moves aside to `composer-stash.json.corrupt` and
/// the caller gets one warning to surface, so a bad stash never breaks UI state loading.
pub fn load(data_dir: &std::path::Path) -> (Stash, Option<Notice>) {
    let path = path(data_dir);
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(_) => return (Stash::default(), None),
    };
    let value: Result<Value, _> = serde_json::from_slice(&raw);
    let entries = value.ok().and_then(|v| v["entries"].as_array().cloned());
    let Some(entries) = entries else {
        let corrupt = data_dir.join(format!("{FILE}.corrupt"));
        let _ = std::fs::rename(&path, &corrupt);
        return (
            Stash::default(),
            Some(
                Notice::new(
                    "stash-corrupt",
                    Variant::Warning,
                    "alert",
                    "Could not read the stash",
                )
                .description(Some(
                    "The saved stash file was set aside and the stash starts empty.".into(),
                )),
            ),
        );
    };
    let stash = Stash {
        entries: entries
            .iter()
            .filter_map(entry_from_value)
            .take(MAX_ENTRIES)
            .collect(),
    };
    (stash, None)
}

/// Write the stash now. KSG persists before clearing the composer, so a rejected write
/// must fail before the draft is touched; `write_atomic` is the store's own primitive.
fn save(s: &State) -> std::io::Result<()> {
    crate::store::write_atomic(
        &path(&s.data_dir),
        serde_json::to_string_pretty(&s.stash.to_value())
            .expect("stash serializes")
            .as_bytes(),
    )
}

fn warn(s: &mut State, id: &str, title: impl Into<String>, description: Option<String>) {
    s.composer_notices
        .push(Notice::new(id, Variant::Warning, "alert", title).description(description));
}

/// Release one entry's staged uploads, as KSG releases assets on delete and evict.
fn release(s: &State, entry: &Entry) {
    for attachment in &entry.snapshot.attachments {
        if let Some(uploaded) = &attachment.uploaded {
            crate::attachment_uploads::release_staged(s, uploaded);
        }
    }
    for attachment in entry.retained.files.values() {
        if let Some(uploaded) = &attachment.uploaded {
            crate::attachment_uploads::release_staged(s, uploaded);
        }
    }
}

/// Stash the current draft, from `useComposerStash.stashPrompt`. The write lands before
/// the composer clears; a rejected write leaves the composer untouched, as KSG's does.
pub fn stash(s: &mut State) {
    let machine = s.machines.selected.clone();
    let t = s.current();
    if !t.has_draft_content() {
        return;
    }
    if s.attachment_uploads.block_reason(&machine, t.id).is_some() {
        warn(
            s,
            "stash-uploads",
            "Wait for file uploads before stashing this prompt",
            None,
        );
        return;
    }
    if t.draft_images.iter().any(Attachment::needs_reattach) {
        warn(
            s,
            "stash-reattach",
            "Attach dropped files again or remove them before stashing",
            None,
        );
        return;
    }
    let t = s.current();
    let entry = Entry {
        id: format!("stash-{}", uuid::Uuid::new_v4()),
        created: crate::projects::now(),
        machine: machine.clone(),
        thread: t.id,
        snapshot: crate::composer_rich::Snapshot::capture_with_context(
            &t.draft,
            &t.draft_images,
            &t.draft_context,
        ),
        reply_to: t.draft_reply_to.clone(),
        retained: t.retained_context.clone(),
        missing: Vec::new(),
    };
    // KSG unshifts the newest entry and evicts the oldest past the cap.
    let mut evicted = None;
    s.stash.entries.insert(0, entry);
    if s.stash.entries.len() > MAX_ENTRIES {
        evicted = s.stash.entries.pop();
    }
    if save(s).is_err() {
        // Roll back exactly like KSG's store: the composer is left as-is.
        s.stash.entries.remove(0);
        if let Some(entry) = evicted {
            s.stash.entries.push(entry);
        }
        warn(
            s,
            "stash-write",
            "Could not stash this prompt",
            Some("Local storage rejected the write, so the composer was left as-is.".into()),
        );
        return;
    }
    if let Some(entry) = evicted {
        release(s, &entry);
        warn(
            s,
            "stash-evicted",
            "Oldest stashed prompt discarded",
            Some(format!(
                "The stash holds {MAX_ENTRIES} prompts; the oldest was removed to make room."
            )),
        );
    }
    // Clear the draft the way a send does; replace_text drops undo history, so the
    // stashed text cannot come back through the editor's undo.
    let t = s.current_mut();
    t.prompt_history.reset();
    t.typing_guard.on_blur_or_send();
    t.paste_history = Default::default();
    t.draft.replace_text("");
    t.draft_images.clear();
    t.draft_context.clear();
    t.draft_reply_to = None;
    t.retained_context = Default::default();
    t.update_submission_error();
    s.stash_pulse += 1;
    s.checkpoint();
}

/// Restore an entry: it leaves the stash and appends to the draft with a blank line,
/// from `useComposerStash.restoreEntry` and `composer_rich::prepend`.
pub fn restore(s: &mut State, id: &str) {
    let Some(position) = s.stash.entries.iter().position(|e| e.id == id) else {
        return;
    };
    let entry = s.stash.entries.remove(position);
    if save(s).is_err() {
        warn(
            s,
            "stash-restore-write",
            "Restored prompt may reappear in the stash",
            Some(
                "Local storage rejected the update, so this entry could still be there after a reload."
                    .into(),
            ),
        );
    }
    let t = s.current_mut();
    // Existing content stays, then the blank-paragraph separator, then the stash.
    let stashed = entry.snapshot.editor();
    t.draft = crate::composer_rich::prepend(&t.draft, &stashed);
    let mut over_limit = false;
    for attachment in entry.snapshot.attachments {
        if t.draft_images.iter().any(|a| a.id == attachment.id) {
            continue;
        }
        if t.draft_images.len() >= crate::attachments::MAX_ATTACHMENTS {
            over_limit = true;
            continue;
        }
        t.draft_images.push(attachment);
    }
    if over_limit {
        t.attachment_error = Some(format!(
            "You can attach up to {} files per message or question response.",
            crate::attachments::MAX_ATTACHMENTS
        ));
    }
    for record in entry.snapshot.context {
        if !t
            .draft_context
            .iter()
            .any(|r| r.context_id == record.context_id)
        {
            t.draft_context.push(record);
        }
    }
    for (k, record) in entry.retained.records {
        t.retained_context.records.entry(k).or_insert(record);
    }
    for (k, file) in entry.retained.files {
        t.retained_context.files.entry(k).or_insert(file);
    }
    if t.draft_reply_to.is_none() && entry.machine == t.machine && entry.thread == t.id {
        t.draft_reply_to = entry.reply_to;
    }
    t.reconcile_draft_context();
    // One warning for everything that did not survive, as KSG emits once per restore.
    let mut missing = entry.missing;
    for attachment in t
        .draft_images
        .iter()
        .filter(|a| a.needs_reattach())
        .cloned()
        .collect::<Vec<_>>()
    {
        missing.push(attachment.name);
    }
    s.current_mut().update_submission_error();
    if !missing.is_empty() {
        warn(
            s,
            "stash-missing",
            "Some attachments were not restored",
            Some(format!("{} could not be restored.", missing.join(", "))),
        );
    }
    s.stash_menu = false;
    s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
    s.checkpoint();
}

/// Delete an entry and release its attachment holds, KSG's `deleteEntry`.
pub fn delete(s: &mut State, id: &str) {
    let Some(position) = s.stash.entries.iter().position(|e| e.id == id) else {
        return;
    };
    let entry = s.stash.entries.remove(position);
    release(s, &entry);
    let _ = save(s);
    s.stash_highlight = s
        .stash_highlight
        .min(s.stash.entries.len().saturating_sub(1));
    if s.stash.entries.is_empty() {
        s.stash_menu = false;
        s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
    }
    s.checkpoint();
}

/// KSG's `--chat-composer-drawer-inset: 1.375rem` leaves 22 px of lane on each side.
const DRAWER_INSET: f32 = 22.0;

/// KSG's stashCommand dispatch: content stashes; an empty composer restores the only
/// entry or opens the menu when several exist.
pub fn command(s: &mut State) {
    if s.current().has_draft_content() {
        stash(s);
        return;
    }
    match s.stash.entries.len() {
        0 => {}
        1 => {
            let id = s.stash.entries[0].id.clone();
            restore(s, &id);
        }
        _ => {
            s.stash_menu = true;
            s.stash_highlight = 0;
        }
    }
}

/// Whether the menu is on screen. Another composer menu or popover opening takes the
/// layer, as KSG closes the drawer when the command menu opens.
pub fn menu_open(s: &State) -> bool {
    s.stash_menu
        && !s.stash.entries.is_empty()
        && super::composer_menu::trigger(s).is_none()
        && !s.machines.picker
        && !s.current().native.access_picker
        && s.settings_picker.is_none()
}

pub fn close_menu(s: &mut State) {
    s.stash_menu = false;
    s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
}

/// KSG's highlighted id resolves to the first entry when its row is gone; the highlight
/// index clamps the same way here.
fn highlight(s: &State) -> usize {
    s.stash_highlight
        .min(s.stash.entries.len().saturating_sub(1))
}

/// Menu keys, scoped exactly as KSG's drawer handler: they act only while focus is in
/// the menu, on the badge, or in the composer, so dialogs and unrelated inputs keep
/// their own bindings.
pub fn menu_key(s: &mut State, event: &rune::KeyEvent, tx: &rune::Tx) -> bool {
    if !menu_open(s) {
        return false;
    }
    let focused = tx.focused().unwrap_or_default();
    let scoped = focused == "composer-input"
        || focused == "composer-stash-badge"
        || focused.starts_with("composer-stash-");
    if !scoped {
        return false;
    }
    let count = s.stash.entries.len();
    match event.key {
        rune::Key::Down => {
            s.stash_highlight = (highlight(s) + 1).min(count.saturating_sub(1));
            true
        }
        rune::Key::Up => {
            s.stash_highlight = highlight(s).saturating_sub(1);
            true
        }
        rune::Key::Enter => {
            let id = s.stash.entries[highlight(s)].id.clone();
            restore(s, &id);
            true
        }
        rune::Key::Escape => {
            close_menu(s);
            true
        }
        rune::Key::Backspace if event.mods.cmd || event.mods.ctrl => {
            let id = s.stash.entries[highlight(s)].id.clone();
            delete(s, &id);
            true
        }
        _ => false,
    }
}

/// KSG's `ComposerStashBadge`: hidden while the stash is empty, archive glyph plus the
/// count in an `outlined` chip inside a 28 px press target, and a pulse on each stash.
pub(super) fn badge(s: &State, p: Palette) -> El {
    let count = s.stash.entries.len();
    if count == 0 {
        return el();
    }
    el().key("composer-stash-badge")
        .role("button")
        .label(format!("Stashed drafts, {count}"))
        .focusable(true)
        .size(28.0, 28.0)
        .center()
        .cursor(rune::Cursor::Pointer)
        .hover(p.selected)
        .press(p.selected)
        .radius(4.0)
        .preserve_focus_on_press(true)
        .on_key(|s, event, tx| menu_key(s, event, tx))
        .on_press(|s: &mut State| {
            s.stash_menu = !menu_open(s);
            s.stash_highlight = 0;
        })
        .child(outlined(
            row()
                .key(format!("composer-stash-badge-chip-{}", s.stash_pulse))
                .h(20.0)
                .px(5.0)
                .gap(4.0)
                .center_y()
                .radius(4.0)
                .fill(p.panel)
                .sequence(rune::Sequence::new(STASH_PULSE))
                .child(icon("archive", p.muted))
                .child(pixel_text(format!("{count}"), p.muted)),
            p,
        ))
}

/// The badge pulse, one shot per stash, replayed by the chip's changing key — KSG's
/// `stash-pulse-count 1400ms` scale beat.
static STASH_PULSE: &[rune::Beat] = &[
    rune::Beat::new(0.0).scale(1.0),
    rune::Beat::new(0.18).scale(1.15),
    rune::Beat::new(1.4).scale(1.0),
];

/// Snippet text as KSG shows it: the draft flattened to one line.
fn snippet(entry: &Entry) -> String {
    entry
        .snapshot
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// KSG's `ComposerStashMenu`: newest-first rows of snippet, relative age and the
/// attachment/context counts, anchored above the badge at the drawer width
/// (`calc(100% - 2 * 1.375rem)` of the composer lane).
pub(super) fn menu(s: &State, p: Palette, lane: f32) -> El {
    let highlighted = highlight(s);
    let mut list = col().key("composer-stash-list").w_full();
    for (index, entry) in s.stash.entries.iter().enumerate() {
        let id = entry.id.clone();
        let delete_id = entry.id.clone();
        let images = entry
            .snapshot
            .attachments
            .iter()
            .filter(|a| a.is_image())
            .count();
        let files = entry.snapshot.attachments.len() - images;
        let records = entry.snapshot.context.len() + entry.retained.records.len();
        let mut meta = vec![relative_time(entry.created).unwrap_or_else(|| "now".into())];
        if images > 0 {
            meta.push(format!(
                "{images} image{}",
                if images == 1 { "" } else { "s" }
            ));
        }
        if files > 0 {
            meta.push(format!("{files} file{}", if files == 1 { "" } else { "s" }));
        }
        if records > 0 {
            meta.push(format!(
                "{records} chip{}",
                if records == 1 { "" } else { "s" }
            ));
        }
        let mut row_el = row()
            .key(format!("composer-stash-row-{index}"))
            .role("listitem")
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .w_full()
            .h(44.0)
            .px(10.0)
            .gap(8.0)
            .center_y()
            .radius(4.0)
            .preserve_focus_on_press(true)
            .on_press(move |s: &mut State| restore(s, &id))
            .child(
                col()
                    .grow(1.0)
                    .shrink()
                    .min_w(0.0)
                    .gap(2.0)
                    .child(label(snippet(entry), SMALL, p.ink).truncate())
                    .child(label(meta.join(" \u{00b7} "), PIXEL, p.muted).truncate()),
            )
            .child(
                el().key(format!("composer-stash-delete-{index}"))
                    .role("button")
                    .label("Delete stashed draft")
                    .focusable(true)
                    .size(24.0, 24.0)
                    .center()
                    .cursor(rune::Cursor::Pointer)
                    .hover(p.selected)
                    .radius(4.0)
                    .preserve_focus_on_press(true)
                    .on_press(move |s: &mut State| delete(s, &delete_id))
                    .child(icon("close", p.muted)),
            );
        if index == highlighted {
            row_el = row_el.fill(p.selected);
        }
        row_el = row_el.on_hover_change(move |s: &mut State, hovering: bool, _| {
            if hovering {
                s.stash_highlight = index;
            }
        });
        list = list.child(row_el);
    }
    outlined(
        col()
            .key("composer-stash-menu")
            .role("list")
            .focusable(true)
            .active_descendant(format!("composer-stash-row-{highlighted}"))
            .label("Stashed drafts")
            .on_key(|s, event, tx| menu_key(s, event, tx))
            .fill(p.panel)
            .radius(6.0)
            .clip_rounded(6.0)
            .w((lane - 2.0 * DRAWER_INSET).max(240.0))
            .max_h(288.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(list)
            .box_shadow(hard_shadow(p)),
        p,
    )
    .anchored_fit(
        "composer-stash-badge",
        rune::Side::Top,
        rune::Align::End,
        8.0,
    )
    .focus_return("composer-input")
    .instant_changes()
    // A light dismiss layer like the composer overflow menu: a press outside the
    // drawer or Escape closes it without trapping the composer.
    .modal(true, |s: &mut State| {
        s.stash_menu = false;
        s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composer_context::ComposerContextData;
    use std::fs;
    use std::path::PathBuf;

    fn state() -> State {
        State::new(crate::store::directory().unwrap(), false)
    }

    fn stash_file(s: &State) -> PathBuf {
        s.data_dir.join(FILE)
    }

    /// A stashed image: its blob lives under the attachments directory, as intake leaves it.
    fn image(id: &str) -> Attachment {
        let directory = crate::attachments::directory().unwrap();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(id), [b'i'; 32]).unwrap();
        Attachment {
            id: id.into(),
            name: id.into(),
            mime: "image/png".into(),
            width: 1,
            height: 1,
            bytes: 32,
            path: None,
            asset_id: None,
            uploaded: None,
            source: None,
        }
    }

    #[test]
    fn stash_clears_the_composer_and_writes_the_file() {
        let mut s = state();
        s.current_mut().draft.replace_text("hold this");
        s.current_mut().draft_images.push(image("stash-a.png"));
        stash(&mut s);
        assert!(s.current().draft.text.is_empty());
        assert!(s.current().draft_images.is_empty());
        let [entry] = s.stash.entries.as_slice() else {
            panic!("one entry");
        };
        assert_eq!(entry.snapshot.text, "hold this");
        assert_eq!(entry.snapshot.attachments.len(), 1);
        assert!(stash_file(&s).is_file());
    }

    #[test]
    fn the_twenty_first_entry_evicts_the_oldest() {
        let mut s = state();
        for i in 0..MAX_ENTRIES + 1 {
            s.current_mut().draft.replace_text(format!("draft {i}"));
            stash(&mut s);
        }
        assert_eq!(s.stash.entries.len(), MAX_ENTRIES);
        assert_eq!(
            s.stash.entries[0].snapshot.text,
            format!("draft {}", MAX_ENTRIES)
        );
        assert_eq!(s.stash.entries.last().unwrap().snapshot.text, "draft 1");
    }

    #[test]
    fn restore_appends_after_a_blank_line_and_removes_the_entry() {
        let mut s = state();
        s.current_mut().draft.replace_text("stashed");
        stash(&mut s);
        s.current_mut().draft.replace_text("current");
        let id = s.stash.entries[0].id.clone();
        restore(&mut s, &id);
        assert_eq!(s.current().draft.text, "current\n\nstashed");
        assert!(s.stash.entries.is_empty());
    }

    #[test]
    fn delete_removes_the_entry_and_rewrites_the_file() {
        let mut s = state();
        s.current_mut().draft.replace_text("first");
        stash(&mut s);
        s.current_mut().draft.replace_text("second");
        stash(&mut s);
        let newest = s.stash.entries[0].id.clone();
        delete(&mut s, &newest);
        assert_eq!(s.stash.entries.len(), 1);
        assert_eq!(s.stash.entries[0].snapshot.text, "first");
        let (loaded, _) = load(&s.data_dir);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].snapshot.text, "first");
    }

    #[test]
    fn the_stash_survives_save_and_reload() {
        let mut s = state();
        s.current_mut().draft.replace_text("kept");
        s.current_mut().draft_images.push(image("kept.png"));
        stash(&mut s);
        let (loaded, warning) = load(&s.data_dir);
        assert!(warning.is_none());
        let [entry] = loaded.entries.as_slice() else {
            panic!("one entry");
        };
        assert_eq!(entry.snapshot.text, "kept");
        assert_eq!(entry.snapshot.attachments.len(), 1);
        assert!(entry.missing.is_empty());
    }

    #[test]
    fn a_corrupt_stash_file_moves_aside_and_warns() {
        let s = state();
        fs::write(s.data_dir.join(FILE), b"not json").unwrap();
        let (loaded, warning) = load(&s.data_dir);
        assert!(loaded.entries.is_empty());
        assert!(warning.is_some());
        assert!(s.data_dir.join(format!("{FILE}.corrupt")).is_file());
        assert!(!s.data_dir.join(FILE).exists());
    }

    #[test]
    fn rich_formatting_and_context_chips_round_trip() {
        let mut s = state();
        {
            let t = s.current_mut();
            t.draft = crate::composer_rich::draft("Title\ntask\nbold\nchip ");
            t.draft
                .set_block_kind(0, rune::BlockKind::Heading { level: 1 });
            t.draft.set_block_kind(
                1,
                rune::BlockKind::Task {
                    level: 0,
                    checked: false,
                },
            );
            t.draft.set_selection(11, 15);
            t.draft.toggle_mark(rune::Mark::Bold);
            t.draft.set_caret(t.draft.text.len());
            t.draft
                .insert_pill("t3-context://v1/model/model-1", "a model");
            t.draft_context.push(ComposerContextRecord {
                version: 1,
                context_id: "model-1".into(),
                label: "a model".into(),
                data: ComposerContextData::Model {
                    instance_id: "i".into(),
                    model: "m".into(),
                    provider_label: None,
                },
            });
        }
        stash(&mut s);
        s.current_mut().draft.replace_text("before");
        let id = s.stash.entries[0].id.clone();
        restore(&mut s, &id);
        let t = s.current();
        assert_eq!(t.draft.block_kind(0), rune::BlockKind::Paragraph);
        assert_eq!(t.draft.block_kind(2), rune::BlockKind::Heading { level: 1 });
        assert_eq!(
            t.draft.block_kind(3),
            rune::BlockKind::Task {
                level: 0,
                checked: false
            }
        );
        assert!(
            t.draft
                .marks()
                .iter()
                .any(|m| matches!(m.mark, rune::Mark::Bold))
        );
        assert!(
            t.draft
                .pills()
                .any(|(_, id)| id == "t3-context://v1/model/model-1")
        );
        assert!(t.draft_context.iter().any(|r| r.context_id == "model-1"));
    }

    #[test]
    fn a_missing_attachment_comes_back_needing_reattach_with_one_warning() {
        let mut s = state();
        let directory = crate::attachments::directory().unwrap();
        let attachment = crate::attachments::store_uploaded_file(
            &directory,
            "notes.txt",
            "text/plain",
            b"notes",
        )
        .unwrap();
        let blob = directory.join(&attachment.id);
        s.current_mut().draft_images.push(attachment);
        s.current_mut().draft.replace_text("with file");
        stash(&mut s);
        fs::remove_file(&blob).unwrap();
        let id = s.stash.entries[0].id.clone();
        restore(&mut s, &id);
        let t = s.current();
        assert_eq!(t.draft_images.len(), 1);
        assert!(t.draft_images[0].needs_reattach());
        let notices = s
            .composer_notices
            .ordered()
            .iter()
            .filter(|n| n.id == "stash-missing")
            .count();
        assert_eq!(notices, 1);
    }

    #[test]
    fn a_stashed_attachment_blob_survives_while_the_entry_lives() {
        let mut s = state();
        s.current_mut().draft_images.push(image("kept-alive.png"));
        s.current_mut().draft.replace_text("x");
        stash(&mut s);
        let (loaded, _) = load(&s.data_dir);
        assert_eq!(loaded.entries[0].snapshot.attachments.len(), 1);
        assert!(loaded.entries[0].missing.is_empty());
        assert!(
            crate::attachments::directory()
                .unwrap()
                .join("kept-alive.png")
                .is_file()
        );
    }

    fn key_event(key: rune::Key) -> rune::KeyEvent {
        rune::KeyEvent {
            key,
            mods: Default::default(),
        }
    }

    /// A headless frame with the stash file kept in the test profile, not the worktree.
    fn stash_runtime(width: f32) -> rune::Runtime<State> {
        let mut rt = crate::ui::tests::runtime(width);
        rt.on_key = Some(crate::key);
        rt.state.data_dir = crate::store::directory().unwrap();
        rt
    }

    fn stash_texts(rt: &mut rune::Runtime<State>, texts: &[&str]) {
        for text in texts {
            rt.state.current_mut().draft.replace_text(*text);
            stash(&mut rt.state);
        }
    }

    #[test]
    fn the_shortcut_stashes_restores_one_and_opens_the_menu_for_many() {
        let mut s = state();
        // Empty composer, empty stash: Mod+S does nothing.
        command(&mut s);
        assert!(s.stash.entries.is_empty() && !s.stash_menu);
        // With content it stashes.
        s.current_mut().draft.replace_text("only draft");
        command(&mut s);
        assert!(s.current().draft.text.is_empty());
        assert_eq!(s.stash.entries.len(), 1);
        // Empty composer and exactly one entry restores it.
        command(&mut s);
        assert_eq!(s.current().draft.text, "only draft");
        assert!(s.stash.entries.is_empty());
        // Several entries open the menu.
        s.current_mut().draft.replace_text("first");
        command(&mut s);
        s.current_mut().draft.replace_text("second");
        command(&mut s);
        assert_eq!(s.stash.entries.len(), 2);
        command(&mut s);
        assert!(menu_open(&s));
    }

    #[test]
    fn the_menu_navigates_restores_deletes_and_closes() {
        let mut rt = stash_runtime(1280.0);
        stash_texts(&mut rt, &["one", "two", "three"]);
        rt.state.stash_menu = true;
        rt.invalidate();
        rt.frame(0.0);
        assert!(rt.presented("composer-stash-menu").is_some());
        assert_eq!(rt.state.stash_highlight, 0);
        // The drawer takes focus when it mounts, like KSG's list focus.
        rt.frame(0.0);
        // Arrows move the highlight.
        rt.key(key_event(rune::Key::Down));
        rt.frame(0.0);
        assert_eq!(rt.state.stash_highlight, 1);
        rt.key(key_event(rune::Key::Down));
        rt.frame(0.0);
        assert_eq!(rt.state.stash_highlight, 2);
        rt.key(key_event(rune::Key::Down));
        rt.frame(0.0);
        assert_eq!(rt.state.stash_highlight, 2, "clamped at the last row");
        rt.key(key_event(rune::Key::Up));
        rt.frame(0.0);
        assert_eq!(rt.state.stash_highlight, 1);
        // Enter restores the highlighted row and closes the menu.
        rt.key(key_event(rune::Key::Enter));
        rt.frame(0.0);
        assert_eq!(rt.state.current().draft.text, "two");
        assert_eq!(rt.state.stash.entries.len(), 2);
        assert!(!menu_open(&rt.state));
        // Mod+Backspace deletes the highlighted entry; the last one closes the menu.
        rt.state.stash_menu = true;
        rt.state.stash_highlight = 0;
        rt.invalidate();
        rt.frame(0.0);
        rt.key(rune::KeyEvent {
            key: rune::Key::Backspace,
            mods: rune::Mods {
                cmd: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(rt.state.stash.entries.len(), 1);
        assert!(menu_open(&rt.state));
        rt.key(rune::KeyEvent {
            key: rune::Key::Backspace,
            mods: rune::Mods {
                cmd: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert!(rt.state.stash.entries.is_empty());
        assert!(!menu_open(&rt.state));
    }

    #[test]
    fn the_menu_restores_on_row_click_deletes_per_row_and_closes_outside() {
        let mut rt = stash_runtime(1280.0);
        stash_texts(&mut rt, &["alpha", "beta"]);
        rt.state.stash_menu = true;
        rt.invalidate();
        rt.frame(0.0);
        // A row click restores that entry.
        crate::ui::tests::press(&mut rt, "composer-stash-row-1");
        assert_eq!(rt.state.current().draft.text, "alpha");
        assert_eq!(rt.state.stash.entries.len(), 1);
        // Reopen: the per-row delete removes its entry.
        rt.state.stash_menu = true;
        rt.invalidate();
        rt.frame(0.0);
        crate::ui::tests::press(&mut rt, "composer-stash-delete-0");
        rt.frame(0.0);
        assert!(rt.state.stash.entries.is_empty());
        assert!(!menu_open(&rt.state), "the last delete closes the menu");
        // Reopen, then a press outside the drawer dismisses it as the composer
        // overflow menu's outside press does.
        stash_texts(&mut rt, &["again"]);
        rt.state.stash_menu = true;
        rt.invalidate();
        rt.frame(0.0);
        assert!(menu_open(&rt.state));
        rt.pointer_down(640.0, 240.0);
        rt.pointer_up(640.0, 240.0);
        rt.frame(0.0);
        assert!(!rt.state.stash_menu);
    }

    #[test]
    fn escape_closes_and_hover_highlights() {
        let mut rt = stash_runtime(1280.0);
        stash_texts(&mut rt, &["one", "two", "three"]);
        rt.state.stash_menu = true;
        rt.invalidate();
        rt.frame(0.0);
        let (x, y) = rt.find("composer-stash-row-2").expect("row 2");
        rt.pointer_move(x, y);
        rt.frame(0.0);
        assert_eq!(rt.state.stash_highlight, 2);
        crate::ui::tests::press(&mut rt, "composer-input");
        rt.key(key_event(rune::Key::Escape));
        rt.frame(0.0);
        assert!(!menu_open(&rt.state));
    }

    #[test]
    fn the_badge_sits_in_the_footer_and_the_menu_fits_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = stash_runtime(width);
            stash_texts(&mut rt, &["one", "two", "three"]);
            rt.state.stash_menu = true;
            rt.frame(0.0);
            rt.frame(1.0);
            let (badge, _) = rt
                .presented("composer-stash-badge")
                .unwrap_or_else(|| panic!("{width}: badge"));
            let (footer, _) = rt
                .presented("composer-footer")
                .unwrap_or_else(|| panic!("{width}: footer"));
            assert!(
                badge[1] >= footer[1] - 1.0 && badge[1] + badge[3] <= footer[1] + footer[3] + 1.0,
                "{width}: badge {badge:?} in footer {footer:?}"
            );
            assert!(
                badge[0] + badge[2] <= footer[0] + footer[2] + 1.0,
                "{width}: badge inside the footer row {footer:?}"
            );
            let (menu_rect, _) = rt
                .presented("composer-stash-menu")
                .unwrap_or_else(|| panic!("{width}: menu"));
            assert!(
                menu_rect[0] >= -1.0 && menu_rect[0] + menu_rect[2] <= width + 1.0,
                "{width}: menu {menu_rect:?} inside the window"
            );
            assert!(menu_rect[1] >= -1.0, "{width}: menu top {menu_rect:?}");
            for i in 0..3 {
                assert!(
                    rt.presented(&format!("composer-stash-row-{i}")).is_some(),
                    "{width}: row {i}"
                );
            }
        }
    }
}
