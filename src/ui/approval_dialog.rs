use super::*;

/// Ported from KSG Code's ComposerPendingApprovalActions.
pub(super) fn approval_actions(
    s: &State,
    id: usize,
    inline: bool,
    allowed: bool,
    p: Palette,
) -> El {
    let Some(t) = s.threads.iter().find(|t| t.id == id) else {
        return el();
    };
    let Some(request) = &t.request else {
        return el();
    };
    let server = s
        .remote_task(id)
        .map(|r| r.server_id.as_str())
        .unwrap_or(&s.machines.selected);
    let disabled = !allowed
        || !t.requests_supported
        || t.responding_request.is_some()
        || !crate::approval::can_respond(request)
        || s.client.as_ref().is_some_and(|client| {
            !client.can_approve(server)
                || !client.supports_capability(server, crate::protocol::COMPOSER_REQUESTS)
        });
    let options = crate::approval::options(request);
    let option_button = |option: &crate::approval::OptionItem| {
        let decision = option.decision.clone();
        let ink = if decision == "accept" {
            p.on_coral
        } else {
            p.ink
        };
        let key = match (inline, decision.as_str()) {
            (false, "accept") => "dialog-confirm".to_owned(),
            (false, "acceptForSession") => "dialog-session".to_owned(),
            (false, "cancel") => "dialog-cancel".to_owned(),
            (false, "decline") => "dialog-decline".to_owned(),
            (true, "accept") => format!("approve-{id}"),
            (true, "decline") => format!("decline-{id}"),
            _ => format!("approval-{id}-{decision}"),
        };
        let mut content = row().gap(4.0).center_y();
        if option.warning.is_some() {
            content = content.child(icon("alert", ink));
        }
        content = content.child(if inline {
            label(&option.label, LABEL, ink).wrap().min_w(0.0)
        } else {
            label(&option.label, LABEL, ink).max_w(160.0).truncate()
        });
        let request_id = request["id"].clone();
        framed(
            row()
                .key(&key)
                .role("button")
                .label(&option.label)
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .center_y()
                .gap(4.0)
                .px(8.0)
                .min_h(24.0)
                .radius(6.0)
                .hover(p.selected)
                .press(p.selected)
                .fill(if option.decision == "accept" {
                    p.coral
                } else {
                    p.card
                })
                .child(content),
            p,
        )
        .on_press(move |s: &mut State| {
            if !disabled
                && s.threads
                    .iter()
                    .find(|t| t.id == id)
                    .and_then(|t| t.request.as_ref())
                    .is_some_and(|r| r["id"] == request_id)
            {
                let _ = s.respond_to_request(id, crate::approval::response(&decision));
            }
        })
        .disabled(disabled)
        .help(option.warning.as_deref().unwrap_or(&option.label))
    };
    let mut actions = row().gap(6.0).center_y().flex_wrap();
    for option in options
        .iter()
        .filter(|o| matches!(o.decision.as_str(), "decline" | "accept"))
    {
        actions = actions.child(option_button(option));
    }
    let more: Vec<_> = options
        .iter()
        .filter(|o| !matches!(o.decision.as_str(), "decline" | "accept"))
        .collect();
    if !more.is_empty() {
        let mut control = col().child(
            button(
                &format!("approval-more-{id}"),
                if inline { "More" } else { "..." },
                p,
                move |s| {
                    if let Some(t) = s.threads.iter_mut().find(|t| t.id == id) {
                        t.native.approval_more = !t.native.approval_more;
                    }
                },
            )
            .label("More approval options")
            .disabled(disabled),
        );
        if t.native.approval_more {
            let mut menu = popover(
                &format!("approval-menu-{id}"),
                "More approval options",
                s,
                p,
            );
            for option in more {
                menu = menu.child(option_button(option));
            }
            control = control.child(
                menu.anchored_fit(
                    format!("approval-more-{id}"),
                    rune::Side::Top,
                    rune::Align::End,
                    4.0,
                )
                .modal(true, move |s: &mut State| {
                    if let Some(t) = s.threads.iter_mut().find(|t| t.id == id) {
                        t.native.approval_more = false;
                    }
                }),
            );
        }
        actions = actions.child(control);
    }
    actions
}

