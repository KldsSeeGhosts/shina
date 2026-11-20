//! One ordered sidebar across linked machines, ported from KSG Code's SidebarThreadRow
//! and Sidebar.logic. Server/thread pairs, never bare thread IDs, identify remote rows.
use super::*;
use serde_json::Value;

pub(super) struct Entry<'a> {
    pub server: &'a str,
    pub id: usize,
    pub local: Option<usize>,
    pub captain: bool,
    pub draft: bool,
    pub life: lifecycle::Lifecycle,
    record: Option<&'a Value>,
    projection: Option<&'a crate::client_history::Projection>,
    view: &'a crate::client_history::ThreadProjection,
    children: Vec<(&'a Value, &'a crate::client_history::ThreadProjection)>,
    pub visible: bool,
    pub title_match: bool,
    pub search_hit: Option<crate::thread_search::ThreadSearchHit>,
}

/// One workspace group in the sidebar (ported from KSG Code's
/// `SidebarProjectFolderHeader`). Groups sort by most recent row, pinned first
/// inside each group, "Chats" (no workspace) last.
pub(super) struct Group<'a> {
    pub server: &'a str,
    pub project: Option<usize>,
    pub name: String,
    pub project_data: Option<crate::projects::Project>,
    pub rows: Vec<&'a Entry<'a>>,
}

impl<'a> Entry<'a> {
    pub(super) fn project(&self, s: &State) -> Option<usize> {
        if let Some(index) = self.local {
            s.threads[index].project
        } else {
            self.record
                .and_then(|r| r["project"].as_u64())
                .and_then(|id| usize::try_from(id).ok())
        }
    }

    pub(super) fn project_data(&self, s: &State) -> Option<crate::projects::Project> {
        let project = self.project(s)?;
        if self.local.is_some() {
            s.project(Some(project)).cloned()
        } else {
            self.projection
                .and_then(|p| p["index"]["projects"].as_array())
                .into_iter()
                .flatten()
                .find(|p| p["id"].as_u64() == Some(project as u64))
                .and_then(|p| crate::projects::Project::from_record(p, false))
        }
    }

    pub(super) fn project_name(&self, s: &State) -> String {
        self.project_data(s)
            .map(|p| p.name)
            .unwrap_or_else(|| "Chats".to_string())
    }

    pub(super) fn selected(&self, s: &State) -> bool {
        self.server == s.machines.selected
            && (self.id == s.selected_row() || s.selected_threads.contains(&self.id))
    }

    pub(super) fn needs_you(&self, s: &State) -> bool {
        if let Some(index) = self.local {
            let thread = &s.threads[index];
            let (status, approval) = row_status(s, thread);
            approval || thread.request.is_some() || status == Status::Waiting
        } else {
            !self.view.record["request"].is_null() || status_of_view(self.view) == Status::Waiting
        }
    }

    pub(super) fn working(&self, s: &State) -> bool {
        if let Some(index) = self.local {
            row_status(s, &s.threads[index]).0 == Status::Running
        } else {
            status_of_view(self.view) == Status::Running
        }
    }

    fn recent_rank(&self, s: &State) -> u64 {
        let updated = if let Some(index) = self.local {
            s.threads[index].updated
        } else {
            self.record.and_then(|r| r["updated"].as_u64()).unwrap_or(0)
        };
        let life_ts = self
            .life
            .latest_user_message_at
            .max(self.life.last_completed_at)
            .max(self.life.pinned_at)
            .unwrap_or(0);
        updated.max(life_ts)
    }
}

