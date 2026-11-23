use super::*;

pub(super) const TAB_GAP: f32 = 4.0;
/// A tab is 28 pt tall inside the 40 pt bar's 36 pt row, the spec's opt-in strip.
pub(super) const TAB_HEIGHT: f32 = 28.0;
/// Below this a tab drops its leading icon and keeps the title.
pub(super) const TAB_PREFIX_WIDTH: f32 = 170.0;
/// The narrowest a tab gets before the strip scrolls.
pub(super) const TAB_MIN_WIDTH: f32 = 120.0;
/// Tabs are packed from the left and stop growing here, so a few tabs do not sit in wide empty
/// slots.
pub(super) const TAB_MAX_WIDTH: f32 = 200.0;
/// Inter 13/500, the title in a tab.
pub(super) const TAB_TITLE: TextStyle = TextStyle::new(13.0, 500.0);
/// The 16 px leading icon in a tab, KSG `PaneTabs`' `ProjectFavicon`. Captain homes and
/// captain workers show the captain's face when the profile exists locally, an explicit
/// remote-delegation badge for peer workers (or missing captain profiles), and the project
/// tile otherwise.
pub(super) fn tab_icon(s: &State, p: Palette, thread: &Thread) -> El {
    let captain = if s.is_captain_home(thread) {
        thread.captain
    } else {
        thread.crew_captain
    };
    if let Some(captain) = captain {
        if s.captains.get(captain).is_some() {
            return avatar(s, p, captain, 16.0, s.captain_status(captain)).a11y_hidden();
        }
        return el()
            .key(format!("tab-icon-crew-{}", thread.id))
            .size(16.0, 16.0)
            .radius(4.0)
            .border(1.0, p.frame)
            .fill(p.selected)
            .center()
            .a11y_hidden()
            .child(icon("crew", p.muted).size(12.0, 12.0));
    }
    if thread.peer_origin.is_some() {
        return el()
            .key(format!("tab-icon-peer-{}", thread.id))
            .size(16.0, 16.0)
            .radius(4.0)
            .border(1.0, p.frame)
            .fill(p.selected)
            .center()
            .a11y_hidden()
            .child(icon("crew", p.muted).size(12.0, 12.0));
    }
    project_tile(s, p, s.project(thread.project), 16.0)
}
/// A tab's look, as KSG's chat tabs: the active tab is a `card` with a hairline and 8 px
/// corners. Others have no fill.
pub(super) fn tab_frame(tab: El, active: bool, p: Palette) -> El {
    if !active {
        // The hover block matches the active tab's 8 px corners, so a tab's fill
        // never shifts shape as it activates.
        return tab.hover(p.selected).radius(8.0);
    }
    tab.fill(p.card).radius(8.0).border(1.0, p.line)
}

/// The new-tab plus: a framed 24 px square.
pub(super) fn new_tab_button(p: Palette) -> El {
    framed(
        icon_button("new-tab", "New thread", "plus", p, |_| {})
            .fill(p.card)
            .on_press_tx(|s: &mut State, tx| s.new_thread_click(tx.mods().shift)),
        p,
    )
    .mr(2.0)
}

