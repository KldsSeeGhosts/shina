//! The floating thread details card, ported from KSG Code's `ThreadDetailsCard`,
//! `ThreadDetailsPanel`, `ThreadDetailsSection`, `ThreadDetailsControl` and
//! `ThreadRelationshipsControl` (`apps/web/src/components/chat`), with
//! `threadDetailsPanelStyles.ts` and `index.css`'s `.thread-details-stack`.
//!
//! A 272 pt stack of tray cards docks 12 pt inside the chat pane's top right corner and the
//! chat lane moves clear of it (`canvas_layout`). Where the pane is too narrow to dock it,
//! the top bar's details button opens the same stack as a popover. The workspace card says
//! where the thread runs. The Lineage card lists who delegated it and the agents it started:
//! crew and provider subagents, the finished ones folded under "N previous agents". A row
//! opens its agent in the right panel or as its own thread.
use super::crew_ui::{active_wash, hover_wash, motion};
use super::*;
use crate::agents::{self, AgentRunId};
use crate::canvas_layout::{DetailsCardEdges, DetailsCardRect};

/// KSG's `THREAD_LINEAGE_PAGE_COUNT`: how many more previous agents one press lists.
const LINEAGE_PAGE: usize = 12;
/// `text-label leading-4`: a lineage row's title.
const LINEAGE_TITLE: TextStyle = TextStyle::new(14.0, 500.0).lh(16.0);
/// `text-caption leading-4`: a lineage row's second line.
const LINEAGE_CAPTION: TextStyle = TextStyle::new(12.0, 500.0).lh(16.0);

/// A blank thread shows its hero and workspace pickers, so the card waits for a
/// conversation. A crew worker always has it, for the way back to its Captain.
pub(super) fn available(s: &State) -> bool {
    let t = s.current();
    !(t.messages.is_empty() && t.tools.is_empty())
        || t.crew_captain.is_some()
        || t.delegated
        || t.peer_origin.is_some()
}

/// Where the card docks in a chat pane `pane_width` wide (KSG's
/// `resolveThreadDetailsCardLayout`), or `None` when it has to be a popover.
pub(super) fn rect(s: &State, pane_width: f32) -> Option<DetailsCardRect> {
    if !available(s) {
        return None;
    }
    crate::canvas_layout::resolve_thread_details_card_layout(
        crate::canvas_layout::ThreadDetailsCardInput {
            container_width: pane_width,
            container_height: s.details.pane_height.get(),
            lane_padding: lane_gutter(pane_width),
            ..Default::default()
        },
    )
}

/// The docked card as the obstacle the chat lane moves clear of.
pub(super) fn edges(s: &State, pane_width: f32) -> Option<DetailsCardEdges> {
    if !s.details.docked {
        return None;
    }
    rect(s, pane_width).map(|rect| rect.edges())
}

/// KSG Code's `ThreadDetailsSection` card: `surface-tray rounded-pane p-1`, a 14 px tray
/// with 4 px of padding that holds an optional heading and one inner surface.
fn details_tray(key: &str, title: &str, p: Palette) -> El {
    col()
        .key(key)
        .role("group")
        .label(title)
        .w_full()
        .min_w(0.0)
        .padding(4.0)
        .radius(14.0)
        .fill(p.tray)
        .border(1.0, p.line)
}

/// The card's 28 px heading (`flex h-7 items-center gap-1 ps-3`): the title, then a muted
/// caption at the end.
fn details_heading(title: &str, summary: Option<String>, p: Palette) -> El {
    let mut heading = row()
        .h(28.0)
        .pl(12.0)
        .pr(8.0)
        .gap(4.0)
        .center_y()
        .child(meta_label(title, p.muted))
        .child(spacer());
    if let Some(summary) = summary {
        heading = heading.child(label(summary, TIMER, p.muted));
    }
    heading
}

/// The card's inner list: `surface-composer rounded-lg p-0.5`.
fn details_surface(p: Palette) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .padding(2.0)
        .radius(10.0)
        .fill(p.card_top)
}

/// The frame of KSG's `THREAD_DETAILS_PANEL_ROW_CLASS`: a 28 px row, 10 px in and 4 px at
/// the end, with 10 px between its parts.
fn details_frame(key: &str) -> El {
    row()
        .key(key)
        .h(28.0)
        .min_w(0.0)
        .pl(10.0)
        .pr(4.0)
        .gap(10.0)
        .center_y()
        .radius(8.0)
}

