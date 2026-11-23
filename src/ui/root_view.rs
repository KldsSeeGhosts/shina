use super::*;

/// The chat pane rounds its top corners (KSG's `--radius-pane: 14px`) and runs square to the
/// window's bottom edge.
pub(super) const PANE_CORNERS: [f32; 4] = [14.0, 14.0, 0.0, 0.0];

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    #[cfg(test)]
    timeline_count(|counts| counts.views += 1);
    let unloaded = s
        .current()
        .transcript_projection
        .as_ref()
        .is_some_and(|projection| !projection.history_loaded);
    s.timeline_positions
        .borrow_mut()
        .open(&s.machines.selected, s.current().id, unloaded);
    let p = palette(s.dark);
    let composer_model = ComposerModelDisplay::new(s);
    composer_ui::measure(s, cx);
    cx.foreground(p.ink);
    cx.focus_ring(2.0, 2.0, p.ring);
    // Every enabled button, tab, link and switch is a Tab stop in document order, as on the web,
    // even one that never said `focusable(true)`. Disabled controls stay out.
    cx.web_focus();
    cx.caption_color(p.muted);
    // The sidebar portraits' animation clock lives outside the view and needs this setting.
    s.portraits.set_reduced_motion(cx.is_reduced_motion());
    let [layout_width, layout_height] = cx
        .layout_with("workspace-root", |r| [r[2], r[3]])
        .unwrap_or([s.viewport_width, s.viewport_height]);
    let narrow = layout_width < 1050.0;
    // Below this width the sidebar stops docking and opens on demand instead.
    let compact = layout_width < 760.0;
    let sidebar_width = sidebar_width_at(s.sidebar_width, layout_width);
    let docked = (s.sidebar && !compact) || (compact && s.compact_sidebar);
    // The chat column's width from the last layout. The first frame guesses from the window,
    // less the gutters and a docked sidebar, so it already lays out close to its final lane.
    let main_width = cx.layout_with("chat-main", |r| r[2]).unwrap_or_else(|| {
        let sidebar = if docked && !compact {
            sidebar_width + 4.0
        } else {
            0.0
        };
        cx.layout_with("workspace-root", move |r| r[2] - 16.0 - sidebar)
            .unwrap_or(900.0)
    });
    let wide = main_width >= WIDE_PANE;
    // The details card docks in the chat column when it fits there. The top bar's button
    // and the lanes below both read the answer.
    s.details.pane_height.set(
        cx.layout_with("chat-main", |r| r[3])
            .unwrap_or(s.viewport_height - 56.0),
    );
    let details_rect = details_ui::rect(s, main_width);
    s.details.fits.set(details_rect.is_some());
    let chrome = cx.chrome_insets();
    // Zoom draws the whole UI k times larger. The root is laid out at the window size divided
    // by k and scaled back up, so layout, hit testing and text stay native at every step.
    let zoom = s.zoom.clamp(0.5, 2.0);
    // Pixel art is scaled to whole device pixels for this scale, so tell it what that is.
    s.artwork.set_scale(cx.env().scale * zoom);
    let leading = if chrome.leading > 0.0 {
        (chrome.leading + 8.0) / zoom
    } else {
        8.0
    };
    let frame = cx.layout_with("zoom-frame", |r| (r[2], r[3]));
    // One strip at the titlebar's height holds the sidebar header, tabs, actions, and the
    // window's caption buttons, so they share a centre line and never overlap the pane.
    let mut top = row()
        .key("top-strip")
        // Native controls move immediately on fullscreen and chrome changes.
        // Titlebar content must not animate through their reserved area.
        .instant_changes()
        .h(chrome.height.max(40.0 * zoom) / zoom)
        .window_drag()
        .center_y()
        .pl(8.0)
        .pr((chrome.trailing / zoom).max(8.0));
    if docked {
        // Keep tabs aligned with the pane while reserving native controls inside
        // the sidebar header. At small zoom levels, keep actions and omit the logo.
        let padding = (leading - 8.0).max(8.0);
        // The header runs to the sidebar's edge, so the compose button at its end has room
        // beside the wordmark in a 232 px sidebar, even with the traffic lights.
        let line = sidebar_width - padding - HEADER_TAB_GAP;
        // The wordmark's size depends on the sidebar's width, not on the platform: a title strip
        // without window buttons still sizes it as if 72 pt were reserved, so a 216 px sidebar
        // with a badge drops to 11 px everywhere.
        let wordmark_line = line.min(sidebar_width - TITLE_CONTROLS - HEADER_TAB_GAP);
        top = top.child(
            sidebar_header(s, p, compact, line, wordmark_line)
                .w(sidebar_width)
                .pl(padding)
                .pr(HEADER_TAB_GAP),
        );
    } else {
        top = top.pl(leading.max(8.0));
        top = top.child(undocked_header(s, p, compact));
    }
    // The tab strip's width from the last layout, for sharing it out among the tabs. The first
    // frame guesses from the window.
    let tabs_width = cx
        .layout_with("thread-tabs", |r| r[2])
        .unwrap_or((s.viewport_width - 400.0).max(200.0));
    top = top.child(top_bar::bar(s, p, narrow, compact, tabs_width));
    // The pane and the sidebar's sea strip both run to the window's bottom edge, so the bottom
    // of the window is one edge, not a strip under the pane and another under the sidebar.
    let mut workspace = row().grow(1.0).min_h(0.0).px(8.0);
    if docked && !compact {
        workspace = workspace.child(sidebar(s, p, sidebar_width));
        if !compact {
            let max_width = sidebar_max_width(layout_width);
            workspace = workspace.child(
                el().key("sidebar-resize")
                    .role("separator")
                    .label("Resize sidebar")
                    .w(4.0)
                    .h_full()
                    .hover(p.line)
                    .focusable(true)
                    .on_double_press(|s: &mut State| {
                        s.sidebar_width = crate::model::DEFAULT_SIDEBAR_WIDTH;
                        s.checkpoint();
                    })
                    .on_drag(move |s: &mut State, drag, _| {
                        if drag.phase == rune::Phase::Down {
                            drag.data = s.sidebar_width;
                        }
                        if drag.cancelled() {
                            s.sidebar_width = drag.data;
                        } else {
                            s.sidebar_width = (drag.data.min(max_width) + drag.dx / zoom)
                                .clamp(crate::model::SIDEBAR_MIN, max_width);
                        }
                        if drag.phase == rune::Phase::Up {
                            s.checkpoint();
                        }
                    })
                    .on_key(move |s: &mut State, ev, _| {
                        let step = match ev.key {
                            rune::Key::Left => -10.0,
                            rune::Key::Right => 10.0,
                            _ => return false,
                        };
                        s.sidebar_width =
                            (sidebar_width + step).clamp(crate::model::SIDEBAR_MIN, max_width);
                        s.checkpoint();
                        true
                    }),
            );
        }
    }
    let mut compact_inspector = None;
    let mut pane = col()
        .key("chat-pane")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .fill(p.panel)
        .corners(PANE_CORNERS)
        .clip_corners(PANE_CORNERS);
    // A retired captain record has no home thread, so it has no Inspector to sit in:
    // it draws as a sheet over the conversation instead.
    let brief_record = s.brief.record.as_ref().is_some_and(|(server, captain)| {
        *server == s.machines.selected
            && s.captain_phase(*captain) == crate::model::CaptainPhase::Retired
    });
    {
        let mut body = row().key("chat-body").grow(1.0).min_h(0.0);
        let main =
            composer_ui::motion_host(s, col().key("chat-main").grow(1.0).min_w(0.0).min_h(0.0));
        let main = if s.current().captain_home
            && s.current().captain.is_some_and(|captain| {
                s.captain_phase(captain) == crate::model::CaptainPhase::Archived
            }) {
            main.child(conversation(s, p, wide, main_width))
                .child(captain_lifecycle_ui::archived_bar(s, p, wide, main_width))
        } else if captain_ui::is_earlier(s) {
            main.child(conversation(s, p, wide, main_width))
                .child(captain_ui::earlier_bar(s, p, wide, main_width))
        } else if s.current().messages.is_empty()
            && s.current().tools.is_empty()
            && s.current().native.compactions.is_empty()
            && s.current().worktree_setup.is_none()
            && !s.current().history_has_more
            && s.current().history_error.is_none()
        {
            // Sizes from the last layout: the hero keeps its height only while the composer
            // still fits under it.
            let pane = cx.layout_with("empty-thread", |r| r[3]);
            let lane = cx.layout_with("composer-lane", |r| r[3]);
            let hero_fits = match (pane, lane) {
                (Some(pane), Some(lane)) => pane - lane >= CAPTAIN_HERO_HEIGHT,
                _ => true,
            };
            main.child(empty_thread(
                s,
                &composer_model,
                p,
                wide,
                main_width,
                hero_fits,
                composer_caret_edges(cx, zoom),
            ))
        } else {
            main.child(conversation(s, p, wide, main_width))
                .child(composer(
                    s,
                    &composer_model,
                    p,
                    wide,
                    main_width,
                    composer_caret_edges(cx, zoom),
                ))
        };
        // The thread's right panel sits at the conversation's right edge inside this row
        // (KSG Code's `ChatRightPanel`). Below 1050 pt (`narrow`), or whenever the body
        // cannot fit a 360 pt panel beside a 360 pt conversation, or when maximized, it
        // covers the conversation column instead (spec §0.1 / §3.3).
        let panel = s.right_panel();
        let panel_open = panel.open;
        let body_width = cx.layout_with("chat-body", |r| r[2]).unwrap_or_else(|| {
            let sidebar = if docked && !compact {
                sidebar_width + 4.0
            } else {
                0.0
            };
            cx.layout_with("workspace-root", move |r| r[2] - 16.0 - sidebar)
                .unwrap_or(900.0)
        });
        let covering =
            panel_open && (panel.maximized || narrow || crate::right_panel::covers(body_width));
        let mut main_wrap = el().grow(1.0).min_w(0.0).min_h(0.0).h_full().child(main);
        // KSG's inline `ThreadDetailsCard`: pinned 12 pt inside the column's top right
        // corner, under a covering Inspector.
        if let Some(rect) = details_rect.filter(|_| s.details.docked) {
            main_wrap = main_wrap.child(details_ui::card(s, p, rect.height).pin(
                None,
                Some(rect.y),
                Some(rect.right_inset),
                None,
            ));
        }
        if covering && compact {
            compact_inspector = Some(inspector_ui::view(s, p, zoom, None));
        } else if covering {
            main_wrap = main_wrap.child(inspector_ui::view(s, p, zoom, None).pin(
                Some(0.0),
                Some(0.0),
                Some(0.0),
                Some(0.0),
            ));
        }
        body = body.child(main_wrap);
        if panel_open && !covering {
            let max = crate::right_panel::max_width(s.viewport_width, body_width);
            let width = crate::right_panel::clamp_width(s.right_panel_width, max);
            body = body.child(inspector_ui::view(s, p, zoom, Some(max)).w(width));
        }
        pane = pane.child(body);
    }
    // The pane has no edge border: the `bg`/`panel` contrast draws its edge (KSG's pane sits
    // borderless on the chrome).
    let mut root = col()
        .key("workspace-root")
        .focusable(false)
        .on_press(|s: &mut State| {
            s.composer_ui.scope_focused.set(false);
            sidebar_preview::dismiss(s, true);
        })
        .on_window_focus(|s: &mut State, focused, _| {
            if !focused {
                sidebar_preview::dismiss(s, true);
            }
        })
        .on_layout(|s: &mut State, rect| {
            s.viewport_width = rect[2];
            s.viewport_height = rect[3];
        })
        .on_key(|s: &mut State, ev, tx| {
            if ev.key == rune::Key::Escape && sidebar_preview::dismiss(s, true) {
                return true;
            }
            if ev.key == rune::Key::Tab {
                sidebar_preview::after_tab(s, tx);
            }
            if ev.key == rune::Key::Escape
                && (s.assistant_selection.is_some() || s.citation_highlight.is_some())
            {
                s.assistant_selection = None;
                s.citation_highlight = None;
                tx.clear_text_selection();
                return true;
            }
            s.undo_key(ev, true)
        })
        .fill(p.bg)
        .child(top)
        .child(workspace.child(pane));
    if compact && docked {
        let top = chrome.height.max(40.0 * zoom) / zoom;
        root = root.child(
            col()
                .key("sidebar-overlay")
                .absolute(8.0, top)
                .size(sidebar_width, (layout_height - top).max(0.0))
                .min_h(0.0)
                .instant_changes()
                .fill(p.bg)
                .box_shadow(soft_shadow(p))
                .child(sidebar(s, p, sidebar_width)),
        );
    }
    if let Some(inspector) = compact_inspector {
        let top = chrome.height.max(40.0 * zoom) / zoom;
        root = root.child(
            col()
                .key("compact-inspector-sheet")
                .absolute(8.0, top)
                .size(
                    (layout_width - 16.0).max(0.0),
                    (layout_height - top).max(0.0),
                )
                .min_h(0.0)
                .child(inspector.w_full()),
        );
    }
    // The context popover floats at the root, after the ring it anchors to, so the composer
    // never makes room for it and no clipping ancestor cuts it off.
    if let Some(toolbar) = assistant_selection_toolbar(s, p, cx) {
        root = root.child(toolbar);
    }
    if composer_menu::trigger(s).is_some() {
        root = root.child(composer_menu::overlay(s, p, cx, s.viewport_width));
    }
    if context_ring_shown(s.current()) && s.current().context.popover_open() {
        root = root.child(context_popover(s, p));
    }
    if docked && s.update_popover {
        root = root.child(updates_ui::popover(s, p));
    }
    if docked && let Some(preview) = sidebar_preview::view(s, p, sidebar_width) {
        root = root.child(preview);
    }
    if let Some(notice) = s.composer_notices.open_details_notice()
        && let Some(popover) =
            composer_notice_details_popover(notice, p, lane_width(main_width, wide), cx)
    {
        root = root.child(popover);
    }
    // Citation chips get a comment action pinned over each pill's right end,
    // mounted at the root like the other composer overlays so nothing clips it.
    // The collapsed preview shares the field's key, so check it is not showing.
    if cx.layout_with("composer-preview", |_| ()).is_none() {
        for action in citation_comment::chip_actions(s, p, cx, zoom) {
            root = root.child(action);
        }
    }
    if brief_record {
        let top = chrome.height.max(40.0 * zoom) / zoom;
        root = root.child(captain_ui::brief_sheet(s, p, top));
    }
    if s.attachment_preview.is_some() {
        root = root.child(attachment_preview::view(s, p));
    } else if s.transcript_preview.is_some() {
        root = root.child(transcript_context::preview(s, p));
    } else if s.quick_switcher {
        root = root.child(quick_switcher::view(s, p));
    } else if s.rename_thread.is_some() || s.delete_thread.is_some() {
        root = root.child(thread_dialog(s, p));
    } else if s.plan_save_dialog.is_some() {
        root = root.child(work_ui::plan_save_dialog(s, p));
    } else if s.confirm_remove_attachment.is_some() {
        root = root.child(remove_attachment_dialog(s, p));
    } else if s.project_picker == Some(ProjectPicker::AddProject) {
        root = root.child(add_project_palette(s, p));
    } else if s.project_picker.is_some() {
        root = root.child(project_picker(s, p));
    } else if s.captain_editor.is_some() {
        let height = cx.layout_with("workspace-root", |r| r[3]).unwrap_or(840.0);
        root = root.child(captain_editor(s, p, height));
    } else if s.settings_picker.is_some() {
        if s.composer_hidden() {
            root = root.child(
                el().key("settings-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.settings_picker = None),
            );
        } else {
            root = root.child(settings_picker(s, &composer_model, p));
        }
    } else if s.captain_picker {
        let anchor = (!s.composer_hidden())
            .then(|| {
                if s.composer_ui.visible_blocks.get() >= 2
                    && cx.layout_with("captain-selector", |_| ()).is_some()
                {
                    Some("captain-selector")
                } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                    Some("composer-overflow")
                } else if cx.layout_with("captain-selector", |_| ()).is_some() {
                    Some("captain-selector")
                } else if cx.layout_with("composer", |_| ()).is_some() {
                    Some("composer")
                } else {
                    None
                }
            })
            .flatten();
        if let Some(anchor) = anchor {
            root = root.child(captain_picker(s, p, anchor));
        } else {
            root = root.child(
                el().key("captain-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.cancel_captain_flow()),
            );
        }
    } else if s.projects.clone_open {
        root = root.child(clone_dialog(s, p));
    } else if s.current().native.access_picker {
        if s.composer_hidden() {
            root = root.child(
                el().key("access-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.current_mut().native.access_picker = false),
            );
        } else {
            let anchor = if cx.layout_with("access-mode", |_| ()).is_some() {
                "access-mode"
            } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                "composer-overflow"
            } else {
                "composer"
            };
            root = root.child(pi_ui::access_picker(s, p, anchor));
        }
    } else if s.current().native.conversation.is_some() {
        root = root.child(pi_ui::conversation_picker(s, p));
    } else if s.thread_menu {
        root = root.child(thread_menu(s, p));
    } else if s.plan_menu.is_some() {
        if let Some(menu) = work_ui::plan_menu(s, p, cx) {
            root = root.child(menu);
        }
    }
    if s.row_menu.is_some() {
        // A right-click opens the menu where it landed; the keyboard opens it beside the row.
        if let Some((x, y)) = s.row_menu_at {
            root = root.child(
                el().key("row-menu-point")
                    .absolute(x / zoom, y / zoom)
                    .size(0.0, 0.0),
            );
        }
        if let Some(menu) = row_menu(s, p, cx) {
            root = root.child(menu);
        }
    }
    if s.tab_menu.is_some() {
        if let Some((x, y)) = s.tab_menu_at {
            root = root.child(
                el().key("thread-tab-menu-point")
                    .absolute(x / zoom, y / zoom)
                    .size(0.0, 0.0),
            );
        }
        root = root.child(thread_tab_menu(s, p));
    }
    if s.snooze_menu.is_some() {
        if let Some((x, y)) = s.snooze_at {
            root = root.child(
                el().key("snooze-menu-point")
                    .absolute(x / zoom, y / zoom)
                    .size(0.0, 0.0),
            );
        }
        if let Some(menu) = snooze_menu(s, p, cx) {
            root = root.child(menu);
        }
    }
    if s.settings_menu {
        root = root.child(settings_ui::dialog(s, p));
    }
    if s.machines.picker {
        let anchor = (!s.composer_hidden())
            .then(|| {
                if s.composer_ui.visible_blocks.get() > 0
                    && cx.layout_with("machine-selector", |_| ()).is_some()
                {
                    Some("machine-selector")
                } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                    Some("composer-overflow")
                } else if cx.layout_with("machine-selector", |_| ()).is_some() {
                    Some("machine-selector")
                } else {
                    None
                }
            })
            .flatten();
        if let Some(anchor) = anchor {
            root = root.child(machine_picker(s, p, anchor));
        } else {
            root = root.child(
                el().key("machine-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.machines.picker = false),
            );
        }
    }
    if s.env_mode_picker {
        let can_show = !s.composer_hidden()
            && matches!(
                s.composer_vcs_controls(),
                crate::model::ComposerVcsControls::Editable { .. }
            );
        let anchor = can_show
            .then(|| {
                if s.composer_ui.visible_blocks.get() > 0
                    && cx.layout_with("env-mode-trigger", |_| ()).is_some()
                {
                    Some("env-mode-trigger")
                } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                    Some("composer-overflow")
                } else if cx.layout_with("env-mode-trigger", |_| ()).is_some() {
                    Some("env-mode-trigger")
                } else {
                    None
                }
            })
            .flatten();
        if let Some(anchor) = anchor {
            root = root.child(env_mode_picker(s, p, anchor));
        } else {
            root = root.child(
                el().key("env-mode-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.env_mode_picker = false),
            );
        }
    }
    if s.base_branch_picker {
        let can_show = !s.composer_hidden()
            && matches!(
                s.composer_vcs_controls(),
                crate::model::ComposerVcsControls::Editable {
                    mode: crate::model::ThreadEnvMode::Worktree,
                    ..
                }
            );
        let anchor = can_show
            .then(|| {
                if cx.layout_with("base-branch-trigger", |_| ()).is_some() {
                    Some("base-branch-trigger")
                } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                    Some("composer-overflow")
                } else {
                    None
                }
            })
            .flatten();
        if let Some(anchor) = anchor {
            root = root.child(base_branch_picker(s, p, anchor));
        } else {
            root = root.child(
                el().key("base-branch-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.base_branch_picker = false),
            );
        }
    }
    if s.composer_ui.overflow.get() {
        if !s.composer_hidden() && cx.layout_with("composer-overflow", |_| ()).is_some() {
            root = root.child(composer_ui::overflow_menu(s, p));
        } else {
            root = root.child(
                el().key("composer-overflow-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.composer_ui.overflow.set(false)),
            );
        }
    }
    // Where the card cannot dock, the top bar's button opens it as a popover (KSG's
    // `PopoverPopup` presentation of the same card).
    if s.details.popover && details_rect.is_none() && details_ui::available(s) {
        root = root.child(
            details_ui::card(s, p, (s.viewport_height - 80.0).max(160.0))
                .anchored_fit("header-details", rune::Side::Bottom, rune::Align::End, 6.0)
                .modal(true, |s: &mut State| s.details.popover = false),
        );
    }
    if let Some(hover) = details_ui::hover_card(s, p) {
        root = root.child(hover);
    }
    if composer_stash::menu_open(s) {
        if !s.composer_hidden() && cx.layout_with("composer-stash-badge", |_| ()).is_some() {
            // KSG's drawer portal, mounted beside the other composer popovers.
            root = root.child(composer_stash::menu(s, p, lane_width(main_width, wide)));
        } else {
            root = root.child(
                el().key("composer-stash-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.stash_menu = false),
            );
        }
    }
    if composer_code_picker::menu_open(s) {
        root = root.child(composer_code_picker::menu(s, p));
    }
    if let Some(popover) = citation_comment::popover(s, p, cx) {
        root = root.child(popover);
    }
    if s.machines.dialog {
        root = root.child(machine_dialog(s, p));
    }
    if s.fleet.open {
        root = root.child(settings_ui::fleet_dialog(s, p));
    }
    if s.settings.archive.is_some() {
        root = root.child(settings_ui::archive_dialog(s, p));
    }
    for dialog in captain_ui::dialogs(s, p) {
        root = root.child(dialog);
    }
    let crew_overlay_captain = captain_ui::brief_captain(s)
        .or_else(|| s.current().crew_captain.filter(|c| *c < s.captains.len()));
    if let Some(captain) = crew_overlay_captain {
        if let Some(menu) = crew_membership_ui::recruit_palette(s, p, captain, cx) {
            root = root.child(menu);
        }
        // A secondary press on a Crew Roster row opens the member's More menu right there,
        // anchored to the row.
        if let Some(thread_id) = s.brief.more {
            let anchor = format!("crew-chip-{thread_id}");
            if cx.layout_with(&anchor, |_| ()).is_some() {
                root = root.child(
                    crew_membership_ui::more_menu(s, p, captain, thread_id)
                        .anchored_fit(anchor, rune::Side::Bottom, rune::Align::End, 4.0)
                        .modal(true, |s: &mut State| s.brief.more = None),
                );
            } else {
                root = root.child(
                    el().key("crew-more-dismiss")
                        .absolute(0.0, 0.0)
                        .size(0.0, 0.0)
                        .a11y_hidden()
                        .on_layout(|s: &mut State, _| s.brief.more = None),
                );
            }
        }
    } else {
        if s.brief.recruit.is_some() {
            root = root.child(
                el().key("crew-recruit-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.brief.recruit = None),
            );
        }
        if s.brief.more.is_some() {
            root = root.child(
                el().key("crew-more-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.brief.more = None),
            );
        }
    }
    if let Some((x, y)) = s.brief.moving.as_ref().and_then(|m| m.at) {
        root = root.child(
            el().key("crew-move-point")
                .absolute(x / zoom, y / zoom)
                .size(0.0, 0.0),
        );
    }
    if let Some(menu) = crew_membership_ui::move_picker(s, p, cx) {
        root = root.child(menu);
    }
    if !s.toasts.is_empty() {
        root = root.child(toast_stack(s, p, chrome.height.max(40.0 * zoom) / zoom));
    }
    root = root.child(composer_ui::measurement_probes(s, &composer_model, p));
    root = match frame {
        Some((w, h)) if (zoom - 1.0).abs() > 0.001 => root.w(w / zoom).h(h / zoom).scale(zoom),
        _ => root.w_full().h_full(),
    };
    el().key("zoom-frame")
        .on_paste_content(composer_ui::paste)
        .w_full()
        .h_full()
        .fill(p.bg)
        .center()
        .child(root)
}
