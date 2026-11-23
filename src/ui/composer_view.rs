use super::*;

/// The composer sits in KSG Code's `.chat-composer-lane`: the same gutters and 46 rem lane as
/// the messages above it, with `pb-4` under the footer.
/// The composer field's height with its 4 pt padding: one line at rest, so the body is 48 pt.
pub(super) const COMPOSER_FIELD_MIN: f32 = 36.0;
/// The composer text field's own padding, inside `composer-input`.
pub(super) const COMPOSER_FIELD_PAD: f32 = 4.0;
/// About how wide the prompt body's inline actions are: attach, the gap and the send key.
pub(super) const COMPOSER_ACTIONS_W: f32 = 28.0 + 6.0 + 32.0;
/// KSG's `max-h-52`: a 200 pt text window inside the 4 pt padding, then the field scrolls.
pub(super) const COMPOSER_FIELD_MAX: f32 = 208.0;
/// A composer that cannot send: demo data, an archived thread or an unreachable machine.
pub(super) fn composer_disabled(s: &State) -> bool {
    s.demo || s.current().archived() || s.unreachable_selected().is_some()
}
/// The composer's 2 px frame: coral while its field has focus, from a click or from Tab, or a
/// file is over it; `frame` otherwise; and the lighter `line` when it cannot send.
pub(super) fn composer_frame(s: &State, p: Palette) -> Color {
    if composer_disabled(s) {
        p.line
    } else if s.attachment_drop_hover || s.current().draft.focused {
        p.ring.alpha(0.6)
    } else {
        p.frame
    }
}
/// Whether the composer's caret is on its first and last visual line, `[first, last]`, or
/// `None` before the field has drawn a caret. Ported from KSG Code's `isCaretOnVisualEdge` in
/// `ComposerPromptEditorTiptap.tsx`: the caret line is on an edge when it is within half its
/// height of that edge of the text. Prompt history uses it, so Up and Down move through a
/// wrapped line before they recall a prompt.
pub(super) fn composer_caret_edges(cx: &Cx<State>, zoom: f32) -> Option<[bool; 2]> {
    // The caret is in window points and layout reads are in the zoomed view's points.
    let caret = cx.caret("composer-input")?.map(|v| v / zoom);
    // Only the field's top and height matter, so a sideways move does not re-run the view.
    let [top, height] = cx.window_layout_with("composer-input", |r| [r[1], r[3]])?;
    let half = caret[3] / 2.0;
    let first = caret[1] - (top + COMPOSER_FIELD_PAD) < half;
    // A field at its minimum height holds one line with spare room under it, so its bottom
    // edge is not the text's. Taller than that, the field ends where its text does.
    let last = if height <= COMPOSER_FIELD_MIN + 0.5 {
        first
    } else {
        top + height - COMPOSER_FIELD_PAD - (caret[1] + caret[3]) < half
    };
    Some([first, last])
}

