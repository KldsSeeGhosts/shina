//! Captain document surfaces in the right panel: a `captain-doc:{captain}:{name}` tab shows
//! the document from the book's projection and hosts its edit draft. Drafts live in
//! `State::captain_drafts`, keyed by server, captain and name, and only an exact accepted
//! `captain.document.write` receipt clears a Saving one.
//!
//! The panel's structure follows the KSG `RightPanelSurfaceContent` bodies: a header row with
//! the document's title and actions, then the scrollable body. Skin and tokens are Pixel
//! Voyage's.
use super::captain_ui::quiet_note;
use super::crew_ui::{ghost_button, hairline, solid_button};
use super::markdown_ui;
use super::*;
use crate::model::CaptainAccess;
use crate::model::{DiscardKind, DocDiscard, DocumentDraft, DocumentDraftKey, DraftState};

/// The document's hard size cap, shared with the context budget a whole document can cost.
const TEXT_CAP: usize = 64 * 1024;

const META: TextStyle = TextStyle::new(11.5, 430.0).lh(16.0);

/// What each crew reads from the document every turn: the captain reads AGENTS.md and
/// NOTES.md itself, CREW.md goes to crew threads, the digest is read on demand.
const CONTEXT_BYTES: usize = 8 * 1024;

/// The friendly labels from the design: AGENTS.md is the captain's rules, NOTES.md its plan,
/// CREW.md the crew's shared rules, DIGEST.md the digest it writes on demand.
pub(super) fn document_title(name: &str) -> &'static str {
    match name {
        "AGENTS.md" => "Captain rules",
        "NOTES.md" => "Plan",
        "CREW.md" => "Crew rules",
        "DIGEST.md" => "Digest",
        _ => "Document",
    }
}

/// Who the document is shared with, from `captain_mcp`'s read rules.
pub(super) fn sharing_note(name: &str) -> &'static str {
    match name {
        "AGENTS.md" => "Only this captain reads these. Crew threads never see them.",
        "NOTES.md" => "Shared with crew threads.",
        "CREW.md" => "Every crew thread reads these.",
        "DIGEST.md" => "Read on demand.",
        _ => "",
    }
}

fn empty_note(name: &str, captain_name: &str) -> String {
    match name {
        "AGENTS.md" => format!(
            "No rules yet. Standing instructions for {captain_name}'s coordination go here."
        ),
        "NOTES.md" => format!("No plan yet. {captain_name} keeps its working plan here."),
        "CREW.md" => "No crew rules yet. Instructions every crew thread follows go here.".into(),
        "DIGEST.md" => {
            format!("No digest yet. {captain_name} writes a summary of its crew's work here.")
        }
        _ => "This document is empty.".into(),
    }
}

fn access_reason(access: CaptainAccess) -> Option<String> {
    match access {
        CaptainAccess::Full => None,
        CaptainAccess::Earlier => Some("Read-only in an earlier conversation.".into()),
        CaptainAccess::UpdateRequired => Some("Update the linked Shina to edit documents.".into()),
        CaptainAccess::Unreachable => Some("The machine is unreachable.".into()),
        CaptainAccess::NoRecords => Some("No captain records.".into()),
    }
}