/// Ported from KSG Code's `ComposerRequestCard`: 32 pt header with tone icon, title and
/// optional pager, inset `surface-well` body/input, and footer actions inside the composer
/// tray.
pub(super) fn dialog(s: &State, p: Palette, page: usize, total: usize) -> El {
    let t = s.current();
    let request = t.request.as_ref().expect("pending dialog");
    let remote = s.remote_task(t.id);
    let linked = remote.is_none_or(|r| {
        s.client
            .as_ref()
            .is_some_and(|c| c.commands_for(&r.server_id).is_some())
    });
    let method = crate::model::string(request, "method");
    let can_answer = linked
        && remote.is_none_or(|r| s.machine_connected(&r.server_id))
        && matches!(method, "confirm" | "input" | "editor" | "select");
    let title = crate::model::string(request, "title");
    let message = crate::model::string(request, "message");
    let header_icon = if method == "confirm" {
        "shield"
    } else {
        "info"
    };
    let mut header = row()
        .min_h(32.0)
        .w_full()
        .min_w(0.0)
        .pl(10.0)
        .pr(13.0)
        .center_y()
        .gap(8.0)
        .child(icon(header_icon, p.brass).size(13.0, 13.0).a11y_hidden())
        .child(
            label(
                if title.is_empty() {
                    if method == "confirm" {
                        "Approval required"
                    } else {
                        "Input required"
                    }
                } else {
                    title
                },
                LABEL,
                p.ink,
            )
            .grow(1.0)
            .min_w(0.0)
            .wrap(),
        );
    if total > 1 {
        header = header.child(crew_ui::pager_controls(p, page, total));
    }
    let mut card = col()
        .key("pi-dialog")
        .role("group")
        .label("Pi needs your input")
        .w_full()
        .min_w(0.0)
        .gap(4.0)
        .child(header);
    if !message.is_empty() {
        card = card.child(request_card_well(
            p,
            false,
            label(message, BODY, p.ink).wrap(),
        ));
    }
    if !t.requests_supported {
        card = card.child(
            label(crate::protocol::REQUESTS_UPDATE_REQUIRED, SMALL, p.danger)
                .pl(10.0)
                .pr(13.0)
                .wrap(),
        );
    }
    let mut actions = row().gap(6.0).flex_wrap().center_y();
    match method {
        "confirm" => {
            actions = approval_actions(s, t.id, false, can_answer, p);
        }
        "select" => {
            if let Some(options) = request["options"].as_array() {
                for (index, option) in options.iter().enumerate() {
                    if let Some(option) = option.as_str() {
                        let value = option.to_owned();
                        actions = actions.child(button(
                            &format!("dialog-option-{index}"),
                            option,
                            p,
                            move |s| {
                                let _ =
                                    s.respond_to_request(s.current().id, json!({"value":value}));
                            },
                        ));
                    }
                }
            }
        }
        _ => {
            card = card.child(
                col().pl(10.0).pr(13.0).w_full().min_w(0.0).child(
                    text_input(&t.reply, BODY)
                        .key("dialog-reply")
                        .label("Reply to Pi")
                        .placeholder(crate::model::string(request, "placeholder"), p.muted)
                        .color(p.ink)
                        .w_full()
                        .min_h(32.0)
                        .max_h(120.0)
                        .px(10.0)
                        .py(6.0)
                        .radius(8.0)
                        .fill(p.card)
                        .field_focus(&t.reply, p)
                        .disabled(!can_answer)
                        .on_edit(|s: &mut State| &mut s.current_mut().reply),
                ),
            );
            actions = actions.child(button("dialog-submit", "Submit reply", p, |s| {
                let value = s.current().reply.text.clone();
                let _ = s.respond_to_request(s.current().id, json!({"value":value}));
            }));
        }
    }
    if method != "confirm" {
        actions = actions.child(button("dialog-cancel", "Cancel", p, |s| {
            let _ = s.respond_to_request(s.current().id, json!({"cancelled":true}));
        }));
    }
    let mut footer_row = row()
        .gap(6.0)
        .flex_wrap()
        .center_y()
        .child(actions.disabled(!can_answer));
    if !linked && let Some(remote) = remote {
        card = card.child(
            label(
                format!("Link this desktop to {} to answer.", remote.label),
                SMALL,
                p.muted,
            )
            .pl(10.0)
            .pr(13.0)
            .wrap(),
        );
        footer_row = footer_row.child(crew_ui::ghost_button(
            s,
            "dialog-link-machine".into(),
            "Link machine",
            p,
            |s| s.machines.dialog = true,
        ));
    }
    card.child(request_card_footer(footer_row))
}
