use super::*;

pub(super) fn sidebar_toggle(p: Palette, compact: bool) -> El {
    icon_button("sidebar-toggle", "Toggle sidebar", "sidebar", p, move |s| {
        if compact {
            s.compact_sidebar = !s.compact_sidebar;
        } else {
            s.sidebar = !s.sidebar;
        }
    })
}

/// The ship mark at 24 px with a 1 px `frame` outline.
pub(super) fn shina_mark(s: &State, p: Palette) -> El {
    el().key("shina-logo")
        .size(
            s.artwork.snap_points(HEADER_MARK),
            s.artwork.snap_points(HEADER_MARK),
        )
        .radius(6.0)
        .clip_rounded(6.0)
        .pixels(&s.artwork.mark_pixels(s.dark, HEADER_MARK))
        // The day mark's sky is the chrome's sky, so a 1 px frame keeps its edge. The frame is
        // an overlay because Rune paints a clipped image over its border.
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(6.0)
                .border(1.0, p.frame)
                .a11y_hidden(),
        )
}

/// Side of the ship mark above an empty list. The mark is 32 cells, so 32 pt is one point per
/// cell and two device pixels at 2x: every cell lands on whole pixels.
pub(super) const EMPTY_MARK: f32 = 32.0;

/// The empty state of a list: the ship mark at a whole multiple of its cells, above the
/// explanation.
pub(super) fn empty_list(s: &State, p: Palette, key: &str, text: &str) -> El {
    col()
        .key(key)
        .role("status")
        .label(text)
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(12.0)
        .child(
            el().key(format!("{key}-mark"))
                .size(
                    s.artwork.snap_points(EMPTY_MARK),
                    s.artwork.snap_points(EMPTY_MARK),
                )
                .radius(6.0)
                .clip_rounded(6.0)
                .pixels(&s.artwork.mark_pixels(s.dark, EMPTY_MARK))
                .a11y_hidden(),
        )
        .child(
            label(text, SMALL, p.muted)
                .align(rune::Align::Center)
                .wrap()
                .w_full(),
        )
}

/// The wordmark's size in points, for a header `line` points wide after the native controls
/// and the gap before the tabs: Inter 14/600 when it fits, 12 in a narrow sidebar, and none
/// in a truly narrow one.
/// Width a macOS title strip reserves for the window buttons, which every platform assumes when
/// it sizes the wordmark.
pub(super) const TITLE_CONTROLS: f32 = 72.0;

pub(super) fn wordmark_size(line: f32) -> Option<f32> {
    // The compose button and its 2 pt gap follow the wordmark's slot, so the wordmark
    // reserves 26 pt for it: the mark drops out earlier on a 200 pt sidebar.
    let before = HEADER_TOGGLE + HEADER_MARK + 26.0;
    if line >= before + wordmark_gap(14.0) + WORDMARK_WIDTH {
        Some(14.0)
    } else if line >= before + wordmark_gap(12.0) + WORDMARK_WIDTH * 0.8 {
        Some(12.0)
    } else {
        None
    }
}
/// The space between the mark and the wordmark: 6 pt at 22 px, 4 pt at 11 px.
pub(super) fn wordmark_gap(size: f32) -> f32 {
    if size < 15.0 { 4.0 } else { WORDMARK_GAP }
}

/// The sidebar's half of the title strip, left-aligned as T3 Code lays it out: the sidebar
/// toggle, then Shina's mark and name. Search and actions sit in the sidebar below.
pub(super) fn sidebar_header(
    s: &State,
    p: Palette,
    compact: bool,
    line: f32,
    wordmark_line: f32,
) -> El {
    let mut header = row().h(36.0).center_y().child(sidebar_toggle(p, compact));
    if line >= HEADER_TOGGLE + HEADER_MARK {
        header = header.child(shina_mark(s, p));
        if let Some(size) = wordmark_size(wordmark_line) {
            header = header.child(
                label(
                    "Shina",
                    TextStyle::new(size, 600.0).tracking(-0.01 * size),
                    p.ink,
                )
                .key("shina-wordmark")
                .ml(wordmark_gap(size)),
            );
        }
    }
    header.child(spacer())
}

/// The title strip's left end when the sidebar is not docked: the toggle and the 24 px mark,
/// then the gap before the tabs. The wordmark waits for the sidebar.
pub(super) fn undocked_header(s: &State, p: Palette, compact: bool) -> El {
    row()
        .h(36.0)
        .center_y()
        .child(sidebar_toggle(p, compact))
        .child(shina_mark(s, p))
        .pr(HEADER_TAB_GAP)
}
/// The space between the compose button, at the header's end, and the first tab.
pub(super) const HEADER_TAB_GAP: f32 = 12.0;
/// The space between the mark and the 22 px wordmark. The toggle and the mark touch their boxes.
pub(super) const WORDMARK_GAP: f32 = 6.0;
pub(super) const HEADER_TOGGLE: f32 = 24.0;
pub(super) const HEADER_MARK: f32 = 24.0;
/// "Shina" in Inter 15/600.
pub(super) const WORDMARK_WIDTH: f32 = 40.0;

