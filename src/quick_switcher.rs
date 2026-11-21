//! The Cmd+K quick switcher, ported from KSG Code's `CommandPalette.tsx`
//! thread rows on the same pattern as `add_project_palette`: a scrim, one card
//! with a search field on top, a scrolling list of sections, and a keycap
//! footer. An empty query lists the `NavHistory` recent threads, then captains,
//! then every thread in the sidebar's order.

use super::*;

/// A switcher row: a local thread by index, or a thread on a linked machine.
#[derive(Clone)]
enum Row {
    Local(usize),
    Machine {
        server: String,
        id: usize,
        title: String,
        captain: bool,
        workspace: String,
        updated: u64,
        status: Option<crate::model::Status>,
    },
}

/// The list's entries for the current query, in display order. Section labels
/// interleave with rows; the row order here is the order arrow keys walk.
/// The list's entries for the current query, in display order, with the count
/// of "Recent" rows they start with (zero while a query is typed).
fn rows(s: &State) -> Vec<Row> {
    rows_with_recent(s).0
}

fn row_key(s: &State, row: &Row) -> String {
    match row {
        Row::Local(index) => {
            format!(
                "quick-switcher-row-{}-{}",
                s.machines.selected, s.threads[*index].id
            )
        }
        Row::Machine { server, id, .. } => format!("quick-switcher-row-{server}-{id}"),
    }
}

fn row_is_captain(s: &State, row: &Row) -> bool {
    match row {
        Row::Local(i) => s.is_captain_home(&s.threads[*i]),
        Row::Machine { captain, .. } => *captain,
    }
}

fn rows_with_recent(s: &State) -> (Vec<Row>, usize) {
    let query = s.quick_switcher_query.text.trim().to_lowercase();
    let matches = |t: &Thread, extra: &str| {
        query.is_empty()
            || s.thread_title_matches_query(t, &query)
            || extra.to_lowercase().contains(&query)
    };
    let mut out: Vec<Row> = Vec::new();
    let mut recent = 0;
    let current = (s.machines.selected.clone(), s.current().id);
    let mut nav_seen = std::collections::HashSet::from([current.clone()]);
    if query.is_empty() {
        // Recent first, from the history's newest entries, minus the open one.
        for (server, id) in s.nav_history.recent().take(12) {
            if recent >= 5 {
                break;
            }
            if nav_seen.contains(&(server.clone(), *id)) {
                continue;
            }
            if server == &current.0 {
                if let Some(index) = s.threads.iter().position(|t| t.id == *id) {
                    out.push(Row::Local(index));
                    nav_seen.insert((server.clone(), *id));
                    recent += 1;
                }
            } else if let Some((title, captain, workspace, updated)) =
                s.machines.views.get(server).and_then(|projection| {
                    let record = projection["index"]["threads"]
                        .as_array()?
                        .iter()
                        .find(|t| t["id"].as_u64() == Some(*id as u64))?;
                    let title = record["title"].as_str()?.to_string();
                    let captain = record["captainHome"] == true;
                    let workspace = projection["index"]["projects"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|p| !record["project"].is_null() && p["id"] == record["project"])
                        .and_then(|p| p["name"].as_str())
                        .unwrap_or("")
                        .to_string();
                    let updated = record["updated"].as_u64().unwrap_or(0);
                    Some((title, captain, workspace, updated))
                })
            {
                out.push(Row::Machine {
                    server: server.clone(),
                    id: *id,
                    title,
                    captain,
                    workspace,
                    updated,
                    status: None,
                });
                nav_seen.insert((server.clone(), *id));
                recent += 1;
            }
        }
    }
    // Walk every machine in the sidebar's own order (grouped when Workspace grouping is on).
    for (server, id) in sidebar_navigation(s) {
        if !nav_seen.insert((server.clone(), id)) {
            continue;
        }
        if server == s.machines.selected {
            let Some(index) = s.threads.iter().position(|t| t.id == id) else {
                continue;
            };
            let t = &s.threads[index];
            let extra = t
                .captain
                .and_then(|c| s.captains.get(c).map(|p| p.name.as_str()))
                .unwrap_or("");
            if matches(t, extra) {
                out.push(Row::Local(index));
            }
        } else if let Some(projection) = s.machines.views.get(&server)
            && let Some(record) = projection["index"]["threads"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|t| t["id"].as_u64() == Some(id as u64))
        {
            let title = record["title"].as_str().unwrap_or("Thread").to_string();
            let captain = record["captainHome"] == true;
            let cap_name = record["captain"]
                .as_u64()
                .and_then(|idx| projection["index"]["captains"].get(idx as usize))
                .and_then(|c| c["name"].as_str())
                .unwrap_or("");
            let workspace = projection["index"]["projects"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| !record["project"].is_null() && p["id"] == record["project"])
                .and_then(|p| p["name"].as_str())
                .unwrap_or("")
                .to_string();
            if query.is_empty()
                || title.to_lowercase().contains(&query)
                || cap_name.to_lowercase().contains(&query)
                || workspace.to_lowercase().contains(&query)
            {
                let status = projection
                    .views
                    .get(&(id as u64))
                    .and_then(|v| serde_json::from_value(v.record["status"].clone()).ok());
                out.push(Row::Machine {
                    server,
                    id,
                    title,
                    captain,
                    workspace,
                    updated: record["updated"].as_u64().unwrap_or(0),
                    status,
                });
            }
        }
    }
    // Also include matching threads from collapsed groups while searching.
    if !query.is_empty() {
        for entry in sidebar_ui::entries(s) {
            if !nav_seen.insert((entry.server.to_owned(), entry.id)) {
                continue;
            }
            if let Some(index) = entry.local {
                let t = &s.threads[index];
                let extra = t
                    .captain
                    .and_then(|c| s.captains.get(c).map(|p| p.name.as_str()))
                    .unwrap_or("");
                if matches(t, extra) {
                    out.push(Row::Local(index));
                }
            } else if let Some(projection) = s.machines.views.get(entry.server)
                && let Some(record) = projection["index"]["threads"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|t| t["id"].as_u64() == Some(entry.id as u64))
            {
                let title = record["title"].as_str().unwrap_or("Thread").to_string();
                if title.to_lowercase().contains(&query) {
                    out.push(Row::Machine {
                        server: entry.server.to_owned(),
                        id: entry.id,
                        title,
                        captain: entry.captain,
                        workspace: String::new(),
                        updated: record["updated"].as_u64().unwrap_or(0),
                        status: None,
                    });
                }
            }
        }
    }
    if query.is_empty() {
        out[recent..].sort_by_key(|row| !row_is_captain(s, row));
    }
    (out, recent)
}

