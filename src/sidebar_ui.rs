//! One ordered sidebar across linked machines, ported from KSG Code's SidebarThreadRow
//! and Sidebar.logic. Server/thread pairs, never bare thread IDs, identify remote rows.
use super::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::Ordering as AtomicOrdering;

use super::sidebar_row::{
    CaptainRowModel, RowStatus, ThreadRowModel, captain_row_el, thread_row_el,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RowId {
    pub server: String,
    pub thread: usize,
}

impl RowId {
    pub fn new(server: &str, thread: usize) -> Self {
        Self {
            server: server.into(),
            thread,
        }
    }
    pub fn key(&self, s: &State) -> String {
        if s.client.is_some() || self.server != s.machines.selected {
            format!("machine-thread-{}-{}", self.server, self.thread)
        } else {
            format!("thread-{}", self.thread)
        }
    }
    fn suffix(&self, s: &State) -> String {
        if self.server == s.machines.selected {
            self.thread.to_string()
        } else {
            format!("{}-{}", self.server, self.thread)
        }
    }
}

#[derive(Default)]
struct Relations {
    local: HashMap<usize, usize>,
    approvals: HashSet<usize>,
    crew: HashMap<usize, Vec<usize>>,
    remote: HashMap<String, HashMap<usize, usize>>,
    remote_children: HashMap<RowId, Vec<usize>>,
    pull_requests: HashMap<RowId, u64>,
}

impl Relations {
    fn build(s: &State) -> Self {
        let mut result = Self::default();
        result.add_pull_requests(&s.machines.selected, local_pull_references(s));
        for (index, thread) in s.threads.iter().enumerate() {
            result.local.insert(thread.id, index);
            if thread
                .request
                .as_ref()
                .is_some_and(|request| request["method"] == "confirm")
                && let Some(id) = thread.parent
            {
                result.approvals.insert(id);
            }
            if let Some(captain) = thread.crew_captain {
                result.crew.entry(captain).or_default().push(index);
            }
        }
        for (server, projection) in &s.machines.views {
            if server == &s.machines.selected {
                continue;
            }
            result.add_pull_requests(server, remote_pull_references(projection));
            let mut rows = HashMap::new();
            let homes = &projection["index"]["captainHomes"];
            for (index, record) in projection["index"]["threads"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let Some(id) = record["id"].as_u64().map(|id| id as usize) else {
                    continue;
                };
                rows.insert(id, index);
                let home = record["crewCaptain"]
                    .as_u64()
                    .and_then(|captain| homes[captain.to_string()].as_u64())
                    .or_else(|| record["parent"].as_u64());
                if let Some(home) = home.filter(|home| *home != id as u64) {
                    result
                        .remote_children
                        .entry(RowId::new(server, home as usize))
                        .or_default()
                        .push(id);
                }
            }
            result.remote.insert(server.clone(), rows);
        }
        result
    }

    fn add_pull_requests<'a>(
        &mut self,
        server: &str,
        references: impl Iterator<Item = (usize, &'a str)>,
    ) {
        for (thread, reference) in references {
            if let Some(number) = pull_request_number(reference) {
                // Reports are ordered by capture. Prefer the latest reported PR.
                self.pull_requests
                    .insert(RowId::new(server, thread), number);
            }
        }
    }
}

fn render_local(s: &State, p: Palette, entry: &Entry<'_>, item: El, rel: &Relations) -> El {
    let thread = &s.threads[entry.local.expect("local row")];
    let row_id = RowId::new(entry.server, entry.id);
    let id = row_id.thread;
    let suffix = row_id.suffix(s);
    let now = lifecycle::now();
    let section = s.thread_section(thread, now);
    let worker = thread.crew_captain.is_some();
    let (status, approval) = local_status(s, thread, rel);
    let mut state = RowStatus::of(
        status,
        approval,
        !thread.life.settled() && thread.unread(),
        thread.life.woke(now),
    );
    let workspace = if entry.captain {
        row_workspace(s, thread)
    } else {
        s.project(thread.project)
            .map(|project| project.name.clone())
    };
    let machine = multiple_machines(s).then(|| machine_caption(s, entry.server));
    let draft = entry.draft;
    let shelved = matches!(section, Section::Snoozed | Section::Settled);
    if entry.captain {
        let captain = thread.captain.unwrap_or(0);
        let profile = s.captains.get(captain);
        let name = profile.map_or(thread.title.as_str(), |profile| &profile.name);
        let portrait_id = captain_portrait(s, thread.captain);
        let crew = rel
            .crew
            .get(&captain)
            .into_iter()
            .flatten()
            .map(|&i| &s.threads[i])
            .filter(|t| t.archived() == s.show_archive);
        let family = || std::iter::once(thread).chain(crew.clone());
        let tasks = s.tasks_for(id);
        let mut attention = family()
            .filter(|thread| !thread.life.settled())
            .filter_map(|member| {
                let task = tasks.iter().find(|task| task.child_id == member.id);
                let mark = if let Some(request) = &member.request {
                    if request["method"] == "confirm" {
                        RowStatus::Approval
                    } else {
                        RowStatus::Input
                    }
                } else if task
                    .is_some_and(|task| task.state == crate::model::TaskState::Interrupted)
                {
                    RowStatus::Interrupted
                } else if member.status == Status::Failed
                    || task.is_some_and(|task| task.state == crate::model::TaskState::Failed)
                {
                    RowStatus::Failed
                } else {
                    return None;
                };
                Some((mark, member.title.as_str()))
            })
            .collect::<Vec<_>>();
        attention.sort_by_key(|(mark, _)| match mark {
            RowStatus::Approval => 0,
            RowStatus::Input => 1,
            RowStatus::Interrupted => 2,
            _ => 3,
        });
        let attention_count = attention.len();
        let status = if let Some((mark, _)) = attention.first() {
            if *mark == RowStatus::Failed {
                Status::Failed
            } else {
                Status::Waiting
            }
        } else {
            family()
                .filter(|member| !member.life.settled())
                .map(|member| member.status)
                .max_by_key(|status| status.urgency())
                .unwrap_or(Status::Ready)
        };
        let state = attention.first().map_or_else(
            || RowStatus::of(status, approval, thread.unread(), thread.life.woke(now)),
            |(mark, _)| {
                if attention.iter().all(|(other, _)| other == mark) {
                    *mark
                } else {
                    RowStatus::Attention
                }
            },
        );
        let unread = family()
            .filter(|thread| !thread.life.settled() && thread.unread())
            .count();
        let timer = if status == Status::Running {
            family()
                .filter(|thread| thread.status == Status::Running)
                .filter_map(|thread| {
                    thread
                        .started
                        .map(|at| at.elapsed().as_secs())
                        .or(thread.elapsed)
                })
                .max()
        } else {
            None
        };
        let activity = if thread.archived() {
            "Archived".into()
        } else if let Some((mark, title)) = attention.first() {
            format!("{}: {title}", mark.word())
        } else if let Some(worker) = crew
            .clone()
            .filter(|thread| thread.status == Status::Running)
            .max_by_key(|thread| (thread.started, thread.updated))
        {
            format!("Delegated: {}", worker.title)
        } else if let Some(tool) = thread.tools.last().filter(|tool| !tool.done) {
            if tool.detail.is_empty() {
                tool.name.clone()
            } else {
                tool.detail.clone()
            }
        } else {
            crate::visuals::CAPTAIN_ROLE_DETAIL
                [portrait_id.min(crate::visuals::CAPTAIN_ROLE_DETAIL.len() - 1)]
            .into()
        };
        let art = sidebar_portrait(s, p, id, portrait_id, 36.0, status);
        return captain_row_el(
            s,
            p,
            item,
            CaptainRowModel {
                id: &suffix,
                name,
                portrait: haloed(s, p, art, 36.0, status),
                status: state,
                timer,
                activity: &activity,
                workspace: workspace.as_deref(),
                machine: machine.as_deref(),
                crew: crew.count(),
                attention_count,
                unread,
                selected: entry.selected(s),
            },
        );
    }
    let lead = if worker {
        let task = crew_ui::worker_task_mark(s, thread);
        if !thread.life.settled() && thread.request.is_none() && task == Some(Status::Waiting) {
            state = RowStatus::Interrupted;
        }
        let mark = task.unwrap_or(status);
        let mut avatar = crewmate_avatar(s, p, &thread.model, mark, 16.0)
            .key(format!("worker-captain-portrait-{id}"));
        if thread.actionable() || thread.unread() || task.is_some() {
            avatar = el().size(16.0, 16.0).child(avatar).key(
                if thread.actionable() || thread.unread() {
                    format!("worker-alert-{id}")
                } else {
                    format!("worker-task-mark-{id}")
                },
            );
        }
        avatar
    } else if !draft && !thread.model.is_empty() {
        thread_avatar(s, p, &thread.model, status, thread.pinned())
            .key(format!("thread-provider-portrait-{suffix}"))
    } else {
        lead_mark(state, thread.pinned(), draft, p)
    };
    let snippet = entry.search_hit.as_ref().map(|hit| {
        search_snippet_el(
            entry.server,
            id,
            &hit.snippet,
            &s.search.text,
            p.muted,
            p.brass,
        )
    });
    let title = if draft && !thread.draft.text.trim().is_empty() {
        thread.draft.text.lines().next().unwrap_or(&thread.title)
    } else {
        &thread.title
    };
    let actions = (thread.machine == "local").then(|| {
        let (controls, width) = row_actions(s, p, thread, section, draft);
        (
            controls,
            width,
            sidebar_preview::row_actions_open(s, &row_id),
        )
    });
    let machine =
        machine.or_else(|| (!shelved && !draft).then(|| machine_caption(s, entry.server)));
    thread_row_el(
        s,
        p,
        item,
        ThreadRowModel {
            id: &suffix,
            title,
            status: state,
            elapsed: (status == Status::Running).then(|| {
                thread
                    .started
                    .map(|time| time.elapsed().as_secs())
                    .or(thread.elapsed)
                    .unwrap_or_default()
            }),
            age: row_age(&thread.life, section, thread.updated, now),
            workspace: workspace.as_deref(),
            branch: local_row_branch(s, thread),
            pull_request: rel.pull_requests.get(&row_id).copied(),
            diff: row_diff(s, &row_id),
            machine: machine.as_deref(),
            harness: Some(&thread.harness),
            model: Some(&thread.model),
            agents_active: crate::agents::active_count(thread),
            lead,
            project_lead: (!worker)
                .then(|| s.project(thread.project))
                .flatten()
                .map(|project| project_tile(s, p, Some(project), 16.0)),
            selected: entry.selected(s),
            shelved,
            draft,
            snippet,
        },
        actions,
    )
}

fn local_row_branch<'a>(s: &'a State, thread: &'a Thread) -> Option<&'a str> {
    thread.branch.as_deref().or_else(|| {
        if thread.worktree_path.is_some() || s.vcs_branches.is_empty() {
            return None;
        }
        let key = s.thread_vcs_repo_key(thread)?;
        s.vcs_branches
            .get(&key)?
            .refs
            .iter()
            .find(|branch| branch.current)
            .map(|branch| branch.name.as_str())
    })
}

// Reports contain evidence links, not GitHub state. Only display a number for a real
// GitHub pull request URL and do not infer open/closed status from a report.
fn pull_request_number(reference: &str) -> Option<u64> {
    let url = url::Url::parse(reference).ok()?;
    if url.scheme() != "https" || url.host_str() != Some("github.com") {
        return None;
    }
    let mut segments = url.path_segments()?;
    let (owner, repo, kind, number) = (
        segments.next()?,
        segments.next()?,
        segments.next()?,
        segments.next()?,
    );
    if owner.is_empty()
        || repo.is_empty()
        || kind != "pull"
        || segments.any(|segment| !segment.is_empty())
    {
        return None;
    }
    number.parse::<u64>().ok().filter(|number| *number > 0)
}

fn local_pull_references(s: &State) -> impl Iterator<Item = (usize, &str)> {
    s.crew_reports().iter().flat_map(|report| {
        report
            .artifacts
            .iter()
            .filter(|artifact| {
                artifact.kind == crate::model::crew_reports::ArtifactKind::PullRequest
            })
            .map(|artifact| (report.thread_id, artifact.reference.as_str()))
    })
}

fn remote_pull_references(
    projection: &crate::client_history::Projection,
) -> impl Iterator<Item = (usize, &str)> {
    projection["index"]["captainReports"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|report| {
            Some((
                usize::try_from(report["threadId"].as_u64()?).ok()?,
                report["artifacts"].as_array()?,
            ))
        })
        .flat_map(|(id, artifacts)| {
            artifacts
                .iter()
                .filter(|artifact| artifact["kind"] == "pull_request")
                .filter_map(move |artifact| Some((id, artifact["reference"].as_str()?)))
        })
}

fn row_diff(s: &State, id: &RowId) -> Option<(u32, u32)> {
    let diff = s
        .diffs
        .get(&crate::right_panel::key(&id.server, id.thread))?
        .ready()?;
    (diff.additions > 0 || diff.deletions > 0).then_some((diff.additions, diff.deletions))
}

fn local_status(s: &State, thread: &Thread, rel: &Relations) -> (Status, bool) {
    let approval = rel.approvals.contains(&thread.id)
        || thread
            .request
            .as_ref()
            .is_some_and(|request| request["method"] == "confirm");
    let status = if approval || thread.request.is_some() {
        Status::Waiting
    } else if s.is_captain_home(thread) {
        s.home_status(thread.id)
    } else if thread.status == Status::Ready
        && (thread.client_creating.is_some()
            || thread.remote_send.is_some()
            || thread.client_promoted.is_some())
    {
        Status::Running
    } else {
        thread.status
    };
    let status = if status == Status::Failed && thread.life.settled() {
        Status::Ready
    } else {
        status
    };
    (status, approval)
}

fn row_age(
    life: &lifecycle::Lifecycle,
    section: Section,
    updated: u64,
    now: u64,
) -> Option<String> {
    match section {
        Section::Snoozed => Some(lifecycle::wake_label(
            life.snoozed_until.unwrap_or_default(),
            now,
        )),
        Section::Settled => relative_time(life.settled_at.unwrap_or_default() / 1000),
        _ => relative_time(updated),
    }
}

fn lead_mark(status: RowStatus, pinned: bool, draft: bool, p: Palette) -> El {
    if status != RowStatus::Idle {
        status.mark(p)
    } else if pinned || draft {
        icon(if pinned { "pin" } else { "edit" }, p.faint).size(12.0, 12.0)
    } else {
        el()
    }
}

fn thread_avatar(s: &State, p: Palette, model: &str, status: Status, pinned: bool) -> El {
    let avatar = crewmate_avatar(s, p, model, status, 16.0);
    if !pinned {
        return avatar;
    }
    el().size(16.0, 16.0).child(avatar).child(
        icon("pin", p.ink)
            .size(9.0, 9.0)
            .pin(None, Some(-2.0), Some(-2.0), None)
            .radius(3.0)
            .fill(p.card),
    )
}

/// KSG Code's `SidebarThreadRow` uses the trailing slot for hover actions.
fn row_actions(s: &State, p: Palette, thread: &Thread, section: Section, draft: bool) -> (El, f32) {
    let id = thread.id;
    let mut actions = super::row().h(20.0).center_y().gap(2.0);
    let mut width = 0.0;
    if draft || !thread.draft.text.is_empty() {
        width += 22.0;
        actions = actions.child(
            icon_button(
                &format!("thread-discard-{id}"),
                "Discard draft",
                "close",
                p,
                move |s| s.discard_draft(id),
            )
            .size(20.0, 20.0),
        );
    }
    if !draft {
        if !matches!(section, Section::Snoozed | Section::Settled) {
            width += 22.0;
            actions = actions.child(
                icon_button(
                    &format!("thread-snooze-{id}"),
                    "Snooze",
                    "clock",
                    p,
                    move |s| s.open_snooze(id),
                )
                .size(20.0, 20.0)
                .blocked(
                    s.snooze_blocker(id),
                    &format!("thread-snooze-{id}"),
                    p,
                    false,
                ),
            );
        }
        let (title, glyph) = match section {
            Section::Snoozed => ("Wake", "clock"),
            Section::Settled => ("Un-settle", "check"),
            _ => ("Settle", "check"),
        };
        let action = move |s: &mut State| {
            if section == Section::Snoozed {
                s.unsnooze_thread(id);
            } else {
                s.settle_thread(id);
            }
        };
        let key = format!("thread-park-{id}");
        let control = if title == "Settle" {
            width += 62.0;
            row()
                .key(&key)
                .size(62.0, 20.0)
                .role("button")
                .label(title)
                .focusable(true)
                .focus_ring(1.0, 1.0, p.ring)
                .cursor(rune::Cursor::Pointer)
                .radius(6.0)
                .hover(p.selected)
                .center()
                .gap(4.0)
                .on_press(action)
                .child(icon(glyph, p.muted).size(14.0, 14.0))
                .child(label(title, SMALL, p.muted))
        } else {
            width += 20.0;
            icon_button(&key, title, glyph, p, action).size(20.0, 20.0)
        };
        actions = actions.child(control.blocked(
            if section == Section::Snoozed {
                None
            } else {
                s.lifecycle_blocker(id)
            },
            &key,
            p,
            false,
        ));
    }
    if draft {
        width -= 2.0;
    }
    (actions.w(width), width)
}

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

    pub(super) fn selected(&self, s: &State) -> bool {
        self.server == s.machines.selected
            && (self.id == s.selected_row() || s.selected_threads.contains(&self.id))
    }

    fn needs_you(&self, s: &State, rel: &Relations) -> bool {
        if let Some(index) = self.local {
            let thread = &s.threads[index];
            let (status, approval) = local_status(s, thread, rel);
            approval || thread.request.is_some() || status == Status::Waiting
        } else {
            !self.view.record["request"].is_null() || status_of_view(self.view) == Status::Waiting
        }
    }

    fn working(&self, s: &State, rel: &Relations) -> bool {
        if let Some(index) = self.local {
            local_status(s, &s.threads[index], rel).0 == Status::Running
        } else {
            status_of_view(self.view) == Status::Running
        }
    }

    fn failed(&self, s: &State, rel: &Relations) -> bool {
        !self.life.settled()
            && if let Some(index) = self.local {
                local_status(s, &s.threads[index], rel).0 == Status::Failed
            } else {
                status_of_view(self.view) == Status::Failed
            }
    }

    fn recent_rank(&self, s: &State) -> u64 {
        // `updated` is in Unix seconds; lifecycle timestamps are in Unix milliseconds.
        // Convert `updated` to ms before comparing, and exclude `pinned_at` so pinning
        // a single row keeps it first inside its group without reordering the whole workspace.
        let updated_ms = if let Some(index) = self.local {
            s.threads[index].updated
        } else {
            self.record.and_then(|r| r["updated"].as_u64()).unwrap_or(0)
        }
        .saturating_mul(1000);
        let life_ts = self
            .life
            .latest_user_message_at
            .max(self.life.last_completed_at)
            .unwrap_or(0);
        updated_ms.max(life_ts)
    }
}

