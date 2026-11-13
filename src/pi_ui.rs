//! Rune ports of KSG Code's Pi command, compaction, and conversation controls.
use super::*;
use crate::model::ConversationMode;

/// Ported from KSG Code's ComposerPendingUserInputPanel: header, 14 px option labels,
/// descriptions, digit shortcuts, radio/checkbox rows and a scrollable composer replacement.
pub(super) fn questionnaire_drawer(s: &State, p: Palette, lane: f32) -> El {
    let t = s.current();
    let form = t.native.questionnaire.as_ref().expect("questionnaire");
    let question = &form.questions[form.index];
    let answer = &form.answers[form.index];
    let thread_id = t.id;
    let request_id = t.request.as_ref().expect("question request")["id"].clone();
    let question_scope = t.question_scope();
    let toggle_scope = question_scope.clone();
    let machine = s.machines.selected.clone();
    let toggle_machine = machine.clone();
    let collapsed = t.questionnaire_collapsed();
    let toggle_title = if collapsed {
        "Show the question and its options"
    } else {
        "Hide the question and its options"
    };
    let dismissible =
        form.capability == "message" || form.response_mode.as_deref() == Some("message");
    let dismissal_disabled = t.responding_request.is_some()
        || t.question_submission_pending()
        || s.client.as_ref().is_some_and(|c| {
            !c.can_approve(&s.machines.selected)
                || !c.supports_capability(&s.machines.selected, crate::protocol::QUESTION_DISMISS)
        });
    let disabled = !t.questionnaire_can_respond()
        || s.client.as_ref().is_some_and(|c| {
            !c.can_approve(&s.machines.selected)
                || !c.supports_capability(&s.machines.selected, crate::protocol::QUESTION_CONTRACT)
        });
    let custom = question.allow_custom && !t.reply.text.trim().is_empty();
    let mut options = col().gap(2.0).w_full();
    for (i, option) in question.options.iter().enumerate() {
        let selected = !custom && answer.selected.contains(&option.value);
        // A square mark: a filled coral square for a chosen checkbox, a coral square inside the
        // frame for a chosen radio.
        let indicator = el()
            .size(14.0, 14.0)
            .center()
            .border(2.0, if selected { p.coral } else { p.field_line })
            .fill(if selected && question.multi {
                p.coral
            } else {
                Color::TRANSPARENT
            })
            .child(if selected && !question.multi {
                el().size(6.0, 6.0).fill(p.coral)
            } else {
                el()
            })
            .a11y_hidden();
        options = options.child(
            row()
                .key(format!("dialog-option-{i}"))
                .role(if question.multi { "checkbox" } else { "radio" })
                .label(format!(
                    "{}{}{}",
                    option.label,
                    if selected { ", selected. " } else { ". " },
                    option.description
                ))
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .w_full()
                .min_h(34.0)
                .py(6.0)
                .px(8.0)
                .gap(10.0)
                .radius(4.0)
                .fill(if selected {
                    p.selected
                } else {
                    Color::TRANSPARENT
                })
                .hover(p.selected)
                .child(el().w(20.0).center_y().child(indicator))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            label(&option.label, TextStyle::new(14.0, 500.0), p.ink)
                                .wrap_anywhere(),
                        )
                        .child(
                            if !option.description.is_empty() && option.description != option.label
                            {
                                label(&option.description, TextStyle::new(11.0, 400.0), p.muted)
                                    .wrap_anywhere()
                            } else {
                                el()
                            },
                        ),
                )
                .child(el().w(28.0).center().child(if i < 9 {
                    keycap_chip(&(i + 1).to_string(), p)
                } else {
                    el()
                }))
                .disabled(disabled)
                .on_press(move |s: &mut State| s.current_mut().questionnaire_toggle(i)),
        );
    }
    let mut live_form = form.clone();
    live_form.edit(t.reply.text.clone());
    let can_advance = live_form.answer(form.index).is_some();
    let last = form.index + 1 == form.questions.len();
    col()
        .key("pi-dialog")
        .role("group")
        .label("Pi needs your input")
        .w_full()
        .child(
            col()
                .key("questionnaire-drawer")
                .w_full()
                .padding(16.0)
                .gap(8.0)
                .radius(6.0)
                .fill(p.card)
                .border(2.0, p.frame)
                .box_shadow(hard_shadow(p))
                .child(
                    row()
                        .key("questionnaire-toggle")
                        .role("button")
                        .label(toggle_title)
                        .help(toggle_title)
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .py(6.0)
                        .gap(8.0)
                        .on_press(move |s: &mut State| {
                            if s.current().id == thread_id
                                && s.machines.selected == toggle_machine
                                && s.current().question_scope() == toggle_scope
                            {
                                s.current_mut().toggle_questionnaire();
                            }
                        })
                        .child(
                            pixel_label(&question.header, p.muted)
                                .grow(if collapsed { 0.0 } else { 1.0 })
                                .max_w(if collapsed {
                                    (lane - 32.0).max(0.0) * 0.5
                                } else {
                                    f32::INFINITY
                                })
                                .min_w(0.0)
                                .truncate(),
                        )
                        .child(if collapsed {
                            row()
                                .grow(1.0)
                                .min_w(0.0)
                                .gap(4.0)
                                .child(label("·", SMALL, p.faint))
                                .child(
                                    label(&question.text, SMALL, p.muted)
                                        .key("questionnaire-summary")
                                        .grow(1.0)
                                        .min_w(0.0)
                                        .truncate(),
                                )
                        } else {
                            el()
                        })
                        .child(if form.questions.len() > 1 {
                            label(
                                format!("{}/{}", form.index + 1, form.questions.len()),
                                TIMER,
                                p.muted,
                            )
                            .key("questionnaire-count")
                        } else {
                            el()
                        })
                        .child(
                            icon(if collapsed { "chevron-up" } else { "chevron" }, p.muted)
                                .a11y_hidden(),
                        )
                        .child(if dismissible {
                            icon_button(
                                "dialog-cancel",
                                "Dismiss question without answering",
                                "close",
                                p,
                                move |s| {
                                    if s.current().id != thread_id
                                        || s.machines.selected != machine
                                        || s.current().question_scope() != question_scope
                                    {
                                        return;
                                    }
                                    if let Err(error) = s.dismiss_question(thread_id, &request_id) {
                                        s.current_mut().error = Some(error);
                                    }
                                },
                            )
                            .help("Dismiss question without answering")
                            .disabled(dismissal_disabled)
                        } else {
                            el()
                        }),
                )
                .child(if !collapsed {
                    col()
                        .key("questionnaire-scroll")
                        .w_full()
                        .max_h((s.viewport_height * 0.32).clamp(96.0, 260.0))
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(
                            markdown_ui::render_with_find(
                                &question.text,
                                p,
                                s.dark,
                                false,
                                false,
                                false,
                                &format!("question-{}-{}", t.id, question.id),
                                None,
                            )
                            .key("questionnaire-markdown"),
                        )
                        .child(options)
                } else {
                    el()
                })
                .child(if question.allow_custom {
                    text_input(&t.reply, BODY)
                        .key("dialog-reply")
                        .label("Other answer")
                        .placeholder(
                            "Type your own answer, or leave this blank to use the selected option",
                            p.muted,
                        )
                        .color(p.ink)
                        .w_full()
                        .min_h(32.0)
                        .max_h(72.0)
                        .px(8.0)
                        .py(6.0)
                        .radius(6.0)
                        .fill(p.card)
                        .field_focus(&t.reply, p)
                        .disabled(disabled)
                        .on_edit(|s: &mut State| s.current_mut().questionnaire_edit())
                        .on_input_change(|s: &mut State, _| {
                            s.current_mut().questionnaire_changed();
                            if let Some(scope) = s.current().question_scope() {
                                s.with_question_draft(s.active, scope, |s| {
                                    s.current_mut().reconcile_draft_context()
                                });
                            }
                        })
                        .on_input_key(|s: &mut State, event, tx| {
                            if event.is_command('c') || event.is_command('x') {
                                let cut = event.is_command('x');
                                if let Some(scope) = s.current().question_scope()
                                    && let Some(content) = s
                                        .with_question_draft(s.active, scope, |s| {
                                            s.copy_composer_selection(cut)
                                        })
                                        .flatten()
                                {
                                    tx.copy_rich(content);
                                    return true;
                                }
                            }
                            false
                        })
                        .on_input_paste(|s: &mut State, content, _| {
                            s.paste_composer_content(content, false)
                        })
                        .on_submit(|s: &mut State, _| s.current_mut().questionnaire_step(false))
                } else {
                    el()
                })
                .child(
                    row()
                        .key("question-attachments")
                        .gap(8.0)
                        .flex_wrap()
                        .children(
                            t.question_files()
                                .iter()
                                .map(|file| attachment_chip(s, p, file)),
                        ),
                )
                .child(
                    row()
                        .gap(6.0)
                        .flex_wrap()
                        .child(
                            if question.allow_custom
                                && s.client.as_ref().is_none_or(|c| {
                                    c.supports_capability(
                                        &s.machines.selected,
                                        crate::protocol::QUESTION_ATTACHMENTS,
                                    )
                                })
                            {
                                attach_control(disabled, p)
                            } else {
                                el()
                            },
                        )
                        .child(if form.index > 0 {
                            button("questionnaire-previous", "Previous", p, |s| {
                                s.current_mut().questionnaire_step(true)
                            })
                            .disabled(disabled)
                        } else {
                            el()
                        })
                        .child(
                            button(
                                if last {
                                    "dialog-submit"
                                } else {
                                    "questionnaire-next"
                                },
                                if t.responding_request.is_some() || t.question_submission_pending()
                                {
                                    "Submitting..."
                                } else if s.viewport_width < 640.0 {
                                    if last { "Submit" } else { "Next" }
                                } else if !last {
                                    "Next question"
                                } else if form.index > 0 {
                                    "Submit answers"
                                } else {
                                    "Submit answer"
                                },
                                p,
                                |s| s.current_mut().questionnaire_step(false),
                            )
                            .disabled(disabled || !can_advance || last && !live_form.complete()),
                        )
                        .child(if t.status.busy() || t.stop_control.phase().is_some() {
                            icon_button("stop-agent", t.stop_control.label(), "stop", p, |s| {
                                s.current_mut().stop();
                            })
                            .disabled(t.stop_control.disabled())
                        } else {
                            el()
                        }),
                ),
        )
}

