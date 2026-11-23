use super::*;

/// In-place captain selector in the composer footer, ported from KSG Code's
/// `BranchToolbarEnvironmentSelector` (`ComposerControl` size `xs`) for the trigger and
/// `ModelPickerContent` for the searchable popover with a trailing creation action.
/// Captains are Shina-only (KSG Code has no captains).
pub(super) fn captain_selector_trigger(s: &State, p: Palette, icon_only: bool) -> El {
    let t = s.current();
    let active = t.captain.and_then(|id| s.captains.get(id).map(|c| (id, c)));
    let locked = t.status.busy() || t.configuring || t.archived() || !t.is_draft();
    let title = active
        .map(|(_, c)| c.name.clone())
        .unwrap_or_else(|| "No captain".into());
    let mut trigger = row()
        .key("captain-selector")
        .role("button")
        .label(format!("Captain: {title}"))
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
        .focusable(!locked)
        .cursor(if locked {
            rune::Cursor::Default
        } else {
            rune::Cursor::Pointer
        })
        .opacity(if locked { 0.65 } else { 1.0 })
        .disabled(locked);
    trigger = trigger.preserve_focus_on_press(true);
    if !locked {
        trigger = trigger.on_press(|s: &mut State| {
            s.begin_composer_menu();
            s.captain_picker = !s.captain_picker;
            if s.captain_picker {
                s.captain_project = Some(s.current().project);
                s.captain_picker_query = rune::TextEdit::default();
                s.captain_picker_query.focused = true;
                s.captain_picker_highlight = 0;
            }
        });
    }
    let lead = match active {
        Some((id, _)) => avatar(s, p, id, 14.0, s.captain_status(id)),
        None => icon("user", p.muted).size(12.0, 12.0),
    };
    trigger
        .child(lead)
        .child(if icon_only {
            el()
        } else {
            label(title, SMALL, if active.is_some() { p.ink } else { p.muted })
                .key("captain-selector-label")
                .truncate()
                .shrink()
                .min_w(0.0)
        })
        .child(icon("chevron", p.muted).size(12.0, 12.0))
}

