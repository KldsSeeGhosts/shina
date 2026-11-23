use super::*;

pub(super) fn assistant_turn(s: &State, p: Palette, wide: bool, start: usize, end: usize) -> El {
    assistant_turn_piece(s, p, wide, start, end, None)
}

/// One virtual row of an assistant turn. `None` is the whole turn, as short turns
/// show; a long live or expanded turn is a head, bounded work slices and a tail.
pub(super) fn assistant_turn_piece(
    s: &State,
    p: Palette,
    wide: bool,
    start: usize,
    end: usize,
    piece: Option<TurnPiece>,
) -> El {
    let t = s.current();
    let structure = timeline_structure(t);
    if let Some(TurnPiece::Work { lo, hi }) = piece {
        return work_slice(s, p, wide, start, end, lo, hi);
    }
    if t.transcript_structure == 0
        || end == t.messages.len() && t.status.busy()
        || structure.external_turns.contains(&start)
    {
        return build_assistant_turn(s, p, wide, start, end, piece);
    }
    let revision = t.timeline_cache.borrow().turn_revision(start);
    let captain = t
        .captain
        .and_then(|id| s.captains.get(id))
        .map(|c| (&c.name, c.portrait));
    let find = s.thread_find_active().then(|| {
        (
            s.thread_find.query.text.as_str(),
            s.thread_find
                .active_match
                .as_ref()
                .map(|m| (&m.entry_id, m.occurrence)),
        )
    });
    let cite = s
        .citation_highlight
        .as_ref()
        .map(|h| (&h.machine, h.thread_id, &h.message_id, &h.text));
    // The response actions: a Copy check on one of this turn's messages, and whether Fork shows.
    let copied = s.copied_message.as_ref().and_then(|(id, _)| {
        t.messages[start..end.min(t.messages.len())]
            .iter()
            .any(|m| &m.id == id)
            .then_some(id.as_str())
    });
    let actions = (
        copied,
        t.session.is_some(),
        t.can_change_conversation(),
        t.can_rewind_conversation(),
        t.status.busy(),
        t.configuring,
    );
    memo(
        format!(
            "assistant-turn-{}{}",
            message_key(t, start),
            piece_suffix(piece)
        ),
        (
            (structure.version, revision, start, end),
            (
                s.dark,
                wide,
                s.code_wrap_version,
                rune::memo::layout_with("chat-lane", |rect| rect[2].to_bits()),
                s.viewport_width.to_bits(),
                s.demo,
            ),
            (
                captain,
                t.captain_home,
                t.archived(),
                t.native.compaction_expanded,
            ),
            (t.harness.as_str(), s.working_directory(), find, cite),
            actions,
        ),
        || build_assistant_turn(s, p, wide, start, end, piece),
    )
}

pub(super) fn piece_suffix(piece: Option<TurnPiece>) -> &'static str {
    match piece {
        None | Some(TurnPiece::Head) => "",
        Some(TurnPiece::Work { .. }) => "-work",
        Some(TurnPiece::Tail) => "-tail",
    }
}

/// An HTML card keeps its own width and is lifted out of a folded turn.
pub(super) fn is_html_card(t: &Thread, part: &TranscriptPart) -> bool {
    matches!(part, TranscriptPart::Tools(indices) if indices.iter().all(|&i|
            t.tools[i].resolved_kind() == crate::work::ToolKind::Html
            && crate::html_render::find_reference(&t.tools[i].structured).is_some()))
}

/// A captain's delegations are its answer, not its working: their subagent rows stay in view
/// when the rest of the turn folds into "Worked for". It reads the thread only, so the
/// timeline structure can split turns without the window state.
pub(super) fn is_lifted_card(t: &Thread, part: &TranscriptPart) -> bool {
    (t.captain_home
        && !t.delegated
        && matches!(part, TranscriptPart::Tools(indices)
            if indices.iter().all(|&index| crew_ui::is_delegation(&t.tools[index]))))
        || is_html_card(t, part)
}

/// The last work part and the preamble above the fold.
pub(super) fn turn_shape(t: &Thread, parts: &[TranscriptPart]) -> (Option<usize>, Option<usize>) {
    let last_work = parts
        .iter()
        .rposition(|part| !matches!(part, TranscriptPart::Text { .. }) && !is_lifted_card(t, part));
    // Initial thinking may precede the preamble on the wire. Keep that thinking in
    // the work section, and the first text before subsequent work above its fold.
    let preamble = last_work.and_then(|last| {
        parts[..last]
            .iter()
            .position(|part| matches!(part, TranscriptPart::Text { .. }))
            .filter(|index| {
                !parts[..*index].iter().any(|part| match part {
                    TranscriptPart::Compaction(_) | TranscriptPart::Retry(_) => true,
                    TranscriptPart::Tools(indices) => indices
                        .iter()
                        .any(|i| t.tools[*i].resolved_kind() == crate::work::ToolKind::Plan),
                    _ => false,
                })
            })
    });
    (last_work, preamble)
}

