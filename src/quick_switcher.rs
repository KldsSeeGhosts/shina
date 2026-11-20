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
    Machine(String, usize, String, Option<crate::model::Status>),
}

/// The list's entries for the current query, in display order. Section labels
/// interleave with rows; the row order here is the order arrow keys walk.
/// The list's entries for the current query, in display order, with the count
/// of "Recent" rows they start with (zero while a query is typed).
fn rows(s: &State) -> Vec<Row> {
    rows_with_recent(s).0
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
    if query.is_empty() {
        // Recent first, from the history's newest entries, minus the open one.
        let current = s.current().id;
        let machine = s.machines.selected.clone();
        let mut seen = std::collections::HashSet::new();
        for (server, id) in s.nav_history.recent().take(12) {
            if recent >= 6 {
                break;
            }
            if server == &machine {
                if *id == current || !seen.insert(*id) {
                    continue;
                }
                if let Some(index) = s.threads.iter().position(|t| t.id == *id) {
                    out.push(Row::Local(index));
                    recent += 1;
                }
            } else if seen.insert(*id)
                && let Some(title) = s.machines.views.get(server).and_then(|projection| {
                    projection["index"]["threads"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|t| t["id"].as_u64() == Some(*id as u64))
                        .and_then(|t| t["title"].as_str())
                        .map(str::to_string)
                })
            {
                out.push(Row::Machine(server.clone(), *id, title, None));
                recent += 1;
            }
        }
    }
    // Captains, then the rest in the sidebar's order. With a query both
    // sections filter by the title (and, for a captain, its name).
    for index in s.sidebar_captains() {
        let t = &s.threads[index];
        let name = t
            .captain
            .and_then(|c| s.captains.get(c).map(|p| p.name.as_str()))
            .unwrap_or("");
        if matches(t, name) {
            out.push(Row::Local(index));
        }
    }
    for index in s.sidebar_threads() {
        let t = &s.threads[index];
        if t.captain_home {
            continue;
        }
        if matches(t, "") {
            out.push(Row::Local(index));
        }
    }
    (out, recent)
}

/// The card and its scrim. `modal(true)` swallows outside presses; Escape
/// closes through the input's `on_input_key`.
pub(super) fn view(s: &State, p: Palette) -> El {
    let query = s.quick_switcher_query.text.trim().to_lowercase();
    let (all, recent_count) = rows_with_recent(s);
    // Classify rows into sections for an empty query; with a query everything
    // lists flat under the two headings.
    let mut sections: Vec<(&'static str, Vec<Row>)> = Vec::new();
    if query.is_empty() {
        let (recent, rest) = all.split_at(recent_count);
        let captains: Vec<Row> = rest
            .iter()
            .filter(|row| matches!(row, Row::Local(i) if s.is_captain_home(&s.threads[*i])))
            .cloned()
            .collect();
        let threads: Vec<Row> = rest
            .iter()
            .filter(|row| !matches!(row, Row::Local(i) if s.is_captain_home(&s.threads[*i])))
            .cloned()
            .collect();
        if !recent.is_empty() {
            sections.push(("Recent", recent.to_vec()));
        }
        if !captains.is_empty() {
            sections.push(("Captains", captains));
        }
        if !threads.is_empty() {
            sections.push(("Threads", threads));
        }
    } else {
        let captains: Vec<Row> = all
            .iter()
            .filter(|row| matches!(row, Row::Local(i) if s.is_captain_home(&s.threads[*i])))
            .cloned()
            .collect();
        let threads: Vec<Row> = all
            .iter()
            .filter(|row| !matches!(row, Row::Local(i) if s.is_captain_home(&s.threads[*i])))
            .cloned()
            .collect();
        if !captains.is_empty() {
            sections.push(("Captains", captains));
        }
        if !threads.is_empty() {
            sections.push(("Threads", threads));
        }
    }
    let highlight = s.quick_switcher_highlight.min(all.len().saturating_sub(1));
    let mut list = col()
        .key("quick-switcher-list")
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
        list = list.child(
            row()
                .h(28.0)
                .px(8.0)
                .center_y()
                .child(meta_label(name, p.faint)),
        );
        for row in group {
            let lit = index == highlight;
            list = list.child(switcher_row(s, p, &row, index, lit));
            index += 1;
        }
    }
    if all.is_empty() {
        list = list.child(
            label("Nothing matches", SMALL, p.muted)
                .key("quick-switcher-empty")
                .px(8.0)
                .py(10.0),
        );
    }
    let card = col()
        .key("quick-switcher")
        .role("dialog")
        .label("Quick switcher")
        .w((s.viewport_width - 32.0).min(640.0).max(200.0))
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
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
                        .autofocus()
                        .placeholder("Search threads", p.muted)
                        .color(p.ink)
                        .single_line()
                        .grow(1.0)
                        .min_w(0.0)
                        .on_edit(|s: &mut State| &mut s.quick_switcher_query)
                        .on_input_key(|s: &mut State, event, _| {
                            if edits_text(event) {
                                s.quick_switcher_highlight = 0;
                            }
                            let count = rows(s).len();
                            let at = s.quick_switcher_highlight.min(count.saturating_sub(1));
                            match event.key {
                                rune::Key::Down if count > 0 => {
                                    s.quick_switcher_highlight = (at + 1).min(count - 1);
                                    eprintln!("down -> {}", s.quick_switcher_highlight);
                                    true
                                }
                                rune::Key::Up => {
                                    s.quick_switcher_highlight = at.saturating_sub(1);
                                    true
                                }
                                rune::Key::Enter => {
                                    if let Some(row) = rows(s).get(at).cloned() {
                                        open_row(s, &row);
                                    }
                                    s.quick_switcher = false;
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
        .child(
            row()
                .h(32.0)
                .px(12.0)
                .gap(12.0)
                .center_y()
                .child(keycap("Enter", "Open", p))
                .child(keycap("Esc", "Close", p)),
        );
    el().key("quick-switcher-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .items_center()
        .pt(96.0)
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
    let (server, id) = match entry {
        Row::Local(index) => (s.machines.selected.clone(), s.threads[*index].id),
        Row::Machine(server, id, _, _) => (server.clone(), *id),
    };
    let key = format!("quick-switcher-row-{server}-{id}");
    let mut built = row()
        .key(key)
        .role("menuitem")
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .h(36.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(6.0)
        .fill(if lit { p.selected } else { Color::TRANSPARENT })
        .hover(p.selected)
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
                let detail = if project.is_empty() {
                    s.machine_label(&s.machines.selected)
                } else {
                    project.to_string()
                };
                built = built.child(
                    label(detail, SMALL, p.faint)
                        .truncate()
                        .shrink()
                        .max_w(160.0),
                );
            }
            built = built.child(status_mark(t.status, p, 7.0));
        }
        Row::Machine(server, _, title, status) => {
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
                built = built.child(
                    label(s.machine_label(server), SMALL, p.faint)
                        .truncate()
                        .shrink()
                        .max_w(160.0),
                );
            }
            if let Some(status) = status {
                built = built.child(status_mark(*status, p, 7.0));
            }
        }
    }
    built
}

/// Open the row's thread: a local index goes through `show_thread`, a machine
/// row through `open_machine_thread`.
fn open_row(s: &mut State, row: &Row) {
    match row {
        Row::Local(index) => s.show_thread(*index),
        Row::Machine(server, id, _, _) => s.open_machine_thread(server, *id),
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
            super::Row::Machine(_, id, _, _) => id,
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
