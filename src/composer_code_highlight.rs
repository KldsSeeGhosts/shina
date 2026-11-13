//! Composer code-block syntax colours, ported from KSG Code's
//! `composerCodeBlockHighlight.ts`. A fenced block's content takes the transcript's
//! `syntax_theme`, so it reads the same in the composer and after it is sent. The
//! spans are view-only decorations: they never enter the serialized Markdown.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::time::{Duration, Instant};

use rune::text::Span;
use rune::{BlockId, BlockKind, LogicalBlock, TextEdit};
use rune_kit::syntax::{Highlight, SyntaxTheme};

use crate::composer_code_languages::language_of_info_string;

/// Tokenizing larger fences can stall the main thread.
const MAX_HIGHLIGHTED_CODE_BLOCK_LENGTH: usize = 20_000;

/// Keep the mapped colours while typing in long fences, then refresh after a pause.
const DEFERRED_HIGHLIGHT_CODE_BLOCK_LENGTH: usize = 2_000;
const DEFERRED_HIGHLIGHT_DELAY: Duration = Duration::from_millis(300);

/// KSG's plugin state: a signature cache (`theme, info, text`) of per-block
/// highlights, plus the spans last emitted for each block so a deferred refresh can
/// keep them mapped through typing. `LogicalBlock::id` survives the edits that move
/// or rewrite a block.
#[derive(Default)]
pub struct ComposerCodeHighlight {
    cache: HashMap<u64, Cached>,
    drawn: HashMap<BlockId, Drawn>,
    doc: u64,
    dark: Option<bool>,
    deferred_until: Option<Instant>,
}

/// KSG's `HighlightedBlock`: the token ranges of one signature.
struct Cached {
    dark: bool,
    info: Box<str>,
    text: Box<str>,
    spans: Box<[Highlight]>,
}

/// What one block last drew: its content and content-relative highlights. A
/// deferred refresh maps them through the edit like `value.map` does in KSG.
struct Drawn {
    text: Box<str>,
    spans: Box<[Highlight]>,
}

/// KSG's `blockSignature` (theme, info string, text), hashed. Signing by the full
/// info string keeps blocks that differ only in fence metadata apart.
fn signature(dark: bool, info: &str, text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    (dark, info, text).hash(&mut h);
    h.finish()
}

/// Text plus block identity, kind, range and attributes, so a language change made
/// by `update_logical_block` (no text change) still counts as a document change.
fn fingerprint(edit: &TextEdit, blocks: &[LogicalBlock]) -> u64 {
    let mut h = DefaultHasher::new();
    edit.text.hash(&mut h);
    for b in blocks {
        (b.id, b.kind, &b.range, &b.attributes).hash(&mut h);
    }
    h.finish()
}

fn code_info(block: &LogicalBlock) -> &str {
    block
        .attributes
        .code()
        .map_or("", rune::CodeAttributes::info)
}

/// `from` maps with `MapBias::After`, `to` with `MapBias::Before`, as `map_pairs`:
/// a position inside the deleted range moves to the insertion's end or start, a
/// position after it shifts by the length delta.
fn map_range(r: &Range<usize>, del: &Range<usize>, ins_len: usize) -> Option<Range<usize>> {
    let del_len = del.len();
    let start = if r.start >= del.start && r.start <= del.end {
        del.start + ins_len
    } else if r.start > del.end {
        r.start - del_len + ins_len
    } else {
        r.start
    };
    let end = if r.end >= del.start && r.end <= del.end {
        del.start
    } else if r.end > del.end {
        r.end - del_len + ins_len
    } else {
        r.end
    };
    (start < end).then_some(start..end)
}

/// `theme.spans` for `spans`, offset to `range` in `edit.text` and clipped to it:
/// colours never leave the block's content, never reaching a header or the rest of
/// the draft.
fn emit(out: &mut Vec<Span>, theme: &SyntaxTheme, spans: &[Highlight], range: &Range<usize>) {
    out.extend(
        theme
            .spans(spans, range.start)
            .into_iter()
            .filter_map(|mut sp| {
                sp.start = sp.start.clamp(range.start as u32, range.end as u32);
                sp.end = sp.end.clamp(range.start as u32, range.end as u32);
                (sp.start < sp.end).then_some(sp)
            }),
    );
}

impl Drawn {
    /// KSG's `value.map(transaction.mapping)` for one block: move the spans it last
    /// drew through the single-edit diff between then and now.
    fn map_to(&mut self, now: &str) {
        if self.text.as_ref() == now {
            return;
        }
        let (del, ins_len) = crate::composer_source::diff_single_edit(&self.text, now);
        let mut spans = Vec::with_capacity(self.spans.len());
        for (r, paint) in self.spans.iter() {
            if let Some(mapped) = map_range(r, &del, ins_len) {
                spans.push((mapped, *paint));
            }
        }
        self.spans = spans.into_boxed_slice();
        self.text = now.into();
    }
}

