use super::*;

/// KSG's provider-title fallback for a command with no command text
/// (`commandlessCommandLabel`): the provider's own title, or the generic status label.
pub(super) fn commandless_label(tool: &Tool, status: &'static str) -> String {
    let generic = match status {
        "inProgress" => "Running command",
        "failed" => "Failed command",
        _ => "Ran command",
    };
    let title = tool.detail.trim();
    if title.is_empty() || matches!(title, "Ran command" | "Run command" | "Command") {
        generic.into()
    } else {
        title.to_owned()
    }
}

/// Ported from KSG Code's `workEntryDisplayLabel` (`MessagesTimeline.logic.ts`): the
/// collapsed row's one label — the command itself for a command, "Searched …" or
/// "Read {path}" for search and reads, the first changed path for an edit, then a
/// single-line detail, then the title.
pub(super) fn entry_label(tool: &Tool, root: &std::path::Path) -> String {
    use crate::work::ToolKind::*;
    let detail = tool.detail.trim();
    match tool.resolved_kind() {
        Command => {
            let command = work_ui::input_text(tool, &["command", "cmd"]);
            return if command.trim().is_empty() {
                commandless_label(tool, "completed")
            } else {
                crate::work::command_display_text(&command)
            };
        }
        // KSG's classifyToolActivity puts "ls" under search.
        FileSearch | WebSearch | List => {
            if let Some(label) = crate::work::search_label(tool) {
                return label;
            }
        }
        Read => {
            let paths = crate::work::read_paths(tool);
            if let Some(first) = paths.first() {
                let path = crate::work::relative_path(first, root);
                return if paths.len() == 1 {
                    format!("Read {path}")
                } else {
                    format!("Read {path} +{} more", paths.len() - 1)
                };
            }
        }
        _ => {}
    }
    if !detail.is_empty() && !detail.contains(['\r', '\n']) {
        return detail.to_owned();
    }
    match tool.resolved_kind() {
        FileChange if !tool.changes.is_empty() => {
            let path = crate::work::relative_path(&tool.changes[0].path, root);
            if tool.changes.len() == 1 {
                path
            } else {
                format!("{path} +{} more", tool.changes.len() - 1)
            }
        }
        Read => "Read file".into(),
        _ => {
            // KSG's last resort: the tool title, first letter capitalized.
            let name = tool.name.trim();
            match name.chars().next() {
                Some(first) => first.to_uppercase().chain(name.chars().skip(1)).collect(),
                None => "Tool call".into(),
            }
        }
    }
}

/// Ported from KSG's `liveWorkEntryLabel`: a running command names its program
/// ("Running cargo"), a failed one reads "Failed cargo", a finished one "Ran cargo";
/// every other entry keeps its display label.
pub(super) fn live_label(tool: &Tool, root: &std::path::Path) -> String {
    if tool.resolved_kind() != crate::work::ToolKind::Command {
        return entry_label(tool, root);
    }
    let command = work_ui::input_text(tool, &["command", "cmd"]);
    let status = if tool.failed {
        "failed"
    } else if !tool.done {
        "inProgress"
    } else {
        "completed"
    };
    if command.trim().is_empty() {
        return commandless_label(tool, status);
    }
    match crate::work::command_program_name(&command) {
        Some(program) => match status {
            "inProgress" => format!("Running {program}"),
            "failed" => format!("Failed {program}"),
            _ => format!("Ran {program}"),
        },
        None => commandless_label(tool, status),
    }
}

pub(super) fn tool_title(tool: &Tool, root: &std::path::Path) -> String {
    entry_label(tool, root)
}

/// Ported from KSG Code's work-log/presentation.ts summarizeToolGroup.
pub(super) fn tool_summary(tools: &[&Tool]) -> String {
    crate::work::summarize(tools)
}

/// Turn and thinking durations, as Cursor writes them: 45s, 2m 5s, 1h 3m.
pub(super) fn elapsed_label(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TranscriptPart {
    Text {
        message: usize,
        from: usize,
        to: usize,
    },
    /// Cards that keep their own place: plans, rendered HTML and a captain's delegations.
    Tools(Vec<usize>),
    /// Thinking before grouping; `turn_parts` folds every run of it into `Work`.
    Thought(Vec<usize>),
    /// One streak of thoughts and ordinary tool calls between prose, in order.
    Work(Vec<WorkEntry>),
    Compaction(usize),
    Retry(usize),
}

/// A step of a work streak: a tool call or a thought, by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum WorkEntry {
    Tool(usize),
    Thought(usize),
}

/// Content indices preserve thinking/tool order even when neither adds visible message text.
pub(super) fn turn_parts(t: &Thread, start: usize, end: usize) -> Vec<TranscriptPart> {
    turn_parts_in(t, &timeline_structure(t).events, start, end)
}

/// A tool that shows as its own card rather than a step: a plan, rendered HTML, a captain's
/// delegation, or a subagent run with its result cards (KSG keeps subagent events out of
/// work groups too).
pub(super) fn is_card_tool(t: &Thread, index: usize) -> bool {
    let tool = &t.tools[index];
    if !tool.subagents.is_empty() {
        return true;
    }
    match tool.resolved_kind() {
        crate::work::ToolKind::Plan | crate::work::ToolKind::Subagent => true,
        crate::work::ToolKind::Html => {
            crate::html_render::find_reference(&tool.structured).is_some()
        }
        _ => t.captain_home && !t.delegated && crew_ui::is_delegation(tool),
    }
}