/// Ported from KSG Code's `replyBannerItems` (`chat/ChatComposer.tsx`) and `ComposerBanner`
/// (`chat/ComposerBanner.tsx`, `px-1 py-1.25 min-h-6 gap-x-1 text-xs/4`).
pub(super) fn composer_reply_banner(t: &Thread, p: Palette) -> Option<El> {
    let reply_to = t.draft_reply_to.as_ref()?;
    let crate::composer_context::ComposerContextData::Response {
        citation,
        subagents,
        ..
    } = &reply_to.data
    else {
        return None;
    };
    let title = match subagents.as_deref() {
        Some(list) if !list.is_empty() => format!(
            "Replying to {}",
            list.iter()
                .map(|subagent| subagent.title.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => "Replying to".to_owned(),
    };
    let target_label = crate::assistant_citation::assistant_citation_label(citation);
    let nav_citation = citation.clone();
    let text_style = TextStyle::new(12.0, 400.0).lh(16.0);
    let title_style = TextStyle::new(12.0, 500.0).lh(16.0);
    let aria_label = format!("{title} {target_label}");
    let has_subagents = subagents.as_deref().is_some_and(|list| !list.is_empty());
    let title_el = if has_subagents {
        label(title, title_style, p.muted)
            .truncate()
            .min_w(0.0)
            .shrink()
    } else {
        label(title, title_style, p.muted)
    };
    Some(
        row()
            .key("composer-reply-banner")
            .keeps_text_selection()
            .on_key(composer_ui::control_key)
            .w_full()
            .min_h(24.0)
            .px(4.0)
            .py(5.0)
            .gap(4.0)
            .center_y()
            .child(
                el().size(24.0, 24.0)
                    .center()
                    .child(icon("back", p.muted).size(12.0, 12.0)),
            )
            .child(
                row()
                    .key("composer-reply-target")
                    .role("button")
                    .label(aria_label)
                    .cursor(rune::Cursor::Pointer)
                    .focusable(true)
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(4.0)
                    .center_y()
                    .radius(6.0)
                    .hover(p.selected)
                    .press(p.selected)
                    .on_press_tx(move |s: &mut State, tx| {
                        s.navigate_to_citation(&nav_citation, tx);
                    })
                    .child(title_el)
                    .child(
                        label(target_label, text_style, p.muted)
                            .truncate()
                            .min_w(0.0)
                            .grow(1.0),
                    ),
            )
            .child(
                el().key("composer-reply-remove")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Remove reply")
                    .focusable(true)
                    .size(20.0, 20.0)
                    .center()
                    .radius(6.0)
                    .hover(p.selected)
                    .press(p.selected)
                    .on_press(|s: &mut State| {
                        s.clear_reply_to();
                        s.restore_composer_focus(composer_ui::FocusReason::Explicit);
                    })
                    .child(icon("close", p.muted).size(12.0, 12.0)),
            ),
    )
}

/// The composer input's decorations: `preview_decorations`' Markdown-source spans,
/// then the draft's per-block syntax colours (KSG's `composerCodeBlockHighlight`),
/// which win colour on an overlap and leave the preview's other attributes alone.
pub(super) fn composer_decorations(
    s: &State,
    p: Palette,
) -> (Vec<rune::text::Span>, Vec<(usize, usize, Option<Color>)>) {
    let t = s.current();
    let (mut spans, strikes) =
        crate::composer_source::preview_decorations(&t.draft, COMPOSER, p.faint, p.ink, p.code_bg);
    spans.extend(t.code_highlight.borrow_mut().spans(
        &t.draft,
        s.dark,
        &markdown_ui::syntax_theme(s.dark, p),
        std::time::Instant::now(),
    ));
    (spans, strikes)
}

/// Budget for the above-composer stack under its 40% viewport cap: how much height the
/// non-roster parts require to remain usable, and how much room (if any) an open crew
/// roster's member rows may take without squeezing Needs you or the Plan editor.
pub(super) struct AboveComposerBudget {
    /// Minimum height for the stack so no visible controls become unreachable.
    floor: f32,
}

pub(super) fn above_composer_budget(s: &State, _cap: f32, _lane: f32) -> AboveComposerBudget {
    let t = s.current();
    let mut parts = 0usize;
    let mut others_floor = 0.0f32;
    if crew_ui::parent_bar_shows(s) {
        parts += 1;
        others_floor += 36.0;
    }
    if crew_ui::crew_bar_shows(s) {
        parts += 1;
        others_floor += 36.0;
    }
    let queued = t.native.queue.len() + t.native.steering.len() + t.native.follow_ups.len();
    if queued > 0 {
        parts += 1;
        others_floor += 48.0;
    }
    if parts > 1 {
        others_floor += (parts - 1) as f32 * 8.0;
    }
    AboveComposerBudget {
        floor: others_floor,
    }
}
/// Actions ported from KSG Code's ComposerPrimaryActions and useComposerResume.
pub(super) fn composer(
    s: &State,
    model: &ComposerModelDisplay,
    p: Palette,
    wide: bool,
    pane_width: f32,
    caret_edges: Option<[bool; 2]>,
) -> El {
    let t = s.current();
    // The lane the conversation resolves too; `None` until C5 mounts the Details card.
    let lane_rect = chat_lane(pane_width, wide, details_ui::edges(s, pane_width));
    let lane = lane_rect.width;
    let inline_files = inline_file_attachment_ids(t);
    let collapsed = s
        .attachment_uploads
        .block_reason(&s.machines.selected, t.id)
        .is_none()
        && !t
            .draft_images
            .iter()
            .any(|a| !a.is_image() && !attachment_is_video(a) && !inline_files.contains(&a.id))
        && composer_ui::collapsed(
            s,
            s.composer_ui.viewport.get(),
            lane - 32.0 - COMPOSER_ACTIONS_W,
        );
    let mut area = col().w_full().max_w(measure(wide)).pb(16.0).gap(8.0);
    if !t.native.notifications.is_empty() {
        area = area.child(pi_ui::notifications(s, p));
    }
    if let Some(error) = &s.storage_error {
        area = area.child(label(error, SMALL, p.danger).wrap_anywhere());
    }
    if let Some(error) = &t.error {
        area = area.child(
            col()
                .max_h(100.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    label(error, BODY, p.danger)
                        .wrap_anywhere()
                        .selectable(true),
                )
                .child(if !t.status.busy() {
                    button("retry-session", "Retry", p, |s| s.retry())
                } else {
                    el()
                }),
        );
        // Antigravity's captured OAuth link opens only on click (KSG sign-in flow).
        if t.harness == "antigravity"
            && let Some(url) = t.harness_state["signInUrl"].as_str()
        {
            let url = url.to_owned();
            area = area.child(
                button("open-sign-in-link", "Sign in with Google", p, |_| {}).on_press_tx(
                    move |_: &mut State, _| {
                        let _ = crate::antigravity::open_browser(&url);
                    },
                ),
            );
        }
        let command = if t.harness == "codex" && error.contains("`codex login`") {
            Some("codex login")
        } else if t.harness == "claude" && error.contains("`claude auth login`") {
            Some("claude auth login")
        } else if t.harness == "antigravity" && error.contains("`agy acp --login`") {
            Some("agy acp --login")
        } else {
            None
        };
        if let Some(command) = command {
            area = area.child(
                button("copy-sign-in-command", "Copy command", p, |_| {}).on_press_tx(
                    move |_: &mut State, tx| {
                        tx.copy(command.to_owned());
                    },
                ),
            );
        }
    }
    if let Some(error) = &t.attachment_error {
        area = area.child(
            label(error, SMALL, p.danger)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    let request_visible = t.request.is_some() && !t.request_held_by_typing_guard();
    // Delegation notices and queued messages stay visible above
    // `ComposerSurface.Shell`, even when a structured questionnaire replaces the draft card.
    let viewport_cap = (s.viewport_height * 0.4).max(120.0);
    let budget = above_composer_budget(s, viewport_cap, lane);
    let cap = viewport_cap.max(budget.floor);
    let mut stack = col()
        .key("above-composer-stack")
        .w_full()
        .gap(6.0)
        .min_h(0.0)
        .max_h(cap);
    for part in [
        crew_ui::parent_bar(s, p, lane),
        crew_ui::crew_bar(s, p, lane),
    ]
    .into_iter()
    .flatten()
    {
        stack = stack.child(part);
    }
    if pi_ui::queued_messages_shows(s) {
        stack = stack.child(pi_ui::queued_messages(s, p));
    }
    area = area.child(stack);
    if t.native.questionnaire.is_some() && request_visible {
        area = area.child(composer_shell(
            s,
            p,
            None,
            pi_ui::questionnaire_drawer(s, p, lane),
            None,
        ));
        return col()
            .key("composer-lane")
            .w_full()
            .pl(lane_gutter(pane_width) + lane_rect.inset_start)
            .pr(lane_gutter(pane_width) + lane_rect.inset_end)
            .items_center()
            .child(area);
    }
    let show_resume = t.can_resume() && !t.has_draft_content() && t.queued_edit.is_none();
    let compact_before_send_tokens = if !collapsed && !show_resume && t.queued_edit.is_none() {
        s.resume_compaction_tokens(t, crate::lifecycle::now())
    } else {
        None
    };
    let keep_full_history = s.keep_full_history(t);
    let compacts_before_send = compact_before_send_tokens.is_some() && !keep_full_history;
    let approval_send_allowed = t
        .request
        .as_ref()
        .filter(|r| r["method"] == "confirm")
        .is_none_or(|request| {
            (crate::approval::can_respond(request) || crate::approval::not_resumable(request))
                && t.requests_supported
                && s.client.as_ref().is_none_or(|client| {
                    (crate::approval::not_resumable(request)
                        || client.can_approve(&s.machines.selected))
                        && client.supports_capability(
                            &s.machines.selected,
                            crate::protocol::COMPOSER_REQUESTS,
                        )
                        && (request["responseCapability"]["supportsFeedback"] != true
                            || client.supports_capability(
                                &s.machines.selected,
                                crate::protocol::APPROVAL_FEEDBACK,
                            ))
                })
        });
    let crew_blocker = t
        .crew_draft
        .as_ref()
        .and_then(|_| s.crew_draft_unavailable());
    let can_send = !s.demo
        && crew_blocker.is_none()
        && s.attachment_send_block_reason().is_none()
        && s.machines.draft_retarget.is_none()
        && (s.client.is_none() || s.machine_connected(&s.machines.selected))
        && (!t.initializing || t.queued_edit.is_some())
        && !t.archived()
        && !t.configuring
        && (!request_visible || t.request.as_ref().is_some_and(|r| r["method"] == "confirm"))
        && !t.approval_send_pending()
        && t.responding_request.is_none()
        && approval_send_allowed
        && (!t.native.operating() || t.queued_edit.is_some())
        && (t.phase != "Stopping" || t.queued_edit.is_some())
        && !t.queued_edit_saving()
        && !t.resume_pending()
        && (!collapsed || !t.status.busy())
        && (t.has_draft_content() || show_resume);
    let mut send = el()
        .key("send-message")
        .role("button")
        .label(if s.machines.draft_retarget.is_some() {
            "Preparing machine"
        } else if t.resume_pending() {
            "Resuming thread"
        } else if show_resume {
            "Resume thread"
        } else if t.queued_edit.is_some() {
            if t.queued_edit_saving() {
                "Updating queued message"
            } else {
                "Update queued message"
            }
        } else if compacts_before_send {
            "Compact and send"
        } else if t.status.busy() {
            if t.follow_up_queues() {
                "Queue message"
            } else {
                "Steer message"
            }
        } else {
            "Submit message"
        })
        .size(32.0, 32.0)
        .radius(7.0)
        .center()
        .child(
            icon(
                if show_resume {
                    "play"
                } else if t.queued_edit.is_some() {
                    "check"
                } else if t.status.busy() && t.follow_up_queues() {
                    "list-plus"
                } else if t.status.busy() {
                    "corner-up-right"
                } else {
                    "arrow-up"
                },
                if can_send { p.ink } else { p.muted },
            )
            .size(14.0, 14.0),
        )
        .disabled(!can_send);
    if can_send {
        send = send
            .cursor(rune::Cursor::Pointer)
            .hover(p.selected)
            .press(p.selected)
            .focusable(true);
        if show_resume {
            send = send.on_press(|s: &mut State| s.resume_thread());
        } else {
            send = send.on_press_tx(|s: &mut State, tx| {
                let previous = s.current().native.follow_up;
                if tx.mods().cmd || tx.mods().ctrl {
                    s.current_mut().native.follow_up = !previous;
                }
                s.send();
                s.current_mut().native.follow_up = previous;
            });
        }
    }
    let [action_x, action_y] = composer_ui::anchor_offset(s, 1, true);
    // KSG Code's `ComposerSendPill` (`surface-pill`): compact, send and stop share one 32 pt
    // recessed control (`--pill-fill`, `field_line` rim >= 3:1 painted above the slots, and
    // the soft `--pill-shadow`), with 7 px slots concentric inside the 8 px rim.
    let mut action = composer_ui::bind_anchor(
        s,
        1,
        row()
            .key("composer-transition-actions")
            .offset(action_x, action_y)
            .h(32.0)
            .radius(8.0)
            .fill(p.pill)
            .box_shadow(control_shadow(p))
            .center_y(),
    );
    let show_send_slot =
        !t.status.busy() || !collapsed && (t.has_draft_content() || t.queued_edit.is_some());
    let compact_disabled =
        composer_disabled(s) || (s.client.is_some() && !s.machine_connected(&s.machines.selected));
    let show_compact_slot = show_send_slot && compact_before_send_tokens.is_some();
    if show_send_slot {
        if let Some(tokens) = compact_before_send_tokens {
            let amount = crate::context::format_context_window_tokens(tokens);
            let tooltip = if keep_full_history {
                format!("Next send keeps all {amount} tokens.\nClick to compact first")
            } else {
                format!("Next send compacts {amount} tokens first.\nClick to keep full history")
            };
            let mut toggle = row()
                .key("composer-compact-send")
                .role("button")
                .label(format!("Compact {amount} tokens of history before sending"))
                .help(tooltip)
                .h(32.0)
                .px(10.0)
                .radius(7.0)
                .center_y()
                .gap(6.0)
                .disabled(compact_disabled)
                .child(icon("minimize", p.muted));
            // Never stack two washes: active state uses one `p.selected` fill, while the
            // inactive state applies `p.selected` only on hover/press when enabled.
            if !keep_full_history {
                toggle = toggle.fill(p.selected);
            } else if !compact_disabled {
                toggle = toggle.hover(p.selected).press(p.selected);
            }
            if !compact_disabled {
                toggle = toggle
                    .cursor(rune::Cursor::Pointer)
                    .focusable(true)
                    .on_press(|s: &mut State| s.toggle_keep_full_history());
            }
            toggle = toggle.child(label(&amount, META.features(TABULAR), p.muted));
            action = action.child(toggle);
        }
        action = action.child(send);
    }
    let show_stop_slot =
        !s.demo && (t.status.busy() && t.queued_edit.is_none() || t.stop_control.phase().is_some());
    let stop_disabled = t.stop_control.disabled()
        || !t.turn_capabilities().supports_interrupt
        || s.client.is_some() && !s.machine_connected(&s.machines.selected);
    if show_stop_slot {
        let mut stop = el()
            .key("stop-agent")
            .role("button")
            .label(t.stop_control.label())
            .disabled(stop_disabled)
            .size(32.0, 32.0)
            .radius(7.0)
            .center()
            .child(
                el().key("composer-stop-mark")
                    .size(10.0, 10.0)
                    .radius(2.0)
                    .fill(p.danger)
                    .a11y_hidden(),
            );
        if !stop_disabled {
            stop = stop
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .hover(p.selected)
                .press(p.selected)
                .on_press(|s: &mut State| s.current_mut().stop());
        }
        action = action.child(stop);
    }
    action = action.child(
        el().key("composer-pill-rim")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(8.0)
            .border(1.0, p.field_line)
            .inert(),
    );
    let locked = s.demo || t.archived() || t.status.busy() || t.configuring;
    let attach_locked = s.demo
        || t.archived()
        || t.configuring
        || t.native.operating() && t.queued_edit.is_none()
        || t.request.is_some();
    // KSG Code's `ComposerSurface.Main`: a 15 px card with a hairline outline. A composer that
    // cannot send drops its shadow and its frame takes the lighter `line` colour.
    // The resting row and the prompt body swap at once, so a press right after expanding
    // never lands on the leaving row.
    let mut card = col().key("composer-natural").w_full().instant_changes();
    card = card
        .on_drop_files(|s: &mut State, files, _| {
            s.attachment_drop_hover = false;
            s.attach_paths(files);
            s.checkpoint();
        })
        .on_drop_hover(|s: &mut State, over, _| s.attachment_drop_hover = over);
    let tray_items = tray_attachments(t);
    let preparing: Vec<_> = s
        .attachment_uploads
        .entries
        .iter()
        .filter(|entry| {
            entry.machine == s.machines.selected
                && entry.thread == t.id
                && entry.image
                && matches!(entry.phase, crate::attachment_uploads::Phase::Preparing)
        })
        .collect();
    let mut first_focusable: Option<String> = None;
    if !collapsed && (!tray_items.is_empty() || !preparing.is_empty()) {
        let mut tray = row()
            .key("composer-attachments")
            .keeps_text_selection()
            .on_key(composer_ui::control_key)
            .opacity(composer_ui::image_arrival(s, false))
            .gap(8.0)
            .flex_wrap()
            .px(12.0)
            .pt(12.0);
        for image in tray_items {
            first_focusable.get_or_insert_with(|| format!("attachment-thumb-{}", image.id));
            tray = tray.child(attachment_chip(s, p, image));
        }
        for entry in preparing {
            first_focusable.get_or_insert_with(|| format!("remove-attachment-{}", entry.id));
            tray = tray.child(preparing_attachment_chip(s, p, entry));
        }
        card = card.child(tray);
    }
    if !collapsed && let Some((rows, file_first)) = attachment_upload_file_rows(s, p) {
        if first_focusable.is_none() {
            first_focusable = file_first;
        }
        card = card.child(rows);
    }
    if let Some(banner) = composer_reply_banner(t, p) {
        first_focusable.get_or_insert_with(|| "composer-reply-target".into());
        card = card.child(banner).child(rule(p));
    }
    if !collapsed {
        *s.composer_ui.first_focusable.borrow_mut() =
            first_focusable.unwrap_or_else(|| "composer-input".into());
    }
    // Ported from KSG Code's composer body (`px-4 pt-4 pb-2`, `ChatComposer.tsx`) and its
    // editor (`max-h-52 overflow-y-auto p-1 -m-1`, `ComposerPromptEditorTiptap.tsx`): the field
    // grows with the draft up to a 200 pt text window, then scrolls inside its own viewport,
    // which Rune keeps on the caret after each edit. The viewport is what the card sizes to, so
    // the actions row always sits below the text and the card never grows past the cap. A bare
    // field with `max_h` laid out at the cap but let the card size to all of its text, which
    // drew the last lines under the actions and left an empty strip below them.
    let composer_style = COMPOSER;
    let offered_suggestion = s.offered_prompt_suggestion();
    let blank_preview = rune::TextEdit::default();
    let field = text_input(
        if collapsed && t.draft.text.trim().is_empty() {
            &blank_preview
        } else {
            &t.draft
        },
        composer_style,
    )
    .key("composer-input")
    .label(format!("Message {}", harness_name(s, &t.harness)))
    .placeholder(
        if s.demo {
            "Open a real workspace to send messages".to_string()
        } else if let Some(machine) = s.unreachable_selected() {
            format!("{machine} is unreachable. Reconnecting.")
        } else if request_visible && t.request.as_ref().is_some_and(|r| r["method"] == "confirm") {
            if t.request
                .as_ref()
                .is_some_and(|r| r["responseCapability"]["supportsFeedback"] == true)
            {
                "Send feedback to decline this request".into()
            } else {
                "Send a message to decline and redirect the agent".into()
            }
        } else if offered_suggestion.is_some() {
            String::new()
        } else if collapsed || lane < 560.0 {
            "Ask anything...".to_string()
        } else {
            "Ask anything, @tag files/folders, $use skills, or / for commands".to_string()
        },
        p.muted,
    )
    .color(p.ink)
    .pill_colors(p.selected, Some(p.ink))
    .pill_style(composer_menu::pill_style(s, p))
    .rich_style(rune::RichStyle {
        code_fg: Some(p.ink),
        code_bg: p.code_bg,
        code_block_bg: p.code_bg,
        link: p.ink,
        quote_bar: p.muted,
        marker: Some(p.muted),
    })
    .block_style(rune::BlockStyle {
        heading_scale: [
            20.0 / 15.0,
            18.0 / 15.0,
            16.0 / 15.0,
            14.0 / 15.0,
            14.0 / 15.0,
            14.0 / 15.0,
        ],
        heading_weight: [600.0; 6],
        // KSG's `.composer-tiptap` headings: line-height 1.3, contrast foreground.
        heading_line_height: [Some(1.3); 6],
        heading_color: [Some(p.ink); 6],
        checkbox_size: 14.0,
        checkbox_stroke: 2.0,
        checkbox_color: Some(p.ink),
        checkbox_checked_fill: Some(p.coral),
        checkbox_mark_color: Some(p.on_coral),
        rule_thickness: 2.0,
        rule_color: Some(p.line),
        // KSG's `.composer-tiptap hr { margin: 0.65rem 0 }`.
        rule_margin_top: Some(10.4),
        rule_margin_bottom: Some(10.4),
    })
    .code_language_headers(s, p, locked)
    .caret(p.ink, p.coral_bg)
    .w_full()
    .min_h(COMPOSER_FIELD_MIN)
    .padding(COMPOSER_FIELD_PAD)
    .on_edit(|s: &mut State| {
        s.current_mut().native.slash_index = 0;
        &mut s.current_mut().draft
    })
    .on_input_key(move |s: &mut State, event, tx| {
        composer_ui::control_key(s, event, tx);
        if pi_ui::composer_pi_key(s, event, tx, caret_edges) {
            composer_menu::scroll_selected(s, tx);
            return true;
        }
        if (event.is_command('c') || event.is_command('x')) && !event.mods.shift && !event.mods.alt
        {
            if s.current().draft.anchor != s.current().draft.focus
                && let Some(content) = s.copy_composer_selection(event.is_command('x'))
            {
                tx.copy_rich(content);
                return true;
            }
            if matches!(
                s.current().draft.selection(),
                rune::EditorSelection::Block(_)
            ) && !s.current().draft.is_secure()
            {
                let cut = event.is_command('x');
                let d = &mut s.current_mut().draft;
                let content =
                    rune::ClipboardContent::new(d.selected()).with_html(d.selected_html());
                tx.copy_rich(content);
                if cut {
                    d.insert("");
                    s.current_mut().update_submission_error();
                }
                return true;
            }
        }
        if event.is_command('v') && event.mods.shift && !event.mods.alt {
            let id = s.current().id;
            let machine = s.machines.selected.clone();
            tx.read_clipboard(move |s: &mut State, content, _| {
                if s.machines.selected != machine {
                    return;
                }
                if let Some(t) = s.threads.iter_mut().find(|t| t.id == id) {
                    t.draft.insert(content.text());
                }
                s.checkpoint();
            });
            return true;
        }
        let t = s.current_mut();
        if t.paste_history
            .key(&mut t.draft, &mut t.draft_images, event)
        {
            s.current_mut().reconcile_draft_context();
            s.checkpoint();
            return true;
        }
        let t = s.current_mut();
        let before = t.draft.text.clone();
        t.composer_source.on_key_before(&t.draft, event);
        let handled = t.composer_rules.key(&mut t.draft, event);
        if handled {
            t.reconcile_draft_context();
            if t.draft.text != before {
                t.composer_source.on_doc_change(&t.draft);
                t.typing_guard.on_type(std::time::Instant::now());
            }
            t.update_submission_error();
            s.ensure_composer_commands();
        }
        handled
    })
    .on_input_change(|s: &mut State, _| {
        composer_ui::changed(s);
    })
    .on_input_paste(|s: &mut State, content, _| {
        let bypass = s.composer_ui.consume_paste_as_text();
        s.paste_composer_content(content, bypass)
    })
    .on_submit(|s: &mut State, _| {
        if !s.demo && !s.current().archived() {
            s.send();
        }
    });
    let (source_spans, source_strikes) = composer_decorations(s, p);
    let field = field
        .on_input_selection_change(|s: &mut State, _| {
            let t = s.current_mut();
            crate::composer_source::on_selection_change(&mut t.composer_source, &mut t.draft);
        })
        .on_input_pill_press(|s: &mut State, id, _, _| s.press_composer_pill(id))
        .decorations(source_spans)
        .strikes(source_strikes);
    if collapsed {
        // KSG's CollapsedComposerPromptPreview is inert. Reuse the real pill renderer without
        // edit handlers or focus, so files, skills and thread chips have exactly the editor's ink.
        let mut preview = row()
            .key("composer-collapsed-row")
            .center_y()
            .gap(8.0)
            .px(12.0)
            .py(8.0)
            .child(
                el().key("composer-preview")
                    .role("button")
                    .label(if t.draft.text.trim().is_empty() {
                        format!(
                            "Expand composer. {}",
                            offered_suggestion.unwrap_or("Ask anything...")
                        )
                    } else {
                        format!("Expand composer. {}", t.draft.display().0)
                    })
                    .cursor(rune::Cursor::Pointer)
                    .grow(1.0)
                    .min_w(0.0)
                    .clip_rounded(0.0)
                    .preserve_focus_on_press(true)
                    .on_press(|s: &mut State| {
                        s.restore_composer_focus(composer_ui::FocusReason::Explicit);
                    })
                    .child(composer_ui::prompt_surface(
                        s,
                        {
                            let preview_field =
                                field.single_line().padding(0.0).h(28.0).min_h(0.0).inert();
                            let mut host = el().w_full().h(28.0).child(preview_field);
                            if let Some(suggestion) = offered_suggestion {
                                host = host.child(
                                    row()
                                        .key("composer-suggestion")
                                        .inert()
                                        .h(28.0)
                                        .center_y()
                                        .gap(8.0)
                                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                                        .child(
                                            label(suggestion, composer_style, p.muted)
                                                .truncate()
                                                .min_w(0.0)
                                                .shrink(),
                                        )
                                        .child(
                                            keycap_chip("Tab", p).key("composer-suggestion-key"),
                                        ),
                                );
                            }
                            host
                        },
                        true,
                    )),
            );
        let resting_tray = tray_attachments(t);
        for image in resting_tray.iter().take(3) {
            let id = image.id.clone();
            preview = preview.child(
                composer_attachment_thumbnail(
                    s,
                    p,
                    image,
                    24.0,
                    24.0,
                    format!("resting-attachment-{}", image.id),
                )
                .opacity(composer_ui::image_arrival(s, true))
                .role("button")
                .label(format!("Preview {}", image.name))
                .preserve_focus_on_press(true)
                .on_press(move |s: &mut State| {
                    s.preview_attachments(s.current().draft_images.clone(), &id, true);
                }),
            );
        }
        if resting_tray.len() > 3 {
            preview = preview.child(button(
                "resting-attachments-more",
                &format!("+{}", resting_tray.len() - 3),
                p,
                |s| s.restore_composer_focus(composer_ui::FocusReason::Explicit),
            ));
        }
        preview = preview.child(attach_control(attach_locked, p));
        if context_ring_shown(t) {
            preview = preview.child(context_ring(s, t, p, lane));
        }
        preview = preview.child(action);
        let controls = composer_ui::controls(s, model, p, lane - 32.0, true, locked);
        let composer_card = composer_ui::frame_card(s, p, card.child(preview).child(controls));
        return col()
            .key("composer-lane")
            .w_full()
            .pl(lane_gutter(pane_width) + lane_rect.inset_start)
            .pr(lane_gutter(pane_width) + lane_rect.inset_end)
            .items_center()
            .child(area.child(composer_shell(
                s,
                p,
                composer_request_banner(s, p, lane),
                attach_composer_notices(s, p, lane, composer_card),
                None,
            )));
    }
    // KSG Code's `.composer-prompt-body`: the editor on the left and the attach, compact,
    // dispatch and send controls inline on the right. A one-line draft centres them on the
    // prompt line; a wrapped draft anchors them to the bottom right.
    let multiline = t.draft.text.contains('\n') || s.composer_ui.wrapped();
    let mut cluster = row()
        .key("composer-actions")
        .on_key(composer_ui::control_key)
        .keeps_text_selection()
        .center_y()
        .gap(6.0)
        .child(
            attach_control(attach_locked, p)
                .size(28.0, 28.0)
                .radius(8.0),
        );
    if t.status.busy() && t.has_draft_content() && t.queued_edit.is_none() {
        cluster = cluster.child(
            button(
                "dispatch-mode",
                if t.follow_up_queues() {
                    "Queue"
                } else {
                    "Steer"
                },
                p,
                |s| {
                    let n = &mut s.current_mut().native;
                    n.follow_up = !n.follow_up;
                },
            )
            .disabled(!t.can_steer()),
        );
    }
    cluster = cluster.child(action);
    let mut prompt_body = row()
        .key("composer-prompt-body")
        .w_full()
        .min_h(48.0)
        .py(6.0)
        .pl(10.0)
        .pr(8.0)
        .gap(6.0);
    prompt_body = if multiline {
        prompt_body.items_end()
    } else {
        prompt_body.center_y()
    };
    card = card.child(
        prompt_body
            .child(
                col()
                    .key("composer-body")
                    .grow(1.0)
                    .min_w(0.0)
                    // A press in the body's padding still lands in the field, as KSG's surface does.
                    .on_press(|s: &mut State| {
                        s.restore_composer_focus(composer_ui::FocusReason::Explicit)
                    })
                    // The field inside is the Tab stop, not this.
                    .focusable(false)
                    .child(composer_ui::prompt_surface(
                        s,
                        col()
                            .key("composer-scroll")
                            .w_full()
                            .shrink()
                            .min_h(COMPOSER_FIELD_MIN)
                            .max_h(COMPOSER_FIELD_MAX)
                            .scroll(ScrollSpec {
                                stick_bottom: false,
                            })
                            // ProseMirror's scroll margin: the caret line keeps the 4 pt padding in view.
                            .scroll_margin(4.0)
                            .child({
                                let mut host = el().key("composer-input-host").w_full();
                                let field = if let Some(suggestion) = offered_suggestion {
                                    let content_w = (lane
                                        - COMPOSER_ACTIONS_W
                                        - 26.0
                                        - COMPOSER_FIELD_PAD * 2.0)
                                        .max(40.0);
                                    let (natural_w, _) =
                                        rune::text::measure(suggestion, &[], composer_style, None);
                                    let min_h = if natural_w + 48.0 <= content_w {
                                        COMPOSER_FIELD_MIN
                                    } else {
                                        let (_, text_h) = rune::text::measure(
                                            suggestion,
                                            &[],
                                            composer_style,
                                            Some(content_w),
                                        );
                                        (text_h + 24.0 + COMPOSER_FIELD_PAD * 2.0)
                                            .max(COMPOSER_FIELD_MIN)
                                    };
                                    host = host.min_h(min_h);
                                    field.min_h(min_h)
                                } else {
                                    host = host.min_h(COMPOSER_FIELD_MIN);
                                    field
                                };
                                host = host.child(field);
                                if let Some(suggestion) = offered_suggestion {
                                    host = host.child(
                                        row()
                                            .key("composer-suggestion")
                                            .inert()
                                            .flex_wrap()
                                            .center_y()
                                            .gap(8.0)
                                            .pin(
                                                Some(COMPOSER_FIELD_PAD),
                                                Some(COMPOSER_FIELD_PAD),
                                                None,
                                                Some(COMPOSER_FIELD_PAD),
                                            )
                                            .child(
                                                label(suggestion, composer_style, p.muted)
                                                    .wrap()
                                                    .min_w(0.0)
                                                    .shrink(),
                                            )
                                            .child(
                                                keycap_chip("Tab", p)
                                                    .key("composer-suggestion-key"),
                                            ),
                                    );
                                }
                                host
                            }),
                        false,
                    )),
            )
            .child(cluster),
    );
    if let Some(error) = &t.submission_error {
        card = card.child(
            col()
                .key("composer-prompt-length-error")
                .px(16.0)
                .pb(8.0)
                .child(label(error, SMALL, p.danger).wrap().selectable(true)),
        );
    }
    let mut card_last = "composer-input";
    if !attach_locked {
        card_last = "attach-image";
    }
    if t.status.busy() && t.has_draft_content() && t.queued_edit.is_none() && t.can_steer() {
        card_last = "dispatch-mode";
    }
    if show_compact_slot && !compact_disabled {
        card_last = "composer-compact-send";
    }
    if show_send_slot && can_send {
        card_last = "send-message";
    }
    if show_stop_slot && !stop_disabled {
        card_last = "stop-agent";
    }
    s.composer_ui.last_focusable.set(card_last);
    let composer_card = composer_ui::frame_card(s, p, card);
    // KSG Code's `ComposerSurface.ContextStrip`: a 30 px muted row inside the tray, the run
    // context on the left and the model, effort, context and access on the right.
    let footer = if s.demo {
        s.composer_ui.last_focusable.set("leave-example");
        row()
            .key("composer-footer")
            .on_key(composer_ui::control_key)
            .keeps_text_selection()
            .h(30.0)
            .px(10.0)
            .center_y()
            .gap(6.0)
            .child(meta_text("Demo data", p.muted))
            .child(spacer())
            .child(
                row()
                    .key("leave-example")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Open a real workspace")
                    .focusable(true)
                    .center_y()
                    .h(22.0)
                    .px(6.0)
                    .radius(6.0)
                    .hover(p.selected)
                    .on_press(|s: &mut State| {
                        s.leave_demo();
                    })
                    .child(meta_text("Open a real workspace", p.sea)),
            )
    } else {
        composer_ui::controls(s, model, p, lane - 16.0, false, locked)
    };
    area = area.child(composer_shell(
        s,
        p,
        composer_request_banner(s, p, lane),
        attach_composer_notices(s, p, lane, composer_card),
        Some(footer),
    ));
    if let Some(reason) = crew_blocker.as_ref().or(t.crew_draft_notice.as_ref()) {
        area = area.child(
            label(
                reason,
                SMALL,
                if crew_blocker.is_some() {
                    p.brass
                } else {
                    p.muted
                },
            )
            .key("composer-crew-status")
            .role("status")
            .px(10.0)
            .pb(4.0)
            .wrap(),
        );
    }
    col()
        .key("composer-lane")
        .w_full()
        .pl(lane_gutter(pane_width) + lane_rect.inset_start)
        .pr(lane_gutter(pane_width) + lane_rect.inset_end)
        .items_center()
        .child(area)
}