/// A details row: a 16 px muted icon before a medium label. Callers add the trailing meta
/// or action.
fn details_row(key: &str, glyph: &str, title: &str, p: Palette) -> El {
    details_frame(key)
        .child(icon(glyph, p.muted).size(16.0, 16.0))
        .child(label(title, TITLE, p.ink).truncate().grow(1.0).min_w(0.0))
}

/// A row the user can press: KSG's `hover:surface-selected-soft`.
fn details_action(s: &State, p: Palette, row_: El, a11y: String) -> El {
    row_.role("button")
        .cursor(rune::Cursor::Pointer)
        .label(a11y)
        .focusable(true)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
}

/// A row's trailing caption (`THREAD_DETAILS_PANEL_META_CLASS`): tabular and muted.
fn details_meta(value: impl Into<String>, p: Palette) -> El {
    label(value, TIMER, p.muted).pr(6.0)
}

/// The rows behind "Show N more": the model, access, harness, last update, session ID and
/// session notices. KSG's card folds its own extra rows the same way
/// (`ThreadDetailsMoreButton`). These facts are Shina's.
fn session_rows(s: &State, p: Palette) -> Vec<El> {
    let t = s.current();
    let model = if t.thinking.is_empty() || t.thinking == "off" {
        t.model.clone()
    } else {
        format!("{} · {}", t.model, effort_name(&t.thinking))
    };
    let mut rows = vec![
        details_row("overview-model", "chip", &model, p),
        details_row("overview-access", "shield", t.native.access.label(), p),
        details_row(
            "overview-harness",
            "terminal",
            harness_name(s, &t.harness),
            p,
        ),
    ];
    if let Some(updated) = relative_time(t.updated) {
        rows.push(
            details_row("overview-updated", "clock", "Updated", p).child(details_meta(updated, p)),
        );
    }
    if let Some(session) = &t.session {
        let copy = session.clone();
        rows.push(
            details_frame("overview-session")
                .child(icon("file", p.muted).size(16.0, 16.0))
                .child(
                    label(session, MONO_META, p.muted)
                        .truncate_middle()
                        .selectable(true)
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    icon_button(
                        "overview-session-copy",
                        "Copy session ID",
                        "copy",
                        p,
                        |_| {},
                    )
                    .on_press_tx(move |_: &mut State, tx| tx.copy(copy.clone())),
                ),
        );
    }
    if !t.diagnostics.is_empty() {
        let key = s.panel_key();
        let expanded = s.session_notices.contains(&key);
        rows.push(
            details_action(
                s,
                p,
                details_row("overview-session-notices", "info", "Session notices", p),
                format!("Session notices, {}", t.diagnostics.len()),
            )
            .expanded(expanded)
            .on_press(move |s: &mut State| {
                if !s.session_notices.remove(&key) {
                    s.session_notices.insert(key.clone());
                }
            })
            .child(details_meta(t.diagnostics.len().to_string(), p))
            .child(
                icon(if expanded { "chevron" } else { "open" }, p.faint)
                    .size(12.0, 12.0)
                    .mr(6.0),
            ),
        );
        if expanded {
            for diagnostic in &t.diagnostics {
                rows.push(
                    label(truncate(diagnostic, 600), SMALL, p.muted)
                        .wrap_anywhere()
                        .selectable(true)
                        .px(10.0)
                        .py(4.0),
                );
            }
        }
    }
    rows
}