#[derive(Clone, Copy)]
pub(super) struct TurnLayout {
    pub(super) last_work: Option<usize>,
    pub(super) preamble: Option<usize>,
    pub(super) live: bool,
    pub(super) inline_work: bool,
}

/// KSG's `deriveTimelineRows` leaves commentary between tool streaks visible even
/// when individual streaks are folded. Keep the turn-wide control for stopped
/// runs and for deferred history, which still needs an explicit load.
pub(super) fn turn_layout(
    t: &Thread,
    structure: &TimelineStructure,
    start: usize,
    end: usize,
    parts: &[TranscriptPart],
) -> TurnLayout {
    let (last_work, mut preamble) = turn_shape(t, parts);
    let live = end == t.messages.len() && t.status.busy();
    let mut saw_tools = false;
    let mut saw_commentary = false;
    let inline_work = !structure.deferred_work.contains_key(&start)
        && !structure
            .work_index
            .get(&start)
            .is_some_and(|&index| t.work[index].stopped)
        && parts.iter().any(|part| match part {
            TranscriptPart::Work(entries) if entry_tools(entries).next().is_some() => {
                saw_tools = true;
                saw_commentary
            }
            TranscriptPart::Text { message, from, to }
                if saw_tools && !t.messages[*message].text[*from..*to].trim().is_empty() =>
            {
                saw_commentary = true;
                false
            }
            _ => false,
        });
    if inline_work {
        preamble = None;
    }
    TurnLayout {
        last_work,
        preamble,
        live,
        inline_work,
    }
}

/// The conversation lane's gap between rows. A split turn's slices and tail rise
/// into it from inside their own row boxes, so the virtual list's measured heights
/// still match where rows sit, and threads without split turns keep their geometry.
pub(super) const LANE_GAP: f32 = 20.0;

/// A turn at or above this many work entries splits into slice rows while it runs or
/// while its work is open. Shorter turns stay one row with their original layout.
pub(super) const SEGMENT_MIN_ITEMS: usize = 48;
#[cfg(test)]
thread_local! {
    static NEVER_SPLIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn segment_min_items() -> usize {
    #[cfg(test)]
    if NEVER_SPLIT.with(std::cell::Cell::get) {
        return usize::MAX;
    }
    SEGMENT_MIN_ITEMS
}

/// Work entries per slice row. Rune mounts the viewport and two viewports either
/// side, so mounted entries follow the viewport, not the turn's length.
pub(super) const SLICE_ITEMS: usize = 4;

/// One entry of a turn's work column, in display order. Unfolded work streaks and thought
/// runs contribute their heading and each row separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkItem {
    Part(usize),
    /// A streak's folded or unfolded header.
    GroupHead(usize),
    /// Step `k` of an unfolded streak.
    GroupEntry(usize, usize),
    ThoughtHead(usize),
    ThoughtBody(usize, usize),
    /// A running turn's tail (`work-live`).
    Live(usize),
}

impl WorkItem {
    pub(super) fn part(self) -> usize {
        match self {
            Self::Part(part)
            | Self::GroupHead(part)
            | Self::GroupEntry(part, _)
            | Self::ThoughtHead(part)
            | Self::ThoughtBody(part, _)
            | Self::Live(part) => part,
        }
    }
}

/// Whether a part in a running turn is current activity or a live card that stays visible even
/// when earlier work history is folded behind the `Working for` bar.
pub(super) fn live_part_stays_visible(t: &Thread, parts: &[TranscriptPart], index: usize) -> bool {
    let at_end = index + 1 == parts.len();
    match &parts[index] {
        TranscriptPart::Work(entries) => {
            at_end
                || entries.iter().any(|entry| match *entry {
                    WorkEntry::Tool(i) => !t.tools[i].done,
                    WorkEntry::Thought(i) => !t.thoughts[i].done,
                })
        }
        TranscriptPart::Tools(indices) => {
            at_end
                || is_lifted_card(t, &parts[index])
                || indices.iter().any(|&i| {
                    let tool = &t.tools[i];
                    !tool.done || tool.resolved_kind() == crate::work::ToolKind::Plan
                })
        }
        TranscriptPart::Compaction(i) => {
            at_end
                || t.native
                    .compactions
                    .get(*i)
                    .is_some_and(|item| item["running"] == true)
        }
        TranscriptPart::Retry(i) => {
            at_end
                || crate::work::retry_events(t)
                    .get(*i)
                    .is_some_and(|event| event["running"] == true)
        }
        TranscriptPart::Text { .. } | TranscriptPart::Thought(_) => at_end,
    }
}

