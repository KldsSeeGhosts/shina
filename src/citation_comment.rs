//! The citation chip's comment editor and its actions, ported from KSG Code's
//! `AssistantCitationChip.tsx`, `AssistantCitationCommentEditor.tsx` and
//! `assistantCitationCommentDismissal.ts`, plus the `removeOnCancel` path in
//! `ComposerPromptEditorTiptap.tsx`.
//!
//! A Rune pill is one atom, so KSG's two chip actions split in two: the pill
//! keeps `on_input_pill_press` for the jump to the cited response (wired in
//! `composer_press`), while a small pencil button overlays the gutter the
//! pill's covered text reserves at its right edge (`chip_text`) and opens the
//! popover anchored above the chip.

use super::*;

use crate::assistant_citation::{
    ASSISTANT_CITATION_MAX_COMMENT_LENGTH, assistant_citation_label, markdown_rendered_text,
    normalize_whitespace, utf16_len, with_assistant_citation_comment,
};
use crate::composer_context::{
    AssistantCitation, ComposerContextData, ComposerContextRecord, format_composer_context_href,
    sanitize_composer_context_label,
};
use crate::ui::composer_ui::FocusReason;
use rune::TextEdit;
use std::ops::Range;

/// Space the pill's covered text carries so the comment action has room at the
/// chip's right edge, inside the pill like KSG's `ContextChipAction`. These are
/// non-breaking spaces: `TextEdit::from_html` collapses ASCII whitespace runs,
/// which would make a restored draft's text differ from the snapshot's and
/// drop the pill; `\u{a0}` survives the round trip, and
/// `sanitize_composer_context_label` still normalizes it out of the Markdown
/// link the provider sees.
pub(crate) const CHIP_ACTION_GUTTER: &str = "\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}";
/// The gutter's width in points (about five spaces at the composer's size).
const ACTION_SLOT: f32 = 20.0;

/// Which spaces `insert_context_chip` added with the citation, so cancelling
/// the editor it auto-opened can undo the whole insert (KSG's `insertedSpaces`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InsertedSpaces {
    pub before: bool,
    pub after: bool,
}

/// An open comment editor: which record it edits, its draft and how closing
/// behaves. One is open at a time, on the current thread's draft.
pub(crate) struct Editing {
    /// The `draft_context` record the popover edits.
    pub context_id: String,
    /// The chip action it stays anchored to (a repeated citation anchors its own).
    pub anchor: String,
    /// The field's content, initialized from the saved comment with the caret at its end.
    pub draft: TextEdit,
    /// `citation.comment` when the editor opened (KSG's `savedComment`).
    pub saved: Option<String>,
    /// Set when the editor opened right after the citation was inserted:
    /// cancel removes the citation and the spaces it arrived with.
    pub remove_on_cancel: Option<InsertedSpaces>,
}

/// KSG's `AssistantCitationCommentDismissal`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Dismissal {
    Commit(String),
    Close,
    KeepOpen,
}

/// KSG's `resolveAssistantCitationCommentDismissal`: Escape or a missing draft
/// closes without saving, an unchanged draft closes, a too-long draft keeps
/// the popover open, and anything else commits.
pub(crate) fn resolve_dismissal(
    escape: bool,
    draft: Option<&str>,
    saved: Option<&str>,
) -> Dismissal {
    let Some(draft) = draft else {
        return Dismissal::Close;
    };
    if escape || draft.trim() == saved.unwrap_or("").trim() {
        return Dismissal::Close;
    }
    if utf16_len(draft) > ASSISTANT_CITATION_MAX_COMMENT_LENGTH {
        return Dismissal::KeepOpen;
    }
    Dismissal::Commit(draft.to_owned())
}

pub(crate) fn comment_key(context_id: &str) -> String {
    format!("citation-comment-{context_id}")
}

/// The pill's covered text for a citation chip: its label plus the gutter that
/// makes room for the comment action at the chip's right end.
pub(crate) fn chip_text(label: &str) -> String {
    format!("{label}{CHIP_ACTION_GUTTER}")
}