/// The workspace card, ported from KSG Code's `ThreadDetailsPanel` Workspace section: where
/// the thread runs (machine, workspace, branch, changes) in the inner surface, the rest
/// behind "Show N more", then the context meter under it (`ThreadDetailsContext`).
fn workspace_card(s: &State, p: Palette) -> El {
    let t = s.current();
    let mut rows = details_surface(p).child(details_row(
        "overview-machine",
        "monitor",
        &s.machine_label(&s.machines.selected),
        p,
    ));
    if let Some(project) = s.project(t.project) {
        rows = rows.child(details_row(
            "overview-workspace",
            "folder",
            &project.name,
            p,
        ));
    }
    if let Some(branch) = &t.branch {
        let copy = branch.clone();
        rows = rows.child(
            details_row("overview-branch", "branch", branch, p).child(
                icon_button(
                    "overview-branch-copy",
                    "Copy branch name",
                    "copy",
                    p,
                    |_| {},
                )
                .on_press_tx(move |_: &mut State, tx| tx.copy(copy.clone())),
            ),
        );
    }
    if let Some(ready) = s
        .diffs
        .get(&s.panel_key())
        .and_then(diff_panel::ThreadDiff::ready)
        && (ready.additions > 0 || ready.deletions > 0)
    {
        let stat =
            diff_panel::diff_stat_label("overview-diff-stat", ready.additions, ready.deletions, p);
        rows = rows.child(
            details_action(
                s,
                p,
                details_row("overview-changes", "diff", "Changes", p),
                "Open changes".into(),
            )
            .on_press(|s: &mut State| {
                let key = s.panel_key();
                s.details.popover = false;
                s.right_panels
                    .open(&key, crate::right_panel::SingletonKind::Diff);
            })
            .child(stat.pr(6.0)),
        );
    }
    // KSG's `ThreadDetailsMoreButton`: a muted row that unfolds the rest of the list.
    let more = s.details.more;
    let extra = session_rows(s, p);
    let count = extra.len();
    if more {
        for row_ in extra {
            rows = rows.child(row_);
        }
    }
    rows = rows.child(
        details_action(
            s,
            p,
            details_frame("details-more")
                .child(
                    icon(if more { "chevron-up" } else { "chevron-down" }, p.muted)
                        .size(16.0, 16.0),
                )
                .child(label(
                    if more {
                        "Show less".to_string()
                    } else {
                        format!("Show {count} more")
                    },
                    TITLE,
                    p.muted,
                )),
            if more {
                "Show fewer details".into()
            } else {
                format!("Show {count} more details")
            },
        )
        .expanded(more)
        .on_press(|s: &mut State| s.details.more = !s.details.more),
    );
    let mut card = details_tray("overview-workspace-card", "Workspace", p).child(rows);
    // KSG's context row: a 32 px line with the gauge, a 4 px meter and the percentage. It
    // opens the Context tab, where the breakdown lives.
    if let Some(percent) = t.context.usage.percent {
        let amount = (percent as f32 / 100.0).clamp(0.0, 1.0);
        card = card.child(
            details_action(
                s,
                p,
                row()
                    .key("overview-context")
                    .h(32.0)
                    .pl(12.0)
                    .pr(8.0)
                    .gap(6.0)
                    .center_y()
                    .radius(8.0)
                    .child(icon("gauge", p.muted).size(16.0, 16.0))
                    .child(label("Context", TITLE, p.ink)),
                format!("Context, {percent:.0}% used. Open the Context tab"),
            )
            .on_press(|s: &mut State| {
                let key = s.panel_key();
                s.details.popover = false;
                s.right_panels
                    .open(&key, crate::right_panel::SingletonKind::Context);
            })
            .child(
                row()
                    .h(4.0)
                    .ml(6.0)
                    .min_w(48.0)
                    .grow(1.0)
                    .radius(2.0)
                    .fill(p.line)
                    .child(el().grow(amount).h_full().radius(2.0).fill(p.ink))
                    .child(el().grow(1.0 - amount)),
            )
            .child(label(context_percent_text(percent), TIMER, p.muted)),
        );
        if more
            && let (Some(tokens), Some(limit)) =
                (t.context.usage.tokens, t.context.usage.context_window)
        {
            card = card.child(
                details_surface(p).child(
                    row()
                        .key("overview-context-tokens")
                        .h(32.0)
                        .px(10.0)
                        .gap(8.0)
                        .center_y()
                        .child(label("Used / max", TITLE, p.ink).grow(1.0).min_w(0.0))
                        .child(label(
                            format!(
                                "{} / {}",
                                context_tokens_text(tokens),
                                context_tokens_text(limit)
                            ),
                            TIMER,
                            p.muted,
                        )),
                ),
            );
        }
    }
    card
}

/// What pressing a lineage row opens.
#[derive(Clone)]
enum Target {
    /// A crew worker's own thread.
    Crew(usize),
    /// A provider subagent, unfolded in the right panel's Work tab.
    Agent((String, usize, String, AgentRunId)),
}

