use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum PickerChoice {
    All,
    NoProject,
    Project(usize),
    MachineProject(String, usize),
    MachineNoProject(String),
}

/// The picker's rows that match its search, in display order.
/// Each choice with its name and, for a linked machine, the machine's name.
pub(super) fn picker_choices(
    s: &State,
    mode: ProjectPicker,
) -> Vec<(PickerChoice, String, Option<String>)> {
    let query = s.picker_query.text.trim().to_lowercase();
    let mut choices = vec![];
    if mode == ProjectPicker::Scope {
        choices.push((PickerChoice::All, "All workspaces".to_string(), None));
    }
    let multiple = s
        .client
        .as_ref()
        .is_some_and(|client| !client.linked.is_empty());
    // The New captain editor shares the new-thread picker's machine grouping, but its
    // choice lands on the editor's `captain_project`, never on a draft thread.
    let draft_pick = matches!(
        mode,
        ProjectPicker::NewThread | ProjectPicker::CaptainWorkspace
    );
    let across = matches!(mode, ProjectPicker::Scope | ProjectPicker::CaptainScope)
        || draft_pick && multiple;
    if mode != ProjectPicker::CaptainScope && !(draft_pick && multiple) {
        choices.push((PickerChoice::NoProject, "No workspace".to_string(), None));
    }
    for id in s.sorted_projects() {
        let project = s.project(Some(id)).expect("sorted project");
        if !project.hidden {
            choices.push((
                PickerChoice::Project(id),
                project.name.clone(),
                (across && (s.client.is_some() || mode == ProjectPicker::CaptainScope))
                    .then(|| s.machine_label(&s.machines.selected)),
            ));
        }
    }
    if mode == ProjectPicker::CaptainScope {
        for peer in &s.peers {
            for project in peer.capabilities["projects"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if project["hidden"] == true {
                    continue;
                }
                if let (Some(id), Some(name)) = (project["id"].as_u64(), project["name"].as_str()) {
                    let choice = PickerChoice::MachineProject(peer.server_id.clone(), id as usize);
                    if !choices.iter().any(|(existing, _, _)| *existing == choice) {
                        choices.push((choice, name.into(), Some(peer.label.clone())));
                    }
                }
            }
        }
    }
    // Scope is authorized by the captain owner's peer links, not this desktop's
    // device links. Their ids can differ, especially when its owner is remote.
    if across && s.client.is_some() && mode != ProjectPicker::CaptainScope {
        for (server, view) in &s.machines.views {
            if server == &s.machines.selected {
                continue;
            }
            if draft_pick
                && (!s.machine_connected(server)
                    || mode == ProjectPicker::NewThread
                        && s.machines
                            .draft_picker
                            .as_ref()
                            .is_some_and(|target| target != server))
            {
                continue;
            }
            for project in view["index"]["projects"].as_array().into_iter().flatten() {
                if project["hidden"] == true {
                    continue;
                }
                if let (Some(id), Some(name)) = (project["id"].as_u64(), project["name"].as_str()) {
                    choices.push((
                        PickerChoice::MachineProject(server.clone(), id as usize),
                        name.to_string(),
                        Some(s.machine_label(server)),
                    ));
                }
            }
        }
    }
    if mode == ProjectPicker::CaptainScope {
        choices.sort_by_key(|(choice, _, _)| match choice {
            PickerChoice::MachineProject(server, _) => (true, server.clone()),
            _ => (false, String::new()),
        });
    }
    if draft_pick && multiple {
        choices.retain(|(choice, _, _)| match choice {
            PickerChoice::Project(_) => {
                s.machine_connected(&s.machines.selected)
                    && (mode == ProjectPicker::CaptainWorkspace
                        || s.machines
                            .draft_picker
                            .as_ref()
                            .is_none_or(|target| target == &s.machines.selected))
            }
            _ => true,
        });
        for server in machine_options(s) {
            if !s.machine_connected(&server)
                || mode == ProjectPicker::NewThread
                    && s.machines
                        .draft_picker
                        .as_ref()
                        .is_some_and(|target| target != &server)
            {
                continue;
            }
            choices.push((
                PickerChoice::MachineNoProject(server.clone()),
                "No workspace".into(),
                Some(s.machine_label(&server)),
            ));
        }
        choices.sort_by_key(|(choice, _, _)| {
            let server = match choice {
                PickerChoice::MachineProject(server, _)
                | PickerChoice::MachineNoProject(server) => server,
                _ => &s.machines.selected,
            };
            (
                server != "local",
                server.clone(),
                !matches!(choice, PickerChoice::MachineNoProject(_)),
            )
        });
    }
    choices.retain(|(_, name, machine)| {
        query.is_empty()
            || format!("{name} {}", machine.as_deref().unwrap_or(""))
                .to_lowercase()
                .contains(&query)
    });
    choices
}