/// Ported from KSG Code's `deriveTimelineRows` grouping (`MessagesTimeline.logic.ts`): every
/// contiguous run of reasoning and tool entries between prose is one work group. Cards,
/// compactions and retries end a run.
pub(super) fn group_work(t: &Thread, parts: Vec<TranscriptPart>) -> Vec<TranscriptPart> {
    let mut out: Vec<TranscriptPart> = Vec::with_capacity(parts.len());
    let push_entry = |out: &mut Vec<TranscriptPart>, entry: WorkEntry| {
        if let Some(TranscriptPart::Work(entries)) = out.last_mut() {
            entries.push(entry);
        } else {
            out.push(TranscriptPart::Work(vec![entry]));
        }
    };
    for part in parts {
        match part {
            TranscriptPart::Thought(indices) => {
                for index in indices {
                    push_entry(&mut out, WorkEntry::Thought(index));
                }
            }
            TranscriptPart::Tools(indices) => {
                // Cards from one call batch stay one part.
                let mut cards_open = false;
                for index in indices {
                    if !is_card_tool(t, index) {
                        push_entry(&mut out, WorkEntry::Tool(index));
                        cards_open = false;
                    } else if cards_open && let Some(TranscriptPart::Tools(cards)) = out.last_mut()
                    {
                        cards.push(index);
                    } else {
                        out.push(TranscriptPart::Tools(vec![index]));
                        cards_open = true;
                    }
                }
            }
            part => out.push(part),
        }
    }
    out
}

pub(super) fn turn_events(events: &[TimelineEvent], start: usize, end: usize) -> &[TimelineEvent] {
    let lo = events.partition_point(|event| event.0.message < start);
    let hi = events.partition_point(|event| event.0.message < end);
    &events[lo..hi]
}

pub(super) fn turn_parts_in(
    t: &Thread,
    events: &[TimelineEvent],
    start: usize,
    end: usize,
) -> Vec<TranscriptPart> {
    group_work(t, event_parts_in(t, events, start, end))
}

/// A turn's parts in event order, before thoughts and tools are grouped into work streaks.
pub(super) fn event_parts_in(
    t: &Thread,
    events: &[TimelineEvent],
    start: usize,
    end: usize,
) -> Vec<TranscriptPart> {
    let mut events = turn_events(events, start, end).iter().copied().peekable();
    let mut parts = vec![];
    for message in start..end {
        let text = &t.messages[message].text;
        let mut from = 0;
        let mut previous_anchor = None;
        while events
            .peek()
            .is_some_and(|event| event.0.message == message)
        {
            let (anchor, _, kind, index) = events.next().unwrap();
            let to = if anchor.offset >= from && text.is_char_boundary(anchor.offset) {
                anchor.offset
            } else {
                from
            };
            if to > from {
                parts.push(TranscriptPart::Text { message, from, to });
            }
            if kind == 2 {
                parts.push(TranscriptPart::Compaction(index));
            } else if kind == 3 {
                parts.push(TranscriptPart::Retry(index));
            } else if kind == 1 {
                if let Some(TranscriptPart::Thought(indices)) = parts.last_mut() {
                    indices.push(index);
                } else {
                    parts.push(TranscriptPart::Thought(vec![index]));
                }
            } else if previous_anchor == Some(anchor)
                && let Some(TranscriptPart::Tools(indices)) = parts.last_mut()
                && !matches!(
                    t.tools[index].resolved_kind(),
                    crate::work::ToolKind::Plan | crate::work::ToolKind::Html
                )
                && indices.last().is_some_and(|i| {
                    !matches!(
                        t.tools[*i].resolved_kind(),
                        crate::work::ToolKind::Plan | crate::work::ToolKind::Html
                    )
                })
            {
                indices.push(index);
            } else {
                parts.push(TranscriptPart::Tools(vec![index]));
            }
            previous_anchor = Some(anchor);
            from = to;
        }
        if from < text.len() {
            parts.push(TranscriptPart::Text {
                message,
                from,
                to: text.len(),
            });
        }
    }
    parts
}

/// Secondary and tertiary transcript inks: a tool's verb, and what it acted on.
pub(super) fn quiet_inks(p: Palette) -> (Color, Color) {
    (p.muted, p.faint)
}

