//! The Inspector's Context tab and its delivery model, Workstream 4 of the
//! wave 2 spec (§4): one place that answers "what goes into the next prompt".
//!
//! `sources` classifies every captain document, the profile and essential
//! memory by what actually reaches a prompt on each audience (captain home
//! versus crew threads), reading the same byte budgets `captain_state` enforces
//! server-side. The tab draws those figures per document, the context window
//! the composer ring already measures, the memory book, the captain's scope
//! and the conversation controls, ported from KSG Code's
//! `chat/ContextWindowMeter.tsx` and the Brief's `ThreadDetailsSection` rows.
use super::*;

use super::crew_ui::{active_wash, ghost_button, hairline, hover_wash, motion, solid_button};
use crate::model::{
    CREW_CONTEXT_MAX_BYTES, ESSENTIAL_BUDGET, MemoryView, NOTES_CONTEXT_MAX_BYTES,
    PROFILE_BUDGET, RULES_CONTEXT_MAX_BYTES,
};

/// KSG's `text-[13px] font-medium` detail rows.
const ROW_TEXT: TextStyle = TextStyle::new(13.0, 500.0).lh(19.0).tracking(-0.08);
const META: TextStyle = TextStyle::new(12.0, 500.0).lh(16.0);
/// The right column of a document row: 132 pt at the panel's 420 pt default,
/// 96 pt at the 360 pt minimum (spec §4.5).
const DOC_META_WIDE: f32 = 132.0;
const DOC_META_NARROW: f32 = 96.0;

/// Which audience's prompt a source's `sent` figure describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Audience {
    Captain,
    Crew,
}

/// How much of a source reaches the prompt on the next turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Delivery {
    /// The whole source enters the prompt every turn.
    Whole,
    /// A bounded prefix enters the prompt; the rest is cut by the byte budget.
    Truncated,
    /// Nothing enters the prompt because the source has no bytes.
    Empty,
    /// Nothing enters the prompt; the document is read only on request.
    OnRequest,
    /// The audience never receives this source.
    NotSent,
}

/// One context source: a captain document, the user profile or essential memory.
#[derive(Clone, Debug)]
pub(super) struct Source {
    /// "AGENTS.md", "NOTES.md", "CREW.md", "DIGEST.md", "profile" or "essential".
    pub key: &'static str,
    /// The friendly title, from `captain_docs_ui::document_title`.
    pub title: String,
    /// Size on disk in bytes.
    pub bytes: usize,
    /// Bytes that enter the prompt for this audience.
    pub sent: usize,
    /// The byte budget that bounds `sent`, where one applies.
    pub budget: Option<usize>,
    pub delivery: Delivery,
    /// AGENTS.md is the captain's private rules; crew threads never see them.
    pub private: bool,
}

/// Bytes a bounded-prefix document actually carries into the prompt. The
/// truncation marker reserves room inside the budget, so a cut document sends
/// fewer bytes than the cap (mirrors `captain_state::covered_bytes`).
fn covered(text_len: usize, budget: usize) -> usize {
    text_len.min(budget)
}

fn document_source(s: &State, captain: usize, name: &'static str, audience: Audience) -> Source {
    let meta = s.captain_document_view(captain, name);
    let title = captain_docs_ui::document_title(name).to_string();
    let (delivery, sent, budget, private) = match (name, audience) {
        ("AGENTS.md", Audience::Captain) => {
            if meta.bytes == 0 {
                (Delivery::Empty, 0, Some(RULES_CONTEXT_MAX_BYTES), true)
            } else if meta.bytes > RULES_CONTEXT_MAX_BYTES {
                (
                    Delivery::Truncated,
                    covered(meta.bytes, RULES_CONTEXT_MAX_BYTES),
                    Some(RULES_CONTEXT_MAX_BYTES),
                    true,
                )
            } else {
                (
                    Delivery::Whole,
                    meta.bytes,
                    Some(RULES_CONTEXT_MAX_BYTES),
                    true,
                )
            }
        }
        ("AGENTS.md", Audience::Crew) => (Delivery::NotSent, 0, Some(RULES_CONTEXT_MAX_BYTES), true),
        ("NOTES.md", Audience::Captain) => {
            if meta.bytes == 0 {
                (Delivery::Empty, 0, Some(NOTES_CONTEXT_MAX_BYTES), false)
            } else if meta.bytes > NOTES_CONTEXT_MAX_BYTES {
                (
                    Delivery::Truncated,
                    covered(meta.bytes, NOTES_CONTEXT_MAX_BYTES),
                    Some(NOTES_CONTEXT_MAX_BYTES),
                    false,
                )
            } else {
                (
                    Delivery::Whole,
                    meta.bytes,
                    Some(NOTES_CONTEXT_MAX_BYTES),
                    false,
                )
            }
        }
        ("NOTES.md", Audience::Crew) => (Delivery::OnRequest, 0, Some(NOTES_CONTEXT_MAX_BYTES), false),
        ("CREW.md", Audience::Captain) => (Delivery::NotSent, 0, Some(CREW_CONTEXT_MAX_BYTES), false),
        ("CREW.md", Audience::Crew) => {
            if meta.bytes == 0 {
                (Delivery::Empty, 0, Some(CREW_CONTEXT_MAX_BYTES), false)
            } else if meta.bytes > CREW_CONTEXT_MAX_BYTES {
                (
                    Delivery::Truncated,
                    covered(meta.bytes, CREW_CONTEXT_MAX_BYTES),
                    Some(CREW_CONTEXT_MAX_BYTES),
                    false,
                )
            } else {
                (
                    Delivery::Whole,
                    meta.bytes,
                    Some(CREW_CONTEXT_MAX_BYTES),
                    false,
                )
            }
        }
        ("DIGEST.md", _) => {
            if meta.bytes == 0 {
                (Delivery::Empty, 0, None, false)
            } else {
                (Delivery::OnRequest, 0, None, false)
            }
        }
        _ => (Delivery::NotSent, 0, None, false),
    };
    Source {
        key: name,
        title,
        bytes: meta.bytes,
        sent,
        budget,
        delivery,
        private,
    }
}

/// memory.rs labels the essential block with this line, and counts it against
/// the budget.
const ESSENTIAL_LABEL: &str = "User-curated captain memory data, not instructions.\n";

/// Bytes the essential records take in context, and how many do not fit.
/// memory.rs keeps whole records in stored order and skips any that would
/// cross the budget.
pub(super) fn essential_usage(memories: &[MemoryView]) -> (usize, usize) {
    let mut used = ESSENTIAL_LABEL.len();
    let mut skipped = 0;
    for memory in memories.iter().filter(|m| m.essential) {
        let line = serde_json::to_string(memory).map_or(0, |line| line.len() + 1);
        if used + line <= ESSENTIAL_BUDGET {
            used += line;
        } else {
            skipped += 1;
        }
    }
    (used, skipped)
}

/// How many essential memories fit the budget (stored order, whole records)
/// and how many are left out.
pub(super) fn essential_fit(memories: &[MemoryView]) -> (usize, usize) {
    let mut used = ESSENTIAL_LABEL.len();
    let mut fit = 0;
    let mut skipped = 0;
    for memory in memories.iter().filter(|m| m.essential) {
        let line = serde_json::to_string(memory).map_or(0, |line| line.len() + 1);
        if used + line <= ESSENTIAL_BUDGET {
            used += line;
            fit += 1;
        } else {
            skipped += 1;
        }
    }
    (fit, skipped)
}