pub(super) fn choose_picker(s: &mut State, mode: ProjectPicker, choice: PickerChoice, shift: bool) {
    let project = match choice {
        PickerChoice::All => {
            s.project_scope = ProjectScope::All;
            s.scope_additional.clear();
            s.machines.scope_remote.clear();
            s.selected_threads.clear();
            s.project_picker = None;
            s.checkpoint();
            return;
        }
        PickerChoice::NoProject => None,
        PickerChoice::Project(id) => Some(id),
        PickerChoice::MachineProject(server, id) => {
            if mode == ProjectPicker::CaptainScope {
                captain_ui::toggle_remote_scope(s, server, id);
            } else if mode == ProjectPicker::Scope {
                s.set_machine_scope(server, Some(id), shift);
            } else if mode == ProjectPicker::NewThread && s.machine_connected(&server) {
                s.select_machine_without_draft(&server);
                if s.machines.selected == server {
                    s.choose_draft_project(Some(id));
                }
            } else if mode == ProjectPicker::CaptainWorkspace {
                captain_workspace_pick(s, &server, Some(id));
            }
            return;
        }
        PickerChoice::MachineNoProject(server) => {
            if mode == ProjectPicker::NewThread && s.machine_connected(&server) {
                s.select_machine_without_draft(&server);
                if s.machines.selected == server {
                    s.choose_draft_project(None);
                }
            } else if mode == ProjectPicker::CaptainWorkspace {
                captain_workspace_pick(s, &server, None);
            }
            return;
        }
    };
    match mode {
        ProjectPicker::NewThread => s.choose_draft_project(project),
        ProjectPicker::Scope => s.set_project_scope(project, shift),
        ProjectPicker::AddProject => {}
        ProjectPicker::CaptainScope => {
            if let Some(project) = project {
                captain_ui::toggle_scope(s, project);
            }
        }
        ProjectPicker::CaptainWorkspace => {
            s.captain_project = Some(project);
            s.project_picker = None;
            s.checkpoint();
        }
    }
}

/// A machine pick in the New captain modal records the target machine and workspace on the
/// editor without switching the active machine or draft behind the modal until Save.
pub(super) fn captain_workspace_pick(s: &mut State, server: &str, project: Option<usize>) {
    if !s.machine_connected(server) {
        return;
    }
    if let Some(editor) = s.captain_editor.as_mut() {
        if editor.editing.is_some() || editor.promotion.is_some() {
            return;
        }
        editor.server = server.into();
        editor.machine_picker = false;
    }
    s.captain_project = Some(project);
    s.projects.captain_target = None;
    s.project_picker = None;
    s.checkpoint();
}

