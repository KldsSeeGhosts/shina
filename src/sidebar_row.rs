//! Shared sidebar layout for local and linked records.
use super::*;

pub(super) const THREAD_HEIGHT: f32 = 52.0;
pub(super) const CAPTAIN_HEIGHT: f32 = 56.0;
pub(super) const SLIM_HEIGHT: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowStatus {
    Idle,
    Working,
    Approval,
    Input,
    Failed,
    Done,
    Woke,
}

impl RowStatus {
    pub(super) fn of(status: Status, approval: bool, unread: bool, woke: bool) -> Self {
        match status {
            Status::Waiting if approval => Self::Approval,
            Status::Waiting => Self::Input,
            Status::Running => Self::Working,
            Status::Failed => Self::Failed,
            _ if woke => Self::Woke,
            _ if unread => Self::Done,
            _ => Self::Idle,
        }
    }

    pub(super) fn word(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Working => "Working",
            Self::Approval => "Needs approval",
            Self::Input => "Needs input",
            Self::Failed => "Failed",
            Self::Done => "Done",
            Self::Woke => "Woke",
        }
    }

    fn ink(self, p: Palette) -> Color {
        match self {
            Self::Working | Self::Woke => p.sea,
            Self::Approval | Self::Input => p.brass,
            Self::Failed => p.danger,
            Self::Done => p.ok,
            Self::Idle => p.faint,
        }
    }

    pub(super) fn mark(self, p: Palette) -> El {
        match self {
            Self::Idle => el(),
            Self::Working => status_mark(Status::Running, p, 8.0),
            Self::Approval | Self::Input => status_mark(Status::Waiting, p, 8.0),
            Self::Failed => status_mark(Status::Failed, p, 8.0),
            Self::Done => status_mark(Status::Completed, p, 8.0),
            Self::Woke => icon("clock", p.sea).size(12.0, 12.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RowFields {
    pub workspace: bool,
    pub branch: bool,
    pub harness: bool,
    pub machine: bool,
    pub crew: bool,
}

pub(super) fn fields_for_width(captain: bool, width: f32, has_request: bool) -> RowFields {
    RowFields {
        workspace: width >= if captain { 256.0 } else { 232.0 },
        branch: !captain && width >= 232.0,
        harness: !captain && width >= 256.0,
        machine: width >= 216.0,
        crew: captain && (width >= 232.0 || has_request),
    }
}

fn room_for_field(available: f32, reserved: f32, text: &str) -> bool {
    let mut chars = text.char_indices();
    let required = if let Some((end, _)) = chars.nth(8) {
        text_width(text).min(text_width(&text[..end]) + text_width("…"))
    } else {
        text_width(text)
    };
    !text.is_empty() && available - reserved >= required
}

pub(super) struct ThreadRowModel<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub status: RowStatus,
    pub elapsed: Option<u64>,
    pub age: Option<String>,
    pub workspace: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub machine: Option<&'a str>,
    pub harness: Option<&'a str>,
    pub model: Option<&'a str>,
    pub agents_active: usize,
    pub lead: El,
    pub selected: bool,
    pub shelved: bool,
    pub draft: bool,
    pub snippet: Option<El>,
}

pub(super) struct CaptainRowModel<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub portrait: El,
    pub status: RowStatus,
    pub timer: Option<u64>,
    pub activity: &'a str,
    pub workspace: Option<&'a str>,
    pub machine: Option<&'a str>,
    pub crew: usize,
    pub requests: usize,
    pub unread: usize,
    pub selected: bool,
}

/// KSG Code's `SidebarThreadRow` with Wave 3's fixed height and metadata line.
pub(super) fn thread_row_el(
    s: &State,
    p: Palette,
    item: El,
    m: ThreadRowModel<'_>,
    actions: Option<El>,
) -> El {
    let id = m.id;
    let width = sidebar_row_width(s);
    let fields = fields_for_width(false, width, false);
    let slim = (m.draft || m.shelved) && m.snippet.is_none();
    let highlighted = m.selected || m.status != RowStatus::Idle;
    let details = m.elapsed.map_or_else(
        || {
            if m.status == RowStatus::Idle {
                m.age.clone().unwrap_or_default()
            } else {
                m.status.word().into()
            }
        },
        elapsed_label,
    );
    let spoken = m.elapsed.map_or_else(
        || {
            if details.is_empty() {
                m.status.word().into()
            } else {
                details.clone()
            }
        },
        |elapsed| format!("Working for {}", timer_label(elapsed)),
    );
    let mut description = vec![m.title.to_owned(), spoken];
    if let Some(workspace) = m.workspace {
        description.push(workspace.to_owned());
    }
    if let Some(branch) = m.branch {
        description.push(format!("branch {branch}"));
    }
    if m.agents_active > 0 {
        description.push(format!("{} agents working", m.agents_active));
    }
    description.extend(
        [m.harness, m.model, m.machine]
            .into_iter()
            .flatten()
            .map(str::to_owned),
    );
    let description = description.join(", ");
    let state = if details.is_empty() {
        el()
    } else {
        label(&details, TIMER, m.status.ink(p)).key(if m.status == RowStatus::Idle || slim {
            format!("thread-age-{id}")
        } else if m.status == RowStatus::Woke {
            format!("thread-woke-{id}")
        } else {
            format!("thread-state-{id}")
        })
    };
    let title = label(
        m.title,
        if highlighted && !m.shelved {
            TITLE
        } else {
            ROW_QUIET
        },
        if m.shelved {
            p.faint
        } else if highlighted {
            p.ink
        } else {
            p.muted
        },
    )
    .key(format!("thread-row-title-{id}"))
    .truncate()
    .grow(1.0)
    .min_w(0.0);
    let lead = el().size(16.0, 16.0).center().child(m.lead);
    let mut line = row()
        .h(if slim { SLIM_HEIGHT } else { 20.0 })
        .center_y()
        .gap(8.0)
        .child(lead)
        .child(title);
    let showing_actions = actions.is_some();
    if let Some(actions) = actions {
        line = line.child(actions);
    }
    let mut content = col().grow(1.0).min_w(0.0).pl(ROW_INSET).pr(8.0);
    if slim {
        if !showing_actions {
            line = line.child(state);
        }
        content = content.child(line);
    } else {
        let agents = (m.agents_active > 0)
            .then(|| agents_ui::chip(&format!("thread-agents-{id}"), m.agents_active, p));
        let mut metadata = row().h(18.0).pl(24.0).gap(5.0).center_y().min_w(0.0);
        if let Some(snippet) = m.snippet {
            metadata = metadata.child(snippet.grow(1.0));
        } else {
            let reserved = 44.0
                + text_width(&details)
                + if agents.is_some() { 37.0 } else { 0.0 }
                + if fields.harness && m.harness.is_some() {
                    19.0
                } else {
                    0.0
                }
                + if fields.machine && m.machine.is_some() {
                    19.0
                } else {
                    0.0
                };
            metadata = metadata.child(state);
            let location = if s.sidebar_grouped() {
                m.branch.or(m.workspace)
            } else {
                m.workspace.or(m.branch)
            };
            if let Some(location) =
                location.filter(|text| fields.branch && room_for_field(width, reserved, text))
            {
                metadata = metadata
                    .child(el().size(3.0, 3.0).radius(1.5).fill(p.faint))
                    .child(
                        label(
                            location,
                            if Some(location) == m.branch {
                                MONO_META
                            } else {
                                SMALL
                            },
                            p.faint,
                        )
                        .key(format!("thread-where-{id}"))
                        .truncate_middle()
                        .shrink()
                        .min_w(0.0),
                    );
            }
            metadata = metadata.child(spacer());
            if let Some(agents) = agents {
                metadata = metadata.child(agents);
            }
            if let Some(harness) = m.harness.filter(|_| fields.harness) {
                metadata = metadata.child(
                    provider_icon(
                        s,
                        p,
                        model_vendor(harness, m.model.unwrap_or(harness)),
                        14.0,
                    )
                    .key(format!("thread-harness-{id}")),
                );
            }
            if let Some(machine) = m.machine.filter(|_| fields.machine) {
                metadata = metadata.child(
                    icon("monitor", p.faint)
                        .size(14.0, 14.0)
                        .help(machine)
                        .key(format!("thread-machine-{id}")),
                );
            }
        }
        content = content.pt(7.0).child(line).child(metadata);
    }
    select_bar(
        item.h(if slim { SLIM_HEIGHT } else { THREAD_HEIGHT })
            .label(&description)
            .description("Preview available")
            .hover_motion(MOTION_FAST, MOTION_FAST),
        m.selected,
        s.dark,
        p,
    )
    .child(content)
}

pub(super) fn captain_row_el(s: &State, p: Palette, item: El, m: CaptainRowModel<'_>) -> El {
    let id = m.id;
    let width = sidebar_row_width(s);
    let fields = fields_for_width(true, width, m.requests > 0);
    let mut description = format!("{}, {}, {}", m.name, m.status.word(), m.activity);
    if let Some(workspace) = m.workspace {
        description.push_str(&format!(", {workspace}"));
    }
    if m.crew > 0 {
        description.push_str(&format!(", {} crew", m.crew));
    }
    if m.requests > 0 {
        description.push_str(&format!(", {} requests", m.requests));
    }
    if m.unread > 0 {
        description.push_str(&format!(", {} unread", m.unread));
    }
    if let Some(machine) = m.machine {
        description.push_str(&format!(", {machine}"));
    }
    let trailing = if m.requests > 0 || matches!(m.status, RowStatus::Approval | RowStatus::Input) {
        count_pill(m.requests.max(1), CountKind::NeedsYou, p).key(format!("captain-attention-{id}"))
    } else if let Some(timer) = m.timer {
        label(elapsed_label(timer), TIMER, p.muted).key(format!("captain-timer-{id}"))
    } else if m.status == RowStatus::Failed {
        label("Failed", META, p.danger)
    } else if m.unread > 0 {
        icon("check", p.ok)
            .size(12.0, 12.0)
            .key(format!("captain-unread-{id}"))
    } else {
        el()
    };
    let mut line = row().h(20.0).center_y().gap(6.0).child(
        label(m.name, ROW_STRONG, p.ink)
            .key(format!("captain-name-{id}"))
            .truncate()
            .help_when_truncated(m.name)
            .shrink()
            .min_w(0.0),
    );
    if let Some(workspace) = m
        .workspace
        .filter(|text| fields.workspace && room_for_field(width, 158.0, text))
    {
        line = line.child(
            label(workspace, SMALL, p.faint)
                .key(format!("thread-workspace-{id}"))
                .truncate()
                .shrink()
                .min_w(0.0),
        );
    }
    line = line.child(spacer()).child(
        row()
            .key(format!("captain-status-{id}"))
            .center_y()
            .child(trailing),
    );
    let mut details = row().h(18.0).center_y().gap(6.0).child(
        label(m.activity, SMALL, p.muted)
            .key(format!("captain-activity-{id}"))
            .truncate()
            .grow(1.0)
            .min_w(0.0),
    );
    if fields.crew && m.crew > 0 {
        details = details.child(label(format!("{} crew", m.crew), SMALL, p.faint));
    }
    if let Some(machine) = m.machine.filter(|_| fields.machine) {
        details = details.child(
            icon("monitor", p.faint)
                .size(14.0, 14.0)
                .help(machine)
                .key(format!("captain-machine-{id}")),
        );
    }
    select_bar(
        item.h(CAPTAIN_HEIGHT)
            .label(&description)
            .help(description)
            .description("Preview available")
            .hover_motion(MOTION_FAST, MOTION_FAST),
        m.selected,
        s.dark,
        p,
    )
    .px(8.0)
    .gap(10.0)
    .center_y()
    .child(
        el().key(format!("portrait-slot-{id}"))
            .size(44.0, 44.0)
            .center()
            .child(m.portrait),
    )
    .child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .child(line)
            .child(details),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_drop_whole_at_the_three_breakpoints() {
        for (width, workspace, crew, machine) in [
            (256.0, true, true, true),
            (255.0, false, true, true),
            (232.0, false, true, true),
            (231.0, false, false, true),
            (216.0, false, false, true),
            (215.0, false, false, false),
        ] {
            let captain = fields_for_width(true, width, false);
            assert_eq!(
                (captain.workspace, captain.crew, captain.machine),
                (workspace, crew, machine)
            );
            assert!(fields_for_width(true, width, true).crew);
            let thread = fields_for_width(false, width, false);
            assert_eq!(
                (thread.harness, thread.branch, thread.machine),
                (workspace, crew, machine)
            );
        }
        assert!(!room_for_field(
            49.0,
            0.0,
            "eight letters before the ellipsis"
        ));
        assert!(room_for_field(
            57.0,
            0.0,
            "eight letters before the ellipsis"
        ));
        assert!(room_for_field(40.0, 0.0, "shina"));
    }
}