pub(crate) fn questionnaire_key(s: &mut State, event: &rune::KeyEvent) -> bool {
    if event.mods.ctrl
        || event.mods.cmd
        || event.mods.alt
        || s.current().reply.focused
        || s.current().draft.focused
        || s.search.focused
        || s.current().questionnaire_collapsed()
        || !s.current().questionnaire_can_respond()
        || s.client.as_ref().is_some_and(|c| {
            !c.can_approve(&s.machines.selected)
                || !c.supports_capability(&s.machines.selected, crate::protocol::QUESTION_CONTRACT)
        })
    {
        return false;
    }
    let rune::Key::Text(digit) = &event.key else {
        return false;
    };
    let Some(form) = &mut s.current_mut().native.questionnaire else {
        return false;
    };
    // Validate with the same helper used by non-UI questionnaire tests.
    let Some(index) = form.digit_index(digit) else {
        return false;
    };
    s.current_mut().questionnaire_toggle(index);
    true
}

pub(super) fn notifications(s: &State, p: Palette) -> El {
    let mut list = col().key("pi-notifications").w_full().gap(4.0);
    for (i, (message, _)) in s.current().native.notifications.iter().enumerate() {
        list = list.child(
            label(message, SMALL, p.ink)
                .key(format!("pi-notification-{i}"))
                .wrap_anywhere()
                .padding(8.0)
                .radius(6.0)
                .fill(p.coral_bg)
                .border(0.5, p.line),
        );
    }
    list
}

