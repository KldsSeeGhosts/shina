//! Provider tasks inside a thread, separate from durable crew threads.
use super::*;
use crate::agents::{self, AgentRun, AgentState, AgentsFilter};

pub(super) fn open(s: &mut State) {
    s.right_panels
        .open(&s.panel_key(), crate::right_panel::SingletonKind::Work);
}

pub(super) fn chip(key: &str, active: usize, p: Palette) -> El {
    row()
        .key(key)
        .gap(3.0)
        .center_y()
        .child(icon("agents", p.sea).size(12.0, 12.0))
        .child(label(active.to_string(), TIMER, p.sea))
}

pub(super) fn status(state: AgentState) -> (Status, &'static str) {
    match state {
        AgentState::Queued => (Status::Ready, "Queued"),
        AgentState::Working => (Status::Running, "Working"),
        AgentState::Done => (Status::Completed, "Done"),
        AgentState::Failed => (Status::Failed, "Failed"),
        AgentState::Stopped => (Status::Ready, "Stopped"),
    }
}

fn jump(key: &str, tool: &str, p: Palette) -> El {
    let id = tool.to_owned();
    icon_button(key, "Show in transcript", "corner-down-left", p, move |s| {
        if s.inspector_covers() {
            s.right_panels.close(&s.panel_key());
        }
        s.reveal_tool(id.clone());
    })
}