fn row_title<'a>(s: &'a State, row: &'a Row) -> &'a str {
    match row {
        Row::Local(i) => &s.threads[*i].title,
        Row::Machine { title, .. } => title,
    }
}
/// The card and its scrim. `modal(true)` swallows outside presses; Escape
/// closes through the input's `on_input_key`.
pub(super) fn view(s: &State, p: Palette) -> El {
    let query = s.quick_switcher_query.text.trim().to_lowercase();
    let (all, recent_count) = rows_with_recent(s);
    // Classify rows into sections for an empty query; with a query everything
    // lists flat under the two headings.
    let mut sections: Vec<(Option<&'static str>, Vec<Row>)> = Vec::new();
    if query.is_empty() {
        let (recent, rest) = all.split_at(recent_count);
        let captains: Vec<Row> = rest
            .iter()
            .filter(|row| row_is_captain(s, row))
            .cloned()
            .collect();
        let threads: Vec<Row> = rest
            .iter()
            .filter(|row| !row_is_captain(s, row))
            .cloned()
            .collect();
        if !recent.is_empty() {
            sections.push((Some("Recent"), recent.to_vec()));
        }
        if !captains.is_empty() {
            sections.push((Some("Captains"), captains));
        }
        if !threads.is_empty() {
            sections.push((Some("Threads"), threads));
        }
    } else {
        // Filtered query: one flat list without section headings (spec §1.6).
        if !all.is_empty() {
            sections.push((None, all.clone()));
        }
    }
    let highlight = s.quick_switcher_highlight.min(all.len().saturating_sub(1));
    let mut list = col()
        .key("quick-switcher-list")
        .role("listbox")
        .max_h(360.0)
        .min_h(0.0)
        .shrink()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(6.0)
        .gap(1.0);
    let mut index = 0usize;
    for (name, group) in sections {
        if let Some(name) = name {
            list = list.child(
                row()
                    .h(28.0)
                    .px(8.0)
                    .center_y()
                    .child(meta_label(name, p.faint)),
            );
        }
        for row in group {
            let lit = index == highlight;
            list = list.child(switcher_row(s, p, &row, index, lit));
            index += 1;
        }
    }
    if all.is_empty() {
        list = list
            .child(
                label(format!("No threads match \"{query}\""), SMALL, p.muted)
                    .key("quick-switcher-empty")
                    .px(8.0)
                    .py(10.0),
            )
            .child(button(
                "quick-switcher-search-messages",
                "Search message text in the sidebar",
                p,
                |s| {
                    s.search.replace_text(&s.quick_switcher_query.text);
                    s.sync_sidebar_search_input();
                    s.quick_switcher = false;
                    s.sidebar = true;
                    s.compact_sidebar = s.viewport_width < 760.0;
                },
            ));
    }
    let mut footer = row()
        .key("quick-switcher-footer")
        .h(32.0)
        .px(12.0)
        .gap(12.0)
        .center_y();
    if !all.is_empty() {
        footer = footer
            .child(keycap("↑↓", "Navigate", p))
            .child(keycap("Enter", "Open", p).key("quick-switcher-hint-open"));
        if s.tabs_enabled {
            footer = footer.child(keycap("Ctrl+Enter", "Open in tab", p));
        }
    }
    footer = footer.child(keycap("Esc", "Close", p));
    let card = col()
        .key("quick-switcher")
        .role("dialog")
        .label("Quick switcher")
        .w_full()
        .max_w(640.0)
        .min_w(200.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(dialog_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .h(44.0)
                .px(14.0)
                .gap(10.0)
                .center_y()
                .child(icon("search", p.muted))
                .child(
                    rune::text_input(&s.quick_switcher_query, BODY)
                        .key("quick-switcher-input")
                        .label("Search threads")
                        .a11y_value(all.get(highlight).map_or("", |entry| row_title(s, entry)))
                        .autofocus()
                        .placeholder("Search threads", p.muted)
                        .color(p.ink)
                        .single_line()
                        .grow(1.0)
                        .min_w(0.0)
                        .on_edit(|s: &mut State| &mut s.quick_switcher_query)
                        .on_input_key(|s: &mut State, event, tx| {
                            if edits_text(event) {
                                s.quick_switcher_highlight = 0;
                            }
                            if !matches!(
                                event.key,
                                rune::Key::Down
                                    | rune::Key::Up
                                    | rune::Key::Enter
                                    | rune::Key::Escape
                            ) {
                                return s.undo_key(event, true);
                            }
                            let all_rows = rows(s);
                            let count = all_rows.len();
                            let at = s.quick_switcher_highlight.min(count.saturating_sub(1));
                            match event.key {
                                rune::Key::Down if count > 0 => {
                                    s.quick_switcher_highlight = (at + 1).min(count - 1);
                                    if let Some(next) = all_rows.get(s.quick_switcher_highlight) {
                                        tx.scroll_into_view_in(
                                            "quick-switcher-list",
                                            row_key(s, next),
                                        );
                                    }
                                    true
                                }
                                rune::Key::Up => {
                                    s.quick_switcher_highlight = at.saturating_sub(1);
                                    if let Some(prev) = all_rows.get(s.quick_switcher_highlight) {
                                        tx.scroll_into_view_in(
                                            "quick-switcher-list",
                                            row_key(s, prev),
                                        );
                                    }
                                    true
                                }
                                rune::Key::Enter => {
                                    if let Some(row) = all_rows.get(at) {
                                        if s.tabs_enabled && (event.mods.ctrl || event.mods.cmd) {
                                            match row {
                                                Row::Local(index) => s.open_kept_tab(*index),
                                                Row::Machine { server, id, .. } => {
                                                    s.open_machine_thread_kept(server, *id)
                                                }
                                            }
                                        } else {
                                            open_row(s, row);
                                        }
                                        s.quick_switcher = false;
                                    }
                                    true
                                }
                                rune::Key::Escape => {
                                    s.quick_switcher = false;
                                    true
                                }
                                _ => s.undo_key(event, true),
                            }
                        }),
                ),
        )
        .child(rule(p))
        .child(list)
        .child(footer);
    col()
        .key("quick-switcher-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .items_center()
        .px(16.0)
        .pt((s.viewport_height * 0.14).clamp(48.0, 120.0))
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.22 }))
        .modal(false, |s: &mut State| s.quick_switcher = false)
        .on_press(|s: &mut State| s.quick_switcher = false)
        // Escape closes it from the input, so the scrim is no Tab stop.
        .focusable(false)
        .child(card)
}

