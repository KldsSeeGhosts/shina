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
    projection: Option<&'a Value>,
    view: &'a Value,
    children: Vec<(&'a Value, &'a Value)>,
    pub visible: bool,
    pub title_match: bool,
    pub search_hit: Option<crate::thread_search::ThreadSearchHit>,
}

pub(super) fn entries(s: &State) -> Vec<Entry<'_>> {
    let mut entries = Vec::new();
    let searching = !s.search.text.is_empty();
    let visible = s.visible_sidebar_threads();
    for (captain, indices) in [(true, s.sidebar_captains()), (false, s.sidebar_threads())] {
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
                view: &Value::Null,
                children: Vec::new(),
                visible: captain || searching || visible.contains(&index),
                title_match,
                search_hit,
            });
        }
    }
    let query = s.search.text.to_lowercase();
    for (server, projection) in &s.machines.views {
        if server == &s.machines.selected {
            continue;
        }
        let views: std::collections::HashMap<_, _> = projection["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|view| view["id"].as_u64().map(|id| (id as usize, view)))
            .collect();
        let metadata: std::collections::HashMap<_, _> = projection["index"]["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|record| record["id"].as_u64().map(|id| (id as usize, record)))
            .collect();
        let mut children: std::collections::HashMap<usize, Vec<(&Value, &Value)>> =
            Default::default();
        for record in projection["index"]["threads"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let (Some(parent), Some(id)) = (record["parent"].as_u64(), record["id"].as_u64()) {
                children.entry(parent as usize).or_default().push((
                    record,
                    views.get(&(id as usize)).copied().unwrap_or(&Value::Null),
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
            // Workspace IDs are paired with their server, including filter selections.
            if life.archived_at.is_some() != s.show_archive
                || !in_scope
                || record["delegated"] == true && !standalone_child
            {
                continue;
            }
            let view = views.get(&id).copied().unwrap_or(&Value::Null);
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
                    && view["messages"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|message| {
                            message["text"]
                                .as_str()
                                .unwrap_or("")
                                .to_lowercase()
                                .contains(&query)
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
                && view["messages"].as_array().is_none_or(Vec::is_empty)
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

fn status_of_view(view: &Value) -> Status {
    serde_json::from_value(view["status"].clone()).unwrap_or(Status::Ready)
}

pub(super) fn row(s: &State, p: Palette, entry: &Entry<'_>) -> El {
    if let Some(index) = entry.local {
        return thread_row(s, index, p, false);
    }
    let record = entry.record.expect("remote record");
    let projection = entry.projection.expect("remote projection");
    let server = entry.server.to_string();
    let id = entry.id;
    let title = record["title"].as_str().unwrap_or("Thread");
    let view = entry.view;
    let mut status = status_of_view(view);
    let mut requests = usize::from(!view["request"].is_null());
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
            requests += usize::from(!child_view["request"].is_null());
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
        .radius(4.0)
        .hover(p.selected)
        .on_press(move |s: &mut State| {
            if has_hit {
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
        let mut detail = Vec::new();
        let running = std::iter::once(view)
            .chain(entry.children.iter().map(|(_, view)| *view))
            .filter(|view| status_of_view(view) == Status::Running)
            .count();
        let approval = std::iter::once(view)
            .chain(entry.children.iter().map(|(_, view)| *view))
            .any(|view| view["request"]["method"] == "confirm");
        match status {
            Status::Running => detail.push(format!(
                "{} {}",
                running.max(1),
                if running > 1 { "threads" } else { "thread" }
            )),
            Status::Waiting if approval => detail.push("approval".into()),
            Status::Waiting if requests > 0 => detail.push("a question".into()),
            _ => {
                if record["renamed"] == true {
                    detail.push(title.into());
                }
                let count = entry
                    .children
                    .iter()
                    .filter(|(record, _)| {
                        lifecycle::Lifecycle::from_record(record)
                            .is_ok_and(|life| life.archived_at.is_some() == s.show_archive)
                    })
                    .count();
                if count > 0 {
                    detail.push(format!("{count} crew"));
                }
            }
        }
        if !workspace_named_by_filter(
            s,
            entry.server,
            record["project"].as_u64().map(|id| id as usize),
        ) {
            detail.push(workspace.into());
        }
        let timer = std::iter::once(view)
            .chain(entry.children.iter().map(|(_, view)| *view))
            .filter(|view| status_of_view(view) == Status::Running)
            .filter_map(|view| view["elapsed"].as_u64())
            .max();
        let mut status_line = rune::row()
            .key(format!("machine-status-{}-{id}", entry.server))
            .h(18.0)
            .w_full()
            .gap(8.0)
            .center_y()
            .child(pixel_text(status.label(), status_ink(status, p)))
            // The workspace keeps `WORKSPACE_FLOOR`, so the machine name gives way first, down
            // to its icon alone.
            .child(
                label(detail.join(", "), CAPTAIN_STATUS, p.muted)
                    .key(format!("machine-detail-{}-{id}", entry.server))
                    .truncate()
                    .shrink()
                    .beside_tag(
                        true,
                        sidebar_row_width(s) - CAPTAIN_TEXT_INSET,
                        status.label().chars().count() as f32 * 6.6
                            + 24.0
                            + ICON
                            + if timer.is_some() { TIMER_SLOT } else { 0.0 },
                        &detail.join(", "),
                    )
                    .min_w(workspace_floor(
                        sidebar_row_width(s) - CAPTAIN_TEXT_INSET,
                        // The status word, three gaps, the machine icon and the timer slot.
                        status.label().chars().count() as f32 * 6.6
                            + 24.0
                            + ICON
                            + if timer.is_some() { TIMER_SLOT } else { 0.0 },
                        &detail.join(", "),
                    )),
            )
            .child(machine_tag(
                s,
                p,
                entry.server,
                format!("machine-tag-{}-{id}", entry.server),
                CAPTAIN_STATUS,
                room_for_machine_name(
                    sidebar_row_width(s) - CAPTAIN_TEXT_INSET,
                    status.label().chars().count() as f32 * 6.6
                        + 24.0
                        + ICON
                        + if timer.is_some() { TIMER_SLOT } else { 0.0 },
                    &detail.join(", "),
                ),
            ))
            // The machine sits 8 px after the workspace; the free space comes after it.
            .child(spacer());
        if let Some(seconds) = timer {
            status_line = status_line.child(
                label(elapsed_label(seconds), PIXEL.features(TABULAR), p.muted)
                    .w(TIMER_SLOT)
                    .align(rune::Align::End),
            );
        }
        return item.radius(4.0).child(
            rune::row()
                .h(CAPTAIN_SLOT + 12.0)
                .w_full()
                .pl(6.0)
                .pr(8.0)
                .gap(10.0)
                .center_y()
                .child(
                    el().size(CAPTAIN_SLOT, CAPTAIN_SLOT).center().child(
                        pixel_portrait(s, &pixels, CAPTAIN_FACE, p)
                            .key(format!("machine-portrait-{}-{id}", entry.server))
                            .a11y_hidden(),
                    ),
                )
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(1.0)
                        .child(
                            rune::row()
                                .h(20.0)
                                .gap(6.0)
                                .center_y()
                                .child(
                                    label(name, CAPTAIN_NAME, p.ink)
                                        .truncate()
                                        .grow(1.0)
                                        .min_w(0.0),
                                )
                                .child(
                                    rune::row()
                                        .w(BADGE_SLOT)
                                        .h(18.0)
                                        .center_y()
                                        .child(spacer())
                                        .child(if requests > 0 || status == Status::Waiting {
                                            count_pill(requests.max(1), CountKind::NeedsYou, p)
                                        } else if unread > 0 {
                                            count_pill(unread, CountKind::Unread, p)
                                        } else {
                                            el()
                                        }),
                                ),
                        )
                        .child(status_line),
                ),
        );
    }
    let shelved = matches!(
        lifecycle::section(&entry.life, lifecycle::now()),
        Section::Settled | Section::Snoozed
    );
    let at = lifecycle::now();
    let state_label = if view["request"]["method"] == "confirm" {
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
        view["elapsed"]
            .as_u64()
            .map(|secs| format!("Working {}", elapsed_label(secs)))
            .or_else(|| Some("Working".into()))
    } else if !state_label.is_empty() {
        Some(state_label.into())
    } else {
        relative_time(record["updated"].as_u64().unwrap_or(0))
    };
    let mut meta = rune::row().h(18.0).gap(8.0).center_y();
    if entry.draft || entry.life.pinned_at.is_some() {
        meta = meta.child(
            el().size(14.0, 16.0)
                .center()
                .child(icon(if entry.draft { "edit" } else { "pin" }, p.muted)),
        );
    }
    meta = meta
        .child(project_tile(s, p, project_tile_data.as_ref(), 16.0))
        .child(
            rune::row()
                .grow(1.0)
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(
                    label(workspace, CARD_META, p.muted)
                        .key(format!("machine-workspace-{}-{id}", entry.server))
                        .truncate()
                        .shrink()
                        .beside_tag(
                            true,
                            sidebar_row_width(s) - 2.0 * ROW_INSET,
                            16.0 + ICON + slot_width(age.as_deref()) + 32.0,
                            workspace,
                        )
                        .min_w(workspace_floor(
                            sidebar_row_width(s) - 2.0 * ROW_INSET,
                            // The project tile, the machine icon, the status or age text and
                            // the gaps between them.
                            16.0 + ICON + slot_width(age.as_deref()) + 32.0,
                            workspace,
                        )),
                )
                .child(machine_tag(
                    s,
                    p,
                    entry.server,
                    format!("machine-tag-{}-{id}", entry.server),
                    CARD_META,
                    room_for_machine_name(
                        sidebar_row_width(s) - 2.0 * ROW_INSET,
                        16.0 + ICON + slot_width(age.as_deref()) + 32.0,
                        workspace,
                    ),
                ))
                // The machine sits 8 px after the workspace; the free space comes after it.
                .child(spacer()),
        );
    if let Some(age) = age {
        let ink = if status == Status::Waiting {
            p.brass
        } else if status == Status::Failed {
            p.danger
        } else if !state_label.is_empty() {
            p.sea
        } else {
            p.muted
        };
        meta = meta.child(label(age, PIXEL.features(TABULAR), ink));
    }
    let saved = &s.machines.ui[entry.server]["drafts"][id.to_string()];
    let draft = saved
        .as_str()
        .or_else(|| saved["text"].as_str())
        .unwrap_or("");
    let title = label(
        if entry.draft && !draft.trim().is_empty() {
            draft.lines().next().unwrap_or(title)
        } else {
            title
        },
        if shelved {
            LABEL
        } else {
            TextStyle::new(14.0, 520.0)
        },
        p.ink,
    )
    .truncate()
    .min_w(0.0);
    if shelved {
        item.child(
            rune::row()
                .h(34.0)
                .px(ROW_INSET)
                .gap(8.0)
                .center_y()
                .child(project_tile(s, p, project_tile_data.as_ref(), 16.0).help(&machine_line))
                .child(title.grow(1.0))
                .child(label(
                    if lifecycle::section(&entry.life, at) == Section::Snoozed {
                        lifecycle::wake_label(entry.life.snoozed_until.unwrap_or(0), at)
                    } else {
                        relative_time(entry.life.settled_at.unwrap_or(0) / 1000).unwrap_or_default()
                    },
                    PIXEL,
                    p.muted,
                )),
        )
    } else {
        let mut body = col()
            .grow(1.0)
            .min_w(0.0)
            .px(ROW_INSET)
            .pt(9.0)
            .pb(10.0)
            .gap(5.0)
            .child(meta)
            .child(title);
        if let Some(hit) = &entry.search_hit {
            body = body.child(search_snippet_el(
                &server,
                id,
                &hit.snippet,
                &s.search.text,
                p.muted,
                p.brass,
            ));
        }
        item.child(body)
    }
}
