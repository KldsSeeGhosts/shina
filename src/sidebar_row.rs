//! Shared sidebar layout for local and linked records.
use super::*;

pub(super) const CAPTAIN_HEIGHT: f32 = 56.0;
const COMPACT_HEIGHT: f32 = 36.0;
const CARD_TITLE_STYLE: TextStyle = TextStyle::new(14.0, 500.0).lh(20.0);
const QUIET_TITLE: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowStatus {
    Idle,
    Working,
    Approval,
    Input,
    Interrupted,
    Attention,
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
            Self::Approval => "Approval",
            Self::Input => "Input",
            Self::Interrupted => "Interrupted",
            Self::Attention => "Attention",
            Self::Failed => "Failed",
            Self::Done => "Done",
            Self::Woke => "Woke",
        }
    }

    fn ink(self, p: Palette) -> Color {
        match self {
            Self::Working => p.sea,
            Self::Input => p.input,
            // Mixed crew attention is Shina-only; KSG's fuchsia is for guest requests alone.
            Self::Approval | Self::Interrupted | Self::Attention | Self::Woke => p.brass,
            Self::Failed => p.danger,
            Self::Done => p.ok,
            Self::Idle => p.faint,
        }
    }

    // KSG's `SIDEBAR_STATUSES` glyphs: a 16 pt Lucide icon beside the status word, 12 pt on
    // slim rows.
    pub(super) fn mark(self, p: Palette, size: f32) -> El {
        let ink = self.ink(p);
        match self {
            Self::Idle => el(),
            Self::Working => icon("unresolved", ink).size(size, size),
            Self::Approval => icon("shield-question", ink).size(size, size),
            Self::Input => icon("message-question", ink).size(size, size),
            Self::Interrupted => icon("alert", ink).size(size, size),
            Self::Attention => icon("user-round", ink).size(size, size),
            Self::Failed => icon("alert", ink).size(size, size),
            Self::Done => icon("circle-check", ink).size(size, size),
            Self::Woke => icon("alarm-clock", ink).size(size, size),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CaptainFields {
    workspace: bool,
    machine: bool,
    crew: bool,
}

fn captain_fields(width: f32, has_request: bool) -> CaptainFields {
    CaptainFields {
        workspace: width >= 256.0,
        machine: width >= 216.0,
        crew: width >= 232.0 || has_request,
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
    pub pull_request: Option<u64>,
    pub diff: Option<(u32, u32)>,
    pub machine: Option<&'a str>,
    pub machine_glyph: &'static str,
    pub harness: Option<&'a str>,
    pub model: Option<&'a str>,
    pub agents_active: usize,
    pub badge: Option<El>,
    pub pinned: bool,
    pub unsent: bool,
    pub worktree: bool,
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
    pub attention_count: usize,
    pub unread: usize,
    pub selected: bool,
}

/// Ported from KSG Code's `SidebarThreadRow` card. Keep the workspace/status,
/// title, and branch/metadata on separate lines in the flat list.
pub(super) fn thread_row_el(
    s: &State,
    p: Palette,
    item: El,
    m: ThreadRowModel<'_>,
    actions: Option<(El, f32, bool)>,
) -> El {
    let id = m.id;
    let compact = (m.draft || m.shelved) && m.snippet.is_none();
    // KSG's `shouldRecedeSidebarThread`: a row that is neither open nor asking for
    // anything recedes, so the open thread and the ones that need the user stand out.
    let quiet =
        !m.selected && (m.shelved || matches!(m.status, RowStatus::Working | RowStatus::Idle));
    let details = if m.status == RowStatus::Idle {
        m.age.unwrap_or_default()
    } else if let Some(seconds) = m.elapsed.filter(|_| !compact) {
        format!("{} {}", m.status.word(), elapsed_label(seconds))
    } else {
        m.status.word().into()
    };
    let spoken = if let Some(elapsed) = m.elapsed {
        format!("Working for {}", timer_label(elapsed))
    } else if details.is_empty() {
        m.status.word().into()
    } else {
        details.clone()
    };
    let mut description = vec![m.title.to_owned(), spoken];
    if let Some(workspace) = m.workspace {
        description.push(workspace.to_owned());
    }
    if let Some(branch) = m.branch {
        description.push(format!("branch {branch}"));
    }
    if let Some(pr) = m.pull_request {
        description.push(format!("pull request {pr}"));
    }
    if let Some((added, removed)) = m.diff {
        description.push(format!("{added} additions, {removed} deletions"));
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
    let mut slot = row()
        .key(format!("thread-status-slot-{id}"))
        .h(if compact { 24.0 } else { 20.0 })
        .min_w(actions.as_ref().map_or(
            32.0,
            |(_, width, open)| {
                if *open { width.max(32.0) } else { 32.0 }
            },
        ))
        .center_y()
        .child(spacer());
    if !details.is_empty() {
        let key = if m.status == RowStatus::Idle {
            format!("thread-age-{id}")
        } else if m.status == RowStatus::Woke {
            format!("thread-woke-{id}")
        } else {
            format!("thread-state-{id}")
        };
        let mut status = row().center_y().gap(4.0);
        if m.status != RowStatus::Idle {
            status = status.child(m.status.mark(p, if compact { 12.0 } else { 16.0 }));
        }
        status = status.child(
            label(
                details,
                if m.status == RowStatus::Idle && !compact {
                    SMALL
                } else {
                    TIMER
                },
                m.status.ink(p),
            )
            .key(key),
        );
        if actions.is_some() && m.status != RowStatus::Woke {
            status = status.group_opacity(1.0, 0.0);
        }
        slot = slot.child(status);
    }
    let woke_actions = m.status == RowStatus::Woke && actions.is_some();
    if let Some((actions, width, open)) = actions {
        let actions = actions.group_opacity(0.0, 1.0);
        slot = if woke_actions && open {
            slot.child(el().w(width).child(actions))
        } else {
            slot.child(actions.pin(None, Some(if compact { 2.0 } else { 0.0 }), Some(0.0), None))
        };
    }
    // A receding title is regular weight at 75% and brightens on hover; the rest are medium
    // in the title colour.
    let mut title = label(
        m.title,
        if quiet { QUIET_TITLE } else { CARD_TITLE_STYLE },
        if quiet { p.recede } else { p.strong },
    )
    .key(format!("thread-row-title-{id}"))
    .truncate()
    .grow(1.0)
    .min_w(0.0);
    if quiet {
        title = title.group_color(p.strong);
    }
    // KSG's slim row dims the project favicon of every row but the open one
    // (`opacity-55`, `dark:opacity-40`) and restores it on hover.
    let badge = m.badge.map(|badge| {
        let badge = el().size(16.0, 16.0).center().child(badge);
        if m.selected || !compact {
            badge
        } else {
            badge.group_opacity(if s.dark { 0.4 } else { 0.55 }, 1.0)
        }
    });
    // KSG's `pinIndicator`: a 12 pt pin after the favicon on pinned rows.
    let pin = m.pinned.then(|| {
        icon("pin", p.faint)
            .size(12.0, 12.0)
            .key(format!("thread-pin-{id}"))
    });
    // KSG's `draftIndicator`: a 12 pt pen before the favicon on rows with unsent text.
    let pen = m.unsent.then(|| {
        icon("compose", p.brass)
            .size(12.0, 12.0)
            .role("img")
            .label("Unsent draft")
            .help("Unsent draft")
            .key(format!("thread-draft-pen-{id}"))
    });
    let machine = || {
        m.machine.map(|machine| {
            icon(m.machine_glyph, p.faint)
                .size(14.0, 14.0)
                .role("img")
                .label(machine)
                .help(machine)
                .key(format!("thread-machine-{id}"))
        })
    };
    let (height, content) = if compact {
        let mut line = row().h(COMPACT_HEIGHT).px(10.0).center_y().gap(10.0);
        if let Some(badge) = badge {
            line = line.child(badge);
        }
        if let Some(pen) = pen {
            line = line.child(pen);
        }
        line = line.child(title);
        if let Some(pin) = pin {
            line = line.child(pin);
        }
        if m.agents_active > 0 && sidebar_row_width(s) >= 232.0 {
            line = line.child(agents_ui::chip(
                &format!("thread-agents-{id}"),
                m.agents_active,
                p,
            ));
        }
        (COMPACT_HEIGHT, line.child(slot))
    } else {
        let scratch_machine = m.machine.filter(|_| m.workspace.is_none());
        let has_meta = m.branch.is_some()
            || m.pull_request.is_some()
            || m.diff.is_some()
            || m.agents_active > 0
            || m.snippet.is_some()
            || scratch_machine.is_some();
        // KSG's card header: draft pen, project favicon, pin, project name, status slot.
        let mut header = row().h(20.0).center_y().gap(6.0);
        if let Some(pen) = pen {
            header = header.child(pen);
        }
        if let Some(badge) = badge {
            header = header.child(badge);
        }
        if let Some(pin) = pin {
            header = header.child(pin);
        }
        header = header
            .child(
                label(m.workspace.unwrap_or_default(), SMALL, p.muted)
                    .key(format!("thread-workspace-{id}"))
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(slot);
        let mut title_line = row().h(20.0).mt(4.0).center_y().gap(6.0).child(title);
        let mut metadata = row().h(16.0).mt(2.0).gap(6.0).center_y().min_w(0.0);
        if let Some(snippet) = m.snippet {
            metadata = metadata.child(snippet.grow(1.0));
        } else if let Some(branch) = m.branch {
            // KSG's `ThreadWorktreeIndicator`: a folder-git glyph ahead of a worktree branch.
            if m.worktree {
                metadata = metadata.child(
                    icon("folder-git2", p.faint)
                        .size(14.0, 14.0)
                        .role("img")
                        .label("Worktree")
                        .help("Worktree"),
                );
            }
            metadata = metadata.child(
                label(branch, SMALL, p.muted)
                    .key(format!("thread-where-{id}"))
                    .truncate_middle()
                    .grow(1.0)
                    .min_w(0.0),
            );
        } else if let Some(machine) = scratch_machine {
            metadata = metadata.child(
                label(machine, SMALL, p.muted)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        } else {
            metadata = metadata.child(spacer());
        }
        if let Some(pr) = m.pull_request {
            metadata = metadata.child(
                row()
                    .key(format!("thread-pr-{id}"))
                    .center_y()
                    .gap(3.0)
                    .label(format!("Pull request {pr}"))
                    .help(format!("Pull request {pr}"))
                    .child(icon("pull-request", p.muted).size(13.0, 13.0))
                    .child(label(pr.to_string(), SMALL, p.muted)),
            );
        }
        if let Some((added, removed)) = m.diff {
            metadata = metadata.child(
                row()
                    .key(format!("thread-diff-{id}"))
                    .center_y()
                    .gap(4.0)
                    .child(label(
                        format!("+{}", diff_panel::format_compact_diff_count(added)),
                        MONO_META,
                        p.ok,
                    ))
                    .child(label(
                        format!("−{}", diff_panel::format_compact_diff_count(removed)),
                        MONO_META,
                        p.danger,
                    )),
            );
        }
        let mut identity = row().center_y().gap(4.0);
        if m.agents_active > 0 {
            identity = identity.child(agents_ui::chip(
                &format!("thread-agents-{id}"),
                m.agents_active,
                p,
            ));
        }
        if let Some(machine) = machine() {
            identity = identity.child(machine);
        }
        if let Some(harness) = m.harness {
            identity = identity.child(
                provider_icon(
                    s,
                    p,
                    model_vendor(harness, m.model.unwrap_or(harness)),
                    12.0,
                )
                .opacity(0.6)
                .key(format!("thread-harness-{id}")),
            );
        }
        if has_meta {
            metadata = metadata.child(identity);
        } else {
            title_line = title_line.child(identity);
        }
        let mut content = col().px(10.0).py(8.0).child(header).child(title_line);
        if has_meta {
            content = content.child(metadata);
        }
        (if has_meta { 78.0 } else { 60.0 }, content)
    };
    let item = item
        .h(height)
        .my(if compact { 0.0 } else { 2.0 })
        .hover_group_focus()
        .label(description.join(", "))
        .description("Preview available")
        .hover_motion(MOTION_FAST, MOTION_FAST);
    // KSG's `draftSurfaceClassName`: a faint warning wash under rows with unsent text.
    let item = if m.unsent && !m.selected {
        item.fill(p.brass.alpha(0.04)).hover(p.brass.alpha(0.08))
    } else {
        item
    };
    select_bar(item, m.selected, s.dark, p).child(content.grow(1.0).min_w(0.0))
}

pub(super) fn captain_row_el(s: &State, p: Palette, item: El, m: CaptainRowModel<'_>) -> El {
    let id = m.id;
    let width = sidebar_row_width(s);
    let fields = captain_fields(width, m.attention_count > 0);
    let mut description = format!("{}, {}, {}", m.name, m.status.word(), m.activity);
    if let Some(workspace) = m.workspace {
        description.push_str(&format!(", {workspace}"));
    }
    if m.crew > 0 {
        description.push_str(&format!(", {} crew", m.crew));
    }
    if m.attention_count > 0 {
        description.push_str(&format!(", {} need attention", m.attention_count));
    }
    if m.unread > 0 {
        description.push_str(&format!(", {} unread", m.unread));
    }
    if let Some(machine) = m.machine {
        description.push_str(&format!(", {machine}"));
    }
    let trailing = if m.attention_count > 0 {
        label(
            format!("{} {}", m.status.word(), m.attention_count),
            TIMER,
            m.status.ink(p),
        )
        .key(format!("captain-attention-{id}"))
    } else if let Some(timer) = m.timer {
        label(elapsed_label(timer), TIMER, p.muted).key(format!("captain-timer-{id}"))
    } else if matches!(
        m.status,
        RowStatus::Failed | RowStatus::Interrupted | RowStatus::Approval | RowStatus::Input
    ) {
        label(m.status.word(), META, m.status.ink(p)).key(format!("captain-attention-{id}"))
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
            let captain = captain_fields(width, false);
            assert_eq!(
                (captain.workspace, captain.crew, captain.machine),
                (workspace, crew, machine)
            );
            assert!(captain_fields(width, true).crew);
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