/// Whether a running turn has earlier work history before its live tail that can fold behind the
/// 32 pt `Working for` bar.
pub(super) fn live_work_foldable(t: &Thread, parts: &[TranscriptPart], layout: TurnLayout) -> bool {
    if !layout.live {
        return false;
    }
    if t.live_work_collapsed {
        return true;
    }
    let Some(last) = layout.last_work else {
        return false;
    };
    parts[..=last].iter().enumerate().any(|(index, _)| {
        Some(index) != layout.preamble && !live_part_stays_visible(t, parts, index)
    })
}

/// A running turn's work column ends in its live tail, which folds into one `work-live` row
/// and unfolds below it. Before prose, and in a finished turn, a streak of two or more steps
/// folds under its header (KSG's `work-toggle`); a lone call stays its own row.
pub(super) fn work_items(
    t: &Thread,
    parts: &[TranscriptPart],
    layout: TurnLayout,
) -> Vec<WorkItem> {
    let Some(last) = layout.last_work else {
        return vec![];
    };
    let tail = layout.live.then(|| live_tail(t, parts)).flatten();
    let mut items = Vec::with_capacity(last + 1);
    for (index, part) in parts[..=last].iter().enumerate() {
        if Some(index) == layout.preamble
            || !layout.live && !layout.inline_work && is_lifted_card(t, part)
        {
            continue;
        }
        if layout.live && t.live_work_collapsed && !live_part_stays_visible(t, parts, index) {
            continue;
        }
        match part {
            TranscriptPart::Work(entries) if layout.live && index + 1 == parts.len() => {
                // The live tail. With nothing to show yet, the Thinking row speaks for it.
                if tail.is_none() {
                    continue;
                }
                items.push(WorkItem::Live(index));
                if group_open(t, entries) {
                    let thinking = entry_tools(entries).next().is_none();
                    items.extend((0..entries.len()).map(|k| {
                        if thinking {
                            WorkItem::ThoughtBody(index, k)
                        } else {
                            WorkItem::GroupEntry(index, k)
                        }
                    }));
                }
            }
            TranscriptPart::Work(entries) if entry_tools(entries).next().is_none() => {
                if t.thoughts[entry_thoughts(entries)[0]].expanded {
                    items.push(WorkItem::ThoughtHead(index));
                    items.extend((0..entries.len()).map(|k| WorkItem::ThoughtBody(index, k)));
                } else {
                    items.push(WorkItem::Part(index));
                }
            }
            TranscriptPart::Work(entries) if entries.len() > 1 => {
                items.push(WorkItem::GroupHead(index));
                if group_open(t, entries) {
                    items.extend((0..entries.len()).map(|k| WorkItem::GroupEntry(index, k)));
                }
            }
            _ => items.push(WorkItem::Part(index)),
        }
    }
    items
}

/// The space above a work entry.
pub(super) fn work_item_gap(item: WorkItem) -> f32 {
    match item {
        WorkItem::ThoughtBody(..) => 4.0,
        _ => WORK_GAP,
    }
}

/// The key part of an entry that does not move when entries are added after it.
pub(super) fn work_item_id(t: &Thread, parts: &[TranscriptPart], item: WorkItem) -> String {
    let thought_id = |index: usize, k: usize| {
        let thought = &t.thoughts[index];
        let message = t
            .messages
            .get(thought.anchor.message)
            .map_or("end", |message| message.id.as_str());
        format!(
            "thought-{message}-{}-{}-{k}",
            thought.block, thought.anchor.offset
        )
    };
    let entry_id = |entry: WorkEntry, k: usize| match entry {
        WorkEntry::Tool(index) => format!("tool-{}", t.tools[index].id),
        WorkEntry::Thought(index) => thought_id(index, k),
    };
    match (&parts[item.part()], item) {
        (TranscriptPart::Work(entries), WorkItem::GroupEntry(_, k)) => entry_id(entries[k], 0),
        (TranscriptPart::Work(entries), WorkItem::ThoughtBody(_, k)) => entry_id(entries[k], k + 1),
        (TranscriptPart::Work(entries), WorkItem::Live(_)) => {
            format!("live-{}", entry_id(entries[0], 0))
        }
        (TranscriptPart::Work(entries), _) => match entries[0] {
            WorkEntry::Tool(index) => format!("tools-{}", t.tools[index].id),
            WorkEntry::Thought(index) => thought_id(index, 0),
        },
        (TranscriptPart::Tools(indices), _) => format!("tools-{}", t.tools[indices[0]].id),
        (TranscriptPart::Thought(indices), _) => thought_id(indices[0], 0),
        (TranscriptPart::Text { message, from, .. }, _) => {
            format!("text-{}-{from}", t.messages[*message].id)
        }
        (TranscriptPart::Compaction(index), _) => format!("compaction-{index}"),
        (TranscriptPart::Retry(index), _) => format!("retry-{index}"),
    }
}

