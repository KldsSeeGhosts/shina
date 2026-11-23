use super::*;

pub(super) type TimelineEvent = (crate::model::ToolAnchor, u64, u8, usize);
pub(super) type BoundaryEvent = (u64, u8, usize);

/// Ported from KSG's useIncrementalTimelineEntries and useIncrementalTimelineRows.
/// This stores derived identities and ordinals only, never old history payloads.
#[derive(Default)]
pub(crate) struct TimelineCache {
    pub(super) input: Option<TimelineInput>,
    pub(super) structure: std::sync::Arc<TimelineStructure>,
    pub(super) content: std::collections::HashMap<usize, u64>,
    pub(super) unfinished_tools: std::collections::HashSet<usize>,
    /// Bumped when a long turn's fold opens or closes, which splits or joins its rows.
    pub(super) disclosure: u64,
    /// The turn's elapsed seconds when a request began waiting on the user, so the working
    /// bar's "Paused at" clock stops there.
    pub(super) paused_at: Option<u64>,
}

impl TimelineCache {
    pub(crate) fn prepare_projection(&mut self, structure: Option<u64>) {
        if self.input.as_ref().map(|input| input.0) != structure {
            *self = Self::default();
        }
    }

    pub(crate) fn touch_item(&mut self, field: &'static str, index: usize) {
        if let Some(turn) = self
            .structure
            .item_turns
            .get(field)
            .and_then(|turns| turns.get(index))
        {
            *self.content.entry(*turn).or_default() += 1;
        }
    }

    pub(crate) fn touch_message(&mut self, message: usize) {
        self.touch_item("messages", message);
    }

    pub(crate) fn touch_disclosure(&mut self) {
        self.disclosure += 1;
    }

    pub(crate) fn touch_tool(&mut self, index: usize, tool: &Tool) {
        self.touch_item("tools", index);
        if tool.done || tool.failed {
            self.unfinished_tools.remove(&index);
        } else {
            self.unfinished_tools.insert(index);
        }
    }

    pub(super) fn turn_revision(&self, start: usize) -> u64 {
        self.content.get(&start).copied().unwrap_or(0)
    }
}

#[derive(Default)]
pub(super) struct TimelineStructure {
    pub(super) rows: std::sync::Arc<Vec<TimelineRow>>,
    pub(super) events: Vec<TimelineEvent>,
    pub(super) boundaries: std::collections::HashMap<usize, Vec<BoundaryEvent>>,
    pub(super) item_turns: std::collections::HashMap<&'static str, Vec<usize>>,
    pub(super) work_index: std::collections::HashMap<usize, usize>,
    pub(super) deferred_work: std::collections::HashMap<usize, (bool, bool)>,
    pub(super) external_turns: std::collections::HashSet<usize>,
    /// Fold flags that decide how long turns split, as they were when this was built.
    pub(super) disclosure_probe: Vec<(u8, usize, bool)>,
    pub(super) version: u64,
}

/// The structure after any fold that splits or joins a long turn changed. Handlers
/// and restored projections set fold flags directly; only a long turn's flags are
/// checked, so ordinary disclosure keeps the cached rows and indexes.
pub(super) fn disclosed_timeline_structure(t: &Thread) -> std::sync::Arc<TimelineStructure> {
    let structure = timeline_structure(t);
    if !structure.disclosure_stale(t) {
        return structure;
    }
    t.timeline_cache.borrow_mut().touch_disclosure();
    timeline_structure(t)
}

pub(super) type TimelineInput = (
    u64,
    bool,
    bool,
    bool,
    Vec<(String, usize)>,
    (usize, usize),
    u64,
);

impl TimelineStructure {
    /// Whether a fold that splits or joins a long turn changed without a handler
    /// reporting it (a restored projection, find's reveal, a test).
    fn disclosure_stale(&self, t: &Thread) -> bool {
        self.disclosure_probe.iter().any(|&(kind, index, open)| {
            let now = match kind {
                0 => t.tools.get(index).map(|tool| tool.group_expanded),
                1 => t.thoughts.get(index).map(|thought| thought.expanded),
                2 => t.work.get(index).map(|work| work.expanded),
                _ => Some(t.live_work_collapsed),
            };
            now != Some(open)
        })
    }
}

