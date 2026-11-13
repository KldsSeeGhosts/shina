# Composer editing parity audit

This change improves editing parity. It does not establish full parity with KSG Code. Do not use the passing tests as a claim that the remaining cases work.

## Reference sources

The reference checkout is `/Volumes/DevDrive/AiStack/ksgcode`. Paths below are relative to its `apps/web/src` directory unless specified otherwise.

| Reference | Rules read |
| --- | --- |
| `components/ComposerPromptEditorTiptap.tsx` | Extension configuration, top-level block input rules, delayed dash-list conversion, selection wrapping, paste, code keys before send, Enter splitting blocks |
| `composer-rich-text-doc.ts` | Mark exclusions, code pairing, selection round-trip guard, block attributes, list normalization, document building, Markdown serialization |
| `composer-rich-text.ts` | Inline parser, unmatched and escaped markers, identifiers, nesting, literal code |
| `composer-code-block.ts` | Two-space indentation, line selection bounds, carried indentation, fence conversion, closing-fence exit, two trailing blank lines |
| `composer-list-continuation.ts` | Authored markers, zero-padded ordered numbers, content-column indentation, outdent, continuation and renumbering |
| `composer-markdown-source.ts` | Editable markers, selection normalization, stored marks, transient history rewrites and edit rebasing |
| `components/composerSelection.ts` | Visible selection updates |
| `components/composerInlineTokenPaste.ts` | Plain-text normalization and context import |
| `components/chat/ComposerCodeBlockLanguagePicker.tsx` | Search by label or id, unknown current language, reset search on close |
| `components/chat/ComposerCodeBlockNodeView.tsx` | Noneditable per-block header, retained info-string suffix, focus at code end after selection |
| `composer-code-languages.ts`, `components/composerCodeBlockHighlight.ts` | Aliases, info-string preservation, per-block syntax decorations |
| `composer-logic.ts` | Literal prefix when pasting into an active path query |
| `packages/shared/src/composerInlineTokens.ts` | File mentions, file-link references, currency skill aliases, numeric exclusions |
| `packages/shared/src/markdownMath.ts` | Formula shielding before style/token parsing |

The input regexes and formatting chords also come from KSG's installed Tiptap 3.31.3 bold, italic, strike and code extensions. KSG overrides underscore italic input, code exclusions and block chords. Shina does not add underline.

## Results