/// Group ordinary active/pinned sidebar rows by `(server, project)`.
pub(super) fn groups<'a>(s: &'a State, rows: &[&'a Entry<'a>]) -> Vec<Group<'a>> {
    let mut grouped: Vec<(Group<'a>, u64)> = Vec::new();
    let mut group_index = HashMap::new();
    for &entry in rows {
        let project = entry.project(s);
        let index = *group_index
            .entry((entry.server, project))
            .or_insert_with(|| {
                let index = grouped.len();
                let data = entry.project_data(s);
                grouped.push((
                    Group {
                        server: entry.server,
                        project,
                        name: data
                            .as_ref()
                            .map_or_else(|| "Chats".into(), |p| p.name.clone()),
                        project_data: data,
                        rows: Vec::new(),
                    },
                    0,
                ));
                index
            });
        let (group, rank) = &mut grouped[index];
        group.rows.push(entry);
        *rank = (*rank).max(entry.recent_rank(s));
    }
    // Sort groups: "Chats" (no workspace) last, otherwise most recent row first.
    // Rows inside each group already preserve pinned-first, lifecycle order.
    grouped.sort_by_cached_key(|(group, rank)| {
        (
            group.project.is_none(),
            std::cmp::Reverse(*rank),
            group.name.to_lowercase(),
            group.server,
            group.project,
        )
    });
    grouped.into_iter().map(|(group, _)| group).collect()
}

