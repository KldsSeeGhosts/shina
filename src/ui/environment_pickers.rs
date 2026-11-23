use super::*;

/// KSG has no crew binding. This adapts its ComposerAttachmentStrip chip and remove
/// action within the ordinary composer footer, without adding another draft editor.
pub(super) fn crew_draft_chip(s: &State, p: Palette, width: f32) -> El {
    let Some(target) = &s.current().crew_draft else {
        return el();
    };
    let server = target.server.clone();
    let captain = target.captain;
    let thread = s.current().id;
    let pending = s.current().client_creating.is_some();
    let text = format!(
        "{} {}'s crew",
        if pending { "Joining" } else { "Joins" },
        target.name
    );
    let blocked = s.crew_draft_unavailable().is_some();
    row()
        .key("composer-crew-chip")
        .role("group")
        .label(&text)
        .help(format!(
            "An independent thread in {}'s crew. It runs on its own and reports each finished turn to {}.",
            target.name, target.name
        ))
        .w(width)
        .min_w(0.0)
        .h(24.0)
        .px(4.0)
        .gap(4.0)
        .center_y()
        .radius(6.0)
        .border(1.0, if blocked { p.brass } else { p.line })
        .child(meta_text(&text, if blocked { p.brass } else { p.muted }).truncate().grow(1.0).min_w(0.0))
        .child(
            icon_button(
                "composer-crew-chip-remove",
                &format!("Don't add to {}'s crew", target.name),
                "close",
                p,
                move |s| {
                    if s.machines.selected == server && s.current().id == thread
                        && s.current().client_creating.is_none()
                        && s.current().crew_draft.as_ref().is_some_and(|target| {
                            target.server == server && target.captain == captain
                        })
                    {
                        s.current_mut().crew_draft = None;
                        s.current_mut().crew_draft_notice = None;
                    }
                },
            )
            .size(18.0, 18.0)
            .disabled(pending),
        )
}

/// Ported from KSG Code's BranchToolbarEnvironmentSelector and ComposerControl size="xs".
pub(super) fn machine_selector(s: &State, p: Palette, icon_only: bool) -> El {
    let title = s.machine_label(&s.machines.selected);
    row()
        .key("machine-selector")
        .role("button")
        .label("Run on")
        .h(if s.client.is_some() && s.viewport_width < 640.0 {
            28.0
        } else {
            24.0
        })
        .px(7.0)
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .shrink()
        .radius(6.0)
        .hover(p.selected)
        .preserve_focus_on_press(true)
        .cursor(rune::Cursor::Pointer)
        .disabled(s.client.is_none() || !s.can_retarget_draft())
        .on_press(|s: &mut State| {
            s.begin_composer_menu();
            s.machines.picker = !s.machines.picker;
        })
        .child(
            icon(
                if s.machines.selected == "local" {
                    "monitor"
                } else {
                    "cloud"
                },
                p.muted,
            )
            .size(12.0, 12.0),
        )
        .child(if icon_only {
            el()
        } else {
            label(title, SMALL, p.muted)
                .key("machine-selector-label")
                .truncate()
                .shrink()
                .min_w(0.0)
        })
        // KSG shows a spinner beside the selector while the draft's machine
        // change is still resolving; Rune's pending glyph is the clock.
        .child(if s.machines.draft_retarget.is_some() {
            icon("clock", p.coral)
                .size(12.0, 12.0)
                .key("machine-selector-pending")
        } else {
            el()
        })
        .child(icon("chevron", p.muted).size(12.0, 12.0))
}

/// The workspace choice's name. A crew draft with placement-v1 runs its worker in the project
/// checkout read only, so "Current checkout" says so there.
pub(super) fn env_mode_title(s: &State, mode: crate::model::ThreadEnvMode) -> &'static str {
    if mode == crate::model::ThreadEnvMode::Local && s.crew_draft_placement() {
        "Read-only checkout"
    } else {
        crate::branch_toolbar::resolve_env_mode_label(mode)
    }
}

/// What each workspace choice means for a crew worker, under its name in the picker, as KSG's
/// `PreviousWorktreeItemContent` puts a branch under its label. KSG has no crew placement.
pub(super) fn crew_env_mode_detail(mode: crate::model::ThreadEnvMode) -> &'static str {
    match mode {
        crate::model::ThreadEnvMode::Worktree => "Writes on its own branch",
        crate::model::ThreadEnvMode::Local => "Sees uncommitted changes, cannot edit",
    }
}