pub(super) fn header(s: &State, p: Palette, _narrow: bool, tabs_width: f32) -> El {
    let mut tabs = row()
        .key("thread-tabs")
        .min_w(0.0)
        .grow(1.0)
        .h(36.0)
        .center_y()
        .gap(TAB_GAP)
        .scroll_x()
        // Tabs hidden past an edge fade out, so a clipped strip reads as scrollable.
        .scroll_edge_fade(12.0);
    // Tabs share the strip evenly, down to a minimum. Only once every tab is at the minimum
    // does the strip scroll.
    let count = s
        .tabs
        .iter()
        .filter(|&&id| s.threads.iter().any(|t| t.id == id))
        .count()
        .max(1) as f32;
    let tab_width = ((tabs_width + TAB_GAP) / count - TAB_GAP).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
    for &id in &s.tabs {
        let Some(index) = s.threads.iter().position(|t| t.id == id) else {
            continue;
        };
        let thread = &s.threads[index];
        let active = index == s.active;
        // KSG draws the ghost tab's title in italics once kept tabs sit
        // beside it; a lone tab never reads as provisional.
        let ghost = s.is_ghost_tab(id) && s.tabs.len() > 1;
        let project = s
            .project(thread.project)
            .map(|p| p.name.as_str())
            .unwrap_or("No workspace");
        let mut tab = tab_frame(
            row()
                // The pressable keeps `tab-{id}` whether focused or not: Rune
                // pairs a double click by element path, so a key that changed
                // on focus would swallow the second click before `keep_tab`.
                .key(format!("tab-{id}"))
                .role("tab")
                .cursor(rune::Cursor::Pointer)
                .label(format!("{project} / {}", thread.title))
                .focusable(true)
                .center_y()
                .gap(8.0)
                .pl(8.0)
                .pr(if s.tabs.len() > 1 { 2.0 } else { 8.0 })
                .h(TAB_HEIGHT)
                .radius(8.0)
                .w(tab_width)
                .min_w(tab_width)
                .hover_group()
                .pointer_buttons([rune::PointerButton::Middle])
                .on_press_tx(move |s: &mut State, tx| {
                    // KSG: a middle click closes the tab.
                    if tx.button() == rune::PointerButton::Middle {
                        s.close_tab(id);
                        return;
                    }
                    s.show_thread(index);
                    tx.scroll_into_view("active-tab");
                })
                .on_double_press(move |s: &mut State| s.keep_tab(id))
                .on_secondary_press(move |s: &mut State, at, _| {
                    s.tab_menu = Some(id);
                    s.tab_menu_at = Some((at.x, at.y));
                }),
            active,
            p,
        )
        .child(if tab_width >= TAB_PREFIX_WIDTH {
            // KSG's `PaneTabs` leads with the project favicon; captain homes and
            // their workers show the captain's face instead.
            tab_icon(s, p, thread)
        } else {
            // A tab at its minimum gives the width to the title.
            el()
        })
        .child(
            // The active marker covers the tab exactly (`absolute` measures
            // from the same corner the border box does): tests and
            // `scroll_into_view` find `active-tab` here while the pressable
            // above keeps a stable `tab-{id}` key so the second click of a
            // double-click still pairs with the first.
            el().key(if active {
                "active-tab".into()
            } else {
                format!("tab-marker-{id}")
            })
            .a11y_hidden()
            .absolute(0.0, 0.0)
            .w(tab_width)
            .h(TAB_HEIGHT),
        )
        .child(
            label(
                &thread.title,
                if ghost { TAB_TITLE.italic() } else { TAB_TITLE },
                p.ink,
            )
            .truncate()
            .grow(1.0),
        );
        if s.tabs.len() > 1 {
            // The close button shows on the active tab and on hover.
            let close = el()
                .key(format!("close-tab-{id}"))
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label("Close tab, keep thread running")
                .focusable(true)
                .size(22.0, 22.0)
                .center()
                .radius(6.0)
                .hover(p.selected)
                .press(p.selected)
                .on_press(move |s: &mut State| s.close_tab(id))
                .child(tool_icon("close", p.muted));
            tab = tab.child(if active {
                close
            } else {
                close.group_opacity(0.0, 1.0)
            });
        }
        tabs = tabs.child(tab);
    }
    row()
        .h(36.0)
        .window_drag()
        .center_y()
        .gap(6.0)
        .child(tabs)
        .child(new_tab_button(p))
        .child(reveal_active_tab(s, tab_width, tabs_width))
}

/// Scrolls the strip so the active tab shows whole, each time the selection, the tab list or
/// the tab width changes. This is a 1 pt marker outside the strip whose key names that state.
/// A new key is a new element, and it reports once when it first shows, which runs the scroll.
/// (`on_visible` on the tab itself only fires once half the tab shows, too late for a tab
/// that starts off screen.)
pub(super) fn reveal_active_tab(s: &State, tab_width: f32, tabs_width: f32) -> El {
    use std::hash::{Hash, Hasher};
    let mut signature = std::collections::hash_map::DefaultHasher::new();
    s.tabs.hash(&mut signature);
    s.threads.get(s.active).map(|t| t.id).hash(&mut signature);
    (tab_width.round() as i32).hash(&mut signature);
    // Tabs stop shrinking at their minimum, so the strip's own width must change the key too.
    (tabs_width.round() as i32).hash(&mut signature);
    el().key(format!("tab-reveal-{:x}", signature.finish()))
        .absolute(0.0, 0.0)
        .size(1.0, 1.0)
        .a11y_hidden()
        .on_visible(|_: &mut State, visible, tx| {
            if visible {
                tx.instant();
                tx.scroll_into_view("active-tab");
            }
        })
}