/// Group ordinary active/pinned sidebar rows by `(server, project)`.
pub(super) fn groups<'a>(s: &'a State, rows: &[&'a Entry<'a>]) -> Vec<Group<'a>> {
    let mut grouped: Vec<Group<'a>> = Vec::new();
    for &entry in rows {
        let project = entry.project(s);
        if let Some(group) = grouped
            .iter_mut()
            .find(|g| g.server == entry.server && g.project == project)
        {
            group.rows.push(entry);
        } else {
            grouped.push(Group {
                server: entry.server,
                project,
                name: entry.project_name(s),
                project_data: entry.project_data(s),
                rows: vec![entry],
            });
        }
    }
    // Sort groups: "Chats" (no workspace) last, otherwise most recent row first.
    // Rows inside each group already preserve pinned-first, lifecycle order.
    grouped.sort_by(|a, b| {
        a.project
            .is_none()
            .cmp(&b.project.is_none())
            .then_with(|| {
                let ra = a.rows.iter().map(|e| e.recent_rank(s)).max().unwrap_or(0);
                let rb = b.rows.iter().map(|e| e.recent_rank(s)).max().unwrap_or(0);
                rb.cmp(&ra)
            })
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.server.cmp(b.server))
            .then_with(|| a.project.cmp(&b.project))
    });
    grouped
}

/// Workspace group header, ported from KSG Code's `SidebarProjectFolderHeader`.
pub(super) fn group_header(s: &State, p: Palette, group: &Group<'_>, collapsed: bool) -> El {
    let project_key = group
        .project
        .map(|id| id.to_string())
        .unwrap_or_else(|| "none".into());
    let key = format!("sidebar-group-{}-{project_key}", group.server);
    let new_key = format!("new-thread-in-{}-{project_key}", group.server);
    let server = group.server.to_string();
    let project = group.project;
    let new_server = server.clone();
    let name = group.name.clone();
    let needs_you = group.rows.iter().any(|e| e.needs_you(s));
    let working = group.rows.iter().any(|e| e.working(s));
    let lead = if group.project.is_some() {
        project_tile(s, p, group.project_data.as_ref(), 16.0)
    } else {
        el().size(16.0, 16.0)
            .center()
            .a11y_hidden()
            .child(icon("thread", p.muted).size(14.0, 14.0))
    };
    let mut header = rune::row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!(
            "{}, {} threads, {}",
            name,
            group.rows.len(),
            if collapsed { "collapsed" } else { "expanded" }
        ))
        .focusable(true)
        .h(28.0)
        .mt(4.0)
        .pl(ROW_INSET)
        .pr(4.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .hover(p.selected)
        .hover_group()
        .on_press(move |s: &mut State| s.toggle_sidebar_group(server.clone(), project))
        .child(lead)
        .child(label(&name, META, p.muted).truncate().grow(1.0).min_w(0.0));
    if needs_you {
        header = header.child(el().size(6.0, 6.0).radius(3.0).fill(p.brass).a11y_hidden());
    } else if working {
        header = header.child(el().size(6.0, 6.0).radius(3.0).fill(p.sea).a11y_hidden());
    }
    if collapsed {
        header = header.child(label(
            group.rows.len().to_string(),
            META.features(TABULAR),
            p.faint,
        ));
    }
    let new_label = format!("New thread in {name}");
    header
        .child(
            icon_button(&new_key, &new_label, "plus", p, move |s| {
                if new_server != s.machines.selected && s.machine_connected(&new_server) {
                    s.select_machine_without_draft(&new_server);
                }
                if new_server == s.machines.selected {
                    s.new_thread_in(project);
                }
            })
            .group_opacity(0.0, 1.0)
            .help(new_label),
        )
        .child(
            icon(
                if collapsed {
                    "chevron-right"
                } else {
                    "chevron-down"
                },
                p.faint,
            )
            .size(12.0, 12.0),
        )
}