/// Workspace group header, ported from KSG Code's `SidebarProjectFolderHeader`.
fn group_header(s: &State, p: Palette, group: &Group<'_>, collapsed: bool, rel: &Relations) -> El {
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
    let attention = aggregate_attention(s, p, group.rows.iter().copied(), rel);
    let working = group.rows.iter().any(|e| e.working(s, rel));
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
    if multiple_machines(s) {
        header = header.child(machine_tag(
            s,
            p,
            group.server,
            format!("sidebar-group-machine-{}-{project_key}", group.server),
            SMALL,
            sidebar_row_width(s) >= 232.0,
        ));
    }
    if let Some((ink, word)) = attention {
        header = header.child(attention_dot(
            format!("sidebar-group-attention-{}-{project_key}", group.server),
            ink,
            word,
        ));
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

fn aggregate_attention<'a>(
    s: &State,
    p: Palette,
    rows: impl Iterator<Item = &'a Entry<'a>>,
    rel: &Relations,
) -> Option<(Color, &'static str)> {
    let mut failed = false;
    for entry in rows {
        if entry.needs_you(s, rel) {
            return Some((p.brass, "Needs input"));
        }
        failed |= entry.failed(s, rel);
    }
    failed.then_some((p.danger, "Failed"))
}

fn attention_dot(key: String, ink: Color, word: &str) -> El {
    el().key(key)
        .size(6.0, 6.0)
        .radius(3.0)
        .fill(ink)
        .help(word)
        .label(word)
}

pub(super) fn entries(s: &State) -> Vec<Entry<'_>> {
    let mut entries = Vec::new();
    let searching = !s.search.text.trim().is_empty();
    let at = lifecycle::now();
    let workers = s.searched_workers();
    let mut ordinary: Vec<_> = s
        .sidebar_thread_candidates()
        .map(|(index, _)| index)
        .collect();
    ordinary.extend(workers);
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
            if searching && !title_match && search_hit.is_none() {
                continue;
            }
            let section = s.thread_section(thread, at);
            let shelf = match section {
                Section::Snoozed => Some(0),
                Section::Settled => Some(1),
                _ => None,
            };
            let visible = captain
                || searching
                || shelf.is_none_or(|i| s.shelves[i] || thread.id == s.selected_row());
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
                visible,
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
        let mut home_by_captain: std::collections::HashMap<usize, usize> = Default::default();
        if let Some(homes) = projection["index"]["captainHomes"].as_object() {
            for (cap, home) in homes {
                if let (Ok(cap), Some(home)) = (cap.parse::<usize>(), home.as_u64()) {
                    home_by_captain.insert(cap, home as usize);
                }
            }
        }
        for record in projection["index"]["threads"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(id) = record["id"].as_u64() else {
                continue;
            };
            let owner_home = record
                .get("crewCaptain")
                .and_then(Value::as_u64)
                .and_then(|cap| home_by_captain.get(&(cap as usize)).copied())
                .or_else(|| record["parent"].as_u64().map(|p| p as usize));
            if let Some(home) = owner_home
                && home != id as usize
            {
                children.entry(home).or_default().push((
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
            // Keep a live remote captain visible in archive view when it has archived workers,
            // matching the local `sidebar_captains` rule.
            let captain_has_archived_worker = captain
                && s.show_archive
                && children.get(&id).is_some_and(|kids| {
                    kids.iter().any(|(child, _)| {
                        lifecycle::Lifecycle::from_record(child)
                            .is_ok_and(|life| life.archived_at.is_some())
                    })
                });
            // Workspace IDs are paired with their server, including filter selections.
            if (life.archived_at.is_some() != s.show_archive && !captain_has_archived_worker)
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
                match lifecycle::section(&life, at) {
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
                visible,
                title_match,
                search_hit,
            });
        }
    }
    let order: HashMap<_, _> = s
        .sidebar_search
        .shown_order
        .iter()
        .enumerate()
        .map(|(rank, (server, id))| ((server.as_str(), *id), rank))
        .collect();
    entries.sort_by_cached_key(|row| {
        let section = lifecycle::section(&row.life, at);
        let rank = lifecycle::rank(section, &row.life);
        let category = if searching {
            if row.title_match { 0 } else { 1 }
        } else if row.captain {
            0
        } else {
            1
        };
        let sort_lifecycle = !row.captain && (!searching || row.title_match);
        (
            category,
            if searching && !row.title_match {
                order
                    .get(&(row.server, row.id))
                    .copied()
                    .unwrap_or(usize::MAX)
            } else {
                0
            },
            sort_lifecycle && !row.draft,
            sort_lifecycle.then_some(section),
            sort_lifecycle.then_some(rank),
            row.server,
            row.id,
        )
    });
    entries
}

fn status_of_view(view: &crate::client_history::ThreadProjection) -> Status {
    serde_json::from_value(view.record["status"].clone()).unwrap_or(Status::Ready)
}

fn view_running_seconds(view: &crate::client_history::ThreadProjection) -> Option<u64> {
    if let Some(started_ms) = view.record["started_at"].as_u64() {
        return Some(lifecycle::now().saturating_sub(started_ms) / 1000);
    }
    view.record["elapsed"].as_u64()
}

fn null_view() -> &'static crate::client_history::ThreadProjection {
    static NULL: std::sync::OnceLock<crate::client_history::ThreadProjection> =
        std::sync::OnceLock::new();
    NULL.get_or_init(|| crate::client_history::ThreadProjection::empty(&serde_json::json!({})))
}

