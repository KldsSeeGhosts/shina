//! The footer and Settings share the same projected update status.
use super::*;
use crate::update_status::{UpdateAction, UpdatePhase, UpdateSummary};

fn visual(phase: &UpdatePhase, p: Palette) -> (&'static str, Color, Color) {
    match phase {
        UpdatePhase::UpToDate => ("check", p.ok, Color::TRANSPARENT),
        UpdatePhase::Available { .. }
        | UpdatePhase::Downloading { .. }
        | UpdatePhase::FleetRunning { .. }
        | UpdatePhase::Ready { blocker: None, .. } => ("arrow-up-circle", p.sea, p.sea_bg),
        UpdatePhase::Ready { .. } => ("arrow-up-circle", p.faint, p.sea_bg),
        UpdatePhase::Failed { .. } => ("alert", p.brass, p.brass_bg),
        _ => ("refresh", p.muted, Color::TRANSPARENT),
    }
}

fn phase_icon(s: &State, summary: &UpdateSummary, p: Palette) -> El {
    let (glyph, ink, _) = visual(&summary.phase, p);
    let image = icon(glyph, ink);
    let progress = match summary.phase {
        UpdatePhase::Downloading { progress, .. } => Some(progress),
        UpdatePhase::FleetRunning { done, total } => {
            Some((total > 0).then(|| done as f32 / total as f32))
        }
        _ => None,
    };
    if let Some(progress) = progress {
        let fraction = progress.unwrap_or(0.28).clamp(0.0, 1.0);
        let shape = if fraction > 0.0 {
            let points: Vec<_> = (0..=24)
                .map(|i| {
                    let angle = i as f32 / 24.0 * fraction * std::f32::consts::TAU;
                    [8.0 + 7.0 * angle.sin(), 8.0 - 7.0 * angle.cos()]
                })
                .collect();
            rune::Shape::new().stroke(points, 2.0, p.sea)
        } else {
            rune::Shape::new()
        };
        let mut arc = el().absolute(0.0, 0.0).size(16.0, 16.0).shape(shape);
        if progress.is_none() && s.portraits.motion_allowed() {
            arc = arc.looping(rune::LoopSpec::rotation(1.2));
        }
        return el()
            .size(16.0, 16.0)
            .radius(8.0)
            .border(2.0, p.line)
            .center()
            .child(arc)
            .child(icon("arrow-up-circle", p.sea).size(8.0, 8.0));
    }
    if matches!(summary.phase, UpdatePhase::Checking) && !s.portraits.reduced_motion() {
        image.looping(rune::LoopSpec::rotation(1.2))
    } else {
        image
    }
}

/// Follows KSG Code's `SidebarUpdatePill`, reduced to a trailing footer control.
pub(super) fn footer(s: &State, p: Palette) -> El {
    let summary = s.update_summary();
    if matches!(summary.phase, UpdatePhase::Disabled) {
        return el();
    }
    let (_, ink, fill) = visual(&summary.phase, p);
    let caption = match &summary.phase {
        UpdatePhase::Available { targets, .. } if *targets > 1 => Some(targets.to_string()),
        UpdatePhase::Ready { .. } => Some("Restart".into()),
        UpdatePhase::FleetRunning { done, total } => Some(format!("{done}/{total}")),
        _ => None,
    };
    let mut trigger = row()
        .key("footer-update")
        .role("button")
        .focusable(true)
        .label(&summary.headline)
        .help(&summary.headline)
        .description(&summary.detail)
        .expanded(s.update_popover)
        .h(28.0)
        .min_w(28.0)
        .px(6.0)
        .gap(4.0)
        .center()
        .radius(8.0)
        .fill(fill)
        .hover(if fill == Color::TRANSPARENT {
            p.selected
        } else {
            fill
        })
        .hover_motion(MOTION_FAST, MOTION_FAST)
        .cursor(rune::Cursor::Pointer)
        .on_press(|s: &mut State| {
            s.update_popover = !s.update_popover;
            s.update_details = false;
            if s.update_popover && matches!(s.update_summary().phase, UpdatePhase::Idle) {
                s.check_for_updates();
            }
        })
        .child(phase_icon(s, &summary, p));
    if let Some(caption) = caption {
        trigger = trigger.child(label(caption, META, ink));
    }
    trigger
}

fn dispatch(s: &mut State, action: &UpdateAction) {
    match action {
        UpdateAction::CheckNow => s.check_for_updates(),
        UpdateAction::Retry => {
            if s.updater.failed_release_detail().is_some() {
                s.settings.notice = s.updater.retry_failed_release().err();
            } else if s.fleet_run_visible() {
                s.capsule_action();
            } else {
                s.check_for_updates();
            }
        }
        UpdateAction::UpdateEverything => {
            s.update_popover = false;
            s.open_fleet();
        }
        UpdateAction::Restart => s.restart_for_update = true,
        UpdateAction::RestartPostponed => s.restart_postponed_update(),
        UpdateAction::Postpone => s.postpone_update_restart(),
        UpdateAction::UpdateMachine(server) => {
            s.update_popover = false;
            s.open_fleet();
            if let Some(index) = s.fleet.rows.iter().position(|row| &row.server == server) {
                s.start_fleet_row(index);
            }
        }
    }
}