/// Ported from KSG Code's CommandPalette and CommandPaletteLookups project/environment
/// grouping. Captain scope membership is Shina-specific.
/// A centred modal that starts with a
/// search field and moves with the arrow keys. Enter picks the highlighted row; Shift adds a
/// workspace to the filter.
pub(super) fn project_picker(s: &State, p: Palette) -> El {
    let mode = s.project_picker.expect("project picker");
    let choices = picker_choices(s, mode);
    let highlight = s.picker_highlight.min(choices.len().saturating_sub(1));
    let placeholder = match mode {
        ProjectPicker::Scope => "Filter by workspace...",
        ProjectPicker::NewThread => "New thread in...",
        ProjectPicker::CaptainWorkspace => "Workspace...",
        ProjectPicker::AddProject => "Add workspace",
        ProjectPicker::CaptainScope => "Add to this captain's scope...",
    };
    let mut card = popover("project-picker", placeholder, s, p)
        .role("dialog")
        .w(520.0)
        .max_h(460.0)
        .padding(4.0)
        .radius(12.0)
        .on_press(|_: &mut State| {})
        .child(
            row()
                .h(46.0)
                // The search box and the rows below share one inset from the card's edge.
                .mx(4.0)
                .px(8.0)
                .gap(10.0)
                .center_y()
                .radius(6.0)
                .fill(field_fill(s, p))
                .field_focus(&s.picker_query, p)
                .child(
                    el().size(16.0, 16.0)
                        .center()
                        .child(icon("search", p.muted)),
                )
                .child(
                    rune::text_input(&s.picker_query, TextStyle::new(15.0, 400.0).tracking(-0.15))
                        .key("picker-search")
                        .label(placeholder)
                        .placeholder(placeholder, p.muted)
                        .color(p.ink)
                        .single_line()
                        .grow(1.0)
                        .min_w(0.0)
                        .on_edit(|s: &mut State| &mut s.picker_query)
                        .on_input_key(move |s: &mut State, event, _| {
                            if edits_text(event) {
                                s.picker_highlight = 0;
                            }
                            let count = s
                                .project_picker
                                .map_or(0, |mode| picker_choices(s, mode).len());
                            match event.key {
                                rune::Key::Down if count > 0 => {
                                    s.picker_highlight = (s.picker_highlight + 1).min(count - 1);
                                    true
                                }
                                // The list's ends, as cmdk's Home and End go in KSG Code's
                                // command palette. The field is one line, so its caret loses
                                // nothing it cannot reach with the arrows.
                                rune::Key::Home => {
                                    s.picker_highlight = 0;
                                    true
                                }
                                rune::Key::End if count > 0 => {
                                    s.picker_highlight = count - 1;
                                    true
                                }
                                rune::Key::Up => {
                                    s.picker_highlight = s.picker_highlight.saturating_sub(1);
                                    true
                                }
                                rune::Key::Enter => {
                                    if let Some(mode) = s.project_picker
                                        && let Some(choice) = picker_choices(s, mode)
                                            .get(s.picker_highlight.min(count.saturating_sub(1)))
                                            .map(|(choice, _, _)| choice.clone())
                                    {
                                        choose_picker(s, mode, choice, event.mods.shift);
                                    }
                                    true
                                }
                                rune::Key::Escape => {
                                    s.project_picker = None;
                                    true
                                }
                                _ => s.undo_key(event, true),
                            }
                        }),
                ),
        )
        // No hairline under the search box: its own frame already ends the field.
        .child(el().h(4.0));
    let mut list = col().gap(1.0).min_h(0.0).shrink().scroll(ScrollSpec {
        stick_bottom: false,
    });
    let mut last_machine = None;
    for (index, (choice, name, machine)) in choices.iter().enumerate() {
        if matches!(
            mode,
            ProjectPicker::NewThread
                | ProjectPicker::CaptainScope
                | ProjectPicker::CaptainWorkspace
        ) && machine != &last_machine
        {
            if let Some(machine) = machine {
                list = list.child(
                    label(machine, SMALL, p.muted)
                        .key(format!("picker-machine-{index}"))
                        .px(12.0)
                        .py(6.0),
                );
            }
            last_machine = machine.clone();
        }
        let remote_project = if let PickerChoice::MachineProject(server, id) = choice {
            s.machines
                .views
                .get(server)
                .and_then(|view| view["index"]["projects"].as_array())
                .and_then(|projects| {
                    projects
                        .iter()
                        .find(|project| project["id"].as_u64() == Some(*id as u64))
                })
                .or_else(|| {
                    s.peers
                        .iter()
                        .find(|p| &p.server_id == server)
                        .and_then(|p| p.capabilities["projects"].as_array())
                        .and_then(|ps| ps.iter().find(|p| p["id"].as_u64() == Some(*id as u64)))
                })
                .and_then(|project| crate::projects::Project::from_record(project, false))
        } else {
            None
        };
        let (key, project) = match choice {
            PickerChoice::All => ("scope-all-projects".to_string(), None),
            PickerChoice::NoProject => ("choose-no-project".to_string(), None),
            PickerChoice::Project(id) => (format!("choose-project-{id}"), s.project(Some(*id))),
            PickerChoice::MachineProject(server, id) => (
                format!("choose-project-{server}-{id}"),
                remote_project.as_ref(),
            ),
            PickerChoice::MachineNoProject(server) => (
                if server == &s.machines.selected {
                    "choose-no-project".into()
                } else {
                    format!("choose-no-project-{server}")
                },
                None,
            ),
        };
        let checked = match (mode, choice) {
            (ProjectPicker::CaptainScope, PickerChoice::Project(id)) => {
                captain_ui::in_scope(s, *id)
            }
            (ProjectPicker::CaptainScope, PickerChoice::MachineProject(server, id)) => {
                captain_ui::remote_in_scope(s, server, *id)
            }
            (ProjectPicker::Scope, PickerChoice::All) => s.project_scope == ProjectScope::All,
            (ProjectPicker::Scope, PickerChoice::NoProject) => {
                s.project_scope == ProjectScope::NoProject
            }
            (ProjectPicker::Scope, PickerChoice::Project(id)) => {
                s.project_scope != ProjectScope::All
                    && s.scope_includes_on(&s.machines.selected, Some(*id))
            }
            (ProjectPicker::Scope, PickerChoice::MachineProject(server, id)) => {
                s.project_scope != ProjectScope::All && s.scope_includes_on(server, Some(*id))
            }
            (ProjectPicker::CaptainWorkspace, PickerChoice::Project(id)) => {
                s.captain_project.unwrap_or(s.current().project) == Some(*id)
            }
            (ProjectPicker::CaptainWorkspace, PickerChoice::NoProject) => {
                s.captain_project.unwrap_or(s.current().project).is_none()
            }
            (ProjectPicker::CaptainWorkspace, PickerChoice::MachineNoProject(server)) => {
                *server == s.machines.selected
                    && s.captain_project.unwrap_or(s.current().project).is_none()
            }
            _ => false,
        };
        let choice = choice.clone();
        let action_choice = choice.clone();
        let mut item = row()
            .key(key)
            .role("menuitem")
            .cursor(rune::Cursor::Pointer)
            .label(match machine {
                Some(machine) => format!("{name}, {machine}"),
                None => name.clone(),
            })
            .focusable(true)
            .h(38.0)
            // Inset 4 px from the frame so the highlight sits inside it.
            .mx(4.0)
            .pl(12.0)
            .pr(8.0)
            .gap(10.0)
            .center_y()
            .radius(6.0)
            .fill(if index == highlight {
                p.coral_bg
            } else {
                Color::TRANSPARENT
            })
            .hover(p.selected)
            .on_press_tx(move |s: &mut State, tx| {
                choose_picker(s, mode, action_choice.clone(), tx.mods().shift)
            })
            .child(match choice {
                PickerChoice::All => el().size(16.0, 16.0).center().child(icon("list", p.muted)),
                _ => project_tile(s, p, project, 16.0),
            })
            .child(
                label(name.as_str(), TextStyle::new(14.0, 500.0), p.ink)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            )
            .child(match machine {
                Some(machine) => label(machine, SMALL, p.muted).truncate().max_w(MACHINE_CAP),
                None => el(),
            })
            // Where the workspace lives, so two folders with one name stay distinct.
            .child(
                label(
                    project
                        .map(|project| project.path.display().to_string())
                        .unwrap_or_default(),
                    SMALL,
                    p.faint,
                )
                .truncate()
                .grow(1.0)
                .min_w(0.0),
            );
        if checked {
            item = item.child(icon("check", p.coral));
        }
        if let PickerChoice::Project(id) = choice {
            let target = RowTarget::Project(id);
            item = item.on_secondary_press(move |s: &mut State, _, _| {
                s.row_menu = Some(target);
                s.row_menu_at = None;
            });
            item = row()
                .key(format!("project-option-{id}"))
                .child(item.grow(1.0));
        }
        list = list.child(item);
    }
    if choices.is_empty() {
        list = list.child(
            label("No matching workspace", SMALL, p.muted)
                .px(8.0)
                .py(8.0),
        );
    }
    card = card
        .child(list)
        .child(el().h(0.5).w_full().my(4.0).fill(p.line))
        .child(
            menu_item("picker-add-project", "Add workspace...", "folder-plus", p)
                .on_press(|s: &mut State| s.open_picker(ProjectPicker::AddProject)),
        );
    let mut hints = row()
        .h(34.0)
        .px(8.0)
        .gap(16.0)
        .center_y()
        .child(keycap("↑↓", "Move", p))
        .child(keycap("Enter", "Choose", p));
    if mode == ProjectPicker::Scope {
        hints = hints.child(keycap("Shift", "Add to filter", p));
    }
    card = card.child(hints.child(spacer()).child(keycap("Esc", "Close", p)));
    modal_frame(
        "project-picker-backdrop",
        s,
        |s| s.project_picker = None,
        card,
    )
}