#[derive(Default)]
pub(crate) struct SidebarIndex {
    signature: Option<u64>,
    relation_signature: Option<u64>,
    plan: Arc<Plan>,
    relations: Arc<Relations>,
    #[cfg(test)]
    pub(crate) rebuilds: usize,
}

#[derive(Clone)]
struct IndexedRow {
    id: RowId,
    captain: bool,
    draft: bool,
    title_match: bool,
}

enum Node {
    Captains(usize),
    Threads(bool),
    Drafts,
    FormerHeading,
    Former(usize),
    SearchHeader(usize),
    Empty,
    Thread(IndexedRow),
    Group {
        server: String,
        project: Option<usize>,
        rows: Vec<IndexedRow>,
    },
    Shelf {
        section: Section,
        rows: Vec<IndexedRow>,
        index: usize,
        inset: f32,
    },
    Drop(Section),
    More(usize),
}

#[derive(Default)]
struct Plan {
    nodes: Vec<(String, Node)>,
    shelves: Vec<usize>,
    navigation: Vec<(String, usize)>,
    targets: HashMap<RowId, String>,
    search_count: Option<usize>,
    range: Vec<usize>,
    version: u64,
}

fn structure_signature(s: &State, at: u64) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    s.machines.selected.hash(&mut hash);
    s.client.is_some().hash(&mut hash);
    s.show_archive.hash(&mut hash);
    s.sidebar_grouped().hash(&mut hash);
    s.shelves.hash(&mut hash);
    s.settled_visible.hash(&mut hash);
    s.drag_thread.is_some().hash(&mut hash);
    if s.sidebar_grouped() || s.shelves.contains(&false) {
        s.selected_row().hash(&mut hash);
    }
    s.selected_threads.hash(&mut hash);
    format!("{:?}", s.project_scope).hash(&mut hash);
    format!("{:?}", s.scope_additional).hash(&mut hash);
    format!("{:?}", s.machines.scope_remote).hash(&mut hash);
    s.machines.scope_server.hash(&mut hash);
    s.search.text.hash(&mut hash);
    s.sidebar_search.shown_order.hash(&mut hash);
    for project in &s.projects.entries {
        project.id.hash(&mut hash);
        project.name.hash(&mut hash);
        s.sidebar_group_collapsed(&s.machines.selected, Some(project.id))
            .hash(&mut hash);
    }
    s.sidebar_group_collapsed(&s.machines.selected, None)
        .hash(&mut hash);
    for t in &s.threads {
        t.id.hash(&mut hash);
        t.machine.hash(&mut hash);
        t.project.hash(&mut hash);
        t.parent.hash(&mut hash);
        t.captain.hash(&mut hash);
        t.captain_home.hash(&mut hash);
        t.crew_captain.hash(&mut hash);
        t.delegated.hash(&mut hash);
        s.draft_listed(t).hash(&mut hash);
        (s.thread_section(t, at) as u8).hash(&mut hash);
        let life = &t.life;
        life.archived_at.hash(&mut hash);
        life.pinned_at.hash(&mut hash);
        life.pin_order_key.hash(&mut hash);
        life.snoozed_until.hash(&mut hash);
        life.snoozed_at.hash(&mut hash);
        life.settled_at.hash(&mut hash);
        life.unsettled_at.hash(&mut hash);
        life.last_requested_at.hash(&mut hash);
        if life.settled() {
            life.last_started_at.hash(&mut hash);
            life.updated_at.hash(&mut hash);
        }
        life.latest_user_message_at.hash(&mut hash);
        life.last_completed_at.hash(&mut hash);
        life.created_at.hash(&mut hash);
        if s.sidebar_grouped() {
            t.updated.hash(&mut hash);
        }
        if !s.search.text.is_empty() {
            t.title.hash(&mut hash);
            t.draft.text.hash(&mut hash);
        }
    }
    for captain in 0..s.captains.len() {
        if s.show_archive || !s.search.text.is_empty() {
            s.captains[captain].name.hash(&mut hash);
        }
        (s.captain_phase(captain) as u8).hash(&mut hash);
        s.captain_scope(captain).hash(&mut hash);
    }
    s.hash_sidebar_coordinators(&mut hash);
    for (server, projection) in &s.machines.views {
        if server == &s.machines.selected {
            continue;
        }
        server.hash(&mut hash);
        s.machine_enabled(server).hash(&mut hash);
        if let Some(records) = projection["index"]["threads"].as_array() {
            records.len().hash(&mut hash);
            for record in records {
                for key in [
                    "id",
                    "project",
                    "captain",
                    "captainHome",
                    "crewCaptain",
                    "parent",
                    "delegated",
                    "neverSent",
                    "session",
                    "renamed",
                    "lifecycle",
                    "updated",
                    "archived",
                    "pinned",
                ] {
                    record[key].hash(&mut hash);
                }
                if !s.search.text.is_empty() {
                    record["title"].hash(&mut hash);
                }
            }
        }
        for key in ["captainHomes", "captainScopes", "projects"] {
            projection["index"][key].hash(&mut hash);
        }
        s.machines.ui[server]["drafts"].hash(&mut hash);
        s.sidebar_group_collapsed(server, None).hash(&mut hash);
        for project in projection["index"]["projects"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(id) = project["id"].as_u64() {
                s.sidebar_group_collapsed(server, Some(id as usize))
                    .hash(&mut hash);
            }
        }
    }
    hash.finish()
}

fn relations_signature(s: &State, structure: u64) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    structure.hash(&mut hash);
    for t in &s.threads {
        if t.request
            .as_ref()
            .is_some_and(|request| request["method"] == "confirm")
        {
            t.id.hash(&mut hash);
        }
    }
    for reference in local_pull_references(s) {
        reference.hash(&mut hash);
    }
    for (server, projection) in &s.machines.views {
        if server != &s.machines.selected {
            server.hash(&mut hash);
            for reference in remote_pull_references(projection) {
                reference.hash(&mut hash);
            }
        }
    }
    hash.finish()
}

fn cached(s: &State) -> (Arc<Plan>, Arc<Relations>) {
    let at = lifecycle::now();
    let signature = structure_signature(s, at);
    let relation_signature = relations_signature(s, signature);
    let mut index = s.sidebar_index.borrow_mut();
    if index.signature != Some(signature) {
        index.signature = Some(signature);
        index.plan = Arc::new(build_plan(s, at, index.plan.version.wrapping_add(1)));
        #[cfg(test)]
        {
            index.rebuilds += 1;
        }
    }
    if index.relation_signature != Some(relation_signature) {
        index.relation_signature = Some(relation_signature);
        index.relations = Arc::new(Relations::build(s));
    }
    (index.plan.clone(), index.relations.clone())
}

impl IndexedRow {
    fn new(row: &Entry<'_>) -> Self {
        Self {
            id: RowId::new(row.server, row.id),
            captain: row.captain,
            draft: row.draft,
            title_match: row.title_match,
        }
    }
}

impl Plan {
    fn add(&mut self, key: impl Into<String>, node: Node) {
        if matches!(node, Node::Shelf { .. }) {
            self.shelves.push(self.nodes.len());
        }
        self.nodes.push((key.into(), node));
    }
    fn thread(&mut self, s: &State, entry: &Entry<'_>) {
        let row = IndexedRow::new(entry);
        let key = if row.captain {
            if s.client.is_some() {
                format!("crew-{}-{}", row.id.server, row.id.thread)
            } else {
                format!("crew-{}", row.id.thread)
            }
        } else {
            row.id.key(s)
        };
        self.navigation.push((row.id.server.clone(), row.id.thread));
        self.targets.insert(row.id.clone(), key.clone());
        if !row.captain && row.id.server == s.machines.selected {
            self.range.push(row.id.thread);
        }
        self.add(key, Node::Thread(row));
    }
}