/// `caret_edges` says whether the caret is on the field's first and last visual line, from
/// `composer_caret_edges`; `None` falls back to the text's own line breaks.
pub(super) fn composer_pi_key(
    s: &mut State,
    event: &rune::KeyEvent,
    tx: &rune::Tx,
    caret_edges: Option<[bool; 2]>,
) -> bool {
    // KSG's editor handles code Enter/Tab and opening-fence Enter before the
    // composer command handler, including before the send shortcut.
    if crate::composer_rich::editor_key_before_commands(s.current_mut(), event) {
        return true;
    }
    if composer_queue_key(s, event, tx) {
        return true;
    }
    if s.undo_key(event, true) {
        return true;
    }
    let is_running = s.current().status.busy();
    let is_draft_thread = s.current().is_draft();
    let is_composing = s.current().draft.is_composing();
    let send_shortcut = s.preferences.send_shortcut;
    let prompt_text = s.current().draft.text.clone();
    let resolved = s.resolved_keybindings();
    let ctx = s.shortcut_context(true);
    let shortcut_command = crate::keybindings::resolve_shortcut_command(
        event,
        &resolved,
        &ctx,
        crate::keybindings::Platform::current(),
        tx.physical_key(),
        tx.key_base(),
    );
    let submission_intent = crate::composer::submission_intent_for_command(
        event,
        tx.key_repeat(),
        is_composing,
        is_running,
        is_draft_thread,
        send_shortcut,
        &prompt_text,
        shortcut_command,
    );
    if matches!(
        submission_intent,
        None | Some(crate::composer::SubmissionIntent::Foreground)
    ) && composer_menu::key(s, event)
    {
        return true;
    }
    if composer_suggestion_key(s, event, tx.key_repeat()) {
        return true;
    }
    if let Some(intent) = submission_intent {
        if !s.demo && !s.current().archived() {
            match intent {
                crate::composer::SubmissionIntent::Foreground => s.send(),
                crate::composer::SubmissionIntent::Alternate => {
                    let previous = s.current().native.follow_up;
                    s.current_mut().native.follow_up = !previous;
                    s.send();
                    s.current_mut().native.follow_up = previous;
                }
                crate::composer::SubmissionIntent::Background => s.send_background(),
            }
        }
        return true;
    }
    if !is_composing
        && tx.key_repeat()
        && matches!(
            shortcut_command,
            Some(
                "composer.sendAlternate" | "composer.sendBackground" | "composer.sendAndNewThread"
            )
        )
    {
        return true;
    }
    if event.key == rune::Key::Enter && !is_composing && !event.mods.alt {
        if tx.key_repeat() {
            return true;
        }
        let newline_event = rune::KeyEvent {
            key: rune::Key::Enter,
            mods: rune::Mods {
                shift: true,
                ..Default::default()
            },
        };
        let t = s.current_mut();
        if !t.composer_rules.key(&mut t.draft, &newline_event) {
            t.draft.key(&newline_event, None);
            t.composer_rules.changed(&mut t.draft);
            t.update_submission_error();
        }
        return true;
    }
    if (event.mods.cmd || event.mods.ctrl || event.mods.alt)
        && let Some(command) = shortcut_command
    {
        let has_selection = s.current().draft.anchor != s.current().draft.focus;
        let yield_bold = command == "sidebar.toggle"
            && crate::keybindings::is_rich_text_bold_shortcut(
                event,
                tx.physical_key(),
                tx.key_base(),
                has_selection,
            );
        if !yield_bold
            && !matches!(
                command,
                "composer.sendAlternate" | "composer.sendBackground" | "composer.sendAndNewThread"
            )
            && crate::dispatch_key_no_tx(
                s,
                event,
                tx.physical_key(),
                tx.key_base(),
                tx.key_repeat(),
            )
        {
            return true;
        }
    }
    let menu_open = composer_menu::trigger(s).is_some();
    let t = s.current_mut();
    let entries = crate::composer::build_history_entries(&t.messages);
    let has_attachments = !t.draft_images.is_empty();
    let has_pending_request = t.request.is_some();
    if t.prompt_history.navigate(
        &mut t.draft,
        &entries,
        event,
        menu_open,
        has_attachments,
        has_pending_request,
        caret_edges.map(|[first, last]| {
            if event.key == rune::Key::Up {
                first
            } else {
                last
            }
        }),
    ) {
        return true;
    }
    false
}