/// Whether a turn shows its work column now: always while it runs, and while its
/// "Worked for" fold is open with its history loaded.
pub(super) fn turn_work_open(
    t: &Thread,
    structure: &TimelineStructure,
    start: usize,
    live: bool,
) -> bool {
    if live {
        return true;
    }
    let expanded = structure
        .work_index
        .get(&start)
        .and_then(|index| t.work.get(*index))
        .is_some_and(|work| work.expanded);
    let (failed, loading) = structure
        .deferred_work
        .get(&start)
        .copied()
        .unwrap_or_default();
    expanded && !failed && !loading
}

/// Matches before part `index` in the same message, so a slice numbers its find
/// highlights as the whole message does.
pub(super) fn find_occurrence_base(
    t: &Thread,
    parts: &[TranscriptPart],
    index: usize,
    query: &str,
) -> usize {
    let TranscriptPart::Text { message, .. } = parts[index] else {
        return 0;
    };
    parts[..index]
        .iter()
        .filter_map(|part| match part {
            TranscriptPart::Text {
                message: m,
                from,
                to,
            } if *m == message => Some(
                crate::thread_find::find_markdown_occurrences(
                    &t.messages[*m].text[*from..*to],
                    query,
                )
                .len(),
            ),
            _ => None,
        })
        .sum()
}

/// One work entry of the work column.
pub(super) fn work_item(
    s: &State,
    p: Palette,
    wide: bool,
    parts: &[TranscriptPart],
    item: WorkItem,
    start: usize,
) -> El {
    let t = s.current();
    let part = &parts[item.part()];
    match (part, item) {
        (TranscriptPart::Work(entries), WorkItem::GroupHead(_)) => work_group_head(s, p, entries),
        (TranscriptPart::Work(entries), WorkItem::GroupEntry(_, k)) => col()
            .min_w(0.0)
            .pl(WORK_LIST_INDENT)
            .child(work_entry_row(s, p, entries[k], true)),
        (TranscriptPart::Work(entries), WorkItem::ThoughtHead(_)) => col()
            .min_w(0.0)
            .child(work_ui::thought_head(s, p, &entry_thoughts(entries), true)),
        (TranscriptPart::Work(entries), WorkItem::ThoughtBody(_, k)) => col()
            .min_w(0.0)
            .child(work_ui::thought_body(s, p, entry_thoughts(entries)[k])),
        (TranscriptPart::Work(_), WorkItem::Live(_)) => match live_tail(t, parts) {
            Some(tail) => col().min_w(0.0).child(live_work(s, p, parts, tail, start)),
            None => el(),
        },
        (part, _) => {
            let query = (s.thread_find_active() && !s.thread_find.query.text.trim().is_empty())
                .then(|| s.thread_find.query.text.trim());
            let mut base = query.map_or(0, |query| {
                find_occurrence_base(t, parts, item.part(), query)
            });
            let row = transcript_part(s, p, wide, part, &mut base);
            if matches!(part, TranscriptPart::Text { .. }) {
                row.py(6.0)
            } else {
                row
            }
        }
    }
}

/// A turn's work entries as one column, `gap` above each as the slices space them.
pub(super) fn work_column(
    s: &State,
    p: Palette,
    wide: bool,
    parts: &[TranscriptPart],
    items: &[WorkItem],
    start: usize,
    key: String,
) -> El {
    let mut rows = col().key(key).min_w(0.0);
    for (index, item) in items.iter().enumerate() {
        let row = work_item(s, p, wide, parts, *item, start);
        rows = rows.child(if index == 0 {
            row
        } else {
            col().min_w(0.0).pt(work_item_gap(*item)).child(row)
        });
    }
    rows
}

/// What a slice's entries show, so a slice rebuilds only when one of them changed.
pub(super) fn work_slice_fingerprint(
    t: &Thread,
    parts: &[TranscriptPart],
    items: &[WorkItem],
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for item in items {
        item.hash_fields(&mut hasher);
        match &parts[item.part()] {
            TranscriptPart::Tools(indices) => {
                for &i in indices {
                    let tool = &t.tools[i];
                    (
                        &tool.id,
                        tool.done,
                        tool.failed,
                        tool.expanded,
                        tool.raw_expanded,
                        tool.group_expanded,
                        tool.output.len(),
                        tool.detail.len(),
                        tool.changes.len(),
                        tool.images.len(),
                        tool.subagents.len(),
                        tool.exit_code,
                    )
                        .hash(&mut hasher);
                }
            }
            TranscriptPart::Thought(indices) => {
                for &i in indices {
                    let thought = &t.thoughts[i];
                    (thought.text.len(), thought.done, thought.expanded).hash(&mut hasher);
                }
            }
            TranscriptPart::Work(entries) => {
                for entry in entries {
                    match *entry {
                        WorkEntry::Tool(i) => {
                            let tool = &t.tools[i];
                            (
                                &tool.id,
                                tool.done,
                                tool.failed,
                                tool.expanded,
                                tool.raw_expanded,
                                tool.group_expanded,
                                tool.output.len(),
                                tool.detail.len(),
                                tool.changes.len(),
                                tool.images.len(),
                                tool.subagents.len(),
                                tool.exit_code,
                            )
                                .hash(&mut hasher);
                        }
                        WorkEntry::Thought(i) => {
                            let thought = &t.thoughts[i];
                            (thought.text.len(), thought.done, thought.expanded).hash(&mut hasher);
                        }
                    }
                }
            }
            TranscriptPart::Text { message, from, to } => (
                &t.messages[*message].id,
                from,
                to,
                t.messages[*message].text.len(),
            )
                .hash(&mut hasher),
            TranscriptPart::Compaction(index) | TranscriptPart::Retry(index) => {
                index.hash(&mut hasher)
            }
        }
    }
    hasher.finish()
}