fn build_plan(s: &State, at: u64, version: u64) -> Plan {
    let mut plan = Plan {
        version,
        ..Plan::default()
    };
    let entries = entries(s);
    let searching = !s.search.text.trim().is_empty();
    if searching {
        let count = entries.iter().filter(|row| row.visible).count();
        plan.search_count = Some(count);
        plan.add("sidebar-search-header", Node::SearchHeader(count));
        for row in entries.iter().filter(|row| row.visible) {
            plan.thread(s, row);
        }
        return plan;
    }
    let captains: Vec<_> = entries.iter().filter(|row| row.captain).collect();
    if !s.show_archive || !captains.is_empty() {
        plan.add("captains-heading", Node::Captains(captains.len()));
        for row in &captains {
            plan.thread(s, row);
        }
    }
    if s.show_archive {
        for captain in 0..s.captains.len() {
            if s.captain_phase(captain) != crate::model::CaptainPhase::Retired {
                continue;
            }
            if !plan
                .nodes
                .iter()
                .any(|(_, node)| matches!(node, Node::FormerHeading))
            {
                plan.add("former-captains", Node::FormerHeading);
            }
            plan.add(format!("former-captain-{captain}"), Node::Former(captain));
        }
    }
    if !s.show_archive {
        plan.add("threads-heading", Node::Threads(!captains.is_empty()));
    }
    let all: Vec<_> = entries.iter().filter(|row| !row.captain).collect();
    let mut drafts = all
        .iter()
        .copied()
        .filter(|row| row.draft && row.visible)
        .peekable();
    if drafts.peek().is_some() {
        plan.add("sidebar-drafts-heading", Node::Drafts);
        for row in drafts {
            plan.thread(s, row);
        }
    }
    let dragging = s.drag_thread.is_some();
    let grouped = s.sidebar_grouped() && !dragging;
    if grouped {
        let active: Vec<_> = all
            .iter()
            .copied()
            .filter(|row| {
                !row.draft
                    && row.visible
                    && matches!(
                        lifecycle::section(&row.life, at),
                        Section::Pinned | Section::Active
                    )
            })
            .collect();
        for group in groups(s, &active) {
            let key = format!(
                "sidebar-group-{}-{}",
                group.server,
                group
                    .project
                    .map_or_else(|| "none".into(), |id| id.to_string())
            );
            let collapsed = s.sidebar_group_collapsed(group.server, group.project);
            plan.add(
                key,
                Node::Group {
                    server: group.server.into(),
                    project: group.project,
                    rows: group.rows.iter().map(|row| IndexedRow::new(row)).collect(),
                },
            );
            for row in group
                .rows
                .into_iter()
                .filter(|row| !collapsed || row.selected(s))
            {
                plan.thread(s, row);
            }
        }
    }
    let settled_exists = all
        .iter()
        .any(|row| !row.draft && lifecycle::section(&row.life, at) == Section::Settled);
    let sections = [
        Section::Pinned,
        Section::Active,
        Section::Snoozed,
        Section::Settled,
    ];
    for section in sections.into_iter().skip(if grouped { 2 } else { 0 }) {
        let rows: Vec<_> = all
            .iter()
            .copied()
            .filter(|row| !row.draft && lifecycle::section(&row.life, at) == section)
            .collect();
        if matches!(section, Section::Snoozed | Section::Settled) {
            if rows.is_empty() && !(dragging && section == Section::Settled) {
                continue;
            }
            plan.add(
                format!("shelf-block-{section:?}"),
                Node::Shelf {
                    section,
                    rows: rows.iter().map(|row| IndexedRow::new(row)).collect(),
                    index: usize::from(section == Section::Settled),
                    inset: if section == Section::Snoozed && (settled_exists || dragging) {
                        SHELF_HEADING
                    } else {
                        0.0
                    },
                },
            );
        } else if dragging {
            plan.add(format!("drop-{section:?}"), Node::Drop(section));
        }
        let limit = if section == Section::Settled {
            s.settled_visible
        } else {
            usize::MAX
        };
        for row in rows.iter().copied().filter(|row| row.visible).take(limit) {
            plan.thread(s, row);
        }
        if section == Section::Settled && s.shelves[1] && rows.len() > limit {
            plan.add("settled-more", Node::More((rows.len() - limit).min(25)));
        }
    }
    if all.is_empty() && captains.is_empty() {
        plan.add("sidebar-empty-slot", Node::Empty);
    }
    plan
}

pub(super) fn navigation(s: &State) -> Vec<(String, usize)> {
    cached(s).0.navigation.clone()
}

pub(super) fn target_key(s: &State, server: &str, id: usize) -> String {
    let row = RowId::new(server, id);
    // The inner button keeps its public key; the virtual wrapper has the captain key.
    s.sidebar_index
        .borrow()
        .plan
        .targets
        .get(&row)
        .cloned()
        .unwrap_or_else(|| row.key(s))
}

pub(crate) fn drag_row_at(
    s: &State,
    dragging: usize,
    [x, y]: [f32; 2],
) -> Option<(Section, Option<usize>)> {
    let cache = s.sidebar_index.borrow();
    let at = lifecycle::now();
    let within = |[rx, ry, rw, rh]: [f32; 4]| x >= rx && x <= rx + rw && y >= ry && y <= ry + rh;
    if s.drop_rects
        .get("sidebar-scroll")
        .is_some_and(|rect| !within(*rect))
    {
        return None;
    }
    for (key, rect) in &s.drop_rects {
        let Some(id) = key
            .strip_prefix("thread-")
            .and_then(|key| key.parse::<usize>().ok())
            .filter(|id| *id != dragging)
        else {
            continue;
        };
        if !within(*rect) {
            continue;
        }
        let Some(place) = cache
            .plan
            .range
            .iter()
            .position(|candidate| *candidate == id)
        else {
            continue;
        };
        let thread = s.threads.get(*cache.relations.local.get(&id)?)?;
        let section = s.thread_section(thread, at);
        let before = if y < rect[1] + rect[3] / 2.0 {
            Some(id)
        } else {
            cache.plan.range[place + 1..].iter().copied().find(|next| {
                cache
                    .relations
                    .local
                    .get(next)
                    .and_then(|&index| s.threads.get(index))
                    .is_some_and(|thread| s.thread_section(thread, at) == section)
            })
        };
        return Some((section, before));
    }
    None
}

fn resolve<'a>(s: &'a State, row: &'a IndexedRow, rel: &Relations) -> Option<Entry<'a>> {
    let server = row.id.server.as_str();
    let id = row.id.thread;
    let (local, record, projection, view, life) = if server == s.machines.selected {
        let index = *rel.local.get(&id)?;
        let thread = s.threads.get(index).filter(|thread| thread.id == id)?;
        (Some(index), None, None, null_view(), thread.life.clone())
    } else {
        let projection = s.machines.views.get(server)?;
        let index = *rel.remote.get(server)?.get(&id)?;
        let record = projection["index"]["threads"].get(index)?;
        let view = projection
            .views
            .get(&(id as u64))
            .map(std::sync::Arc::as_ref)
            .unwrap_or(null_view());
        (
            None,
            Some(record),
            Some(projection),
            view,
            lifecycle::Lifecycle::from_record(record).ok()?,
        )
    };
    let search_hit = if !row.title_match {
        s.sidebar_search
            .visible_matches
            .get(&(server.to_owned(), id))
            .cloned()
    } else {
        None
    };
    Some(Entry {
        server,
        id,
        local,
        captain: row.captain,
        draft: row.draft,
        life,
        record,
        projection,
        view,
        visible: true,
        title_match: row.title_match,
        search_hit,
    })
}

fn remote_record<'a>(
    projection: &'a crate::client_history::Projection,
    rel: &Relations,
    server: &str,
    id: usize,
) -> Option<&'a Value> {
    projection["index"]["threads"].get(*rel.remote.get(server)?.get(&id)?)
}