pub(super) fn entries(s: &State) -> Vec<Entry<'_>> {
    let mut entries = Vec::new();
    let searching = !s.search.text.is_empty();
    let visible = s.visible_sidebar_threads();
    let workers: std::collections::HashSet<usize> = s.searched_workers().into_iter().collect();
    let mut ordinary = s.sidebar_threads();
    ordinary.extend(workers.iter().copied());
    for (captain, indices) in [(true, s.sidebar_captains()), (false, ordinary)] {
        for index in indices {
            let thread = &s.threads[index];
            let title_match = !searching || s.thread_title_search_matches(thread);
            let search_hit = if searching && !title_match {
                s.sidebar_search
                    .visible_matches
                    .get(&(s.machines.selected.clone(), thread.id))
                    .cloned()
            } else {
                None
            };
            entries.push(Entry {
                server: &s.machines.selected,
                id: thread.id,
                local: Some(index),
                captain,
                draft: s.draft_listed(thread),
                life: thread.life.clone(),
                record: None,
                projection: None,
                view: null_view(),
                children: Vec::new(),
                visible: captain
                    || searching
                    || visible.contains(&index)
                    || workers.contains(&index),
                title_match,
                search_hit,
            });
        }
    }
    let query = s.search.text.to_lowercase();
    for (server, projection) in &s.machines.views {
        if server == &s.machines.selected || !s.machine_enabled(server) {
            continue;
        }
        let views: std::collections::HashMap<_, _> = projection
            .views
            .iter()
            .map(|(id, view)| (*id as usize, &**view))
            .collect();
        let metadata: std::collections::HashMap<_, _> = projection["index"]["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|record| record["id"].as_u64().map(|id| (id as usize, record)))
            .collect();
        let mut children: std::collections::HashMap<
            usize,
            Vec<(&Value, &crate::client_history::ThreadProjection)>,
        > = Default::default();
        for record in projection["index"]["threads"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let (Some(parent), Some(id)) = (record["parent"].as_u64(), record["id"].as_u64()) {
                children.entry(parent as usize).or_default().push((
                    record,
                    match views.get(&(id as usize)) {
                        Some(view) => *view,
                        None => null_view(),
                    },
                ));
            }
        }
        for record in projection["index"]["threads"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(id) = record["id"]
                .as_u64()
                .and_then(|id| usize::try_from(id).ok())
            else {
                continue;
            };
            let Ok(life) = lifecycle::Lifecycle::from_record(record) else {
                continue;
            };
            let captain = record["captainHome"] == true;
            let in_scope = s
                .scope_includes_on(server, record["project"].as_u64().map(|id| id as usize))
                || captain
                    && record["captain"].as_u64().is_some_and(|captain| {
                        projection["index"]["captainScopes"][captain.to_string()]
                            .as_array()
                            .is_some_and(|scope| {
                                scope
                                    .iter()
                                    .filter_map(Value::as_u64)
                                    .any(|id| s.scope_includes_on(server, Some(id as usize)))
                            })
                    });
            let standalone_child = record["parent"]
                .as_u64()
                .and_then(|parent| metadata.get(&(parent as usize)))
                .is_some_and(|parent| parent["captainHome"] != true);
            // Canonical crew membership decides, as `crew_captain` does locally: an
            // independently recruited thread is crew even without a parent link. Older
            // records without `crewCaptain` still mark a captain home's delegated child.
            let crew_member = record
                .get("crewCaptain")
                .is_some_and(|value| !value.is_null())
                || record["delegated"] == true
                    && !standalone_child
                    && record["parent"]
                        .as_u64()
                        .and_then(|parent| metadata.get(&(parent as usize)))
                        .is_some_and(|parent| parent["captainHome"] == true);
            // Crew members list flat during search and in the archive, like local ones.
            let crew_worker = crew_member && (!query.is_empty() || s.show_archive);
            // Workspace IDs are paired with their server, including filter selections.
            if life.archived_at.is_some() != s.show_archive
                || !in_scope
                || crew_member && !crew_worker
            {
                continue;
            }
            let view = match views.get(&id) {
                Some(view) => *view,
                None => null_view(),
            };
            let saved = &s.machines.ui[server]["drafts"][id.to_string()];
            let draft_text = saved
                .as_str()
                .or_else(|| saved["text"].as_str())
                .unwrap_or("");
            let name = record["captain"]
                .as_u64()
                .and_then(|index| projection["index"]["captains"].get(index as usize))
                .and_then(|captain| captain["name"].as_str())
                .unwrap_or("");
            let title = record["title"].as_str().unwrap_or("Thread");
            let server_supports = s
                .client
                .as_ref()
                .and_then(|c| c.linked.get(server))
                .is_some_and(|c| c.has_capability(crate::protocol::THREAD_SEARCH));
            let title_match = query.is_empty()
                || title.to_lowercase().contains(&query)
                || name.to_lowercase().contains(&query)
                || draft_text.to_lowercase().contains(&query)
                || (!server_supports
                    && view.fields.get("messages").is_some_and(|messages| {
                        messages.items.iter().any(|message| {
                            message["text"]
                                .as_str()
                                .unwrap_or("")
                                .to_lowercase()
                                .contains(&query)
                        })
                    }));
            let search_hit = if !query.is_empty() && !title_match {
                s.sidebar_search
                    .visible_matches
                    .get(&(server.to_string(), id))
                    .cloned()
            } else {
                None
            };
            if !query.is_empty() && !title_match && search_hit.is_none() {
                continue;
            }
            let draft = !captain
                && (record["session"].is_null() || record["neverSent"] == true)
                && life.pinned_at.is_none()
                && !life.settled()
                && life.snoozed_at.is_none()
                && life.failed_at.is_none()
                && life.latest_user_message_at.is_none()
                && life.last_completed_at.is_none()
                && view
                    .fields
                    .get("messages")
                    .is_none_or(|messages| messages.items.is_empty())
                && status_of_view(view) == Status::Ready;
            // Match local draft-listing rules. Blank remote landing slots are not chats.
            if draft
                && draft_text.trim().is_empty()
                && !saved["richHtml"]
                    .as_str()
                    .is_some_and(|html| html.contains("data-pill"))
                && record["renamed"] != true
                && saved["images"].as_array().is_none_or(Vec::is_empty)
            {
                continue;
            }
            let visible = if !query.is_empty() {
                true
            } else {
                match lifecycle::section(&life, lifecycle::now()) {
                    Section::Snoozed => s.shelves[0],
                    Section::Settled => s.shelves[1],
                    _ => true,
                }
            };
            entries.push(Entry {
                server,
                id,
                local: None,
                captain,
                draft,
                life,
                record: Some(record),
                projection: Some(projection),
                view,
                children: children.remove(&id).unwrap_or_default(),
                visible,
                title_match,
                search_hit,
            });
        }
    }
    let now = lifecycle::now();
    let order_pos = |server: &str, id: usize| -> usize {
        s.sidebar_search
            .shown_order
            .iter()
            .position(|(srv, tid)| srv == server && *tid == id)
            .unwrap_or(usize::MAX)
    };
    entries.sort_by(|a, b| {
        b.captain.cmp(&a.captain).then_with(|| {
            if a.captain && b.captain {
                // Selecting a captain must not move the other machines' captains.
                a.server.cmp(b.server).then(a.id.cmp(&b.id))
            } else if searching {
                b.title_match.cmp(&a.title_match).then_with(|| {
                    if !a.title_match && !b.title_match {
                        order_pos(a.server, a.id)
                            .cmp(&order_pos(b.server, b.id))
                            .then(a.server.cmp(b.server))
                            .then(a.id.cmp(&b.id))
                    } else {
                        b.draft
                            .cmp(&a.draft)
                            .then(
                                lifecycle::section(&a.life, now)
                                    .cmp(&lifecycle::section(&b.life, now)),
                            )
                            .then_with(|| {
                                lifecycle::compare(
                                    lifecycle::section(&a.life, now),
                                    &a.life,
                                    &b.life,
                                )
                            })
                            .then(a.server.cmp(b.server))
                            .then(a.id.cmp(&b.id))
                    }
                })
            } else {
                b.draft
                    .cmp(&a.draft)
                    .then(lifecycle::section(&a.life, now).cmp(&lifecycle::section(&b.life, now)))
                    .then_with(|| {
                        lifecycle::compare(lifecycle::section(&a.life, now), &a.life, &b.life)
                    })
                    .then(a.server.cmp(b.server))
                    .then(a.id.cmp(&b.id))
            }
        })
    });
    entries
}