pub(super) fn content(s: &State, p: Palette, summary: &UpdateSummary, settings: bool) -> El {
    let mut card = col()
        .gap(8.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(phase_icon(s, summary, p))
                .child(
                    label(&summary.headline, CARD_TITLE, p.ink)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0)
                        .key(if settings {
                            "settings-update-headline"
                        } else {
                            "update-popover-headline"
                        }),
                ),
        )
        .child(label(&summary.detail, SMALL, p.muted).wrap());
    if let UpdatePhase::Downloading { progress, .. } = summary.phase {
        let mut track = row()
            .w_full()
            .h(4.0)
            .radius(2.0)
            .fill(p.line)
            .role("progressbar")
            .label("Download progress")
            .key(if settings {
                "settings-update-progress"
            } else {
                "update-popover-progress"
            });
        if let Some(progress) = progress {
            let ratio = progress.clamp(0.0, 1.0);
            track = track
                .a11y_value(format!("{}%", (ratio * 100.0).round()))
                .child(el().grow(ratio).h_full().radius(2.0).fill(p.sea))
                .child(el().grow(1.0 - ratio));
        }
        card = card.child(track);
    }
    if let Some(cause) = &summary.cause {
        card = card.child(
            label(cause, SMALL, p.brass)
                .wrap()
                .padding(8.0)
                .radius(8.0)
                .fill(p.brass_bg)
                .key(if settings {
                    "settings-failed-release"
                } else {
                    "update-failure-cause"
                }),
        );
    } else if settings && matches!(summary.phase, UpdatePhase::Failed { .. }) {
        card = card.child(
            label("See the update log for details.", SMALL, p.brass)
                .key("settings-failed-release")
                .wrap()
                .padding(8.0)
                .radius(8.0)
                .fill(p.brass_bg),
        );
    }
    if summary.machines.iter().any(|machine| !machine.is_app) {
        card = card
            .child(rule(p))
            .child(label("Machines", META_LABEL, p.faint));
        for machine in &summary.machines {
            let mut line = row()
                .min_h(28.0)
                .gap(6.0)
                .center_y()
                .child(
                    label(&machine.name, SMALL, p.ink)
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .child(
                    label(&machine.status, SMALL, p.muted)
                        .max_w(100.0)
                        .truncate(),
                );
            if let Some(action) = &machine.action {
                let action = action.clone();
                line = line.child(button(
                    &format!(
                        "{}-machine-{}",
                        if settings {
                            "settings-update"
                        } else {
                            "update"
                        },
                        machine.server
                    ),
                    "Update",
                    p,
                    move |s| dispatch(s, &action),
                ));
            }
            card = card.child(line);
        }
    }
    if let Some(action) = &summary.action {
        let (key, title) = match action {
            UpdateAction::CheckNow => ("update-check-now", "Check now"),
            UpdateAction::Restart => ("update-restart", "Restart now"),
            UpdateAction::RestartPostponed => ("update-restart", "Restart now"),
            UpdateAction::Postpone => ("update-popover-later", "Later"),
            UpdateAction::Retry => ("retry-update", "Retry"),
            UpdateAction::UpdateEverything => ("update-all-machines", "Update everything"),
            UpdateAction::UpdateMachine(_) => ("update-machine", "Update machine"),
        };
        let action = action.clone();
        let key = if settings {
            if key == "retry-update" {
                "settings-retry-update".into()
            } else {
                format!("settings-{key}-primary")
            }
        } else {
            key.into()
        };
        card = card.child(primary_button(&key, title, p, move |s| {
            dispatch(s, &action)
        }));
    }
    if let UpdatePhase::Ready {
        blocker: Some(reason),
        ..
    } = &summary.phase
    {
        card = card.child(
            primary_button(
                if settings {
                    "settings-update-restart"
                } else {
                    "update-restart"
                },
                "Restart now",
                p,
                |_| {},
            )
            .blocked(Some(reason.clone()), "update-restart", p, false),
        );
    }
    if !summary.raw.is_empty() {
        let key = if settings {
            "settings-update-log"
        } else {
            "update-popover-details"
        };
        card = card.child(
            button(
                key,
                if settings { "Update log" } else { "Details" },
                p,
                |s| s.update_details = !s.update_details,
            )
            .expanded(s.update_details),
        );
        if s.update_details {
            card = card.child(
                label(summary.raw.join("\n"), MONO_META, p.muted)
                    .wrap_anywhere()
                    .selectable(true),
            );
        }
    }
    if summary.dismissible {
        card = card.child(button(
            if settings {
                "settings-update-dismiss"
            } else {
                "update-popover-dismiss"
            },
            "Dismiss update notice",
            p,
            |s| {
                s.dismiss_result_notice();
                s.dismiss_provider_pill();
            },
        ));
    }
    card
}

pub(super) fn popover(s: &State, p: Palette) -> El {
    let summary = s.update_summary();
    content(s, p, &summary, false)
        .key("update-popover")
        .role("dialog")
        .label("Updates")
        .child(button(
            "open-update-settings",
            "Open update settings",
            p,
            |s| {
                s.update_popover = false;
                s.update_details = false;
                s.open_settings();
                s.settings.section = 5;
            },
        ))
        .w((s.viewport_width - 16.0).min(300.0))
        .max_h((s.viewport_height - 48.0).max(280.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(12.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .anchored_fit("footer-update", rune::Side::Top, rune::Align::End, 6.0)
        .modal(true, |s: &mut State| s.update_popover = false)
}