pub(super) fn timeline_structure(t: &Thread) -> std::sync::Arc<TimelineStructure> {
    let input = (
        t.transcript_structure,
        t.history_has_more || t.history_error.is_some(),
        t.worktree_setup.is_some(),
        t.status.busy(),
        t.handoffs
            .iter()
            .map(|handoff| (handoff.id.clone(), handoff.message_index))
            .collect(),
        (
            t.deferred_work.len(),
            t.deferred_work
                .iter()
                .filter(|marker| marker["error"].is_string())
                .count(),
        ),
        t.timeline_cache.borrow().disclosure,
    );
    let mut cache = t.timeline_cache.borrow_mut();
    // The desktop consumes revisioned projections. Untracked fixtures and
    // in-process demo threads retain the complete rebuild fallback.
    if t.transcript_structure != 0 && cache.input.as_ref() == Some(&input) {
        return cache.structure.clone();
    }
    #[cfg(test)]
    timeline_count(|counts| counts.index_builds += 1);
    let mut structure = TimelineStructure::default();
    cache.unfinished_tools.clear();
    let mut turn = 0;
    let mut message_turns = Vec::with_capacity(t.messages.len() + 1);
    for (index, message) in t.messages.iter().enumerate() {
        if message.role != "Pi" || index == 0 || t.messages[index - 1].role != "Pi" {
            turn = index;
        }
        message_turns.push(turn);
    }
    message_turns.push(t.messages.len());
    let owner = |message: usize| message_turns.get(message).copied().unwrap_or(message);
    for (field, turns) in [
        (
            "tools",
            t.tools
                .iter()
                .map(|tool| owner(tool.anchor.message))
                .collect(),
        ),
        (
            "thoughts",
            t.thoughts
                .iter()
                .map(|thought| owner(thought.anchor.message))
                .collect(),
        ),
        (
            "work",
            t.work.iter().map(|work| owner(work.start)).collect(),
        ),
        (
            "compactions",
            t.native
                .compactions
                .iter()
                .map(|item| owner(crate::work::compaction_anchor(t, item).message))
                .collect(),
        ),
    ] {
        structure.item_turns.insert(field, turns);
    }
    for (index, tool) in t.tools.iter().enumerate() {
        if !tool.done && !tool.failed {
            cache.unfinished_tools.insert(index);
        }
        if crew_ui::is_delegation(tool)
            || !tool.subagents.is_empty()
            || matches!(
                tool.resolved_kind(),
                crate::work::ToolKind::Plan
                    | crate::work::ToolKind::Html
                    | crate::work::ToolKind::Subagent
            )
        {
            structure.external_turns.insert(owner(tool.anchor.message));
        }
        if tool.name.rsplit("__").next() != Some("html_preview") {
            structure.events.push((
                tool.anchor,
                if tool.order == 0 {
                    tool.block as u64
                } else {
                    tool.order
                },
                0,
                index,
            ));
        }
    }
    for (index, thought) in t.thoughts.iter().enumerate() {
        structure.events.push((
            thought.anchor,
            if thought.order == 0 {
                thought.block as u64
            } else {
                thought.order
            },
            1,
            index,
        ));
    }
    for (index, item) in t.native.compactions.iter().enumerate() {
        let anchor = crate::work::compaction_anchor(t, item);
        let order = item["order"].as_u64().unwrap_or(u64::MAX);
        structure.events.push((anchor, order, 2, index));
        structure
            .boundaries
            .entry(anchor.message)
            .or_default()
            .push((order, 0, index));
    }
    for (index, item) in crate::work::retry_events(t).iter().enumerate() {
        if let Ok(anchor) =
            serde_json::from_value::<crate::model::ToolAnchor>(item["anchor"].clone())
        {
            let order = item["order"].as_u64().unwrap_or(u64::MAX);
            structure.events.push((anchor, order, 3, index));
            structure
                .boundaries
                .entry(anchor.message)
                .or_default()
                .push((order, 1, index));
        }
    }
    for (index, work) in t.work.iter().enumerate() {
        structure.work_index.entry(work.start).or_insert(index);
        if work.stopped && (work.start < t.messages.len() || !t.status.busy()) {
            let order = if work.stopped_order == 0 {
                u64::MAX
            } else {
                work.stopped_order
            };
            structure
                .boundaries
                .entry(work.start)
                .or_default()
                .push((order, 2, 0));
        }
    }
    for marker in &t.deferred_work {
        if let Some(start) = marker["start"]
            .as_u64()
            .and_then(|start| usize::try_from(start).ok())
        {
            let flags = structure.deferred_work.entry(start).or_default();
            flags.0 |= marker["error"].is_string();
            flags.1 |= !marker["error"].is_string();
        }
    }
    structure.events.sort_by_key(|event| (event.0, event.1));
    for events in structure.boundaries.values_mut() {
        events.sort_by_key(|event| event.0);
    }
    structure.rows = std::sync::Arc::new(derive_timeline_rows(t, &mut structure));
    structure.item_turns.insert("messages", message_turns);
    static VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    structure.version = VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    cache.input = Some(input);
    cache.content.clear();
    cache.structure = std::sync::Arc::new(structure);
    cache.structure.clone()
}