fn status_of_view(view: &crate::client_history::ThreadProjection) -> Status {
    serde_json::from_value(view.record["status"].clone()).unwrap_or(Status::Ready)
}

fn null_view() -> &'static crate::client_history::ThreadProjection {
    static NULL: std::sync::OnceLock<crate::client_history::ThreadProjection> =
        std::sync::OnceLock::new();
    NULL.get_or_init(|| crate::client_history::ThreadProjection::empty(&serde_json::json!({})))
}

pub(super) fn row(s: &State, p: Palette, entry: &Entry<'_>) -> El {
    if let Some(index) = entry.local {
        return thread_row(s, index, p);
    }
    let record = entry.record.expect("remote record");
    let projection = entry.projection.expect("remote projection");
    let server = entry.server.to_string();
    let id = entry.id;
    let title = record["title"].as_str().unwrap_or("Thread");
    let view = entry.view;
    let mut status = status_of_view(view);
    let mut requests = usize::from(!view.record["request"].is_null());
    let mut unread = usize::from(entry.life.unread());
    if entry.captain {
        for (child, child_view) in &entry.children {
            let Ok(life) = lifecycle::Lifecycle::from_record(child) else {
                continue;
            };
            if life.archived_at.is_some() != s.show_archive {
                continue;
            }
            let child_status = status_of_view(child_view);
            if child_status.urgency() > status.urgency() {
                status = child_status;
            }
            requests += usize::from(!child_view.record["request"].is_null());
            unread += usize::from(life.unread());
        }
    }
    if requests > 0 {
        status = Status::Waiting;
    }
    let project = projection["index"]["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|project| !record["project"].is_null() && project["id"] == record["project"]);
    let workspace = project
        .and_then(|project| project["name"].as_str())
        .unwrap_or("No workspace");
    let machine_line = machine_caption(s, &server);
    let project_tile_data =
        project.and_then(|project| crate::projects::Project::from_record(project, false));
    let has_hit = entry.search_hit.is_some();
    let press_server = server.clone();
    let item = rune::row()
        .key(format!("machine-thread-{server}-{id}"))
        .role("button")
        .label(format!("{title}, {machine_line}, {}", status.label()))
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .w_full()
        .min_w(0.0)
        .radius(6.0)
        .hover(p.selected)
        .pointer_buttons([rune::PointerButton::Middle])
        .on_press_tx(move |s: &mut State, tx| {
            // KSG: a middle click opens the thread as a kept tab.
            if tx.button() == rune::PointerButton::Middle {
                s.open_machine_thread_kept(&press_server, id);
            } else if has_hit {
                s.open_thread_at_search_match(&press_server, id);
            } else {
                s.open_machine_thread(&press_server, id);
            }
        });
    if entry.captain {
        let captain = record["captain"]
            .as_u64()
            .and_then(|index| projection["index"]["captains"].get(index as usize));
        let name = captain
            .and_then(|captain| captain["name"].as_str())
            .unwrap_or(title);
        let portrait = captain
            .and_then(|captain| captain["portrait"].as_u64())
            .unwrap_or(0) as usize;
        let pixels = s
            .artwork
            .face_pixels(portrait, face_for(status), CAPTAIN_FACE);
        let crew_count = entry
            .children
            .iter()
            .filter(|(record, _)| {
                lifecycle::Lifecycle::from_record(record)
                    .is_ok_and(|life| life.archived_at.is_some() == s.show_archive)
            })
            .count();
        let detail_text = if !workspace_named_by_filter(
            s,
            entry.server,
            record["project"].as_u64().map(|id| id as usize),
        ) {
            workspace.to_string()
        } else if crew_count > 0 {
            format!("{crew_count} crew")
        } else {
            String::new()
        };
        let timer = std::iter::once(view)
            .chain(entry.children.iter().map(|(_, view)| *view))
            .filter(|view| status_of_view(view) == Status::Running)
            .filter_map(|view| view.record["elapsed"].as_u64())
            .max();
        let mut trailing = rune::row()
            .key(format!("machine-status-{}-{id}", entry.server))
            .h(20.0)
            .gap(6.0)
            .center_y();
        if requests > 0 || status == Status::Waiting {
            trailing = trailing.child(count_pill(requests.max(1), CountKind::NeedsYou, p));
        } else if status == Status::Running {
            trailing = trailing.child(el().size(6.0, 6.0).radius(3.0).fill(p.sea).a11y_hidden());
            if let Some(seconds) = timer {
                trailing = trailing.child(label(
                    elapsed_label(seconds),
                    META.features(TABULAR),
                    p.muted,
                ));
            }
        } else if unread > 0 {
            trailing = trailing.child(el().size(6.0, 6.0).radius(3.0).fill(p.sea).a11y_hidden());
        } else if status == Status::Failed {
            trailing = trailing
                .child(el().size(6.0, 6.0).radius(3.0).fill(p.danger).a11y_hidden())
                .child(meta_text("Failed", p.danger));
        }
        let portrait_el = pixel_portrait(s, &pixels, CAPTAIN_FACE, p)
            .key(format!("machine-portrait-{}-{id}", entry.server))
            .a11y_hidden();
        let portrait_el = if requests > 0 || status == Status::Waiting {
            status_badge(p, portrait_el, CAPTAIN_FACE, Status::Waiting)
        } else {
            portrait_el
        };
        let show_detail = sidebar_row_width(s) >= 224.0 && !detail_text.is_empty();
        let show_machine_name = sidebar_row_width(s) >= 232.0
            && room_for_machine_name(
                sidebar_row_width(s) - CAPTAIN_TEXT_INSET,
                ICON + 48.0,
                &detail_text,
            );
        let mut description = format!("{name}, {}", status.label());
        if !detail_text.is_empty() {
            description.push_str(&format!(", {detail_text}"));
        }
        description.push_str(&format!(", {machine_line}"));
        let mut row_el = rune::row()
            .h(40.0)
            .w_full()
            .pl(8.0)
            .pr(8.0)
            .gap(10.0)
            .center_y()
            .child(
                el().size(CAPTAIN_SLOT, CAPTAIN_SLOT)
                    .center()
                    .child(portrait_el),
            )
            .child(
                label(name, CAPTAIN_NAME, p.ink)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        if show_detail {
            row_el = row_el.child(
                label(detail_text, CAPTAIN_STATUS, p.faint)
                    .key(format!("machine-detail-{}-{id}", entry.server))
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        }
        row_el = row_el
            .child(machine_tag(
                s,
                p,
                entry.server,
                format!("machine-tag-{}-{id}", entry.server),
                CAPTAIN_STATUS,
                show_machine_name,
            ))
            .child(spacer())
            .child(trailing);
        return item
            .label(description.clone())
            .help(description)
            .radius(6.0)
            .child(row_el);
    }
    let shelved = matches!(
        lifecycle::section(&entry.life, lifecycle::now()),
        Section::Settled | Section::Snoozed
    );
    let at = lifecycle::now();
    let grouped = s.sidebar_grouped() && !shelved;
    let approval = view.record["request"]["method"] == "confirm";
    let state_label = if approval {
        "Approval"
    } else if requests > 0 {
        "Input"
    } else if status == Status::Running {
        "Working"
    } else if status == Status::Waiting {
        "Waiting"
    } else if status == Status::Failed {
        "Failed"
    } else if entry.life.woke(at) {
        "Woke"
    } else if entry.life.unread() {
        "Done"
    } else {
        ""
    };
    let age = if state_label == "Working" {
        view.record["elapsed"]
            .as_u64()
            .map(elapsed_label)
            .or_else(|| Some("Working".into()))
    } else if matches!(state_label, "Approval" | "Input" | "Waiting" | "Failed") {
        Some(state_label.into())
    } else if shelved {
        if lifecycle::section(&entry.life, at) == Section::Snoozed {
            Some(lifecycle::wake_label(
                entry.life.snoozed_until.unwrap_or(0),
                at,
            ))
        } else {
            relative_time(entry.life.settled_at.unwrap_or(0) / 1000)
        }
    } else {
        relative_time(record["updated"].as_u64().unwrap_or(0))
    };
    let delegated_to_home = record["delegated"] == true
        && record["parent"]
            .as_u64()
            .and_then(|parent| {
                projection["index"]["threads"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|record| record["id"].as_u64() == Some(parent))
            })
            .is_some_and(|parent| parent["captainHome"] == true);
    let worker_captain = (!entry.captain
        && (record
            .get("crewCaptain")
            .is_some_and(|value| !value.is_null())
            || delegated_to_home))
        .then(|| {
            record
                .get("crewCaptain")
                .and_then(|value| value.as_u64())
                .or_else(|| record["captain"].as_u64())
                .and_then(|index| projection["index"]["captains"].get(index as usize))
        })
        .flatten();
    let dot = |ink: Color| el().size(6.0, 6.0).radius(3.0).fill(ink).a11y_hidden();
    let lead_content = if grouped {
        if approval || requests > 0 || status == Status::Waiting {
            Some(status_mark(Status::Waiting, p, 7.0).a11y_hidden())
        } else if status == Status::Running || status == Status::Failed {
            Some(status_mark(status, p, 7.0).a11y_hidden())
        } else if entry.life.unread() || entry.life.woke(at) {
            Some(dot(p.sea))
        } else if entry.life.pinned_at.is_some() {
            Some(icon("pin", p.faint).size(12.0, 12.0).a11y_hidden())
        } else if entry.draft {
            Some(icon("edit", p.faint).size(12.0, 12.0).a11y_hidden())
        } else {
            None
        }
    } else {
        let tile = match worker_captain {
            Some(captain) => {
                let portrait = captain["portrait"].as_u64().unwrap_or(0) as usize;
                let pixels = s.artwork.face_pixels(portrait, face_for(status), 16.0);
                pixel_portrait(s, &pixels, 16.0, p)
                    .key(format!("machine-captain-portrait-{}-{id}", entry.server))
                    .a11y_hidden()
            }
            None => project_tile(s, p, project_tile_data.as_ref(), 16.0),
        };
        Some(status_badge(p, tile, 16.0, status))
    };
    let lead = el()
        .size(16.0, 16.0)
        .center()
        .child(lead_content.unwrap_or_else(el));
    let saved = &s.machines.ui[entry.server]["drafts"][id.to_string()];
    let draft = saved
        .as_str()
        .or_else(|| saved["text"].as_str())
        .unwrap_or("");
    let highlighted = status == Status::Running
        || status == Status::Waiting
        || requests > 0
        || entry.life.woke(at)
        || entry.life.unread();
    let (title_style, title_ink) = if shelved {
        (ROW_QUIET, p.faint)
    } else if highlighted {
        (TITLE, p.ink)
    } else {
        (ROW_QUIET, p.muted)
    };
    let title_el = label(
        if entry.draft && !draft.trim().is_empty() {
            draft.lines().next().unwrap_or(title)
        } else {
            title
        },
        title_style,
        title_ink,
    )
    .truncate()
    .grow(1.0)
    .min_w(0.0);
    let mut slot = rune::row().h(20.0).gap(4.0).center_y();
    if let Some(age) = age {
        let ink = if approval || status == Status::Waiting || requests > 0 {
            p.brass
        } else if status == Status::Failed {
            p.danger
        } else if state_label == "Working" {
            p.sea
        } else {
            p.faint
        };
        slot = slot.child(label(age, META.features(TABULAR), ink));
    }
    let main_line = rune::row()
        .h(32.0)
        .w_full()
        .pl(ROW_INSET)
        .pr(8.0)
        .gap(8.0)
        .center_y()
        .child(lead)
        .child(title_el)
        .child(slot);
    if let Some(hit) = &entry.search_hit {
        item.help(format!("{workspace}, {machine_line}")).child(
            col().w_full().min_w(0.0).pb(6.0).child(main_line).child(
                el().pl(ROW_INSET + 24.0).pr(8.0).child(search_snippet_el(
                    &server,
                    id,
                    &hit.snippet,
                    &s.search.text,
                    p.muted,
                    p.brass,
                )),
            ),
        )
    } else {
        item.help(format!("{workspace}, {machine_line}"))
            .child(main_line)
    }
}