/// One agent in the Lineage card: a crew worker or a provider subagent.
struct LineageRow {
    key: String,
    title: String,
    /// The model's ID, which picks the avatar.
    model: String,
    /// The model's display name, as the model picker words it.
    model_name: String,
    status: Status,
    word: String,
    /// The elapsed time, or the status word when no time is known.
    clock: String,
    /// What it is doing or what it returned, for the hover card.
    note: String,
    target: Target,
}

fn model_name(s: &State, model: &str) -> String {
    if model.is_empty() {
        String::new()
    } else {
        s.friendly_model_name(&s.current().harness, model)
    }
}

/// The thread's agents, as KSG's `useThreadLineage` groups them: the ones still at work,
/// then the finished ones, newest first.
fn lineage_rows(s: &State) -> (Vec<LineageRow>, Vec<LineageRow>) {
    let t = s.current();
    let (mut active, mut previous) = (Vec::new(), Vec::new());
    for member in inspector_ui::crew_members(s) {
        let live = matches!(member.status, Status::Running | Status::Waiting) || member.queued;
        let row_ = LineageRow {
            key: format!("lineage-row-crew-{}", member.thread_id),
            title: member.title,
            model_name: model_name(s, &member.model),
            model: member.model,
            status: member.status,
            note: member
                .place
                .map_or_else(|| member.word.clone(), |place| place.replace('\n', " · ")),
            word: member.word,
            clock: member.clock,
            target: Target::Crew(member.thread_id),
        };
        if live {
            active.push(row_);
        } else {
            previous.push(row_);
        }
    }
    let mut finished = Vec::new();
    for run in agents::runs(t) {
        let (status, word) = agents_ui::status(run.state);
        let elapsed = if run.state.active() {
            run.started.map(|at| at.elapsed().as_secs())
        } else {
            run.duration_ms.map(|ms| ms / 1000)
        };
        let note = run
            .activity
            .filter(|line| !line.is_empty())
            .or_else(|| {
                run.output
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
            })
            .unwrap_or(run.brief);
        let row_ = LineageRow {
            key: format!("lineage-row-agent-{}-{}", run.tool_id, run.index),
            title: run.name.to_string(),
            model_name: model_name(s, run.model.unwrap_or_default()),
            model: run.model.unwrap_or_default().to_string(),
            status,
            word: word.into(),
            clock: elapsed.map_or_else(|| word.to_owned(), elapsed_label),
            note: truncate(note, 200),
            target: Target::Agent(run.owned_key(&s.machines.selected, t.id)),
        };
        if run.state.active() {
            active.push(row_);
        } else {
            finished.push(row_);
        }
    }
    finished.reverse();
    previous.extend(finished);
    (active, previous)
}

/// One agent, as KSG's lineage rows draw it: a 40 px two-line row with the agent's avatar,
/// its title over its model, and its state and time at the end. Hovering it shows the
/// agent's card beside the stack (`hover_card`).
fn lineage_row(s: &State, p: Palette, item: &LineageRow) -> El {
    let failed = item.status == Status::Failed;
    let mut first = row().center_y().gap(6.0).min_w(0.0).child(
        label(&item.title, LINEAGE_TITLE, p.ink)
            .truncate()
            .grow(1.0)
            .min_w(0.0),
    );
    first = match item.status {
        Status::Failed => first.child(label("Failed", LINEAGE_CAPTION, p.danger)),
        Status::Running => first.child(status_mark(Status::Running, p, 8.0)),
        Status::Waiting => first.child(label(&item.word, LINEAGE_CAPTION, p.brass)),
        _ => first,
    };
    let mut second = row().center_y().gap(6.0).min_w(0.0);
    if !item.model.is_empty() {
        second = second.child(
            label(&item.model_name, LINEAGE_CAPTION, p.muted)
                .truncate()
                .shrink()
                .min_w(0.0),
        );
    }
    second = second.child(spacer());
    if !failed {
        second = second.child(label(
            &item.clock,
            LINEAGE_CAPTION.features(TABULAR),
            p.muted,
        ));
    }
    let model = if item.model.is_empty() {
        s.current().model.clone()
    } else {
        item.model.clone()
    };
    let (hover_key, target) = (item.key.clone(), item.target.clone());
    details_action(
        s,
        p,
        row()
            .key(item.key.clone())
            .min_h(40.0)
            .min_w(0.0)
            .pl(10.0)
            .pr(8.0)
            .gap(10.0)
            .center_y()
            .radius(8.0)
            .child(crewmate_avatar(s, p, &model, item.status, 16.0))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(first)
                    .child(second),
            ),
        format!("{}. {}. Open", item.title, item.word),
    )
    .on_hover_change(move |s: &mut State, hot, _| {
        if hot {
            s.details.hover = Some(hover_key.clone());
        } else if s.details.hover.as_deref() == Some(hover_key.as_str()) {
            s.details.hover = None;
        }
    })
    .on_press(move |s: &mut State| {
        s.details.hover = None;
        s.details.popover = false;
        match &target {
            Target::Crew(id) => super::crew_ui::open_thread(s, *id),
            Target::Agent(key) => {
                s.agents_expanded.insert(key.clone());
                agents_ui::open(s);
            }
        }
    })
}