impl WorkItem {
    fn hash_fields(self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        match self {
            Self::Part(i) => (0u8, i, 0usize),
            Self::GroupHead(i) => (1, i, 0),
            Self::GroupEntry(i, k) => (2, i, k),
            Self::ThoughtHead(i) => (3, i, 0),
            Self::ThoughtBody(i, k) => (4, i, k),
            Self::Live(i) => (5, i, 0),
        }
        .hash(hasher);
    }
}

/// Work entries `lo..hi` of a long turn's work column, as one virtual row. The
/// rows above and below are the turn's head and tail.
#[allow(clippy::too_many_arguments)]
pub(super) fn work_slice(
    s: &State,
    p: Palette,
    wide: bool,
    start: usize,
    end: usize,
    lo: usize,
    hi: usize,
) -> El {
    let t = s.current();
    let parts = turn_parts(t, start, end);
    let layout = turn_layout(t, &timeline_structure(t), start, end, &parts);
    let live = layout.live;
    let items = work_items(t, &parts, layout);
    let lo = lo.min(items.len());
    let hi = hi.min(items.len());
    // Above the first slice: the head's bar, 12 pt up in the turn's column.
    let top = if lo > 0 && lo < items.len() {
        work_item_gap(items[lo])
    } else {
        12.0
    };
    let key = format!("work-rows-{}-{lo}", message_key(t, start));
    let build = || {
        #[cfg(test)]
        timeline_count(|counts| counts.built_slices += 1);
        col().child(
            work_column(s, p, wide, &parts, &items[lo..hi], start, key.clone()).mt(top - LANE_GAP),
        )
    };
    if t.transcript_structure == 0 || timeline_structure(t).external_turns.contains(&start) {
        return build();
    }
    let find = s.thread_find_active().then(|| {
        (
            s.thread_find.query.text.as_str(),
            s.thread_find
                .active_match
                .as_ref()
                .map(|m| (&m.entry_id, m.occurrence)),
        )
    });
    let cite = s
        .citation_highlight
        .as_ref()
        .map(|h| (&h.machine, h.thread_id, &h.message_id, &h.text));
    // The response actions: a Copy check on one of this turn's messages, and whether Fork shows.
    let copied = s.copied_message.as_ref().and_then(|(id, _)| {
        t.messages[start..end.min(t.messages.len())]
            .iter()
            .any(|m| &m.id == id)
            .then_some(id.as_str())
    });
    let actions = (
        copied,
        t.session.is_some(),
        t.can_change_conversation(),
        t.can_rewind_conversation(),
        t.status.busy(),
        t.configuring,
    );
    memo(
        key.clone(),
        (
            (
                work_slice_fingerprint(t, &parts, &items[lo..hi]),
                lo,
                hi,
                top.to_bits(),
                live,
                (
                    t.request.is_some(),
                    s.portraits.reduced_motion(),
                    s.html_lane_width,
                ),
            ),
            (
                s.dark,
                wide,
                s.code_wrap_version,
                rune::memo::layout_with("chat-lane", |rect| rect[2].to_bits()),
                s.viewport_width.to_bits(),
            ),
            (t.harness.as_str(), s.working_directory(), find, cite),
            actions,
        ),
        build,
    )
}

