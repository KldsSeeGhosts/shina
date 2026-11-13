//! Per-code-block language header and picker, ported from KSG Code's
//! `ComposerCodeBlockNodeView.tsx` and `ComposerCodeBlockLanguagePicker.tsx`.
//!
//! Each code block in the composer draft gets a non-editable header through Rune's
//! `InputBlockView`, holding the language trigger. The popover mounts beside the
//! other composer popovers at the root, anchored under the trigger it belongs to.

use super::*;
use crate::composer_code_languages as languages;

/// Open picker state on `State`, beside the other composer popovers.
#[derive(Default)]
pub(crate) struct Picker {
    /// The block whose menu is open. Identity survives edits and moves.
    pub open: Option<rune::BlockId>,
    /// The search field's model, reset when the menu closes, as KSG's
    /// `onOpenChange` clears its query.
    pub query: rune::TextEdit,
}

/// Code blocks in document order.
fn code_blocks(edit: &rune::TextEdit) -> impl Iterator<Item = rune::LogicalBlock> + '_ {
    edit.logical_blocks()
        .filter(|b| b.kind == rune::BlockKind::Code)
}

/// A code block's complete info string (`CodeAttributes::info`), or "".
fn code_info(block: &rune::LogicalBlock) -> &str {
    block
        .attributes
        .code()
        .map_or("", rune::CodeAttributes::info)
}

/// The block the open menu is bound to, or None when it left the draft.
fn open_block(s: &State) -> Option<rune::LogicalBlock> {
    let id = s.code_picker.open?;
    code_blocks(&s.current().draft).find(|b| b.id == id)
}

/// Whether the menu is on screen: the open block id still resolves in the draft.
pub(super) fn menu_open(s: &State) -> bool {
    open_block(s).is_some()
}

/// The menu's anchor key: the trigger of the open block, indexed among code blocks.
fn anchor(s: &State, id: rune::BlockId) -> String {
    let index = code_blocks(&s.current().draft)
        .position(|b| b.id == id)
        .unwrap_or(0);
    format!("composer-code-language-{index}")
}

/// `El::input_block_view` takes one view per call, so the per-block loop lives behind
/// this one builder call on the composer field. `disabled` is the composer input's
/// locked condition, like KSG's `disabled={!editor.isEditable}`.
pub(super) trait CodeLanguageHeaders {
    fn code_language_headers(self, s: &State, p: Palette, disabled: bool) -> Self;
}

impl CodeLanguageHeaders for El {
    fn code_language_headers(mut self, s: &State, p: Palette, disabled: bool) -> Self {
        let blocks: Vec<(rune::BlockId, String)> = code_blocks(&s.current().draft)
            .map(|block| (block.id, code_info(&block).to_owned()))
            .collect();
        for (index, (id, info)) in blocks.into_iter().enumerate() {
            self = self.input_block_view(rune::InputBlockView::header(
                id,
                header(index, id, &info, disabled, p),
            ));
        }
        self
    }
}

/// KSG's `MarkdownCodeBlockFrame` header: `pt-1.5 pr-1.5 pb-0 pl-3` on the code
/// background, a `frame` hairline under it, and the block's top corners. The
/// `ComboboxTrigger` is a `ghost-muted` Button `size="xs"` (`h-6 gap-1 px-[7px]`,
/// desktop `text-xs`) inside a `-ml-1.5` wrapper that aligns its label under the
/// block padding; `outlined` marks it as informative chrome, like the stash badge.
fn header(index: usize, id: rune::BlockId, info: &str, disabled: bool, p: Palette) -> El {
    let language = languages::language_of_info_string(info);
    let shown = if language.is_empty() {
        "text".to_owned()
    } else {
        languages::code_language_label(language).to_owned()
    };
    let mut trigger = row()
        .key(format!("composer-code-language-{index}"))
        .role("button")
        .label(format!("Code block language, {shown}"))
        .h(24.0)
        .px(7.0)
        .gap(4.0)
        .ml(-6.0)
        .center_y()
        .radius(4.0)
        .focusable(!disabled)
        .cursor(if disabled {
            rune::Cursor::Default
        } else {
            rune::Cursor::Pointer
        })
        .opacity(if disabled { 0.65 } else { 1.0 })
        .disabled(disabled)
        // KSG caps the label at `max-w-40` and truncates it.
        .child(
            pixel_text(shown, p.muted)
                .truncate()
                .shrink()
                .min_w(0.0)
                .max_w(160.0),
        )
        .child(icon("chevron", p.muted));
    if !disabled {
        trigger = trigger.hover(p.selected).on_press(move |s: &mut State| {
            s.begin_composer_menu();
            s.code_picker.open = Some(id);
            s.code_picker.query = rune::TextEdit::default();
            s.code_picker.query.focused = true;
        });
    }
    col()
        .w_full()
        .fill(p.code_bg)
        .corners([4.0, 4.0, 0.0, 0.0])
        // The fork's rule: the control inside takes focus, the header itself does not.
        .focusable(false)
        .child(
            col()
                .items_start()
                .pt(6.0)
                .pl(12.0)
                .pr(6.0)
                .child(outlined(trigger, p)),
        )
        .child(el().w_full().h(1.0).fill(p.frame))
}

