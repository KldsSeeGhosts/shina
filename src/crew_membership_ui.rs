//! Crew membership pickers for the Brief: the crew row's More menu, the move picker that
//! offers the other captains, and the recruit palette over ordinary root threads. The menu
//! and picker hang off their anchors as KSG's `ModelPickerContent` does; the palette's
//! structure follows `add_project_palette` (KSG `CommandPalette`). Disabled choices carry
//! the real blocker reason.
use super::captain_ui::quiet_note;
use super::crew_ui::{hover_wash, motion};
use super::*;
use crate::model::CaptainAccess;
use crate::model::{CrewKind, MoveMode};

/// The crew row's More menu: Open thread, Move to another captain, Release. Disabled
/// entries keep their reason in a non-focusable line under them, and keyboard skips them.
pub(super) fn more_menu(s: &State, p: Palette, captain: usize, thread_id: usize) -> El {
    let mut menu = popover("brief-crew-menu", "Crew member", s, p)
        .role("menu")
        .focus_group()
        .w(260.0);
    let Some(index) = s.threads.iter().position(|t| t.id == thread_id) else {
        return menu;
    };
    let access = s.captain_access(captain);
    let idle = s.crew_idle(index).err();
    let reason = access_reason(access).or(idle);
    let expected = s.machines.selected.clone();
    let open_server = expected.clone();
    let move_server = expected.clone();
    menu = menu.child(menu_row(
        s,
        p,
        "crew-menu-open",
        "Open thread",
        None,
        move |s: &mut State| {
            if s.machines.selected != open_server {
                return;
            }
            if let Some(index) = s.threads.iter().position(|t| t.id == thread_id) {
                s.brief.more = None;
                s.show_worker_ghost(index);
            }
        },
    ));
    let move_reason = reason.clone();
    menu = menu
        .child(menu_row(
            s,
            p,
            "crew-menu-move",
            "Move to another captain...",
            move_reason.as_deref(),
            move |s: &mut State| {
                if s.machines.selected != move_server
                    || s.captain_access(captain) != CaptainAccess::Full
                    || s.threads
                        .iter()
                        .position(|t| t.id == thread_id)
                        .is_none_or(|index| s.crew_idle(index).is_err())
                {
                    return;
                }
                s.brief.more = None;
                s.brief.moving = Some(crate::model::MovePicker {
                    server: move_server.clone(),
                    thread: thread_id,
                    mode: MoveMode::Move { from: captain },
                    anchor: format!("crew-chip-{thread_id}"),
                });
            },
        ))
        .child(menu_row(
            s,
            p,
            "crew-menu-release",
            "Release from crew",
            reason.as_deref(),
            move |s: &mut State| {
                if s.captain_access(captain) != CaptainAccess::Full
                    || s.threads
                        .iter()
                        .position(|t| t.id == thread_id)
                        .is_none_or(|index| s.crew_idle(index).is_err())
                {
                    return;
                }
                let result =
                    s.captain_crew_command(&expected, captain, thread_id, CrewKind::Release);
                report_mutation(s, result);
                s.brief.more = None;
            },
        ));
    if let Some(reason) = reason {
        menu = menu.child(
            label(reason, META_MENU, p.muted)
                .key("crew-menu-reason")
                .wrap()
                .px(8.0)
                .pb(4.0),
        );
    }
    menu
}

