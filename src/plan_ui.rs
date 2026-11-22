//! A Captain home's task plan panel (issue #34), opened from the crew tray's Plan control.
//!
//! The panel sits where KSG Code's `QueuedRunsControl` sits: a compact banner above the
//! composer, a header row with a count, and a list capped at a scrolling height whose rows
//! each carry their own actions. Its node rows take the shape of that queue's rows. The plan's
//! status badge follows `ProposedPlanCard`'s "Plan" badge beside a truncating title. KSG has
//! no counterpart for an editable dependency graph, revision checks, launch receipts or the
//! audited override confirmation: those rows are Shina designs in the crew tray's language
//! (4 px rows, 1 px `line` chips, Inter status words, `framed` only on actions).
//!
//! Everything shown is the value the MCP `task_plan` tool returns for this Captain home, and
//! every action is a `task.plan.*` command routed to the same reducer as the MCP tools.
use super::crew_ui::{ghost_button, hover_wash, motion, solid_button};
use super::*;
use rune::ScrollSpec;
use serde_json::Value;

const ROW_RADIUS: f32 = 4.0;
/// The panel's body scrolls past KSG's `max-h-32` queue height, since a plan row carries a
/// dependency line and a launch line under its title. It takes at most a fifth of the window.
const LIST_MAX: f32 = 300.0;
/// Dependency and launch lines start under the title, past the checkbox and the badge gap.
const LINE_INDENT: f32 = 26.0;
const META: TextStyle = TextStyle::new(12.0, 400.0).lh(17.0);
const FIELD: TextStyle = TextStyle::new(13.0, 400.0).lh(19.0);

/// The word and ink for a node's plan status.
fn status_word(status: &str, p: Palette) -> (&'static str, Color) {
    match status {
        "ready" => ("Ready", p.sea),
        "blocked" => ("Blocked", p.danger),
        "launched" => ("Launched", p.ink),
        _ => ("Waiting", p.brass),
    }
}

fn dep_ink(state: &str, p: Palette) -> Color {
    match state {
        "satisfied" => p.sea,
        "overridden" => p.brass,
        "failed" | "cancelled" | "interrupted" | "missing" => p.danger,
        _ => p.muted,
    }
}

/// The Plan control at the right of the crew tray's header.
pub(super) fn plan_toggle(s: &State, p: Palette) -> Option<El> {
    s.plan_home()?;
    let open = s.plan_panel_open();
    let unavailable = s.plan_capability().err();
    let count = s
        .plan_view()
        .and_then(|view| {
            view["plans"].as_array().map(|plans| {
                plans
                    .iter()
                    .flat_map(|plan| plan["nodes"].as_array().into_iter().flatten())
                    .filter(|node| node["status"] != "launched")
                    .count()
            })
        })
        .unwrap_or(0);
    let mut button = row()
        .key("crew-tray-plan")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(if open {
            "Close the task plan"
        } else {
            "Open the task plan"
        })
        .checked(open)
        .focusable(true)
        .center_y()
        .gap(6.0)
        .h(24.0)
        .px(8.0)
        .radius(ROW_RADIUS)
        .fill(if open { p.selected } else { Color::TRANSPARENT })
        .hover(hover_wash(s, p))
        .hover_motion(motion(), motion())
        .disabled(unavailable.is_some())
        .on_press(|s: &mut State| s.toggle_plan_panel())
        .child(icon("list", if open { p.ink } else { p.muted }))
        .child(meta_label("Plan", if open { p.ink } else { p.muted }));
    if count > 0 {
        button = button.child(meta_text(count.to_string(), p.muted).key("crew-tray-plan-count"));
    }
    Some(match unavailable {
        Some(reason) => button.help(reason).opacity(0.5),
        None => button,
    })
}

fn checkbox(key: String, title: String, on: bool, enabled: bool, p: Palette, node: String) -> El {
    let mut mark = el()
        .key(key)
        .role("checkbox")
        .label(title)
        .checked(on)
        .focusable(enabled)
        .disabled(!enabled)
        .cursor(if enabled {
            rune::Cursor::Pointer
        } else {
            rune::Cursor::Default
        })
        .size(14.0, 14.0)
        .min_w(14.0)
        .border(1.0, if enabled { p.ink } else { p.line })
        .center()
        .fill(if on { p.coral } else { Color::TRANSPARENT });
    if on {
        mark = mark.child(icon("check", p.on_coral).size(10.0, 10.0));
    }
    if enabled {
        mark = mark.on_press(move |s: &mut State| s.plan_toggle_node(&node));
    }
    mark
}