pub(super) fn build_assistant_turn(
    s: &State,
    p: Palette,
    wide: bool,
    start: usize,
    end: usize,
    piece: Option<TurnPiece>,
) -> El {
    #[cfg(test)]
    timeline_count(|counts| counts.built_turns += 1);
    let t = s.current();
    let parts = turn_parts(t, start, end);
    let card = |part: &TranscriptPart| is_lifted_card(t, part);
    let structure = timeline_structure(t);
    let layout = turn_layout(t, &structure, start, end, &parts);
    let TurnLayout {
        last_work,
        preamble,
        live,
        inline_work,
    } = layout;
    let work = structure
        .work_index
        .get(&start)
        .and_then(|index| t.work.get(*index));
    // A split turn's head carries the turn's keys; its tail follows the last slice.
    let head = piece != Some(TurnPiece::Tail);
    let tail = piece != Some(TurnPiece::Head);
    let whole = piece.is_none();
    let mut response = col()
        .key(format!(
            "assistant-turn-{}{}",
            message_key(t, start),
            piece_suffix(piece)
        ))
        .gap(12.0);
    let mut tail_children = 0usize;
    // Captains sign their turns. A lone Pi thread needs no speaker label.
    if let Some(captain) = t.captain.filter(|_| head) {
        response = response.child(
            row()
                .center_y()
                .gap(8.0)
                // The captain's resting face, as its sidebar portrait shows when idle. A turn's
                // own status ("Done", "Working") would change the expression and the mark.
                .child(speaker_avatar(s, p, captain))
                .child(label(
                    format!("Captain {}", s.captains[captain].name),
                    TextStyle::new(14.0, 600.0),
                    p.ink,
                )),
        );
    }
    let mut occ_base = 0usize;
    let mut part_occs = vec![0usize; parts.len()];
    if s.thread_find_active() && !s.thread_find.query.text.trim().is_empty() {
        let query = s.thread_find.query.text.trim();
        let mut last_msg = None;
        for (i, part) in parts.iter().enumerate() {
            if let TranscriptPart::Text { message, from, to } = part {
                if last_msg != Some(*message) {
                    occ_base = 0;
                    last_msg = Some(*message);
                }
                part_occs[i] = occ_base;
                occ_base += crate::thread_find::find_markdown_occurrences(
                    &t.messages[*message].text[*from..*to],
                    query,
                )
                .len();
            }
        }
    }
    let render_part = |part_index: usize| {
        let mut base = part_occs[part_index];
        transcript_part(s, p, wide, &parts[part_index], &mut base)
    };
    let items = work_items(t, &parts, layout);
    let work_key = format!("work-rows-{}", message_key(t, start));
    if live {
        // KSG's `WorkingTimelineRow`: the turn's 32 pt bar at its top while it runs.
        if head {
            response = response.child(working_bar(
                t,
                start,
                live_work_foldable(t, &parts, layout),
                p,
            ));
        }
    } else if !inline_work && (last_work.is_some() || work.is_some_and(|work| work.stopped)) {
        let expanded = work.is_some_and(|work| work.expanded);
        let (work_failed, loading) = structure
            .deferred_work
            .get(&start)
            .copied()
            .unwrap_or_default();
        let work_loading = expanded && loading;
        let mut title = match (
            work.is_some_and(|work| work.stopped),
            work.and_then(|work| work.elapsed),
        ) {
            (true, Some(seconds)) => format!("You stopped after {}", elapsed_label(seconds)),
            (true, None) => "You stopped this response".into(),
            (false, Some(seconds)) => format!("Worked for {}", elapsed_label(seconds)),
            (false, None) => "Worked".into(),
        };
        if work_failed {
            title.push_str(" · Couldn't load work. Click to retry.");
        }
        let toggle_key = format!("toggle-work-{}-{start}", t.id);
        let press_key = toggle_key.clone();
        let turn_msg_id = t.messages.get(start).map(|m| m.id.clone());
        // The running turn's bar, now its fold: KSG's `WorkFoldToggle` in the same place.
        let mut section = col()
            .key(format!("work-section-{}", message_key(t, start)))
            .w_full()
            .child(
                working_bar_frame(p).child(
                    work_fold_toggle(
                        toggle_key,
                        format!(
                            "{} work section. {title}",
                            if expanded { "Collapse" } else { "Expand" }
                        ),
                        &title,
                        expanded,
                        p,
                    )
                    .on_press_tx(move |s: &mut State, tx| {
                        tx.keep_on_screen(press_key.clone());
                        let t = s.current_mut();
                        let current_start = match turn_msg_id.as_deref() {
                            Some(id) => {
                                let Some(index) = t.messages.iter().position(|m| m.id == id) else {
                                    return;
                                };
                                index
                            }
                            None => start,
                        };
                        t.timeline_cache.get_mut().touch_message(current_start);
                        let retry = t
                            .work
                            .iter()
                            .any(|work| work.start == current_start && work.expanded)
                            && t.deferred_work.iter().any(|marker| {
                                marker["start"] == current_start && marker["error"].is_string()
                            });
                        if retry {
                            s.load_deferred_work(current_start, true);
                            return;
                        }
                        match t.work.iter_mut().find(|work| work.start == current_start) {
                            Some(work) => work.expanded = !work.expanded,
                            // Turns from before work records existed fold too; opening one
                            // records it.
                            None => t.work.push(crate::model::AssistantWork {
                                start: current_start,
                                stopped: false,
                                stopped_order: 0,
                                expanded: true,
                                elapsed: None,
                                outcome: None,
                            }),
                        }
                        if s.current()
                            .work
                            .iter()
                            .any(|work| work.start == current_start && work.expanded)
                        {
                            s.load_deferred_work(current_start, false);
                        }
                    }),
                ),
            );
        if expanded {
            let restored_id = t.messages.get(start).map(|message| message.id.clone());
            section = section.on_layout_tx(move |s: &mut State, _, tx| {
                if let Some(start) = restored_id.as_ref().and_then(|id| {
                    s.current()
                        .messages
                        .iter()
                        .position(|message| &message.id == id)
                }) {
                    s.load_deferred_work(start, false);
                }
                tx.live_only();
            });
        }
        if head {
            response = response.child(section);
            if let Some(index) = preamble {
                response = response.child(render_part(index));
            }
            if work_loading {
                let mut skeleton = col()
                    .key(format!("work-loading-{}", message_key(t, start)))
                    .role("status")
                    .label("Loading work history")
                    .pb(4.0)
                    .px(4.0)
                    .gap(8.0);
                for fraction in [0.4, 0.6, 1.0 / 3.0] {
                    skeleton = skeleton.child(
                        row()
                            .gap(8.0)
                            .center_y()
                            .child(el().size(12.0, 12.0).fill(p.selected))
                            .child(
                                el().w((lane_width(s.viewport_width, wide) * fraction).max(20.0))
                                    .h(10.0)
                                    .fill(p.selected),
                            ),
                    );
                }
                response = response.child(skeleton);
            } else if expanded && !work_failed && whole && !items.is_empty() {
                response = response.child(work_column(
                    s,
                    p,
                    wide,
                    &parts,
                    &items,
                    start,
                    work_key.clone(),
                ));
            }
        }
        if let Some(last) = last_work.filter(|_| tail) {
            for (index, part) in parts[..=last].iter().enumerate() {
                if card(part) {
                    response = response.child(render_part(index));
                    tail_children += 1;
                }
            }
        }
    }
    if live && head {
        if let Some(index) = preamble {
            response = response.child(render_part(index));
        } else if last_work.is_none() {
            // Until Pi emits work, streamed text is the preamble.
            for index in 0..parts.len() {
                response = response.child(render_part(index));
            }
        }
    }
    // KSG's `ThinkingTimelineRow`: while the turn runs and its tail is not the activity, the
    // Thinking row sits at the very bottom. A failed last call puts it in the tail instead.
    let thinking = live
        && t.phase != "Compacting context"
        && match live_tail(t, &parts) {
            Some(tail) if tail.failed => false,
            Some(tail) => !tail.active && t.request.is_none(),
            None => t.request.is_none(),
        };
    let from = last_work.map_or(0, |index| index + 1);
    let trailing = tail && last_work.is_some() && from < parts.len();
    let mut thinking_after_text = thinking;
    if (live || inline_work) && whole && !items.is_empty() {
        let mut rows = work_column(s, p, wide, &parts, &items, start, work_key.clone());
        if thinking && !trailing {
            rows = rows.child(col().pt(WORK_GAP).child(live_status_row(s, t, start, p)));
            thinking_after_text = false;
        }
        response = response.child(rows);
    }
    // KSG's `AssistantChangedFilesSection`: the changed-files card mounts inside the turn's
    // last assistant message, under its prose and above the meta row, once the turn has
    // finished.
    let mut changed_files_card = if tail && !live {
        changed_files::card(s, p, start, end)
    } else {
        None
    };
    if tail && (!live || last_work.is_some()) {
        let card_after = changed_files_card
            .is_some()
            .then(|| {
                t.messages[start..end]
                    .iter()
                    .rposition(|m| m.role == "Pi")
                    .map(|index| start + index)
            })
            .flatten()
            .and_then(|message| {
                parts[from..]
                    .iter()
                    .rposition(|part| {
                        matches!(part, TranscriptPart::Text { message: m, .. } if *m == message)
                    })
                    .map(|index| from + index)
            });
        for index in from..parts.len() {
            response = response.child(render_part(index));
            tail_children += 1;
            if Some(index) == card_after
                && let Some(card) = changed_files_card.take()
            {
                response = response.child(card);
                tail_children += 1;
            }
        }
        // The last message has no visible prose (empty, folded into the work group or
        // outside the tail): the card follows its content like KSG's in-message mount.
        if let Some(card) = changed_files_card.take() {
            response = response.child(card);
            tail_children += 1;
        }
    }
    // A split running turn's tail follows its last slice, 2 pt below it as the work column
    // spaces its rows, or 12 pt below prose that ends the turn.
    let mut tail_gap = 12.0;
    if live && tail && thinking_after_text {
        if !whole && !items.is_empty() && !trailing {
            tail_gap = WORK_GAP;
        }
        response = response.child(live_status_row(s, t, start, p));
        tail_children += 1;
    }
    if tail
        && !live
        && !s.demo
        && !t.archived()
        && let Some(reply_msg) = t.messages[start..end]
            .iter()
            .rev()
            .find(|m| m.role == "Pi" && !m.text.trim().is_empty())
    {
        response = response.child(assistant_message_meta(
            s,
            p,
            reply_msg,
            end == t.messages.len(),
        ));
        tail_children += 1;
    }
    if piece == Some(TurnPiece::Tail) && tail_children > 0 {
        return col().child(response.mt(tail_gap - LANE_GAP));
    }
    response
}