fn response_record<'a>(t: &'a Thread, context_id: &str) -> Option<&'a ComposerContextRecord> {
    t.draft_context
        .iter()
        .find(|r| r.context_id == context_id)
        .filter(|r| matches!(r.data, ComposerContextData::Response { .. }))
}

/// Open the editor, from a chip action press or right after the citation was
/// inserted (`remove_on_cancel` set: KSG opens the editor on cite, and Cancel
/// there undoes the insert).
pub(crate) fn open(
    s: &mut State,
    context_id: &str,
    anchor: String,
    remove_on_cancel: Option<InsertedSpaces>,
) {
    let Some(record) = response_record(s.current(), context_id) else {
        return;
    };
    let ComposerContextData::Response { citation, .. } = &record.data else {
        return;
    };
    let saved = citation.comment.clone();
    let mut draft = TextEdit::new(saved.clone().unwrap_or_default());
    draft.set_caret(draft.text.len());
    s.citation_comment = Some(Editing {
        context_id: context_id.to_owned(),
        anchor,
        draft,
        saved,
        remove_on_cancel,
    });
}

pub(crate) fn open_after_insert(s: &mut State, context_id: &str, spaces: InsertedSpaces) {
    open(s, context_id, comment_key(context_id), Some(spaces));
}

/// Escape or Cancel: KSG never saves, and a just-inserted citation goes away
/// with the spaces it brought (`removeOnCancel`).
pub(crate) fn cancel(s: &mut State) {
    let Some(ed) = s.citation_comment.take() else {
        return;
    };
    if let Some(spaces) = ed.remove_on_cancel {
        remove_citation(s.current_mut(), &ed.context_id, spaces);
        s.checkpoint();
    }
    s.restore_composer_focus(FocusReason::MenuClosed);
}

/// Enter or Save: a too-long draft stays open (the error line and the disabled
/// "Shorten comment" say why); a valid one commits and closes.
pub(crate) fn save(s: &mut State) -> bool {
    let Some(ed) = &s.citation_comment else {
        return false;
    };
    if utf16_len(&ed.draft.text) > ASSISTANT_CITATION_MAX_COMMENT_LENGTH {
        return false;
    }
    let ed = s.citation_comment.take().unwrap();
    commit(s, &ed);
    s.restore_composer_focus(FocusReason::MenuClosed);
    true
}

/// Mod+Enter: save, then send if the composer can (KSG's `onSubmitAndSend`).
pub(crate) fn save_and_send(s: &mut State) {
    if save(s) {
        s.send();
    }
}

/// An outside press follows `resolveAssistantCitationCommentDismissal`.
pub(crate) fn outside_dismiss(s: &mut State) {
    let Some(ed) = &s.citation_comment else {
        return;
    };
    match resolve_dismissal(false, Some(&ed.draft.text), ed.saved.as_deref()) {
        Dismissal::Commit(_) => {
            save(s);
        }
        Dismissal::Close => {
            s.citation_comment = None;
            s.restore_composer_focus(FocusReason::MenuClosed);
        }
        Dismissal::KeepOpen => {}
    }
}

/// KSG's between-open close rules: the record can leave the draft (the pill was
/// deleted or the thread changed) and the cited message can be revised away
/// (`observeAssistantCitationCommentSource`). Runs each frame; returns whether
/// the state changed.
pub(crate) fn settle(s: &mut State) -> bool {
    let Some(ed) = &s.citation_comment else {
        return false;
    };
    let (context_id, draft, saved) = (
        ed.context_id.clone(),
        ed.draft.text.clone(),
        ed.saved.clone(),
    );
    let t = s.current();
    let Some(record) = response_record(t, &context_id) else {
        s.citation_comment = None;
        return true;
    };
    let ComposerContextData::Response { citation, .. } = &record.data else {
        return false;
    };
    if source_available(t, citation) {
        return false;
    }
    match resolve_dismissal(false, Some(&draft), saved.as_deref()) {
        Dismissal::Commit(_) => save(s),
        Dismissal::Close => {
            s.citation_comment = None;
            s.restore_composer_focus(FocusReason::MenuClosed);
            true
        }
        Dismissal::KeepOpen => false,
    }
}