#[allow(clippy::too_many_arguments)]
/// A work row's line, ported from KSG Code's `WorkLogRow`/`WorkLogLine` (`chat/WorkLog.tsx`):
/// a `rounded-md pe-1.5` row (`py-0.5` standalone, `py-0` inside a group list), a 24 pt
/// icon slot holding a 14 pt glyph in `text-dim-foreground`, one truncating `text-body`
/// label in `text-secondary-label`, then the edit counts, a 12 pt X on failure
/// (`text-tool-error-icon/40`), and a 12 pt chevron at 70% opacity when the row expands.
/// `interactive` is KSG's `canExpandProjectedItem`: only it makes the row a button.
/// `severe` is KSG's destructive row style for a failed test run: medium destructive text.
pub(super) fn activity_line(
    key: String,
    a11y: String,
    title: String,
    counts: Option<(usize, usize)>,
    failed: bool,
    severe: bool,
    open: bool,
    interactive: bool,
    listed: bool,
    glyph: &str,
    p: Palette,
) -> El {
    // KSG keeps a failed call's label and glyph quiet: the glyph drops to a 40% error
    // tint and a same-tint X sits in the trailing slot.
    let icon_ink = if failed { p.danger.alpha(0.4) } else { p.faint };
    let mut line = row()
        .key(key)
        .role(if interactive { "button" } else { "group" })
        .label(a11y)
        .min_w(0.0)
        .center_y()
        .gap(5.0)
        .min_h(24.0)
        .pr(6.0)
        .radius(6.0);
    if interactive {
        line = line
            .cursor(rune::Cursor::Pointer)
            .expanded(open)
            .focusable(true)
            .hover(p.selected);
    }
    if !listed {
        line = line.py(2.0);
    }
    line = line
        .child(
            el().size(24.0, 24.0)
                .center()
                .child(icon(glyph, icon_ink).size(14.0, 14.0)),
        )
        .child(
            label(
                title,
                if severe {
                    TextStyle::new(13.0, 500.0).tracking(-0.08)
                } else {
                    ACTIVITY
                },
                if severe { p.danger } else { p.muted },
            )
            .truncate()
            .shrink()
            .min_w(0.0),
        );
    if let Some((additions, deletions)) = counts {
        line = line.child(
            label(format!("+{additions} −{deletions}"), BAR_TIMER, p.faint).selectable(true),
        );
    }
    line = line.child(spacer());
    if failed {
        line = line.child(icon("x", icon_ink).size(12.0, 12.0));
    }
    if interactive {
        line = line.child(el().size(16.0, 16.0).center().child(
            icon(if open { "chevron" } else { "open" }, p.faint.alpha(0.7)).size(12.0, 12.0),
        ));
    }
    line
}