/// Every context source for one audience, in the order the Documents and
/// Memory sections draw them.
pub(super) fn sources(s: &State, captain: usize, audience: Audience) -> Vec<Source> {
    let mut out: Vec<Source> = [
        document_source(s, captain, "AGENTS.md", audience),
        document_source(s, captain, "NOTES.md", audience),
        document_source(s, captain, "CREW.md", audience),
        document_source(s, captain, "DIGEST.md", audience),
    ]
    .into();
    let profile = s.user_profile();
    let profile_bytes = profile_bytes(&profile);
    out.push(match audience {
        Audience::Captain => Source {
            key: "profile",
            title: "You".into(),
            bytes: profile_bytes,
            sent: profile_bytes.min(PROFILE_BUDGET),
            budget: Some(PROFILE_BUDGET),
            delivery: if profile_bytes == 0 {
                Delivery::Empty
            } else {
                Delivery::Whole
            },
            private: true,
        },
        Audience::Crew => Source {
            key: "profile",
            title: "You".into(),
            bytes: profile_bytes,
            sent: 0,
            budget: Some(PROFILE_BUDGET),
            delivery: Delivery::NotSent,
            private: true,
        },
    });
    let memories = s.memories(captain);
    let (used, _) = essential_usage(&memories);
    out.push(match audience {
        Audience::Captain => Source {
            key: "essential",
            title: "Essential memory".into(),
            bytes: used,
            sent: used,
            budget: Some(ESSENTIAL_BUDGET),
            delivery: if used == 0 { Delivery::Empty } else { Delivery::Whole },
            private: true,
        },
        Audience::Crew => Source {
            key: "essential",
            title: "Essential memory".into(),
            bytes: used,
            sent: 0,
            budget: Some(ESSENTIAL_BUDGET),
            delivery: Delivery::NotSent,
            private: true,
        },
    });
    out
}

/// "about 600 tokens": two significant digits of the sent-byte estimate.
fn about_tokens(sent: usize) -> String {
    let tokens = sent as f64 / crate::context_breakdown::CHARS_PER_TOKEN;
    let rounded = if tokens >= 100.0 {
        (tokens / 10.0).round() * 10.0
    } else {
        tokens.round()
    };
    format!("about {} tokens", rounded as u64)
}

fn kib(bytes: usize) -> String {
    format!("{:.1}", bytes as f32 / 1024.0)
}

/// Open a Captain document in the Context tab's viewer, closing a covering
/// panel first where needed. From an earlier conversation of that captain the
/// document opens on this thread's panel, read-only.
pub(super) fn open_captain_document(s: &mut State, captain: usize, name: &str) {
    let earlier = s.captain_access(captain) == crate::model::CaptainAccess::Earlier;
    let key = if earlier {
        s.panel_key()
    } else {
        match s.captain_home(captain) {
            Some(home) => crate::right_panel::key(&s.machines.selected, home),
            None => return,
        }
    };
    s.open_captain_document(&key, captain, name);
}

/// The audience switch over the Documents rows on a captain home: the same
/// segmented control as the report policy (`radiogroup`, arrows move focus
/// and Space or Enter selects).
fn audience_row(s: &State, p: Palette, captain: usize) -> El {
    let name = s
        .captains
        .get(captain)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "the captain".into());
    let crew = s.context_audience_crew;
    const KEYS: [&str; 2] = ["context-audience-captain", "context-audience-crew"];
    let mut group = row()
        .role("radiogroup")
        .focus_group()
        .label("Context audience")
        .padding(2.0)
        .gap(2.0)
        .radius(8.0)
        .fill(p.ink.alpha(if s.dark { 0.07 } else { 0.06 }))
        .border(1.0, p.line)
        .h(28.0)
        .shrink()
        .on_key(|s: &mut State, ev: &rune::KeyEvent, tx: &mut Tx| {
            let index = |focused: Option<&str>| KEYS.iter().position(|key| Some(*key) == focused);
            let step = match ev.key {
                rune::Key::Left | rune::Key::Up => -1i64,
                rune::Key::Right | rune::Key::Down => 1,
                rune::Key::Enter => {
                    if let Some(index) = index(tx.focused()) {
                        s.context_audience_crew = index == 1;
                    }
                    return true;
                }
                rune::Key::Text(ref text) if text == " " => {
                    if let Some(index) = index(tx.focused()) {
                        s.context_audience_crew = index == 1;
                    }
                    return true;
                }
                _ => return false,
            };
            let next = (index(tx.focused()).unwrap_or(0) as i64 + step)
                .rem_euclid(KEYS.len() as i64) as usize;
            tx.focus(KEYS[next], true);
            true
        });
    for (index, key) in KEYS.iter().enumerate() {
        let is_crew = index == 1;
        let on = crew == is_crew;
        let word = if is_crew { "Crew threads".to_string() } else { name.clone() };
        group = group.child(
            row()
                .key(*key)
                .role("radio")
                .checked(on)
                .label(if is_crew {
                    "Sent on the next turn to crew threads".to_string()
                } else {
                    format!("Sent on the next turn to {name}")
                })
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .h(24.0)
                .px(8.0)
                .center()
                .radius(8.0)
                .fill(if on { p.card } else { Color::TRANSPARENT })
                .border(1.0, if on { p.frame } else { Color::TRANSPARENT })
                .hover(if on { p.card } else { hover_wash(s, p) })
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| s.context_audience_crew = is_crew)
                .child(label(word, META, if on { p.ink } else { p.muted })),
        );
    }
    row()
        .key("context-audience")
        .h(40.0)
        .px(16.0)
        .center_y()
        .gap(8.0)
        .child(meta_label("Sent on the next turn to", p.muted))
        .child(spacer())
        .child(group)
}

/// The state line and mini meter a document row's right column draws, by
/// delivery class (spec §4.3's table).
fn delivery_column(source: &Source, name: &str, narrow: bool, p: Palette) -> El {
    let mut col_ = col()
        .w(if narrow { DOC_META_NARROW } else { DOC_META_WIDE })
        .min_w(if narrow { DOC_META_NARROW } else { DOC_META_WIDE })
        .gap(4.0)
        .items_end();
    let size = match source.delivery {
        // "8.0 of 14.2 KiB": sent of total, so a cut document reads honestly.
        Delivery::Whole | Delivery::Truncated => {
            format!("{} of {} KiB", kib(source.sent), kib(source.bytes))
        }
        _ => format!("{} KiB", kib(source.bytes)),
    };
    match source.delivery {
        Delivery::Whole => {
            let fraction = source
                .budget
                .map(|budget| (source.sent as f32 / budget as f32).clamp(0.0, 1.0))
                .unwrap_or(1.0);
            col_ = col_
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            row()
                                .w(40.0)
                                .h(4.0)
                                .radius(2.0)
                                .clip_rounded(2.0)
                                .fill(p.line)
                                .child(el().w_frac(fraction).h_full().radius(2.0).fill(p.sea)),
                        )
                        .child(label(size, TIMER, p.muted).key(format!("context-doc-size-{name}"))),
                )
                .child(
                    label(about_tokens(source.sent), META, p.faint)
                        .key(format!("context-doc-state-{name}")),
                );
        }
        Delivery::Truncated => {
            col_ = col_
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            row()
                                .w(40.0)
                                .h(4.0)
                                .radius(2.0)
                                .clip_rounded(2.0)
                                .fill(p.line)
                                .child(el().w_frac(1.0).h_full().radius(2.0).fill(p.brass)),
                        )
                        .child(label(size, TIMER, p.muted).key(format!("context-doc-size-{name}"))),
                )
                .child(
                    label("Truncated", META, p.brass).key(format!("context-doc-state-{name}")),
                );
        }
        Delivery::Empty => {
            col_ = col_
                .child(row().h(4.0))
                .child(label("Empty", META, p.faint).key(format!("context-doc-state-{name}")));
        }
        Delivery::OnRequest => {
            col_ = col_
                .child(label(size, TIMER, p.muted).key(format!("context-doc-size-{name}")))
                .child(
                    label("Read on request", META, p.faint)
                        .key(format!("context-doc-state-{name}")),
                );
        }
        Delivery::NotSent => {
            col_ = col_.child(row().h(4.0)).child(
                label(
                    if source.private {
                        "Private, never sent"
                    } else {
                        "Not sent"
                    },
                    META,
                    p.faint,
                )
                .key(format!("context-doc-state-{name}")),
            );
        }
    }
    col_
}