fn render_remote(s: &State, p: Palette, entry: &Entry<'_>, item: El, rel: &Relations) -> El {
    let projection = entry.projection.expect("remote projection");
    let record = entry.record.expect("remote record");
    let view = entry.view;
    let id = entry.id;
    let row_id = RowId::new(entry.server, id);
    let suffix = row_id.suffix(s);
    let model = view.record["model"]
        .as_str()
        .or_else(|| record["model"].as_str());
    let mut status = status_of_view(view);
    if status == Status::Failed && entry.life.settled() {
        status = Status::Ready;
    }
    let mut approval = view.record["request"]["method"] == "confirm";
    let mut elapsed = view_running_seconds(view);
    let mut requests = usize::from(!view.record["request"].is_null());
    let mut failures = usize::from(status == Status::Failed);
    let mut unread = usize::from(!entry.life.settled() && entry.life.unread());
    let mut crew = 0;
    for child in rel.remote_children.get(&row_id).into_iter().flatten() {
        let child_view = projection
            .views
            .get(&(*child as u64))
            .map(std::sync::Arc::as_ref)
            .unwrap_or(null_view());
        if !entry.captain {
            approval |= child_view.record["request"]["method"] == "confirm";
            continue;
        }
        let Some(child_record) = remote_record(projection, rel, entry.server, *child) else {
            continue;
        };
        let Ok(life) = lifecycle::Lifecycle::from_record(child_record) else {
            continue;
        };
        if life.archived_at.is_some() != s.show_archive {
            continue;
        }
        crew += 1;
        if life.settled() {
            continue;
        }
        approval |= child_view.record["request"]["method"] == "confirm";
        requests += usize::from(!child_view.record["request"].is_null());
        unread += usize::from(life.unread());
        let state = status_of_view(child_view);
        failures += usize::from(state == Status::Failed && child_view.record["request"].is_null());
        if state == Status::Running {
            elapsed = elapsed.max(view_running_seconds(child_view));
        }
        if state.urgency() > status.urgency() {
            status = state;
        }
    }
    if approval || requests > 0 {
        status = Status::Waiting;
    }
    let now = lifecycle::now();
    let state = RowStatus::of(status, approval, unread > 0, entry.life.woke(now));
    let title = record["title"].as_str().unwrap_or("Thread");
    let project = entry.project_data(s);
    let workspace = if entry.captain {
        (!workspace_named_by_filter(s, entry.server, entry.project(s))).then(|| {
            project
                .as_ref()
                .map_or("No workspace", |project| project.name.as_str())
        })
    } else {
        project.as_ref().map(|project| project.name.as_str())
    };
    let machine = machine_caption(s, entry.server);
    if entry.captain {
        let captain = record["captain"]
            .as_u64()
            .and_then(|index| projection["index"]["captains"].get(index as usize));
        let name = captain
            .and_then(|cap| cap["name"].as_str())
            .unwrap_or(title);
        let slot = captain
            .and_then(|cap| cap["portrait"].as_u64())
            .unwrap_or_default() as usize;
        let activity = if requests > 0 {
            format!(
                "{requests} {}",
                if requests == 1 { "request" } else { "requests" }
            )
        } else if failures > 0 {
            format!("{failures} failed")
        } else if status == Status::Running {
            "Working".into()
        } else {
            crate::visuals::CAPTAIN_ROLE_DETAIL
                [slot.min(crate::visuals::CAPTAIN_ROLE_DETAIL.len() - 1)]
            .into()
        };
        return captain_row_el(
            s,
            p,
            item,
            CaptainRowModel {
                id: &suffix,
                name,
                portrait: haloed(
                    s,
                    p,
                    captain_face(s, p, slot, face_for(status), 36.0)
                        .key(format!("machine-portrait-{}-{id}", entry.server)),
                    36.0,
                    status,
                ),
                status: if requests > 0 && failures > 0 {
                    RowStatus::Attention
                } else {
                    state
                },
                timer: elapsed.filter(|_| status == Status::Running),
                activity: &activity,
                workspace,
                machine: Some(&machine),
                crew,
                attention_count: requests + failures,
                unread,
                selected: entry.selected(s),
            },
        );
    }
    let section = lifecycle::section(&entry.life, now);
    let shelved = matches!(section, Section::Snoozed | Section::Settled);
    let worker = !record["crewCaptain"].is_null()
        || record["parent"]
            .as_u64()
            .and_then(|id| remote_record(projection, rel, entry.server, id as usize))
            .is_some_and(|parent| parent["captainHome"] == true);
    let lead = if worker {
        crewmate_avatar(s, p, model.unwrap_or_default(), status, 16.0)
            .key(format!("crewmate-{}-{id}", entry.server))
    } else if let Some(model) = model.filter(|model| !model.is_empty() && !entry.draft) {
        thread_avatar(s, p, model, status, entry.life.pinned_at.is_some())
            .key(format!("thread-provider-portrait-{suffix}"))
    } else {
        lead_mark(state, entry.life.pinned_at.is_some(), entry.draft, p)
    };
    let saved = &s.machines.ui[entry.server]["drafts"][id.to_string()];
    let draft = saved
        .as_str()
        .or_else(|| saved["text"].as_str())
        .unwrap_or_default();
    let title = if entry.draft && !draft.trim().is_empty() {
        draft.lines().next().unwrap_or(title)
    } else {
        title
    };
    let snippet = entry.search_hit.as_ref().map(|hit| {
        search_snippet_el(
            entry.server,
            id,
            &hit.snippet,
            &s.search.text,
            p.muted,
            p.brass,
        )
    });
    thread_row_el(
        s,
        p,
        item,
        ThreadRowModel {
            id: &suffix,
            title,
            status: state,
            elapsed: elapsed.filter(|_| status == Status::Running),
            age: row_age(
                &entry.life,
                section,
                record["updated"].as_u64().unwrap_or_default(),
                now,
            ),
            workspace,
            branch: view.record["branch"]
                .as_str()
                .or_else(|| record["branch"].as_str()),
            pull_request: rel.pull_requests.get(&row_id).copied(),
            diff: row_diff(s, &row_id),
            machine: Some(&machine),
            harness: view.record["harness"]
                .as_str()
                .or_else(|| record["harness"].as_str()),
            model,
            agents_active: view.record["agentsActive"]
                .as_u64()
                .or_else(|| record["agentsActive"].as_u64())
                .unwrap_or_default() as usize,
            lead,
            project_lead: project
                .as_ref()
                .filter(|_| !worker)
                .map(|project| project_tile(s, p, Some(project), 16.0)),
            selected: entry.selected(s),
            shelved,
            draft: entry.draft,
            snippet,
        },
        None,
    )
}

fn render_thread(s: &State, p: Palette, row: &IndexedRow, rel: &Relations) -> El {
    let Some(entry) = resolve(s, row, rel) else {
        return el();
    };
    let id = row.id.thread;
    let local = entry.local.is_some();
    let target = RowTarget::Thread(id);
    let key = row.id.key(s);
    let press_id = row.id.clone();
    let hover_id = row.id.clone();
    let key_id = row.id.clone();
    let mut item = super::row()
        .key(&key)
        .role("button")
        .focusable(true)
        .focus_ring(1.0, 1.0, p.ring)
        .cursor(rune::Cursor::Pointer)
        .w_full()
        .min_w(0.0)
        .radius(6.0)
        .hover(p.selected)
        .pointer_buttons([rune::PointerButton::Middle])
        .on_press_tx(move |s: &mut State, tx| {
            super::sidebar_preview::dismiss(s, true);
            let current = press_id.server == s.machines.selected;
            if tx.button() == rune::PointerButton::Middle {
                s.open_machine_thread_kept(&press_id.server, id);
            } else if s
                .sidebar_search
                .visible_matches
                .contains_key(&(press_id.server.clone(), id))
                && !s.search.text.is_empty()
                && (!current
                    || s.threads
                        .iter()
                        .find(|thread| thread.id == id)
                        .is_some_and(|thread| !s.thread_title_search_matches(thread)))
            {
                s.open_thread_at_search_match(&press_id.server, id);
            } else if current
                && let Some(index) = s.threads.iter().position(|thread| thread.id == id)
            {
                let plan = s.sidebar_index.borrow().plan.clone();
                s.click_thread_in_order(index, tx.mods(), &plan.range);
            } else {
                s.open_machine_thread(&press_id.server, id);
            }
        })
        .on_hover_change(move |s: &mut State, hovered, tx| {
            super::sidebar_preview::row_hover(s, &hover_id, hovered, tx);
        })
        .on_key(move |s: &mut State, ev, tx| match ev.key {
            rune::Key::Function(2) if local => {
                s.begin_rename(id);
                true
            }
            rune::Key::Up | rune::Key::Down => {
                let (plan, _) = cached(s);
                let Some(index) = plan
                    .navigation
                    .iter()
                    .position(|(server, id)| server == &key_id.server && id == &key_id.thread)
                else {
                    return false;
                };
                let next = if ev.key == rune::Key::Up {
                    index.checked_sub(1)
                } else {
                    index.checked_add(1)
                };
                let Some((server, id)) = next.and_then(|index| plan.navigation.get(index)) else {
                    return true;
                };
                let next = RowId::new(server, *id);
                tx.scroll_into_view_in("sidebar-scroll", target_key(s, server, *id));
                tx.focus(next.key(s), true);
                super::sidebar_preview::keyboard(s, next, tx);
                true
            }
            _ => false,
        });
    if local {
        let drop_key = format!("thread-{id}");
        let visible_key = drop_key.clone();
        let local_thread = &s.threads[entry.local.unwrap()];
        item = item
            .on_double_press(move |s: &mut State| s.begin_rename(id))
            .on_secondary_press(move |s: &mut State, at, _| {
                super::sidebar_preview::dismiss(s, true);
                s.row_menu = Some(target);
                s.row_menu_at = Some((at.x, at.y));
            });
        if !entry.captain && local_thread.crew_captain.is_none() && local_thread.machine == "local"
        {
            item = item
                .on_drag(move |s: &mut State, drag, _| s.drag_pointer(id, drag))
                .on_drawn_layout_tx(move |s: &mut State, rect, tx| {
                    s.drop_rects.insert(drop_key.clone(), rect);
                    tx.live_only();
                })
                .on_visible(move |s: &mut State, visible, tx| {
                    if !visible {
                        s.drop_rects.remove(&visible_key);
                    }
                    tx.live_only();
                });
        }
    }
    let built = if local {
        render_local(s, p, &entry, item, rel)
    } else {
        render_remote(s, p, &entry, item, rel)
    };
    if entry.captain {
        col().child(built)
    } else {
        built
    }
}