/// KSG's `ComboboxPopup`: a search field above the language list in a `w-56` card,
/// anchored under the trigger. `composer-code-language-menu` is the list: focusing
/// its key lands on the first option row, so Down leaves the search field onto the
/// first language and Up/Down, Enter and Escape follow the group's keys.
pub(super) fn menu(s: &State, p: Palette) -> El {
    let Some(block) = open_block(s) else {
        return el();
    };
    let info = code_info(&block).to_owned();
    let block_id = block.id;
    let language = languages::language_of_info_string(&info).to_owned();
    let current = languages::code_language_entry(&language)
        .map(|entry| entry.id)
        .unwrap_or(language.as_str())
        .to_owned();
    // KSG prepends `{id: current, label: current}` when the current id is not in the
    // catalog, so an unknown language stays visible and selectable.
    let mut items: Vec<(String, String)> = Vec::new();
    if !languages::CODE_BLOCK_LANGUAGES
        .iter()
        .any(|entry| entry.id == current)
    {
        items.push((current.clone(), current.clone()));
    }
    items.extend(
        languages::CODE_BLOCK_LANGUAGES
            .iter()
            .map(|entry| (entry.id.to_owned(), entry.label.to_owned())),
    );
    let needle = s.code_picker.query.text.trim().to_lowercase();
    let shown: Vec<(String, String)> = items
        .into_iter()
        .filter(|(id, name)| needle.is_empty() || languages::language_matches(id, name, &needle))
        .collect();

    let header = row().center_y().gap(6.0).px(10.0).py(8.0).child(
        row()
            .center_y()
            .gap(6.0)
            .h(30.0)
            .px(10.0)
            .radius(4.0)
            .fill(p.selected)
            .field_focus(&s.code_picker.query, p)
            .grow(1.0)
            .min_w(0.0)
            .child(icon("search", p.muted))
            .child(
                text_input(&s.code_picker.query, TextStyle::new(13.0, 420.0))
                    .key("composer-code-language-search")
                    .label("Search languages")
                    .placeholder("Search languages", p.muted)
                    .color(p.ink)
                    .caret(p.coral, p.coral_bg)
                    .grow(1.0)
                    .min_w(0.0)
                    .on_edit(|s: &mut State| &mut s.code_picker.query)
                    .on_input_key(|s: &mut State, event, tx| {
                        if matches!(event.key, rune::Key::Down) {
                            s.code_picker.query.focused = false;
                            tx.focus("composer-code-language-menu", true);
                            return true;
                        }
                        s.undo_key(event, true)
                    }),
            ),
    );

    let mut list = col()
        .key("composer-code-language-menu")
        .role("listbox")
        .label("Languages")
        .focus_group()
        .w_full()
        .max_h(264.0)
        .pb(4.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    for (id, name) in &shown {
        let selected = *id == current;
        let option_key = if id.is_empty() { "plain" } else { id.as_str() };
        let chosen = id.clone();
        let mut item = row()
            .key(format!("composer-code-language-option-{option_key}"))
            .role("option")
            .label(name.clone())
            .checked(selected)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .center_y()
            .gap(8.0)
            .h(28.0)
            .mx(4.0)
            .px(8.0)
            .radius(4.0)
            .hover(p.selected)
            .focus_fill(p.selected)
            .focus_selected(selected)
            .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                choose(s, block_id, &chosen);
                tx.instant();
            })
            .child(
                label(name, TextStyle::new(13.0, 450.0), p.ink)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if selected {
            item = item.child(icon("check", p.sea));
        }
        list = list.child(item);
    }
    if shown.is_empty() {
        list = list.child(
            label("No matching languages.", SMALL, p.muted)
                .px(16.0)
                .py(8.0),
        );
    }

    col()
        .key("composer-code-language-card")
        .role("dialog")
        .label("Code block language")
        .w(224.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .on_press(|_: &mut State| {})
        .child(header)
        .child(list)
        .anchored_fit(
            anchor(s, block_id),
            rune::Side::Bottom,
            rune::Align::Start,
            4.0,
        )
        .focus_return("composer-input")
        .instant_changes()
        .modal_tx(true, |s: &mut State, tx: &mut Tx| {
            s.code_picker.open = None;
            s.code_picker.query = rune::TextEdit::default();
            tx.instant();
        })
}

/// KSG's `changeLanguage`: rewrite only the info string's language token, in one
/// undoable `update_logical_block`, then put the caret at the end of that block's
/// content and focus the editor. Picking the language already shown only closes.
fn choose(s: &mut State, id: rune::BlockId, next: &str) {
    s.code_picker.open = None;
    s.code_picker.query = rune::TextEdit::default();
    let t = s.current_mut();
    let Some(block) = code_blocks(&t.draft).find(|b| b.id == id) else {
        return;
    };
    let info = code_info(&block).to_owned();
    let language = languages::language_of_info_string(&info).to_owned();
    let current = languages::code_language_entry(&language)
        .map(|entry| entry.id)
        .unwrap_or(language.as_str())
        .to_owned();
    // KSG's `onValueChange` ignores the row that is already current.
    if next == current {
        return;
    }
    let code = block.attributes.code().cloned().unwrap_or_default();
    let attributes = block.attributes.clone().with_code(
        rune::CodeAttributes::new(
            code.fence(),
            languages::with_info_string_language(&info, next),
        )
        .with_close(code.close().map(str::to_owned)),
    );
    let end = block.range.end;
    let draft = &mut t.draft;
    if draft.update_logical_block(id, rune::BlockKind::Code, attributes) {
        draft.set_caret(end);
        draft.focused = true;
        composer_ui::changed(s);
        s.checkpoint();
    }
}