/// KSG Code's `WORKING_BAR_CLASS_NAME` (`chat/TimelineRowParts.tsx`): a 32 pt bar over a 1 px
/// `--timeline-working-rule` (the palette's `line`), 5 pt in at the start and 4 pt at the end.
/// A running turn's "Working for" and a finished turn's "Worked for" are this one bar.
pub(super) fn working_bar_frame(p: Palette) -> El {
    row()
        .w_full()
        .min_w(0.0)
        .h(32.0)
        .center_y()
        .gap(10.0)
        .pl(5.0)
        .pr(4.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
}

/// The folded work bar's button, ported from KSG Code's `WorkFoldToggle` and `WorkFoldCaret`:
/// muted 13 px text that brightens on hover, with a 12 pt chevron 8 pt after it. It goes in
/// a `working_bar_frame`.
pub(super) fn work_fold_toggle(
    key: String,
    a11y: String,
    title: &str,
    open: bool,
    p: Palette,
) -> El {
    row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(a11y)
        .focusable(true)
        .hover_group()
        .h_full()
        .center_y()
        .gap(8.0)
        .radius(6.0)
        .child(label(title, ACTIVITY, p.muted).group_color(p.ink))
        .child(icon(if open { "chevron" } else { "open" }, p.faint).size(12.0, 12.0))
}

/// The bar's live label, as KSG's `WorkingTimelineRow` words it: stopping, paused on a
/// request (the clock stops), compacting, or working, with the turn's tabular timer.
pub(super) fn working_label(t: &Thread) -> (&'static str, Option<u64>) {
    let elapsed = t.started.map(|start| start.elapsed().as_secs());
    let mut cache = t.timeline_cache.borrow_mut();
    if t.request.is_none() {
        cache.paused_at = None;
    }
    if t.phase == "Stopping" || t.stop_control.phase().is_some() {
        return ("Stopping…", None);
    }
    if t.request.is_some() {
        let at = *cache.paused_at.get_or_insert(elapsed.unwrap_or(0));
        return ("Paused at ", Some(at));
    }
    match (t.phase == "Compacting context", elapsed) {
        (true, Some(seconds)) => ("Compacting for ", Some(seconds)),
        (true, None) => ("Compacting…", None),
        (false, Some(seconds)) if seconds > 0 => ("Working for ", Some(seconds)),
        (false, _) => ("Working...", None),
    }
}

/// The tabular timer in the bar's own 13 px type, so each tick keeps its width.
pub(super) const BAR_TIMER: TextStyle = ACTIVITY.features(TABULAR);

/// Ported from KSG Code's `WorkingTimelineRow`: the running turn's 32 pt bar at its top.
/// When earlier work history exists before the live tail (or is currently folded), clicking the
/// bar folds or unfolds earlier work while keeping the live tail visible.
pub(super) fn working_bar(t: &Thread, start: usize, foldable: bool, p: Palette) -> El {
    let (words, seconds) = working_label(t);
    let expanded = !t.live_work_collapsed;
    let mut text = row().center_y().min_w(0.0).shrink().child(
        label(words, ACTIVITY, p.muted)
            .group_color(p.ink)
            .key(format!("working-label-{}", t.id))
            .truncate()
            .shrink()
            .min_w(0.0),
    );
    if let Some(seconds) = seconds {
        text = text.child(
            label(timer_label(seconds), BAR_TIMER, p.muted)
                .group_color(p.ink)
                .key(format!("live-timer-{}", t.id)),
        );
    }
    let key = format!("working-bar-{}", t.id);
    if !foldable {
        return working_bar_frame(p).key(key).role("status").child(text);
    }
    let press_key = key.clone();
    working_bar_frame(p).child(
        row()
            .key(key)
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(if expanded {
                "Collapse work history"
            } else {
                "Expand work history"
            })
            .expanded(expanded)
            .focusable(true)
            .hover_group()
            .h_full()
            .center_y()
            .gap(8.0)
            .radius(6.0)
            .child(text)
            .child(icon(if expanded { "chevron" } else { "open" }, p.faint).size(12.0, 12.0))
            .on_press_tx(move |s: &mut State, tx| {
                tx.keep_on_screen(press_key.clone());
                let t = s.current_mut();
                t.live_work_collapsed = !t.live_work_collapsed;
                t.timeline_cache.get_mut().touch_message(start);
                t.timeline_cache.get_mut().touch_disclosure();
            }),
    )
}

/// A tool call's collapsed row, ported from KSG Code's `WorkEntryLogRow`: one display
/// label, `+a −d` counts after an edit, " for Ns" after a running command, and a
/// disclosure only while the body has something to show.
pub(super) fn inline_tool(s: &State, p: Palette, index: usize, listed: bool) -> El {
    let t = s.current();
    let tool = &t.tools[index];
    if tool.resolved_kind() == crate::work::ToolKind::Html
        && crate::html_render::find_reference(&tool.structured).is_some()
    {
        return work_ui::html_card(s, p, index);
    }
    if tool.resolved_kind() == crate::work::ToolKind::Plan {
        return work_ui::plan_card(s, p, index);
    }
    let root = s.working_directory();
    let mut title = entry_label(tool, &root);
    let counts = (tool.resolved_kind() == crate::work::ToolKind::FileChange)
        .then(|| {
            (
                tool.changes.iter().map(|c| c.additions).sum::<usize>(),
                tool.changes.iter().map(|c| c.deletions).sum::<usize>(),
            )
        })
        .filter(|counts| counts.0 > 0 || counts.1 > 0);
    // KSG's RunningFor: " for 1m 5s" after a running command's label.
    if tool.resolved_kind() == crate::work::ToolKind::Command && !tool.done && !tool.failed {
        if let Some(seconds) = tool.started.map(|started| started.elapsed().as_secs()) {
            title.push_str(&format!(" for {}", elapsed_label(seconds)));
        }
    }
    let expandable = work_ui::has_body(tool);
    // KSG's severeFailure: a failed test command's label reads medium destructive.
    let severe = tool.failed
        && tool.resolved_kind() == crate::work::ToolKind::Command
        && crate::work::command_runs_tests(&work_ui::input_text(tool, &["command", "cmd"]));
    let a11y = if tool.failed {
        format!("{title}, tool call failed")
    } else {
        title.clone()
    };
    let tool_id = tool.id.clone();
    let line_key = format!("inline-tool-{}-{index}", t.id);
    let press_key = line_key.clone();
    let flashing = s.flash_tool.as_ref().is_some_and(|(id, _)| id == &tool.id);
    let mut content = col()
        .key(format!("tool-{}", tool.id))
        .gap(4.0)
        .radius(6.0)
        .fill(if flashing {
            p.sea_bg
        } else {
            Color::TRANSPARENT
        })
        .child(
            activity_line(
                line_key,
                a11y,
                title,
                counts,
                tool.failed,
                severe,
                tool.expanded,
                expandable,
                listed,
                work_ui::kind_icon(tool.resolved_kind()),
                p,
            )
            .on_press_tx(move |s: &mut State, tx| {
                tx.keep_on_screen(press_key.clone());
                let thread = s.current_mut();
                if let Some(tool) = thread.tools.iter_mut().find(|tool| tool.id == tool_id) {
                    tool.expanded = !tool.expanded;
                    let message = tool.anchor.message;
                    thread.timeline_cache.get_mut().touch_message(message);
                }
            }),
        );
    if !tool.subagents.is_empty() {
        content = content.child(pi_ui::subagent_results(s, tool, p));
    }
    if tool.expanded {
        content = content.child(work_ui::expanded(s, p, index));
    }
    content
}

pub(super) fn inline_tools(s: &State, p: Palette, indices: &[usize]) -> El {
    let entries: Vec<_> = indices.iter().map(|&i| WorkEntry::Tool(i)).collect();
    work_part(s, p, &entries)
}

/// The tool calls of a work streak, in order.
pub(super) fn entry_tools(entries: &[WorkEntry]) -> impl Iterator<Item = usize> + '_ {
    entries.iter().filter_map(|entry| match entry {
        WorkEntry::Tool(index) => Some(*index),
        WorkEntry::Thought(_) => None,
    })
}

/// The thoughts of a work streak, in order.
pub(super) fn entry_thoughts(entries: &[WorkEntry]) -> Vec<usize> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            WorkEntry::Thought(index) => Some(*index),
            WorkEntry::Tool(_) => None,
        })
        .collect()
}

/// Whether a work streak is unfolded. Its first tool call holds the flag, as a tool group's did;
/// a streak of thinking alone opens with its first thought.
pub(super) fn group_open(t: &Thread, entries: &[WorkEntry]) -> bool {
    match entry_tools(entries).next() {
        Some(index) => t.tools[index].group_expanded,
        None => entry_thoughts(entries)
            .first()
            .is_some_and(|&index| t.thoughts[index].expanded),
    }
}

/// Flips a work streak's fold flag. `tool_id` names its first call; `None` is a streak of
/// thinking alone, which folds with its first thought (`thought`).
pub(super) fn toggle_group(
    t: &mut Thread,
    tool_id: Option<&str>,
    thought: Option<(usize, usize, usize)>,
) {
    if let Some(id) = tool_id {
        if let Some(tool) = t.tools.iter_mut().find(|tool| tool.id == id) {
            tool.group_expanded = !tool.group_expanded;
            let message = tool.anchor.message;
            t.timeline_cache.get_mut().touch_message(message);
        }
    } else if let Some((message, block, offset)) = thought
        && let Some(thought) = t.thoughts.iter_mut().find(|thought| {
            thought.anchor.message == message
                && thought.block == block
                && thought.anchor.offset == offset
        })
    {
        thought.expanded = !thought.expanded;
        t.timeline_cache.get_mut().touch_message(message);
    }
}