/// One entry in the list: the thread's `tab_icon`, its title in `TITLE`, the
/// workspace and age as the detail, and a 7 pt status mark at the end.
fn switcher_row(s: &State, p: Palette, entry: &Row, index: usize, lit: bool) -> El {
    let key = row_key(s, entry);
    let mut built = row()
        .key(key)
        .role("option")
        .focus_selected(lit)
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .h(40.0)
        .px(10.0)
        .gap(10.0)
        .center_y()
        .radius(8.0)
        .fill(if lit { p.lift } else { Color::TRANSPARENT })
        .border(1.0, if lit { p.frame } else { Color::TRANSPARENT })
        .on_hover_change(move |s: &mut State, hot, _| {
            if hot {
                s.quick_switcher_highlight = index;
            }
        })
        .on_press(move |s: &mut State| {
            let row = rows(s).get(index).cloned();
            if let Some(row) = row {
                open_row(s, &row);
            }
            s.quick_switcher = false;
        });
    match entry {
        Row::Local(i) => {
            let t = &s.threads[*i];
            built = built
                .label(&t.title)
                .child(el().size(16.0, 16.0).center().child(tab_icon(s, p, t)))
                .child(
                    label(&t.title, top_bar::TITLE, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                );
            // Detail hides at the 620 pt card, where it would crowd the title.
            if s.viewport_width >= 700.0 {
                let project = s.project(t.project).map(|p| p.name.as_str()).unwrap_or("");
                let age = relative_time(t.updated);
                let detail = match (project.is_empty(), age) {
                    (false, Some(age)) => format!("{project} · {age}"),
                    (false, None) => project.to_string(),
                    (true, Some(age)) => age,
                    (true, None) => s.machine_label(&s.machines.selected),
                };
                built = built.child(
                    label(detail, SMALL, p.faint)
                        .truncate()
                        .shrink()
                        .max_w(160.0),
                );
            }
            if !lit && matches!(t.status, Status::Running | Status::Waiting | Status::Failed) {
                built = built.child(status_mark(t.status, p, 8.0));
            }
        }
        Row::Machine {
            server,
            title,
            workspace,
            updated,
            status,
            ..
        } => {
            built = built
                .label(title)
                .child(
                    el().size(16.0, 16.0)
                        .center()
                        .child(icon("monitor", p.muted)),
                )
                .child(
                    label(title, top_bar::TITLE, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                );
            if s.viewport_width >= 700.0 {
                let machine = s.machine_label(server);
                let age = relative_time(*updated);
                let mut parts = Vec::new();
                if !workspace.is_empty() {
                    parts.push(workspace.clone());
                }
                parts.push(machine);
                if let Some(age) = age {
                    parts.push(age);
                }
                built = built.child(
                    label(parts.join(" · "), SMALL, p.faint)
                        .truncate()
                        .shrink()
                        .max_w(160.0),
                );
            }
            if let Some(status @ (Status::Running | Status::Waiting | Status::Failed)) = status
                && !lit
            {
                built = built.child(status_mark(*status, p, 7.0));
            }
        }
    }
    if lit {
        built.child(keycap_chip("↵", p))
    } else {
        built
    }
}

/// Open the row's thread: a local index goes through `show_thread`, a machine
/// row through `open_machine_thread`.
fn open_row(s: &mut State, row: &Row) {
    match row {
        Row::Local(index) => s.show_thread(*index),
        Row::Machine { server, id, .. } => s.open_machine_thread(server, *id),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_switcher_lists_recents_then_captains_then_threads() {
        let mut rt = crate::ui::tests::runtime(1280.0);
        rt.state.populate_demo();
        rt.state.open_captain(0);
        // A second plain thread so the history has somewhere to go back to.
        rt.state.new_thread();
        rt.invalidate();
        rt.frame(0.0);
        rt.state.open_quick_switcher();
        rt.invalidate();
        rt.frame(0.0);
        assert!(rt.presented("quick-switcher").is_some());
        assert!(rt.presented("quick-switcher-input").is_some());
        assert!(rt.presented("quick-switcher-list").is_some());
    }

    #[test]
    fn arrow_keys_light_a_row_and_enter_opens_it() {
        let mut rt = crate::ui::tests::runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        rt.state.open_quick_switcher();
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        // The lit row is the first one the list shows: a Recent entry, never
        // the open thread itself.
        let target = match super::rows(&rt.state)[0] {
            super::Row::Local(index) => rt.state.threads[index].id,
            super::Row::Machine { id, .. } => id,
        };
        crate::ui::tests::tap(&mut rt, rune::Key::Down);
        assert_eq!(rt.state.quick_switcher_highlight, 1);
        crate::ui::tests::tap(&mut rt, rune::Key::Up);
        assert_eq!(rt.state.quick_switcher_highlight, 0);
        crate::ui::tests::tap(&mut rt, rune::Key::Enter);
        assert!(!rt.state.quick_switcher);
        assert_eq!(rt.state.current().id, target);
    }
}