/// How long a copied message shows its check, as KSG's `useCopyToClipboard` timeout.
pub(crate) const COPIED_FEEDBACK: Duration = Duration::from_millis(1500);

/// One quiet action under an assistant turn: a 24 pt icon button in the faint colour.
pub(super) fn meta_action(key: String, title: &'static str, glyph: &str, p: Palette) -> El {
    el().key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .help(title)
        .focusable(true)
        .size(24.0, 24.0)
        .center()
        .radius(6.0)
        .hover(p.selected)
        .press(p.selected)
        .child(icon(glyph, p.faint).size(14.0, 14.0))
}

/// Ported from KSG Code's `AssistantMessageMeta` (`chat/TimelineAssistantRows.tsx`): Copy, Fork
/// and Reply under the response, on the prose's left edge. Shina messages carry no timestamp,
/// so the time KSG shows after the actions is left out.
pub(super) fn assistant_message_meta(
    s: &State,
    p: Palette,
    message: &crate::model::Message,
    latest: bool,
) -> El {
    let t = s.current();
    let id = message.id.clone();
    let copied = s
        .copied_message
        .as_ref()
        .is_some_and(|(copied, _)| copied == &id);
    let text = message.display_text().to_owned();
    let copy_id = id.clone();
    let mut actions = row()
        .key(format!("assistant-actions-{}-{id}", t.id))
        .gap(4.0)
        .ml(-4.0)
        .center_y()
        .child(
            meta_action(
                format!("copy-message-{id}"),
                if copied { "Copied" } else { "Copy message" },
                if copied { "check" } else { "copy" },
                p,
            )
            .on_press_tx(move |s: &mut State, tx| {
                tx.copy(text.clone());
                s.copied_message = Some((copy_id.clone(), std::time::Instant::now()));
                tx.wake_after(COPIED_FEEDBACK + Duration::from_millis(50));
            }),
        );
    if t.session.is_some() && t.can_change_conversation() {
        let locked = t.status.busy() || t.configuring || (!latest && !t.can_rewind_conversation());
        actions = actions.child(
            meta_action(
                format!("fork-message-{id}"),
                "Fork from this response",
                "fork",
                p,
            )
            .disabled(locked)
            .on_press(move |s: &mut State| {
                // The newest response forks the whole conversation. An older one opens the
                // fork points, where the user picks the turn to branch from.
                if latest {
                    s.fork_conversation(None);
                } else {
                    s.conversation_points(crate::model::ConversationMode::Fork);
                }
            }),
        );
    }
    let reply_id = id.clone();
    actions.child(
        meta_action(
            format!("reply-message-{id}"),
            "Reply to this response",
            "reply",
            p,
        )
        .on_press(move |s: &mut State| {
            s.reply_to_message(&reply_id);
        }),
    )
}