/// One step of an unfolded work streak.
pub(super) fn work_entry_row(s: &State, p: Palette, entry: WorkEntry, listed: bool) -> El {
    match entry {
        WorkEntry::Tool(index) => inline_tool(s, p, index, listed),
        WorkEntry::Thought(index) => work_ui::thoughts(s, p, &[index], listed),
    }
}

/// A work streak as one transcript part: a lone call is its own row, thinking alone keeps its
/// "Thought" disclosure, and anything longer folds under one header, as KSG Code's
/// `WorkGroupToggleTimelineRow` and `WorkLogList` (16 pt in, `ps-4`) show it.
pub(super) fn work_part(s: &State, p: Palette, entries: &[WorkEntry]) -> El {
    let t = s.current();
    let thoughts = entry_thoughts(entries);
    if thoughts.len() == entries.len() {
        return work_ui::thoughts(s, p, &thoughts, false);
    }
    if entries.len() == 1 {
        return work_entry_row(s, p, entries[0], false);
    }
    let mut group = work_group_head(s, p, entries);
    if group_open(t, entries) {
        let mut rows = col().pl(WORK_LIST_INDENT).gap(WORK_GAP);
        for entry in entries {
            rows = rows.child(work_entry_row(s, p, *entry, true));
        }
        group = group.child(rows);
    }
    group
}

/// KSG's `WorkLogList` indent (`ps-4`) for the steps of an unfolded group.
pub(super) const WORK_LIST_INDENT: f32 = 16.0;
/// The space between work rows.
pub(super) const WORK_GAP: f32 = 2.0;

/// A work streak's keyed column and summary line, without its rows, ported from KSG Code's
/// `WorkGroupHeader`. The summary counts the calls (`summarizeToolGroup` leaves thinking out);
/// the icon is the calls' own when they are all one kind. Long turns place each row of an
/// unfolded streak in its own transcript slice.
pub(super) fn work_group_head(s: &State, p: Palette, entries: &[WorkEntry]) -> El {
    let t = s.current();
    let indices: Vec<usize> = entry_tools(entries).collect();
    let tools: Vec<_> = indices.iter().map(|index| &t.tools[*index]).collect();
    let first = indices[0];
    let first_id = tools[0].id.clone();
    let expanded = group_open(t, entries);
    // KSG's hasFailure: any call in the group, not just the last.
    let failed = tools.iter().any(|tool| tool.failed);
    let summary = tool_summary(&tools);
    let kind = tools[0].resolved_kind();
    let glyph = if tools.iter().all(|tool| tool.resolved_kind() == kind) {
        work_ui::kind_icon(kind)
    } else {
        "list"
    };
    let toggle_key = format!("toggle-tool-group-{}-{first}", t.id);
    let press_key = toggle_key.clone();
    col()
        .key(format!("tool-group-{}-{first}", t.id))
        .gap(WORK_GAP)
        .child(
            activity_line(
                toggle_key,
                format!("{} {summary}", if expanded { "Collapse" } else { "Expand" }),
                summary.clone(),
                None,
                failed,
                false,
                expanded,
                true,
                false,
                glyph,
                p,
            )
            .on_press_tx(move |s: &mut State, tx| {
                tx.keep_on_screen(press_key.clone());
                toggle_group(s.current_mut(), Some(&first_id), None);
            }),
        )
}

pub(super) fn message_key(thread: &Thread, index: usize) -> String {
    match thread.messages.get(index) {
        Some(message) => format!("message-{}-id-{}", thread.id, message.id),
        None => format!("message-{}-boundary-{index}", thread.id),
    }
}

pub(super) fn assistant_body_key(thread: &Thread, index: usize, from: usize) -> String {
    format!("{}-body-{from}", message_key(thread, index))
}

