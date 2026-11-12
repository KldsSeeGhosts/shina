//! Rune ports of KSG Code's Pi command, compaction, and conversation controls.
use super::*;
use crate::model::ConversationMode;

/// Ported from KSG Code's ComposerPendingUserInputPanel: header, 14 px option labels,
/// descriptions, digit shortcuts, radio/checkbox rows and a scrollable composer replacement.
pub(super) fn questionnaire_drawer(s: &State, p: Palette) -> El {
    let t = s.current();
    let form = t.native.questionnaire.as_ref().expect("questionnaire");
    let question = &form.questions[form.index];
    let answer = &form.answers[form.index];
    let custom = !t.reply.text.trim().is_empty();
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
                            label(&option.description, TextStyle::new(11.0, 400.0), p.muted)
                                .wrap_anywhere(),
                        ),
                )
                .child(el().w(28.0).center().child(if i < 9 {
                    keycap_chip(&(i + 1).to_string(), p)
                } else {
                    el()
                }))
                .on_press(move |s: &mut State| s.current_mut().questionnaire_toggle(i)),
        );
    }
    let can_advance = custom || !answer.selected.is_empty();
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
                        .gap(8.0)
                        .child(
                            pixel_label(&question.header, p.muted)
                                .grow(1.0)
                                .min_w(0.0)
                                .truncate(),
                        )
                        .child(label(
                            format!("{}/{}", form.index + 1, form.questions.len()),
                            TIMER,
                            p.muted,
                        )),
                )
                .child(
                    col()
                        .key("questionnaire-scroll")
                        .w_full()
                        .max_h((s.viewport_height * 0.32).clamp(96.0, 260.0))
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(label(&question.text, BODY, p.ink).wrap_anywhere())
                        .child(options),
                )
                .child(
                    text_input(&t.reply, BODY)
                        .key("dialog-reply")
                        .label("Other answer")
                        .placeholder("Other answer...", p.muted)
                        .color(p.ink)
                        .w_full()
                        .min_h(32.0)
                        .max_h(72.0)
                        .px(8.0)
                        .py(6.0)
                        .radius(6.0)
                        .fill(p.card)
                        .field_focus(&t.reply, p)
                        .on_edit(|s: &mut State| s.current_mut().questionnaire_edit())
                        .on_submit(|s: &mut State, _| s.current_mut().questionnaire_step(false)),
                )
                .child(
                    row()
                        .gap(6.0)
                        .flex_wrap()
                        .child(
                            button("questionnaire-previous", "Previous", p, |s| {
                                s.current_mut().questionnaire_step(true)
                            })
                            .disabled(form.index == 0),
                        )
                        .child(
                            button(
                                if last {
                                    "dialog-submit"
                                } else {
                                    "questionnaire-next"
                                },
                                if last { "Submit reply" } else { "Next" },
                                p,
                                |s| s.current_mut().questionnaire_step(false),
                            )
                            .disabled(!can_advance),
                        )
                        .child(secondary_button("dialog-cancel", "Cancel", p, |s| {
                            s.current_mut().respond(json!({"cancelled":true}))
                        })),
                ),
        )
}