/// One document row: icon, title (+Private badge), file/revision line, then
/// the delivery column. Press opens the document in the tab's viewer.
fn document_row(s: &State, p: Palette, captain: usize, source: &Source, narrow: bool) -> El {
    let name = source.key;
    let captain_name = s
        .captains
        .get(captain)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "the captain".into());
    let meta = s.captain_document_view(captain, name);
    let draft_key = crate::model::DocumentDraftKey {
        server: s.machines.selected.clone(),
        captain,
        name: name.to_string(),
    };
    let draft = s.captain_drafts.get(&draft_key);
    let saving = draft.is_some_and(|d| matches!(d.state, crate::model::DraftState::Saving { .. }));
    let dirty = draft.is_some_and(|d| {
        !matches!(d.state, crate::model::DraftState::Saving { .. }) && d.text.text != d.base_text
    });
    let mut file_line = row().gap(6.0).center_y().min_w(0.0).child(label(
        format!(
            "{name}, rev {}{}",
            meta.revision,
            relative_time(meta.updated_at / 1000)
                .map(|age| format!(", {age} ago"))
                .unwrap_or_default()
        ),
        SMALL,
        p.faint,
    ));
    if dirty {
        file_line = file_line.child(
            el().size(6.0, 6.0)
                .fill(p.coral)
                .key(format!("brief-document-draft-{name}")),
        );
    }
    if saving {
        file_line = file_line.child(label("Saving", META_LABEL, p.muted));
    }
    let mut title_line = row()
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .child(label(&source.title, ROW_TEXT, p.ink).truncate().min_w(0.0));
    if source.private {
        title_line = title_line.child(outlined(
            el().h(16.0)
                .px(5.0)
                .center()
                .radius(6.0)
                .child(label("Private", META_LABEL, p.muted)),
            p,
        ));
    }
    let expected = s.machines.selected.clone();
    let mut row_ = row()
        .key(format!("context-doc-{name}"))
        .role("button")
        .label(format!("{}, {name}", source.title))
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .min_h(44.0)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(6.0)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .on_press(move |s: &mut State| {
            if s.machines.selected == expected {
                open_captain_document(s, captain, name);
            }
        })
        .child(
            icon(if source.private { "lock" } else { "file" }, p.muted).size(16.0, 16.0),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(title_line)
                .child(file_line),
        )
        .child(delivery_column(source, name, narrow, p));
    if source.delivery == Delivery::Truncated {
        row_ = row_.help(format!(
            "{captain_name} gets the first {} KiB each turn and can read the rest on request.",
            kib(source.budget.unwrap_or(0))
        ));
    }
    row_
}

/// The Documents section: one `Source` row per captain document.
pub(super) fn documents_section(s: &State, p: Palette, captain: usize, audience: Audience) -> El {
    let narrow = s.right_panel_width <= 380.0;
    let mut rows = col().gap(2.0);
    for source in sources(s, captain, audience)
        .into_iter()
        .filter(|source| source.key.ends_with(".md"))
    {
        rows = rows.child(document_row(s, p, captain, &source, narrow));
    }
    captain_ui::section("context-documents", "Documents", None, vec![], p).child(rows)
}

/// How much of the essential budget is in use, as KSG's `ContextWindowMeter` bar.
fn budget_meter(memories: &[MemoryView], p: Palette) -> El {
    let (used, skipped) = essential_usage(memories);
    let fraction = (used as f32 / ESSENTIAL_BUDGET as f32).clamp(0.0, 1.0);
    let ink = if skipped > 0 {
        p.danger
    } else if fraction > 0.85 {
        p.brass
    } else {
        p.sea
    };
    let summary = format!("{} of 8 KiB", kib(used));
    let mut meter = col()
        .key("memory-budget")
        .role("meter")
        .label(format!("Essential memory uses {summary} of context"))
        .px(10.0)
        .pt(2.0)
        .pb(6.0)
        .gap(6.0)
        .child(
            row()
                .center_y()
                .child(label("Essential, always in context", META, p.muted))
                .child(spacer())
                .child(label(summary, TIMER, p.muted)),
        )
        .child(
            row()
                .h(4.0)
                .w_full()
                .radius(4.0)
                .clip_rounded(4.0)
                .fill(p.ink.alpha(0.08))
                .child(el().h(4.0).grow(fraction).min_w(0.0).fill(ink))
                .child(el().h(4.0).grow(1.0 - fraction).min_w(0.0)),
        );
    if skipped > 0 {
        meter = meter.child(
            label(
                format!(
                    "{skipped} essential {} past the limit and left out of context.",
                    if skipped == 1 {
                        "memory is"
                    } else {
                        "memories are"
                    }
                ),
                META,
                p.danger,
            )
            .wrap(),
        );
    }
    meter
}

/// One quiet line about learned memory (Honcho). Nothing shows while it is
/// disabled; the dot is `sea` connected and `danger` offline.
fn memory_backend_status(s: &State) -> Option<(String, bool)> {
    use crate::model::MemoryBackendStatus as M;
    let queued = |n: usize| {
        if n == 0 {
            String::new()
        } else {
            format!(", {n} waiting to sync")
        }
    };
    match s.memory_backend_status() {
        M::Disabled => None,
        M::Connected {
            queued_count,
            deriver_pending,
        } => Some((
            format!(
                "Learned memory: Honcho connected{}{}",
                match deriver_pending {
                    Some(n) if n > 0 => format!(", learning from {n} new messages"),
                    _ => String::new(),
                },
                queued(queued_count)
            ),
            true,
        )),
        M::Offline {
            last_error,
            queued_count,
        } => Some((
            format!("Learned memory offline{}. {last_error}", queued(queued_count)),
            false,
        )),
    }
}

fn memory_input(p: Palette, edit: &rune::TextEdit, key: String, title: &str) -> El {
    text_input(edit, ROW_TEXT)
        .key(key)
        .label(title)
        .placeholder("Something this captain should always know", p.muted)
        .color(p.ink)
        .w_full()
        .min_h(56.0)
        .max_h(140.0)
        .px(10.0)
        .py(8.0)
        .radius(12.0)
        .fill(p.card)
        .field_focus(edit, p)
}

/// Keep the last rejection on screen; a success clears it.
fn report<T>(s: &mut State, result: Result<T, String>) -> bool {
    match result {
        Ok(_) => {
            s.brief.error = None;
            true
        }
        Err(error) => {
            s.brief.error = Some(error);
            false
        }
    }
}

fn memory_editor(s: &State, p: Palette, captain: usize, id: Option<String>) -> El {
    let (edit, key, title) = match &id {
        Some(id) => (&s.brief.edit, format!("memory-input-{id}"), "Edit memory"),
        None => (
            &s.brief.draft,
            "brief-memory-input".to_string(),
            "New memory",
        ),
    };
    let editor_key = format!("{key}-editor");
    let count = edit.text.chars().count();
    let empty = edit.text.trim().is_empty();
    let save_id = id.clone();
    let save = move |s: &mut State| {
        let ok = match &save_id {
            Some(id) => {
                let text = s.brief.edit.text.clone();
                let result = s.edit_memory(captain, id, text);
                report(s, result)
            }
            None => {
                let text = s.brief.draft.text.clone();
                let result = s.add_memory(captain, text, false);
                report(s, result)
            }
        };
        if ok {
            s.brief.editing = None;
            s.brief.adding = false;
            s.brief.edit = rune::TextEdit::default();
            s.brief.draft = rune::TextEdit::default();
        }
    };
    let submit = save.clone();
    let field = memory_input(p, edit, key, title)
        .on_edit(move |s: &mut State| {
            if id.is_some() {
                &mut s.brief.edit
            } else {
                &mut s.brief.draft
            }
        })
        .on_submit(move |s: &mut State, _| submit(s));
    let cancel = |s: &mut State| {
        s.brief.editing = None;
        s.brief.adding = false;
    };
    col()
        .key(editor_key)
        .px(2.0)
        .py(4.0)
        .gap(6.0)
        .child(field)
        .child(
            row()
                .gap(6.0)
                .center_y()
                .child(label(
                    format!("{count} / 500"),
                    TIMER,
                    if count > 500 { p.danger } else { p.muted },
                ))
                .child(spacer())
                .child(ghost_button(s, "cancel-memory".into(), "Cancel", p, cancel))
                .child(
                    solid_button("save-memory".into(), "Save", p, save)
                        .disabled(empty || count > 500)
                        .opacity(if empty || count > 500 { 0.5 } else { 1.0 }),
                ),
        )
}