pub(super) fn transcript_part(
    s: &State,
    p: Palette,
    wide: bool,
    part: &TranscriptPart,
    occ_base: &mut usize,
) -> El {
    let t = s.current();
    match part {
        TranscriptPart::Tools(indices) => crew_ui::tool_part(s, p, indices),
        TranscriptPart::Thought(indices) => work_ui::thoughts(s, p, indices, false),
        TranscriptPart::Work(entries) => work_part(s, p, entries),
        TranscriptPart::Compaction(index) => pi_ui::compaction_activity(s, p, *index),
        TranscriptPart::Retry(index) => work_ui::retry_row(s, p, *index),
        TranscriptPart::Text { message, from, to } => {
            let chunk = &t.messages[*message].text[*from..*to];
            let message_id = &t.messages[*message].id;
            let offset = *from;
            let thread_id = t.id;
            let streaming = t.status == Status::Running && message + 1 == t.messages.len();
            let find_query = (s.thread_find_active()
                && !s.thread_find.query.text.trim().is_empty())
            .then(|| s.thread_find.query.text.trim());
            let cite_query = find_query.is_none().then_some(()).and_then(|()| {
                s.citation_highlight
                    .as_ref()
                    .filter(|hl| {
                        hl.machine == s.machines.selected
                            && hl.thread_id == thread_id
                            && hl.message_id == *message_id
                            && !hl.text.trim().is_empty()
                    })
                    .map(|hl| hl.text.trim())
            });
            let mut cite_matched = false;
            let find_param = if let Some(query) = find_query {
                let hits = crate::thread_find::find_markdown_occurrences(chunk, query).len();
                let active_occ = s
                    .thread_find
                    .active_match
                    .as_ref()
                    .filter(|m| m.entry_id == *message_id)
                    .and_then(|m| {
                        (m.occurrence >= *occ_base && m.occurrence < *occ_base + hits)
                            .then(|| m.occurrence - *occ_base)
                    });
                *occ_base += hits;
                Some((query, active_occ))
            } else if let Some(query) = cite_query {
                if !crate::thread_find::find_markdown_occurrences(chunk, query).is_empty() {
                    cite_matched = true;
                    Some((query, Some(0)))
                } else {
                    None
                }
            } else {
                None
            };
            let memo_find = find_param.map(|(q, occ)| (q.to_owned(), occ));
            let rendered = memo(
                assistant_body_key(t, *message, *from),
                (
                    chunk,
                    s.dark,
                    streaming,
                    wide,
                    s.code_wrap_version,
                    memo_find,
                ),
                || {
                    markdown_ui::render_with_find(
                        chunk,
                        p,
                        s.dark,
                        wide,
                        false,
                        streaming,
                        &format!("{thread_id}-{message_id}-{offset}"),
                        &s.unwrapped_code_blocks,
                        find_param,
                    )
                },
            );
            if cite_query.is_some() && !cite_matched {
                rendered.fill(p.coral_bg).radius(6.0)
            } else {
                rendered
            }
        }
    }
}

/// What the agent is doing while no step of the live tail is in progress, as KSG Code's
/// `ThinkingTimelineRow` words it: "Thinking", "Waiting on subagents", or a phase that says
/// more, such as a retry.
pub(super) fn live_status(t: &Thread, start: usize) -> String {
    let waiting = t.tools.iter().any(|tool| {
        tool.anchor.message >= start
            && !tool.done
            && !tool.failed
            && (tool.resolved_kind() == crate::work::ToolKind::Subagent
                || crew_ui::is_delegation(tool))
    });
    if waiting {
        return "Waiting on subagents".into();
    }
    match t.phase.as_str() {
        ""
        | "Working"
        | "Waiting for the model"
        | "Thinking"
        | "Stopping"
        | "Compacting context" => "Thinking".into(),
        phase if phase.starts_with("Running ") => "Thinking".into(),
        phase => phase.into(),
    }
}

/// The live tail of a running turn: the work streak after its latest prose, which KSG Code's
/// `activeWorkRow` shows as one `work-live` row.
#[derive(Clone, Copy, Debug)]
pub(super) struct LiveTail {
    pub(super) part: usize,
    /// The newest step with something to show (thinking with no text yet has none).
    pub(super) latest: WorkEntry,
    /// The newest step still in progress.
    pub(super) running: Option<WorkEntry>,
    /// The newest step is a call that failed, and nothing is running.
    pub(super) failed: bool,
    /// The row stands for what the turn is doing now: a step runs, or the last call
    /// succeeded and nothing waits on the user.
    pub(super) active: bool,
    /// The newest thought with text, for the preview line.
    pub(super) thought: Option<usize>,
}

pub(super) fn live_tail(t: &Thread, parts: &[TranscriptPart]) -> Option<LiveTail> {
    let part = parts.len().checked_sub(1)?;
    let TranscriptPart::Work(entries) = &parts[part] else {
        return None;
    };
    let visible = |entry: &&WorkEntry| match entry {
        WorkEntry::Tool(_) => true,
        WorkEntry::Thought(index) => !t.thoughts[*index].text.trim().is_empty(),
    };
    let latest = *entries.iter().rfind(visible)?;
    let running = entries
        .iter()
        .filter(visible)
        .rfind(|entry| match entry {
            WorkEntry::Tool(index) => !t.tools[*index].done && !t.tools[*index].failed,
            WorkEntry::Thought(index) => !t.thoughts[*index].done,
        })
        .copied();
    let latest_tool = match latest {
        WorkEntry::Tool(index) => Some(&t.tools[index]),
        WorkEntry::Thought(_) => None,
    };
    let failed = running.is_none() && latest_tool.is_some_and(|tool| tool.failed);
    let active = running.is_some()
        || latest_tool.is_some_and(|tool| tool.done && !tool.failed) && t.request.is_none();
    let thought = entries
        .iter()
        .filter(visible)
        .rev()
        .find_map(|entry| match entry {
            WorkEntry::Thought(index) => Some(*index),
            WorkEntry::Tool(_) => None,
        });
    Some(LiveTail {
        part,
        latest,
        running,
        failed,
        active,
        thought,
    })
}

/// How much of a thought KSG reads for its preview (`THOUGHT_PREVIEW_SOURCE_MAX_LENGTH`).
pub(super) const THOUGHT_PREVIEW_SOURCE: usize = 8_192;