| Requested item | KSG rule and file | Shina implementation | Result |
| --- | --- | --- | --- |
| Formatting chords | `Composer*Extension`, Tiptap shortcuts, source-history toggle | `Rules::key`, `composer_source::toggle_mark_command` | Ported. Mod-B/Mod-Shift-B, Mod-I/Mod-Shift-I, Mod-E, Mod-Shift-S toggle the four marks and collapse shown source before toggling on selections or marker edges. |
| Typed inline delimiters | `markInputRuleOutsideCode`, `ComposerCodeExtension`, Tiptap input regexes | `input_mark`, `open_code_before_text`, `Rules::changed`, `composer_source` | Ported for ordinary typing, delimiter-code exclusions, disabled underscore italic, opening tick typed last, immediate Backspace reversal, filled typed backtick pairs on caret exit, styled-edge caret stops, and source-aware exclusions. |
| Selected text plus opening delimiter | `handleTextInput`, `surroundSelectionWithMark` | `surround`, `source::parse` | Shina layer for plain selections, mark round-trip checks, selection retention, all KSG wrapper pairs, styled-text/pill rejection and literal multiline/code wrapping. Formula shielding and mention-source-boundary checks are not fully ported. |
| Markdown paste and tokens | `handlePaste`, `buildTiptapContent`, shared token collector | `composer_paste::insert`, `paste_with_path_query`, `fragment`, `tokens`, `mention_path` | Shina layer for four inline marks, literal fenced code, CRLF normalization, known file/skill pills, file-link decoding, currency aliases, chip boundary spaces, unknown skills and active-path literal prefixes. Canonical blocks can split top-level prose at the start, middle or end. See deviations below. |
| Markdown sent for every mark/block | `appendInlineRuns`, `serializeDoc`, `codeBlockSource` | `inline`, `markdown_for`, `fence`, `Token::source_for` | Shina layer for the four marks on representable documents. Stable nesting, whitespace trimming, code whitespace, pill marks, quote separation, empty code and closing-line fence collisions now follow KSG. Block metadata needs R1/R2. Existing links remain compatible. |
| Headings, checked tasks, horizontal rules | Heading/task/rule input rules and attributed node types | `Rules::changed`, `Rules::key`, `fragment`, `markdown_for`, `BlockStyle` | Ported on top of the Rune fork (`BlockKind::{Heading, Task, Rule}`, `BlockAttributes`, `BlockStyle`, atomic rule selection, checkbox click). |
| Full list grammar, authored source and nesting | `parseListPrefix`, list normalization, `composer-list-continuation.ts` | `composer_list.rs`, `Rules::changed`, `fragment`, `markdown_for` | Ported with `-`/`*`/`+` markers, ordered start/delimiter, zero-padding, renumbering, KSG Tab/Shift+Tab/Enter-lift and `MAX_LIST_LEVEL` nesting. |
| Fence info, code editing and exits | `composer-code-block.ts`, `codeBlockSource` | `composer_code.rs`, `Rules::key`, `editor_key_before_commands`, `fragment`, `markdown_for` | Ported with Rune `LogicalBlock` + `CodeAttributes` (fence char/length, info string, exact close/unclosed state, distinct adjacent fences). |
| Language picker, retained language, highlighting | `ComposerCodeBlockLanguagePicker`, `ComposerCodeBlockNodeView`, `composer-code-languages.ts`, highlight extension | `composer_code_picker.rs`, `composer_code_languages.rs`, Rune `InputBlockView` | Picker ported: a noneditable header on each code block carries the language trigger, the searchable popover anchors under it, and choosing a language rewrites only the info-string token before focusing the editor at the block's end. Per-block syntax highlighting remains open. |
| Editable source at caret and reparse on leave | `composer-markdown-source.ts`, styled-edge helpers | `composer_source.rs`, `ui.rs` (`on_input_selection_change`) | Ported on top of the Rune fork (`on_input_selection_change`, `AppMark`, `TransactionOptions::Transient`, `EditMap` selection mapping, and `preview_decorations`). |

## Pinned Rune findings

The dependency remains unchanged at `22fcaf167b5e8d74e2ab173a698b7cf2053c5889`.

Read APIs include `crates/rune/src/edit.rs`, `edit/model.rs`, `edit/marks.rs`, `edit/input.rs`, `edit/history.rs`, `edit/html.rs`, `edit/display.rs`, `el/input.rs`, `el/text.rs`, `text/span.rs`, `text/rich_layout.rs`, and the input callback routing in `runtime/input/keys.rs`. Rune Kit's `markdown.rs` and its parser/rendering modules are display components, not editable `TextEdit` documents.

`Block` is a Copy enum containing `Paragraph`, `Bullet(u8)`, `Numbered(u8)`, `Quote` and `Code`. Its maximum list level is 3. Attributes are one value per newline-delimited line. Consecutive code lines have no separate logical fence identity.

`Mark` contains bold, italic, underline, strike, code, link and pill. There is no app-defined nonvisual mark. Code blocks strip every inline mark, so a link/pill cannot safely carry code metadata. Empty blocks cannot carry a nonempty mark range at all.

`RichStyle` sets colors for code, code backgrounds, links, quote bars and list markers. `El::decorations` accepts `Span` values with size, weight, italic, mono and colors. These can style visible source or highlight code. They cannot replace a text range with a block/widget, change its caret semantics or attach a noneditable language header.