#[derive(Clone)]
pub(super) enum TimelineRow {
    HistoryControl {
        key: String,
    },
    WorktreeSetup {
        key: String,
    },
    Boundary {
        message_index: usize,
        key: String,
    },
    UserOrSystem {
        index: usize,
        message_id: String,
        key: String,
    },
    Handoff {
        index: usize,
        key: String,
    },
    AssistantTurn {
        start: usize,
        end: usize,
        key: String,
    },
    LiveTurn {
        start: usize,
        key: String,
    },
    /// A piece of a long running or expanded turn. Its head keeps the turn's row key.
    TurnPiece {
        start: usize,
        end: usize,
        piece: TurnPiece,
        key: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TurnPiece {
    Head,
    /// Work entries `lo..hi`, in `work_items` order.
    Work {
        lo: usize,
        hi: usize,
    },
    Tail,
}

impl TimelineRow {
    fn entry_id<'a>(&'a self, thread: &'a Thread) -> Option<&'a str> {
        let index = match self {
            Self::UserOrSystem { message_id, .. } => return Some(message_id),
            Self::AssistantTurn { start, .. }
            | Self::LiveTurn { start, .. }
            | Self::TurnPiece { start, .. } => *start,
            Self::Boundary { message_index, .. } => *message_index,
            Self::Handoff { index, .. } => thread.handoffs.get(*index)?.message_index,
            Self::HistoryControl { .. } | Self::WorktreeSetup { .. } => return None,
        };
        thread
            .messages
            .get(index)
            .map(|message| message.id.as_str())
    }

    pub(super) fn key(&self) -> &str {
        match self {
            Self::HistoryControl { key }
            | Self::WorktreeSetup { key }
            | Self::Boundary { key, .. }
            | Self::UserOrSystem { key, .. }
            | Self::Handoff { key, .. }
            | Self::AssistantTurn { key, .. }
            | Self::LiveTurn { key, .. }
            | Self::TurnPiece { key, .. } => key,
        }
    }
}

pub(super) fn push_unique_row_key(
    seen: &mut std::collections::HashSet<String>,
    base: String,
) -> String {
    if seen.insert(base.clone()) {
        return base;
    }
    let mut suffix = 2usize;
    loop {
        let candidate = format!("{base}-{suffix}");
        if seen.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

pub(super) fn build_timeline_rows(t: &Thread) -> std::sync::Arc<Vec<TimelineRow>> {
    timeline_structure(t).rows.clone()
}

pub(super) fn derive_timeline_rows(
    t: &Thread,
    structure: &mut TimelineStructure,
) -> Vec<TimelineRow> {
    let boundaries = &structure.boundaries;
    let mut probe = vec![];
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if t.history_has_more || t.history_error.is_some() {
        rows.push(TimelineRow::HistoryControl {
            key: push_unique_row_key(&mut seen, format!("history-control-{}", t.id)),
        });
    }
    if t.worktree_setup.is_some() {
        rows.push(TimelineRow::WorktreeSetup {
            key: push_unique_row_key(&mut seen, format!("worktree-setup-row-{}", t.id)),
        });
    }
    // A handoff row sits before the message that carried it; a pending one rides at
    // the end, where KSG Code's `resolvePendingHandoff` shows it.
    let mut pending_handoffs = t
        .handoffs
        .iter()
        .enumerate()
        .map(|(index, _)| index)
        .peekable();
    let mut emit_handoffs =
        |upto: usize, rows: &mut Vec<TimelineRow>, seen: &mut std::collections::HashSet<String>| {
            while let Some(&index) = pending_handoffs.peek() {
                if t.handoffs[index].message_index > upto {
                    break;
                }
                pending_handoffs.next();
                rows.push(TimelineRow::Handoff {
                    index,
                    key: push_unique_row_key(seen, format!("handoff-row-{index}")),
                });
            }
        };
    let mut index = 0usize;
    while index < t.messages.len() {
        let message = &t.messages[index];
        emit_handoffs(index, &mut rows, &mut seen);
        if message.role != "Pi" && boundaries.contains_key(&index) {
            rows.push(TimelineRow::Boundary {
                message_index: index,
                key: push_unique_row_key(&mut seen, format!("boundary-{}-{}", t.id, message.id)),
            });
        }
        if message.role == "Pi" {
            let end = t.messages[index..]
                .iter()
                .position(|message| message.role != "Pi")
                .map_or(t.messages.len(), |offset| index + offset);
            let key = push_unique_row_key(&mut seen, format!("turn-{}-{}", t.id, message.id));
            if !push_turn_pieces(
                t, structure, index, end, &key, &mut rows, &mut seen, &mut probe,
            ) {
                rows.push(TimelineRow::AssistantTurn {
                    start: index,
                    end,
                    key,
                });
            }
            index = end;
            continue;
        }
        rows.push(TimelineRow::UserOrSystem {
            index,
            message_id: message.id.clone(),
            key: push_unique_row_key(&mut seen, format!("row-{}-{}", t.id, message.id)),
        });
        index += 1;
    }
    emit_handoffs(usize::MAX, &mut rows, &mut seen);
    if boundaries.contains_key(&t.messages.len()) {
        rows.push(TimelineRow::Boundary {
            message_index: t.messages.len(),
            key: push_unique_row_key(&mut seen, format!("boundary-end-{}", t.id)),
        });
    }
    if t.status.busy() && t.messages.last().is_none_or(|message| message.role != "Pi") {
        rows.push(TimelineRow::LiveTurn {
            start: t.messages.len(),
            key: push_unique_row_key(&mut seen, format!("turn-live-{}", t.id)),
        });
    }
    structure.disclosure_probe = probe;
    rows
}

/// The row that shows text of message `message` in a split turn: the head for its
/// preamble, a slice for prose between steps, the tail for the answer. `hit` sees the
/// message's text chunks in order and returns true for the one holding the target.
/// `None` when the turn is one row, whose key the caller already has.
pub(crate) fn timeline_text_row(
    t: &Thread,
    message: usize,
    mut hit: impl FnMut(&str) -> bool,
) -> Option<String> {
    let structure = disclosed_timeline_structure(t);
    let pieces: Vec<_> = structure
        .rows
        .iter()
        .filter_map(|row| match row {
            TimelineRow::TurnPiece {
                start,
                end,
                piece,
                key,
            } if (*start..*end).contains(&message) => Some((*start, *end, *piece, key)),
            _ => None,
        })
        .collect();
    let &(start, end, _, _) = pieces.first()?;
    let parts = turn_parts_in(t, &structure.events, start, end);
    let target = parts.iter().position(|part| {
        matches!(part, TranscriptPart::Text { message: m, from, to }
            if *m == message && hit(&t.messages[*m].text[*from..*to]))
    })?;
    let layout = turn_layout(t, &structure, start, end, &parts);
    let want = if Some(target) == layout.preamble {
        TurnPiece::Head
    } else if layout.last_work.is_none_or(|last| target > last) {
        TurnPiece::Tail
    } else {
        let item = work_items(t, &parts, layout)
            .iter()
            .position(|item| item.part() == target)?;
        return pieces.iter().find_map(|(_, _, piece, key)| match piece {
            TurnPiece::Work { lo, hi } if (*lo..*hi).contains(&item) => Some((*key).clone()),
            _ => None,
        });
    };
    pieces
        .iter()
        .find(|(_, _, piece, _)| *piece == want)
        .map(|(_, _, _, key)| (*key).clone())
}

/// The virtual row for a tool jump, including the work slice of a long turn.
pub(crate) fn timeline_tool_row(t: &Thread, index: usize) -> Option<String> {
    let tool = t.tools.get(index)?;
    let structure = disclosed_timeline_structure(t);
    let message = tool.anchor.message;
    for row in structure.rows.iter() {
        match row {
            TimelineRow::AssistantTurn { start, end, key } if (*start..*end).contains(&message) => {
                return Some(format!("{key}/inline-tool-{}-{index}", t.id));
            }
            TimelineRow::TurnPiece {
                start,
                end,
                key,
                piece: TurnPiece::Work { lo, hi },
            } if (*start..*end).contains(&message) => {
                let parts = turn_parts_in(t, &structure.events, *start, *end);
                let layout = turn_layout(t, &structure, *start, *end, &parts);
                let contains_tool = |item: WorkItem| match (&parts[item.part()], item) {
                    (TranscriptPart::Work(entries), WorkItem::GroupEntry(_, k)) => {
                        matches!(entries[k], WorkEntry::Tool(i) if i == index)
                    }
                    (TranscriptPart::Work(entries), WorkItem::Part(_)) => {
                        entry_tools(entries).any(|i| i == index)
                    }
                    (TranscriptPart::Tools(indices), _) => indices.contains(&index),
                    _ => false,
                };
                let items = work_items(t, &parts, layout);
                if items
                    .get(*lo..(*hi).min(items.len()))
                    .is_some_and(|items| items.iter().copied().any(contains_tool))
                {
                    return Some(format!("{key}/inline-tool-{}-{index}", t.id));
                }
            }
            TimelineRow::LiveTurn { start, key } if *start == message => {
                return Some(format!("{key}/inline-tool-{}-{index}", t.id));
            }
            _ => {}
        }
    }
    None
}

/// Splits a long running or expanded turn into a head, slices of `SLICE_ITEMS` work
/// entries and a tail, keyed by the turn and each slice's first entry. Returns false
/// for a turn that stays one row.
#[allow(clippy::too_many_arguments)]
pub(super) fn push_turn_pieces(
    t: &Thread,
    structure: &TimelineStructure,
    start: usize,
    end: usize,
    key: &str,
    rows: &mut Vec<TimelineRow>,
    seen: &mut std::collections::HashSet<String>,
    probe: &mut Vec<(u8, usize, bool)>,
) -> bool {
    let events = turn_events(&structure.events, start, end);
    // Each event yields at most a heading and a row, plus prose between events.
    if events.len() * 2 + (end - start) < segment_min_items() {
        return false;
    }
    let live = end == t.messages.len() && t.status.busy();
    if live {
        probe.push((3, 0, t.live_work_collapsed));
    }
    if let Some(&index) = structure.work_index.get(&start) {
        probe.push((2, index, t.work[index].expanded));
    }
    let parts = turn_parts_in(t, &structure.events, start, end);
    for part in &parts {
        match part {
            TranscriptPart::Work(entries) => {
                // The streak's fold flag: its first call's, or for thinking alone its first
                // thought's.
                match entry_tools(entries).next() {
                    Some(index) => probe.push((0, index, t.tools[index].group_expanded)),
                    None => {
                        let index = entry_thoughts(entries)[0];
                        probe.push((1, index, t.thoughts[index].expanded));
                    }
                }
            }
            TranscriptPart::Tools(indices) if indices.len() > 1 => {
                probe.push((0, indices[0], t.tools[indices[0]].group_expanded));
            }
            _ => {}
        }
    }
    let layout = turn_layout(t, structure, start, end, &parts);
    if !layout.inline_work && !turn_work_open(t, structure, start, live) {
        return false;
    }
    let items = work_items(t, &parts, layout);
    if items.len() < segment_min_items() {
        return false;
    }
    rows.push(TimelineRow::TurnPiece {
        start,
        end,
        piece: TurnPiece::Head,
        key: key.to_owned(),
    });
    let mut lo = 0;
    while lo < items.len() {
        // The last slice takes entries that arrive before the next structure change.
        let hi = if lo + SLICE_ITEMS >= items.len() {
            usize::MAX
        } else {
            lo + SLICE_ITEMS
        };
        rows.push(TimelineRow::TurnPiece {
            start,
            end,
            piece: TurnPiece::Work { lo, hi },
            key: push_unique_row_key(
                seen,
                format!("{key}-work-{}", work_item_id(t, &parts, items[lo])),
            ),
        });
        lo = hi;
    }
    rows.push(TimelineRow::TurnPiece {
        start,
        end,
        piece: TurnPiece::Tail,
        key: push_unique_row_key(seen, format!("{key}-tail")),
    });
    true
}

/// KSG timeline position recovery, applied with the history commit's scroll
/// transaction instead of waiting for the fixed viewport to resize.
pub(crate) fn restore_timeline_position(
    s: &mut State,
    tx: &mut rune::Tx,
    load_earlier: bool,
) -> bool {
    let thread = s.current();
    if thread
        .transcript_projection
        .as_ref()
        .is_some_and(|projection| !projection.history_loaded)
    {
        return false;
    }
    let thread_id = thread.id;
    let Some(visit) = s
        .timeline_positions
        .borrow()
        .current(&s.machines.selected, thread_id)
    else {
        return false;
    };
    let position = visit.restore.lock().unwrap().clone();
    let Some(position) = position else {
        return false;
    };
    let row_loaded = !position.at_end
        && build_timeline_rows(thread)
            .iter()
            .any(|row| row.key() == position.row);
    if !position.at_end && !row_loaded && thread.history_has_more {
        if load_earlier && !thread.history_loading && thread.history_error.is_none() {
            s.load_earlier_history(thread_id, position.entry_id);
            return true;
        }
        return false;
    }
    visit.restore.lock().unwrap().take();
    if position.at_end {
        tx.scroll_to_bottom_instant(format!("conversation-{thread_id}"));
    } else if row_loaded {
        tx.scroll_into_view_in(format!("conversation-{thread_id}"), &position.row);
        tx.scroll_key_to(&position.row, -position.offset);
    } else {
        tx.scroll_to_y(format!("conversation-{thread_id}"), position.scroll);
    }
    true
}

pub(super) fn build_user_or_system_row(
    s: &State,
    p: Palette,
    wide: bool,
    pane_width: f32,
    index: usize,
    message_id: &str,
) -> El {
    let t = s.current();
    let message = &t.messages[index];
    let key = message_key(t, index);
    let body = if message.role == "You" {
        // KSG Code's `TimelineUserMessageRow`: `max-w-[80%] rounded-2xl p-3`.
        let max_width = (lane_width(pane_width, wide) * 0.8).max(160.0);
        let mut user = col()
            .items_end()
            .gap(6.0)
            .max_w(max_width)
            .shrink()
            .min_w(0.0);
        let unchipped = transcript_context::unchipped_attachments(message);
        if !unchipped.is_empty() {
            user = user.child(message_images(s, p, &unchipped, &key));
        }
        if !message.text.trim().is_empty() {
            let find_query = (s.thread_find_active()
                && !s.thread_find.query.text.trim().is_empty())
            .then(|| s.thread_find.query.text.trim());
            let active_occ = find_query.and_then(|_| {
                s.thread_find
                    .active_match
                    .as_ref()
                    .filter(|m| m.entry_id == message_id)
                    .map(|m| m.occurrence)
            });
            let memo_find = find_query.map(|q| (q.to_owned(), active_occ));
            let content = if transcript_context::has_context(message) {
                transcript_context::render(s, p, wide, message, max_width - 28.0)
            } else {
                memo(
                    format!("msg-md-{}-{message_id}", t.id),
                    (&message.text, s.dark, wide, s.code_wrap_version, memo_find),
                    || {
                        markdown_ui::render_with_find(
                            &message.text,
                            p,
                            s.dark,
                            wide,
                            true,
                            false,
                            &format!("user-{}-{message_id}", t.id),
                            &s.unwrapped_code_blocks,
                            find_query.map(|q| (q, active_occ)),
                        )
                    },
                )
            };
            // Ported from KSG Code's `TimelineUserMessageRow`: borderless `surface-bubble`
            // with radius 14 and `px-3.5 py-2.5` padding.
            user = user.child(
                content
                    .key(format!("{key}-text"))
                    .max_w(max_width)
                    .min_w(0.0)
                    .px(14.0)
                    .py(10.0)
                    .radius(14.0)
                    .fill(p.message_bg),
            );
        }
        row().justify_end().child(user)
    } else if let Some(report) = (message.role == "Shina")
        .then(|| crew_ui::report_message(s, p, &message.text))
        .flatten()
    {
        report
    } else if let Some(completion) = s
        .is_captain_home(t)
        .then(|| crew_ui::parse_completion(message.display_text()))
        .flatten()
    {
        crew_ui::result_block(s, p, &completion)
    } else if let Some(brief) = (message.role == "Shina" && t.delegated)
        .then(|| crew_ui::parse_brief(message.display_text()))
        .flatten()
    {
        crew_ui::brief_block(s, p, wide, brief)
    } else {
        let display = message.display_text();
        let mut lbl = label(display, body_style(wide), p.ink)
            .wrap_anywhere()
            .selectable(true);
        if s.thread_find_active() && !s.thread_find.query.text.trim().is_empty() {
            let query = s.thread_find.query.text.trim();
            let active_occ = s
                .thread_find
                .active_match
                .as_ref()
                .filter(|m| m.entry_id == message_id)
                .map(|m| m.occurrence);
            let ranges = crate::thread_find::find_plain_occurrences(display, query);
            if !ranges.is_empty() {
                lbl = lbl.text_highlights(ranges.into_iter().enumerate().map(|(i, range)| {
                    let is_active = Some(i) == active_occ;
                    let bg = if is_active {
                        p.sea.alpha(0.32)
                    } else {
                        p.brass.alpha(0.32)
                    };
                    let mut hl = rune::TextHighlight::new(range, bg);
                    if is_active {
                        hl = hl.key("thread-find-active");
                    }
                    hl
                }));
            }
        }
        col()
            .gap(4.0)
            .px(12.0)
            .py(8.0)
            .radius(12.0)
            .fill(p.coral_bg)
            .child(label("Shina", TextStyle::new(13.0, 600.0), p.ink))
            .child(lbl)
    };
    let mut wrapper = col().w_full();
    if message.role == "You" && t.is_fresh_session_message(&message.id) {
        wrapper = wrapper.gap(8.0).child(fresh_session_row(&message.id, p));
    }
    wrapper.child(body.key(key))
}

/// Ported from KSG Code's `FreshSessionRow` (`components/chat/V2LifecycleRow.tsx`) and
/// `TimelineSystemDivider` (`components/chat/TimelineSystemDivider.tsx`).
pub(super) fn fresh_session_row(message_id: &str, p: Palette) -> El {
    row()
        .key(format!("fresh-session-row-{message_id}"))
        .role("status")
        .label("New session")
        .w_full()
        .min_w(0.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
        .child(
            row()
                .center_y()
                .gap(6.0)
                .child(icon("alert", p.muted))
                .child(meta_text("New session", p.muted)),
        )
        .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
}

/// Ported from KSG Code's `HandoffRow` (`components/chat/V2LifecycleRow.tsx`): the
/// divider's label is KSG's title and its detail the two provider endpoints.
pub(super) fn handoff_row(
    s: &State,
    index: usize,
    handoff: &crate::handoff::Handoff,
    p: Palette,
) -> El {
    use crate::handoff::HandoffStatus;
    let pending = handoff.status == HandoffStatus::Pending;
    let failed = handoff.status == HandoffStatus::Failed;
    let tone = if failed { p.danger } else { p.muted };
    let title = if pending {
        "Context handoff pending"
    } else if failed {
        "Context handoff failed"
    } else {
        "Context handoff"
    };
    // `HandoffEndpoint`: the provider mark, then the model name (the instance id
    // when the record names no model).
    let endpoint = |instance: &str, model: Option<&str>| {
        let name = model
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(instance);
        row()
            .center_y()
            .gap(4.0)
            .min_w(0.0)
            .shrink()
            .child(provider_icon(s, p, model_vendor(name, ""), 12.0))
            .child(meta_text(name, p.muted).truncate().shrink().min_w(0.0))
    };
    let mut detail = row().center_y().gap(6.0).min_w(0.0).shrink();
    let sources: Vec<(&str, Option<&str>)> = if handoff.from_models.is_empty() {
        handoff
            .from_instances
            .iter()
            .map(|instance| (instance.as_str(), None))
            .collect()
    } else {
        handoff
            .from_models
            .iter()
            .map(|selection| {
                (
                    selection.instance_id.as_str(),
                    Some(selection.model.as_str()),
                )
            })
            .collect()
    };
    for (position, (instance, model)) in sources.iter().enumerate() {
        if position > 0 {
            detail = detail.child(meta_text(",", p.muted));
        }
        detail = detail.child(endpoint(instance, *model));
    }
    if !sources.is_empty() {
        detail = detail.child(meta_text("→", p.muted));
    }
    detail = detail.child(endpoint(&handoff.to_instance, handoff.to_model.as_deref()));
    if handoff.delivery.status == crate::handoff::DeliveryStatus::Inline {
        let covered = handoff.delivery.delivered_item_ids.len();
        let omitted = handoff.delivery.omitted_item_ids.len();
        detail = detail.child(meta_text(
            format!("· {covered} items, {omitted} omitted"),
            p.muted,
        ));
    }
    row()
        .key(format!("handoff-row-{index}"))
        .role("status")
        .label(format!("{title}: {}", handoff.to_instance))
        .w_full()
        .min_w(0.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
        .child(
            row()
                .center_y()
                .gap(6.0)
                .min_w(0.0)
                .shrink()
                .child(icon("swap", tone))
                .child(meta_text(title, tone).shrink())
                .child(detail),
        )
        .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
}

/// Ported from KSG Code's `ThreadFindBar` (`components/chat/ThreadFindBar.tsx`).
pub(super) fn thread_find_bar(s: &State, p: Palette, pane_width: f32) -> El {
    let find = &s.thread_find;
    let trimmed = find.query.text.trim();
    let has_query = !trimmed.is_empty();
    let no_results = has_query
        && !find.loading
        && find.error.is_none()
        && !find.unsupported
        && find.total_matches == 0;
    let status_text = if find.unsupported {
        "Update required".to_owned()
    } else if find.loading && find.total_matches == 0 {
        "Searching…".to_owned()
    } else if find.error.is_some() {
        "Search failed".to_owned()
    } else if has_query {
        if find.total_matches == 0 {
            "0/0".to_owned()
        } else {
            format!(
                "{}/{}",
                find.active_index.min(find.total_matches - 1) + 1,
                find.total_matches
            )
        }
    } else {
        String::new()
    };
    let nav_disabled = find.total_matches == 0 || find.loading || find.unsupported;
    let bar_width = (pane_width - 24.0).clamp(200.0, 340.0);
    let mut bar = row()
        .key("thread-find-bar")
        .role("search")
        .label("Find in thread")
        .w(bar_width)
        .h(36.0)
        .px(10.0)
        .gap(4.0)
        .center_y()
        .radius(12.0)
        .fill(p.card)
        .on_layout_tx(|s: &mut State, _rect, tx| {
            let changed = s.sync_thread_find_input();
            let revealed = s.consume_pending_find_reveal(tx);
            if !changed && !revealed {
                tx.live_only();
            }
        })
        .child(
            text_input(&find.query, LABEL)
                .key("thread-find-input")
                .label("Find in thread")
                .placeholder("Find in thread", p.muted)
                .color(p.ink)
                .single_line()
                .grow(1.0)
                .min_w(0.0)
                .on_edit(|s: &mut State| &mut s.thread_find.query)
                .on_input_key(|s: &mut State, event, tx| {
                    if event.key == rune::Key::Enter {
                        if event.mods.shift {
                            s.step_thread_find(-1);
                        } else {
                            s.step_thread_find(1);
                        }
                        s.consume_pending_find_reveal(tx);
                        true
                    } else if event.key == rune::Key::Escape {
                        s.close_thread_find();
                        s.restore_composer_focus(composer_ui::FocusReason::Explicit);
                        true
                    } else {
                        false
                    }
                }),
        );
    if !status_text.is_empty() {
        bar = bar.child(
            label(
                status_text,
                SMALL.features(TABULAR),
                if no_results || find.error.is_some() || find.unsupported {
                    p.danger
                } else {
                    p.muted
                },
            )
            .key("thread-find-count")
            .min_w(36.0)
            .align(rune::Align::Center),
        );
    }
    if find.error.is_some() && !find.unsupported {
        bar = bar.child(
            label("Retry", SMALL, p.ink)
                .key("thread-find-retry")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .px(4.0)
                .py(2.0)
                .radius(6.0)
                .hover(p.selected)
                .on_press(|s: &mut State| s.retry_thread_find()),
        );
    }
    bar = bar
        .child(
            icon_button(
                "thread-find-prev",
                "Previous match",
                "chevron-up",
                p,
                |_| {},
            )
            .disabled(nav_disabled)
            .on_press_tx(|s: &mut State, tx| {
                s.step_thread_find(-1);
                s.consume_pending_find_reveal(tx);
            }),
        )
        .child(
            icon_button("thread-find-next", "Next match", "chevron", p, |_| {})
                .disabled(nav_disabled)
                .on_press_tx(|s: &mut State, tx| {
                    s.step_thread_find(1);
                    s.consume_pending_find_reveal(tx);
                }),
        )
        .child(icon_button(
            "thread-find-close",
            "Close find",
            "close",
            p,
            |s: &mut State| {
                s.close_thread_find();
                s.restore_composer_focus(composer_ui::FocusReason::Explicit);
            },
        ));
    framed(bar, p).pin(None, Some(12.0), Some(12.0), None)
}

/// A running turn's height motion. Streaming prose reveals at the stream's pace. While the
/// turn ends in work, its live row replaces itself as steps start and end and its thought
/// preview comes and goes, so the row snaps to each new height instead of easing toward it
/// behind the feathered edge, which flickered and cut the newest step in half.
pub(super) fn live_turn_motion(t: &Thread, turn: El, start: usize, end: usize) -> El {
    let parts = turn_parts(t, start, end);
    if parts.is_empty() || live_tail(t, &parts).is_some() {
        turn.fold_glide(rune::Spring::NONE).fold_feather(0.0)
    } else {
        turn.streaming(true)
    }
}

/// Ported from KSG Code's `MessagesTimeline` (`LegendList` virtualisation, `estimatedItemSize={90}`,
/// scroll anchoring and stick-to-bottom).
pub(super) fn conversation(s: &State, p: Palette, wide: bool, pane_width: f32) -> El {
    let t = s.current();
    let visit = s.timeline_positions.borrow().visit();
    let gutter = lane_gutter(pane_width);
    // The lane the composer resolves too; `None` until C5 mounts the Details card.
    let lane_rect = chat_lane(pane_width, wide, details_ui::edges(s, pane_width));
    // A captain's delegations show as subagent rows inside a turn, so with no messages they
    // have nothing to hang from. The home counts as empty and greets.
    let visible_tools = t.messages.is_empty()
        && t.tools
            .iter()
            .any(|tool| !(s.is_captain_home(t) && crew_ui::is_delegation(tool)));
    if t.messages.is_empty()
        && !visible_tools
        && !t.status.busy()
        && t.native.compactions.is_empty()
        && t.worktree_setup.is_none()
        && !t.history_has_more
        && t.history_error.is_none()
    {
        let mut introduction = col()
            .key("empty-state")
            .w_full()
            .max_w(measure(wide) + 2.0 * gutter + lane_rect.inset_start + lane_rect.inset_end)
            .pl(gutter + lane_rect.inset_start)
            .pr(gutter + lane_rect.inset_end)
            .pb(12.0)
            .gap(16.0);
        if let Some(captain) = t.captain {
            introduction = introduction
                .items_center()
                .pb(16.0)
                .child(captain_hero(s, p, captain, pane_width));
        } else {
            introduction = introduction
                .items_center()
                .px(0.0)
                .pb(16.0)
                .gap(24.0)
                .child(draft_headline(s, p, pane_width));
        }
        if t.restoring_history() {
            introduction = introduction.child(label("Restoring conversation...", SMALL, p.muted));
        }
        return introduction;
    }

    let thread_id = t.id;
    let structure = disclosed_timeline_structure(t);
    let rows = structure.rows.clone();
    let key_rows = rows.clone();
    let build_rows = rows.clone();
    let visible_rows = rows.clone();
    let visible_visit = visit.clone();
    let visible_server = s.machines.selected.clone();
    let row_visit = visit.clone();
    let layout_visit = visit.clone();
    let lane = col()
        .key("chat-lane")
        .on_layout_tx(|s: &mut State, rect, tx| {
            s.html_lane_width = rect[2].round().clamp(240.0, 1600.0) as u32;
            tx.live_only();
        })
        .w_full()
        .max_w(measure(wide))
        // 24 pt above the first message, counting the cap.
        .pt(24.0 - TRANSCRIPT_CAP)
        .pb(24.0)
        .gap(LANE_GAP)
        .growth(rune::GrowthSpec::default())
        .virtual_rows(
            rows.len(),
            90.0,
            move |_: &State, index| {
                #[cfg(test)]
                timeline_count(|counts| counts.keys += 1);
                std::borrow::Cow::Owned(key_rows[index].key().to_owned())
            },
            move |s: &State, index| {
                #[cfg(test)]
                timeline_count(|counts| counts.row_builders += 1);
                let row = match &build_rows[index] {
                    TimelineRow::HistoryControl { .. } => history_control(s, p).unwrap_or_else(el),
                    TimelineRow::WorktreeSetup { .. } => {
                        worktree_setup_card(s, p).unwrap_or_else(el)
                    }
                    TimelineRow::Boundary { message_index, .. } => {
                        work_ui::boundary_events(s, p, wide, *message_index).unwrap_or_else(el)
                    }
                    TimelineRow::UserOrSystem {
                        index: msg_idx,
                        message_id,
                        ..
                    } => build_user_or_system_row(s, p, wide, pane_width, *msg_idx, message_id),
                    TimelineRow::Handoff { index, .. } => s
                        .current()
                        .handoffs
                        .get(*index)
                        .map(|handoff| handoff_row(s, *index, handoff, p))
                        .unwrap_or_else(el),
                    // A running turn grows at the stack's streaming pace behind its feathered
                    // edge. A finished one folds its work open and shut over 200 ms, as KSG
                    // Code's `transition-[height] duration-200 ease-out` does.
                    TimelineRow::AssistantTurn { start, end, .. } => {
                        let turn = assistant_turn(s, p, wide, *start, *end);
                        if *end == s.current().messages.len() && s.current().status.busy() {
                            live_turn_motion(s.current(), turn, *start, *end)
                        } else {
                            turn.fold_glide(rune::Spring::new(0.20, 0.0))
                                .fold_feather(14.0)
                        }
                    }
                    TimelineRow::LiveTurn { start, .. } => live_turn_motion(
                        s.current(),
                        assistant_turn(s, p, wide, *start, *start),
                        *start,
                        *start,
                    ),
                    TimelineRow::TurnPiece {
                        start, end, piece, ..
                    } => assistant_turn_piece(s, p, wide, *start, *end, Some(*piece)),
                };
                let key = build_rows[index].key().to_owned();
                let entry_id = build_rows[index].entry_id(s.current()).map(str::to_owned);
                let row_visit = row_visit.clone();
                row.on_layout_tx(move |_: &mut State, rect, tx| {
                    row_visit.row(&key, entry_id.as_deref(), rect);
                    tx.live_only();
                })
            },
        )
        .virtual_rows_version(structure.version)
        .on_visible_rows(move |s: &mut State, range| {
            if s.current().id == thread_id && s.machines.selected == visible_server {
                let near_start = range.start <= 1 && range.end < visible_rows.len();
                visible_visit.visible_rows(
                    range.filter_map(|index| {
                        visible_rows.get(index).map(|row| row.key().to_owned())
                    }),
                );
                let t = s.current();
                // Keep KSG's manual control for retries and pages too short to scroll.
                if near_start
                    && t.history_has_more
                    && !t.history_loading
                    && t.history_error.is_none()
                    && visible_visit.restore.lock().unwrap().is_none()
                {
                    s.load_earlier_history(thread_id, None);
                    return true;
                }
            }
            false
        });

    let find_open = s.thread_find_active();
    let stick_bottom = !find_open && (!t.messages.is_empty() || !t.tools.is_empty());
    let scroller = col()
        .key(format!("conversation-{thread_id}"))
        .animated_changes()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom })
        .on_scroll_offset(move |at| *visit.scroll.lock().unwrap() = at)
        .on_layout_tx(move |s: &mut State, rect, tx| {
            layout_visit.viewport(rect);
            if s.current().id != thread_id || !restore_timeline_position(s, tx, false) {
                tx.live_only();
            }
        })
        .on_far_from_bottom(40.0, move |s: &mut State, far| {
            s.set_timeline_far_from_bottom(thread_id, far);
        })
        .on_at_bottom(move |s: &mut State, at| {
            s.set_timeline_at_bottom(thread_id, at);
        })
        .on_selection_change(move |s: &mut State, change, tx| {
            if change.dragging() {
                if s.assistant_selection.is_some() {
                    s.assistant_selection = None;
                } else {
                    tx.live_only();
                }
                return;
            }
            let next = change.selection().and_then(|sel| {
                let t = s.current();
                if t.id != thread_id {
                    return None;
                }
                let paths: Vec<&str> = sel
                    .texts()
                    .iter()
                    .filter(|st| st.is_selected())
                    .map(|st| st.path())
                    .collect();
                let target = crate::assistant_citation::validate_single_assistant_selection(
                    t.id,
                    &t.messages,
                    &paths,
                    sel.text(),
                )?;
                Some(crate::model::AssistantSelection {
                    thread_id: t.id,
                    machine: t.machine.clone(),
                    message_index: target.message_index,
                    message_id: target.message_id,
                    chunk_from: target.chunk_from,
                    text: sel.text().to_owned(),
                })
            });
            if s.assistant_selection != next {
                s.assistant_selection = next;
            } else {
                tx.live_only();
            }
        })
        // A transcript longer than the pane rests stuck to its bottom, so its top is always
        // under the header's rule. An opaque 8 pt cap in the panel colour sits right under the
        // rule, stuck to the viewport's top, and the 28 pt fade starts below it, so nothing
        // scrolled shows through as a sliver under the rule.
        .scroll_fade(rune::ScrollFade::strength(28.0).below_sticky())
        .pl(gutter + lane_rect.inset_start)
        .pr(gutter + lane_rect.inset_end)
        .items_center()
        .child(
            el().key("transcript-cap")
                .w_full()
                .h(TRANSCRIPT_CAP)
                .fill(p.panel)
                .sticky_top(0.0)
                .a11y_hidden(),
        )
        .child(lane);

    let mut wrapper = col()
        .key(format!("conversation-wrap-{thread_id}"))
        .grow(1.0)
        .min_h(0.0)
        .child(
            col()
                .key(format!("timeline-server-{}", s.machines.selected))
                .grow(1.0)
                .min_h(0.0)
                .child(scroller),
        );

    if s.timeline_is_far_from_bottom(thread_id) && !find_open {
        let pill = framed(
            row()
                .key("scroll-to-end")
                .role("button")
                .label("Scroll to end")
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .h(28.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(12.0)
                .fill(p.card)
                .hover(p.selected)
                .press(p.selected)
                .on_press_tx(move |s: &mut State, tx| {
                    s.set_timeline_far_from_bottom(thread_id, false);
                    tx.scroll_to_bottom(format!("conversation-{thread_id}"));
                })
                .child(icon("chevron", p.muted))
                .child(label("Scroll to end", SMALL, p.ink)),
            p,
        );
        wrapper = wrapper.child(
            row()
                .pin(Some(0.0), None, Some(0.0), Some(12.0))
                .justify_center()
                .child(pill),
        );
    }
    if find_open {
        wrapper = wrapper.child(thread_find_bar(s, p, pane_width));
    }

    wrapper
}

/// Ported from KSG Code's `WorktreeSetupCard.tsx`.
pub(super) fn worktree_setup_card(s: &State, p: Palette) -> Option<El> {
    let t = s.current();
    let setup = t.worktree_setup.as_ref()?;
    let (status, status_word, title) = match setup.phase {
        crate::model::WorktreeSetupPhase::Running => {
            (Status::Running, "RUNNING", "Setting up worktree…")
        }
        crate::model::WorktreeSetupPhase::Succeeded => (
            Status::Completed,
            if setup.skipped { "SKIPPED" } else { "READY" },
            if setup.skipped {
                "Worktree setup skipped"
            } else {
                "Worktree ready"
            },
        ),
        crate::model::WorktreeSetupPhase::Failed => {
            (Status::Failed, "FAILED", "Worktree setup failed")
        }
    };
    let tone = status_ink(status, p);
    let mut header = row()
        .w_full()
        .min_h(24.0)
        .center_y()
        .gap(8.0)
        .child(status_mark(status, p, 6.0))
        .child(
            label(title, TextStyle::new(14.0, 500.0).lh(20.0), p.ink)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(meta_text(status_word, tone));
    if setup.duration_ms > 0 {
        let dur = if setup.duration_ms < 1000 {
            format!("{}ms", setup.duration_ms)
        } else {
            format!("{:.1}s", setup.duration_ms as f64 / 1000.0)
        };
        header = header.child(meta_text(dur, p.muted));
    }

    let mut card = col()
        .w_full()
        .padding(12.0)
        .gap(8.0)
        .radius(12.0)
        .fill(p.card)
        .child(header);

    if let Some(script_name) = &setup.script_name {
        let desc = match &setup.script_command {
            Some(cmd) if !cmd.trim().is_empty() => format!("{script_name}: {cmd}"),
            _ => script_name.clone(),
        };
        card = card.child(label(desc, SMALL, p.muted).wrap());
    }
    if let Some(err) = &setup.error {
        card = card.child(label(err, SMALL, p.danger).wrap().selectable(true));
    }

    let tail_lines: Vec<&str> = setup.output.lines().collect();
    if !tail_lines.is_empty() {
        let start = tail_lines.len().saturating_sub(4);
        let tail = tail_lines[start..].join("\n");
        let open = t.worktree_setup_output_open;
        let toggle = row()
            .key("worktree-setup-output")
            .role("button")
            .label(if open { "Hide output" } else { "Show output" })
            .cursor(rune::Cursor::Pointer)
            .focusable(true)
            .center_y()
            .gap(6.0)
            .h(24.0)
            .px(6.0)
            .radius(6.0)
            .hover(p.selected)
            .on_press(|s: &mut State| {
                s.current_mut().worktree_setup_output_open =
                    !s.current().worktree_setup_output_open;
            })
            .child(icon(if open { "chevron-up" } else { "chevron" }, p.muted).size(12.0, 12.0))
            .child(meta_text("Output", p.muted));
        card = card.child(toggle);
        if open {
            let mono = TextStyle::new(12.0, 400.0)
                .lh(17.0)
                .family(rune::Family::SystemMono);
            card = card.child(
                col()
                    .key("worktree-setup-output-tail")
                    .w_full()
                    .px(10.0)
                    .py(6.0)
                    .radius(6.0)
                    .fill(if setup.phase == crate::model::WorktreeSetupPhase::Failed {
                        p.coral_bg
                    } else {
                        p.code_bg
                    })
                    .border(1.0, p.line)
                    .child(
                        label(
                            tail,
                            mono,
                            if setup.phase == crate::model::WorktreeSetupPhase::Failed {
                                p.danger
                            } else {
                                p.muted
                            },
                        )
                        .wrap_anywhere()
                        .selectable(true),
                    ),
            );
        }
    }

    let mut actions = row().w_full().gap(8.0).center_y();
    let mut has_actions = false;
    if setup.phase == crate::model::WorktreeSetupPhase::Failed {
        has_actions = true;
        actions = actions.child(framed(
            button("worktree-setup-retry", "Retry", p, |s| {
                s.retry_worktree_setup();
            }),
            p,
        ));
    }
    if matches!(
        setup.phase,
        crate::model::WorktreeSetupPhase::Running | crate::model::WorktreeSetupPhase::Failed
    ) {
        has_actions = true;
        actions = actions.child(button("worktree-setup-skip", "Skip", p, |s| {
            s.skip_worktree_setup();
        }));
    }
    if has_actions {
        card = card.child(actions);
    }

    Some(
        col().w_full().child(
            card.key("worktree-setup-card")
                .w_full()
                .radius(12.0)
                .border(1.0, p.frame),
        ),
    )
}

/// Ported from KSG Code's `AssistantSelectionToolbar` (`chat/AssistantSelectionToolbar.tsx`).
pub(super) fn assistant_selection_toolbar(s: &State, p: Palette, cx: &Cx<State>) -> Option<El> {
    let sel = s.assistant_selection.as_ref()?;
    let t = s.current();
    if s.demo
        || sel.thread_id != t.id
        || sel.machine != t.machine
        || t.archived()
        || t.request.is_some()
        || t.native.questionnaire.is_some()
    {
        return None;
    }
    if s.transcript_preview.is_some()
        || s.rename_thread.is_some()
        || s.delete_thread.is_some()
        || s.plan_save_dialog.is_some()
        || s.confirm_remove_attachment.is_some()
        || s.project_picker.is_some()
        || s.captain_editor.is_some()
        || s.settings_picker.is_some()
        || s.captain_picker
        || s.env_mode_picker
        || s.base_branch_picker
        || s.machines.picker
        || s.settings.archive.is_some()
    {
        return None;
    }
    let rects = cx.selection_rects();
    if rects.is_empty() {
        return None;
    }
    let viewport = cx.layout_with(&format!("conversation-{}", t.id), |r| r);
    let first_visible = match viewport {
        Some([vx, vy, vw, vh]) => rects.into_iter().find(|&[rx, ry, rw, rh]| {
            rx + rw.max(1.0) > vx && rx < vx + vw && ry + rh.max(1.0) > vy && ry < vy + vh
        })?,
        None => rects[0],
    };
    let anchor_rect = match viewport {
        Some([vx, vy, vw, vh]) => {
            let left = first_visible[0].clamp(vx, vx + vw);
            let right = (first_visible[0] + first_visible[2]).clamp(left, vx + vw);
            let top = first_visible[1].clamp(vy, vy + vh);
            let bottom = (first_visible[1] + first_visible[3]).clamp(top, vy + vh);
            [left, top, (right - left).max(1.0), (bottom - top).max(1.0)]
        }
        None => first_visible,
    };
    let zoom = s.zoom.clamp(0.5, 2.0);
    let anchor = el()
        .key("assistant-selection-anchor")
        .absolute(anchor_rect[0] / zoom, anchor_rect[1] / zoom)
        .size(anchor_rect[2] / zoom, anchor_rect[3] / zoom)
        .inert()
        .a11y_hidden();
    let too_long = crate::assistant_citation::utf16_len(&sel.text)
        > crate::assistant_citation::ASSISTANT_CITATION_MAX_TEXT_LENGTH;
    let mut cite_btn = row()
        .key("assistant-selection-cite")
        .role("button")
        .label(if too_long {
            "Selection is too long to cite"
        } else {
            "Cite selection in composer"
        })
        .h(28.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .fill(p.card)
        .keeps_text_selection()
        .disabled(too_long)
        .child(icon("thread", if too_long { p.muted } else { p.ink }))
        .child(meta_text(
            if too_long {
                "Shorten selection"
            } else {
                "Cite"
            },
            if too_long { p.muted } else { p.ink },
        ));
    if !too_long {
        cite_btn = cite_btn
            .cursor(rune::Cursor::Pointer)
            .focusable(true)
            .hover(p.selected)
            .press(p.selected)
            .on_press_tx(|s: &mut State, tx| {
                s.cite_assistant_selection(tx);
            });
    }
    let toolbar = row()
        .key("assistant-selection-toolbar")
        .role("toolbar")
        .label("Assistant selection actions")
        .keeps_text_selection()
        .gap(4.0)
        .child(framed(cite_btn, p))
        .anchored_fit(
            "assistant-selection-anchor",
            rune::Side::Top,
            rune::Align::Start,
            6.0,
        );
    Some(
        el().key("assistant-selection-layer")
            .absolute(0.0, 0.0)
            .size(0.0, 0.0)
            .child(anchor)
            .child(toolbar),
    )
}