/// Ported from KSG Code's thread tab context menu on the `row_menu` pattern: a
/// ghost tab offers "Keep tab" first, then the close group. Items that would do
/// nothing are disabled, as KSG does.
pub(super) fn thread_tab_menu(s: &State, p: Palette) -> El {
    let id = s.tab_menu.expect("tab menu only mounts with a tab");
    let index = s.tabs.iter().position(|tab| *tab == id);
    let last = s.tabs.len().saturating_sub(1);
    let anchor = if s.tab_menu_at.is_some() {
        "thread-tab-menu-point".to_string()
    } else {
        format!("tab-{id}")
    };
    enum TabMenu {
        Keep,
        Close,
        Others,
        ToRight,
        All,
    }
    let item = |key: &str, title: &str, glyph: &str, action: Option<TabMenu>| {
        let Some(action) = action else {
            return disabled_menu_item(key, title, glyph, p);
        };
        menu_item(key, title, glyph, p).on_press(move |s: &mut State| {
            s.tab_menu = None;
            match action {
                TabMenu::Keep => s.keep_tab(id),
                TabMenu::Close => s.close_tab(id),
                TabMenu::Others => s.close_other_tabs(id),
                TabMenu::ToRight => s.close_tabs_to_right(id),
                TabMenu::All => s.close_all_tabs(),
            }
        })
    };
    let keep = s.is_ghost_tab(id).then_some(TabMenu::Keep);
    let close = (s.tabs.len() > 1).then_some(TabMenu::Close);
    let others = (s.tabs.len() > 1).then_some(TabMenu::Others);
    let to_right = index.is_some_and(|i| i < last).then_some(TabMenu::ToRight);
    let all = (s.tabs.len() > 1).then_some(TabMenu::All);
    col()
        .key("thread-tab-menu")
        .role("menu")
        .focus_group()
        .label("Tab actions")
        .autofocus()
        .w(222.0)
        .padding(4.0)
        .gap(1.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .anchored_fit(anchor, rune::Side::Bottom, rune::Align::Start, 2.0)
        .modal(true, |s: &mut State| s.tab_menu = None)
        .child(item("menu-tab-keep", "Keep tab", "pin", keep))
        .child(item("menu-tab-close", "Close", "close", close))
        .child(item("menu-tab-others", "Close others", "close", others))
        .child(item(
            "menu-tab-right",
            "Close to the right",
            "chevron",
            to_right,
        ))
        .child(item("menu-tab-all", "Close all", "close", all))
}

/// Pin and archive, with their names, under the header's "More actions" button.
pub(super) fn thread_menu(s: &State, p: Palette) -> El {
    col()
        .key("thread-menu")
        .role("menu")
        .focus_group()
        .label("Thread actions")
        // Wide enough for "Auto-settle after" and its value on one row.
        .w(232.0)
        .padding(4.0)
        .gap(1.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .anchored_fit("thread-actions", rune::Side::Bottom, rune::Align::End, 4.0)
        .modal(true, |s: &mut State| s.thread_menu = false)
        .child(thread_actions(s, s.current().id, p, "thread-actions"))
}

pub(super) fn thread_actions(s: &State, id: usize, p: Palette, anchor: &str) -> El {
    let Some(thread) = s.threads.iter().find(|t| t.id == id) else {
        return el();
    };
    let captain = s.is_captain_home(thread);
    let mut actions = col()
        .gap(1.0)
        .shrink()
        .min_h(0.0)
        .max_h((s.viewport_height - 80.0).max(240.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if captain && id == s.current().id {
        // KSG's BranchToolbarEnvironmentSelector uses a machine row per
        // environment. Shina keeps the owner selected and sends one turn there.
        for (machine, project, label) in s.captain_continue_targets(id) {
            actions = actions.child(
                gated_menu_item(
                    &format!("captain-continue-{machine}-{project}"),
                    &format!("Continue on {label}"),
                    "monitor",
                    p,
                    s.continue_blocker(),
                )
                .on_press(move |s: &mut State| {
                    s.thread_menu = false;
                    s.row_menu = None;
                    s.continue_captain_draft(&machine, project);
                }),
            );
        }
    }
    if id == s.current().id && thread.pi.is_some() {
        actions = actions.child(
            menu_item("menu-refresh-commands", "Refresh Pi commands", "refresh", p).on_press(
                |s: &mut State| {
                    s.thread_menu = false;
                    s.row_menu = None;
                    s.current_mut().refresh_commands();
                },
            ),
        );
    }
    if id == s.current().id && thread.session.is_some() {
        actions = actions
            .child(
                gated_menu_item(
                    "menu-fork-conversation",
                    "Fork conversation",
                    "thread",
                    p,
                    s.conversation_blocker(thread),
                )
                .on_press(|s: &mut State| {
                    s.thread_menu = false;
                    s.row_menu = None;
                    s.fork_conversation(None);
                }),
            )
            .child(
                gated_menu_item(
                    "menu-fork-message",
                    "Fork from message...",
                    "thread",
                    p,
                    s.rewind_blocker(thread),
                )
                .on_press(|s: &mut State| {
                    s.thread_menu = false;
                    s.row_menu = None;
                    s.conversation_points(crate::model::ConversationMode::Fork);
                }),
            )
            .child(
                gated_menu_item(
                    "menu-rewind",
                    "Edit from here...",
                    "edit",
                    p,
                    s.rewind_blocker(thread),
                )
                .on_press(|s: &mut State| {
                    s.thread_menu = false;
                    s.row_menu = None;
                    s.conversation_points(crate::model::ConversationMode::Rewind);
                }),
            );
    }
    if !captain {
        actions = actions
            .child(
                gated_menu_item(
                    "menu-pin-thread",
                    if thread.pinned() {
                        "Unpin thread"
                    } else {
                        "Pin thread"
                    },
                    "pin",
                    p,
                    s.lifecycle_blocker(id),
                )
                .on_press(move |s: &mut State| {
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.pin_thread(id);
                }),
            )
            .child(
                gated_menu_item(
                    "menu-settle-thread",
                    if thread.life.settled() {
                        "Un-settle thread"
                    } else {
                        "Settle thread"
                    },
                    "check",
                    p,
                    s.lifecycle_blocker(id),
                )
                .on_press(move |s: &mut State| {
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.settle_thread(id);
                }),
            );
        if thread.life.snoozed(lifecycle::now()) {
            actions = actions.child(
                menu_item("menu-snooze-thread", "Wake thread", "clock", p).on_press(
                    move |s: &mut State| {
                        s.row_menu = None;
                        s.thread_menu = false;
                        s.unsnooze_thread(id);
                    },
                ),
            );
        } else {
            let snooze_anchor = anchor.to_owned();
            actions = actions.child(
                gated_menu_item(
                    "menu-snooze-thread",
                    "Snooze...",
                    "clock",
                    p,
                    s.snooze_blocker(id),
                )
                .on_press(move |s: &mut State| {
                    let at = if snooze_anchor == "thread-actions" {
                        None
                    } else {
                        s.row_menu_at
                    };
                    s.open_snooze_from(id, Some(snooze_anchor.clone()), at);
                }),
            );
        }
    }
    if !thread.captain_home
        && !thread.client_draft
        && !thread.delegated
        && thread.parent.is_none()
        && thread.peer_origin.is_none()
    {
        let server = s.machines.selected.clone();
        let reason = s.captain_promotion_unavailable(&server, id);
        let item = gated_menu_item(
            "menu-promote-captain",
            "Make this a captain...",
            "grid",
            p,
            reason,
        );
        actions = actions.child(item.on_press(move |s: &mut State| {
            s.row_menu = None;
            s.thread_menu = false;
            if let Err(error) = s.begin_captain_promotion(&server, id) {
                s.push_toast(
                    crate::model::ToastKind::Error,
                    "Could not make the thread a captain",
                    Some(error),
                    None,
                    Some(6000),
                );
            }
        }));
    }
    actions = actions
        .child(rule(p))
        .child(
            menu_item(
                "menu-rename-thread",
                if captain {
                    "Name this body of work"
                } else {
                    "Rename thread"
                },
                "edit",
                p,
            )
            .on_press(move |s: &mut State| s.begin_rename(id)),
        )
        .child(
            gated_menu_item(
                "menu-mark-unread",
                "Mark unread",
                "thread",
                p,
                s.unread_blocker(thread),
            )
            .on_press(move |s: &mut State| {
                if let Some(t) = s.threads.iter_mut().find(|t| t.id == id) {
                    t.life.mark_unread();
                }
                s.row_menu = None;
                s.thread_menu = false;
                s.checkpoint();
            }),
        );
    if !captain {
        let project = thread.project;
        actions = actions
            .child(
                menu_item(
                    "menu-filter-project",
                    if s.project_scope == ProjectScope::All {
                        "Filter by workspace"
                    } else {
                        "Show all workspaces"
                    },
                    "folder",
                    p,
                )
                .on_press(move |s: &mut State| {
                    s.project_scope = if s.project_scope != ProjectScope::All {
                        ProjectScope::All
                    } else {
                        project
                            .map(ProjectScope::Project)
                            .unwrap_or(ProjectScope::NoProject)
                    };
                    s.scope_additional.clear();
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.checkpoint();
                }),
            )
            .child(
                menu_item(
                    "menu-auto-settle",
                    if thread.life.auto_settle_disabled_at.is_some() {
                        "Enable auto-settle"
                    } else {
                        "Disable auto-settle"
                    },
                    "check",
                    p,
                )
                .on_press(move |s: &mut State| {
                    s.toggle_thread_auto_settle(id);
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.checkpoint();
                }),
            );
        actions = actions.child(
            menu_value_item(
                "menu-auto-settle-days",
                "Auto-settle after",
                &match s.auto_settle_days {
                    1 => "1 day".to_string(),
                    days => format!("{days} days"),
                },
                "clock",
                p,
            )
            .on_press(|s: &mut State| {
                s.cycle_auto_settle_days();
            }),
        );
        if !thread.draft.text.is_empty() {
            actions = actions.child(
                menu_item("menu-discard-draft", "Discard draft", "close", p).on_press(
                    move |s: &mut State| {
                        s.row_menu = None;
                        s.thread_menu = false;
                        s.discard_draft(id);
                    },
                ),
            );
        }
        // Crew membership on the thread's own machine: an idle member can move or leave,
        // an ordinary root can join through the same picker the Brief uses. Hidden where
        // membership cannot exist; disabled rows carry the real reason.
        if let Some(owner) = thread.crew_captain {
            let expected = s.machines.selected.clone();
            let move_server = expected.clone();
            let reason = crew_membership_ui::access_reason(s.captain_access(owner)).or_else(|| {
                s.threads
                    .iter()
                    .position(|t| t.id == id)
                    .and_then(|index| s.crew_idle(index).err())
            });
            let move_anchor = anchor.to_string();
            actions = actions
                .child(
                    menu_item("menu-crew-move", "Move to another captain...", "switch", p)
                        .label(match &reason {
                            Some(reason) => format!("Move to another captain, {reason}"),
                            None => "Move to another captain".to_string(),
                        })
                        .disabled(reason.is_some())
                        .on_press(move |s: &mut State| {
                            if s.machines.selected != move_server
                                || s.captain_access(owner) != crate::model::CaptainAccess::Full
                            {
                                return;
                            }
                            let at = if move_anchor == "thread-actions" {
                                None
                            } else {
                                s.row_menu_at
                            };
                            s.row_menu = None;
                            s.row_menu_at = None;
                            s.thread_menu = false;
                            s.brief.moving = Some(crate::model::MovePicker {
                                server: move_server.clone(),
                                thread: id,
                                mode: crate::model::MoveMode::Move { from: owner },
                                anchor: move_anchor.clone(),
                                at,
                            });
                        }),
                )
                .child(
                    menu_item("menu-crew-release", "Release from crew", "close", p)
                        .label(match &reason {
                            Some(reason) => format!("Release from crew, {reason}"),
                            None => "Release from crew".to_string(),
                        })
                        .disabled(reason.is_some())
                        .on_press(move |s: &mut State| {
                            s.row_menu = None;
                            s.thread_menu = false;
                            if let Err(error) = s.captain_crew_command(
                                &expected,
                                owner,
                                id,
                                crate::model::CrewKind::Release,
                            ) {
                                s.push_toast(
                                    crate::model::ToastKind::Error,
                                    "Could not release the thread",
                                    Some(error),
                                    None,
                                    Some(6000),
                                );
                            }
                        }),
                );
        } else if !s.captains.is_empty()
            && !thread.captain_home
            && thread.parent.is_none()
            && thread.peer_origin.is_none()
            && !thread.delegated
            && !thread.archived()
            && (0..s.captains.len()).any(|captain| {
                !matches!(
                    s.captain_access(captain),
                    crate::model::CaptainAccess::UpdateRequired
                        | crate::model::CaptainAccess::NoRecords
                )
            })
        {
            let reason = s
                .threads
                .iter()
                .position(|t| t.id == id)
                .and_then(|index| s.join_gate(index).err());
            let join_anchor = anchor.to_string();
            let join_server = s.machines.selected.clone();
            actions = actions.child(
                gated_menu_item(
                    "menu-crew-recruit",
                    "Add to a captain's crew...",
                    "switch",
                    p,
                    reason,
                )
                .on_press(move |s: &mut State| {
                    if s.machines.selected != join_server {
                        return;
                    }
                    let at = if join_anchor == "thread-actions" {
                        None
                    } else {
                        s.row_menu_at
                    };
                    s.row_menu = None;
                    s.row_menu_at = None;
                    s.thread_menu = false;
                    s.brief.moving = Some(crate::model::MovePicker {
                        server: join_server.clone(),
                        thread: id,
                        mode: crate::model::MoveMode::Join,
                        anchor: join_anchor.clone(),
                        at,
                    });
                }),
            );
        }
    }
    actions = actions
        .child(rule(p))
        .child(
            menu_item("menu-copy-thread-id", "Copy thread ID", "thread", p).on_press_tx(
                move |s: &mut State, tx| {
                    tx.copy(id.to_string());
                    s.row_menu = None;
                    s.thread_menu = false;
                },
            ),
        )
        .child(
            menu_item("menu-copy-path", "Copy path", "folder", p).on_press_tx(
                move |s: &mut State, tx| {
                    if let Some(i) = s.threads.iter().position(|t| t.id == id)
                        && let Ok(path) = s.thread_directory(i)
                    {
                        tx.copy(path.to_string_lossy().into_owned());
                    }
                    s.row_menu = None;
                    s.thread_menu = false;
                },
            ),
        );
    let primary_captain = thread
        .captain
        .filter(|&captain| thread.captain_home && s.captain_home(captain) == Some(id));
    if let Some(captain) = primary_captain {
        use crate::model::{CaptainLifecycleAction as Action, CaptainPhase};
        let archived = s.captain_phase(captain) == CaptainPhase::Archived;
        actions = actions.child(captain_lifecycle_menu_item(
            s,
            p,
            captain,
            if archived {
                Action::Unarchive
            } else {
                Action::Archive
            },
            if archived {
                "menu-unarchive-captain"
            } else {
                "menu-archive-captain"
            },
            if archived {
                "Unarchive captain"
            } else {
                "Archive captain..."
            },
            "archive",
            false,
        ));
        if !archived {
            actions = actions.child(captain_lifecycle_menu_item(
                s,
                p,
                captain,
                Action::Demote,
                "menu-demote-captain",
                "Make it an ordinary thread...",
                "thread",
                false,
            ));
        }
    } else {
        let grouped_archive = thread
            .crew_captain
            .filter(|&captain| s.captain_phase(captain) == crate::model::CaptainPhase::Archived);
        let reason = grouped_archive
            .map(|captain| {
                format!(
                    "Archived with {}. Unarchive the captain to restore it.",
                    s.captains[captain].name
                )
            })
            .or_else(|| s.lifecycle_blocker(id));
        let archive = gated_menu_item(
            "menu-archive-thread",
            if thread.archived() {
                "Restore thread"
            } else {
                "Archive thread"
            },
            "archive",
            p,
            reason,
        );
        actions = actions.child(archive.on_press(move |s: &mut State| {
            s.row_menu = None;
            s.thread_menu = false;
            s.archive_thread(id);
        }));
    }
    if s.is_captain_home(thread)
        && let Some(captain) = thread.captain
    {
        actions = actions.child(
            menu_item("menu-edit-captain", "Edit captain profile", "edit", p).on_press(
                move |s: &mut State| {
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.edit_captain(captain);
                },
            ),
        );
    }
    actions = actions.child(rule(p));
    if let Some(captain) = primary_captain {
        actions.child(captain_lifecycle_menu_item(
            s,
            p,
            captain,
            crate::model::CaptainLifecycleAction::Delete,
            "menu-delete-captain",
            "Delete captain...",
            "trash",
            true,
        ))
    } else {
        actions.child(
            danger_menu_item("menu-delete-thread", "Delete thread...", "trash", p)
                .on_press(move |s: &mut State| s.request_delete(id)),
        )
    }
}

/// Captain group actions have no KSG equivalent. Their menu rows reuse KSG's Sidebar
/// archive/delete entries and open the shared alert-dialog confirmation.
pub(super) fn captain_lifecycle_menu_item(
    s: &State,
    p: Palette,
    captain: usize,
    action: crate::model::CaptainLifecycleAction,
    key: &str,
    title: &str,
    glyph: &str,
    danger: bool,
) -> El {
    let server = s.machines.selected.clone();
    let reason = s.captain_lifecycle_unavailable(&server, captain, action);
    let item = if reason.is_some() {
        gated_menu_item(key, title, glyph, p, reason)
    } else if danger {
        danger_menu_item(key, title, glyph, p)
    } else {
        menu_item(key, title, glyph, p)
    };
    item.on_press(move |s: &mut State| {
        s.row_menu = None;
        s.thread_menu = false;
        if let Err(error) = s.open_captain_lifecycle(&server, captain, action) {
            s.push_toast(
                crate::model::ToastKind::Error,
                "Could not open the captain action",
                Some(error),
                None,
                Some(6000),
            );
        }
    })
}

pub(super) fn snooze_menu(s: &State, p: Palette, cx: &Cx<State>) -> Option<El> {
    let (id, choices) = s.snooze_menu.as_ref()?;
    let id = *id;
    let row_key = format!("thread-{id}");
    // Under Rune #1339 an anchored overlay whose anchor is not mounted is
    // suppressed visually while still holding modal focus. Use `thread-actions`
    // for header launches, a mounted row or captured pointer point for sidebar
    // launches, and never mount a modal without a live anchor.
    let (anchor, side, align) = if s.snooze_anchor.as_deref() == Some("thread-actions")
        && cx.layout_with("thread-actions", |_| ()).is_some()
    {
        (
            "thread-actions".to_owned(),
            rune::Side::Bottom,
            rune::Align::End,
        )
    } else if let Some(a) = s
        .snooze_anchor
        .as_deref()
        .filter(|a| cx.layout_with(a, |_| ()).is_some())
    {
        (a.to_owned(), rune::Side::Right, rune::Align::Start)
    } else if cx.layout_with(&row_key, |_| ()).is_some() {
        (row_key, rune::Side::Right, rune::Align::Start)
    } else if s.snooze_at.is_some() {
        (
            "snooze-menu-point".to_owned(),
            rune::Side::Right,
            rune::Align::Start,
        )
    } else if cx.layout_with("thread-actions", |_| ()).is_some() {
        (
            "thread-actions".to_owned(),
            rune::Side::Bottom,
            rune::Align::End,
        )
    } else {
        return None;
    };
    let mut card = col()
        .key("snooze-menu")
        .role("menu")
        .focus_group()
        .w(280.0)
        .padding(4.0)
        .gap(1.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        // The title is a pixel label, inset 8 px so it lines up with the rows' icons.
        .child(meta_label("Snooze thread", p.muted).px(8.0).pt(6.0).pb(4.0));
    if s.custom_snooze_open {
        card = card
            .child(label("Local date and time, or duration", SMALL, p.muted).px(8.0))
            .child(
                text_input(&s.snooze_edit, LABEL)
                    .key("snooze-custom-input")
                    .label("Snooze time")
                    .placeholder("2026-11-12 09:00 or 90m, 3h, 2d", p.muted)
                    .single_line()
                    .color(p.ink)
                    .w_full()
                    .padding(8.0)
                    .mx(4.0)
                    .radius(6.0)
                    .fill(field_fill(s, p))
                    .field_focus(&s.snooze_edit, p)
                    .on_edit(|s: &mut State| &mut s.snooze_edit),
            )
            .child(button("snooze-custom-save", "Snooze", p, move |s| {
                if let Some(until) =
                    lifecycle::custom_snooze(&s.snooze_edit.text, chrono::Local::now())
                {
                    let ids = s.snooze_targets.clone();
                    for id in ids {
                        s.snooze_thread(id, until);
                    }
                } else {
                    s.lifecycle_error =
                        Some("Choose a valid future local time. DST gaps are not valid.".into());
                }
            }));
    } else {
        for (i, choice) in choices.iter().enumerate() {
            card = card.child(
                menu_item(&format!("snooze-preset-{i}"), &choice.label, "clock", p)
                    // The label already grows, so the time sits against the right edge on its
                    // own. A spacer here would split the row and clip the label.
                    .child(
                        label(&choice.time, TIMER, p.muted)
                            .w(72.0)
                            .align(rune::Align::End),
                    )
                    .on_press(move |s: &mut State| {
                        let until = s
                            .snooze_menu
                            .as_ref()
                            .and_then(|(_, c)| c.get(i))
                            .and_then(|c| lifecycle::pick_preset(c, lifecycle::now()));
                        if let Some(until) = until {
                            let ids = s.snooze_targets.clone();
                            for id in ids {
                                s.snooze_thread(id, until);
                            }
                        } else {
                            s.lifecycle_error =
                                Some("That snooze time has passed. Choose another time.".into());
                        }
                    }),
            );
        }
        card = card.child(rule(p)).child(
            menu_item("snooze-custom", "Custom...", "clock", p).on_press(|s: &mut State| {
                s.custom_snooze_open = true;
                s.snooze_edit.focused = true;
            }),
        );
    }
    if let Some(error) = &s.lifecycle_error {
        card = card.child(label(error, SMALL, p.danger).wrap());
    }
    // A popover on the thread's row or header trigger: no scrim, no Cancel row. Escape and a
    // press outside close it.
    Some(
        card.anchored_fit(anchor, side, align, 4.0)
            .modal(true, |s: &mut State| {
                s.snooze_menu = None;
                s.snooze_anchor = None;
                s.snooze_at = None;
                s.lifecycle_error = None;
            }),
    )
}

pub(super) fn bulk_thread_actions(s: &State, p: Palette) -> El {
    let count = s.selected_threads.len();
    let mut actions = col().gap(1.0);
    for (key, title, action) in [
        ("bulk-unpin", "Unpin", 0),
        ("bulk-settle", "Settle", 1),
        ("bulk-snooze", "Snooze...", 2),
        ("bulk-unread", "Mark unread", 3),
        ("bulk-delete", "Delete...", 4),
    ] {
        actions = actions.child(
            menu_item(
                key,
                &format!("{title} ({count})"),
                ["pin", "check", "clock", "dot", "trash"][action],
                p,
            )
            .on_press(move |s: &mut State| {
                let ids = s.selected_threads.clone();
                let at = s.row_menu_at;
                s.row_menu = None;
                s.row_menu_at = None;
                if action == 2 {
                    if let Some(id) = ids.first() {
                        s.open_snooze_from(*id, Some(format!("thread-{}", *id)), at);
                    }
                    return;
                }
                if action == 4 {
                    if let Some(id) = ids.first() {
                        s.request_delete(*id);
                    }
                    return;
                }
                for id in ids {
                    if action == 0 {
                        if s.threads.iter().any(|t| t.id == id && t.pinned()) {
                            s.pin_thread(id);
                        }
                    } else if action == 1 {
                        if s.threads.iter().any(|t| t.id == id && !t.life.settled()) {
                            s.settle_thread(id);
                        }
                    } else if let Some(t) = s.threads.iter_mut().find(|t| t.id == id) {
                        t.life.mark_unread();
                    }
                }
                s.checkpoint();
            }),
        );
    }
    actions
}

pub(super) fn row_menu(s: &State, p: Palette, cx: &Cx<State>) -> Option<El> {
    let target = s.row_menu?;
    let (anchor, actions) = match target {
        RowTarget::Thread(id) => {
            let row_key = format!("thread-{id}");
            let anchor = if s.row_menu_at.is_some() {
                "row-menu-point".to_owned()
            } else if cx.layout_with(&row_key, |_| ()).is_some() {
                row_key
            } else if cx.layout_with("thread-actions", |_| ()).is_some() {
                "thread-actions".to_owned()
            } else {
                return None;
            };
            (
                anchor,
                if s.selected_threads.contains(&id) && s.selected_threads.len() > 1 {
                    bulk_thread_actions(s, p)
                } else {
                    thread_actions(s, id, p, &format!("thread-{id}"))
                },
            )
        }
        RowTarget::Project(id) => {
            let option_key = format!("project-option-{id}");
            let anchor = if s.row_menu_at.is_some() {
                "row-menu-point".to_owned()
            } else if cx.layout_with(&option_key, |_| ()).is_some() {
                option_key
            } else {
                return None;
            };
            (
                anchor,
                col()
                    .child(
                        menu_item("menu-project-new-thread", "New thread", "compose", p).on_press(
                            move |s: &mut State| {
                                s.row_menu = None;
                                s.new_thread_in(Some(id));
                            },
                        ),
                    )
                    .child(
                        menu_item("menu-project-captain", "Open Captain", "grid", p).on_press(
                            move |s: &mut State| {
                                s.row_menu = None;
                                s.project_picker = None;
                                s.choose_project_captain(id);
                            },
                        ),
                    )
                    .child(
                        menu_item(
                            "menu-remove-project",
                            "Remove workspace from list",
                            "close",
                            p,
                        )
                        .on_press(move |s: &mut State| {
                            s.row_menu = None;
                            s.project_picker = None;
                            s.remove_project(id);
                        }),
                    ),
            )
        }
    };
    Some(
        col()
            .key("row-context-menu")
            .instant_changes()
            .role("menu")
            .focus_group()
            .label("Row actions")
            .w(222.0)
            .padding(4.0)
            .gap(1.0)
            .radius(12.0)
            .fill(p.card)
            .border(1.0, p.frame)
            .box_shadow(soft_shadow(p))
            .anchored_fit(
                anchor,
                // Beside the point, as a native context menu opens, so the window clamps it
                // vertically instead of cutting it short.
                if s.row_menu_at.is_some() || matches!(target, RowTarget::Thread(_)) {
                    rune::Side::Right
                } else {
                    rune::Side::Bottom
                },
                rune::Align::Start,
                if s.row_menu_at.is_some() { 2.0 } else { 4.0 },
            )
            .modal(true, |s: &mut State| {
                s.row_menu = None;
                s.row_menu_at = None;
            })
            .child(actions),
    )
}

pub(super) fn thread_dialog(s: &State, p: Palette) -> El {
    let renaming = s.rename_thread.is_some();
    let id = s.rename_thread.or(s.delete_thread).expect("thread dialog");
    let captain = s
        .threads
        .iter()
        .any(|thread| thread.id == id && s.is_captain_home(thread));
    let title = if renaming && captain {
        "Name this body of work"
    } else if renaming {
        "Rename thread"
    } else {
        "Delete thread"
    };
    let mut card = col()
        .key("thread-management-dialog")
        .role("dialog")
        .label(title)
        .w(380.0)
        .max_w(540.0)
        .padding(16.0)
        .gap(12.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(label(title, DIALOG_TITLE, p.ink));
    if renaming {
        if captain {
            card = card.child(
                label(
                    "Name the goal, feature, or ongoing work. The captain profile stays unchanged.",
                    SMALL,
                    p.muted,
                )
                .wrap(),
            );
        }
        card = card.child(
            text_input(&s.rename_edit, BODY)
                .key("thread-name-input")
                .label("Thread name")
                .single_line()
                .color(p.ink)
                .w_full()
                .padding(8.0)
                .radius(12.0)
                .fill(if s.dark { p.bg } else { p.panel })
                .field_focus(&s.rename_edit, p)
                .on_edit(|s: &mut State| &mut s.rename_edit)
                .on_submit(|s: &mut State, _| s.save_thread_name()),
        );
    } else {
        let roots = if s.delete_roots.is_empty() {
            vec![id]
        } else {
            s.delete_roots.clone()
        };
        let count = roots.len();
        let all: std::collections::HashSet<_> =
            roots.iter().flat_map(|id| s.deletion_ids(*id)).collect();
        let descendants = all.len().saturating_sub(count);
        let target = if count > 1 {
            format!("these {count} threads and their")
        } else {
            "this thread and its".into()
        };
        card = card.child(label(format!(
            "Delete {target} {descendants} delegated children? Running agents will stop. Project files and Pi session files stay on disk."
        ), BODY, p.ink).wrap());
    }
    card = card.child(
        row()
            .gap(8.0)
            .child(spacer())
            .child(secondary_button("cancel-thread-dialog", "Cancel", p, |s| {
                s.rename_thread = None;
                s.delete_thread = None;
            }))
            .child(if renaming {
                button("save-thread-name", "Save name", p, |s| s.save_thread_name())
                    .disabled(s.rename_edit.text.trim().is_empty())
            } else {
                button(
                    "confirm-delete-thread",
                    "Delete thread and children",
                    p,
                    |s| s.confirm_delete(),
                )
            }),
    );
    el().key("thread-dialog-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .padding(16.0)
        .fill(Color::BLACK.alpha(0.3))
        .modal(false, |s: &mut State| {
            s.rename_thread = None;
            s.delete_thread = None;
        })
        .child(card)
}

/// A centred modal over a dimmed window, as T3 Code opens its command palette: it sits a little
/// above the middle, and a press outside or Escape closes it.
pub(super) fn modal_frame(key: &str, s: &State, close: fn(&mut State), card: El) -> El {
    col()
        .key(key)
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .items_center()
        .pt((s.viewport_height * 0.16).clamp(40.0, 160.0))
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.22 }))
        .modal(false, close)
        .on_press(close)
        // Escape closes it from the keyboard, so the scrim is no Tab stop.
        .focusable(false)
        .child(card)
}

/// A keyboard hint for a modal's footer: the key in a small cap, then what it does.
/// A key name in an outlined chip.
pub(super) fn keycap_chip(keys: &str, p: Palette) -> El {
    outlined(
        row()
            .h(20.0)
            .px(6.0)
            .center_y()
            .radius(6.0)
            .fill(p.card)
            .child(meta_text(keys, p.muted)),
        p,
    )
}
pub(super) fn keycap(keys: &str, action: &str, p: Palette) -> El {
    row()
        .gap(8.0)
        .center_y()
        .child(keycap_chip(keys, p))
        .child(label(action, SMALL, p.muted))
}

/// A floating surface for menus and pickers, styled as T3 Code's popovers: one radius, a
/// hairline edge and a soft shadow that reads in both themes.
pub(super) fn popover(key: &str, name: &str, _: &State, p: Palette) -> El {
    col()
        .key(key)
        .label(name)
        .padding(4.0)
        .gap(1.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
}