/// KSG Code's thread.editQueuedMessage and thread.steerQueuedMessage shortcuts.
fn composer_queue_key(s: &mut State, event: &rune::KeyEvent, tx: &rune::Tx) -> bool {
    if s.current().draft.is_composing() || s.current().request.is_some() {
        return false;
    }
    let resolved = s.resolved_keybindings();
    let ctx = s.shortcut_context(true);
    let command = crate::keybindings::resolve_shortcut_command(
        event,
        &resolved,
        &ctx,
        crate::keybindings::Platform::current(),
        tx.physical_key(),
        tx.key_base(),
    );
    let edit_latest = command == Some("thread.editQueuedMessage")
        && s.current().queued_edit.is_none()
        && s.current().draft.focus == 0
        && s.current().draft.anchor == 0;
    let promote_next = command == Some("thread.steerQueuedMessage");
    let id = if edit_latest {
        s.current().native.queue.back().map(|q| q.id)
    } else if promote_next {
        s.current().native.queue.front().map(|q| q.id)
    } else {
        return false;
    };
    let Some(id) = id else { return false };
    if tx.key_repeat() {
        return true;
    }
    if edit_latest {
        s.current_mut().edit_queued(id);
    } else {
        s.current_mut().steer_queued(id);
    }
    s.checkpoint();
    true
}