/// Ported from KSG Code's `TimelineHistoryControl` (`MessagesTimeline.tsx`).
pub(super) fn history_control(s: &State, p: Palette) -> Option<El> {
    let t = s.current();
    if !t.history_has_more && t.history_error.is_none() {
        return None;
    }
    let thread_id = t.id;
    let mut section = col()
        .key(format!("history-control-{thread_id}"))
        .w_full()
        .gap(6.0)
        .pb(8.0);
    if t.history_has_more {
        let loading = t.history_loading;
        let text = if loading {
            "Loading earlier turns…"
        } else {
            "Load earlier turns"
        };
        let mut btn = row()
            .key("history-load-earlier")
            .role("button")
            .label("Load earlier turns")
            .w_full()
            .py(6.0)
            .gap(12.0)
            .center_y()
            .disabled(loading)
            .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
            .child(label(text, SMALL, p.muted))
            .child(el().h(1.0).grow(1.0).fill(p.line).hairline());
        if !loading {
            btn = btn
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .hover(p.selected)
                .on_press(move |s: &mut State| s.load_earlier_history(thread_id, None));
        }
        section = section.child(btn);
    }
    if let Some(err) = &t.history_error {
        section = section.child(
            label(err, SMALL, p.muted)
                .key("history-load-error")
                .role("status")
                .align(rune::Align::Center)
                .wrap()
                .w_full(),
        );
    }
    Some(section)
}