fn clear_search(s: &mut State) {
    s.search.replace_text("");
    s.sync_sidebar_search_input();
}

pub(super) fn search_clear(s: &State, p: Palette) -> El {
    if s.search.text.is_empty() {
        return el();
    }
    icon_button(
        "thread-search-clear",
        "Clear search",
        "close",
        p,
        clear_search,
    )
    .size(22.0, 22.0)
}

fn search_empty(s: &State, p: Palette) -> El {
    if s.sidebar_search.pending {
        return el();
    }
    let mut body = col()
        .key("sidebar-empty")
        .px(12.0)
        .py(10.0)
        .gap(6.0)
        .child(icon("search", p.faint).size(20.0, 20.0))
        .child(label("No matching conversations", TITLE, p.ink).wrap())
        .child(
            label(
                format!("No results for \"{}\"", s.search.text.trim()),
                SMALL,
                p.muted,
            )
            .wrap_anywhere(),
        )
        .child(button(
            "sidebar-search-clear",
            "Clear search",
            p,
            clear_search,
        ));
    if s.project_scope != ProjectScope::All
        || !s.scope_additional.is_empty()
        || !s.machines.scope_remote.is_empty()
    {
        body = body.child(button(
            "sidebar-search-all-workspaces",
            "Search all workspaces",
            p,
            |s| {
                s.project_scope = ProjectScope::All;
                s.scope_additional.clear();
                s.machines.scope_remote.clear();
                s.machines.scope_server = None;
                s.selected_threads.clear();
                s.checkpoint();
            },
        ));
    }
    body
}

fn search_status(s: &State, p: Palette) -> El {
    let mut status = col();
    if s.sidebar_search.pending {
        let mut glyph = icon("refresh", p.muted).size(12.0, 12.0);
        if s.portraits.motion_allowed() {
            glyph = glyph.looping(rune::LoopSpec::rotation(1.2));
        }
        status = status.child(
            super::row()
                .key("sidebar-search-pending")
                .role("status")
                .label("Searching message text")
                .h(28.0)
                .px(12.0)
                .gap(6.0)
                .center_y()
                .child(glyph)
                .child(label("Searching message text…", SMALL, p.muted)),
        );
    }
    if !s.sidebar_search.unsupported_servers.is_empty() {
        status = status.child(
            label(
                format!(
                    "Titles only on {} (update required)",
                    s.sidebar_search.unsupported_servers.join(", ")
                ),
                SMALL,
                p.muted,
            )
            .key("sidebar-search-unsupported")
            .px(ROW_INSET)
            .help(crate::protocol::THREAD_SEARCH_UPDATE_REQUIRED)
            .truncate(),
        );
    }
    status
}