/// What the Add workspace palette offers before browsing, as T3 Code's Add project sources.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum AddSource {
    LocalFolder,
    GitUrl,
    SystemPicker,
    Restore(std::path::PathBuf),
}

pub(super) fn add_sources(s: &State) -> Vec<(AddSource, String, String)> {
    let mut items = vec![
        (
            AddSource::LocalFolder,
            "Local folder".to_string(),
            "Browse a folder on disk".to_string(),
        ),
        (
            AddSource::GitUrl,
            "Git URL".into(),
            "Clone from a remote URL".into(),
        ),
        (
            AddSource::SystemPicker,
            "System folder picker".into(),
            "Choose a folder in the system dialog".into(),
        ),
    ];
    if s.machines.selected != "local" {
        items[0].1 = "Folder on this machine".into();
        items.retain(|(source, _, _)| *source != AddSource::SystemPicker);
    }
    let mut removed: Vec<_> = s.projects.entries.iter().filter(|p| p.hidden).collect();
    removed.sort_by_key(|p| std::cmp::Reverse(p.updated));
    for project in removed {
        items.push((
            AddSource::Restore(project.path.clone()),
            format!("Restore {}", project.name),
            project.path.display().to_string(),
        ));
    }
    let query = s.picker_query.text.trim().to_lowercase();
    items.retain(|(_, title, detail)| {
        query.is_empty()
            || title.to_lowercase().contains(&query)
            || detail.to_lowercase().contains(&query)
    });
    items
}