/// Ported from KSG Code's `ModelPickerContent` (searchable list in a bordered popover card)
/// and `BranchToolbarEnvironmentSelector` (anchored above the composer footer control).
/// Captains, portraits, and the "No captain" / "New captain" rows are Shina-specific.
pub(super) fn captain_picker(s: &State, p: Palette, anchor: &str) -> El {
    let t = s.current();
    let selected_captain = t.captain;
    let query = s.captain_picker_query.text.trim().to_lowercase();
    let filtered: Vec<usize> = s
        .picker_captains()
        .filter(|&id| {
            if query.is_empty() {
                return true;
            }
            let c = &s.captains[id];
            c.name.to_lowercase().contains(&query) || c.role.to_lowercase().contains(&query)
        })
        .collect();

    let header = row().center_y().gap(6.0).px(10.0).py(8.0).child(
        row()
            .center_y()
            .gap(6.0)
            .h(30.0)
            .px(10.0)
            .radius(6.0)
            .fill(p.selected)
            .field_focus(&s.captain_picker_query, p)
            .grow(1.0)
            .min_w(0.0)
            .child(icon("search", p.muted))
            .child(
                text_input(&s.captain_picker_query, TextStyle::new(13.0, 400.0))
                    .key("captain-search")
                    .label("Search captains")
                    .placeholder("Search captains", p.muted)
                    .color(p.ink)
                    .caret(p.ink, p.coral_bg)
                    .grow(1.0)
                    .min_w(0.0)
                    .on_edit(|s: &mut State| &mut s.captain_picker_query)
                    .on_input_key(|s: &mut State, event, tx| {
                        if matches!(event.key, rune::Key::Down) {
                            s.captain_picker_query.focused = false;
                            tx.focus("captain-list", true);
                            return true;
                        }
                        false
                    }),
            ),
    );

    let mut list = col()
        .key("captain-list")
        .role("listbox")
        .label("Captains")
        .focus_group()
        .w_full()
        .max_h(260.0)
        .pb(4.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });

    let no_captain_selected = selected_captain.is_none();
    let mut no_captain = row()
        .key("choose-no-captain")
        .role("option")
        .label("No captain (plain thread)")
        .checked(no_captain_selected)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .center_y()
        .gap(10.0)
        .h(34.0)
        .mx(4.0)
        .pl(12.0)
        .pr(8.0)
        .radius(6.0)
        .fill(if no_captain_selected {
            p.coral_bg
        } else {
            Color::TRANSPARENT
        })
        .hover(if no_captain_selected {
            p.coral_bg
        } else {
            p.selected
        })
        .focus_fill(p.selected)
        .focus_selected(no_captain_selected)
        .on_press(|s: &mut State| {
            s.captain_picker = false;
            s.captain_project = None;
            s.projects.captain_target = None;
            if s.current().captain.is_some() {
                let project = s.current().project;
                s.new_thread_in(project);
            }
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
        .child(icon("user", p.muted).size(16.0, 16.0))
        .child(
            label("No captain", TextStyle::new(14.0, 500.0), p.ink)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        );
    if no_captain_selected {
        no_captain = no_captain.child(icon("check", p.sea));
    }
    list = list.child(no_captain);

    if !filtered.is_empty() {
        list = list.child(meta_label("Captains", p.muted).px(16.0).pt(6.0).pb(4.0));
    }

    let filtered_empty = filtered.is_empty() && !query.is_empty();
    for captain in filtered {
        let profile = &s.captains[captain];
        let state = s.captain_status(captain);
        let selected = selected_captain == Some(captain);
        let mut item = row()
            .key(format!("choose-captain-{captain}"))
            .role("option")
            .label(format!("Open Captain {}, {}", profile.name, state.label()))
            .checked(selected)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .center_y()
            .gap(10.0)
            .h(36.0)
            .mx(4.0)
            .pl(12.0)
            .pr(8.0)
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
                s.captain_picker = false;
                s.open_captain(captain);
            })
            .child(avatar(s, p, captain, 18.0, state))
            .child(
                col()
                    .gap(1.0)
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        label(&profile.name, TextStyle::new(14.0, 500.0), p.ink)
                            .truncate()
                            .min_w(0.0),
                    )
                    .child(if profile.role.is_empty() {
                        el()
                    } else {
                        label(&profile.role, SMALL, p.muted).truncate().min_w(0.0)
                    }),
            );
        if selected {
            item = item.child(icon("check", p.sea));
        }
        list = list.child(item);
    }
    if filtered_empty {
        list = list.child(
            label("No matching captains", SMALL, p.muted)
                .px(16.0)
                .py(8.0),
        );
    }

    let mut card = col()
        .key("captain-picker")
        .role("dialog")
        .label("Choose a captain")
        .w(PICKER_WIDTH)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(header)
        .child(list)
        .child(rule(p));
    let new_captain = row()
        .key("picker-new-captain")
        .role("button")
        .label("New captain")
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .center_y()
        .gap(10.0)
        .h(34.0)
        .mx(4.0)
        .pl(12.0)
        .pr(8.0)
        .radius(6.0)
        .hover(p.selected)
        .focus_fill(p.selected)
        .on_press(|s: &mut State| s.begin_new_captain())
        .child(icon("plus", p.muted))
        .child(label("New captain", TextStyle::new(14.0, 500.0), p.ink));
    card = card.child(col().py(4.0).child(new_captain));

    card.anchored_fit(anchor, rune::Side::Top, rune::Align::Start, 6.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.cancel_captain_flow();
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// A portrait's cell in the captain editor's grid: the 40 pt face and 4 pt each side.
pub(super) const PICKER_CELL: f32 = 48.0;
pub(super) fn captain_editor(s: &State, p: Palette, height: f32) -> El {
    let editor = s.captain_editor.as_ref().unwrap();
    let editing = editor.editing.is_some();
    let promotion = editor.promotion.as_ref();
    let pending = promotion.is_some_and(|promotion| promotion.pending.is_some());
    let restore = promotion.is_some_and(|promotion| promotion.restore.is_some());
    let profile_locked = pending || restore;
    let promotion_reason = promotion.and_then(|promotion| {
        s.captain_promotion_unavailable(&editor.server, promotion.thread)
            .or_else(|| pending.then(|| "Waiting for the machine to confirm.".into()))
            .or_else(|| {
                promotion
                    .preview
                    .as_ref()
                    .and_then(|preview| preview["blockers"].as_array())
                    .and_then(|blockers| blockers.first())
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .or_else(|| {
                promotion
                    .preview
                    .is_none()
                    .then(|| "Waiting for the promotion preview.".into())
            })
    });
    let profile = editor.profile();
    let field = field_fill(s, p);
    // The portrait grid is ported from KSG Code's `ProjectIconPickerDialog` emoji grid: square
    // cells with `gap-1`, `rounded-md` and a transparent border, the chosen one bordered on an
    // accent fill. Names stay in the accessible label, and the chosen one shows by the heading.
    // Six columns fit the editor's fixed 480 px card, so the grid is the same at every width.
    const COLUMNS: usize = 6;
    let chosen = editor.portrait.min(crate::visuals::PORTRAITS - 1);
    // Each 40 pt face sits in a 48 pt cell for its hover fill. The rows reach 4 pt past the
    // fields on each side and space the cells evenly, so the outer faces' edges line up with the
    // fields' edges and the grid spans the same width.
    let cell_pad = (PICKER_CELL - PICKER_PORTRAIT) / 2.0;
    let mut portraits = col().key("captain-portraits").gap(4.0).mx(-cell_pad);
    for first in (0..crate::visuals::PORTRAITS).step_by(COLUMNS) {
        let mut line = row().justify_between();
        for portrait in first..(first + COLUMNS).min(crate::visuals::PORTRAITS) {
            let selected = chosen == portrait;
            line = line.child(
                col()
                    .key(format!("captain-portrait-{portrait}"))
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label(format!(
                        "Use {} portrait",
                        crate::visuals::PORTRAIT_NAMES[portrait]
                    ))
                    .checked(selected)
                    .disabled(profile_locked)
                    .focusable(true)
                    .size(PICKER_CELL, PICKER_CELL)
                    .center()
                    .radius(6.0)
                    .hover(p.selected)
                    .on_press(move |s: &mut State| {
                        s.captain_editor.as_mut().unwrap().portrait = portrait;
                    })
                    // The selected portrait's own frame turns the ring colour. There is no wash
                    // around it, so all twelve keep the same outer bounds.
                    .child(
                        portrait_face(
                            s,
                            p,
                            portrait,
                            PICKER_PORTRAIT,
                            Status::Ready,
                            selected.then_some(p.ring),
                        )
                        // The editor's header shows the chosen portrait at the same size.
                        .key(format!("captain-portrait-face-{portrait}")),
                    ),
            );
        }
        portraits = portraits.child(line);
    }
    let captain_project = promotion.map_or_else(
        || s.captain_project.unwrap_or(s.current().project),
        |promotion| promotion.project,
    );
    let mut body = col()
        .key("captain-form-body")
        .gap(12.0)
        .min_h(0.0)
        .max_h((height - 180.0).max(180.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child({
            // Ported from KSG Code's BranchToolbarEnvironmentSelector. A captain's host
            // is immutable after creation, unlike an unsent conversation draft.
            if machine_options(s).len() > 1 || promotion.is_some() {
                let mut lives = col()
                    .key("captain-machine-row")
                    .gap(5.0)
                    .child(meta_label("Lives on", p.muted))
                    .child(
                        menu_item(
                            if promotion.is_some() { "captain-promote-machine" } else { "captain-machine-choice" },
                            &s.machine_label(&editor.server),
                            if editor.server == "local" {
                                "monitor"
                            } else {
                                "cloud"
                            },
                            p,
                        )
                        .disabled(editing || promotion.is_some())
                        .on_press(|s: &mut State| {
                            let editor = s.captain_editor.as_mut().unwrap();
                            if editor.editing.is_none() && editor.promotion.is_none() {
                                editor.machine_picker = !editor.machine_picker;
                            }
                        }),
                    );
                if editor.machine_picker && !editing && promotion.is_none() {
                    for server in machine_options(s) {
                        if !s.machine_connected(&server) {
                            continue;
                        }
                        let title = s.machine_label(&server);
                        lives = lives.child(
                            machine_option(
                                &format!("captain-machine-{server}"),
                                &title,
                                &server,
                                true,
                                p,
                            )
                            .on_press(move |s: &mut State| s.retarget_captain(&server)),
                        );
                    }
                }
                lives
            } else {
                el()
            }
        })
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Workspace", p.muted))
                .child(
                    row()
                        .key(if promotion.is_some() { "captain-promote-workspace" } else { "captain-project-choice" })
                        .role("button")
                        .cursor(rune::Cursor::Pointer)
                        .label(format!(
                            "Workspace: {}",
                            s.project(captain_project)
                                .map(|p| p.name.as_str())
                                .unwrap_or("No workspace")
                        ))
                        .focusable(!editing && promotion.is_none())
                        .disabled(promotion.is_some())
                        // The same height as the text inputs below it.
                        .h(37.0)
                        .px(8.0)
                        .gap(10.0)
                        .center_y()
                        .radius(6.0)
                        .fill(field)
                        .border(1.0, p.field_line)
                        .opacity(if editing { 0.6 } else { 1.0 })
                        .on_press(move |s: &mut State| {
                            if s.captain_editor
                                .as_ref()
                                .is_some_and(|e| e.editing.is_none() && e.promotion.is_none())
                            {
                                s.open_picker(ProjectPicker::CaptainWorkspace);
                            }
                        })
                        .child(project_tile(s, p, s.project(captain_project), 16.0))
                        .child(
                            label(
                                s.project(captain_project)
                                    .map(|p| p.name.as_str())
                                    .unwrap_or("No workspace"),
                                BODY,
                                p.ink,
                            )
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                        )
                        .child(icon("chevron", p.muted)),
                ),
        )
        .child(
            col()
                .gap(6.0)
                .child(
                    row()
                        .gap(6.0)
                        .child(meta_label("Portrait", p.muted))
                        .child(label(crate::visuals::PORTRAIT_NAMES[chosen], SMALL, p.ink)),
                )
                .child(portraits),
        )
        .child(
            col().gap(5.0).child(meta_label("Name", p.muted)).child(
                text_input(&editor.name, BODY)
                    .key("captain-name")
                    .label("Captain name")
                    .placeholder("e.g. Atlas", p.muted)
                    .single_line()
                    .color(p.ink)
                    .caret(p.ink, p.coral_bg)
                    .w_full()
                    .padding(8.0)
                    .radius(6.0)
                    .fill(field)
                    .field_focus(&editor.name, p)
                    .disabled(profile_locked)
                    .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().name),
            ),
        )
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Role (optional)", p.muted))
                .child(
                    text_input(&editor.role, BODY)
                        .key("captain-role")
                        .label("Captain role")
                        .placeholder("e.g. Code review", p.muted)
                        .single_line()
                        .color(p.ink)
                        .caret(p.ink, p.coral_bg)
                        .w_full()
                        .padding(8.0)
                        .radius(6.0)
                        .fill(field)
                        .field_focus(&editor.role, p)
                        .disabled(profile_locked)
                        .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().role),
                ),
        )
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Instructions for Pi", p.muted))
                .child(
                    text_input(&editor.instructions, BODY)
                        .key("captain-instructions")
                        .label("Captain instructions")
                        .placeholder("What should this captain focus on? Optional.", p.muted)
                        .color(p.ink)
                        .caret(p.ink, p.coral_bg)
                        .w_full()
                        .min_h(76.0)
                        .max_h(140.0)
                        .padding(8.0)
                        .radius(6.0)
                        .fill(field)
                        .field_focus(&editor.instructions, p)
                        .disabled(profile_locked)
                        .on_edit(|s: &mut State| {
                            &mut s.captain_editor.as_mut().unwrap().instructions
                        }),
                ),
        )
        .child(
            label(
                if promotion.is_some() {
                    "Promotion keeps this thread's model and access mode. Private captain instructions stay with the captain."
                } else {
                    "The captain and its crew use these instructions and Pi's configured CPA model."
                },
                SMALL,
                p.muted,
            )
            .wrap(),
        );
    if let Some(promotion) = promotion {
        // KSG has no Captain role. This adapts the existing captain editor and its
        // ProjectIconPickerDialog layout with the approved role-change summary.
        body = body.child(
            col().key("captain-promote-summary").gap(6.0)
                .child(meta_label("What stays", p.muted))
                .child(label("The conversation, its model, access mode, workspace, machine, tasks and pull requests.", BODY, p.muted).wrap())
                .child(meta_label("What changes", p.muted))
                .child(label("It moves to Captains and gets a Brief. Its scope starts as this workspace.", BODY, p.muted).wrap()),
        );
        if let Some(captain) = promotion.restore_candidate {
            let server = editor.server.clone();
            let thread = promotion.thread;
            body = body.child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        captain_ui::switch(
                            s,
                            "captain-promote-restore",
                            "Restore the former captain's profile",
                            restore,
                            p,
                            move |s| {
                                if s.machines.selected == server
                                    && s.captain_editor
                                        .as_ref()
                                        .and_then(|editor| editor.promotion.as_ref())
                                        .is_some_and(|promotion| promotion.thread == thread)
                                    && let Err(error) = s.set_captain_promotion_restore(!restore)
                                    && let Some(editor) = s.captain_editor.as_mut()
                                {
                                    editor.error = Some(error);
                                }
                            },
                        )
                        .disabled(pending),
                    )
                    .child(
                        label(
                            format!(
                                "Restore {}'s profile, plan, rules and memory",
                                s.captains[captain].name
                            ),
                            SMALL,
                            p.muted,
                        )
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                    ),
            );
        }
    }
    if let Some(error) = editor.error.as_ref().or(promotion_reason.as_ref()) {
        body = body.child(
            label(error, SMALL, p.danger)
                .key("captain-editor-reason")
                .role("alert")
                .wrap(),
        );
    }
    let title = if let Some(promotion) = promotion {
        format!("Make \"{}\" a captain", promotion.title)
    } else if editing {
        "Edit captain".into()
    } else {
        "New captain".into()
    };
    let save_label = if promotion.is_some() {
        "Make captain"
    } else if editing {
        "Save changes"
    } else {
        "Create captain"
    };
    let card = col()
        .key("captain-editor")
        .role("dialog")
        .label(&title)
        .w(480.0)
        .max_h((height - 32.0).max(240.0))
        .padding(18.0)
        .gap(14.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .center_y()
                .gap(12.0)
                .child(portrait_avatar(s, p, editor.portrait, 40.0, Status::Ready))
                .child(
                    col()
                        .gap(3.0)
                        .child(label(&title, DIALOG_TITLE, p.ink).wrap())
                        .child(label(
                            "Name it, pick a portrait, and say what it should focus on.",
                            SMALL,
                            p.muted,
                        )),
                )
                .child(spacer())
                .child(icon_button(
                    "close-captain-editor",
                    "Close",
                    "close",
                    p,
                    |s| s.cancel_captain_flow(),
                )),
        )
        .child(body)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(spacer())
                .child(secondary_button("cancel-captain", "Cancel", p, |s| {
                    s.cancel_captain_flow()
                }))
                .child(
                    if profile.validate().is_ok() && promotion_reason.is_none() {
                        primary_button("save-captain", save_label, p, |s| s.save_captain())
                    } else {
                        button("save-captain", save_label, p, |_| {})
                            .h(32.0)
                            .px(14.0)
                            .opacity(0.5)
                            .disabled(true)
                    },
                ),
        );
    el().key("captain-editor-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .padding(16.0)
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.28 }))
        .center()
        .modal(false, |s: &mut State| s.cancel_captain_flow())
        .on_press(|s: &mut State| s.cancel_captain_flow())
        // A press on the scrim closes the editor; Escape does that from the keyboard, so the
        // scrim is no Tab stop.
        .focusable(false)
        .child(card)
}