/// The Lineage card, ported from KSG Code's `ThreadRelationshipsPanel`: who delegated the
/// thread and where it runs, the agents at work, then `ThreadLineageGroup`'s folded
/// "N previous agents" list, which grows a page at a time.
fn lineage_card(s: &State, p: Palette) -> Option<El> {
    let t = s.current();
    let captain = t
        .captain
        .or(t.crew_captain)
        .filter(|c| *c < s.captains.len() && !s.is_captain_home(t));
    let mut rows = details_surface(p).gap(2.0);
    let mut any = false;
    if let Some(captain) = captain {
        let profile = &s.captains[captain];
        any = true;
        rows = rows.child(
            details_action(
                s,
                p,
                details_frame("open-captain-from-overview")
                    .child(avatar(s, p, captain, 16.0, s.captain_status(captain)))
                    .child(
                        label(&profile.name, TITLE, p.ink)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
                format!("Open {}", profile.name),
            )
            .on_press(move |s: &mut State| {
                s.details.popover = false;
                s.open_captain(captain);
            })
            .child(details_meta("Captain", p))
            .child(icon("open", p.faint).size(12.0, 12.0).mr(6.0)),
        );
    } else if let Some(origin) = &t.peer_origin {
        let server_id = origin.server_id.clone();
        let home_id = origin.parent_thread_id;
        let linked = s
            .client
            .as_ref()
            .is_some_and(|c| c.linked.contains_key(&server_id));
        any = true;
        let base = details_row(
            "open-captain-from-overview",
            "crew",
            &format!("{} on {}", origin.captain, origin.server_label),
            p,
        );
        rows = rows.child(if linked {
            details_action(
                s,
                p,
                base,
                format!("Open {} on {}", origin.captain, origin.server_label),
            )
            .on_press(move |s: &mut State| {
                s.details.popover = false;
                s.open_machine_thread(&server_id, home_id);
            })
            .child(icon("open", p.faint).size(12.0, 12.0).mr(6.0))
        } else {
            base
        });
    }
    if let Some(placement) = crew_ui::worker_placement(s, t.id) {
        let machine = crew_ui::placement_machine(s, t.id, &placement);
        any = true;
        rows = rows.child(
            label(
                crew_ui::placement_details(&placement, &machine),
                SMALL,
                p.muted,
            )
            .key("worker-placement")
            .wrap()
            .px(10.0)
            .py(6.0),
        );
    }
    let (active, previous) = lineage_rows(s);
    for item in &active {
        any = true;
        rows = rows.child(lineage_row(s, p, item));
    }
    if !previous.is_empty() {
        any = true;
        let open = s.details.previous_open;
        let count = previous.len();
        let failed = previous
            .iter()
            .filter(|item| item.status == Status::Failed)
            .count();
        let title = format!(
            "{count} previous {}",
            if count == 1 { "agent" } else { "agents" }
        );
        let mut header = details_action(
            s,
            p,
            details_row("lineage-previous", "history", &title, p).h(32.0),
            title.clone(),
        )
        .expanded(open)
        .on_press(|s: &mut State| s.details.previous_open = !s.details.previous_open);
        if failed > 0 {
            header = header.child(label(format!("{failed} failed"), TIMER, p.danger));
        }
        rows = rows.child(
            header.child(
                icon(if open { "chevron" } else { "open" }, p.faint)
                    .size(12.0, 12.0)
                    .mr(6.0),
            ),
        );
        if open {
            let shown = s.details.previous_shown.min(count);
            for item in &previous[..shown] {
                rows = rows.child(lineage_row(s, p, item));
            }
            let page = (count - shown).min(LINEAGE_PAGE);
            if page > 0 {
                rows = rows.child(
                    details_action(
                        s,
                        p,
                        details_frame("lineage-more")
                            .child(icon("plus", p.muted).size(16.0, 16.0))
                            .child(label(format!("Show {page} more"), TITLE, p.muted)),
                        format!("Show {page} more agents"),
                    )
                    .on_press(|s: &mut State| s.details.previous_shown += LINEAGE_PAGE),
                );
            }
        }
    }
    let running = active
        .iter()
        .filter(|item| item.status == Status::Running)
        .count();
    any.then(|| {
        details_tray("brief-delegated", "Lineage", p)
            .child(details_heading(
                "Lineage",
                (running > 0).then(|| format!("{running} running")),
                p,
            ))
            .child(rows)
    })
}

/// KSG's `.thread-details-stack` lifts its own text a step: `--foreground` is `#f5f5f5` and
/// `--muted-foreground` `#a3a3a3` in dark, and `--foreground` `#18181b` in light.
fn stack_palette(s: &State, p: Palette) -> Palette {
    if s.dark {
        Palette {
            ink: color("#f5f5f5"),
            muted: color("#a3a3a3"),
            ..p
        }
    } else {
        Palette {
            ink: color("#18181b"),
            ..p
        }
    }
}

/// The card: KSG's `thread-details-stack`, tray cards 4 px apart in a 272 px column that
/// scrolls once it is taller than `max_h`. The caller docks it or anchors it as a popover.
pub(super) fn card(s: &State, p: Palette, max_h: f32) -> El {
    let p = stack_palette(s, p);
    let mut stack = col()
        .w_full()
        .min_w(0.0)
        .gap(4.0)
        .child(workspace_card(s, p));
    if let Some(lineage) = lineage_card(s, p) {
        stack = stack.child(lineage);
    }
    col()
        .key("details-card")
        .role("group")
        .label("Thread details")
        .w(crate::canvas_layout::THREAD_DETAILS_CARD_WIDTH)
        .max_h(max_h)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(stack)
}

/// The agent under the pointer, drawn beside the card as KSG's lineage tooltip: its title,
/// what it runs on, its state and time, and what it last said.
pub(super) fn hover_card(s: &State, p: Palette) -> Option<El> {
    let key = s.details.hover.as_ref()?;
    let (active, previous) = lineage_rows(s);
    let item = active
        .iter()
        .chain(&previous)
        .find(|item| item.key == *key)?;
    let p = stack_palette(s, p);
    let mut card = col()
        .key("lineage-hover")
        .role("tooltip")
        .label(format!("{}. {}", item.title, item.word))
        .w(320.0)
        .padding(12.0)
        .gap(6.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .anchored_fit(key.clone(), rune::Side::Left, rune::Align::Start, 8.0)
        .child(label(&item.title, TextStyle::new(13.0, 600.0).lh(18.0), p.strong).truncate());
    if !item.model.is_empty() {
        card = card.child(
            row()
                .center_y()
                .gap(6.0)
                .child(crewmate_avatar(s, p, &item.model, item.status, 16.0))
                .child(label(&item.model_name, TITLE, p.muted).truncate()),
        );
    }
    card = card.child(
        row()
            .center_y()
            .gap(6.0)
            .child(status_mark(item.status, p, 8.0))
            .child(label(&item.word, TITLE, status_ink(item.status, p)))
            .child(spacer())
            .child(label(&item.clock, TIMER, p.muted)),
    );
    if !item.note.is_empty() && item.note != item.word {
        card = card.child(
            row()
                .center_y()
                .gap(6.0)
                .min_w(0.0)
                .child(icon("terminal", p.muted).size(14.0, 14.0))
                .child(
                    label(&item.note, TITLE, p.muted)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    Some(card)
}