pub(super) fn run_add_source(s: &mut State, tx: &mut Tx, source: AddSource) {
    match source {
        AddSource::LocalFolder => s.browse_for_project(),
        AddSource::GitUrl => {
            s.project_picker = None;
            s.open_clone_dialog();
        }
        AddSource::SystemPicker => {
            s.project_picker = None;
            crate::projects::pick_folder(s, tx);
        }
        AddSource::Restore(path) => match s.add_project(path) {
            Ok(_) => {
                s.project_picker = None;
                s.projects.menu_error = None;
                s.checkpoint();
            }
            Err(error) => s.projects.menu_error = Some(error),
        },
    }
}

/// The folders the browse field lists, and the typed text up to the folder they are in.
pub(super) fn browse_entries(s: &State) -> Vec<String> {
    if s.projects.browser.request.as_ref()
        != Some(&(s.machines.selected.clone(), s.picker_query.text.clone()))
    {
        return vec![];
    }
    s.projects
        .browser
        .listing
        .as_ref()
        .map(|listing| {
            listing
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a key changes a search field's text, which sends a palette's highlight back to the
/// top. Rune also calls `on_edit` for keys that only move the caret or the highlight, so the
/// reset cannot live there.
pub(super) fn edits_text(event: &rune::KeyEvent) -> bool {
    matches!(
        event.key,
        rune::Key::Text(_) | rune::Key::Backspace | rune::Key::Delete
    ) || matches!(event.key, rune::Key::Command('v' | 'x' | 'z'))
}
/// Move the browse field into a listed folder with a trailing separator, matching KSG Code's
/// `appendBrowsePathSegment` / `browseTo`.
pub(super) fn browse_into(s: &mut State, name: &str) {
    if let Some(path) = s
        .projects
        .browser
        .listing
        .as_ref()
        .and_then(|listing| listing.entries.iter().find(|entry| entry.name == name))
        .map(|entry| entry.path.clone())
    {
        let sep = if path.contains('\\') && !path.contains('/') {
            '\\'
        } else {
            '/'
        };
        let next = if path.ends_with(['/', '\\']) {
            path
        } else {
            format!("{path}{sep}")
        };
        s.picker_query.replace_text(next);
    }
    s.picker_highlight = 0;
}

/// Shell-style Tab completion for folder browsing, ported from KSG Code's
/// `getFilesystemBrowseTabCompletion`: a single match enters the directory, several extend
/// to their common case-insensitive prefix.
pub(super) fn browse_tab_complete(s: &mut State, folders: &[String]) {
    let Some(first) = folders.first() else {
        return;
    };
    if folders.len() == 1 {
        browse_into(s, first);
        return;
    }
    let query = &s.picker_query.text;
    let leaf = if query.ends_with(['/', '\\']) {
        ""
    } else {
        query.rsplit(['/', '\\']).next().unwrap_or("")
    };
    let mut prefix: Vec<char> = first.chars().collect();
    for name in &folders[1..] {
        let chars: Vec<char> = name.chars().collect();
        let mut len = 0;
        while len < prefix.len()
            && len < chars.len()
            && prefix[len].to_lowercase().eq(chars[len].to_lowercase())
        {
            len += 1;
        }
        prefix.truncate(len);
    }
    let common: String = prefix.into_iter().collect();
    let lower_common = common.to_lowercase();
    if common.chars().count() > leaf.chars().count()
        && folders
            .iter()
            .all(|name| name.to_lowercase().starts_with(&lower_common))
    {
        let dir_prefix = if query.ends_with(['/', '\\']) {
            query.as_str()
        } else if let Some(idx) = query.rfind(['/', '\\']) {
            &query[..=idx]
        } else {
            ""
        };
        s.picker_query.replace_text(format!("{dir_prefix}{common}"));
        s.picker_highlight = 0;
    }
}

/// A palette row with a title and a quieter description, as T3 Code lists its sources.
pub(super) fn palette_row(
    key: String,
    glyph: &str,
    title: &str,
    detail: &str,
    lit: bool,
    p: Palette,
) -> El {
    row()
        .key(key)
        .role("menuitem")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .min_h(44.0)
        // Inset 4 px from the card's padding, in line with the search row above it.
        .mx(4.0)
        .px(8.0)
        .py(6.0)
        .gap(10.0)
        .center_y()
        .radius(6.0)
        .fill(if lit { p.coral_bg } else { Color::TRANSPARENT })
        .hover(p.selected)
        .child(el().size(16.0, 16.0).center().child(icon(glyph, p.muted)))
        .child({
            let text = col()
                .gap(1.0)
                .grow(1.0)
                .min_w(0.0)
                .child(label(title, TextStyle::new(14.0, 500.0), p.ink).truncate());
            if detail.is_empty() {
                text
            } else {
                text.child(label(detail, SMALL, p.muted).truncate())
            }
        })
}

/// The modifier the palette's add shortcut uses on this platform.
pub(super) const SUBMIT_CHORD: &str = if cfg!(target_os = "macos") {
    "⌘ Enter"
} else {
    "Ctrl Enter"
};

/// Ported from KSG Code's CommandPalette Add project flow: choose a source, then browse by
/// typing a path. Enter opens the highlighted folder, Ctrl+Enter adds the typed one, and a
/// path that does not exist yet is created first.
pub(super) fn add_project_palette(s: &State, p: Palette) -> El {
    let browsing = s.picker_browsing;
    let query = s.picker_query.text.clone();
    let folders = if browsing { browse_entries(s) } else { vec![] };
    let sources = if browsing { vec![] } else { add_sources(s) };
    let count = if browsing {
        folders.len()
    } else {
        sources.len()
    };
    let highlight = s.picker_highlight.min(count.saturating_sub(1));
    let current_listing = s.projects.browser.request.as_ref()
        == Some(&(s.machines.selected.clone(), query.clone()))
        && s.projects.browser.pending.is_none();
    let creates = browsing
        && current_listing
        && !query.trim().is_empty()
        && (s
            .projects
            .browser
            .listing
            .as_ref()
            .is_some_and(|listing| !listing.target_exists)
            || s.projects.browser.error.as_deref() == Some("That folder does not exist."));
    let submit = if creates { "Create folder" } else { "Add" };
    let placeholder = if browsing {
        "Type a folder path"
    } else {
        "Add a workspace from..."
    };
    let mut field = row()
        .h(46.0)
        .mx(4.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(6.0)
        .fill(field_fill(s, p))
        .field_focus(&s.picker_query, p)
        .child(el().size(16.0, 16.0).center().child(icon(
            if browsing { "folder" } else { "folder-plus" },
            p.muted,
        )))
        .child(
            rune::text_input(
                &s.picker_query,
                if browsing {
                    TextStyle::new(15.0, 400.0).family(rune::Family::SystemMono)
                } else {
                    TextStyle::new(15.0, 400.0).tracking(-0.15)
                },
            )
            .key("picker-search")
            .label(placeholder)
            .placeholder(placeholder, p.muted)
            .color(p.ink)
            .single_line()
            .grow(1.0)
            .min_w(0.0)
            .on_edit(|s: &mut State| {
                s.projects.menu_error = None;
                &mut s.picker_query
            })
            .on_input_key(|s: &mut State, event, tx| {
                if edits_text(event) {
                    s.picker_highlight = 0;
                }
                let browsing = s.picker_browsing;
                let folders = if browsing { browse_entries(s) } else { vec![] };
                let sources = if browsing { vec![] } else { add_sources(s) };
                let count = if browsing {
                    folders.len()
                } else {
                    sources.len()
                };
                let at = s.picker_highlight.min(count.saturating_sub(1));
                let command = event.mods.cmd || event.mods.ctrl;
                match event.key {
                    rune::Key::Down if count > 0 => {
                        s.picker_highlight = (at + 1).min(count - 1);
                        true
                    }
                    rune::Key::Home => {
                        s.picker_highlight = 0;
                        true
                    }
                    rune::Key::End if count > 0 => {
                        s.picker_highlight = count - 1;
                        true
                    }
                    rune::Key::Up => {
                        s.picker_highlight = at.saturating_sub(1);
                        true
                    }
                    rune::Key::Tab if browsing => {
                        browse_tab_complete(s, &folders);
                        true
                    }
                    rune::Key::Backspace if browsing && s.picker_query.text.is_empty() => {
                        s.open_picker(ProjectPicker::AddProject);
                        true
                    }
                    rune::Key::Enter if browsing => {
                        match folders.get(at) {
                            Some(name) if !command => browse_into(s, &name.clone()),
                            _ => s.add_browsed_project(),
                        }
                        true
                    }
                    rune::Key::Enter => {
                        if let Some((source, _, _)) = sources.get(at).cloned() {
                            run_add_source(s, tx, source);
                        }
                        true
                    }
                    rune::Key::Escape => {
                        if browsing {
                            s.open_picker(ProjectPicker::AddProject);
                        } else {
                            s.project_picker = None;
                        }
                        true
                    }
                    _ => s.undo_key(event, true),
                }
            }),
        );
    if browsing {
        field = field.child(
            row()
                .key("browse-add")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(if creates {
                    "Create folder and add workspace"
                } else {
                    "Add this folder"
                })
                .focusable(true)
                .h(28.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .border(1.0, p.frame)
                .hover(p.selected)
                .disabled(s.client.is_some() && !s.machine_connected(&s.machines.selected))
                .on_press(|s: &mut State| s.add_browsed_project())
                .child(label(submit, TextStyle::new(13.0, 600.0), p.ink).key(
                    if s.machines.selected == "local" {
                        "local-project-add"
                    } else {
                        "remote-project-add"
                    },
                ))
                .child(
                    meta_text(
                        if folders.is_empty() {
                            "Enter"
                        } else {
                            SUBMIT_CHORD
                        },
                        p.muted,
                    )
                    .px(4.0)
                    .radius(6.0)
                    .fill(p.selected),
                ),
        );
    }
    let mut card = popover("project-picker", "Add workspace", s, p)
        .role("dialog")
        .w(560.0)
        .max_h(480.0)
        .padding(4.0)
        .radius(12.0)
        .on_press(|_: &mut State| {});
    if machine_options(s).len() > 1 {
        let mut machines = col().key("project-machine-row").gap(1.0);
        for server in machine_options(s) {
            let online = s.machine_connected(&server);
            let title = s.machine_label(&server);
            let key = format!("project-machine-{server}");
            let locked = s
                .captain_editor
                .as_ref()
                .is_some_and(|editor| editor.server != server)
                || s.projects.return_picker == Some(ProjectPicker::CaptainScope)
                    && server != s.machines.selected;
            machines = machines.child(
                machine_option(&key, &title, &server, online, p)
                    .disabled(!online || locked)
                    .on_press(move |s: &mut State| s.choose_project_machine(&server)),
            );
        }
        card = card.child(machines);
    }
    card = card
        .child(
            el().key(if s.machines.selected == "local" {
                "local-project-path"
            } else {
                "remote-project-path"
            })
            .child(field),
        )
        .child(el().h(4.0));
    let mut list = col().gap(1.0).min_h(0.0).shrink().scroll(ScrollSpec {
        stick_bottom: false,
    });
    if browsing {
        if let Some(parent) = s
            .projects
            .browser
            .listing
            .as_ref()
            .and_then(|listing| listing.parent.clone())
        {
            let sep = if parent.contains('\\') && !parent.contains('/') {
                '\\'
            } else {
                '/'
            };
            let parent_dir = if parent.ends_with(['/', '\\']) {
                parent
            } else {
                format!("{parent}{sep}")
            };
            list = list.child(
                palette_row(
                    "browse-parent".into(),
                    "folder",
                    "..",
                    &parent_dir,
                    false,
                    p,
                )
                .min_h(34.0)
                .on_press(move |s: &mut State| {
                    s.picker_query.replace_text(&parent_dir);
                    s.picker_highlight = 0;
                }),
            );
        }
        for (index, name) in folders.iter().enumerate() {
            let entry = name.clone();
            list = list.child(
                palette_row(
                    format!("browse-{index}"),
                    "folder",
                    name,
                    "",
                    index == highlight,
                    p,
                )
                .min_h(34.0)
                .on_press(move |s: &mut State| browse_into(s, &entry)),
            );
        }
        if s.projects.browser.pending.is_some()
            || s.projects.browser.request.as_ref()
                != Some(&(s.machines.selected.clone(), query.clone()))
        {
            list = list.child(
                meta_text("Loading", p.muted)
                    .key("browse-loading")
                    .px(8.0)
                    .py(10.0),
            );
        } else if let Some(error) = &s.projects.browser.error {
            list = list.child(
                label(error, SMALL, p.muted)
                    .key("browse-error")
                    .wrap()
                    .px(8.0)
                    .py(10.0),
            );
        } else if folders.is_empty() {
            list = list.child(
                label(
                    if creates {
                        "No folder here yet."
                    } else {
                        "No folders inside this one."
                    },
                    SMALL,
                    p.muted,
                )
                .px(8.0)
                .py(10.0),
            );
        }
        if s.projects
            .browser
            .listing
            .as_ref()
            .is_some_and(|listing| listing.truncated)
        {
            list = list.child(
                label(
                    "Showing the first 200 folders. Type a more specific path.",
                    SMALL,
                    p.muted,
                )
                .key("browse-truncated")
                .wrap()
                .px(8.0)
                .py(6.0),
            );
        }
    } else {
        for (index, (source, title, detail)) in sources.iter().enumerate() {
            let glyph = match source {
                AddSource::LocalFolder => "folder-plus",
                AddSource::GitUrl => "branch",
                AddSource::SystemPicker => "folder",
                AddSource::Restore(_) => "archive",
            };
            let key = match source {
                AddSource::LocalFolder => "add-local-folder".to_string(),
                AddSource::GitUrl => "project-clone".to_string(),
                AddSource::SystemPicker => "project-open-folder".to_string(),
                AddSource::Restore(_) => format!("project-{index}"),
            };
            let source = source.clone();
            list = list.child(
                palette_row(key, glyph, title, detail, index == highlight, p)
                    .on_press_tx(move |s: &mut State, tx| run_add_source(s, tx, source.clone())),
            );
        }
        if sources.is_empty() {
            list = list.child(label("Nothing matches", SMALL, p.muted).px(8.0).py(10.0));
        }
    }
    card = card.child(list);
    if let Some(error) = &s.projects.menu_error {
        card = card.child(label(error, SMALL, p.danger).wrap().px(8.0).py(6.0));
    }
    let hints = if browsing {
        row()
            .h(34.0)
            .px(8.0)
            .gap(16.0)
            .center_y()
            .child(keycap("Enter", "Open folder", p))
            .child(keycap("Tab", "Complete", p))
            .child(keycap(SUBMIT_CHORD, submit, p))
            .child(spacer())
            .child(keycap("Esc", "Back", p))
    } else {
        row()
            .h(34.0)
            .px(8.0)
            .gap(16.0)
            .center_y()
            .child(keycap("↑↓", "Move", p))
            .child(keycap("Enter", "Choose", p))
            .child(spacer())
            .child(keycap("Esc", "Close", p))
    };
    card = card
        .child(el().h(0.5).w_full().mt(4.0).fill(p.line))
        .child(hints);
    if s.machines.selected != "local" {
        card = card.child(button("remote-project-cancel", "Cancel", p, |s| {
            s.project_picker = None
        }));
        card = el().key("remote-project-dialog").child(card);
    }
    let modal = modal_frame(
        "project-picker-backdrop",
        s,
        |s| s.project_picker = None,
        card,
    );
    if s.machines.selected == "local" {
        modal
    } else {
        el().key("remote-project-backdrop")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .child(modal)
    }
}