/// Ported from KSG Code's `BranchToolbarEnvModeSelector` and `ComposerControl` size `xs`.
pub(super) fn env_mode_trigger(
    s: &State,
    p: Palette,
    icon_only: bool,
    mode: crate::model::ThreadEnvMode,
) -> El {
    let title = env_mode_title(s, mode);
    let glyph = match mode {
        crate::model::ThreadEnvMode::Worktree => "branch",
        _ if s.crew_draft_placement() => "lock",
        _ => "folder",
    };
    row()
        .key("env-mode-trigger")
        .role("button")
        .label(format!("Workspace: {title}"))
        .help(title)
        .h(if s.client.is_some() && s.viewport_width < 640.0 {
            28.0
        } else {
            24.0
        })
        .px(7.0)
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .shrink()
        .radius(6.0)
        .hover(p.selected)
        .preserve_focus_on_press(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(|s: &mut State| {
            let opening = !s.env_mode_picker;
            s.begin_composer_menu();
            s.env_mode_picker = opening;
        })
        .child(icon(glyph, p.muted).size(12.0, 12.0))
        .child(if icon_only {
            el()
        } else {
            label(title, SMALL, p.muted)
                .key("env-mode-trigger-label")
                .truncate()
                .shrink()
                .min_w(0.0)
        })
        .child(icon("chevron", p.muted).size(12.0, 12.0))
}

/// Ported from KSG Code's `BranchToolbarBranchSelector` and `ComposerControl` size `xs`.
pub(super) fn base_branch_trigger(
    s: &State,
    p: Palette,
    base_branch: Option<&str>,
    start_from_origin: bool,
    max_w: f32,
) -> El {
    let t = s.current();
    let is_remote = base_branch.and_then(|name| {
        s.thread_vcs_repo_key(t).and_then(|key| {
            s.vcs_branches
                .get(&key)
                .and_then(|entry| entry.refs.iter().find(|r| r.name == name))
                .map(|r| r.is_remote == Some(true))
        })
    });
    let title = crate::branch_toolbar::resolve_branch_trigger_label(
        None,
        crate::model::ThreadEnvMode::Worktree,
        base_branch,
        is_remote,
        start_from_origin,
    );
    row()
        .key("base-branch-trigger")
        .role("button")
        .label(format!("Base branch: {title}"))
        .help(title.clone())
        .h(if s.client.is_some() && s.viewport_width < 640.0 {
            28.0
        } else {
            24.0
        })
        .px(7.0)
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .max_w(max_w)
        .shrink()
        .radius(6.0)
        .hover(p.selected)
        .preserve_focus_on_press(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(|s: &mut State| {
            let opening = !s.base_branch_picker;
            s.begin_composer_menu();
            s.base_branch_picker = opening;
            if opening {
                s.base_branch_query = rune::TextEdit::default();
                s.base_branch_query.focused = true;
                s.ensure_vcs_branches(true);
            }
        })
        .child(icon("branch", p.muted).size(12.0, 12.0))
        .child(
            label(
                title,
                META,
                if base_branch.is_some() {
                    p.ink
                } else {
                    p.muted
                },
            )
            .key("base-branch-trigger-label")
            .truncate()
            .shrink()
            .min_w(0.0),
        )
        .child(icon("chevron", p.muted).size(12.0, 12.0))
}

/// Read-only branch indicator for an active worktree thread, ported from KSG Code's
/// `BranchToolbarBranchSelector` / `BranchToolbarEnvModeSelector`.
pub(super) fn worktree_branch_indicator(
    p: Palette,
    branch: Option<&str>,
    worktree_path: &str,
    max_w: f32,
) -> El {
    let fallback = crate::branch_toolbar::resolve_locked_workspace_label(
        Some(worktree_path),
        crate::model::ThreadEnvMode::Worktree,
    );
    let title = branch.unwrap_or(&fallback);
    row()
        .key("worktree-branch-indicator")
        .label(format!("Worktree branch: {title}"))
        .help(worktree_path)
        .h(24.0)
        .px(7.0)
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .max_w(max_w)
        .shrink()
        .child(icon("branch", p.muted).size(12.0, 12.0))
        .child(label(title, META, p.muted).truncate().shrink().min_w(0.0))
}

pub(crate) fn step_reasoning_effort(state: &mut State, direction: isize) -> bool {
    let t = state.current();
    if t.status.busy()
        || t.configuring
        || t.archived()
        || t.request.is_some()
        || crate::descriptors::read_prompt_ultrathink(&t.draft.text)
            == crate::descriptors::PromptUltrathink::Body
    {
        return false;
    }
    let descriptors = state.active_model_descriptors();
    let controls = crate::descriptors::split_column_and_footer_controls(&descriptors);
    let levels = match &controls.effort {
        Some(desc @ crate::descriptors::ProviderOptionDescriptor::Select { .. }) => {
            let choices = crate::descriptors::selectable_provider_option_choices(desc);
            if choices.is_empty() {
                t.thinking_levels.clone()
            } else {
                choices.into_iter().map(|c| c.id).collect()
            }
        }
        _ => t.thinking_levels.clone(),
    };
    if levels.is_empty() {
        return false;
    }
    let current = levels
        .iter()
        .position(|level| level == &t.thinking)
        .unwrap_or(0);
    let next =
        (current as isize + direction).clamp(0, levels.len().saturating_sub(1) as isize) as usize;
    let level = levels[next].clone();
    state.set_composer_thinking_sticky(level);
    true
}

pub(crate) fn cycle_model_picker_harness(state: &mut State, direction: isize) -> bool {
    let snap = picker_snapshot(state);
    if snap.sidebar_entries.is_empty() {
        return false;
    }
    let next = crate::model_picker::adjacent_model_picker_provider(
        &snap.sidebar_entries,
        &snap.selected_tab,
        direction,
        &snap.locked_disabled_ids,
        &snap.selectable_unavailable_ids,
    );
    state.model_search.replace_text("");
    state.model_picker_tab = Some(next);
    true
}

pub(crate) fn jump_model_picker_index(state: &mut State, index: usize) -> bool {
    let t = state.current();
    if t.status.busy() || t.configuring {
        return false;
    }
    let snap = picker_snapshot(state);
    let Some(chosen) = snap
        .visible_models
        .into_iter()
        .filter(|m| {
            state
                .model_disabled_reason_for(&m.instance_id, &m.slug)
                .is_none()
        })
        .nth(index)
    else {
        return false;
    };
    state.select_model_from_picker(&chosen.instance_id, &chosen.slug);
    finish_model_pick(state);
    true
}

pub(super) fn machine_options(s: &State) -> Vec<String> {
    std::iter::once("local".to_owned())
        .chain(
            s.client
                .iter()
                .flat_map(|client| client.linked.keys().cloned()),
        )
        .collect()
}

/// The same machine row is used in Run on, Add workspace and Lives on.
pub(super) fn machine_option(key: &str, title: &str, server: &str, online: bool, p: Palette) -> El {
    let item = menu_item(
        key,
        title,
        if server == "local" {
            "monitor"
        } else {
            "cloud"
        },
        p,
    )
    .disabled(!online);
    if online {
        item
    } else {
        item.child(meta_text("Offline", p.muted))
    }
}

/// Ported from KSG Code's BranchToolbarEnvironmentSelector SelectPopup/SelectGroup.
pub(super) fn machine_picker(s: &State, p: Palette, anchor: &str) -> El {
    let mut menu = col()
        .key("machine-picker")
        .role("menu")
        .focus_group()
        .label("Run on")
        .w(270.0)
        .max_h(360.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(4.0)
        .gap(1.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .child(meta_label("Run on", p.muted).px(8.0).py(6.0));
    for server in machine_options(s) {
        let title = s.machine_label(&server);
        let key = format!("machine-option-{server}");
        menu = menu.child(
            machine_option(&key, &title, &server, s.machine_connected(&server), p).on_press(
                move |s: &mut State| {
                    s.retarget_draft(&server);
                    s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
                },
            ),
        );
    }
    menu = menu.child(
        menu_item("link-machine", "Add to another machine", "plus", p).on_press(|s: &mut State| {
            s.machines.picker = false;
            s.machines.dialog = true;
        }),
    );
    menu.anchored_fit(anchor, rune::Side::Top, rune::Align::Start, 6.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.machines.picker = false;
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// Ported from KSG Code's `BranchToolbarEnvModeSelector` `SelectPopup`.
pub(super) fn env_mode_picker(s: &State, p: Palette, anchor: &str) -> El {
    let force_worktree = s.is_multi_model_active();
    let active = if force_worktree {
        crate::model::ThreadEnvMode::Worktree
    } else {
        s.effective_env_mode(s.current())
    };
    let mut menu = col()
        .key("env-mode-picker")
        .role("menu")
        .focus_group()
        .label("Workspace")
        .w(if s.crew_draft_placement() {
            280.0
        } else {
            240.0
        })
        .padding(4.0)
        .gap(1.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .child(meta_label("Workspace", p.muted).px(8.0).py(6.0));
    let crew = s.crew_draft_placement();
    for (mode, slug, glyph) in [
        (
            crate::model::ThreadEnvMode::Local,
            "local",
            if crew { "lock" } else { "folder" },
        ),
        (crate::model::ThreadEnvMode::Worktree, "worktree", "branch"),
    ] {
        let title = env_mode_title(s, mode);
        let selected = active == mode;
        let disabled = force_worktree && mode == crate::model::ThreadEnvMode::Local;
        let name = label(title, TextStyle::new(13.0, 500.0), p.ink)
            .truncate()
            .min_w(0.0);
        let mut item = row()
            .key(format!("env-mode-option-{slug}"))
            .role("menuitemradio")
            .checked(selected)
            .label(if crew {
                format!("{title}. {}", crew_env_mode_detail(mode))
            } else {
                title.to_owned()
            })
            .focusable(!disabled)
            .disabled(disabled)
            .opacity(if disabled { 0.55 } else { 1.0 })
            .center_y()
            .gap(8.0)
            .h(if crew { 44.0 } else { 30.0 })
            .px(8.0)
            .radius(6.0)
            .fill(if selected {
                p.selected
            } else {
                Color::TRANSPARENT
            })
            .hover(if disabled {
                Color::TRANSPARENT
            } else {
                p.selected
            })
            .child(icon(glyph, p.muted).size(14.0, 14.0))
            .child(if crew {
                col().gap(2.0).grow(1.0).min_w(0.0).child(name).child(
                    label(crew_env_mode_detail(mode), SMALL, p.muted)
                        .key(format!("env-mode-option-{slug}-detail"))
                        .truncate()
                        .min_w(0.0),
                )
            } else {
                name.grow(1.0)
            });
        if !disabled {
            item = item
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    s.set_draft_env_mode(mode);
                    s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
                });
        }
        if selected {
            item = item.child(icon("check", p.sea));
        }
        menu = menu.child(item);
    }
    menu.anchored_fit(anchor, rune::Side::Top, rune::Align::Start, 6.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.env_mode_picker = false;
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// Ported from KSG Code's `BranchPicker` and `BranchToolbarBranchSelector`.
pub(super) fn base_branch_picker(s: &State, p: Palette, anchor: &str) -> El {
    let t = s.current();
    let selected_branch = t.base_branch.as_deref();
    let start_from_origin = t.effective_start_from_origin();
    let query = s.base_branch_query.text.trim().to_ascii_lowercase();
    let repo_key = s.thread_vcs_repo_key(t);
    let project_cwd = t
        .project
        .and_then(|id| s.project(Some(id)))
        .map(|proj| proj.path.to_string_lossy().into_owned());
    let entry = repo_key.as_ref().and_then(|k| s.vcs_branches.get(k));

    let header = row().center_y().gap(8.0).px(10.0).py(8.0).child(
        row()
            .center_y()
            .gap(6.0)
            .h(30.0)
            .px(10.0)
            .radius(6.0)
            .fill(p.selected)
            .field_focus(&s.base_branch_query, p)
            .grow(1.0)
            .min_w(0.0)
            .child(icon("search", p.muted))
            .child(
                text_input(&s.base_branch_query, TextStyle::new(13.0, 400.0))
                    .key("base-branch-search")
                    .label("Search refs")
                    .placeholder("Search refs...", p.muted)
                    .color(p.ink)
                    .caret(p.ink, p.coral_bg)
                    .grow(1.0)
                    .min_w(0.0)
                    .autofocus()
                    .on_edit(|s: &mut State| &mut s.base_branch_query)
                    .on_input_key(|s: &mut State, event, tx| {
                        if matches!(event.key, rune::Key::Down)
                            && !event.mods.cmd
                            && !event.mods.ctrl
                            && !event.mods.alt
                            && !event.mods.shift
                        {
                            s.base_branch_query.focused = false;
                            tx.focus("base-branch-list", true);
                            return true;
                        }
                        false
                    }),
            ),
    );

    let mut list = col()
        .key("base-branch-list")
        .role("listbox")
        .label("Branches")
        .focus_group()
        .w_full()
        .max_h(224.0)
        .pb(4.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });

    if let Some(err) = entry.and_then(|e| e.error.as_deref()) {
        list = list.child(
            label(err, SMALL, p.danger)
                .key("base-branch-error")
                .wrap()
                .px(12.0)
                .py(8.0),
        );
    } else if entry.is_none_or(|e| e.loading && e.refs.is_empty()) {
        list = list.child(label("Loading refs...", SMALL, p.muted).px(12.0).py(8.0));
    } else if let Some(entry) = entry {
        let ordered = crate::branch_toolbar::order_branches_for_picker(&entry.refs);
        let filtered: Vec<&crate::vcs::VcsRef> = ordered
            .into_iter()
            .filter(|r| {
                crate::branch_toolbar::should_include_branch_picker_item(
                    &r.name, &query, None, None,
                )
            })
            .collect();
        if filtered.is_empty() {
            list = list.child(label("No refs found.", SMALL, p.muted).px(12.0).py(8.0));
        } else {
            for r in filtered {
                let name = r.name.clone();
                let selected = selected_branch == Some(name.as_str());
                let has_secondary_worktree = r
                    .worktree_path
                    .as_deref()
                    .is_some_and(|wp| project_cwd.as_deref().is_some_and(|cwd| wp != cwd));
                let badge = if r.current {
                    Some("current")
                } else if has_secondary_worktree {
                    Some("worktree")
                } else if r.is_remote == Some(true) {
                    Some("remote")
                } else if r.is_default {
                    Some("default")
                } else {
                    None
                };
                let mut item = row()
                    .key(format!("base-branch-option-{name}"))
                    .role("option")
                    .checked(selected)
                    .label(&name)
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .center_y()
                    .gap(8.0)
                    .h(30.0)
                    .mx(4.0)
                    .px(8.0)
                    .radius(6.0)
                    .fill(if selected {
                        p.coral_bg
                    } else {
                        Color::TRANSPARENT
                    })
                    .hover(if selected { p.coral_bg } else { p.selected })
                    .focus_fill(p.selected)
                    .focus_selected(selected)
                    .on_press(move |s: &mut State| {
                        s.set_draft_base_branch(name.clone());
                        s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
                    })
                    .child(
                        label(&r.name, TextStyle::new(13.0, 500.0), p.ink)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    );
                if selected {
                    item = item.child(icon("check", p.sea));
                }
                if let Some(badge) = badge {
                    item = item.child(meta_text(badge, p.faint));
                }
                list = list.child(item);
            }
        }
    }

    let origin_row = row()
        .key("start-from-remote-row")
        .center_y()
        .justify_between()
        .gap(12.0)
        .px(12.0)
        .py(8.0)
        .help("Uses the latest commit of the base branch's tracked remote branch, even if its name differs. Otherwise uses the same-named branch on origin. Uses your local branch if that remote branch is missing.")
        .child(
            row()
                .center_y()
                .gap(6.0)
                .min_w(0.0)
                .child(icon("cloud", p.muted).size(12.0, 12.0))
                .child(label("Start from remote", SMALL, p.muted).truncate()),
        )
        .child(captain_ui::switch(
            s,
            "start-from-remote-toggle",
            "Start worktree from remote",
            start_from_origin,
            p,
            |s: &mut State| {
                let next = !s.current().effective_start_from_origin();
                s.set_draft_start_from_origin(next);
            },
        ));

    col()
        .key("base-branch-picker")
        .role("dialog")
        .label("Base branch")
        .w(320.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .child(header)
        .child(rule(p))
        .child(list)
        .child(rule(p))
        .child(origin_row)
        .anchored_fit(anchor, rune::Side::Top, rune::Align::Start, 6.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.base_branch_picker = false;
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// Ported from KSG Code's AuthorizedClientsSettings QR share panel. Shina uses
/// its direct-tailnet link rather than KSG's hosted pairing endpoint.
pub(super) fn mobile_pairing_card(s: &State, p: Palette) -> El {
    let mut card = col()
        .key("mobile-pairing")
        .w_full()
        .gap(8.0)
        .child(label("Pair a phone", BODY, p.ink))
        .child(
            label(
                "The phone needs Tailscale and access to this computer's tailnet.",
                SMALL,
                p.muted,
            )
            .wrap(),
        );
    if let Some(share) = s
        .machines
        .mobile_share
        .as_ref()
        .filter(|share| !share.link.expired(crate::lifecycle::now()))
    {
        let (pixels, size) = share.pixels(s.artwork.device_scale());
        card = card
            .child(
                row().w_full().center().child(
                    el().key("mobile-pairing-qr")
                        .role("img")
                        .label("Phone pairing link. Scan with Shina on your phone.")
                        .size(size, size)
                        .pixels(&pixels),
                ),
            )
            .child(
                label(share.link.encode(), SMALL, p.ink)
                    .key("mobile-pairing-link")
                    .w_full()
                    .wrap_anywhere()
                    .selectable(true),
            )
            .child(label("One use. Expires after five minutes.", SMALL, p.muted).wrap())
            .child(
                row()
                    .flex_wrap()
                    .gap(8.0)
                    .child(
                        settings_ui::settings_button("mobile-pairing-copy", "Copy link", p, |_| {})
                            .on_press_tx(|s: &mut State, tx| {
                                if let Some(share) = &s.machines.mobile_share
                                    && !share.link.expired(crate::lifecycle::now())
                                {
                                    tx.copy(share.link.encode());
                                }
                            }),
                    )
                    .child(settings_ui::settings_button(
                        "mobile-pairing-hide",
                        "Hide link",
                        p,
                        |s| s.machines.mobile_share = None,
                    )),
            );
    } else {
        card = card
            .child(meta_label("This computer's tailnet host", p.muted))
            .child(
                text_input(&s.machines.mobile_host, BODY)
                    .key("mobile-pairing-host")
                    .label("This computer's tailnet host")
                    .placeholder("studio-mac", p.muted)
                    .single_line()
                    .w_full()
                    .padding(8.0)
                    .radius(12.0)
                    .field_focus(&s.machines.mobile_host, p)
                    .color(p.ink)
                    .disabled(s.machines.pending.is_some())
                    .on_edit(|s: &mut State| &mut s.machines.mobile_host),
            )
            .child(
                settings_ui::settings_button(
                    "mobile-pairing-create",
                    "Create phone link",
                    p,
                    |s| s.machine_action("pair-mobile", ""),
                )
                .disabled(
                    s.machines.pending.is_some() || s.machines.mobile_host.text.trim().is_empty(),
                ),
            );
    }
    card
}

/// Ported from KSG Code's settings/AddEnvironmentDialog and AuthorizedClientsSettings.
pub(super) fn machine_panel(s: &State, p: Palette, embedded: bool) -> El {
    let busy = s.machines.pending.is_some();
    let mut card = col()
        .key("machine-dialog")
        .role(if embedded { "group" } else { "dialog" })
        .label("Linked machines")
        .w_full()
        .gap(12.0);
    if !embedded {
        card = card.child(
            row()
                .center_y()
                .child(label("Linked machines", DIALOG_TITLE, p.ink))
                .child(spacer())
                .child(icon_button("close-machines", "Close", "close", p, |s| {
                    s.machines.dialog = false;
                    s.settings_menu = false;
                    s.machines.code.replace_text("");
                    s.machines.peer_code.replace_text("");
                    s.machines.mobile_share = None;
                    s.machines.notice = None;
                })),
        );
    }
    card = card.child(mobile_pairing_card(s, p)).child(rule(p));
    card=card.child(label("Create a code on the other machine with shina pair. Paste its MagicDNS host and code here.",SMALL,p.muted).wrap())
        .child(meta_label("Host",p.muted))
        .child(text_input(&s.machines.host,BODY).key("machine-host").label("Host").placeholder("studio-mac",p.muted)
            .single_line().w_full().padding(8.0).radius(12.0).field_focus(&s.machines.host, p).color(p.ink)
            .disabled(busy).on_edit(|s: &mut State| &mut s.machines.host))
        .child(meta_label("Pairing code",p.muted))
        .child(text_input(&s.machines.code,BODY).key("machine-code").label("Pairing code").placeholder("PAIRCODE",p.muted)
            .single_line().w_full().padding(8.0).radius(12.0).field_focus(&s.machines.code, p).color(p.ink)
            .disabled(busy).on_edit(|s: &mut State| &mut s.machines.code))
        .child(primary_button("machine-link-submit","Link machine",p,|s| s.machine_action("link","")).disabled(busy))
        .child(row().flex_wrap().gap(8.0)
            .child(settings_ui::settings_button("machine-pair","Create a code for this host",p,|s| s.machine_action("pair","")).disabled(busy))
            .child(settings_ui::settings_button("machine-devices","Authorized devices",p,|s| s.machine_action("devices","")).disabled(busy)));
    if let Some(notice) = &s.machines.notice {
        card = card.child(
            label(notice, SMALL, p.muted)
                .key("machine-notice")
                .wrap_anywhere()
                .selectable(true),
        );
    }
    if let Some(client) = &s.client {
        for (id, remote) in &client.linked {
            let server = id.clone();
            let toggle_id = id.clone();
            let enabled = remote.enabled();
            let version = remote.version().unwrap_or_else(|| {
                s.machines
                    .views
                    .get(id)
                    .and_then(|view| view["updateStatus"]["runningVersion"].as_str())
                    .unwrap_or("unknown")
                    .to_owned()
            });
            card = card.child(
                col()
                    .key(format!("settings-linked-{id}"))
                    .gap(6.0)
                    .child(
                        row()
                            .center_y()
                            .gap(8.0)
                            .child(
                                col()
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .gap(4.0)
                                    .child(
                                        label(
                                            remote
                                                .remote
                                                .as_ref()
                                                .map(|link| link.host.as_str())
                                                .unwrap_or(id),
                                            TextStyle::new(14.0, 500.0),
                                            p.ink,
                                        )
                                        .truncate()
                                        .min_w(0.0),
                                    )
                                    .child(
                                        row()
                                            .gap(8.0)
                                            .flex_wrap()
                                            .child(meta_text(version, p.muted))
                                            .child(meta_text(
                                                if !enabled {
                                                    "Disabled"
                                                } else if s.machine_connected(id) {
                                                    "Reachable"
                                                } else {
                                                    "Offline"
                                                },
                                                p.muted,
                                            )),
                                    ),
                            )
                            .child(
                                settings_ui::destructive_button(
                                    &format!("unlink-{id}"),
                                    "Unlink and revoke",
                                    p,
                                    move |s| s.machine_action("unlink", &server),
                                )
                                .disabled(busy),
                            ),
                    )
                    .child(
                        row()
                            .key(format!("settings-linked-enabled-row-{id}"))
                            .center_y()
                            .gap(8.0)
                            .child(meta_text("Enabled", p.muted))
                            .child(spacer())
                            .child(
                                captain_ui::switch(
                                    s,
                                    &format!("linked-enabled-{id}"),
                                    "Enabled",
                                    enabled,
                                    p,
                                    move |s| s.toggle_machine_enabled(&toggle_id),
                                )
                                .disabled(busy),
                            ),
                    ),
            );
        }
    }
    for device in s.machines.devices.as_array().into_iter().flatten() {
        if !device["revokedAt"].is_null() {
            continue;
        }
        let id = device["deviceId"].as_str().unwrap_or("").to_owned();
        let grant_id = id.clone();
        let maintenance = device["scopes"]
            .as_array()
            .is_some_and(|scopes| scopes.iter().any(|scope| scope == "maintain"));
        let update = device["scopes"]
            .as_array()
            .is_some_and(|scopes| scopes.iter().any(|scope| scope == "update"));
        let paired = device["scopes"]
            .as_array()
            .is_some_and(|scopes| scopes.iter().any(|scope| scope == "paired"));
        let title = format!("Device {}", &id[..id.len().min(8)]);
        card = card.child(
            row()
                .center_y()
                .gap(8.0)
                .child(label(title, SMALL, p.muted).grow(1.0))
                .child(meta_text("Updates", p.muted))
                .child(
                    captain_ui::switch(
                        s,
                        &format!("device-update-{id}"),
                        if maintenance {
                            "Local maintenance permits updates"
                        } else if paired {
                            "Pairing permits updates"
                        } else {
                            "Allow updates"
                        },
                        update || maintenance || paired,
                        p,
                        move |s| {
                            s.machine_action(
                                if update {
                                    "revoke-update"
                                } else {
                                    "grant-update"
                                },
                                &grant_id,
                            )
                        },
                    )
                    .disabled(busy || maintenance || paired),
                )
                .child(
                    settings_ui::destructive_button(
                        &format!("revoke-{id}"),
                        "Revoke",
                        p,
                        move |s| s.machine_action("revoke", &id),
                    )
                    .disabled(busy),
                ),
        );
    }
    // The existing AddEnvironmentDialog fields also redeem a server-to-server peer code.
    // Outbound links belong to the selected server, not to this desktop's client links.
    card=card.child(rule(p)).child(meta_label("Peers",p.muted))
        .child(label("A peer link lets this server delegate work. Create its code on the target with shina pair --peer.",SMALL,p.muted).wrap());
    let peer_busy = busy || s.machines.selected != "local";
    for peer in &s.peers {
        let id = peer.server_id.clone();
        let reachable = peer.last_error.is_none()
            && peer
                .last_reached_at
                .is_some_and(|at| crate::lifecycle::now().saturating_sub(at) < 90_000);
        card = card.child(
            row()
                .key(format!("peer-row-{id}"))
                .center_y()
                .gap(8.0)
                .child(
                    label(&peer.label, SMALL, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(meta_text(
                    if reachable { "Reachable" } else { "Offline" },
                    p.muted,
                ))
                .child(
                    button(
                        &format!("peer-unlink-{id}"),
                        "Unlink and revoke",
                        p,
                        move |s| s.machine_action("peer-unlink", &id),
                    )
                    .disabled(peer_busy),
                ),
        );
    }
    card = card
        .child(meta_label("Peer host", p.muted))
        .child(
            text_input(&s.machines.peer_host, BODY)
                .key("peer-host")
                .label("Peer host")
                .single_line()
                .w_full()
                .padding(8.0)
                .radius(12.0)
                .field_focus(&s.machines.peer_host, p)
                .color(p.ink)
                .disabled(peer_busy)
                .on_edit(|s: &mut State| &mut s.machines.peer_host),
        )
        .child(meta_label("Peer code", p.muted))
        .child(
            text_input(&s.machines.peer_code, BODY)
                .key("peer-code")
                .label("Peer code")
                .single_line()
                .w_full()
                .padding(8.0)
                .radius(12.0)
                .field_focus(&s.machines.peer_code, p)
                .color(p.ink)
                .disabled(peer_busy)
                .on_edit(|s: &mut State| &mut s.machines.peer_code),
        )
        .child(
            primary_button("peer-link-submit", "Link peer", p, |s| {
                s.machine_action("peer-link", "")
            })
            .disabled(peer_busy),
        );
    if s.machines.selected != "local" {
        card = card.child(
            label(
                "Manage this server's peer links on that host with shina peer.",
                SMALL,
                p.muted,
            )
            .wrap(),
        );
    }
    card
}

pub(super) fn machine_dialog(s: &State, p: Palette) -> El {
    el().key("machines-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(0.28))
        .center()
        .modal(false, |s: &mut State| {
            s.machines.dialog = false;
            s.machines.code.replace_text("");
            s.machines.peer_code.replace_text("");
            s.machines.mobile_share = None;
            s.machines.notice = None;
        })
        .child(
            machine_panel(s, p, false)
                .w((s.viewport_width - 32.0).clamp(280.0, 520.0))
                .max_h((s.viewport_height - 64.0).max(240.0))
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .padding(16.0)
                .radius(12.0)
                .fill(p.card)
                .border(1.0, p.frame)
                .box_shadow(soft_shadow(p)),
        )
}