/// One menu row: disabled rows keep their reason as the accessible label so the keyboard
/// and pointer skip over them but the reason still reads out.
fn menu_row(
    s: &State,
    p: Palette,
    key: &str,
    title: &str,
    reason: Option<&str>,
    action: impl Fn(&mut State) + 'static,
) -> El {
    let disabled = reason.is_some();
    let mut row_ = row()
        .key(key)
        .role("menuitem")
        .cursor(if disabled {
            rune::Cursor::Default
        } else {
            rune::Cursor::Pointer
        })
        .label(match reason {
            Some(reason) => format!("{title}, {reason}"),
            None => title.to_string(),
        })
        .h(28.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .disabled(disabled)
        .child(
            label(
                title,
                TextStyle::new(13.0, 500.0),
                if disabled { p.faint } else { p.ink },
            )
            .grow(1.0)
            .min_w(0.0)
            .truncate(),
        );
    if reason.is_none() {
        row_ = row_
            .focusable(true)
            .focus_highlight(hover_wash(s, p))
            .hover(hover_wash(s, p))
            .hover_motion(motion(), motion())
            .on_press(action);
    }
    row_
}

const META_MENU: TextStyle = TextStyle::new(12.0, 500.0);

/// Report a crew command's outcome on the Brief's error line; a pending id needs no banner.
fn report_mutation(s: &mut State, result: Result<crate::model::CrewMutation, String>) {
    match result {
        Ok(_) => s.brief.error = None,
        Err(error) => s.brief.error = Some(error),
    }
}

/// The move picker: the other captains with their portraits, in the row structure of
/// `captain_picker`. In Join mode every captain is a target; in Move mode the current
/// owner drops out. Disabled rows carry the scope/idle reason.
pub(super) fn move_picker(s: &State, p: Palette) -> Option<El> {
    let picker = s.brief.moving.as_ref()?;
    if picker.server != s.machines.selected {
        return None;
    }
    let thread_id = picker.thread;
    let expected = picker.server.clone();
    let thread_title = s
        .threads
        .iter()
        .find(|t| t.id == thread_id)
        .map(|t| {
            if t.title.is_empty() {
                "Untitled".to_string()
            } else {
                t.title.clone()
            }
        })
        .unwrap_or_else(|| "Untitled".to_string());
    let heading = match picker.mode {
        MoveMode::Move { .. } => format!("Move {thread_title} to"),
        MoveMode::Join => format!("Add {thread_title} to"),
    };
    let mut menu = popover("crew-move-picker", &heading, s, p)
        .role("menu")
        .focus_group()
        .w(280.0)
        .child(label(heading, META_LABEL, p.muted).px(8.0).pt(4.0).pb(2.0));
    let mut any = false;
    for target in s.picker_captains() {
        if let MoveMode::Move { from } = picker.mode
            && target == from
        {
            continue;
        }
        any = true;
        // The source's access decides whether the user may act at all; each target row's
        // own eligibility comes from the scope/idle checks.
        let source = match picker.mode {
            MoveMode::Move { from } => Some(from),
            MoveMode::Join => Some(target),
        };
        let reason = s
            .threads
            .iter()
            .position(|t| t.id == thread_id)
            .map(|index| {
                source
                    .and_then(|source| access_reason(s.captain_access(source)))
                    .or_else(|| s.move_blocker(target, index).err())
            })
            .flatten();
        let status = s.captain_status(target);
        let name = s.captains[target].name.clone();
        let mode = picker.mode;
        let mut row_ = row()
            .key(format!("crew-move-{target}"))
            .role("menuitem")
            .label(match &reason {
                Some(reason) => format!("{name}, {reason}"),
                None => name.clone(),
            })
            .cursor(if reason.is_some() {
                rune::Cursor::Default
            } else {
                rune::Cursor::Pointer
            })
            .min_h(40.0)
            .px(8.0)
            .gap(10.0)
            .center_y()
            .radius(6.0)
            .disabled(reason.is_some())
            .child(avatar(s, p, target, 40.0, status))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        label(
                            &name,
                            TextStyle::new(13.0, 500.0),
                            if reason.is_some() { p.faint } else { p.ink },
                        )
                        .truncate(),
                    )
                    .children(
                        reason
                            .iter()
                            .map(|reason| label(reason, META_MENU, p.faint).wrap()),
                    ),
            );
        if reason.is_none() {
            let expected = expected.clone();
            row_ = row_
                .focusable(true)
                .focus_highlight(hover_wash(s, p))
                .hover(hover_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| {
                    let (command_captain, kind) = match mode {
                        MoveMode::Move { from } => (from, CrewKind::Move { to: target }),
                        MoveMode::Join => (target, CrewKind::Join),
                    };
                    let result =
                        s.captain_crew_command(&expected, command_captain, thread_id, kind);
                    report_mutation(s, result);
                    s.brief.moving = None;
                });
        }
        menu = menu.child(row_);
    }
    if !any {
        menu = menu.child(quiet_note("No other captains.", p));
    }
    Some(
        menu.anchored_fit(
            picker.anchor.clone(),
            rune::Side::Bottom,
            rune::Align::End,
            4.0,
        )
        .modal(true, |s: &mut State| s.brief.moving = None),
    )
}

/// Structurally ineligible candidates never appear in the palette at all: captain homes,
/// subagents, remote workers, members of any crew, archived threads and unsent drafts.
fn structurally_excluded(thread: &crate::model::Thread) -> bool {
    thread.captain_home
        || thread.delegated
        || thread.peer_origin.is_some()
        || thread.crew_captain.is_some()
        || thread.archived()
        || thread.is_draft()
}