impl ComposerCodeHighlight {
    /// Colour spans for `edit`'s fenced code blocks, in `edit.text` byte offsets, to
    /// merge into the composer input's decorations after `preview_decorations`.
    /// `now` drives the deferred refresh for long fences.
    pub fn spans(
        &mut self,
        edit: &TextEdit,
        dark: bool,
        theme: &SyntaxTheme,
        now: Instant,
    ) -> Vec<Span> {
        self.spans_with(edit, dark, theme, now, &rune_kit::syntax::highlight)
    }

    /// The wait until a deferred block wants fresh spans, for `Tx::wake_after`.
    pub fn deferred_refresh_due(&self, now: Instant) -> Option<Duration> {
        self.deferred_until
            .map(|until| until.saturating_duration_since(now))
    }

    /// `spans` with the tokenizer injected so tests can count tokenizations.
    fn spans_with(
        &mut self,
        edit: &TextEdit,
        dark: bool,
        theme: &SyntaxTheme,
        now: Instant,
        highlight: &dyn Fn(Option<&str>, &str) -> std::rc::Rc<[Highlight]>,
    ) -> Vec<Span> {
        let all: Vec<LogicalBlock> = edit.logical_blocks().collect();
        let doc = fingerprint(edit, &all);
        // The first scan is KSG's `refresh(false)` at view creation: it tokenizes
        // every missing block, so a long fence opens already coloured.
        let init = self.dark.is_none();
        let changed = doc != self.doc;
        let theme_changed = self.dark.is_some_and(|d| d != dark);
        self.doc = doc;
        self.dark = Some(dark);
        let previous_until = self.deferred_until.take();
        let defer_active = previous_until.is_some_and(|u| now < u);

        if changed || theme_changed {
            // Prune obsolete signatures each scan so the cache is bounded by the
            // draft, and drop the drawn spans of blocks that left it.
            let live: HashSet<u64> = all
                .iter()
                .filter(|b| b.kind == BlockKind::Code)
                .map(|b| signature(dark, code_info(b), &edit.text[b.range.clone()]))
                .collect();
            self.cache.retain(|sig, _| live.contains(sig));
            self.drawn
                .retain(|id, _| all.iter().any(|b| b.kind == BlockKind::Code && b.id == *id));
        }

        let mut out = Vec::new();
        for b in all.iter().filter(|b| b.kind == BlockKind::Code) {
            let Some(text) = edit.text.get(b.range.clone()) else {
                continue;
            };
            // KSG's `node.textContent.length` counts UTF-16 code units.
            let len = text.encode_utf16().count();
            if len > MAX_HIGHLIGHTED_CODE_BLOCK_LENGTH {
                self.drawn.remove(&b.id);
                continue;
            }
            let info = code_info(b);
            let sig = signature(dark, info, text);
            let hit = self
                .cache
                .get(&sig)
                .filter(|c| c.dark == dark && c.info.as_ref() == info && c.text.as_ref() == text)
                .map(|c| c.spans.clone());
            if let Some(spans) = hit {
                emit(&mut out, theme, &spans, &b.range);
                self.remember(b.id, text, &spans);
                continue;
            }
            // Long fences keep their mapped colours while typing and refresh after
            // the pause, KSG's `deferLong`. A theme change repaints at once.
            let defer = !init
                && !theme_changed
                && len > DEFERRED_HIGHLIGHT_CODE_BLOCK_LENGTH
                && (changed || defer_active);
            if defer {
                self.deferred_until = Some(if changed || !defer_active {
                    now + DEFERRED_HIGHLIGHT_DELAY
                } else {
                    previous_until.unwrap_or(now + DEFERRED_HIGHLIGHT_DELAY)
                });
                if let Some(d) = self.drawn.get_mut(&b.id) {
                    d.map_to(text);
                    emit(&mut out, theme, &d.spans, &b.range);
                }
                continue;
            }
            // Stored info includes metadata after the language; Highlights needs
            // only the name. As KSG leaves them, unknown or empty languages draw
            // plain, and that result caches so failed looks do not retry per frame.
            let lang = language_of_info_string(info);
            let spans: Box<[Highlight]> = if rune_kit::syntax::known(lang) {
                Vec::from(&*highlight(Some(lang), text)).into()
            } else {
                Vec::new().into()
            };
            emit(&mut out, theme, &spans, &b.range);
            self.remember(b.id, text, &spans);
            self.cache.insert(
                sig,
                Cached {
                    dark,
                    info: info.into(),
                    text: text.into(),
                    spans,
                },
            );
        }
        out
    }

    fn remember(&mut self, id: BlockId, text: &str, spans: &[Highlight]) {
        match self.drawn.get_mut(&id) {
            Some(d) if d.text.as_ref() == text => d.spans = spans.into(),
            _ => {
                self.drawn.insert(
                    id,
                    Drawn {
                        text: text.into(),
                        spans: spans.into(),
                    },
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "composer_code_highlight_tests.rs"]
mod tests;