fn short(value: &str, n: usize) -> String {
    value.chars().take(n).collect()
}

/// One draft or launched node: select, status, title, revision and edit, then the reason it
/// waits on each dependency, then the identity of its launch.
fn node_row(s: &State, p: Palette, node: &Value, busy: bool) -> El {
    let key = node["key"].as_str().unwrap_or_default().to_owned();
    let status = node["status"].as_str().unwrap_or("draft");
    let launched = status == "launched";
    let (word, ink) = status_word(status, p);
    let selected = s.plan_panel.selected.contains(&key);
    let title = node["title"].as_str().unwrap_or(&key).to_owned();
    let mut head = row()
        .gap(8.0)
        .center_y()
        .h(28.0)
        .child(if launched {
            el().size(14.0, 14.0).min_w(14.0)
        } else {
            checkbox(
                format!("plan-select-{key}"),
                format!("Select {title} to start"),
                selected,
                !busy,
                p,
                key.clone(),
            )
        })
        .child(
            outlined(
                row()
                    .key(format!("plan-status-{key}"))
                    .h(18.0)
                    .px(5.0)
                    .center_y()
                    .radius(4.0)
                    .child(meta_label(word, ink)),
                p,
            )
            .min_w(0.0),
        )
        .child(
            label(&title, TextStyle::new(13.0, 500.0), p.ink)
                .key(format!("plan-title-{key}"))
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(spacer())
        .child(
            meta_text(format!("{key} r{}", node["revision"]), p.faint)
                .key(format!("plan-revision-{key}"))
                .truncate()
                .shrink()
                .min_w(0.0),
        );
    if launched {
        if let Some(thread) = node["launch"]["thread_id"].as_u64() {
            let thread = thread as usize;
            head = head.child(ghost_button(
                s,
                format!("plan-open-{key}"),
                "Open",
                p,
                move |s: &mut State| super::crew_ui::open_thread(s, thread),
            ));
        }
    } else {
        let edit = key.clone();
        head = head.child(
            icon_button(
                &format!("plan-edit-{key}"),
                &format!("Edit {title}"),
                "edit",
                p,
                move |s: &mut State| s.plan_edit_node(&edit),
            )
            .disabled(busy),
        );
    }
    let mut card = col()
        .key(format!("plan-node-{key}"))
        .role("group")
        .label(format!("{title}. {word}"))
        .px(8.0)
        .py(4.0)
        .gap(2.0)
        .radius(ROW_RADIUS)
        .fill(if selected {
            p.selected
        } else {
            Color::TRANSPARENT
        })
        .child(head);
    if let Some(deps) = node["dependencies"].as_array().filter(|d| !d.is_empty()) {
        let stacked = node["stacked_on"].as_str();
        let mut line = row()
            .key(format!("plan-deps-{key}"))
            .pl(LINE_INDENT)
            .gap(6.0)
            .flex_wrap()
            .child(label("After", META, p.muted));
        for dep in deps {
            let name = dep["node"].as_str().unwrap_or_default();
            let state = dep["state"].as_str().unwrap_or("waiting");
            let detail = dep["detail"].as_str().unwrap_or_default();
            let mut chip = row()
                .key(format!("plan-dep-{key}-{name}"))
                .gap(5.0)
                .center_y()
                .px(5.0)
                .h(20.0)
                .radius(4.0)
                .border(1.0, p.line)
                .label(format!("{name}: {detail}"))
                .help(detail.to_owned())
                .child(label(name, META, p.ink))
                .child(meta_text(state, dep_ink(state, p)));
            if stacked == Some(name) {
                chip = chip.child(meta_text("base", p.muted));
            }
            line = line.child(chip);
        }
        card = card.child(line);
        // A dependency that holds the node back says why, in the server's words.
        let reasons: Vec<&str> = deps
            .iter()
            .filter(|dep| dep["satisfied"] != true && !launched)
            .filter_map(|dep| dep["detail"].as_str())
            .collect();
        if !reasons.is_empty() {
            card = card.child(
                label(
                    reasons.join("; "),
                    META,
                    if status == "blocked" {
                        p.danger
                    } else {
                        p.muted
                    },
                )
                .key(format!("plan-reason-{key}"))
                .wrap()
                .pl(LINE_INDENT)
                .w_full(),
            );
        }
    }
    if let Some(launch) = node["launch"].as_object() {
        let task = launch
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let thread = launch.get("thread_id").and_then(Value::as_u64).unwrap_or(0);
        let state = launch
            .get("task_state")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        let request = launch
            .get("request_key")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut identity = format!(
            "Task {} on thread #{thread}, {state}. Request {}",
            short(task, 12),
            short(request, 28)
        );
        if let Some(base) = launch.get("resolved_base").filter(|b| !b.is_null()) {
            identity.push_str(&format!(
                ". Based on {} at {}{}",
                base["node"].as_str().unwrap_or_default(),
                short(base["commit"].as_str().unwrap_or_default(), 10),
                if base["uncommitted"] == true {
                    ", upstream had uncommitted changes"
                } else {
                    ""
                }
            ));
        }
        if let Some(overrode) = launch
            .get("overrode")
            .and_then(Value::as_array)
            .filter(|o| !o.is_empty())
        {
            let names: Vec<&str> = overrode.iter().filter_map(Value::as_str).collect();
            identity.push_str(&format!(". Overrode {}", names.join(", ")));
        }
        card = card.child(
            label(identity, META, p.faint)
                .key(format!("plan-launch-{key}"))
                .selectable(true)
                .wrap_anywhere()
                .pl(LINE_INDENT)
                .w_full(),
        );
    }
    card
}

fn field(
    edit: &rune::TextEdit,
    key: &str,
    title: &str,
    hint: &str,
    multiline: bool,
    p: Palette,
    pick: fn(&mut State) -> &mut rune::TextEdit,
) -> El {
    let input = text_input(edit, FIELD)
        .key(key)
        .label(title)
        .placeholder(hint, p.muted)
        .color(p.ink)
        .w_full()
        .px(8.0)
        .py(6.0)
        .radius(ROW_RADIUS)
        .fill(p.card)
        .field_focus(edit, p)
        .on_edit(move |s: &mut State| pick(s));
    if multiline {
        input.min_h(52.0).max_h(120.0)
    } else {
        input.h(32.0)
    }
}

fn editor_of(s: &mut State) -> &mut crate::model::task_plan_client::PlanEditor {
    s.plan_panel.editor.get_or_insert_with(Default::default)
}

/// Create or revise one draft. A revision names the revision it started from, so a draft
/// changed elsewhere refuses the save instead of overwriting it.
fn editor(s: &State, p: Palette, plan: Option<&Value>) -> El {
    let editor = s.plan_panel.editor.as_ref().expect("editor open");
    let creating = editor.node.is_none();
    let heading = match &editor.node {
        Some(node) => format!("Edit {node}, revision {}", editor.revision),
        None if editor.plan.is_some() => "New draft".into(),
        None => "New plan".into(),
    };
    let mut form = col()
        .key("plan-editor")
        .role("group")
        .label(heading.clone())
        .gap(6.0)
        .padding(8.0)
        .radius(ROW_RADIUS)
        .border(1.0, p.line)
        .fill(p.panel)
        .child(label(heading, TextStyle::new(13.0, 600.0), p.ink));
    if editor.conflict {
        form = form.child(
            row()
                .key("plan-edit-conflict")
                .gap(8.0)
                .center_y()
                .flex_wrap()
                .child(
                    label(
                        format!(
                            "{} Load the latest to discard your edits, or keep yours and save over it.",
                            s.plan_panel
                                .error
                                .as_deref()
                                .unwrap_or("This draft changed since you opened it.")
                        ),
                        META,
                        p.danger,
                    )
                    .key("plan-error")
                    .wrap()
                    .w(0.0)
                    .grow(1.0)
                    .min_w(160.0),
                )
                .child(ghost_button(
                    s,
                    "plan-edit-reload".into(),
                    "Load latest",
                    p,
                    {
                        let node = editor.node.clone().unwrap_or_default();
                        move |s: &mut State| s.plan_edit_node(&node)
                    },
                ))
                .child(ghost_button(
                    s,
                    "plan-edit-rebase".into(),
                    "Keep my edits",
                    p,
                    |s: &mut State| s.plan_rebase_edit(),
                )),
        );
    }
    if creating {
        form = form.child(field(
            &editor.key,
            "plan-edit-key",
            "Node key",
            "A short key, unique in this plan",
            false,
            p,
            |s| &mut editor_of(s).key,
        ));
    }
    form = form.child(field(
        &editor.title,
        "plan-edit-title",
        "Title",
        "Title",
        false,
        p,
        |s| &mut editor_of(s).title,
    ));
    // Dependencies sit under the title, where a short edit keeps them in view.
    let others: Vec<String> = plan
        .and_then(|plan| plan["nodes"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|node| node["key"].as_str().map(str::to_owned))
        .filter(|key| editor.node.as_deref() != Some(key))
        .collect();
    if !others.is_empty() {
        let mut deps = row()
            .key("plan-edit-deps")
            .gap(6.0)
            .flex_wrap()
            .center_y()
            .child(label("Starts after", META, p.muted));
        for other in &others {
            let on = editor.depends_on.contains(other);
            let stacked = editor.stacked_on.as_deref() == Some(other.as_str());
            let (toggle, stack) = (other.clone(), other.clone());
            let mut chip = row()
                .gap(2.0)
                .center_y()
                .radius(4.0)
                .border(1.0, if on { p.frame } else { p.line })
                .child(
                    row()
                        .key(format!("plan-edit-dep-{other}"))
                        .role("checkbox")
                        .checked(on)
                        .label(format!("Start after {other}"))
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .h(22.0)
                        .px(6.0)
                        .center_y()
                        .hover(hover_wash(s, p))
                        .on_press(move |s: &mut State| s.plan_toggle_dependency(&toggle))
                        .child(label(other, META, if on { p.ink } else { p.muted })),
                );
            if on {
                // A stacked node starts from this upstream's verified commit, not the default
                // branch.
                chip = chip.child(
                    row()
                        .key(format!("plan-edit-stack-{other}"))
                        .role("switch")
                        .checked(stacked)
                        .label(format!("Use {other}'s commit as the base"))
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .h(22.0)
                        .px(6.0)
                        .center_y()
                        .fill(if stacked {
                            p.selected
                        } else {
                            Color::TRANSPARENT
                        })
                        .hover(hover_wash(s, p))
                        .on_press(move |s: &mut State| s.plan_toggle_stack(&stack))
                        .child(meta_text("base", if stacked { p.ink } else { p.faint })),
                );
            }
            deps = deps.child(chip);
        }
        form = form.child(deps);
    }
    form = form
        .child(field(
            &editor.brief,
            "plan-edit-brief",
            "Brief",
            "The complete brief the worker receives",
            true,
            p,
            |s| &mut editor_of(s).brief,
        ))
        .child(field(
            &editor.outputs,
            "plan-edit-outputs",
            "Expected outputs",
            "What you will check when it finishes (optional)",
            true,
            p,
            |s| &mut editor_of(s).outputs,
        ));
    form
}

/// The editor's Cancel and Save, pinned in the panel's footer so they never scroll away.
fn editor_actions(s: &State, p: Palette, busy: bool) -> El {
    let editor = s.plan_panel.editor.as_ref().expect("editor open");
    let creating = editor.node.is_none();
    let blocked = editor.brief.text.trim().is_empty()
        || creating && editor.key.text.trim().is_empty()
        || busy
        || editor.conflict;
    row()
        .gap(6.0)
        .center_y()
        .child(spacer())
        .child(ghost_button(
            s,
            "plan-edit-cancel".into(),
            "Cancel",
            p,
            |s: &mut State| s.plan_panel.editor = None,
        ))
        .child(
            solid_button(
                "plan-edit-save".into(),
                if creating {
                    "Save draft"
                } else {
                    "Save revision"
                },
                p,
                |s: &mut State| s.plan_save_draft(),
            )
            .disabled(blocked)
            .opacity(if blocked { 0.5 } else { 1.0 }),
        )
}

/// The explicit override step: every selected node that is not ready, why, and the reason the
/// plan will record before anything starts.
fn override_confirm(p: Palette, s: &State) -> El {
    let mut list = col().gap(2.0);
    for (node, reason) in s.plan_unready() {
        list = list.child(
            label(format!("{node}: {reason}"), META, p.ink)
                .key(format!("plan-override-node-{node}"))
                .wrap()
                .w_full(),
        );
    }
    let reason = &s.plan_panel.override_reason;
    col()
        .key("plan-override")
        .role("alertdialog")
        .label("Start with an override")
        .gap(6.0)
        .padding(8.0)
        .radius(ROW_RADIUS)
        .border(1.0, p.brass)
        .child(
            label(
                "These nodes are not ready. Starting them records an override on the plan.",
                TextStyle::new(13.0, 600.0),
                p.ink,
            )
            .wrap()
            .w_full(),
        )
        .child(list)
        .child(field(
            reason,
            "plan-override-reason",
            "Override reason",
            "Why start before the dependencies are satisfied",
            false,
            p,
            |s| &mut s.plan_panel.override_reason,
        ))
}

/// Cancel, or start with the recorded reason. The reason is required.
fn override_actions(s: &State, p: Palette, busy: bool) -> El {
    let blocked = s.plan_panel.override_reason.text.trim().is_empty() || busy;
    row()
        .gap(6.0)
        .center_y()
        .child(spacer())
        .child(ghost_button(
            s,
            "plan-override-cancel".into(),
            "Cancel",
            p,
            |s: &mut State| s.plan_cancel_start(),
        ))
        .child(
            solid_button(
                "plan-override-confirm".into(),
                "Start with override",
                p,
                |s: &mut State| s.plan_start_selected(),
            )
            .disabled(blocked)
            .opacity(if blocked { 0.5 } else { 1.0 }),
        )
}

/// The task plan panel, mounted inside the Inspector's Crew tab when toggled open.
pub(super) fn plan_panel(s: &State, p: Palette, _lane: f32) -> Option<El> {
    if !s.plan_panel_open() {
        return None;
    }
    let busy = s.plan_panel.pending.is_some();
    let view = s.plan_view();
    let plans: Vec<Value> = view
        .as_ref()
        .and_then(|view| view["plans"].as_array().cloned())
        .unwrap_or_default();
    let plan = s.shown_plan();
    let nodes: Vec<Value> = plan
        .as_ref()
        .and_then(|plan| plan["nodes"].as_array().cloned())
        .unwrap_or_default();
    let ready = nodes.iter().filter(|n| n["status"] == "ready").count();
    let mut header = row()
        .key("plan-header")
        .h(28.0)
        .px(4.0)
        .gap(8.0)
        .center_y()
        .child(outlined(
            row()
                .h(18.0)
                .px(5.0)
                .center_y()
                .radius(4.0)
                .child(meta_label("Plan", p.ink)),
            p,
        ))
        .child(
            label(
                plan.as_ref()
                    .and_then(|plan| plan["id"].as_str())
                    .unwrap_or("No plan yet"),
                TextStyle::new(13.0, 500.0),
                p.ink,
            )
            .key("plan-id")
            .truncate()
            .shrink()
            .min_w(0.0),
        )
        .child(spacer());
    if !nodes.is_empty() {
        header = header.child(
            meta_text(format!("{ready} ready of {}", nodes.len()), p.muted).key("plan-count"),
        );
    }
    header = header.child(icon_button(
        "plan-close",
        "Close the task plan",
        "close",
        p,
        |s: &mut State| s.toggle_plan_panel(),
    ));
    let editor_open = s.plan_panel.editor.is_some();
    let min_body = if editor_open { 144.0 } else { 60.0 };
    let mut panel = col()
        .key("plan-panel")
        .role("region")
        .label("Task plan")
        .shrink()
        .gap(6.0)
        .padding(6.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.line)
        .child(header);
    if plans.len() > 1 {
        let mut tabs = row().key("plan-tabs").gap(4.0).flex_wrap();
        for other in &plans {
            let id = other["id"].as_str().unwrap_or_default().to_owned();
            let on = plan.as_ref().is_some_and(|plan| plan["id"] == id.as_str());
            let pick = id.clone();
            tabs = tabs.child(
                row()
                    .key(format!("plan-tab-{id}"))
                    .role("tab")
                    .checked(on)
                    .label(format!("Show plan {id}"))
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .h(22.0)
                    .px(6.0)
                    .center_y()
                    .radius(4.0)
                    .border(1.0, if on { p.frame } else { p.line })
                    .hover(hover_wash(s, p))
                    .on_press(move |s: &mut State| s.plan_select(&pick))
                    .child(label(&id, META, if on { p.ink } else { p.muted }).truncate()),
            );
        }
        panel = panel.child(tabs);
    }
    if view.is_none() {
        panel = panel.child(
            label("Reading the plan", META, p.muted)
                .px(4.0)
                .key("plan-loading"),
        );
    } else if nodes.is_empty() && s.plan_panel.editor.is_none() {
        panel = panel.child(
            label(
                "Lay out the crew's work as drafts. Nothing starts until you select ready drafts and start them.",
                META,
                p.muted,
            )
            .key("plan-empty")
            .wrap()
            .px(4.0)
            .w_full(),
        );
    }
    // One scrolling body under a fixed header and footer. An open editor or override step
    // takes the list's place, so the panel stays compact above the composer.
    let confirming = s
        .plan_panel
        .start
        .as_ref()
        .is_some_and(|start| start.confirming);
    let mut body = col().key("plan-body").gap(6.0);
    if s.plan_panel.editor.is_some() {
        body = body.child(editor(s, p, plan.as_ref()));
    } else if confirming {
        body = body.child(override_confirm(p, s));
    } else if !nodes.is_empty() {
        let mut list = col().key("plan-nodes").gap(2.0);
        for node in &nodes {
            list = list.child(node_row(s, p, node, busy));
        }
        body = body.child(list);
    }
    let body_max = if s.viewport_height > 0.0 {
        (s.viewport_height * 0.34).clamp(144.0, 360.0)
    } else {
        LIST_MAX
    };
    // Let the scrolling body shrink inside a short viewport while keeping the panel's
    // header and footer intact.
    let mut list_box = col()
        .key("plan-list")
        .shrink()
        .max_h(body_max.max(min_body))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(body);
    if editor_open {
        list_box = list_box.min_h(min_body);
    } else if !nodes.is_empty() {
        // Leave enough height for rows (and their edit actions) when the panel shares space
        // with Needs you, while still shrinking below body_max on very short viewports.
        list_box = list_box.min_h(60.0);
    }
    panel = panel.child(list_box);
    let conflict = s
        .plan_panel
        .editor
        .as_ref()
        .is_some_and(|editor| editor.conflict);
    // A revision conflict carries the server's words in the editor's own notice.
    if let Some(error) = s.plan_panel.error.as_ref().filter(|_| !conflict) {
        panel = panel.child(
            label(error, META, p.danger)
                .key("plan-error")
                .selectable(true)
                .wrap()
                .px(4.0)
                .w_full(),
        );
    } else if let Some(notice) = &s.plan_panel.notice {
        panel = panel.child(
            label(notice, META, p.muted)
                .key("plan-notice")
                .wrap()
                .px(4.0)
                .w_full(),
        );
    }
    let selected = nodes
        .iter()
        .filter(|node| {
            node["status"] != "launched"
                && node["key"]
                    .as_str()
                    .is_some_and(|key| s.plan_panel.selected.contains(key))
        })
        .count();
    let start_title = if selected == 0 {
        "Start selected".to_owned()
    } else {
        format!("Start {selected} selected")
    };
    if s.plan_panel.editor.is_some() {
        return Some(panel.child(editor_actions(s, p, busy).key("plan-footer")));
    }
    if confirming {
        return Some(panel.child(override_actions(s, p, busy).key("plan-footer")));
    }
    let can_start = selected > 0 && !busy;
    let mut footer = row()
        .key("plan-footer")
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(
            ghost_button(
                s,
                "plan-new-draft".into(),
                "New draft",
                p,
                |s: &mut State| s.plan_new_draft(false),
            )
            .disabled(busy),
        );
    if plan.is_some() {
        footer = footer.child(
            ghost_button(s, "plan-new-plan".into(), "New plan", p, |s: &mut State| {
                s.plan_new_draft(true)
            })
            .disabled(busy),
        );
    }
    footer = footer.child(spacer()).child(
        solid_button("plan-start".into(), &start_title, p, |s: &mut State| {
            s.plan_start_selected()
        })
        .disabled(!can_start)
        .opacity(if can_start { 1.0 } else { 0.5 }),
    );
    Some(panel.child(footer))
}