`on_input_change` fires only if the text string changes. It does not report selection-only movement or mark-only changes. `on_edit` is a mutable-model accessor used repeatedly during runtime input processing, not an after-selection transaction notification.

`TextEdit::undo_group` groups document snapshots. Each snapshot contains text, selection, marks, blocks and soft breaks. It cannot include app-owned attributes, distinguish equal-text code fences or map history through transient text rewrites. `set_marks`/`set_blocks` can participate in an enclosing undo group, which the paste implementation uses.

## Upstream API requests

These are proposed APIs, not existing Rune methods. Their names make the required contract explicit.

### R1. Editable block kinds and authored attributes

Add `Block::Heading { level: u8 }`, `Block::Task { level: u8, checked: bool }` and `Block::Rule`.

Add `BlockAttributes` and:

```rust
TextEdit::block_attributes(line: usize) -> &BlockAttributes
TextEdit::set_block_attributes(line: usize, attributes: BlockAttributes)
```

`BlockAttributes` must retain authored `indent`, `marker`, `space`, task `marker_space`/`content_space`, quote `prefix`, and heading `space`. Task edits must expose checked-state updates. The setter must record undo and redo. Text edits, splitting, joining, indent/outdent and rich-copy/persistence must map these attributes without discarding them. Nesting cannot be capped at three if KSG parity is required.

### R2. Logical block identity and code attributes

Add `BlockId`, `LogicalBlock { id, range, kind, attributes }` and:

```rust
TextEdit::logical_blocks() -> impl Iterator<Item = LogicalBlock>
TextEdit::set_logical_block(range: Range<usize>, kind: Block, attributes: BlockAttributes)
```

Logical code blocks need stable identity even when adjacent and equal in text. Code attributes must retain `fence`, the complete `info` string, and exact `close`, including an absent close, whitespace and fence character/length. Empty blocks need the same attributes. All must participate in text edits, undo, redo and persistence.

This is also needed to decide whether a selection is in one fence. Grouping contiguous `Block::Code` lines cannot answer that question for adjacent fences.

### R3. Editable block view integration

Add a block-view builder such as:

```rust
El::input_block_view(builder: impl Fn(BlockId, &BlockAttributes) -> InputBlockView)
```

`InputBlockView` needs a noneditable header/widget slot, editable-content layout, and a task checkbox/rule renderer. Pointer/key events on the header must not move the editor selection. Headers must track the block while it wraps or scrolls. Selection and navigation must still work across blocks in the same editor. This supports KSG's language combobox and task interaction without replacing the composer with independent text fields.

Rules also need an atomic selection. Add `EditorSelection::Block(BlockId)`, `TextEdit::select_block(BlockId)` and `TextEdit::selection() -> EditorSelection`. Clicking a rule must select that block; navigation, replacement, Backspace/Delete, undo and redo must treat it as one object rather than an empty paragraph.

No upstream syntax-highlighting method is needed. `El::decorations` is sufficient once the app can retrieve the code language and content range.

### R4. Selection and history-aware source transformations

Add:

```rust
El::on_input_selection_change(callback)
TextEdit::set_app_mark(range: Range<usize>, mark: AppMark)
TextEdit::transact(options: TransactionOptions, edit: impl FnOnce(&mut TextEdit))
```

The selection callback must run after keyboard, pointer, accessibility, IME, programmatic selection and history movement. `AppMark` needs an id, inclusive/noninclusive boundaries, no intrinsic styling, and persistence/history support.

`TransactionOptions` must permit a transient rewrite that preserves redo and returns an `EditMap`. Recorded history and selections must be rebasable through that map. KSG reveals/collapses source after undo without clearing redo, then replays a new recorded edit against the history's original document shape. A history-free text assignment or clearing Rune history is not equivalent.

These methods also enable filled typed backtick pairs to convert on caret exit and offer the two mark states at styled edges.

## Shina-layer alternatives assessed