/// Ported from KSG Code's `liveThoughtLine` (`work-log/presentation.ts`): a thought's bold
/// heading, or else its first sentence, as plain text.
pub(super) fn live_thought_line(markdown: &str) -> String {
    let markdown = match markdown.char_indices().nth(THOUGHT_PREVIEW_SOURCE) {
        Some((at, _)) => &markdown[..at],
        None => markdown,
    };
    let heading = markdown.trim_start().strip_prefix("**").and_then(|rest| {
        let end = rest.find("**")?;
        let inner = &rest[..end];
        let after = rest[end + 2..].split('\n').next().unwrap_or("");
        (!inner.is_empty()
            && !inner.contains(['*', '\r', '\n'])
            && after
                .trim_end_matches('\r')
                .chars()
                .all(|c| c == ' ' || c == '\t'))
        .then_some(inner)
    });
    let source = heading.unwrap_or(markdown);
    // Links and images keep their text.
    let mut unlinked = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(open) = rest.find('[') {
        let (before, from) = rest.split_at(open);
        let close = from.find(']');
        let link = close
            .filter(|close| from[close + 1..].starts_with('('))
            .and_then(|close| {
                let end = from[close + 2..].find(')')?;
                Some((close, close + 2 + end))
            });
        match link {
            Some((close, end)) if !from[1..close].contains(']') => {
                unlinked.push_str(before.strip_suffix('!').unwrap_or(before));
                unlinked.push_str(&from[1..close]);
                rest = &from[end + 1..];
            }
            _ => {
                unlinked.push_str(before);
                unlinked.push('[');
                rest = &from[1..];
            }
        }
    }
    unlinked.push_str(rest);
    // Headings, bullets and list numbers at the start of a line.
    let mut lines = vec![];
    for line in unlinked.split('\n') {
        let trimmed = line.trim_start_matches([' ', '\t']);
        let marker = trimmed
            .find([' ', '\t'])
            .filter(|&at| {
                let mark = &trimmed[..at];
                (1..=6).contains(&mark.len()) && mark.chars().all(|c| c == '#')
                    || matches!(mark, "-" | "*" | "+")
                    || mark.len() > 1
                        && mark.ends_with('.')
                        && mark[..mark.len() - 1].chars().all(|c| c.is_ascii_digit())
            })
            .map(|at| trimmed[at..].trim_start_matches([' ', '\t']));
        lines.push(marker.unwrap_or(line));
    }
    let joined = lines
        .join("\n")
        .replace("**", "")
        .replace("~~", "")
        .replace('`', "");
    // Single `*` or `_` emphasis around words.
    let chars: Vec<char> = joined.chars().collect();
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut plain = String::with_capacity(joined.len());
    for (i, c) in chars.iter().enumerate() {
        let (before, after) = (
            i.checked_sub(1).and_then(|i| chars.get(i)),
            chars.get(i + 1),
        );
        let opens = !word(before) && word(after) && after != Some(&'_');
        let closes = word(before) && !word(after) && after != Some(&'*');
        if matches!(c, '*' | '_') && (opens || closes) {
            continue;
        }
        plain.push(*c);
    }
    let text = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if heading.is_some() {
        return text;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, &(at, c)) in chars.iter().enumerate() {
        if !matches!(c, '.' | '?' | '!') {
            continue;
        }
        let mut end = at + c.len_utf8();
        let mut next = chars.get(i + 1).map(|&(_, c)| c);
        if let Some(close) = next.filter(|c| matches!(c, '"' | '\'' | '”' | '’' | ')')) {
            end += close.len_utf8();
            next = chars.get(i + 2).map(|&(_, c)| c);
        }
        if next.is_some_and(char::is_whitespace) {
            return text[..end].to_owned();
        }
    }
    text
}

/// The sweep's window (`--live-activity-focus-width`, 4.5 rem) and period.
pub(super) const SHIMMER_WINDOW: f32 = 72.0;
pub(super) const SHIMMER_PERIOD: f64 = 2.2;

/// Ported from KSG Code's `ActivityShimmerOverlay` and `.live-activity-focus`: a copy of the
/// row in the foreground colour shows through a 72 pt window with soft edges that sweeps the
/// row every 2.2 s. The window slides one way and the copy inside it the other, so the copy
/// stays over the row. The renderer runs both slides on its own clock, so the sweep costs no
/// view runs or wakes, and it leaves with the live row: a settled window draws nothing.
pub(super) fn shimmer(base: El, highlight: El, width: f32) -> El {
    let travel = width + SHIMMER_WINDOW;
    let sweep = |to: f32| {
        rune::LoopSpec::slide(SHIMMER_PERIOD, [0.0, 0.0], [to, 0.0]).easing(rune::Easing::LINEAR)
    };
    el().w_full().min_w(0.0).child(base).child(
        el().absolute(-SHIMMER_WINDOW, 0.0)
            .w(SHIMMER_WINDOW)
            .h_full()
            .clip_fade(SHIMMER_WINDOW / 2.0, 0.0, SHIMMER_WINDOW / 2.0, 0.0)
            .looping(sweep(travel))
            .inert()
            .a11y_hidden()
            .child(
                el().absolute(SHIMMER_WINDOW, 0.0)
                    .w(width)
                    .looping(sweep(-travel))
                    .child(highlight),
            ),
    )
}

/// Whether live rows sweep: the turn runs, nothing waits on the user, and motion is allowed.
pub(super) fn shimmer_allowed(s: &State, t: &Thread) -> bool {
    t.request.is_none() && !s.portraits.reduced_motion()
}

/// The width a live row lays out at, for the shimmer's copy: the lane.
pub(super) fn live_row_width(s: &State) -> f32 {
    (s.html_lane_width as f32).max(0.0)
}