pub(super) fn suggestion_menu_open(s: &State) -> bool {
    composer_menu::trigger(s).is_some()
        || s.settings_picker.is_some()
        || s.captain_picker
        || s.machines.picker
        || s.current().native.access_picker
        || s.project_picker.is_some()
        || s.open_option_menu.is_some()
        || s.row_menu.is_some()
        || s.thread_menu
        || s.settings_menu
        || s.attention_menu
        || s.plan_menu.is_some()
        || s.snooze_menu.is_some()
        || s.current().context.popover_open()
}

fn composer_suggestion_key(s: &mut State, event: &rune::KeyEvent, key_repeat: bool) -> bool {
    if suggestion_menu_open(s) || s.offered_prompt_suggestion().is_none() {
        return false;
    }
    if event.key == rune::Key::Tab
        && !event.mods.shift
        && !event.mods.alt
        && !event.mods.cmd
        && !event.mods.ctrl
    {
        if key_repeat {
            return true;
        }
        return s.accept_prompt_suggestion();
    }
    if event.key == rune::Key::Escape
        && !event.mods.shift
        && !event.mods.alt
        && !event.mods.cmd
        && !event.mods.ctrl
    {
        if key_repeat {
            return true;
        }
        return s.dismiss_prompt_suggestion();
    }
    false
}

/// Ported from KSG Code's QueuedRunsControl: right-aligned unsent messages, per-message edit,
/// remove and Send now actions, three initial rows and an explicit expansion control.
pub(super) fn queued_messages(s: &State, p: Palette) -> El {
    let t = s.current();
    let count = t.native.queue.len() + t.native.steering.len() + t.native.follow_ups.len();
    if count == 0 {
        return el();
    }
    let mut list = col()
        .key("queued-messages")
        .w_full()
        .gap(8.0)
        .max_h(260.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let limit = if t.native.queue_expanded || t.queued_edit.is_some() {
        usize::MAX
    } else {
        3
    };
    for queued in t.native.queue.iter().take(limit) {
        let id = queued.id;
        let editing = t
            .queued_edit
            .as_ref()
            .is_some_and(|edit| edit.input_id == id);
        let mut actions = row().gap(2.0).flex_wrap();
        if editing {
            actions = actions
                .child(label("Editing in the composer", SMALL, p.ink))
                .child(
                    button(&format!("queue-edit-{id}"), "Cancel", p, |s| {
                        s.current_mut().cancel_queued_edit();
                        s.checkpoint();
                    })
                    .label("Cancel editing queued message"),
                );
        } else {
            actions = actions
                .child(
                    button(&format!("queue-steer-{id}"), "Send now", p, move |s| {
                        s.current_mut().steer_queued(id);
                        s.checkpoint();
                    })
                    .disabled(t.status.busy() && !t.can_steer()),
                )
                .child(
                    button(&format!("queue-edit-{id}"), "Edit", p, move |s| {
                        s.current_mut().edit_queued(id);
                        s.checkpoint();
                    })
                    .disabled(t.request.is_some()),
                );
        }
        actions = actions.child(button(
            &format!("queue-remove-{id}"),
            "Remove",
            p,
            move |s| {
                s.current_mut().cancel_queued(id);
                s.checkpoint();
            },
        ));
        let preview_text = crate::composer_context::replace_composer_context_references(
            crate::composer_context::strip_attachment_only_bootstrap_prompt(&queued.prompt.text),
            |reference| {
                if reference.kind == "image" && !queued.prompt.images.is_empty() {
                    String::new()
                } else {
                    reference.label.clone()
                }
            },
        );
        let mut body = col()
            .gap(6.0)
            .px(12.0)
            .py(8.0)
            .radius(6.0)
            .border(1.0, p.line)
            .max_w(580.0)
            .shrink()
            .min_w(0.0)
            .child(label(preview_text.trim(), BODY, p.muted).wrap_anywhere());
        if !queued.prompt.images.is_empty() {
            body = body.child(message_images(
                s,
                p,
                &queued.prompt.images,
                &format!("queued-{id}"),
            ));
        }
        list = list.child(
            col()
                .key(format!("queued-{id}"))
                .items_end()
                .gap(4.0)
                .child(body)
                .child(actions),
        );
    }
    if t.native.queue.len() > 3 {
        list = list.child(button(
            "queue-expand",
            if t.native.queue_expanded {
                "Show fewer"
            } else {
                "Show all queued"
            },
            p,
            |s| {
                let n = &mut s.current_mut().native;
                n.queue_expanded = !n.queue_expanded;
            },
        ));
    }
    list.child(
        row()
            .gap(8.0)
            .flex_wrap()
            .center_y()
            .child(label(format!("{count} queued messages"), SMALL, p.muted))
            .child(button("clear-queued-messages", "Return to draft", p, |s| {
                s.current_mut().clear_user_queue();
                s.checkpoint();
            })),
    )
}

/// Ported from KSG Code's TimelineV2EventRow/V2SubagentGroup and provider-pi result parsing.
/// These are extension-owned tasks, not Shina crew threads, so they have no invented thread links.
pub(super) fn subagent_results(tool: &Tool, p: Palette) -> El {
    let mut group = col()
        .key(format!("pi-subagents-{}", tool.id))
        .gap(6.0)
        .child(label(
            format!("{} Pi subagents", tool.subagents.len()),
            SMALL,
            p.muted,
        ));
    if tool.expanded {
        for (i, agent) in tool.subagents.iter().enumerate() {
            let mut card = col()
                .key(format!("pi-subagent-{}-{i}", tool.id))
                .padding(8.0)
                .gap(4.0)
                .radius(6.0)
                .border(0.5, p.line)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(icon("thread", p.muted))
                        .child(
                            label(crate::model::string(agent, "name"), SMALL, p.ink)
                                .truncate()
                                .grow(1.0)
                                .min_w(0.0),
                        )
                        .child(label(
                            crate::model::string(agent, "status"),
                            SMALL,
                            if agent["status"] == "Failed" {
                                p.danger
                            } else {
                                p.muted
                            },
                        )),
                )
                .child(label(crate::model::string(agent, "task"), SMALL, p.muted).wrap_anywhere());
            if let Some(model) = agent["model"].as_str() {
                card = card.child(label(model, SMALL, p.muted).wrap_anywhere());
            }
            if let Some(output) = agent["output"].as_str().filter(|s| !s.is_empty()) {
                card = card.child(
                    col()
                        .max_h(160.0)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(label(output, BODY, p.ink).wrap_anywhere().selectable(true)),
                );
            }
            group = group.child(card);
        }
    }
    group
}