Keeping full Markdown in `TextEdit.text` would retain arbitrary metadata. Spans could make headings larger and color code. But Rune cannot hide source ranges while removing their caret stops, render checked task nodes or a selectable rule in their place, or put a language control inside a logical block. Leaving those markers visible everywhere changes KSG's editor.

An external attribute table keyed by line/text cannot distinguish two equal-text fences or two history states with different language/list metadata. Native/context-menu edits and undo use Rune's private snapshots. Intercepting a few keyboard chords would not cover them.

Independent per-block `TextEdit` fields could render headers and checkboxes. They would lose cross-block selection, one document undo stack, cursor movement and IME continuity. That is not a faithful implementation either.

The shipped fallback preserves unsupported pasted source rather than stripping attributes. It is explicitly a fallback, not rich-block parity.

## Fork dependency

Shina builds against the local Rune fork via `[patch]` entries in `Cargo.toml` (adding `BlockKind`, `BlockAttributes`, `BlockStyle`, `LogicalBlock`/`CodeAttributes`, `EditorSelection::Block`, `MAX_LIST_LEVEL = u8::MAX`, `AppMark`, `TransactionOptions`/`EditMap`, `on_input_selection_change`, and `InputBlockView`). The composer uses `InputBlockView::header` for the code-block language picker headers described in `FORK.md`.

## Deviations and remaining work

- Shina splits top-level prose around representable pasted blocks through newline insertion and block attributes. The split rules were checked against KSG's installed `insertContentAt`/ProseMirror `replaceWith`. Pasting structural content across nested/mixed block selections is not fully verified.
- Ordinary source parser cases match the copied tests, but KSG's complete math shielding and context/model/citation token import/reconciliation are not ported. File and skill tokens are the supported Shina types. Formula/mention-boundary guards therefore do not establish full selection-wrap parity.
- Native Rune undo grouping remains in use outside explicit input-rule reversal, grouped paste, and `TransactionOptions::Transient` source reveal/collapse.
- The code-block language picker is ported; per-block syntax highlighting remains open (`composerCodeBlockHighlight.ts` is not ported).
- Legacy Shina link marks still serialize as links. KSG disables new link marks. Existing file pills without an authored source keep their previous expanded-path transport; newly pasted file references retain their authored source.
- `BlockStyle` does not express a separate checked checkbox fill color/check glyph (`checkbox_color` sets both frame and fill in `ink`), per-heading line-height (`1.3`) or `h6` muted color, or vertical margins around rules (`0.65rem`).

## Verification

The first six parity tests failed against the original implementation. Paste tests also failed against plain insertion before implementation. Regression tests include KSG's own inline parser, selection surround, path-query paste and code indentation cases, plus document and Markdown assertions, paste undo/redo, structural paste around prose, source-preserving block fallbacks and token boundaries.

The headless UI regression covers 620, 960, 1280 and 1920 px. It verifies resolved skill pills, unknown skill text, composer bounds and code Enter before send. A second regression puts a draft with two code blocks through the language picker at the same widths: each header trigger stays inside the composer card, the open menu stays inside the window, choosing a language rewrites only that block's fence, the menu closes on selection, and a reopened menu's Escape changes nothing.

Final checks passed:

- `cargo fmt --check`
- `cargo build`
- `cargo test -- --quiet`, 895 unit tests and 40 integration tests passed. Eleven pre-existing unit tests remained ignored.
- All 22 focused editor tests passed. This includes 18 new editor test functions; one additional new UI test covers all four widths.

Native IME, bidi and screen-reader behavior remain unverified. Tests simulate editing and use headless layout; they do not replace those native checks. No installed app, running Shina process, live user data or CPA service was changed.

Implementation commits:

- `a003af6` adds the initial failing parity tests.
- `8007652` ports inline rules, serialization and code commands.
- `e546478` adds Markdown paste, token-source retention, structural paste and integration tests.