/// One memory row: the text with a "Yours" badge when user-controlled, the
/// "Always in context" switch on the right, and edit/delete on hover. A row
/// that is essential but left out by the budget says so in danger ink.
fn memory_row(s: &State, p: Palette, captain: usize, memory: &MemoryView, left_out: bool) -> El {
    let record =
        s.brief.record.is_some() || s.captain_access(captain) != crate::model::CaptainAccess::Full;
    if !record && s.brief.editing.as_deref() == Some(memory.id.as_str()) {
        return memory_editor(s, p, captain, Some(memory.id.clone()));
    }
    let id = memory.id.clone();
    let essential = memory.essential;
    let mut text = col()
        .key(format!("memory-text-{id}"))
        .grow(1.0)
        .min_w(0.0)
        .gap(3.0)
        .pt(2.0)
        .child(label(&memory.text, ROW_TEXT, p.ink).wrap().selectable(true));
    if left_out {
        text = text.child(
            label("Left out, over 8 KiB", META, p.danger).key(format!("memory-left-out-{id}")),
        );
    }
    if memory.user_controlled {
        text = text.child(
            row().child(
                row()
                    .key(format!("memory-lock-{id}"))
                    .label("You control this memory. The captain cannot change or forget it.")
                    .h(18.0)
                    .px(6.0)
                    .gap(4.0)
                    .center_y()
                    .radius(6.0)
                    .fill(p.ink.alpha(if s.dark { 0.06 } else { 0.05 }))
                    .child(
                        rune::icon(&super::icons::LOCK, super::ICON)
                            .color(p.muted)
                            .a11y_hidden(),
                    )
                    .child(meta_label("Yours", p.muted)),
            ),
        );
    }
    let mut row_ = row()
        .key(format!("memory-{id}"))
        .px(10.0)
        .min_h(36.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .hover(hover_wash(s, p))
        .hover_motion(motion(), motion())
        .hover_group_focus();
    row_ = row_.child(text);
    if record {
        return row_;
    }
    let toggle_id = id.clone();
    row_ = row_.child(captain_ui::switch(
        s,
        &format!("memory-pin-{id}"),
        "Always in context",
        essential,
        p,
        move |s: &mut State| {
            let result = s.set_memory_essential(captain, &toggle_id, !essential);
            report(s, result);
        },
    ));
    let edit_id = id.clone();
    let text_value = memory.text.clone();
    let wash = hover_wash(s, p);
    let backing = p.panel.mix(wash, wash.a);
    row_.child(
        row()
            .key(format!("memory-actions-{id}"))
            .pin(None, Some(4.0), Some(4.0), None)
            .gap(2.0)
            .px(2.0)
            .radius(6.0)
            .fill(backing)
            .child(
                captain_ui::icon_action(
                    s,
                    format!("memory-edit-{id}"),
                    "Edit memory",
                    icon("edit", p.muted),
                    p,
                    |_| {},
                )
                .on_press_tx(move |s: &mut State, tx| {
                    s.brief.adding = false;
                    s.brief.editing = Some(edit_id.clone());
                    s.brief.edit = rune::TextEdit::default();
                    s.brief.edit.replace_text(&text_value);
                    s.brief.edit.focused = true;
                    tx.scroll_into_view(format!("memory-input-{edit_id}-editor"));
                }),
            )
            .child(captain_ui::icon_action(
                s,
                format!("memory-delete-{id}"),
                "Delete memory",
                icon("trash", p.muted),
                p,
                move |s| {
                    let result = s.delete_memory(captain, &id);
                    report(s, result);
                },
            ))
            .group_opacity(0.0, 1.0),
    )
}

/// The profile row inside the Memory section: `profile_item` with its byte
/// count in TIMER faint and a pencil.
fn profile_row(s: &State, p: Palette) -> El {
    let profile = if s.settings_menu {
        s.settings_profile()
    } else {
        s.user_profile()
    };
    let bytes = profile_bytes(&profile);
    row()
        .center_y()
        .gap(8.0)
        .px(2.0)
        .child(profile_item(s, p).grow(1.0).min_w(0.0))
        .child(
            label(format!("{} of 1 KiB", kib(bytes)), TIMER, p.faint)
                .key("context-profile-bytes"),
        )
        .child(captain_ui::icon_action(
            s,
            "context-profile-edit".into(),
            "Edit your profile",
            icon("edit", p.muted),
            p,
            open_profile,
        ))
}

/// The Memory section for the captain audience: budget meter, profile, memory
/// rows with the essential switch, a fold for non-essential memories past the
/// first three, and the learned-memory status line.
pub(super) fn memory_section(s: &State, p: Palette, captain: usize) -> El {
    let memories = s.memories(captain);
    let record =
        s.brief.record.is_some() || s.captain_access(captain) != crate::model::CaptainAccess::Full;
    let add = captain_ui::icon_action(
        s,
        "brief-memory-add".into(),
        "Add memory",
        icon("plus", p.muted),
        p,
        |_| {},
    )
    .on_press_tx(|s: &mut State, tx| {
        s.brief.editing = None;
        s.brief.adding = true;
        s.brief.draft = rune::TextEdit::default();
        s.brief.draft.focused = true;
        tx.scroll_into_view("brief-memory-input-editor");
    });
    let mut section = captain_ui::section(
        "context-memory",
        "Memory",
        (!memories.is_empty()).then(|| memories.len().to_string()),
        if record { vec![] } else { vec![add] },
        p,
    );
    section = section.child(budget_meter(&memories, p));
    section = section.child(profile_row(s, p));
    if s.brief.adding && !record {
        section = section.child(memory_editor(s, p, captain, None));
    }
    if memories.is_empty() && (record || !s.brief.adding) {
        section = section.child(super::empty_list(
            s,
            p,
            "brief-memory-empty",
            "Nothing yet. Add what this captain should remember, or let it remember as it works.",
        ));
    }
    // Which essential memories the budget leaves out, in stored order.
    let mut used = ESSENTIAL_LABEL.len();
    let mut left_out: std::collections::HashSet<String> = std::collections::HashSet::new();
    for memory in memories.iter().filter(|m| m.essential) {
        let line = serde_json::to_string(memory).map_or(0, |line| line.len() + 1);
        if used + line <= ESSENTIAL_BUDGET {
            used += line;
        } else {
            left_out.insert(memory.id.clone());
        }
    }
    let essential: Vec<&MemoryView> = memories.iter().filter(|m| m.essential).collect();
    let extra: Vec<&MemoryView> = memories.iter().filter(|m| !m.essential).collect();
    for memory in &essential {
        section = section.child(memory_row(
            s,
            p,
            captain,
            memory,
            left_out.contains(&memory.id),
        ));
    }
    // Non-essential memories beyond the first three fold behind one row.
    let shown = extra.len().min(3);
    for memory in extra.iter().take(shown) {
        section = section.child(memory_row(s, p, captain, memory, false));
    }
    if extra.len() > shown {
        let count = extra.len() - shown;
        if s.context_memories_expanded {
            for memory in extra.iter().skip(shown) {
                section = section.child(memory_row(s, p, captain, memory, false));
            }
        } else {
            section = section.child(
                row()
                    .key("context-memory-more")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label(format!("{count} more, found by search. Expand"))
                    .focusable(true)
                    .min_h(32.0)
                    .px(10.0)
                    .gap(6.0)
                    .center_y()
                    .radius(6.0)
                    .hover(hover_wash(s, p))
                    .press(active_wash(s, p))
                    .hover_motion(motion(), motion())
                    .on_press(|s: &mut State| s.context_memories_expanded = true)
                    .child(
                        label(format!("{count} more, found by search"), META, p.muted)
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(icon("chevron-right", p.muted).size(14.0, 14.0)),
            );
        }
    }
    if let Some((status, connected)) = memory_backend_status(s) {
        section = section.child(
            row()
                .key("context-memory-backend")
                .px(10.0)
                .py(4.0)
                .gap(6.0)
                .center_y()
                .child(
                    el().size(6.0, 6.0)
                        .radius(3.0)
                        .fill(if connected { p.sea } else { p.danger }),
                )
                .child(label(status, SMALL, p.muted).wrap().grow(1.0).min_w(0.0)),
        );
    }
    section
}

/// The one locked row the Memory section collapses to for the crew audience.
fn memory_locked(p: Palette) -> El {
    captain_ui::section("context-memory", "Memory", None, vec![], p).child(
        row()
            .key("context-memory-locked")
            .px(10.0)
            .py(6.0)
            .gap(8.0)
            .center_y()
            .child(
                rune::icon(&super::icons::LOCK, super::ICON)
                    .color(p.muted)
                    .a11y_hidden(),
            )
            .child(
                label(
                    "Your profile, memories and captain rules are never sent to crew threads.",
                    SMALL,
                    p.muted,
                )
                .wrap()
                .grow(1.0)
                .min_w(0.0),
            ),
    )
}

/// The earlier-conversations rows, moved from the Brief: the list collapses
/// to one row with a count and a chevron, expanding in place. `with_reset` is
/// false inside a retired record's sheet, where the lifecycle section already
/// owns the reset row and its `brief-reset` key.
pub(super) fn conversation_section_with(
    s: &State,
    p: Palette,
    captain: usize,
    with_reset: bool,
) -> El {
    let earlier = s.earlier_conversations(captain);
    let mut section = captain_ui::section(
        "context-conversation",
        "Conversation",
        (!earlier.is_empty()).then(|| earlier.len().to_string()),
        vec![],
        p,
    );
    if earlier.is_empty() {
        section = section.child(captain_ui::quiet_note(
            "A reset keeps the current conversation here, read-only.",
            p,
        ));
    } else if !s.context_earlier_expanded {
        let count = earlier.len();
        section = section.child(
            row()
                .key("context-earlier-toggle")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!("Earlier conversations, {count}. Expand"))
                .focusable(true)
                .min_h(32.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(6.0)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press(|s: &mut State| s.context_earlier_expanded = true)
                .child(
                    label("Earlier conversations", ROW_TEXT, p.ink)
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(label(count.to_string(), TIMER, p.faint))
                .child(icon("chevron-right", p.muted).size(14.0, 14.0)),
        );
    } else {
        for id in earlier {
            let Some(thread) = s.threads.iter().find(|t| t.id == id) else {
                continue;
            };
            let open = s.current().id == id;
            let mut item = row()
                .key(format!("earlier-{id}"))
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!("Open {} read-only", thread.title))
                .focusable(true)
                .h(32.0)
                .px(10.0)
                .gap(10.0)
                .center_y()
                .radius(6.0)
                .fill(if open {
                    active_wash(s, p)
                } else {
                    Color::TRANSPARENT
                })
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| {
                    if let Some(index) = s.threads.iter().position(|t| t.id == id) {
                        s.show_thread(index);
                    }
                })
                .child(icon("thread", p.muted))
                .child(
                    label(&thread.title, ROW_TEXT, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                );
            if let Some(age) = relative_time(thread.updated) {
                item = item.child(label(age, TIMER, p.muted));
            }
            section = section.child(item);
        }
    }
    if with_reset {
        section = section.child(
            ghost_button(
                s,
                "brief-reset".into(),
                "Reset conversation",
                p,
                |s: &mut State| s.brief.confirm_reset = true,
            )
            .mx(10.0),
        );
    }
    section
}

pub(super) fn conversation_section(s: &State, p: Palette, captain: usize) -> El {
    conversation_section_with(s, p, captain, true)
}

/// The Workspaces section: the Brief's scope chips and picker, unchanged.
pub(super) fn scope_section(s: &State, p: Palette, captain: usize) -> El {
    let scope = s.captain_scope(captain);
    let remote_scope = s.remote_captain_scope(captain);
    let add = captain_ui::icon_action(
        s,
        "brief-scope-add".into(),
        "Add a workspace to this captain's scope",
        icon("plus", p.muted),
        p,
        |s| s.open_picker(crate::model::ProjectPicker::CaptainScope),
    );
    let mut section = captain_ui::section(
        "context-scope",
        "Workspaces",
        (!(scope.is_empty() && remote_scope.is_empty()))
            .then(|| (scope.len() + remote_scope.len()).to_string()),
        vec![add],
        p,
    );
    if scope.is_empty() && remote_scope.is_empty() {
        return section.child(captain_ui::quiet_note(
            "No workspace. This captain works as a general assistant in its own folder.",
            p,
        ));
    }
    let mut chips = row().px(6.0).py(2.0).gap(6.0).flex_wrap();
    for (index, id) in scope.iter().copied().enumerate() {
        let project = s.project(Some(id));
        let name = project
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Missing workspace".into());
        let path = project
            .map(|p| p.path.display().to_string())
            .unwrap_or_default();
        chips = chips.child(
            row()
                .key(format!("brief-scope-{id}"))
                .label(if index == 0 {
                    format!("{name}, primary workspace, {path}")
                } else {
                    format!("{name}, {path}")
                })
                .h(28.0)
                .pl(8.0)
                .pr(2.0)
                .gap(6.0)
                .center_y()
                .max_w(captain_ui::PANEL_WIDTH - 32.0)
                .radius(6.0)
                .border(1.0, hairline(p))
                .child(super::project_tile(s, p, project, 14.0))
                .child(
                    label(name.clone(), TextStyle::new(13.0, 500.0), p.ink)
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                )
                .child(captain_ui::icon_action(
                    s,
                    format!("brief-scope-remove-{id}"),
                    &format!("Remove {name} from scope"),
                    icon("close", p.muted),
                    p,
                    move |s| {
                        let mut scope = s.captain_scope(captain);
                        scope.retain(|&p| p != id);
                        let result = s.set_captain_scope(captain, scope);
                        report(s, result);
                    },
                )),
        );
    }
    for entry in remote_scope {
        let peer = s.peers.iter().find(|peer| peer.server_id == entry.server);
        let name = peer
            .and_then(|p| p.capabilities["projects"].as_array())
            .and_then(|ps| {
                ps.iter()
                    .find(|p| p["id"].as_u64() == Some(entry.project_id as u64))
            })
            .and_then(|p| p["name"].as_str())
            .unwrap_or("Missing workspace");
        let machine = peer.map(|p| p.label.as_str()).unwrap_or(&entry.server);
        let text = format!("{name}, {machine}");
        let server = entry.server.clone();
        let id = entry.project_id;
        chips = chips.child(
            row()
                .key(format!("brief-scope-{server}-{id}"))
                .label(&text)
                .h(28.0)
                .pl(8.0)
                .pr(2.0)
                .gap(6.0)
                .center_y()
                .max_w(captain_ui::PANEL_WIDTH - 32.0)
                .radius(6.0)
                .border(1.0, hairline(p))
                .child(
                    label(text.clone(), TextStyle::new(13.0, 500.0), p.ink)
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                )
                .child(captain_ui::icon_action(
                    s,
                    format!("brief-scope-remove-{server}-{id}"),
                    &format!("Remove {text} from scope"),
                    icon("close", p.muted),
                    p,
                    move |s| captain_ui::toggle_remote_scope(s, server.clone(), id),
                )),
        );
    }
    section = section.child(chips);
    if scope.len() > 1 {
        section = section.child(captain_ui::quiet_note(
            "The first workspace hosts the conversation. Tasks name the one they work in.",
            p,
        ));
    }
    section
}

/// The Context window section every thread gets: the usage figure, the
/// segmented bar and legend the popover draws, and the Compact control.
fn window_section(s: &State, p: Palette) -> El {
    let t = s.current();
    let limit = context_limit(t);
    let usage = &t.context.usage;
    let legend = context_legend_for_thread(t);
    let mut section = captain_ui::section("context-window", "Context window", None, vec![], p);
    let used = match (usage.tokens, limit) {
        (Some(tokens), Some(limit)) => format!(
            "{} of {}",
            context_tokens_text(tokens),
            context_tokens_text(limit)
        ),
        (Some(tokens), None) => context_tokens_text(tokens),
        (None, Some(limit)) => format!("of {}", context_tokens_text(limit)),
        (None, None) => String::new(),
    };
    if usage.percent.is_none() && used.is_empty() {
        return section.child(captain_ui::quiet_note(
            "Usage appears after the first reply.",
            p,
        ));
    }
    section = section.child(
        row()
            .px(10.0)
            .pb(4.0)
            .center_y()
            .gap(8.0)
            .child(label(used, TIMER, p.faint).key("context-window-usage"))
            .child(spacer())
            .child(
                label(
                    usage
                        .percent
                        .map(context_percent_text)
                        .unwrap_or_default(),
                    TIMER,
                    usage
                        .percent
                        .map(|percent| context_ink(percent, p))
                        .unwrap_or(p.faint),
                )
                .key("context-window-percent"),
            ),
    );
    if let Some(legend) = &legend {
        let approximate =
            crate::context_breakdown::is_approximate(usage.breakdown.as_deref(), Some(legend));
        section = section.child(
            el().px(10.0).child(
                el().key("context-window-bar").child(context_breakdown_view(
                    legend,
                    approximate,
                    t.context.active_segment(),
                    p,
                )),
            ),
        );
    } else if let Some(limit) = limit
        && limit > 0
    {
        section = section.child(
            el().px(10.0).child(usage_progress_bar(
                "context-window-bar".into(),
                Some("context-window-bar-fill".into()),
                usage.percent.unwrap_or(0.0),
                p,
            )),
        );
    }
    let can_compact = !s.demo && t.can_compact();
    let keep_full = s.keep_full_history(t);
    let compactable = s
        .resume_compaction_tokens(t, crate::lifecycle::now())
        .is_some();
    let mut actions = row().px(10.0).pt(6.0).gap(8.0).center_y();
    if compactable {
        actions = actions.child(
            row()
                .key("context-keep-history")
                .role("switch")
                .checked(keep_full)
                .cursor(rune::Cursor::Pointer)
                .label(if keep_full {
                    "Keep full history on the next send"
                } else {
                    "Compact history on the next send"
                })
                .focusable(true)
                .h(24.0)
                .px(6.0)
                .gap(6.0)
                .center_y()
                .radius(6.0)
                .hover(p.selected)
                .on_press(|s: &mut State| s.toggle_keep_full_history())
                .child(
                    label(
                        if keep_full { "Keep" } else { "Compact" },
                        META,
                        p.ink,
                    )
                    .key("context-keep-history-label"),
                )
                .child(label("history before send", META, p.muted)),
        );
    }
    actions = actions.child(spacer()).child(
        ghost_button(s, "context-compact".into(), "Compact", p, |s: &mut State| {
            s.compact_context(None);
        })
        .disabled(!can_compact)
        .opacity(if can_compact { 1.0 } else { 0.55 }),
    );
    section.child(actions)
}

/// The thread's attached files with sizes; pressing a row opens the preview.
fn attachments_section(s: &State, p: Palette) -> El {
    let t = s.current();
    let tray = tray_attachments(t);
    let sent: Vec<&Attachment> = t
        .messages
        .iter()
        .flat_map(|message| message.images.iter())
        .collect();
    let section = captain_ui::section(
        "context-attachments",
        "Attachments",
        (!(tray.is_empty() && sent.is_empty()))
            .then(|| (tray.len() + sent.len()).to_string()),
        vec![],
        p,
    );
    if tray.is_empty() && sent.is_empty() {
        return section.child(captain_ui::quiet_note(
            "No attachments on this thread yet.",
            p,
        ));
    }
    let mut rows = col().gap(2.0);
    for attachment in tray.into_iter().chain(sent.into_iter()) {
        let id = attachment.id.clone();
        let name = attachment.name.clone();
        let size = format!("{} KiB", kib(attachment.bytes as usize));
        rows = rows.child(
            row()
                .key(format!("context-attachment-{id}"))
                .role("button")
                .label(format!("{name}, {size}"))
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .min_h(32.0)
                .px(10.0)
                .gap(10.0)
                .center_y()
                .radius(6.0)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| {
                    s.preview_attachments(s.current().draft_images.clone(), &id, true);
                })
                .child(icon("file", p.muted).size(16.0, 16.0))
                .child(
                    label(name, ROW_TEXT, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(label(size, TIMER, p.faint)),
        );
    }
    section.child(rows)
}

/// One read-only fact row in the Workspace section of a plain thread.
fn fact_row(key: String, name: &str, value: String, p: Palette) -> El {
    row()
        .key(key)
        .min_h(28.0)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .child(label(name, META, p.muted))
        .child(spacer())
        .child(label(value, ROW_TEXT, p.ink).truncate().min_w(0.0).shrink())
}

/// The plain-thread Workspace section: project, branch or worktree, machine.
fn workspace_section(s: &State, p: Palette) -> El {
    let t = s.current();
    let mut section = captain_ui::section("context-scope", "Workspace", None, vec![], p);
    let project = t
        .project
        .and_then(|id| s.project(Some(id)))
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "No workspace".into());
    section = section.child(fact_row(
        "context-workspace-project".into(),
        "Project",
        project,
        p,
    ));
    let branch = t
        .worktree_path
        .clone()
        .or_else(|| t.branch.clone())
        .unwrap_or_else(|| "Default branch".into());
    section = section.child(fact_row(
        "context-workspace-branch".into(),
        "Branch",
        branch,
        p,
    ));
    section = section.child(fact_row(
        "context-workspace-machine".into(),
        "Machine",
        s.machine_label(&s.machines.selected),
        p,
    ));
    section
}

/// What a crew worker sees under "From the captain": the brief that assigned
/// the work, plus CREW.md and NOTES.md with their crew delivery.
fn from_captain_section(s: &State, p: Palette, captain: usize) -> El {
    let t = s.current();
    let mut section =
        captain_ui::section("context-documents", "From the captain", None, vec![], p);
    let brief = t.messages.iter().enumerate().find(|(_, message)| {
        message.role == "Shina" && crew_ui::parse_brief(message.display_text()).is_some()
    });
    if let Some((index, message)) = brief {
        let bytes = message.display_text().len();
        let key = message_key(t, index);
        let thread_id = t.id;
        section = section.child(
            row()
                .key("context-crew-brief")
                .role("button")
                .label(format!("Brief, {} KiB. Show in the transcript", kib(bytes)))
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .min_h(32.0)
                .px(10.0)
                .gap(10.0)
                .center_y()
                .radius(6.0)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press_tx(move |s: &mut State, tx| {
                    if s.current().id == thread_id {
                        tx.scroll_into_view_in(format!("conversation-{thread_id}"), &key);
                    }
                })
                .child(icon("file", p.muted).size(16.0, 16.0))
                .child(label("Brief", ROW_TEXT, p.ink).grow(1.0).min_w(0.0))
                .child(label(format!("{} KiB", kib(bytes)), TIMER, p.faint)),
        );
    }
    let narrow = s.right_panel_width <= 380.0;
    for source in sources(s, captain, Audience::Crew)
        .into_iter()
        .filter(|source| matches!(source.key, "CREW.md" | "NOTES.md"))
    {
        section = section.child(document_row(s, p, captain, &source, narrow));
    }
    if s.captain_access(captain) != crate::model::CaptainAccess::Full {
        section = section.child(captain_ui::quiet_note("Read-only from this thread.", p));
    }
    section
}

fn rule_inset(p: Palette) -> El {
    el().h(1.0).mx(16.0).fill(hairline(p))
}

/// The document viewer's header: a back control, the title and file name, the
/// sent figure when the document is truncated, then the document panel's own
/// Edit, Save and Cancel actions.
fn doc_header(s: &State, p: Palette, captain: usize, name: &str) -> El {
    let panel = s.panel_key();
    let meta = s.captain_document_view(captain, name);
    let budget = match name {
        "AGENTS.md" => Some(RULES_CONTEXT_MAX_BYTES),
        "NOTES.md" => Some(NOTES_CONTEXT_MAX_BYTES),
        "CREW.md" => Some(CREW_CONTEXT_MAX_BYTES),
        _ => None,
    };
    let truncated = budget.is_some_and(|budget| meta.bytes > budget);
    let title = captain_docs_ui::document_title(name);
    let mut header = row()
        .key("context-doc-header")
        .h(36.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .child(icon_button(
            "context-back",
            "Back to context",
            "chevron-left",
            p,
            move |s: &mut State| s.request_close_viewer(&panel),
        ))
        .child(label(title, ROW_TEXT, p.ink).truncate().min_w(0.0))
        .child(label(name.to_string(), SMALL, p.faint));
    if truncated {
        header = header.child(spacer()).child(
            label(
                format!(
                    "{} of {} KiB sent",
                    kib(covered(meta.bytes, budget.unwrap())),
                    kib(meta.bytes)
                ),
                TIMER,
                p.brass,
            )
            .key("context-doc-sent"),
        );
    }
    let key = crate::model::DocumentDraftKey {
        server: s.machines.selected.clone(),
        captain,
        name: name.to_string(),
    };
    if s.captain_drafts.get(&key).is_none()
        && s.captain_access(captain) == crate::model::CaptainAccess::Full
    {
        if !truncated {
            header = header.child(spacer());
        }
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
    header
}

/// The Context tab's list body (spec §4.3): audience switch, context window,
/// documents, memory, workspaces and conversation on a captain home; window,
/// attachments and workspace on a plain thread; window and "From the captain"
/// on a crew worker.
pub(super) fn tab(s: &State, p: Palette) -> El {
    let t = s.current();
    let captain = captain_ui::home_brief_captain(s);
    let crew_captain = t.crew_captain.filter(|captain| *captain < s.captains.len());
    let mut body = col().key("context-tab").min_h(0.0).w_full().pb(8.0);
    if let Some(error) = captain_ui::brief_error_banner(s, p) {
        body = body.child(error);
    }
    match (captain, crew_captain) {
        (Some(captain), _) => {
            let access = s.captain_access(captain);
            let knowable = !matches!(
                access,
                crate::model::CaptainAccess::UpdateRequired | crate::model::CaptainAccess::NoRecords
            );
            let crew = s.context_audience_crew;
            body = body.child(audience_row(s, p, captain));
            body = body.child(rule_inset(p));
            body = body.child(window_section(s, p));
            if knowable {
                body = body
                    .child(rule_inset(p))
                    .child(documents_section(
                        s,
                        p,
                        captain,
                        if crew { Audience::Crew } else { Audience::Captain },
                    ));
            }
            body = body.child(rule_inset(p));
            if crew {
                body = body.child(memory_locked(p));
            } else {
                body = body.child(memory_section(s, p, captain));
            }
            body = body
                .child(rule_inset(p))
                .child(
                    scope_section(s, p, captain)
                        .disabled(access != crate::model::CaptainAccess::Full),
                )
                .child(rule_inset(p))
                .child(conversation_section(s, p, captain));
        }
        (None, Some(captain)) => {
            body = body
                .child(window_section(s, p))
                .child(rule_inset(p))
                .child(from_captain_section(s, p, captain));
        }
        (None, None) => {
            body = body
                .child(window_section(s, p))
                .child(rule_inset(p))
                .child(attachments_section(s, p))
                .child(rule_inset(p))
                .child(workspace_section(s, p));
        }
    }
    col()
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(body)
}

/// Entry point for the Inspector's Context tab (both the list and the open
/// captain-document viewer route).
pub(super) fn view(s: &State, p: Palette) -> (String, El) {
    if let Some(crate::right_panel::Surface::CaptainDocument { captain, name }) =
        &s.right_panel().viewer
    {
        let (captain, name) = (*captain, name.clone());
        return (
            format!("right-panel-body-captain-doc:{captain}:{name}"),
            col()
                .min_h(0.0)
                .w_full()
                .h_full()
                .child(doc_header(s, p, captain, &name))
                .child(rule(p))
                .child(
                    captain_docs_ui::document_panel_body(s, p, captain, &name)
                        .grow(1.0)
                        .min_h(0.0),
                ),
        );
    }
    ("right-panel-body-context".into(), tab(s, p))
}

/// The settings menu's entry for the profile editor, and the same row inside
/// the Context tab's Memory section.
pub(super) fn profile_item(s: &State, p: Palette) -> El {
    let profile = if s.settings_menu {
        s.settings_profile()
    } else {
        s.user_profile()
    };
    let detail = if profile.name.trim().is_empty() {
        "Add your name and preferences".to_string()
    } else {
        profile.name.trim().to_string()
    };
    row()
        .key("open-profile")
        .role("menuitem")
        .cursor(rune::Cursor::Pointer)
        .label("You: profile shared with every captain")
        .focusable(true)
        .h(44.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(12.0)
        .focus_highlight(p.selected)
        .on_press(open_profile)
        .child(
            el().size(28.0, 28.0)
                .center()
                .radius(6.0)
                .fill(p.coral)
                .border(1.0, p.frame)
                .child(super::meta_text(
                    profile
                        .name
                        .trim()
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_else(|| "Y".into()),
                    p.on_coral,
                )),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(label("You", TextStyle::new(13.0, 500.0), p.ink))
                .child(label(detail, META, p.muted).truncate()),
        )
        .child(icon("open", p.muted))
}

pub(super) fn open_profile(s: &mut State) {
    let profile = if s.settings_menu {
        s.settings_profile()
    } else {
        s.user_profile()
    };
    s.settings.profile_target = s.settings_menu.then(|| s.settings.machine.clone());
    s.settings_menu = false;
    s.brief.profile_open = true;
    s.brief.error = None;
    for (edit, value) in [
        (&mut s.brief.profile_name, profile.name),
        (&mut s.brief.profile_zone, profile.time_zone),
        (&mut s.brief.profile_preferences, profile.preferences),
    ] {
        *edit = rune::TextEdit::default();
        edit.replace_text(&value);
    }
    s.brief.profile_name.focused = true;
}

fn draft_profile(s: &State) -> crate::model::UserProfile {
    crate::model::UserProfile {
        name: s.brief.profile_name.text.clone(),
        time_zone: s.brief.profile_zone.text.clone(),
        preferences: s.brief.profile_preferences.text.clone(),
    }
}

/// The profile's size as memory.rs measures it: its serialized JSON.
pub(super) fn profile_bytes(profile: &crate::model::UserProfile) -> usize {
    serde_json::to_vec(profile).map_or(0, |bytes| bytes.len())
}

fn close_profile(s: &mut State) {
    s.brief.profile_open = false;
}

fn save_profile(s: &mut State) {
    let profile = draft_profile(s);
    let result = if s.settings.profile_target.is_some() {
        s.save_settings_profile(profile)
    } else {
        s.set_user_profile(profile)
    };
    if report(s, result) {
        s.brief.profile_open = false;
    }
}

/// One labelled field, as a KSG settings row with its control below the title.
fn profile_field(p: Palette, title: &str, note: &str, field: El) -> El {
    col()
        .gap(6.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(meta_label(title, p.muted))
                .child(
                    label(note, TextStyle::new(12.0, 400.0), p.muted)
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                ),
        )
        .child(field)
}

fn profile_input(s: &State, p: Palette, edit: &rune::TextEdit, key: &str, title: &str) -> El {
    text_input(edit, TextStyle::new(14.0, 400.0).lh(20.0))
        .key(key)
        .label(title)
        .color(p.ink)
        .w_full()
        .px(10.0)
        .py(7.0)
        .radius(6.0)
        .fill(field_fill(s, p))
        .field_focus(edit, p)
}

/// "You": the global profile every captain reads, ported from KSG's settings panels
/// (`settings/settingsLayout.tsx` rows inside a `SettingsGroup` card) as an editor dialog.
/// A live count holds it to 1 KiB.
pub(super) fn profile_dialog(s: &State, p: Palette) -> Option<El> {
    if !s.brief.profile_open {
        return None;
    }
    let bytes = profile_bytes(&draft_profile(s));
    let over = bytes > PROFILE_BUDGET;
    let name = profile_field(
        p,
        "Name",
        "What captains call you",
        profile_input(s, p, &s.brief.profile_name, "profile-name", "Name")
            .single_line()
            .placeholder("Your name", p.muted)
            .on_edit(|s: &mut State| &mut s.brief.profile_name)
            .on_submit(|s: &mut State, _| save_profile(s)),
    );
    let zone = profile_field(
        p,
        "Time zone",
        "For schedules and dates",
        profile_input(s, p, &s.brief.profile_zone, "profile-zone", "Time zone")
            .single_line()
            .placeholder("Europe/Berlin", p.muted)
            .on_edit(|s: &mut State| &mut s.brief.profile_zone)
            .on_submit(|s: &mut State, _| save_profile(s)),
    );
    let preferences = profile_field(
        p,
        "Preferences",
        "How you like to work",
        profile_input(
            s,
            p,
            &s.brief.profile_preferences,
            "profile-preferences",
            "Preferences",
        )
        .placeholder("Short answers first. Ask before large refactors.", p.muted)
        .min_h(96.0)
        .max_h(180.0)
        .on_edit(|s: &mut State| &mut s.brief.profile_preferences),
    );
    let fraction = (bytes as f32 / PROFILE_BUDGET as f32).clamp(0.0, 1.0);
    let ink = if over {
        p.danger
    } else if fraction > 0.85 {
        p.brass
    } else {
        p.muted
    };
    let mut card = col()
        .key("profile-dialog")
        .role("dialog")
        .label("You")
        .w(460.0)
        .max_w(s.viewport_width - 32.0)
        .padding(20.0)
        .gap(16.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .gap(8.0)
                .child(
                    col()
                        .gap(4.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .child(label(
                            s.settings.profile_target.as_ref().map_or_else(
                                || "You".into(),
                                |machine| format!("You on {}", s.machine_label(machine)),
                            ),
                            super::DIALOG_TITLE,
                            p.ink,
                        ))
                        .child(
                            label(
                                "Every captain reads this with its memory. It is data about you, not instructions.",
                                TextStyle::new(13.0, 400.0).lh(18.0),
                                p.muted,
                            )
                            .wrap(),
                        ),
                )
                .child(icon_button("close-profile", "Close", "close", p, close_profile)),
        )
        .child(name)
        .child(zone)
        .child(preferences);
    if let Some(error) = &s.brief.error {
        card = card.child(label(error, SMALL, p.danger).wrap());
    }
    card = card.child(
        row()
            .gap(8.0)
            .center_y()
            .child(
                label(format!("{bytes} / {PROFILE_BUDGET} bytes"), TIMER, ink)
                    .key("profile-count")
                    .label(if over {
                        format!(
                            "{bytes} of {PROFILE_BUDGET} bytes. Shorten the profile to save it."
                        )
                    } else {
                        format!("{bytes} of {PROFILE_BUDGET} bytes used")
                    }),
            )
            .child(spacer())
            .child(secondary_button(
                "cancel-profile",
                "Cancel",
                p,
                close_profile,
            ))
            .child(
                primary_button("save-profile", "Save", p, save_profile)
                    .disabled(over)
                    .opacity(if over { 0.5 } else { 1.0 }),
            ),
    );
    Some(modal_frame("profile-backdrop", s, close_profile, card))
}

/// The reset confirmation, as KSG's `ui/alert-dialog.tsx`: a title, what
/// happens, and what is kept, with Cancel before the action.
pub(super) fn reset_dialog(s: &State, p: Palette) -> Option<El> {
    let captain = captain_ui::brief_captain(s).filter(|_| s.brief.confirm_reset)?;
    let name = s.captains[captain].name.clone();
    let close = |s: &mut State| s.brief.confirm_reset = false;
    let card = col()
        .key("reset-conversation-dialog")
        .role("alertdialog")
        .label("Reset conversation")
        .w(400.0)
        .max_w(s.viewport_width - 32.0)
        .padding(20.0)
        .gap(10.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            label(
                format!("Reset the conversation with {name}?"),
                super::DIALOG_TITLE,
                p.ink,
            )
            .wrap(),
        )
        .child(
            label(
                "Memory, scope, routines and tasks are kept. The current conversation moves to Earlier conversations, where you can still read it.",
                BODY,
                p.muted,
            )
            .wrap(),
        )
        .child(
            row()
                .pt(8.0)
                .gap(8.0)
                .child(spacer())
                .child(secondary_button(
                    "cancel-reset-conversation",
                    "Cancel",
                    p,
                    close,
                ))
                .child(primary_button(
                    "confirm-reset-conversation",
                    "Reset conversation",
                    p,
                    move |s| {
                        s.brief.confirm_reset = false;
                        let result = s.reset_captain_conversation(captain);
                        if report(s, result)
                            && let Some(index) = s
                                .captain_home(captain)
                                .and_then(|home| s.threads.iter().position(|t| t.id == home))
                        {
                            s.show_thread(index);
                        }
                    },
                )),
        );
    Some(
        el().key("reset-conversation-backdrop")
            .instant_changes()
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .padding(16.0)
            .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.28 }))
            .modal(false, close)
            .child(card),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store-backed captain home on a temp dir, so documents really save.
    fn captain_state() -> State {
        let dir = std::env::temp_dir().join(format!(
            "shina-context-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::new(dir, false);
        state.load_global_for_test();
        state.captains = crate::captains::Captain::presets();
        state.open_captain(0);
        state
    }

    #[test]
    fn a_large_notes_document_is_truncated_for_the_captain_and_on_request_for_crew() {
        let mut s = captain_state();
        s.captain_document_write(0, "NOTES.md", "x".repeat(14_540), 0)
            .unwrap();
        let captain = sources(&s, 0, Audience::Captain);
        let notes = captain
            .iter()
            .find(|source| source.key == "NOTES.md")
            .unwrap();
        assert_eq!(notes.delivery, Delivery::Truncated);
        assert!(notes.sent <= NOTES_CONTEXT_MAX_BYTES && notes.sent > 0);
        let crew = sources(&s, 0, Audience::Crew);
        let notes = crew.iter().find(|source| source.key == "NOTES.md").unwrap();
        assert_eq!(notes.delivery, Delivery::OnRequest);
        assert_eq!(notes.sent, 0);
        std::fs::remove_dir_all(&s.data_dir).ok();
    }

    #[test]
    fn agents_md_is_private_and_never_sent_to_crew() {
        let mut s = captain_state();
        s.captain_document_write(0, "AGENTS.md", "rule".into(), 0)
            .unwrap();
        let crew = sources(&s, 0, Audience::Crew);
        let rules = crew
            .iter()
            .find(|source| source.key == "AGENTS.md")
            .unwrap();
        assert_eq!(rules.delivery, Delivery::NotSent);
        assert!(rules.private);
        let captain = sources(&s, 0, Audience::Captain);
        let rules = captain
            .iter()
            .find(|source| source.key == "AGENTS.md")
            .unwrap();
        assert_eq!(rules.delivery, Delivery::Whole);
        std::fs::remove_dir_all(&s.data_dir).ok();
    }

    #[test]
    fn crew_md_is_not_sent_to_the_captain_but_is_whole_for_crew() {
        let mut s = captain_state();
        s.captain_document_write(0, "CREW.md", "shared rule".into(), 0)
            .unwrap();
        let captain = sources(&s, 0, Audience::Captain);
        let crew_doc = captain
            .iter()
            .find(|source| source.key == "CREW.md")
            .unwrap();
        assert_eq!(crew_doc.delivery, Delivery::NotSent);
        let crew = sources(&s, 0, Audience::Crew);
        let crew_doc = crew.iter().find(|source| source.key == "CREW.md").unwrap();
        assert_eq!(crew_doc.delivery, Delivery::Whole);
        assert_eq!(crew_doc.sent, "shared rule".len());
        std::fs::remove_dir_all(&s.data_dir).ok();
    }

    #[test]
    fn an_empty_document_reports_empty_and_zero_sent() {
        let s = captain_state();
        for audience in [Audience::Captain, Audience::Crew] {
            let list = sources(&s, 0, audience);
            let notes = list.iter().find(|source| source.key == "NOTES.md").unwrap();
            match audience {
                Audience::Captain => assert_eq!(notes.delivery, Delivery::Empty),
                Audience::Crew => assert_eq!(notes.delivery, Delivery::OnRequest),
            }
            assert_eq!(notes.sent, 0);
        }
        std::fs::remove_dir_all(&s.data_dir).ok();
    }

    #[test]
    fn essential_usage_counts_records_and_reports_those_left_out() {
        let memory = |n: usize, essential| MemoryView {
            id: format!("memory-{n}"),
            text: "x".repeat(480),
            essential,
            user_controlled: true,
        };
        let few: Vec<_> = (0..3).map(|n| memory(n, true)).collect();
        let (used, skipped) = essential_usage(&few);
        assert!(used > 3 * 480 && used < ESSENTIAL_BUDGET && skipped == 0);
        let many: Vec<_> = (0..20).map(|n| memory(n, n % 5 != 0)).collect();
        let (used, skipped) = essential_usage(&many);
        assert!(used <= ESSENTIAL_BUDGET && skipped > 0);
    }
}