/// KSG Code's context meter exposes its native compact action next to context details.
/// Ported from KSG Code's MessagesTimeline.logic.ts `context-compaction` rows:
/// collapsed summary, status and token figures.
pub(super) fn compaction_activity(s: &State, p: Palette, selected: usize) -> El {
    let t = s.current();
    if t.native.compactions.is_empty() {
        return el();
    }
    let mut list = col()
        .key(if selected == 0 {
            "compaction-history".into()
        } else {
            format!("compaction-position-{selected}")
        })
        .gap(6.0)
        .w_full()
        .min_w(0.0);
    for (index, item) in t
        .native
        .compactions
        .iter()
        .enumerate()
        .filter(|(index, _)| *index == selected)
    {
        let result = if item["result"].is_object() {
            &item["result"]
        } else {
            item
        };
        let title = if item["running"] == true {
            "Compacting context"
        } else if item["aborted"] == true {
            "Context compaction stopped"
        } else if item["errorMessage"].is_string() {
            "Context compaction failed"
        } else {
            "Context compacted"
        };
        let mut card = col()
            .key(format!("compaction-{index}"))
            .gap(6.0)
            .px(12.0)
            .py(8.0)
            .radius(6.0)
            .border(0.5, p.line)
            .child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        pixel_text(title, p.muted)
                            .min_w(0.0)
                            .shrink()
                            .wrap_anywhere(),
                    )
                    .child(spacer())
                    .child(button(
                        &format!("compaction-toggle-{index}"),
                        if t.native.compaction_expanded {
                            "Hide"
                        } else {
                            "Details"
                        },
                        p,
                        |s| {
                            let n = &mut s.current_mut().native;
                            n.compaction_expanded = !n.compaction_expanded;
                        },
                    )),
            );
        if let Some(before) = result["tokensBefore"].as_u64() {
            card = card.child(pixel_text(
                match result["estimatedTokensAfter"].as_u64() {
                    Some(after) => format!("{before} tokens before, about {after} after"),
                    None => format!("{before} tokens before"),
                },
                p.muted,
            ));
        }
        if t.native.compaction_expanded {
            let summary = item["errorMessage"]
                .as_str()
                .or_else(|| result["summary"].as_str())
                .unwrap_or(if t.harness == "claude" {
                    if item["running"] == true {
                        "Claude is compacting the conversation."
                    } else {
                        "Claude did not report a compaction summary or token counts."
                    }
                } else if t.harness == "codex" {
                    if item["running"] == true {
                        "Codex is compacting the conversation."
                    } else {
                        "Codex did not report a compaction summary or token counts."
                    }
                } else {
                    "Pi is summarizing the conversation."
                });
            card = card.child(
                col()
                    .max_h(240.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(label(summary, BODY, p.ink).wrap_anywhere().selectable(true)),
            );
        }
        list = list.child(card);
    }
    list
}