/// Status and provider treatment follow KSG Code's `ProviderSubagentBar`. The row shares
/// the crew roster's 10 pt inset and 6 pt radius so both halves of the Work tab line up.
fn agent_row(s: &State, p: Palette, run: &AgentRun<'_>) -> El {
    let key = run.owned_key(&s.machines.selected, s.current().id);
    let expanded = s.agents_expanded.contains(&key);
    let suffix = format!("{}-{}", run.tool_id, run.index);
    let (state, word) = status(run.state);
    let model = run.model.filter(|model| !model.is_empty());
    let elapsed = if run.state.active() {
        run.started.map(|at| at.elapsed().as_secs())
    } else {
        run.duration_ms.map(|ms| ms / 1000)
    };
    let summary = run
        .activity
        .filter(|line| !line.is_empty())
        .unwrap_or(run.brief);
    let ink = status_ink(state, p);
    let mut name = row().center_y().gap(5.0).min_w(0.0).child(
        label(run.name.as_ref(), TITLE, p.ink)
            .truncate()
            .grow(1.0)
            .min_w(0.0),
    );
    if let Some(model) = model {
        name = name.child(outlined(
            label(model, SMALL, p.faint)
                .truncate()
                .max_w(80.0)
                .px(4.0)
                .radius(6.0),
            p,
        ));
    }
    name = name.child(
        label(
            elapsed.map_or_else(|| word.to_owned(), elapsed_label),
            TIMER,
            ink,
        )
        .key(format!("agent-elapsed-{suffix}")),
    );
    let header = row()
        .key(format!("agent-row-{suffix}"))
        .h(44.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .hover(p.selected)
        .hover_motion(MOTION_FAST, MOTION_FAST)
        .role("button")
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .label(format!("{}, {word}", run.name))
        .expanded(expanded)
        .on_press(move |s: &mut State| {
            if !s.agents_expanded.remove(&key) {
                s.agents_expanded.insert(key.clone());
            }
        })
        .child(crewmate_avatar(
            s,
            p,
            model.unwrap_or(&s.current().model),
            state,
            24.0,
        ))
        .child(
            col().gap(1.0).grow(1.0).min_w(0.0).child(name).child(
                label(
                    if summary.is_empty() { word } else { summary },
                    SMALL,
                    p.muted,
                )
                .truncate(),
            ),
        )
        .child(
            jump(&format!("agent-jump-{suffix}"), run.tool_id, p).description(format!(
                "Originating call at message {}",
                run.anchor.message.saturating_add(1)
            )),
        )
        .child(
            icon(
                if expanded {
                    "chevron-up"
                } else {
                    "chevron-down"
                },
                p.faint,
            )
            .size(12.0, 12.0),
        );
    let mut card = col().w_full().child(header);
    if expanded {
        let mut detail = col()
            .mx(10.0)
            .mb(6.0)
            .padding(8.0)
            .gap(6.0)
            .radius(8.0)
            .fill(p.card)
            .border(1.0, p.frame);
        if !run.brief.is_empty() {
            detail = detail.child(label("Task", META, p.faint)).child(
                label(run.brief, BODY, p.ink)
                    .wrap_anywhere()
                    .selectable(true),
            );
        }
        if let Some(activity) = run.activity.filter(|value| !value.is_empty()) {
            detail = detail.child(label("Latest activity", META, p.faint)).child(
                label(activity, SMALL, p.muted)
                    .wrap_anywhere()
                    .selectable(true),
            );
        }
        if !run.output.is_empty() {
            let failed = run.state == AgentState::Failed;
            let output = run.output.to_owned();
            detail = detail
                .child(label("Output", META, p.faint))
                .child(
                    col()
                        .key(format!("agent-output-{suffix}"))
                        .max_h(220.0)
                        .radius(6.0)
                        .fill(if failed { p.danger_bg } else { p.tray })
                        .padding(8.0)
                        .scroll(ScrollSpec {
                            stick_bottom: run.state.active(),
                        })
                        .child(
                            label(
                                run.output,
                                TextStyle::new(13.0, 400.0).lh(20.0),
                                if failed { p.danger } else { p.ink },
                            )
                            .wrap_anywhere()
                            .selectable(true),
                        ),
                )
                .child(
                    button(&format!("agent-copy-{suffix}"), "Copy output", p, |_| {})
                        .on_press_tx(move |_: &mut State, tx| tx.copy(output.clone())),
                );
        }
        card = card.child(detail.key(format!("agent-detail-{suffix}")));
    }
    card
}

#[derive(Hash)]
enum AgentRow {
    Group { tool_id: String, count: usize },
    Run { tool: usize, index: usize },
}

pub(super) fn view(s: &State, p: Palette) -> El {
    let t = s.current();
    let active = agents::active_count(t);
    let total = agents::total_count(t);
    let runs = agents::runs(t);
    let mut group_counts = std::collections::HashMap::new();
    for run in &runs {
        *group_counts.entry(run.tool_id).or_insert(0) += run.count;
    }
    // One 32 pt header in the crew groups' style: the section word and its count on the
    // left, the active and finished split on the right.
    let mut summary = row()
        .key("agents-summary")
        .h(32.0)
        .px(10.0)
        .center_y()
        .gap(6.0)
        .child(meta_label("Agents", p.muted))
        .child(meta_text(total.to_string(), p.faint))
        .child(spacer())
        .child(
            label(
                format!(
                    "{active} active · {} finished",
                    total.saturating_sub(active)
                ),
                TIMER,
                p.faint,
            )
            .truncate(),
        );
    if active > 0 && active < total {
        for (filter, key, word) in [
            (AgentsFilter::All, "agents-filter-all", "All"),
            (AgentsFilter::Active, "agents-filter-active", "Active"),
        ] {
            summary = summary.child(
                button(key, word, p, move |s| s.agents_filter = filter)
                    .checked(s.agents_filter == filter)
                    .fill(if s.agents_filter == filter {
                        p.lift
                    } else {
                        Color::TRANSPARENT
                    }),
            );
        }
    }
    let mut list = col().gap(1.0).min_h(0.0).grow(1.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    let mut previous = None;
    let mut rows: Vec<(String, AgentRow)> = Vec::new();
    for run in runs
        .iter()
        .filter(|run| s.agents_filter != AgentsFilter::Active || active == 0 || run.state.active())
    {
        if previous != Some(run.tool_id) {
            previous = Some(run.tool_id);
            let count = group_counts[run.tool_id];
            rows.push((
                format!("agents-group-{}", run.tool_id),
                AgentRow::Group {
                    tool_id: run.tool_id.into(),
                    count,
                },
            ));
        }
        rows.push((
            format!("agent-slot-{}-{}", run.tool_id, run.index),
            AgentRow::Run {
                tool: run.tool_position,
                index: run.index,
            },
        ));
    }
    if rows.is_empty() {
        list = list.child(
            label("Agent details load with the conversation.", SMALL, p.muted)
                .wrap()
                .px(10.0),
        );
    } else {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        s.panel_key().hash(&mut hash);
        rows.hash(&mut hash);
        let rows = std::sync::Arc::new(rows);
        let keys = rows.clone();
        list = list.child(
            col()
                .key("agents-rows")
                .w_full()
                .gap(1.0)
                .growth(rune::GrowthSpec::default())
                .virtual_rows(
                    rows.len(),
                    44.0,
                    move |_: &State, index| keys[index].0.clone().into(),
                    move |s: &State, index| match &rows[index].1 {
                        AgentRow::Group { tool_id, count } => row()
                            .h(28.0)
                            .px(10.0)
                            .center_y()
                            .child(
                                label(
                                    format!(
                                        "Delegated {count} {}",
                                        if *count == 1 { "task" } else { "tasks" }
                                    ),
                                    META_LABEL,
                                    p.faint,
                                )
                                .grow(1.0),
                            )
                            .child(jump(&format!("agents-jump-{tool_id}"), tool_id, p)),
                        AgentRow::Run { tool, index } => agents::run(s.current(), *tool, *index)
                            .map_or_else(el, |run| agent_row(s, p, &run)),
                    },
                )
                .virtual_rows_version(hash.finish()),
        );
    }
    col()
        .key("agents-panel")
        .w_full()
        .h_full()
        .min_h(0.0)
        .pt(4.0)
        .child(summary)
        .child(list)
}