/// The recruit palette over the Brief's Recruit button: a search field, the eligible
/// roots under "Can join", then the unavailable rows with their real blocker under
/// "Can't join yet". Keyboard follows the focus group: arrows skip disabled rows.
pub(super) fn recruit_palette(s: &State, p: Palette, captain: usize) -> Option<El> {
    let picker = s.brief.recruit.as_ref()?;
    if picker.server != s.machines.selected || picker.captain != captain {
        return None;
    }
    let expected = picker.server.clone();
    let expected_submit = expected.clone();
    let captain_name = s
        .captains
        .get(captain)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let query = picker.query.text.to_lowercase();
    let mut can: Vec<(usize, String)> = Vec::new();
    let mut cannot: Vec<(usize, String, String)> = Vec::new();
    for (index, thread) in s.threads.iter().enumerate() {
        if structurally_excluded(thread) {
            continue;
        }
        let title = if thread.title.is_empty() {
            "Untitled".to_string()
        } else {
            thread.title.clone()
        };
        if !query.is_empty() && !title.to_lowercase().contains(&query) {
            continue;
        }
        match s.join_blocker(captain, index) {
            Ok(()) => can.push((index, title)),
            Err(reason) => {
                // A thread in another captain's crew names its owner instead of the
                // generic membership text.
                let reason = if thread.crew_captain.is_some_and(|owner| owner != captain) {
                    let owner = thread.crew_captain.unwrap();
                    let name = s
                        .captains
                        .get(owner)
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| "another captain".into());
                    format!("In {name}'s crew. Move it from there.")
                } else {
                    reason
                };
                cannot.push((index, title, reason));
            }
        }
    }
    let menu = popover(
        "crew-recruit-palette",
        &format!("Recruit to {captain_name}'s crew"),
        s,
        p,
    )
    .role("dialog")
    .label(format!("Recruit to {captain_name}'s crew"))
    .focus_group()
    .w(300.0)
    .max_w(s.viewport_width - 16.0)
    .child(
        label(
            format!("Recruit to {captain_name}'s crew"),
            DIALOG_TITLE,
            p.ink,
        )
        .px(8.0)
        .pt(4.0),
    )
    .child(
        row()
            .key("crew-recruit-search")
            .mx(4.0)
            .mt(6.0)
            .h(28.0)
            .px(8.0)
            .radius(6.0)
            .fill(field_fill(s, p))
            .child({
                let picker_focus = "crew-recruit-search".to_string();
                text_input(&picker.query, TextStyle::new(13.0, 400.0))
                    .placeholder("Search threads", p.faint)
                    .color(p.ink)
                    .w_full()
                    .center_y()
                    .on_edit(rune::edit_access(move |s: &mut State| {
                        &mut s.brief.recruit.as_mut().expect("recruit palette").query
                    }))
                    .on_input_key(move |s: &mut State, event, tx| {
                        // Down leaves the search field for the first enabled row.
                        if matches!(event.key, rune::Key::Down) {
                            let query = s
                                .brief
                                .recruit
                                .as_ref()
                                .map(|picker| picker.query.text.to_lowercase())
                                .unwrap_or_default();
                            let first = s
                                .threads
                                .iter()
                                .enumerate()
                                .filter(|(_, t)| !structurally_excluded(t))
                                .filter(|(_, t)| {
                                    query.is_empty()
                                        || (if t.title.is_empty() {
                                            "Untitled"
                                        } else {
                                            t.title.as_str()
                                        })
                                        .to_lowercase()
                                        .contains(&query)
                                })
                                .find(|(index, _)| s.join_blocker(captain, *index).is_ok())
                                .map(|(_, t)| format!("crew-recruit-{}", t.id));
                            if let Some(row) = first {
                                if let Some(picker) = s.brief.recruit.as_mut() {
                                    picker.query.focused = false;
                                }
                                tx.focus(&row, true);
                            }
                            return true;
                        }
                        false
                    })
                    .on_submit(move |s: &mut State, tx| {
                        let expected = expected_submit.clone();
                        tx.focus(&picker_focus, false);
                        // Return recruits the first enabled row of the filtered list,
                        // matching the same title rule the visible rows use.
                        let query = s
                            .brief
                            .recruit
                            .as_ref()
                            .map(|picker| picker.query.text.to_lowercase())
                            .unwrap_or_default();
                        let Some((index, _)) = s
                            .threads
                            .iter()
                            .enumerate()
                            .filter(|(_, t)| !structurally_excluded(t))
                            .filter(|(_, t)| {
                                query.is_empty()
                                    || (if t.title.is_empty() {
                                        "Untitled"
                                    } else {
                                        t.title.as_str()
                                    })
                                    .to_lowercase()
                                    .contains(&query)
                            })
                            .find(|(index, _)| s.join_blocker(captain, *index).is_ok())
                        else {
                            return;
                        };
                        let id = s.threads[index].id;
                        let result = s.captain_crew_command(&expected, captain, id, CrewKind::Join);
                        report_mutation(s, result);
                        s.brief.recruit = None;
                        tx.focus("crew-roster-recruit", true);
                    })
            }),
    );
    let mut list = col().key("crew-recruit-list").pt(6.0).gap(2.0);
    if can.is_empty() && cannot.is_empty() {
        list = list.child(
            col()
                .key("crew-recruit-empty")
                .center()
                .py(16.0)
                .gap(8.0)
                .child(if query.is_empty() {
                    quiet_note(
                        "No threads can join yet. Start a thread in one of this captain's workspaces, then recruit it.",
                        p,
                    )
                } else {
                    quiet_note("No threads match.", p)
                }),
        );
    }
    if !can.is_empty() {
        list = list.child(
            label("Can join", META_LABEL, p.muted)
                .key("crew-recruit-can")
                .px(8.0)
                .pb(2.0),
        );
        for (index, title) in can {
            let expected = expected.clone();
            let thread = &s.threads[index];
            let id = thread.id;
            let workspace = s
                .project(thread.project)
                .map(|project| project.name.clone())
                .unwrap_or_default();
            let age = relative_time(thread.updated).unwrap_or_default();
            list = list.child(
                row()
                    .key(format!("crew-recruit-{id}"))
                    .role("option")
                    .label(format!("{title}, {workspace}"))
                    .cursor(rune::Cursor::Pointer)
                    .h(28.0)
                    .px(8.0)
                    .gap(8.0)
                    .center_y()
                    .radius(6.0)
                    .focusable(true)
                    .focus_highlight(hover_wash(s, p))
                    .hover(hover_wash(s, p))
                    .hover_motion(motion(), motion())
                    .on_press(move |s: &mut State| {
                        if s.captain_access(captain) != CaptainAccess::Full {
                            return;
                        }
                        let Some(index) = s.threads.iter().position(|t| t.id == id) else {
                            return;
                        };
                        if s.join_blocker(captain, index).is_err() {
                            return;
                        }
                        let result = s.captain_crew_command(&expected, captain, id, CrewKind::Join);
                        report_mutation(s, result);
                        s.brief.recruit = None;
                    })
                    .child(icon("thread", p.muted))
                    .child(
                        label(title, TextStyle::new(13.0, 500.0), p.ink)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(label(workspace, META_MENU, p.muted).truncate().shrink())
                    .child(label(age, META_MENU, p.faint)),
            );
        }
    }
    if !cannot.is_empty() {
        list = list.child(
            label("Can't join yet", META_LABEL, p.muted)
                .key("crew-recruit-cannot")
                .px(8.0)
                .pt(6.0)
                .pb(2.0),
        );
        for (index, title, reason) in cannot {
            let thread = &s.threads[index];
            let id = thread.id;
            let workspace = s
                .project(thread.project)
                .map(|project| project.name.clone())
                .unwrap_or_default();
            list = list.child(
                row()
                    .key(format!("crew-recruit-{id}"))
                    .role("option")
                    .label(format!("{title}, {reason}"))
                    .cursor(rune::Cursor::Default)
                    .px(8.0)
                    .py(4.0)
                    .gap(8.0)
                    .center_y()
                    .radius(6.0)
                    .disabled(true)
                    .child(icon("thread", p.faint))
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .child(label(title, TextStyle::new(13.0, 500.0), p.faint).truncate())
                            .child(
                                label(reason, META_MENU, p.faint)
                                    .key(format!("crew-recruit-reason-{id}"))
                                    .wrap(),
                            ),
                    )
                    .child(label(workspace, META_MENU, p.faint).truncate().shrink()),
            );
        }
    }
    Some(
        menu.child(
            col()
                .max_h(s.viewport_height * 0.6)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
        .anchored_fit(
            "crew-roster-recruit",
            rune::Side::Bottom,
            rune::Align::End,
            4.0,
        )
        .modal(true, |s: &mut State| s.brief.recruit = None),
    )
}

pub(super) fn access_reason(access: CaptainAccess) -> Option<String> {
    match access {
        CaptainAccess::Full => None,
        CaptainAccess::Earlier => Some("Read only: an earlier conversation is open".into()),
        CaptainAccess::UpdateRequired => Some("Update the linked Shina".into()),
        CaptainAccess::Unreachable => Some("The machine is unreachable".into()),
        CaptainAccess::NoRecords => Some("No captain records".into()),
    }
}