/// KSG Code's `LiveActivityContent`: a 24 pt icon slot and one truncating label. The glyph
/// sits in `text-dim-foreground`, the label in `text-secondary-label`; the shimmer's
/// highlight copy brightens both to the foreground.
pub(super) fn activity_content(
    glyph: &str,
    text: &str,
    ink: Color,
    icon_ink: Color,
    key: Option<String>,
) -> El {
    let mut words = label(text, ACTIVITY, ink).truncate().shrink().min_w(0.0);
    if let Some(key) = key {
        words = words.key(key);
    }
    // A full-width row bounds the label, so it truncates at the lane instead of overflowing.
    row()
        .min_h(24.0)
        .center_y()
        .gap(5.0)
        .w_full()
        .min_w(0.0)
        .child(
            el().size(24.0, 24.0)
                .center()
                .child(icon(glyph, icon_ink).size(14.0, 14.0)),
        )
        .child(words)
}

/// KSG Code's `LiveActivityRow`: the muted row, swept by the shimmer while it is the turn's
/// current activity.
pub(super) fn live_activity(
    s: &State,
    p: Palette,
    glyph: &str,
    text: &str,
    animated: bool,
    key: Option<String>,
) -> El {
    let base = activity_content(glyph, text, p.muted, p.faint, key);
    if !animated {
        return base;
    }
    shimmer(
        base,
        activity_content(glyph, text, p.ink, p.ink, None),
        live_row_width(s),
    )
}

/// Ported from KSG Code's `ThinkingTimelineRow`: "Thinking" with the brain icon at the bottom
/// of a running turn, while no step of its tail is in progress.
pub(super) fn live_status_row(s: &State, t: &Thread, start: usize, p: Palette) -> El {
    row()
        .key(format!("live-status-{}", t.id))
        .role("status")
        .w_full()
        .min_w(0.0)
        .child(live_activity(
            s,
            p,
            "brain",
            &live_status(t, start),
            shimmer_allowed(s, t),
            Some(format!("live-status-text-{}", t.id)),
        ))
}

/// KSG Code's `LiveWorkToggle`: a live row that is its work group's disclosure. Same
/// chrome as the settled rows (`rounded-md pe-1.5`).
pub(super) fn live_toggle(key: String, a11y: String, p: Palette) -> El {
    col()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(a11y)
        .focusable(true)
        .w_full()
        .min_w(0.0)
        .pr(6.0)
        .radius(6.0)
        .hover(p.selected)
}

/// Ported from KSG Code's `LiveWorkEntryTimelineRow`: a running turn's tail as one row. The
/// newest thought's first line sits above the step in progress, which replaces itself as
/// each call starts and ends; the whole row unfolds the tail's steps. When the last call
/// failed, the "Thinking" row stands in for it and does the unfolding, as KSG's does.
pub(super) fn live_work(
    s: &State,
    p: Palette,
    parts: &[TranscriptPart],
    tail: LiveTail,
    start: usize,
) -> El {
    let t = s.current();
    let TranscriptPart::Work(entries) = &parts[tail.part] else {
        return el();
    };
    let open = group_open(t, entries);
    let first_tool = entry_tools(entries)
        .next()
        .map(|index| t.tools[index].id.clone());
    let first_thought = entry_thoughts(entries).first().map(|&index| {
        let thought = &t.thoughts[index];
        (thought.anchor.message, thought.block, thought.anchor.offset)
    });
    let toggle = move |key: String| {
        move |s: &mut State, tx: &mut rune::Tx| {
            tx.keep_on_screen(key.clone());
            toggle_group(s.current_mut(), first_tool.as_deref(), first_thought);
        }
    };
    let verb = if open { "Hide" } else { "Show" };
    if tail.failed {
        let key = format!("live-status-{}", t.id);
        return live_toggle(key.clone(), format!("{verb} steps. A tool call failed"), p)
            .child(live_activity(
                s,
                p,
                "brain",
                &live_status(t, start),
                shimmer_allowed(s, t),
                Some(format!("live-status-text-{}", t.id)),
            ))
            .on_press_tx(toggle(key));
    }
    let thought_line = tail
        .thought
        .filter(|_| !open)
        .map(|index| live_thought_line(&t.thoughts[index].text))
        .filter(|line| !line.is_empty());
    let entry = tail.running.unwrap_or(tail.latest);
    let (glyph, text) = match entry {
        WorkEntry::Thought(index) => (
            "brain",
            if open || thought_line.is_some() {
                if tail.active { "Thinking" } else { "Thought" }.to_owned()
            } else {
                t.thoughts[index]
                    .text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            },
        ),
        WorkEntry::Tool(index) => {
            let tool = &t.tools[index];
            (
                work_ui::kind_icon(tool.resolved_kind()),
                live_label(tool, &s.working_directory()),
            )
        }
    };
    let key = format!("live-work-{}", t.id);
    let mut live_row = live_toggle(key.clone(), format!("{verb} steps. {text}"), p);
    if let Some(line) = thought_line {
        live_row = live_row.child(
            row()
                .key(format!("live-thought-{}", t.id))
                .gap(5.0)
                .min_w(0.0)
                .child(
                    el().size(24.0, 24.0)
                        .center()
                        .child(icon("brain", p.faint).size(14.0, 14.0)),
                )
                .child(
                    label(line, ACTIVITY.lh(20.0), p.muted)
                        .wrap()
                        .max_lines(4)
                        .pt(2.0)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0),
                ),
        );
    }
    let failed = matches!(entry, WorkEntry::Tool(index) if t.tools[index].failed);
    live_row
        .child(live_activity(
            s,
            p,
            glyph,
            &text,
            tail.active && !failed && shimmer_allowed(s, t),
            Some(format!("live-work-text-{}", t.id)),
        ))
        .on_press_tx(toggle(key))
}