/// KSG watches the cited source's DOM anchor: a citation in another thread has
/// none here and the editor stays open; one in this thread goes when the
/// message or its quoted text does.
fn source_available(t: &Thread, citation: &AssistantCitation) -> bool {
    if citation.thread_id != t.id.to_string() {
        return true;
    }
    let Some(message) = t
        .messages
        .iter()
        .find(|m| m.id == citation.message_id && m.role == "Pi")
    else {
        return false;
    };
    let rendered = markdown_rendered_text(&message.text);
    rendered.contains(&citation.text)
        || normalize_whitespace(&rendered).contains(&normalize_whitespace(&citation.text))
}

/// Write the draft back: trim like `withAssistantCitationComment`, an empty
/// comment removes it, and every chip re-pills since the label previews it.
fn commit(s: &mut State, ed: &Editing) {
    let t = s.current_mut();
    let Some(index) = t
        .draft_context
        .iter()
        .position(|r| r.context_id == ed.context_id)
    else {
        return;
    };
    let covered = {
        let ComposerContextData::Response { citation, .. } = &mut t.draft_context[index].data
        else {
            return;
        };
        *citation = with_assistant_citation_comment(citation, &ed.draft.text);
        let label =
            sanitize_composer_context_label(&assistant_citation_label(citation), "response");
        t.draft_context[index].label = label.clone();
        chip_text(&label)
    };
    let href = format_composer_context_href("response", &ed.context_id);
    let ranges: Vec<Range<usize>> = t
        .draft
        .pills()
        .filter(|(_, id)| *id == href)
        .map(|(range, _)| range)
        .collect();
    t.draft.undo_group(|e| {
        for range in ranges {
            e.replace_with_pill(range, href.clone(), &covered);
        }
    });
    t.update_submission_error();
    s.checkpoint();
}

/// KSG's `removeOnCancel`: the citation goes with the spaces its insert added.
fn remove_citation(t: &mut Thread, context_id: &str, spaces: InsertedSpaces) {
    let href = format_composer_context_href("response", context_id);
    let mut ranges: Vec<Range<usize>> = t
        .draft
        .pills()
        .filter(|(_, id)| *id == href)
        .map(|(range, _)| range)
        .collect();
    ranges.sort_by_key(|range| range.start);
    t.draft.undo_group(|e| {
        for range in ranges.iter().rev() {
            let mut start = range.start;
            let mut end = range.end;
            if spaces.after && e.text[end..].starts_with(' ') {
                end += 1;
            }
            if spaces.before && e.text[..start].ends_with(' ') {
                start -= 1;
            }
            e.set_selection(start, end);
            e.insert("");
        }
    });
    t.draft_context.retain(|r| r.context_id != context_id);
    t.retained_context.records.remove(context_id);
    t.update_submission_error();
}

/// KSG's `ContextChipAction` pencil on each citation pill, positioned over the
/// pill's reserved right edge with `Cx::input_pill_range_rect`.
pub(super) fn chip_actions(s: &State, p: Palette, cx: &Cx<State>, zoom: f32) -> Vec<El> {
    let t = s.current();
    let Some(field) = cx.layout_with("composer-input", |r| r) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for record in &t.draft_context {
        let ComposerContextData::Response { citation, .. } = &record.data else {
            continue;
        };
        let href = format_composer_context_href("response", &record.context_id);
        let mut occurrence = 0usize;
        for (range, id) in t.draft.pills() {
            if id != href {
                continue;
            }
            occurrence += 1;
            let Some(rect) = cx.input_pill_range_rect("composer-input", &href, range) else {
                continue;
            };
            // A pill scrolled out of the field's visible box keeps no action.
            if rect[1] + rect[3] < field[1] || rect[1] > field[1] + field[3] {
                continue;
            }
            let key = if occurrence == 1 {
                comment_key(&record.context_id)
            } else {
                format!("{}-{occurrence}", comment_key(&record.context_id))
            };
            let has_comment = citation.comment.is_some();
            let (context_id, anchor) = (record.context_id.clone(), key.clone());
            out.push(
                el().key(key)
                    .role("button")
                    .label(if has_comment {
                        "Edit citation comment"
                    } else {
                        "Add comment to citation"
                    })
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .preserve_focus_on_press(true)
                    .keeps_text_selection()
                    .pin(
                        Some((rect[0] + rect[2] - ACTION_SLOT) / zoom),
                        Some(rect[1] / zoom),
                        None,
                        None,
                    )
                    .size(ACTION_SLOT, rect[3] / zoom)
                    .radius(4.0)
                    .hover(p.selected)
                    .center()
                    .on_press(move |s: &mut State| {
                        open(s, &context_id, anchor.clone(), None);
                    })
                    .child(icon("pencil", if has_comment { p.coral } else { p.muted })),
            );
        }
    }
    out
}