/// KSG's `SidebarHeaderIconButton`: a 28 pt icon control in the sidebar's header row.
pub(super) fn header_control(item: El, p: Palette) -> El {
    item.size(28.0, 28.0)
        .center()
        .radius(8.0)
        .hover(p.selected)
        .press(p.selected)
}

pub(super) fn sidebar(s: &State, p: Palette, width: f32) -> El {
    let mut side = col()
        .key("sidebar")
        .w(width)
        .h_full()
        .min_h(0.0)
        .pr(8.0)
        .gap(1.0);
    // One row holds search and the sidebar's actions, as in T3 Code: filter by workspace,
    // add a workspace, and start a thread.
    let scoped = !s.scope_additional.is_empty() || s.project_scope != ProjectScope::All;
    let mut scope_machine: Option<String> = None;
    let scope = if !s.scope_additional.is_empty()
        || !s.machines.scope_remote.is_empty() && s.project_scope != ProjectScope::All
    {
        format!(
            "{} workspaces",
            s.scope_additional.len() + s.machines.scope_remote.len() + 1
        )
    } else {
        match s.project_scope {
            ProjectScope::All => "All workspaces".into(),
            ProjectScope::NoProject => "No workspace".into(),
            ProjectScope::Project(id) => {
                let owner = s
                    .machines
                    .scope_server
                    .as_deref()
                    .unwrap_or(&s.machines.selected);
                let name = if owner == s.machines.selected {
                    s.project(Some(id)).map(|p| p.name.as_str())
                } else {
                    s.machines
                        .views
                        .get(owner)
                        .and_then(|view| view["index"]["projects"].as_array())
                        .and_then(|projects| {
                            projects
                                .iter()
                                .find(|project| project["id"].as_u64() == Some(id as u64))
                        })
                        .and_then(|project| project["name"].as_str())
                }
                .unwrap_or("Workspace");
                if s.client.is_some() {
                    scope_machine = Some(s.machine_label(owner));
                }
                name.into()
            }
        }
    };
    // The accessibility label and tooltip join the machine with a comma; the chip shows it as
    // its own label.
    let scope_words = match &scope_machine {
        Some(machine) => format!("{scope}, {machine}"),
        None => scope.clone(),
    };
    if let Some(notice) = crew_ui::provider_recovery_notice(s, p) {
        side = side.child(notice);
    }
    // KSG's `SidebarThreadHeader`: the search field beside 28 pt controls for the project
    // scope, Add project and New thread, at 4 pt gaps.
    let scoped_projects: Vec<crate::projects::ProjectId> = match s.project_scope {
        ProjectScope::Project(id) => {
            let mut ids = vec![id];
            ids.extend(s.scope_additional.iter().copied());
            ids
        }
        _ => s.scope_additional.clone(),
    };
    let scope_glyph = if scoped_projects.len() > 1 {
        row()
            .child(project_tile(
                s,
                p,
                s.project(Some(scoped_projects[0])),
                14.0,
            ))
            .child(
                el().size(14.0, 14.0)
                    .ml(-4.0)
                    .radius(4.0)
                    .border(1.0, p.bg)
                    .child(project_tile(
                        s,
                        p,
                        s.project(Some(scoped_projects[1])),
                        14.0,
                    )),
            )
    } else if let Some(&id) = scoped_projects.first() {
        project_tile(s, p, s.project(Some(id)), 16.0)
    } else {
        icon("folder", if scoped { p.sea } else { p.muted }).size(14.0, 14.0)
    };
    let scope_button = header_control(
        el().key("project-scope")
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("Workspace: {scope_words}"))
            .help(format!("Workspace: {scope_words}"))
            .focusable(true)
            .on_press(|s: &mut State| s.open_picker(ProjectPicker::Scope))
            .child(scope_glyph),
        p,
    );
    side = side.child(
        row()
            .key("workspace-context")
            .h(28.0)
            .mt(8.0)
            .mb(8.0)
            .center_y()
            .gap(4.0)
            .child(
                row()
                    .key("thread-search-field")
                    .h(28.0)
                    .pl(6.0)
                    .pr(6.0)
                    .gap(6.0)
                    .center_y()
                    .radius(8.0)
                    .grow(1.0)
                    .min_w(0.0)
                    // KSG's `InputGroup size="sidebar"`: a 28 px `surface-well` (the raised
                    // fill under a hairline rim) that takes the ring while focused.
                    .fill(p.row)
                    .caret(p.ink, p.coral_bg)
                    .border(1.0, if s.search.focused { p.ring } else { p.frame })
                    // KSG seats the 13 px glyph in a 16 px slot, so the placeholder starts
                    // where the rows' labels do.
                    .child(
                        el().size(16.0, 16.0)
                            .center()
                            .child(icon("search", p.muted).size(13.0, 13.0)),
                    )
                    .child(
                        text_input(&s.search, ROW_QUIET)
                            .key("thread-search")
                            .label("Search conversations")
                            .placeholder("Search", p.muted)
                            .color(p.ink)
                            .single_line()
                            .grow(1.0)
                            .min_w(0.0)
                            .on_edit(|s: &mut State| &mut s.search)
                            .on_input_change(|s: &mut State, tx| {
                                s.sync_sidebar_search_input();
                                s.poll_sidebar_search(tx);
                            })
                            .on_input_key(|s: &mut State, ev, tx| {
                                if ev.key == rune::Key::Escape && !s.search.text.is_empty() {
                                    s.search.replace_text("");
                                    s.sync_sidebar_search_input();
                                    s.poll_sidebar_search(tx);
                                    true
                                } else {
                                    s.undo_key(ev, true)
                                }
                            }),
                    )
                    .child(sidebar_ui::search_clear(s, p)),
            )
            .child(scope_button)
            .child(header_control(
                el().key("project-switcher")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Add workspace")
                    .focusable(true)
                    .on_press(|s: &mut State| s.open_picker(ProjectPicker::AddProject))
                    .help("Add workspace")
                    .child(icon("folder-plus", p.muted).size(14.0, 14.0)),
                p,
            ))
            .child(header_control(
                el().key("new-thread")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("New thread")
                    .focusable(true)
                    .on_press_tx(|s: &mut State, tx| s.new_thread_click(tx.mods().shift))
                    .help(if s.projects.entries.iter().all(|project| project.hidden) {
                        "New thread"
                    } else {
                        "New thread. Shift-click: in the current workspace"
                    })
                    .child(icon("compose", p.muted).size(14.0, 14.0)),
                p,
            )),
    );
    // A workspace filter names itself and clears in one click.
    if scoped {
        side = side.child(
            row()
                .key("scope-chip")
                .h(24.0)
                .ml(4.0)
                .mb(8.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .radius(6.0)
                .fill(p.selected)
                .child(icon("folder", p.muted))
                .child(label(&scope, SMALL, p.ink).truncate().shrink().min_w(0.0))
                .child(match &scope_machine {
                    Some(machine) => label(machine, SMALL, p.faint).truncate().max_w(MACHINE_CAP),
                    None => el(),
                })
                .child(spacer())
                .child(
                    row()
                        .key("clear-project-scope")
                        .role("button")
                        .label("Show all workspaces")
                        .cursor(rune::Cursor::Pointer)
                        .focusable(true)
                        .size(16.0, 16.0)
                        .radius(4.0)
                        .center()
                        .hover(p.selected)
                        .on_press(|s: &mut State| {
                            s.project_scope = ProjectScope::All;
                            s.scope_additional.clear();
                            s.machines.scope_remote.clear();
                            s.selected_threads.clear();
                            s.checkpoint();
                        })
                        .child(icon("close", p.muted).size(12.0, 12.0)),
                ),
        );
    }
    if s.show_archive {
        side = side.child(
            row()
                .key("archive-heading")
                .h(30.0)
                .pl(ROW_INSET)
                .pr(8.0)
                .gap(8.0)
                .center_y()
                .child(icon("archive", p.muted))
                .child(label("Archive", TextStyle::new(13.0, 600.0), p.ink))
                .child(spacer())
                .child(
                    label("Back", SMALL, p.muted)
                        .key("leave-archive")
                        .role("button")
                        .cursor(rune::Cursor::Pointer)
                        .focusable(true)
                        .on_press(|s: &mut State| s.show_archive = false),
                ),
        );
    }
    side = side.child(sidebar_ui::content(s, p));
    if let Some(notice) = &s.undo {
        let count = notice.entries.len();
        let title = if count > 1 {
            format!("{}, {count} threads", notice.action.undo_label())
        } else {
            notice.action.undo_label().to_string()
        };
        // An outlined card with the shortcut as a keycap at its right edge.
        side = side.child(
            row()
                .key("thread-undo")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!("{title}, Ctrl+Z"))
                .focusable(true)
                .h(32.0)
                .mx(8.0)
                .mb(4.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .fill(p.card)
                .border(1.0, p.frame)
                .hover(p.selected)
                .press(p.selected)
                .on_press(|s: &mut State| s.undo_thread_action())
                .child(
                    label(title, TextStyle::new(12.0, 500.0), p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(keycap_chip("Ctrl+Z", p)),
        );
    }
    // The footer is a row of quiet icons, as in T3 Code: no divider and no wordmark.
    // Ported from KSG Code's `SidebarChrome` footer (`SidebarUtilityItem`): 32 px icon-only
    // controls with 16 px glyphs, 4 px apart, in an 8/12 px padded strip. Settings holds
    // preferences such as the theme.
    side = side.child(
        row()
            .key("sidebar-footer")
            .pt(8.0)
            .pb(12.0)
            .px(8.0)
            .gap(4.0)
            .center_y()
            .child(
                footer_button(
                    "sidebar-settings",
                    "Settings",
                    "settings",
                    s.settings_menu,
                    p,
                    |s| s.open_settings(),
                )
                .help("Settings"),
            )
            .child(
                footer_button(
                    "show-archive",
                    if s.show_archive {
                        "Back to threads"
                    } else {
                        "Archive"
                    },
                    "archive",
                    s.show_archive,
                    p,
                    |s| s.show_archive = !s.show_archive,
                )
                .help("Archive"),
            )
            .child(spacer())
            .child(updates_ui::footer(s, p)),
    );
    side
}

/// KSG's `SidebarUtilityItem`: a 32 px `size="footer"` icon button with a 16 px glyph and
/// the shared 8 px control radius, showing its on state with a card fill like `toggle_button`.
pub(super) fn footer_button(
    key: &str,
    title: &str,
    glyph: &str,
    on: bool,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    let active_fill = if p.panel.luminance() < 0.5 {
        p.lift
    } else {
        p.card
    };
    el().key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .checked(on)
        .focusable(true)
        .size(32.0, 32.0)
        .center()
        .radius(8.0)
        .fill(if on { active_fill } else { Color::TRANSPARENT })
        .hover(p.selected)
        .on_press(action)
        .child(tool_icon(glyph, p.muted))
}

pub(super) fn menu_item(key: &str, title: &str, glyph: &str, p: Palette) -> El {
    menu_item_in(key, title, glyph, p, p.ink, p.muted)
}
/// A menu item that destroys something: the title and icon use the danger colour.
pub(super) fn danger_menu_item(key: &str, title: &str, glyph: &str, p: Palette) -> El {
    menu_item_in(key, title, glyph, p, p.danger, p.danger)
}
/// A menu row that stays greyed out because acting on it would do nothing, like KSG's
/// `disabled` DropdownMenuItem: faint ink, no press, not a tab stop.
pub(super) fn disabled_menu_item(key: &str, title: &str, glyph: &str, p: Palette) -> El {
    menu_item_in(key, title, glyph, p, p.faint, p.faint)
        .focusable(false)
        .cursor(rune::Cursor::Default)
}
pub(super) trait BlockedAction {
    fn blocked(self, reason: Option<String>, key: &str, p: Palette, inline: bool) -> El;
}

impl BlockedAction for El {
    fn blocked(self, reason: Option<String>, key: &str, p: Palette, inline: bool) -> El {
        let Some(reason) = reason else { return self };
        let control = self
            .disabled(true)
            .focusable(false)
            .cursor(rune::Cursor::Default)
            .description(&reason)
            .help(&reason);
        if inline {
            control.child(
                label(reason, SMALL, p.faint)
                    .key(format!("{key}-reason"))
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            )
        } else {
            control
        }
    }
}

pub(super) fn gated_menu_item(
    key: &str,
    title: &str,
    glyph: &str,
    p: Palette,
    reason: Option<String>,
) -> El {
    let Some(reason) = reason else {
        return menu_item(key, title, glyph, p);
    };
    if reason.chars().count() <= 24 {
        disabled_menu_item(key, title, glyph, p).blocked(Some(reason), key, p, true)
    } else {
        col()
            .key(key)
            .role("menuitem")
            .label(title)
            .disabled(true)
            .help(&reason)
            .description(&reason)
            .h(44.0)
            .px(8.0)
            .justify_center()
            .child(
                row()
                    .center_y()
                    .gap(8.0)
                    .child(icon(glyph, p.faint))
                    .child(label(title, TITLE, p.faint).truncate()),
            )
            .child(
                label(reason, SMALL, p.faint)
                    .key(format!("{key}-reason"))
                    .pl(24.0)
                    .truncate(),
            )
    }
}
/// A menu item with a value at its right edge in pixel type, such as "3 days".
pub(super) fn menu_value_item(key: &str, title: &str, value: &str, glyph: &str, p: Palette) -> El {
    menu_item(key, title, glyph, p)
        .label(format!("{title}, {value}"))
        .child(meta_text(value, p.muted))
}
pub(super) fn menu_item_in(
    key: &str,
    title: &str,
    glyph: &str,
    p: Palette,
    ink: Color,
    icon_ink: Color,
) -> El {
    row()
        .key(key)
        .role("menuitem")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .h(28.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(6.0)
        // Keys and the pointer share one highlight, as in an NSMenu. Every row asks for focus;
        // the menu's modal gives it to the first, so arrows work as soon as the menu opens.
        .focus_highlight(p.selected)
        .autofocus()
        .child(el().size(ICON, ICON).center().child(icon(glyph, icon_ink)))
        .child(
            label(title, TextStyle::new(13.0, 500.0), ink)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
}
/// Clone a repository into a new folder and open it.
pub(super) fn clone_dialog(s: &State, p: Palette) -> El {
    let projects = &s.projects;
    let cloning = projects.cloning();
    let field = field_fill(s, p);
    let mut card = col()
        .key("clone-dialog")
        .role("dialog")
        .label("Clone repository")
        .w(460.0)
        .padding(16.0)
        .gap(12.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .center_y()
                .child(
                    col()
                        .gap(2.0)
                        .child(label("Clone repository", DIALOG_TITLE, p.ink))
                        .child(label(
                            "Uses your existing git and GitHub CLI login.",
                            SMALL,
                            p.muted,
                        )),
                )
                .child(spacer())
                .child(icon_button(
                    "close-clone",
                    if cloning { "Stop the clone" } else { "Close" },
                    "close",
                    p,
                    |s| s.close_clone_dialog(),
                )),
        )
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Repository", p.muted))
                .child(
                    text_input(&projects.clone_source, BODY)
                        .key("clone-source")
                        .label("Repository URL or owner/repo")
                        .placeholder("owner/repo or https://github.com/owner/repo", p.muted)
                        .single_line()
                        .color(p.ink)
                        .caret(p.ink, p.coral_bg)
                        .w_full()
                        .padding(8.0)
                        .radius(6.0)
                        .fill(field)
                        .field_focus(&projects.clone_source, p)
                        .disabled(cloning)
                        .on_edit(|s: &mut State| &mut s.projects.clone_source)
                        .on_submit(|s: &mut State, _| s.start_clone()),
                ),
        )
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Clone into", p.muted))
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            row()
                                .h(32.0)
                                .px(8.0)
                                .center_y()
                                .grow(1.0)
                                .min_w(0.0)
                                .radius(6.0)
                                .fill(field)
                                .border(1.0, p.field_line)
                                .child(
                                    label(projects.clone_parent.display().to_string(), BODY, p.ink)
                                        .truncate()
                                        .min_w(0.0),
                                ),
                        )
                        .child(
                            secondary_button("clone-choose-parent", "Choose...", p, |_| {})
                                .on_press_tx(crate::projects::pick_clone_parent)
                                .disabled(cloning),
                        ),
                ),
        );
    if s.machines.selected != "local" {
        card = card
            .child(meta_label("Destination path on this server", p.muted))
            .child(
                text_input(
                    &s.projects.clone_path,
                    BODY.family(rune::Family::SystemMono),
                )
                .key("remote-clone-path")
                .label("Host destination path")
                .single_line()
                .w_full()
                .padding(8.0)
                .radius(6.0)
                .fill(field)
                .field_focus(&s.projects.clone_path, p)
                .color(p.ink)
                .on_edit(|s: &mut State| &mut s.projects.clone_path),
            );
    }
    let hint = match s.clone_target() {
        Ok((_, target)) => format!("Creates {}", target.display()),
        Err(_) if projects.clone_source.text.trim().is_empty() => {
            "Shorthand clones with gh. Other URLs clone with git.".into()
        }
        Err(error) => error,
    };
    card = card.child(label(hint, SMALL, p.muted).wrap_anywhere());
    if !projects.clone_progress.is_empty() {
        card = card.child(
            row()
                .key("clone-progress")
                .gap(6.0)
                .center_y()
                .child(status_mark(Status::Running, p, 8.0))
                .child(
                    label(&projects.clone_progress, SMALL, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    if let Some(error) = &projects.clone_error {
        card = card.child(
            label(error, SMALL, p.danger)
                .key("clone-error")
                .wrap_anywhere(),
        );
    }
    let mut actions = row().gap(8.0).center_y().child(spacer());
    actions = actions.child(secondary_button(
        "clone-cancel",
        if cloning { "Stop" } else { "Cancel" },
        p,
        |s| s.close_clone_dialog(),
    ));
    actions = actions.child(if cloning {
        primary_button("clone-start", "Cloning...", p, |_| {}).disabled(true)
    } else {
        primary_button("clone-start", "Clone", p, |s| s.start_clone())
    });
    card = card.child(actions);
    el().key("clone-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.28 }))
        .center()
        .modal(false, |s: &mut State| {
            if !s.projects.cloning() {
                s.close_clone_dialog();
            }
        })
        .child(card)
}

/// A sidebar section label: Inter 12/500, sentence case, muted. Counts and buttons follow it.
pub(super) fn section_title(title: &str, p: Palette) -> El {
    row()
        .h(28.0)
        .pl(ROW_INSET)
        .pr(4.0)
        .center_y()
        .child(meta_label(title, p.faint))
        .child(spacer())
}
/// A shelf heading's height.
// KSG's `CollapsibleSectionHeader` is `h-8` (32 px).
pub(super) const SHELF_HEADING: f32 = 32.0;
/// A selected sidebar row: KSG's `row` fill; light theme adds a hairline border.
pub(super) fn select_bar(item: El, selected: bool, dark: bool, p: Palette) -> El {
    if !selected {
        return item;
    }
    let bar = item.fill(p.row).radius(6.0);
    if dark { bar } else { bar.border(1.0, p.line) }
}
pub(super) fn face_for(status: Status) -> crate::visuals::Face {
    use crate::visuals::Face;
    match status {
        Status::Ready => Face::Normal,
        Status::Running => Face::Determined,
        Status::Waiting => Face::Surprised,
        Status::Completed => Face::Happy,
        Status::Failed | Status::Paused => Face::Dizzy,
    }
}
/// The captain's face beside its name in the transcript: 24 pt of the upper bust, a window of
/// 24 cells that is one pixel a cell at 1x and two at 2x, with a 1 px frame. The 20 pt head crop
/// it replaces had 28 cells, so it cropped or padded at both scales and lost Ember's goggles.
pub(super) fn speaker_avatar(s: &State, p: Palette, captain: usize) -> El {
    const SIZE: f32 = 24.0;
    let portrait = s.captains.get(captain).map(|c| c.portrait).unwrap_or(0);
    let portrait = portrait.min(crate::visuals::CAPTAIN_NAMES.len() - 1);
    let pixels = s
        .artwork
        .bust_window_pixels(portrait, crate::visuals::Face::Normal, SIZE);
    pixel_portrait_framed(s, &pixels, SIZE, p, None)
        .key(format!("speaker-avatar-{portrait}"))
        .a11y_hidden()
}
pub(super) fn avatar(s: &State, p: Palette, captain: usize, size: f32, status: Status) -> El {
    let portrait = s.captains.get(captain).map(|c| c.portrait).unwrap_or(0);
    portrait_avatar(s, p, portrait, size, status)
}
pub(super) fn portrait_avatar(
    s: &State,
    p: Palette,
    captain: usize,
    size: f32,
    status: Status,
) -> El {
    portrait_face(s, p, captain, size, status, None)
}
pub(super) fn portrait_face(
    s: &State,
    p: Palette,
    captain: usize,
    size: f32,
    status: Status,
    accent: Option<Color>,
) -> El {
    let captain = captain.min(crate::visuals::CAPTAIN_NAMES.len() - 1);
    let pixels = s.artwork.face_pixels(captain, face_for(status), size);
    let fitted = if s.artwork.is_pixel_art(captain) {
        s.artwork.snap_points(size)
    } else {
        size
    };
    let portrait = portrait(&pixels, fitted, p, accent)
        .key(format!("avatar-{captain}-{}", size as u32))
        .a11y_hidden();
    haloed(s, p, portrait, size, status)
}

/// The portrait in the captain picker and the editor's grid. The head art is 40 cells, so 40
/// points is a whole number of device pixels per cell at every integer window scale.
pub(super) const PICKER_PORTRAIT: f32 = 40.0;
/// Every machine uses the same portrait size, including before a captain is selected.
/// The widest a machine name and its connection note grow beside a workspace or a detail.
pub(super) const MACHINE_CAP: f32 = 110.0;

/// A sidebar portrait. It plays its status's animation strip through the shared stage in
/// `crate::portraits`, which also learns here whether the row is on screen. Under reduced
/// motion, or for a set without strips, it is the static face.
pub(super) fn sidebar_portrait(
    s: &State,
    p: Palette,
    row: usize,
    slot: usize,
    size: f32,
    status: Status,
) -> El {
    let slot = slot.min(crate::visuals::CAPTAIN_NAMES.len() - 1);
    let face = face_for(status);
    let pixels = s
        .portraits
        .pixels(slot, face, size)
        .unwrap_or_else(|| s.artwork.face_pixels(slot, face, size));
    portrait(
        &pixels,
        if s.artwork.is_pixel_art(slot) {
            s.artwork.snap_points(size)
        } else {
            size
        },
        p,
        None,
    )
    .key(format!("portrait-{row}"))
    .a11y_hidden()
    .on_visible(move |s: &mut State, on, tx| {
        s.portraits.set_visible(row, on);
        // The view doesn't read this; the next poll does.
        tx.live_only();
        tx.wake_after(std::time::Duration::ZERO);
    })
}

/// The corner mark that shows a portrait's status in pickers and headers.
pub(super) fn status_badge(p: Palette, portrait: El, size: f32, status: Status) -> El {
    status_badge_keyed(p, portrait, size, status, None)
}

pub(super) fn status_badge_keyed(
    p: Palette,
    portrait: El,
    size: f32,
    status: Status,
    mark_key: Option<String>,
) -> El {
    let show_mark =
        matches!(status, Status::Running | Status::Waiting | Status::Failed) || mark_key.is_some();
    if show_mark {
        // A small square in the panel color, with a 1 px frame, keeps the mark readable over
        // the artwork. Every live avatar sits on the panel (chat turns, the hero, the brief).
        let (backing, mark) = if size <= 16.0 {
            (8.0, 5.0)
        } else if size >= 30.0 {
            (13.0, 9.0)
        } else {
            (10.0, 7.0)
        };
        let offset = size - backing + 2.0;
        let mut dot = status_mark(status, p, mark);
        if let Some(key) = mark_key {
            dot = dot.key(key);
        }
        return el().size(size, size).child(portrait).child(
            el().absolute(offset, offset)
                .size(backing, backing)
                .radius(backing / 2.0)
                .fill(p.panel)
                .border(1.0, p.frame)
                .center()
                .child(dot)
                .a11y_hidden(),
        );
    }
    portrait
}
/// A sidebar row's status, and whether it waits on an approval: a pending request, or a
/// worker's approval, outranks the run. A captain's home stands for its whole crew.
pub(super) fn row_status(s: &State, thread: &Thread) -> (Status, bool) {
    let id = thread.id;
    let confirm = |t: &Thread| t.request.as_ref().is_some_and(|r| r["method"] == "confirm");
    let approval = confirm(thread)
        || s.threads
            .iter()
            .any(|child| child.parent == Some(id) && confirm(child));
    let status = if approval || thread.request.is_some() {
        Status::Waiting
    } else if s.is_captain_home(thread) {
        s.home_status(id)
    } else if thread.status == Status::Ready
        && (thread.client_creating.is_some()
            || thread.remote_send.is_some()
            || thread.client_promoted.is_some())
    {
        Status::Running
    } else {
        thread.status
    };
    (status, approval)
}

/// Ported from KSG Code's `Toasts` (`components/ui/toast.tsx`) and `stackedThreadToast`,
/// styled as a framed card.
pub(super) fn toast_stack(s: &State, p: Palette, top: f32) -> El {
    let mut stack = col()
        .key("toast-stack")
        .pin(Some(top + 8.0), Some(16.0), None, None)
        .w((s.viewport_width - 32.0).clamp(240.0, 360.0))
        .gap(8.0);
    for toast in s.toasts.iter().rev() {
        let id = toast.id;
        let (glyph_name, ink) = match toast.kind {
            crate::model::ToastKind::Info => ("dot", p.sea),
            crate::model::ToastKind::Success => ("check", p.sea),
            crate::model::ToastKind::Warning => ("alert", p.brass),
            crate::model::ToastKind::Error => ("alert", p.danger),
        };
        let mut card = col()
            .key(format!("toast-{id}"))
            .role("status")
            .label(toast.title.clone())
            .w_full()
            .padding(12.0)
            .gap(6.0)
            .radius(12.0)
            .fill(p.card)
            .border(1.0, p.frame)
            .box_shadow(soft_shadow(p))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(icon(glyph_name, ink))
                    .child(
                        label(&toast.title, TextStyle::new(13.0, 600.0), p.ink)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(icon_button(
                        &format!("toast-close-{id}"),
                        "Dismiss",
                        "close",
                        p,
                        move |s| {
                            s.dismiss_toast(id);
                        },
                    )),
            );
        if let Some(description) = &toast.description {
            card = card.child(label(description, SMALL, p.muted).wrap().selectable(true));
        }
        if let Some(action_label) = &toast.action_label {
            card = card.child(row().w_full().justify_end().child(button(
                &format!("toast-action-{id}"),
                action_label,
                p,
                move |s| s.activate_toast(id),
            )));
        }
        stack = stack.child(card);
    }
    stack
}

/// The portrait a captain wears; threads without a profile wear the first.
pub(super) fn captain_portrait(s: &State, captain: Option<usize>) -> usize {
    captain
        .and_then(|c| s.captains.get(c))
        .map(|c| c.portrait)
        .unwrap_or(0)
}

/// Whether the sidebar is on screen, as `view` decides it.
pub(super) fn sidebar_docked(s: &State) -> bool {
    let compact = s.viewport_width < 760.0;
    (s.sidebar && !compact) || (compact && s.compact_sidebar)
}

/// Advances the sidebar's portrait animations. Called from the app's poll; the answer is
/// when to wake for the next frame, or None when no portrait plays.
pub(crate) fn tick_portraits(
    s: &mut State,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    let rows: Vec<_> = if sidebar_docked(s) {
        s.sidebar_captains()
            .into_iter()
            .map(|i| {
                let t = &s.threads[i];
                let face = face_for(row_status(s, t).0);
                (t.id, captain_portrait(s, t.captain), face)
            })
            .collect()
    } else {
        vec![]
    };
    let mut rows = rows;
    let t = s.current();
    if let Some(captain) = t
        .captain
        .filter(|_| t.messages.is_empty() && t.tools.is_empty())
    {
        rows.push((
            HERO_PORTRAIT_ROW,
            captain_portrait(s, Some(captain)),
            face_for(shown_status(s)),
        ));
    }
    s.portraits.tick(now, rows)
}

pub(super) fn search_snippet_el(
    server: &str,
    id: usize,
    snippet: &str,
    query: &str,
    muted: Color,
    brass: Color,
) -> El {
    let ranges = crate::thread_search::find_snippet_token_ranges(snippet, query);
    let mut lbl = label(snippet, SMALL, muted)
        .key(format!("sidebar-search-snippet-{server}-{id}"))
        .truncate()
        .min_w(0.0);
    if !ranges.is_empty() {
        lbl = lbl.text_highlights(
            ranges
                .into_iter()
                .map(|r| rune::TextHighlight::new(r, brass.alpha(0.32))),
        );
    }
    lbl
}

/// Tile colours for projects without an icon. Each project keeps one by its name.
pub(super) const PROJECT_HUES: [&str; 6] = [
    "#6E8FE8", "#4FA58A", "#9A7BDD", "#C99A32", "#CF6A8C", "#3E9DBF",
];

/// A project's icon, as T3 Code shows each project's favicon: the repository's own PNG when
/// it has one, otherwise its initial on a tile in the project's colour.
pub(super) fn project_tile(
    s: &State,
    p: Palette,
    project: Option<&crate::projects::Project>,
    size: f32,
) -> El {
    // A pixel-framed square. At 16 px the frame is 1 px, so the initial still has room.
    let tile = el()
        .size(size, size)
        .radius(4.0)
        .clip_rounded(4.0)
        .border(1.0, p.frame)
        .a11y_hidden();
    let Some(project) = project else {
        return tile.center().child(icon("thread", p.muted));
    };
    if let Some(path) = &project.icon {
        let handle = s.attachment_cache.file(path.clone(), size, 2.0);
        if rune::is_headless() {
            let _ = handle.wait();
        }
        return tile.pixels(handle.pixels());
    }
    let hash = project.name.bytes().fold(0usize, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as usize)
    });
    let hue = color(PROJECT_HUES[hash % PROJECT_HUES.len()]);
    let initial = project
        .name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    // Keep initial-on-tile contrast >= 4.5:1 on both dark card (#1e1f23) and light card (#ffffff).
    tile.center()
        .fill(hue.alpha(if s.dark { 0.30 } else { 0.28 }))
        .child(label(initial, META, p.ink))
}

/// A row's workspace name, unless the Workspace filter already names that one workspace.
pub(super) fn row_workspace(s: &State, thread: &Thread) -> Option<String> {
    let named_by_filter = workspace_named_by_filter(s, &s.machines.selected, thread.project);
    (!named_by_filter).then(|| {
        s.project(thread.project)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "No workspace".into())
    })
}
pub(super) fn workspace_named_by_filter(s: &State, server: &str, project: Option<usize>) -> bool {
    s.scope_additional.is_empty()
        && s.machines.scope_remote.is_empty()
        && match s.project_scope {
            ProjectScope::Project(id) => {
                project == Some(id)
                    && s.machines
                        .scope_server
                        .as_deref()
                        .unwrap_or(&s.machines.selected)
                        == server
            }
            ProjectScope::NoProject => project.is_none(),
            ProjectScope::All => false,
        }
}
/// A machine's name for labels and tooltips. Whether this desktop can reach it is not part of
/// a row; the open thread says so in the composer notice stack.
pub(super) fn machine_caption(s: &State, server: &str) -> String {
    s.machine_label(server)
}

/// KSG Code's EnvironmentMachineIcon distinguishes a desktop from a linked server.
pub(super) fn machine_glyph(s: &State, server: &str) -> &'static str {
    if server == "local"
        || s.machines
            .views
            .get("local")
            .and_then(|view| view["serverId"].as_str())
            == Some(server)
    {
        "monitor"
    } else {
        "server"
    }
}

/// Whether more than one machine is linked, so rows say which machine they belong to.
pub(super) fn multiple_machines(s: &State) -> bool {
    s.client
        .as_ref()
        .is_some_and(|client| !client.linked.is_empty())
}

/// A sidebar row's width in points, from the sidebar setting and the last window width.
pub(super) fn sidebar_row_width(s: &State) -> f32 {
    sidebar_width_at(s.sidebar_width, s.viewport_width) - 16.0
}

pub(super) fn sidebar_max_width(viewport: f32) -> f32 {
    (viewport - 480.0).clamp(crate::model::SIDEBAR_MIN, crate::model::SIDEBAR_MAX)
}

pub(super) fn sidebar_width_at(preferred: f32, viewport: f32) -> f32 {
    if viewport < 760.0 {
        // The compact drawer sits in front of the chat rather than beside it.
        // Widen the docked sidebar without covering more of a small conversation.
        preferred.min(288.0).min((viewport - 56.0).max(0.0))
    } else {
        preferred.min(sidebar_max_width(viewport))
    }
}

/// About how wide `text` is at the sidebar's 13 px.
pub(super) fn text_width(text: &str) -> f32 {
    text.chars().count() as f32 * 6.2
}

/// A linked machine on a row: its icon, then its name. The name gives way first and can drop
/// out, which leaves the icon alone. Rows never show whether the machine is reachable.
pub(super) fn machine_tag(
    s: &State,
    p: Palette,
    server: &str,
    key: String,
    style: TextStyle,
    show_name: bool,
) -> El {
    let caption = machine_caption(s, server);
    let mut tag = row()
        .key(key)
        .role("img")
        .label(caption.clone())
        .help(caption)
        // Keep the icon visible when the machine name truncates.
        .gap(4.0)
        .center_y()
        .shrink()
        .min_w(ICON)
        .max_w(MACHINE_CAP)
        .child(icon(machine_glyph(s, server), p.faint));
    // A name with no room would show as a bare ellipsis, so it is left out instead.
    if show_name {
        tag = tag.child(
            label(s.machine_label(server), style, p.faint)
                .truncate()
                .shrink()
                .min_w(0.0)
                .a11y_hidden(),
        );
    }
    tag
}

pub(super) fn relative_time(updated: u64) -> Option<String> {
    if updated == 0 {
        return None;
    }
    let seconds = crate::projects::now().saturating_sub(updated);
    Some(match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    })
}