pub(crate) fn questionnaire_key(s: &mut State, event: &rune::KeyEvent) -> bool {
    if event.mods.ctrl
        || event.mods.cmd
        || event.mods.alt
        || s.current().reply.focused
        || s.search.focused
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

/// Ported from KSG Code's chat/ComposerCommandMenu.tsx: rounded rows, 12 px label and description,
/// a source badge, and a scrollable drawer above the composer. Pi supplies the workspace inventory.
pub(super) fn slash_menu(s: &State, p: Palette) -> El {
    let t = s.current();
    let matches = t.slash_matches();
    if !t.draft.text.starts_with('/')
        || t.draft.text.chars().any(char::is_whitespace)
        || !t.draft.focused
        || t.native.slash_dismissed.as_ref() == Some(&t.draft.text)
    {
        return el();
    }
    let mut list = col()
        .key("slash-menu")
        .role("listbox")
        .label("Pi commands")
        .w_full()
        .max_h(288.0)
        .padding(4.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if !t.native.commands_loaded {
        list = list.child(
            label(
                t.native
                    .commands_error
                    .as_deref()
                    .unwrap_or("Loading workspace commands..."),
                SMALL,
                p.muted,
            )
            .px(16.0)
            .py(10.0),
        );
    }
    if matches.is_empty() {
        list = list.child(
            label("No matching command.", SMALL, p.muted)
                .px(16.0)
                .py(10.0),
        );
    }
    let at = t.native.slash_index.min(matches.len().saturating_sub(1));
    for (i, command) in matches.iter().enumerate() {
        let name = command.name.clone();
        list = list.child(
            row()
                .key(format!("slash-item-{i}"))
                .role("option")
                .label(format!("/{} {}", command.name, command.description))
                .min_h(32.0)
                .px(8.0)
                .gap(8.0)
                .radius(4.0)
                .center_y()
                .cursor(rune::Cursor::Pointer)
                .fill(if i == at {
                    p.coral_bg
                } else {
                    Color::TRANSPARENT
                })
                .hover(p.selected)
                .on_press(move |s: &mut State| s.choose_slash(name.clone()))
                .child(
                    label(
                        format!("/{}", command.name),
                        TextStyle::new(12.0, 500.0),
                        p.ink,
                    )
                    .max_w(220.0)
                    .truncate()
                    .shrink(),
                )
                .child(
                    label(&command.description, SMALL, p.muted)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(pixel_label(&command.source, p.muted)),
        );
    }
    list
}

pub(super) fn composer_pi_key(s: &mut State, event: &rune::KeyEvent) -> bool {
    if s.undo_key(event, true) {
        return true;
    }
    let matches = s.current().slash_matches();
    if !matches.is_empty() {
        let at = s.current().native.slash_index.min(matches.len() - 1);
        match event.key {
            rune::Key::Down => {
                s.current_mut().native.slash_index = (at + 1).min(matches.len() - 1);
                return true;
            }
            rune::Key::Up => {
                s.current_mut().native.slash_index = at.saturating_sub(1);
                return true;
            }
            // The list's ends, as cmdk goes in KSG Code. The slash draft is one short line.
            rune::Key::Home => {
                s.current_mut().native.slash_index = 0;
                return true;
            }
            rune::Key::End => {
                s.current_mut().native.slash_index = matches.len() - 1;
                return true;
            }
            rune::Key::Tab | rune::Key::Enter
                if !event.mods.shift && !event.mods.ctrl && !event.mods.cmd =>
            {
                s.choose_slash(matches[at].name.clone());
                return true;
            }
            rune::Key::Escape => {
                s.current_mut().native.slash_dismissed = Some(s.current().draft.text.clone());
                return true;
            }
            _ => {}
        }
    }
    if event.key == rune::Key::Enter && (event.mods.ctrl || event.mods.cmd) {
        let previous = s.current().native.follow_up;
        s.current_mut().native.follow_up = !previous;
        s.send();
        s.current_mut().native.follow_up = previous;
        return true;
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
    let limit = if t.native.queue_expanded {
        usize::MAX
    } else {
        3
    };
    for queued in t.native.queue.iter().take(limit) {
        let id = queued.id;
        let mut body = col()
            .gap(6.0)
            .px(12.0)
            .py(8.0)
            .radius(6.0)
            .border(1.0, p.line)
            .max_w(580.0)
            .shrink()
            .min_w(0.0)
            .child(label(&queued.prompt.text, BODY, p.muted).wrap_anywhere());
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
                .child(
                    row()
                        .gap(2.0)
                        .flex_wrap()
                        .child(button(
                            &format!("queue-steer-{id}"),
                            "Send now",
                            p,
                            move |s| {
                                s.current_mut().steer_queued(id);
                                s.checkpoint();
                            },
                        ))
                        .child(
                            button(&format!("queue-edit-{id}"), "Edit", p, move |s| {
                                s.current_mut().edit_queued(id);
                                s.checkpoint();
                            })
                            .disabled(t.request.is_some()),
                        )
                        .child(button(
                            &format!("queue-remove-{id}"),
                            "Remove",
                            p,
                            move |s| {
                                s.current_mut().cancel_queued(id);
                                s.checkpoint();
                            },
                        )),
                ),
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
pub(super) fn compaction_activity(s: &State, p: Palette) -> El {
    let t = s.current();
    if t.native.compactions.is_empty() {
        return el();
    }
    let mut list = col().key("compaction-history").gap(6.0).w_full();
    for (index, item) in t.native.compactions.iter().enumerate() {
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
                    .child(label(title, SMALL, p.muted))
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
            card = card.child(label(
                match result["estimatedTokensAfter"].as_u64() {
                    Some(after) => format!("{before} tokens before, about {after} after"),
                    None => format!("{before} tokens before"),
                },
                SMALL,
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
    use crate::model::AccessMode;
    let mut card = col()
        .key("access-picker")
        .w((s.viewport_width - 32.0).clamp(280.0, 400.0))
        .padding(8.0)
        .gap(4.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p));
    for (mode, description) in [
        (
            AccessMode::ReadOnly,
            "Read files and coordinate. Delegate code changes to a thread.",
        ),
        (
            AccessMode::Supervised,
            "Ask before commands and file changes.",
        ),
        (
            AccessMode::Edits,
            "Auto-approve edits, ask before other actions.",
        ),
        (
            AccessMode::Full,
            "Allow commands and edits without prompts.",
        ),
    ] {
        card = card.child(
            col()
                .key(format!("access-{}", mode.id()))
                .role("button")
                .label(mode.label())
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .padding(8.0)
                .gap(4.0)
                .radius(4.0)
                .hover(p.selected)
                .child(label(mode.label(), LABEL, p.ink))
                .child(label(description, SMALL, p.muted).wrap())
                .on_press(move |s: &mut State| s.set_access_mode(mode)),
        );
    }
    card = card.child(label("Tool approvals do not sandbox the process. Trusted extensions retain your account's permissions.",SMALL,p.muted).wrap().padding(8.0));
    modal_frame(
        "access-modal",
        s,
        |s| s.current_mut().native.access_picker = false,
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
}