/// KSG's Edit from here confirmation and fork actions, using Pi's native list of forkable messages.
/// KSG has no equivalent fork-message picker; Pi's entry IDs require this list rather than text matching.
pub(super) fn conversation_picker(s: &State, p: Palette) -> El {
    let t = s.current();
    let mode = t.native.conversation.unwrap();
    let mut card = col()
        .key("conversation-picker")
        .w((s.viewport_width - 32.0).clamp(280.0, 560.0))
        .max_h((s.viewport_height - 120.0).max(240.0))
        .padding(16.0)
        .gap(12.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p));
    if let Some(entry) = &t.native.confirm_rewind {
        let entry = entry.clone();
        card = card.child(label("Edit from here?", TextStyle::new(16.0,600.0),p.ink))
            .child(label("Rewind chat to before this message. Your prompt returns to the composer. Files are not changed. Pi keeps the original session on disk.",BODY,p.muted).wrap())
            .child(row().gap(8.0).flex_wrap()
                .child(button("rewind-confirm", "Revert and keep changes",p,move |s| { s.current_mut().rewind(entry.clone()); s.checkpoint(); }))
                .child(secondary_button("rewind-cancel", "Cancel",p,|s| s.current_mut().native.confirm_rewind = None)));
    } else {
        card = card.child(label(
            if mode == ConversationMode::Fork {
                "Fork conversation"
            } else {
                "Edit from here"
            },
            DIALOG_TITLE,
            p.ink,
        ));
        let mut list = col().max_h(360.0).gap(4.0).scroll(ScrollSpec {
            stick_bottom: false,
        });
        if t.native.fork_points.is_empty() {
            list = list.child(label("No forkable messages loaded.", BODY, p.muted));
        }
        for (i, point) in t.native.fork_points.iter().enumerate() {
            let Some(entry) = point["entryId"].as_str() else {
                continue;
            };
            let entry = entry.to_owned();
            list = list.child(
                row()
                    .key(format!("fork-point-{i}"))
                    .role("button")
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .min_h(36.0)
                    .px(8.0)
                    .py(6.0)
                    .radius(4.0)
                    .hover(p.selected)
                    .child(
                        label(crate::model::string(point, "text"), BODY, p.ink)
                            .truncate()
                            .shrink()
                            .min_w(0.0),
                    )
                    .on_press(move |s: &mut State| {
                        if mode == ConversationMode::Fork {
                            s.fork_conversation(Some(entry.clone()));
                        } else {
                            s.current_mut().native.confirm_rewind = Some(entry.clone());
                        }
                    }),
            );
        }
        card = card
            .child(list)
            .child(secondary_button("conversation-cancel", "Cancel", p, |s| {
                s.current_mut().native.conversation = None
            }));
    }
    modal_frame(
        "conversation-modal",
        s,
        |s| s.current_mut().native.conversation = None,
        card,
    )
}