fn render_node(s: &State, p: Palette, node: &Node, rel: &Relations) -> El {
    match node {
        Node::Thread(row) => render_thread(s, p, row, rel),
        Node::Captains(count) => {
            let mut heading = section_title("Captains", p);
            if *count > 0 {
                heading = heading.child(
                    label(count.to_string(), TIMER, p.faint)
                        .key("captain-count")
                        .mr(6.0),
                );
            }
            if !s.show_archive {
                heading = heading.child(icon_button(
                    "assign-captain",
                    "New captain thread",
                    "plus",
                    p,
                    |s| s.open_captain_selector(),
                ));
            }
            heading
        }
        Node::Threads(after_captains) => section_title("Threads", p)
            .mt(if *after_captains { 8.0 } else { 0.0 })
            .child(project_switcher(s, p).help("Add workspace")),
        Node::Drafts => section_title("Drafts", p),
        Node::FormerHeading => section_title("Former captains", p),
        Node::Former(index) => {
            let index = *index;
            let profile = &s.captains[index];
            let server = s.machines.selected.clone();
            captain_row_el(
                s,
                p,
                super::row()
                    .role("button")
                    .focusable(true)
                    .hover(p.selected)
                    .on_press(move |s: &mut State| {
                        if s.machines.selected == server
                            && s.captain_phase(index) == crate::model::CaptainPhase::Retired
                        {
                            s.brief.record = Some((server.clone(), index));
                        }
                    }),
                CaptainRowModel {
                    id: &format!("former-{index}"),
                    name: &profile.name,
                    portrait: captain_face(s, p, profile.portrait, face_for(Status::Ready), 36.0),
                    status: RowStatus::Idle,
                    timer: None,
                    activity: "Demoted",
                    workspace: None,
                    machine: None,
                    crew: 0,
                    attention_count: 0,
                    unread: 0,
                    selected: false,
                },
            )
        }
        Node::Group {
            server,
            project,
            rows,
        } => {
            let resolved: Vec<_> = rows.iter().filter_map(|row| resolve(s, row, rel)).collect();
            let data = resolved.first().and_then(|row| row.project_data(s));
            let group = Group {
                server,
                project: *project,
                name: data
                    .as_ref()
                    .map_or_else(|| "Chats".into(), |project| project.name.clone()),
                project_data: data,
                rows: resolved.iter().collect(),
            };
            group_header(
                s,
                p,
                &group,
                s.sidebar_group_collapsed(server, *project),
                rel,
            )
        }
        Node::SearchHeader(count) => section_title(
            &format!("Results for \"{}\"", s.search.text.trim()),
            p,
        )
        .child(label(count.to_string(), TIMER, p.faint)),
        Node::Empty => col().child(empty_list(
            s,
            p,
            "sidebar-empty",
            "No threads in this workspace yet",
        )),
        Node::More(count) => button("settled-more", &format!("Show {count} more"), p, |s| {
            s.settled_visible += 25
        }),
        Node::Drop(section) => {
            let section = *section;
            let key = format!("drop-{section:?}");
            let selected = s.drag_target.is_some_and(|(to, _)| to == section);
            super::row()
                .h(28.0)
                .px(8.0)
                .center_y()
                .fill(if selected { p.coral_bg } else { p.selected })
                .on_drawn_layout_tx(move |s: &mut State, rect, tx| {
                    s.drop_rects.insert(key.clone(), rect);
                    tx.live_only();
                })
                .child(meta_label(&format!("{section:?}"), p.muted))
        }
        Node::Shelf {
            section,
            rows,
            index,
            inset,
        } => {
            let index = *index;
            let count = rows.len();
            let attention = (!s.shelves[index])
                .then(|| {
                    let entries: Vec<_> =
                        rows.iter().filter_map(|row| resolve(s, row, rel)).collect();
                    aggregate_attention(s, p, entries.iter(), rel)
                })
                .flatten();
            let title = format!("{section:?}");
            let key = format!("drop-{section:?}");
            col()
                .h(SHELF_HEADING)
                .fill(p.bg)
                .sticky_bottom(*inset)
                .child(
                    super::row()
                        .key(format!("shelf-{section:?}"))
                        .role("button")
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .label(format!("{title}, {count} threads"))
                        .h(SHELF_HEADING)
                        .pl(ROW_INSET)
                        .pr(4.0)
                        .radius(6.0)
                        .center_y()
                        .gap(6.0)
                        .hover(p.selected)
                        .on_drawn_layout_tx(move |s: &mut State, rect, tx| {
                            s.drop_rects.insert(key.clone(), rect);
                            tx.live_only();
                        })
                        .on_press(move |s: &mut State| {
                            s.shelves[index] = !s.shelves[index];
                            s.checkpoint();
                        })
                        .child(meta_label(&title, p.faint))
                        .child(spacer())
                        .child(attention.map_or_else(el, |(ink, word)| {
                            attention_dot(format!("shelf-attention-{section:?}"), ink, word)
                        }))
                        .child(label(count.to_string(), TIMER, p.faint))
                        .child(
                            icon(
                                if s.shelves[index] {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                p.faint,
                            )
                            .size(12.0, 12.0),
                        ),
                )
        }
    }
}

fn windowed(
    plan: &Arc<Plan>,
    relations: &Arc<Relations>,
    p: Palette,
    start: usize,
    end: usize,
    key: &str,
) -> El {
    let keys = plan.clone();
    let build = plan.clone();
    let relations = relations.clone();
    col()
        .key(key)
        .w_full()
        .gap(1.0)
        .growth(rune::GrowthSpec::default())
        .virtual_rows(
            end.saturating_sub(start),
            64.0,
            move |_: &State, index| keys.nodes[start + index].0.clone().into(),
            move |s: &State, index| {
                let (key, node) = &build.nodes[start + index];
                if let Node::Thread(row) = node {
                    let revision = row_revision(s, row, &relations);
                    rune::memo(key.as_str(), revision, || {
                        render_thread(s, p, row, &relations)
                    })
                } else {
                    render_node(s, p, node, &relations)
                }
            },
        )
        .virtual_rows_version(plan.version)
}

pub(super) fn content(s: &State, p: Palette) -> El {
    let (plan, relations) = cached(s);
    let scroll = s.sidebar_preview.scroll.clone();
    let watching_scroll = s.sidebar_preview.watching_scroll.clone();
    let wake = s.wake.clone();
    let mut body = col()
        .key("sidebar-content")
        .w_full()
        .gap(1.0)
        .pb(2.0)
        .grow(1.0);
    let mut first = 0;
    for (shelf, &index) in plan.shelves.iter().enumerate() {
        if index > first {
            body = body.child(windowed(
                &plan,
                &relations,
                p,
                first,
                index,
                if shelf == 0 {
                    "sidebar-rows"
                } else {
                    "sidebar-snoozed-rows"
                },
            ));
        }
        if shelf == 0 {
            body = body
                .child(spacer().min_h(12.0))
                .child(rule(p).mx(ROW_INSET));
        }
        let (key, node) = &plan.nodes[index];
        body = body.child(render_node(s, p, node, &relations).key(key));
        first = index + 1;
    }
    if first < plan.nodes.len() {
        body = body.child(windowed(
            &plan,
            &relations,
            p,
            first,
            plan.nodes.len(),
            if first == 0 {
                "sidebar-rows"
            } else {
                "sidebar-shelf-tail"
            },
        ));
    }
    if let Some(count) = plan.search_count {
        body = body.child(search_status(s, p));
        if count == 0 {
            body = body.child(search_empty(s, p));
        }
    }
    col()
        .key("sidebar-scroll")
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_drawn_layout_tx(|s: &mut State, rect, tx| {
            s.drop_rects.insert("sidebar-scroll".into(), rect);
            tx.live_only();
        })
        .on_scroll_offset(move |at| {
            let bits = at.offset[1].to_bits();
            if scroll.swap(bits, AtomicOrdering::Relaxed) != bits
                && watching_scroll.load(AtomicOrdering::Relaxed)
            {
                wake();
            }
        })
        .child(body)
}

fn row_revision(s: &State, row: &IndexedRow, rel: &Relations) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let id = row.id.thread;
    let (server, local) = (row.id.server.as_str(), row.id.server == s.machines.selected);
    row.id.hash(&mut hash);
    row.captain.hash(&mut hash);
    row.draft.hash(&mut hash);
    row.title_match.hash(&mut hash);
    rel.pull_requests.get(&row.id).hash(&mut hash);
    s.dark.hash(&mut hash);
    s.artwork.device_scale().to_bits().hash(&mut hash);
    s.portraits.motion_allowed().hash(&mut hash);
    s.sidebar_grouped().hash(&mut hash);
    sidebar_row_width(s).to_bits().hash(&mut hash);
    s.search.text.hash(&mut hash);
    s.show_archive.hash(&mut hash);
    let now = lifecycle::now();
    (now / 60_000).hash(&mut hash);
    s.machine_label(server).hash(&mut hash);
    multiple_machines(s).hash(&mut hash);
    if let Some(hit) = s
        .sidebar_search
        .visible_matches
        .get(&(server.to_owned(), id))
    {
        hit.snippet.hash(&mut hash);
    }
    if local && let Some(thread) = rel.local.get(&id).and_then(|&index| s.threads.get(index)) {
        thread.title.hash(&mut hash);
        thread.model.hash(&mut hash);
        thread.harness.hash(&mut hash);
        local_row_branch(s, thread).hash(&mut hash);
        row_diff(s, &row.id).hash(&mut hash);
        thread.updated.hash(&mut hash);
        thread.unread().hash(&mut hash);
        thread.life.woke(now).hash(&mut hash);
        let (state, approval) = local_status(s, thread, rel);
        (state as u8).hash(&mut hash);
        approval.hash(&mut hash);
        thread.request.is_some().hash(&mut hash);
        if state == Status::Running {
            thread
                .started
                .map(|at| at.elapsed().as_secs())
                .or(thread.elapsed)
                .hash(&mut hash);
        }
        crate::agents::active_count(thread).hash(&mut hash);
        (s.selected_row() == id || s.selected_threads.contains(&id)).hash(&mut hash);
        workspace_named_by_filter(s, server, thread.project).hash(&mut hash);
        if let Some(project) = s.project(thread.project) {
            project.name.hash(&mut hash);
            project.icon.hash(&mut hash);
        }
        (s.thread_section(thread, now) as u8).hash(&mut hash);
        if !row.captain && thread.machine == "local" {
            sidebar_preview::row_actions_open(s, &row.id).hash(&mut hash);
            s.lifecycle_busy(id).hash(&mut hash);
            s.row_pending(thread).hash(&mut hash);
            thread.queued().hash(&mut hash);
        }
        thread.pinned().hash(&mut hash);
        if row.draft {
            thread.draft.text.lines().next().hash(&mut hash);
        } else {
            thread.draft.text.is_empty().hash(&mut hash);
        }
        if let Some(captain) = thread.captain.filter(|_| row.captain) {
            if let Some(profile) = s.captains.get(captain) {
                profile.name.hash(&mut hash);
                profile.role.hash(&mut hash);
                profile.portrait.hash(&mut hash);
            }
            if let Some(tool) = thread.tools.last() {
                tool.detail.hash(&mut hash);
                tool.done.hash(&mut hash);
            }
            for child in rel
                .crew
                .get(&captain)
                .into_iter()
                .flatten()
                .map(|&index| &s.threads[index])
            {
                (child.status as u8).hash(&mut hash);
                child.title.hash(&mut hash);
                child.unread().hash(&mut hash);
                child.archived().hash(&mut hash);
                child.request.is_some().hash(&mut hash);
                child
                    .request
                    .as_ref()
                    .map(|request| request["method"].as_str())
                    .hash(&mut hash);
                child.life.settled().hash(&mut hash);
                if child.status == Status::Running {
                    child
                        .started
                        .map(|at| at.elapsed().as_secs())
                        .or(child.elapsed)
                        .hash(&mut hash);
                }
            }
            for task in s.tasks_for(id) {
                task.child_id.hash(&mut hash);
                task.state.id().hash(&mut hash);
            }
        }
    } else if let Some(projection) = s.machines.views.get(server) {
        row_diff(s, &row.id).hash(&mut hash);
        let record = remote_record(projection, rel, server, id);
        record.hash(&mut hash);
        projection["index"]["projects"].hash(&mut hash);
        if row.captain {
            projection["index"]["captains"].hash(&mut hash);
        }
        for thread in std::iter::once(id).chain(
            rel.remote_children
                .get(&row.id)
                .into_iter()
                .flatten()
                .copied(),
        ) {
            if let Some(view) = projection.views.get(&(thread as u64)) {
                for key in [
                    "status",
                    "request",
                    "elapsed",
                    "agentsActive",
                    "branch",
                    "model",
                    "harness",
                ] {
                    view.record[key].hash(&mut hash);
                }
            }
        }
        workspace_named_by_filter(
            s,
            server,
            record
                .and_then(|record| record["project"].as_u64())
                .map(|id| id as usize),
        )
        .hash(&mut hash);
        s.machines.ui[server]["drafts"][id.to_string()].hash(&mut hash);
    }
    hash.finish()
}

pub(super) fn focused_row(s: &State, key: &str) -> Option<RowId> {
    s.sidebar_index
        .borrow()
        .plan
        .navigation
        .iter()
        .find_map(|(server, id)| {
            let row = RowId::new(server, *id);
            (row.key(s) == key).then_some(row)
        })
}

pub(super) fn indexed(s: &State, id: &RowId) -> bool {
    s.sidebar_index.borrow().plan.targets.contains_key(id)
}

pub(super) fn preview_record<'a>(
    s: &'a State,
    id: &RowId,
) -> Option<(&'a Value, &'a crate::client_history::ThreadProjection)> {
    let projection = s.machines.views.get(&id.server)?;
    let rel = &s.sidebar_index.borrow().relations;
    let record = remote_record(projection, rel, &id.server, id.thread)?;
    let view = match projection.views.get(&(id.thread as u64)) {
        Some(view) => view.as_ref(),
        None => null_view(),
    };
    Some((record, view))
}