/// The comment editor card: KSG's `PopoverPopup` (`width="md"`,
/// `padding="compact"`) above the chip — the field, the over-limit line and
/// the Cancel/Save row.
pub(super) fn popover(s: &State, p: Palette) -> Option<El> {
    let ed = s.citation_comment.as_ref()?;
    let too_long = utf16_len(&ed.draft.text) > ASSISTANT_CITATION_MAX_COMMENT_LENGTH;
    let field = text_input(&ed.draft, TextStyle::new(14.0, 400.0))
        .key("citation-comment-input")
        .label("Comment on selected text")
        .help("Enter to save the citation comment; Command/Ctrl+Enter to save and send; Shift+Enter for a new line.")
        .placeholder("Add an optional comment...", p.muted)
        .color(p.ink)
        .w_full()
        .min_h(64.0)
        .max_h(160.0)
        .px(4.0)
        .py(6.0)
        .autofocus()
        .on_edit(|s: &mut State| &mut s.citation_comment.as_mut().unwrap().draft)
        .on_input_key(|s: &mut State, event, _| match event.key {
            rune::Key::Escape => {
                cancel(s);
                true
            }
            rune::Key::Enter if !event.mods.shift => {
                if event.mods.primary() {
                    save_and_send(s);
                } else {
                    save(s);
                }
                true
            }
            _ => false,
        });
    let mut card = col()
        .key("citation-comment-popover")
        .role("dialog")
        .label("Edit citation comment")
        .w(320.0)
        .px(12.0)
        .py(8.0)
        .radius(6.0)
        .clip_rounded(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .anchored_fit(&ed.anchor, rune::Side::Top, rune::Align::End, 4.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| outside_dismiss(s))
        .on_key(|s: &mut State, event, _| {
            if event.key == rune::Key::Escape {
                cancel(s);
                true
            } else {
                false
            }
        })
        .child(field);
    if too_long {
        card = card.child(
            label(
                "Comments can contain up to 8,000 characters.",
                SMALL,
                p.danger,
            )
            .pt(4.0),
        );
    }
    Some(
        card.child(
            row()
                .w_full()
                .justify_end()
                .gap(8.0)
                .mt(8.0)
                .child(
                    row()
                        .key("citation-comment-cancel")
                        .role("button")
                        .label("Cancel")
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .preserve_focus_on_press(true)
                        .center_y()
                        .h(28.0)
                        .px(10.0)
                        .radius(4.0)
                        .hover(p.selected)
                        .press(p.selected)
                        .on_press(|s: &mut State| cancel(s))
                        .child(label("Cancel", TextStyle::new(12.0, 450.0), p.ink)),
                )
                .child(save_button(p, too_long)),
        ),
    )
}

/// KSG's save button: `framed` since it acts, disabled and relabeled "Shorten
/// comment" while the draft is over the limit.
fn save_button(p: Palette, too_long: bool) -> El {
    let title = if too_long { "Shorten comment" } else { "Save" };
    let mut button = row()
        .key("citation-comment-save")
        .role("button")
        .label(title)
        .center_y()
        .h(28.0)
        .px(10.0)
        .radius(4.0);
    if too_long {
        button = button
            .fill(p.selected)
            .child(label(title, TextStyle::new(12.0, 600.0), p.muted));
    } else {
        button = button
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .preserve_focus_on_press(true)
            .fill(p.coral)
            .press(p.coral.alpha(0.8))
            .on_press(|s: &mut State| {
                save(s);
            })
            .child(label(title, TextStyle::new(12.0, 600.0), p.on_coral));
    }
    framed(button, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dismissal_covers_ksg_branches() {
        // Escape or a missing draft always closes without saving.
        assert_eq!(
            resolve_dismissal(true, Some("changed"), None),
            Dismissal::Close
        );
        assert_eq!(resolve_dismissal(false, None, None), Dismissal::Close);
        // An unchanged draft (trim-insensitive) closes.
        assert_eq!(
            resolve_dismissal(false, Some("  hi  "), Some("hi")),
            Dismissal::Close
        );
        assert_eq!(
            resolve_dismissal(false, Some("   "), None),
            Dismissal::Close
        );
        // Over the limit the popover stays.
        let long = "x".repeat(ASSISTANT_CITATION_MAX_COMMENT_LENGTH + 1);
        assert_eq!(
            resolve_dismissal(false, Some(&long), None),
            Dismissal::KeepOpen
        );
        // A changed, valid draft commits verbatim (trimming happens on commit).
        assert_eq!(
            resolve_dismissal(false, Some("  note  "), None),
            Dismissal::Commit("  note  ".into())
        );
        assert_eq!(
            resolve_dismissal(false, Some("new"), Some("old")),
            Dismissal::Commit("new".into())
        );
    }

    #[test]
    fn comment_limit_counts_utf16_at_the_boundary() {
        // KSG compares `comment.length`, UTF-16 code units: "😀" counts 2.
        let at_limit = "a".repeat(ASSISTANT_CITATION_MAX_COMMENT_LENGTH - 2) + "😀";
        assert_eq!(utf16_len(&at_limit), ASSISTANT_CITATION_MAX_COMMENT_LENGTH);
        assert_eq!(
            resolve_dismissal(false, Some(&at_limit), None),
            Dismissal::Commit(at_limit)
        );
        let over = "a".repeat(ASSISTANT_CITATION_MAX_COMMENT_LENGTH - 1) + "😀";
        assert_eq!(
            resolve_dismissal(false, Some(&over), None),
            Dismissal::KeepOpen
        );
    }

    #[test]
    fn committing_trims_and_drops_an_empty_comment() {
        let mut citation = AssistantCitation {
            version: 1,
            environment_id: "local".into(),
            thread_id: "1".into(),
            message_id: "m1".into(),
            text: "quoted".into(),
            comment: Some("old".into()),
            start: 0,
            end: 6,
            prefix: String::new(),
            suffix: String::new(),
        };
        let next = with_assistant_citation_comment(&citation, "  new note  ");
        assert_eq!(next.comment.as_deref(), Some("new note"));
        let cleared = with_assistant_citation_comment(&next, "   ");
        assert_eq!(cleared.comment, None);
        citation.comment = None;
        let untouched = with_assistant_citation_comment(&citation, "");
        assert_eq!(untouched.comment, None);
    }

    #[test]
    fn draft_snapshot_round_trip_keeps_the_comment() {
        // Drafts, stashes and queued-message edits all serialize
        // `Snapshot.context`; the record carries `citation.comment` through.
        let record = ComposerContextRecord {
            version: 1,
            context_id: "ctx_r".into(),
            label: "quoted".into(),
            data: ComposerContextData::Response {
                citation: AssistantCitation {
                    version: 1,
                    environment_id: "local".into(),
                    thread_id: "1".into(),
                    message_id: "m1".into(),
                    text: "quoted".into(),
                    comment: Some("why this".into()),
                    start: 0,
                    end: 6,
                    prefix: String::new(),
                    suffix: String::new(),
                },
                item_id: "m1".into(),
                subagents: None,
            },
        };
        let mut draft = TextEdit::new("See ");
        draft.set_caret(draft.text.len());
        draft.insert_pill(
            format_composer_context_href("response", "ctx_r"),
            &chip_text("quoted"),
        );
        let snapshot = crate::composer_rich::Snapshot::capture_with_context(&draft, &[], &[record]);
        let restored: crate::composer_rich::Snapshot =
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        let ComposerContextData::Response { citation, .. } = &restored.context[0].data else {
            panic!("response record survives the round trip");
        };
        assert_eq!(citation.comment.as_deref(), Some("why this"));
        let restored_draft = restored.editor();
        assert_eq!(restored_draft.pills().count(), 1);
    }
}