/// The right-panel body of a Captain document surface: header, then the reading view or
/// the editor for this machine's draft.
pub(super) fn document_panel(s: &State, p: Palette, captain: usize, name: &str) -> El {
    let key = DocumentDraftKey {
        server: s.machines.selected.clone(),
        captain,
        name: name.into(),
    };
    let (_, updated_at, text) = s.captain_document_body(captain, name);
    let captain_name = s
        .captains
        .get(captain)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let access = s.captain_access(captain);
    let draft = s.captain_drafts.get(&key);
    let title = document_title(name);
    let age = match relative_time(updated_at / 1000) {
        Some(age) => format!("Updated {age} ago"),
        None => "Never edited".into(),
    };
    let mut meta = row()
        .gap(8.0)
        .center_y()
        .child(label(name.to_string(), PIXEL_LABEL, p.faint))
        .child(label(age, META, p.muted))
        .child(
            label(sharing_note(name), META, p.muted)
                .truncate()
                .min_w(0.0),
        );
    if name == "AGENTS.md" {
        meta = el()
            .children([
                outlined(
                    el().h(16.0).px(5.0).center().radius(3.0).child(label(
                        "Private",
                        PIXEL_LABEL,
                        p.muted,
                    )),
                    p,
                ),
                meta,
            ])
            .gap(6.0)
            .center_y();
    }
    let mut header = row()
        .key("captain-doc-header")
        .px(10.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(label(title, DIALOG_TITLE, p.ink).truncate())
                .child(meta),
        );
    if draft.is_none() && access == CaptainAccess::Full {
        let name_owned = name.to_string();
        header = header.child(ghost_button(
            s,
            "captain-doc-edit".into(),
            "Edit",
            p,
            move |s: &mut State| {
                s.captain_document_begin_edit(captain, &name_owned);
            },
        ));
    }
    let mut body = col()
        .key("captain-doc-panel")
        .role("region")
        .label(format!("{title}, {captain_name}"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(header)
        .child(el().h(1.0).fill(hairline(p)));
    if let Some(reason) = access_reason(access) {
        body = body.child(quiet_note(reason, p).key("captain-doc-readonly"));
    }
    if let Some(draft) = draft {
        body = body.child(editor_body(s, p, &key, draft));
    } else {
        body = body.child(reading_body(s, p, captain, name, text, &captain_name));
    }
    body
}

/// The saved text, rendered as markdown in the panel's reading lane.
fn reading_body(
    s: &State,
    p: Palette,
    captain: usize,
    name: &str,
    text: &str,
    captain_name: &str,
) -> El {
    let mut body = col()
        .key("captain-doc-reading")
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .px(12.0)
        .py(10.0);
    if text.is_empty() {
        body = body.child(quiet_note(empty_note(name, captain_name), p));
    } else {
        body = body.child(markdown_ui::render_with_find(
            text,
            p,
            s.dark,
            false,
            false,
            false,
            "captain-doc-markdown",
            None,
        ));
    }
    let _ = captain;
    body
}

/// The edit view: a state banner, the mono source field (a selectable copy while Saving)
/// and the footer with the byte count, context note and Save/Cancel.
fn editor_body(s: &State, p: Palette, key: &DocumentDraftKey, draft: &DocumentDraft) -> El {
    let revision = s.captain_document_body(key.captain, &key.name).0;
    let saving = matches!(draft.state, DraftState::Saving { .. });
    let machine = machine_label(s, &key.server);
    let newest = match draft.state {
        DraftState::Conflict { theirs_revision } => theirs_revision.max(revision),
        _ => revision,
    };
    let theirs_newer = newest > draft.base_revision;
    let conflict = !saving && (theirs_newer || matches!(draft.state, DraftState::Conflict { .. }));
    let dirty = draft.text.text != draft.base_text;
    let bytes = draft.text.text.len();
    let oversized = bytes > TEXT_CAP;
    let has_nul = draft.text.text.contains('\0');
    let mut body = col().grow(1.0).min_h(0.0);
    if let DraftState::Rejected { message } = &draft.state {
        body = body.child(error_row("captain-doc-error", message, p));
    }
    if conflict {
        let captain_name = s
            .captains
            .get(key.captain)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "The captain".into());
        let mut row_ = row()
            .key("captain-doc-conflict")
            .role("alert")
            .mx(12.0)
            .mt(10.0)
            .px(10.0)
            .py(8.0)
            .gap(8.0)
            .radius(4.0)
            .border(2.0, p.brass)
            .fill(p.brass.alpha(0.12))
            .child(icon("alert", p.brass))
            .child(
                label(
                    format!(
                        "{captain_name} or another window changed {} after you started editing. Your draft is kept.",
                        document_title(&key.name)
                    ),
                    TextStyle::new(11.5, 430.0),
                    p.ink,
                )
                .wrap()
                .grow(1.0)
                .min_w(0.0),
            );
        let seen = draft.show_theirs == Some(revision);
        let key2 = key.clone();
        row_ = row_.child(
            ghost_button(
                s,
                "captain-doc-conflict-theirs".into(),
                if seen {
                    "Hide their version"
                } else {
                    "Show their version"
                },
                p,
                move |s: &mut State| {
                    if let Some(draft) = s.captain_drafts.get_mut(&key2) {
                        draft.show_theirs = if draft.show_theirs == Some(revision) {
                            None
                        } else {
                            Some(revision)
                        };
                    }
                },
            )
            .h(26.0),
        );
        body = body.child(row_);
    }
    if oversized || has_nul {
        body = body.child(error_row(
            "captain-doc-invalid",
            &if oversized {
                "Over the 64 KiB limit. Shorten it to save.".to_string()
            } else {
                "The document contains a null character.".to_string()
            },
            p,
        ));
    }
    if draft.show_theirs == Some(revision) {
        let (_, _, theirs) = s.captain_document_body(key.captain, &key.name);
        body = body.child(
            col()
                .key("captain-doc-theirs-view")
                .max_h(s.viewport_height * 0.4)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .px(12.0)
                .py(10.0)
                .child(markdown_ui::render_with_find(
                    theirs,
                    p,
                    s.dark,
                    false,
                    false,
                    false,
                    "captain-doc-theirs-markdown",
                    None,
                )),
        );
    }
    let source_style = TextStyle::new(12.0, 420.0)
        .lh(18.0)
        .family(rune::Family::SystemMono);
    if saving {
        // Read-only selectable source: the pending receipt owns the draft's outcome.
        body = body.child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .px(12.0)
                .py(10.0)
                .child(
                    label(draft.text.text.clone(), source_style, p.ink)
                        .key("captain-doc-text")
                        .wrap()
                        .selectable(true)
                        .w_full(),
                ),
        );
    } else {
        let key2 = key.clone();
        let editor = rune::auto_grow(
            text_input(&draft.text, source_style)
                .key("captain-doc-text")
                .label(format!("{} contents", key.name))
                .color(p.ink)
                .w_full()
                .on_submit({
                    let key = key.clone();
                    move |s: &mut State, _| {
                        if let Some(draft) = s.captain_drafts.get_mut(&key) {
                            draft.text.insert("\n");
                        }
                    }
                })
                .on_edit(rune::edit_access(move |s: &mut State| {
                    &mut s
                        .captain_drafts
                        .get_mut(&key2)
                        .expect("open document draft")
                        .text
                })),
            6,
            24,
        );
        body = body.child(editor.px(12.0).py(10.0).grow(1.0).min_h(0.0));
    }
    // The footer names the disable reason, so pointer and keyboard users see the same text.
    let save_block = if saving {
        Some(format!("Waiting for {machine} to confirm."))
    } else if let Some(reason) = access_reason(s.captain_access(key.captain)) {
        Some(reason)
    } else if oversized {
        Some("Over the 64 KiB limit. Shorten it to save.".into())
    } else if has_nul {
        Some("The document contains a null character.".into())
    } else if conflict {
        Some("Choose how to handle the newer version first.".into())
    } else if !dirty {
        Some("No changes to save.".into())
    } else {
        None
    };
    let context_note = match key.name.as_str() {
        "DIGEST.md" => None,
        "CREW.md" => Some(format!(
            "Crew threads read the first {} KiB each turn and the rest on request.",
            CONTEXT_BYTES / 1024
        )),
        _ => Some(format!(
            "The captain reads the first {} KiB each turn and the rest on request.",
            CONTEXT_BYTES / 1024
        )),
    };
    let mut footer = row()
        .key("captain-doc-footer")
        .px(12.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(
            label(
                format!(
                    "{:.1} KiB of {} KiB",
                    bytes as f32 / 1024.0,
                    TEXT_CAP / 1024
                ),
                PIXEL_LABEL,
                if oversized { p.danger } else { p.muted },
            )
            .key("captain-doc-bytes"),
        );
    if let Some(reason) = &save_block {
        footer = footer.child(
            label(
                reason,
                META,
                if oversized || has_nul {
                    p.danger
                } else {
                    p.muted
                },
            )
            .key("captain-doc-status")
            .truncate()
            .shrink()
            .min_w(0.0),
        );
    } else if saving {
        footer = footer
            .child(label(format!("Saving to {machine}"), META, p.muted).key("captain-doc-status"));
    } else if let Some(note) = context_note {
        footer = footer.child(
            label(note, META, p.muted)
                .key("captain-doc-status")
                .truncate()
                .min_w(0.0),
        );
    }
    footer = footer.child(spacer());
    let cancel_key = key.clone();
    let panel_key = s.panel_key();
    let surface_id = format!("captain-doc:{}:{}", key.captain, key.name);
    let mut cancel = ghost_button(
        s,
        "captain-doc-cancel".into(),
        "Cancel",
        p,
        move |s: &mut State| {
            s.request_close_document(&cancel_key, &panel_key, &surface_id);
        },
    )
    .h(26.0)
    .disabled(saving);
    if saving {
        cancel = cancel.help(format!("Waiting for {machine} to confirm."));
    }
    footer = footer.child(cancel);
    if conflict {
        let seen = draft.show_theirs == Some(revision);
        let replace_key = key.clone();
        let mut replace = ghost_button(
            s,
            "captain-doc-conflict-replace".into(),
            "Replace with mine",
            p,
            move |s| {
                let _ = s.captain_document_overwrite_reviewed(&replace_key, revision);
            },
        )
        .h(26.0)
        .disabled(!seen);
        if !seen {
            replace = replace.help("Show their version first.");
        }
        footer = footer.child(replace);
        let discard_key = key.clone();
        let panel = s.panel_key().clone();
        let surface = format!("captain-doc:{}:{}", key.captain, key.name);
        footer = footer.child(
            ghost_button(
                s,
                "captain-doc-conflict-discard".into(),
                "Discard mine",
                p,
                move |s| {
                    s.doc_discard = Some(DocDiscard {
                        panel: panel.clone(),
                        surface: surface.clone(),
                        key: discard_key.clone(),
                        kind: DiscardKind::Discard,
                    });
                },
            )
            .h(26.0),
        );
    } else {
        let save_key = key.clone();
        let mut save = solid_button("captain-doc-save".into(), "Save", p, move |s| {
            s.captain_document_save_draft(&save_key);
        })
        .h(26.0)
        .disabled(save_block.is_some() || conflict);
        if let Some(reason) = &save_block {
            save = save.help(reason.clone());
        }
        footer = footer.child(save);
    }
    body.child(el().h(1.0).fill(hairline(p))).child(footer)
}

fn machine_label(s: &State, server: &str) -> String {
    if server == "local" {
        return "this machine".into();
    }
    s.peers
        .iter()
        .find(|peer| peer.server_id == server)
        .map(|peer| peer.label.clone())
        .unwrap_or_else(|| server.to_string())
}

fn error_row(key: &str, message: &str, p: Palette) -> El {
    row()
        .key(key)
        .role("alert")
        .mx(12.0)
        .mt(10.0)
        .px(10.0)
        .py(8.0)
        .gap(8.0)
        .radius(4.0)
        .fill(p.danger.alpha(0.08))
        .child(icon("alert", p.danger))
        .child(
            label(message, TextStyle::new(11.5, 430.0), p.danger)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}

/// The discard confirmation shared by dirty closes and "Discard mine". It stays bound to
/// the server that asked: hidden on other hosts, and confirm only drops the original key.
pub(super) fn discard_dialog(s: &State, p: Palette) -> Option<El> {
    let discard = s.doc_discard.as_ref()?;
    if discard.key.server != s.machines.selected {
        return None;
    }
    let title = format!(
        "Discard your changes to {}?",
        document_title(&discard.key.name)
    );
    let card = col()
        .key("captain-doc-discard-dialog")
        .role("dialog")
        .label(&title)
        .w(420.0)
        .max_w(s.viewport_width - 32.0)
        .padding(16.0)
        .gap(10.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .on_press(|_: &mut State| {})
        .child(label(title, DIALOG_TITLE, p.ink))
        .child(
            label(
                "Your draft has not been saved.",
                TextStyle::new(12.5, 430.0),
                p.muted,
            )
            .wrap(),
        )
        .child(
            row()
                .gap(8.0)
                .child(spacer())
                .child(secondary_button(
                    "captain-doc-keep",
                    "Keep editing",
                    p,
                    |s| s.doc_discard = None,
                ))
                .child(primary_button(
                    "captain-doc-drop",
                    "Discard changes",
                    p,
                    |s| s.finish_doc_discard(),
                )),
        );
    Some(modal_frame(
        "doc-discard-backdrop",
        s,
        |s| s.doc_discard = None,
        card,
    ))
}