/// Ported from KSG Code's ComposerFooterModeControls and runtimeModeConfig permission menu.
pub(super) fn access_picker(s: &State, p: Palette) -> El {
    let t = s.current();
    let (modes, _) = crate::model::resolve_compatible_access_mode(
        t.native.access,
        crate::model::supported_access_modes(&t.harness),
    );
    let mut card = col()
        .key("access-picker")
        .w((s.viewport_width - 32.0).clamp(280.0, 400.0))
        .padding(8.0)
        .gap(4.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p));
    for mode in modes {
        let description = mode.description();
        let denial = crate::model::managed_access_mode_denial(s.managed_policy.as_ref(), mode);
        let disabled = denial.is_some();
        let mut row = col()
            .key(format!("access-{}", mode.id()))
            .role("button")
            .label(mode.label())
            .disabled(disabled)
            .padding(8.0)
            .gap(4.0)
            .radius(4.0)
            .child(label(
                mode.label(),
                LABEL,
                if disabled { p.muted } else { p.ink },
            ))
            .child(label(description, SMALL, p.muted).wrap());
        if let Some(reason) = denial {
            row = row.child(
                label(&reason, SMALL, p.muted)
                    .key(format!("access-{}-denial", mode.id()))
                    .wrap(),
            );
        } else {
            row = row
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .hover(p.selected)
                .on_press(move |s: &mut State| {
                    s.set_access_mode(mode);
                    s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
                });
        }
        card = card.child(row);
    }
    card = card.child(label("Tool approvals do not sandbox the process. Trusted extensions retain your account's permissions.",SMALL,p.muted).wrap().padding(8.0));
    modal_frame(
        "access-modal",
        s,
        |s| {
            s.current_mut().native.access_picker = false;
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        },
        card,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_option_fits_the_existing_permissions_menu_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut state = State::new(".".into(), false);
            state.current_mut().native.access_picker = true;
            let mut rt = rune::Runtime::with_text(
                state,
                view,
                (width, 720.0),
                rune::Env::new(false, 1.0),
                rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
            );
            rt.frame(0.0);
            let (option, _) = rt.presented("access-read-only").expect("Read only option");
            assert!(option[2] > 0.0 && option[3] > 0.0);
            assert!(option[0] >= 0.0 && option[0] + option[2] <= width);
        }
    }

    #[test]
    fn access_picker_filters_by_harness_and_disables_modes_above_managed_ceiling() {
        fn find_node<'a>(tree: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
            if tree["key"] == key {
                return Some(tree);
            }
            tree["children"]
                .as_array()?
                .iter()
                .find_map(|child| find_node(child, key))
        }

        let dir = crate::store::directory().unwrap();
        let policy_path = dir.join("policy.json");
        std::fs::write(
            &policy_path,
            r#"{"schemaVersion":1,"organization":"Example Corp","limits":{"maxRuntimeMode":"auto-accept-edits"}}"#,
        )
        .unwrap();
        let policy = crate::model::pi_session::read_managed_policy_from_path(&policy_path);

        for width in [620.0, 1280.0] {
            // Codex thread shows Auto.
            let mut codex_state = State::new(".".into(), false);
            codex_state.managed_policy = policy.clone();
            codex_state.current_mut().harness = "codex".into();
            codex_state.open_access_picker();
            let mut rt = rune::Runtime::with_text(
                codex_state,
                view,
                (width, 720.0),
                rune::Env::new(false, 1.0),
                rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
            );
            rt.frame(0.0);
            assert!(
                rt.presented("access-auto").is_some(),
                "{width}: codex shows Auto"
            );
            let tree = rt.accessibility_json();
            let full_node = find_node(&tree, "access-full-access").expect("Full access row");
            assert!(
                full_node["states"]
                    .as_array()
                    .is_some_and(|states| states.iter().any(|s| s == "disabled")),
                "{width}: Full access should be disabled under auto-accept-edits ceiling"
            );
            assert!(
                rt.presented("access-full-access-denial").is_some(),
                "{width}: Full access denial reason should be presented"
            );
            assert!(
                tree.to_string()
                    .contains("Example Corp limits threads to Auto-accept edits."),
                "{width}: denial reason text should appear in accessibility tree"
            );

            // Pi thread does not show Auto.
            let mut pi_state = State::new(".".into(), false);
            pi_state.managed_policy = policy.clone();
            pi_state.current_mut().harness = "pi".into();
            pi_state.open_access_picker();
            let mut rt = rune::Runtime::with_text(
                pi_state,
                view,
                (width, 720.0),
                rune::Env::new(false, 1.0),
                rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
            );
            rt.frame(0.0);
            assert!(
                rt.presented("access-auto").is_none(),
                "{width}: pi hides Auto"
            );
            let tree = rt.accessibility_json();
            let full_node = find_node(&tree, "access-full-access").expect("Full access row");
            assert!(
                full_node["states"]
                    .as_array()
                    .is_some_and(|states| states.iter().any(|s| s == "disabled")),
                "{width}: Full access should be disabled on pi too"
            );
        }
    }
}
