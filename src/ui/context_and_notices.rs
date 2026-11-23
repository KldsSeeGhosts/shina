use super::*;

/// The pill's ring is smaller than the popover era's: 16 pt inside a 24 pt row
/// (spec §4.4).
pub(super) const CONTEXT_PILL_RING: f32 = 16.0;

pub(super) fn context_limit(t: &Thread) -> Option<u64> {
    t.context.usage.limit(t.context_window)
}

/// The ring appears once Pi or the model catalog knows a capacity, or once Pi reports usage.
pub(super) fn context_available(t: &Thread) -> bool {
    t.context.usage.has_usage() || context_limit(t).is_some() || usage_limits_available(t)
}

pub(super) fn usage_limits_available(t: &Thread) -> bool {
    t.harness_state["usageLimits"]["windows"]
        .as_array()
        .is_some_and(|windows| !windows.is_empty())
}

/// A fresh draft has nothing to measure yet, so it shows no empty dashed ring.
pub(super) fn context_ring_shown(t: &Thread) -> bool {
    usage_limits_available(t)
        || context_available(t)
            && (t.context.usage.has_usage()
                || t.context.phase != ContextPhase::Idle
                || !t.messages.is_empty())
}

pub(super) fn context_ink(percent: f64, p: Palette) -> Color {
    if percent >= 90.0 {
        p.danger
    } else if percent >= 70.0 {
        p.brass
    } else {
        p.sea
    }
}

/// Rune strokes a polyline, so a circle is a ring of points in the element's own points.
pub(super) fn circle_points(cx: f32, cy: f32, radius: f32, steps: usize) -> Vec<[f32; 2]> {
    (0..steps.max(8))
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / steps.max(8) as f32;
            [cx + radius * angle.cos(), cy + radius * angle.sin()]
        })
        .collect()
}

pub(super) fn arc_points(
    cx: f32,
    cy: f32,
    radius: f32,
    start_degrees: f32,
    sweep_degrees: f32,
    steps: usize,
) -> Vec<[f32; 2]> {
    let steps = steps.max(2);
    (0..=steps)
        .map(|step| {
            let degrees = start_degrees + sweep_degrees * step as f32 / steps as f32;
            let angle = degrees.to_radians();
            [cx + radius * angle.cos(), cy + radius * angle.sin()]
        })
        .collect()
}

/// A real circular progress ring drawn with Rune's vector canvas. The track stays visible when
/// usage is unknown, so looking full is never implied by empty data.
pub(super) fn context_ring_shape(context: &ContextMeter, p: Palette, size: f32) -> Shape {
    let center = size / 2.0;
    let radius = size / 2.0 - 2.5;
    let width = 2.0;
    let track = if context.phase == ContextPhase::Compacting {
        p.brass.alpha(0.65)
    } else {
        p.field_line.alpha(0.55)
    };
    let measured = context.phase != ContextPhase::Compacting && context.usage.percent.is_some();
    let mut shape = Shape::new();
    if measured {
        shape = shape.stroke_with(
            circle_points(center, center, radius, 64),
            true,
            width,
            LineCap::Round,
            track,
        );
    } else {
        // A dashed track reads as "no measurement", not as a genuine zero.
        for dash in 0..10 {
            shape = shape.stroke(
                arc_points(center, center, radius, -90.0 + dash as f32 * 36.0, 24.0, 6),
                width,
                track,
            );
        }
    }
    if let Some(percent) = context.usage.percent
        && measured
    {
        let sweep = (percent as f32).clamp(0.0, 100.0) * 3.6;
        if sweep >= 359.99 {
            shape = shape.stroke_with(
                circle_points(center, center, radius, 64),
                true,
                width,
                LineCap::Round,
                context_ink(percent, p),
            );
        } else if sweep > 0.0 {
            let steps = (sweep / 6.0).ceil().clamp(8.0, 96.0) as usize;
            shape = shape.stroke(
                arc_points(center, center, radius, -90.0, sweep, steps),
                width,
                context_ink(percent, p),
            );
        }
    }
    shape
}

/// KSG Code opens the meter's popover after the pointer rests 150 ms on the ring and, when it
/// offers Compact, closes it 150 ms after the pointer leaves both the ring and the popover.
pub(super) const CONTEXT_HOVER_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

/// Escape closes the usage popover from the ring or from inside it.
pub(super) fn context_escape(s: &mut State, event: &rune::KeyEvent) -> bool {
    if event.key == rune::Key::Escape && s.current().context.popover_open() {
        s.current_mut().context.close_popover();
        true
    } else {
        false
    }
}

/// A press, Enter or Space on the ring opens the popover, or closes one it opened.
pub(super) fn context_toggle(s: &mut State) {
    let context = &mut s.current_mut().context;
    if context.details_open {
        context.close_popover();
    } else {
        context.details_open = true;
    }
}

pub(super) fn context_pill_warnings(s: &State) -> (usize, usize) {
    let Some(captain) = captain_ui::home_brief_captain(s) else {
        return (0, 0);
    };
    let truncated = context_ui::sources(s, captain, context_ui::Audience::Captain)
        .iter()
        .filter(|source| source.delivery == context_ui::Delivery::Truncated)
        .count();
    let (_, left_out) = context_ui::essential_fit(&s.memories(captain));
    (truncated, left_out)
}

/// KSG Code's `ContextWindowMeter` keeps usage in the composer and details in
/// the popover. Shina also shows a warning when Captain context will be lost.
pub(super) fn context_ring(s: &State, t: &Thread, p: Palette, lane: f32) -> El {
    let limit = context_limit(t);
    let open = t.context.popover_open();
    let first_legend_key = open
        .then(|| context_legend_for_thread(t))
        .flatten()
        .and_then(|legend| breakdown_keys(&legend).into_iter().next())
        .map(|(_, legend_key)| legend_key);
    let (truncated, left_out) = context_pill_warnings(s);
    let warning = truncated + left_out;
    let mut a11y = t.context.accessible_label(limit);
    if truncated > 0 {
        a11y.push_str(&format!(
            ". {truncated} document{} truncated",
            if truncated == 1 { "" } else { "s" }
        ));
    }
    if left_out > 0 {
        a11y.push_str(&format!(
            ". {left_out} essential {} left out",
            if left_out == 1 { "memory" } else { "memories" }
        ));
    }
    // Under 480 pt the pill keeps only the ring and percent; under 400 pt only
    // the ring, with a brass corner dot while a warning applies.
    let compact = lane < 480.0;
    let minimal = lane < 400.0;
    let mut pill = row()
        .key("context-usage-ring")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(t.context_harness_text(&a11y))
        .focusable(true)
        .expanded(open)
        .h(24.0)
        .pl(4.0)
        .pr(if minimal { 4.0 } else { 8.0 })
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .fill(if open { p.selected } else { Color::TRANSPARENT })
        .hover(p.selected)
        .press(p.selected)
        .on_press(|s: &mut State| {
            let key = s.panel_key();
            s.right_panels
                .toggle(&key, crate::right_panel::SingletonKind::Context);
        })
        .on_secondary_press(|s: &mut State, _, _| context_toggle(s))
        .on_key(move |s: &mut State, event, tx| {
            if event.key == rune::Key::Enter {
                let key = s.panel_key();
                s.right_panels
                    .toggle(&key, crate::right_panel::SingletonKind::Context);
                return true;
            }
            if event.key == rune::Key::Tab
                && !event.mods.shift
                && let Some(first) = &first_legend_key
            {
                s.composer_ui.scope_focused.set(false);
                tx.focus(first.as_str(), true);
                s.current_mut().context.focus_segment(Some(0));
                return true;
            }
            context_escape(s, event)
        })
        .on_hover_change(|s: &mut State, inside, _| s.current_mut().context.hover(true, inside))
        .hover_delays(CONTEXT_HOVER_DELAY, CONTEXT_HOVER_DELAY)
        .child({
            let ring = canvas(context_ring_shape(&t.context, p, CONTEXT_PILL_RING))
                .size(CONTEXT_PILL_RING, CONTEXT_PILL_RING)
                .a11y_hidden();
            if minimal && warning > 0 {
                el().size(CONTEXT_PILL_RING, CONTEXT_PILL_RING)
                    .child(ring)
                    .child(
                        el().size(6.0, 6.0)
                            .fill(p.brass)
                            .radius(3.0)
                            .border(1.0, p.tray)
                            .pin(None, Some(-2.0), Some(-2.0), None)
                            .a11y_hidden(),
                    )
            } else {
                el().size(CONTEXT_PILL_RING, CONTEXT_PILL_RING).child(ring)
            }
        });
    if !minimal && let Some(percent) = t.context.usage.percent {
        pill = pill.child(
            label(
                context_percent_text(percent),
                TIMER,
                context_ink(percent, p),
            )
            .key("context-pill-percent"),
        );
    }
    if !compact && warning > 0 {
        let text = match (truncated > 0, left_out > 0) {
            (true, false) => format!("{truncated} truncated"),
            (false, true) => format!("{left_out} left out"),
            _ => format!("{warning} issues"),
        };
        pill = pill.child(
            row()
                .gap(5.0)
                .center_y()
                .child(el().size(6.0, 6.0).fill(p.brass).radius(3.0).a11y_hidden())
                .child(label(text, META, p.brass).key("context-pill-warning")),
        );
    }
    pill
}

/// KSG Code's `formatPercentage`: one decimal under 10 %, whole numbers above.
pub(super) fn context_percent_text(percent: f64) -> String {
    if !percent.is_finite() {
        return String::new();
    }
    if percent < 10.0 {
        let text = format!("{percent:.1}");
        format!("{}%", text.strip_suffix(".0").unwrap_or(&text))
    } else {
        format!("{}%", percent.round() as i64)
    }
}

/// KSG Code's `formatContextWindowTokens`: 950, 9.5k, 120k, 1.2m.
pub(super) fn context_tokens_text(value: u64) -> String {
    let trim = |text: String| text.strip_suffix(".0").map(str::to_owned).unwrap_or(text);
    match value {
        0..1_000 => value.to_string(),
        1_000..10_000 => format!("{}k", trim(format!("{:.1}", value as f64 / 1_000.0))),
        10_000..999_500 => format!("{}k", (value as f64 / 1_000.0).round() as u64),
        _ => format!("{}m", trim(format!("{:.1}", value as f64 / 1_000_000.0))),
    }
}

/// KSG Code's `formatContextWindowCost`: four decimals under a cent, two otherwise.
pub(super) fn context_cost_text(cost: f64) -> String {
    if cost > 0.0 && cost < 0.01 {
        format!("USD {cost:.4}")
    } else {
        format!("USD {cost:.2}")
    }
}

pub(super) fn attach_composer_notices(s: &State, p: Palette, lane: f32, composer_card: El) -> El {
    if let Some(notices) = composer_notice_stack(s, p, lane) {
        col().w_full().gap(0.0).child(notices).child(composer_card)
    } else {
        composer_card
    }
}

/// Ported from KSG Code's `ComposerRequestCard.Well`: inset radius-8 `surface-well`
/// (`p.code_bg` + hairline `p.line`) holding commands, question text, evidence or options.
pub(super) fn request_card_well(p: Palette, options: bool, content: El) -> El {
    let mut well = col()
        .w_full()
        .min_w(0.0)
        .radius(8.0)
        .fill(p.code_bg)
        .border(1.0, p.line);
    well = if options {
        well.padding(4.0)
    } else {
        well.px(10.0).py(8.0)
    };
    col()
        .pl(10.0)
        .pr(13.0)
        .w_full()
        .min_w(0.0)
        .child(well.child(content))
}

/// Ported from KSG Code's `ComposerRequestCard.Footer`: aligned action row below the body.
pub(super) fn request_card_footer(content: El) -> El {
    row()
        .w_full()
        .min_w(0.0)
        .flex_wrap()
        .center_y()
        .justify_end()
        .gap(8.0)
        .py(6.0)
        .pl(10.0)
        .pr(13.0)
        .child(content)
}

/// One unified request banner docked at the top of `ComposerSurface.Shell` (ported from
/// KSG Code's `ComposerRequestCard` and `ChildThreadApprovals`). Pages cleanly across the
/// current thread's non-questionnaire request, Captain home crew requests, and crew report
/// questions so at most one request surface ever appears above the composer card.
pub(super) fn composer_request_banner(s: &State, p: Palette, lane: f32) -> Option<El> {
    let t = s.current();
    let has_thread_req = t.request.is_some()
        && !t.request_held_by_typing_guard()
        && t.native.questionnaire.is_none();
    let is_home = s.is_captain_home(t);
    let crew_items = if is_home {
        crew_ui::needs_you_items(s)
    } else {
        Vec::new()
    };
    let user_reports = if is_home {
        crew_questions_ui::user_question_count(s)
    } else {
        0
    };
    let captain_line = if is_home {
        crew_questions_ui::captain_questions_line(s, p)
    } else {
        None
    };
    let thread_offset = usize::from(has_thread_req);
    let total = thread_offset + crew_items.len() + user_reports;
    if total == 0 && captain_line.is_none() {
        return None;
    }
    let page = if total > 0 {
        s.needs_you_page.min(total - 1)
    } else {
        0
    };
    let mut stack = col().w_full().min_w(0.0).padding(4.0).gap(4.0);
    let mut is_report_form = false;
    if total > 0 {
        if has_thread_req && page == 0 {
            stack = stack.child(dialog(s, p, page, total));
        } else {
            let crew_idx = page - thread_offset;
            if let Some(item) = crew_items.get(crew_idx) {
                stack = stack.child(crew_ui::needs_you_card(s, p, item, page, total, lane));
            } else {
                let report_idx = crew_idx - crew_items.len();
                let pager = (total > 1).then(|| crew_ui::pager_controls(p, page, total));
                if let Some(report_card) =
                    crew_questions_ui::user_question_card(s, p, report_idx, pager)
                {
                    is_report_form = true;
                    stack = stack.child(report_card);
                }
            }
        }
    }
    if let Some(line) = captain_line {
        if total > 0 {
            stack = stack.child(el().h(1.0).mx(10.0).fill(p.line).hairline());
        }
        stack = stack.child(line);
    }
    let mut region = col()
        .key(if is_home {
            "needs-you"
        } else {
            "composer-request"
        })
        .role("region")
        .label(if is_home {
            "Needs you"
        } else {
            "Pending request"
        })
        .w_full()
        .min_w(0.0)
        .shrink();
    if is_report_form {
        region = region.max_h(220.0).scroll(ScrollSpec {
            stick_bottom: false,
        });
    }
    Some(region.child(stack))
}

/// KSG Code's `ComposerSurface.Shell`: the 16 px tray that holds the unified request
/// banner, attached notices, the 15 px main card and the context strip under it, with a
/// hairline rim painted above the inner surfaces (matching `ComposerSurface.Shell::after`)
/// so the opaque card never covers the straight tray edges and the rim adds zero layout
/// height. The shell is always present, while the strip joins only when content renders
/// below the card.
pub(super) fn composer_shell(
    s: &State,
    p: Palette,
    request_banner: Option<El>,
    inner: El,
    strip: Option<El>,
) -> El {
    let rim = if !composer_disabled(s) && (s.attachment_drop_hover || s.current().draft.focused) {
        p.ring.alpha(0.6)
    } else {
        p.line
    };
    let mut shell = col()
        .key("composer-shell")
        .w_full()
        .radius(16.0)
        .fill(p.tray)
        .box_shadow(rune::Shadow {
            x: 0.0,
            y: 4.0,
            blur: 16.0,
            spread: 0.0,
            color: p.shadow.alpha(p.shadow.a * 0.6),
            inset: false,
        });
    if let Some(banner) = request_banner {
        shell = shell
            .child(banner)
            .child(el().w_full().h(1.0).fill(p.line).hairline());
    }
    shell = shell.child(inner);
    if let Some(strip) = strip {
        shell = shell.child(strip);
    }
    shell.child(
        el().key("composer-shell-rim")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(16.0)
            .border(1.0, rim)
            .inert(),
    )
}

pub(super) fn composer_notice_overflows(
    notice: &crate::composer_notices::Notice,
    lane: f32,
) -> bool {
    let title_style = TextStyle::new(12.0, 500.0).lh(16.0);
    let desc_style = TextStyle::new(12.0, 400.0).lh(16.0);
    let action_style = TextStyle::new(12.0, 500.0);
    let mut actions_width = 0.0;
    for act in &notice.actions {
        actions_width += 20.0 + rune::text::measure(&act.label, &[], action_style, None).0 + 4.0;
    }
    let dismiss_width = if notice.dismiss_label.is_some() {
        24.0 + 4.0
    } else {
        0.0
    };
    let available_text =
        (lane - 4.0 - 8.0 - 24.0 - 4.0 - actions_width - dismiss_width - 4.0).max(0.0);
    let title_width = rune::text::measure(&notice.title, &[], title_style, None).0;
    let desc_width = notice.description.as_deref().map_or(0.0, |desc| {
        4.0 + rune::text::measure(desc, &[], desc_style, None).0
    });
    title_width + desc_width > available_text + 0.5
}

/// Ported from KSG Code's `ComposerBannerStack` and `ComposerBanner` (`attached`,
/// `density="comfortable"`), inside the composer tray: 15 px outer corners on the top notice,
/// variant colour on the icon and a 2 px left edge.
pub(super) fn composer_notice_stack(s: &State, p: Palette, lane: f32) -> Option<El> {
    use crate::composer_notices::Variant;
    let notices = s.composer_notices.ordered();
    if notices.is_empty() {
        return None;
    }
    let title_style = TextStyle::new(12.0, 500.0).lh(16.0);
    let desc_style = TextStyle::new(12.0, 400.0).lh(16.0);
    let mut stack = col().key("composer-notices").w_full().gap(0.0);
    for (index, notice) in notices.iter().enumerate() {
        let top = index == 0;
        let corners = if top {
            [15.0, 15.0, 0.0, 0.0]
        } else {
            [0.0; 4]
        };
        let variant_color = match notice.variant {
            Variant::Error => p.danger,
            Variant::Warning => p.brass,
            Variant::Success | Variant::Info => p.sea,
            Variant::Default => p.muted,
        };
        let role = match notice.variant {
            Variant::Error | Variant::Warning => "alert",
            Variant::Default | Variant::Info | Variant::Success => "status",
        };
        let a11y_label = match &notice.description {
            Some(desc) if !desc.is_empty() => format!("{}: {desc}", notice.title),
            _ => notice.title.clone(),
        };
        let overflows = composer_notice_overflows(notice, lane);

        let mut content = row()
            .key(format!("composer-notice-content-{}", notice.id))
            .grow(1.0)
            .shrink()
            .min_w(0.0)
            .center_y()
            .gap(4.0)
            .child(
                label(&notice.title, title_style, p.ink)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        if let Some(desc) = &notice.description {
            content = content.child(
                label(desc, desc_style, p.muted)
                    .truncate()
                    .grow(1.0)
                    .shrink()
                    .min_w(0.0),
            );
        }

        let mut actions = row().center_y().gap(4.0);
        if overflows {
            let details_id = notice.id.clone();
            actions = actions.child(button(
                &format!("composer-notice-details-{}", notice.id),
                "Details",
                p,
                move |s| s.composer_notices.toggle_details(&details_id),
            ));
        }
        for (action_index, act) in notice.actions.iter().enumerate() {
            let handler = act.handler.clone();
            let mut btn = button(
                &format!("composer-notice-action-{}-{action_index}", notice.id),
                &act.label,
                p,
                move |s| handler.call(s),
            )
            .disabled(act.disabled);
            if let Some(tip) = &act.tooltip {
                btn = btn.help(tip.clone());
            }
            actions = actions.child(btn);
        }
        if let Some(dismiss_label) = &notice.dismiss_label {
            let dismiss_id = notice.id.clone();
            actions = actions.child(icon_button(
                &format!("composer-notice-dismiss-{}", notice.id),
                dismiss_label,
                "close",
                p,
                move |s| s.dismiss_composer_notice(&dismiss_id),
            ));
        }

        let card_row = row()
            .min_h(24.0)
            .w_full()
            .min_w(0.0)
            .center_y()
            .gap(4.0)
            .child(
                el().size(24.0, 24.0)
                    .center()
                    .child(icon(notice.icon, variant_color)),
            )
            .child(content)
            .child(actions);

        let mut card = col()
            .key(format!("composer-notice-{}", notice.id))
            .role(role)
            .label(a11y_label)
            .w_full()
            .px(4.0)
            .py(5.0)
            .corners(corners)
            .clip_corners(corners)
            .child(
                el().w(2.0)
                    .fill(variant_color)
                    .pin(Some(0.0), Some(0.0), None, Some(0.0))
                    .a11y_hidden(),
            )
            .child(card_row);
        if let Some(body) = &notice.body {
            match body {
                crate::composer_notices::NoticeBody::UsageLimits(report) => {
                    card = card.child(usage_limits_notice_body(s, report, p));
                }
            }
        }

        stack = stack.child(card);
    }
    Some(stack)
}

pub(super) fn usage_progress_bar(
    key: String,
    fill_key: Option<String>,
    percent: f64,
    p: Palette,
) -> El {
    let fraction = percent.clamp(0.0, 100.0) as f32 / 100.0;
    let mut fill = el()
        .w_frac(fraction)
        .h_full()
        .radius(4.0)
        .fill(if percent >= 90.0 { p.danger } else { p.faint });
    if let Some(fill_key) = fill_key {
        fill = fill.key(fill_key);
    }
    row()
        .key(key)
        .role("progressbar")
        .label(format!("{} used", context_percent_text(percent)))
        .w_full()
        .h(6.0)
        .radius(4.0)
        .clip_rounded(4.0)
        .fill(p.line)
        .child(fill)
}

/// Ported from KSG Code's `UsageLimitsBannerBody` (`ComposerUsageLimits.tsx`) and
/// `ComposerBanner.Scroll` (`ComposerBanner.tsx`: `max-h-[min(24rem,40dvh)]`), skinned per
/// KSG Code's theme using `usage_progress_bar` and 12 px Inter figures.
pub(super) fn usage_limits_notice_body(
    s: &State,
    report: &crate::usage_limits::UsageLimitsReport,
    p: Palette,
) -> El {
    let max_h = if s.viewport_height > 0.0 {
        384.0_f32.min((s.viewport_height * 0.4).max(120.0))
    } else {
        384.0
    };
    let now_ms = crate::usage_limits::parse_iso_ms(&report.created_at)
        .unwrap_or_else(|| crate::lifecycle::now() as i64);
    let mut body = col()
        .w_full()
        .min_w(0.0)
        .pl(28.0)
        .pr(8.0)
        .pt(4.0)
        .pb(6.0)
        .gap(8.0);
    for (acc_idx, account) in report.accounts.iter().enumerate() {
        let mut acc_col = col()
            .key(format!("usage-limits-account-{acc_idx}"))
            .w_full()
            .min_w(0.0)
            .gap(4.0);
        if report.accounts.len() > 1 {
            acc_col = acc_col.child(
                label(
                    crate::usage_limits::account_summary(account),
                    SMALL,
                    p.muted,
                )
                .truncate()
                .w_full(),
            );
        }
        if let Some(notice_text) = crate::usage_limits::limits_notice(&account.limits) {
            acc_col = acc_col.child(label(notice_text, SMALL, p.muted).wrap().w_full());
        } else {
            for (win_idx, window) in account.limits.windows.iter().enumerate() {
                let reset_text =
                    crate::usage_limits::format_resets_in(window, now_ms).unwrap_or_default();
                let bar = usage_progress_bar(
                    format!("usage-limits-bar-{acc_idx}-{win_idx}"),
                    None,
                    window.used_percent,
                    p,
                );
                let win_row = row()
                    .key(format!("usage-limits-window-{acc_idx}-{win_idx}"))
                    .w_full()
                    .min_w(0.0)
                    .center_y()
                    .gap(12.0)
                    .child(
                        row()
                            .w(136.0)
                            .min_w(0.0)
                            .center_y()
                            .gap(6.0)
                            .child(
                                label(&window.label, SMALL, p.muted)
                                    .truncate()
                                    .min_w(0.0)
                                    .grow(1.0),
                            )
                            .child(meta_text(context_percent_text(window.used_percent), p.ink)),
                    )
                    .child(row().grow(1.0).min_w(48.0).center_y().child(bar))
                    .child(
                        row()
                            .min_w(84.0)
                            .justify_end()
                            .center_y()
                            .child(meta_text(reset_text, p.muted)),
                    );
                acc_col = acc_col.child(win_row);
            }
        }
        if let Some(external) = &account.limits.external_usage {
            let url = external.url.clone();
            acc_col = acc_col.child(row().w_full().child(button(
                &format!("usage-limits-manage-{acc_idx}"),
                "Manage usage",
                p,
                move |s| s.open_external_url(&url),
            )));
        }
        if let Some(input) = &account.reset_credit_input
            && let Some(credits) = &account.limits.reset_credits
            && credits.available_count > 0
        {
            for (c_idx, choice) in
                crate::usage_limits::reset_credit_choices(credits, input, now_ms, false)
                    .into_iter()
                    .enumerate()
            {
                if choice.available_count == 0 {
                    continue;
                }
                let btn_key = if c_idx == 0 {
                    format!("usage-limits-reset-{acc_idx}")
                } else {
                    format!("usage-limits-reset-{acc_idx}-{c_idx}")
                };
                let choice_input = choice.input.clone();
                acc_col = acc_col.child(
                    row()
                        .w_full()
                        .min_w(0.0)
                        .center_y()
                        .gap(12.0)
                        .child(meta_text(choice.summary, p.muted))
                        .child(button(&btn_key, &choice.action_label, p, move |s| {
                            s.redeem_usage_reset_credit(choice_input.clone())
                        })),
                );
            }
        }
        body = body.child(acc_col);
    }
    for notice_line in &report.notices {
        body = body.child(label(notice_line, SMALL, p.muted).wrap().w_full());
    }
    col()
        .key("usage-limits-scroll")
        .w_full()
        .min_w(0.0)
        .max_h(max_h)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(body)
}

/// Overflow details popover for a composer notice, reusing `context_popover` styling.
pub(super) fn composer_notice_details_popover(
    notice: &crate::composer_notices::Notice,
    p: Palette,
    lane: f32,
    cx: &Cx<State>,
) -> Option<El> {
    let trigger = format!("composer-notice-details-{}", notice.id);
    let row_key = format!("composer-notice-{}", notice.id);
    let anchor =
        if composer_notice_overflows(notice, lane) && cx.layout_with(&trigger, |_| ()).is_some() {
            trigger
        } else if cx.layout_with(&row_key, |_| ()).is_some() {
            row_key
        } else if cx.layout_with("composer", |_| ()).is_some() {
            "composer".to_owned()
        } else {
            return Some(
                el().key("composer-notice-details-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.composer_notices.close_details()),
            );
        };
    let mut card = col()
        .key("composer-notice-details-popover")
        .role("dialog")
        .label("Notice details")
        .w(360.0)
        .max_w(480.0)
        .padding(12.0)
        .gap(8.0)
        .radius(12.0)
        .clip_rounded(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .anchored_fit(anchor, rune::Side::Top, rune::Align::End, 4.0)
        .modal(true, |s: &mut State| s.composer_notices.close_details())
        .on_key(|s: &mut State, event, _| {
            if event.key == rune::Key::Escape {
                s.composer_notices.close_details();
                true
            } else {
                false
            }
        })
        .child(
            row()
                .center_y()
                .gap(12.0)
                .child(meta_label("Notice details", p.muted))
                .child(spacer())
                .child(icon_button(
                    "close-notice-details",
                    "Close",
                    "close",
                    p,
                    |s| s.composer_notices.close_details(),
                )),
        )
        .child(
            label(&notice.title, TextStyle::new(12.0, 500.0).lh(16.0), p.ink)
                .wrap()
                .selectable(true),
        );
    if let Some(desc) = &notice.description {
        card = card.child(
            label(desc, SMALL.lh(16.0), p.muted)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    Some(card)
}

/// Derives the breakdown legend for a thread when breakdown rows are available.
pub(super) fn context_legend_for_thread(
    t: &Thread,
) -> Option<Vec<crate::context_breakdown::ContextLegendItem>> {
    let limit = context_limit(t);
    let usage = &t.context.usage;
    let used_tokens = usage.tokens.unwrap_or_else(|| {
        usage
            .breakdown
            .as_deref()
            .map(|rows| {
                rows.iter()
                    .filter(|r| r.kind != crate::context_breakdown::BreakdownRowKind::Buffer)
                    .map(|r| r.tokens)
                    .sum()
            })
            .unwrap_or(0)
    });
    crate::context_breakdown::build_context_legend(usage.breakdown.as_deref(), limit, used_tokens)
}

pub(super) fn breakdown_slug(
    label: &str,
    kind: crate::context_breakdown::ContextLegendKind,
) -> String {
    use crate::context_breakdown::ContextLegendKind;
    match kind {
        ContextLegendKind::Other => "other".into(),
        ContextLegendKind::Buffer => "buffer".into(),
        ContextLegendKind::Free => "free".into(),
        ContextLegendKind::Used | ContextLegendKind::Estimated | ContextLegendKind::Inferred => {
            let mut slug = String::with_capacity(label.len());
            let mut prev_dash = false;
            for ch in label.chars() {
                if ch.is_ascii_alphanumeric() {
                    slug.push(ch.to_ascii_lowercase());
                    prev_dash = false;
                } else if !prev_dash && !slug.is_empty() {
                    slug.push('-');
                    prev_dash = true;
                }
            }
            while slug.ends_with('-') {
                slug.pop();
            }
            if slug.is_empty() {
                "segment".into()
            } else {
                slug
            }
        }
    }
}

pub(super) fn breakdown_keys(
    legend: &[crate::context_breakdown::ContextLegendItem],
) -> Vec<(String, String)> {
    use crate::context_breakdown::ContextLegendKind;
    let mut seen = std::collections::HashSet::new();
    legend
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let base = breakdown_slug(&item.label, item.kind);
            let unique = if seen.insert(base.clone()) {
                base
            } else {
                let disambiguated = format!("{base}-{index}");
                seen.insert(disambiguated.clone());
                disambiguated
            };
            let segment_key = if item.kind == ContextLegendKind::Buffer {
                "context-breakdown-reserve".to_owned()
            } else {
                format!("context-breakdown-segment-{unique}")
            };
            let legend_key = format!("context-breakdown-legend-{unique}");
            (segment_key, legend_key)
        })
        .collect()
}

/// Distinct existing Palette fills per segment kind, ordered by preference:
/// primary accent for the largest conversational segment, then status/existing fields.
pub(super) fn breakdown_segment_color(
    kind: crate::context_breakdown::ContextLegendKind,
    conversational_index: usize,
    p: Palette,
) -> Color {
    use crate::context_breakdown::ContextLegendKind;
    match kind {
        ContextLegendKind::Used | ContextLegendKind::Estimated => {
            let colors = [p.coral, p.sea, p.brass, p.ink, p.frame, p.message_bg];
            colors[conversational_index % colors.len()]
        }
        ContextLegendKind::Inferred => p.muted,
        ContextLegendKind::Other => p.field_line,
        ContextLegendKind::Free => p.line,
        ContextLegendKind::Buffer => p.faint,
    }
}

/// Segmented context breakdown bar and legend, ported from KSG Code's
/// `ContextWindowMeter.tsx` (`ContextWindowBreakdown`).
pub(super) fn context_breakdown_view(
    legend: &[crate::context_breakdown::ContextLegendItem],
    approximate: bool,
    active_segment: Option<usize>,
    p: Palette,
) -> El {
    use crate::context_breakdown::ContextLegendKind;
    let keys = breakdown_keys(legend);
    let legend_keys: Vec<String> = keys.iter().map(|(_, lk)| lk.clone()).collect();
    let total_tokens: u64 = legend.iter().map(|item| item.tokens).sum();

    let mut conversational_count = 0usize;
    let mut bar = row()
        .key("context-breakdown-bar")
        .role("img")
        .label("Context window breakdown")
        .w_full()
        .h(6.0);
    let mut rows = col().key("context-breakdown-legend").w_full().gap(2.0);

    for (index, item) in legend.iter().enumerate() {
        let color = breakdown_segment_color(item.kind, conversational_count, p);
        if matches!(
            item.kind,
            ContextLegendKind::Used | ContextLegendKind::Estimated
        ) {
            conversational_count += 1;
        }
        let (segment_key, legend_key) = &keys[index];
        let opacity = if active_segment.is_none() || active_segment == Some(index) {
            1.0
        } else {
            0.35
        };

        let mut segment = el()
            .key(segment_key.clone())
            .w(0.0)
            .grow(item.tokens.max(1) as f32)
            .min_w(0.0)
            .h_full()
            .opacity(opacity);
        if item.kind == ContextLegendKind::Buffer {
            segment = segment.border(1.0, p.faint);
        } else {
            segment = segment.fill(color);
        }
        bar = bar.child(segment);

        let prefix = if approximate
            && !matches!(
                item.kind,
                ContextLegendKind::Free | ContextLegendKind::Buffer
            ) {
            "~"
        } else {
            ""
        };
        let count_label = format!("{prefix}{}", context_tokens_text(item.tokens));
        let pct_label = if total_tokens > 0 {
            context_percent_text(item.tokens as f64 * 100.0 / total_tokens as f64)
        } else {
            "0%".into()
        };

        let mut swatch = el().size(8.0, 8.0);
        if item.kind == ContextLegendKind::Buffer {
            swatch = swatch.border(1.0, p.faint);
        } else {
            swatch = swatch.fill(color);
        }

        let nav_keys = legend_keys.clone();
        let legend_row = row()
            .key(legend_key.clone())
            .role("button")
            .focusable(true)
            .w_full()
            .min_h(16.0)
            .px(2.0)
            .gap(6.0)
            .center_y()
            .hover(p.selected)
            .focus_fill(p.selected)
            .label(format!("{}, {}, {}", item.label, count_label, pct_label))
            .on_hover_change(move |s: &mut State, inside, _| {
                s.current_mut().context.hover_segment(index, inside);
            })
            .on_press(move |s: &mut State| {
                s.current_mut().context.focus_segment(Some(index));
            })
            .on_key(move |s: &mut State, event, tx| {
                let count = nav_keys.len();
                match event.key {
                    rune::Key::Tab if !event.mods.shift => {
                        if index + 1 < count {
                            tx.focus(nav_keys[index + 1].as_str(), true);
                            s.current_mut().context.focus_segment(Some(index + 1));
                            true
                        } else {
                            s.current_mut().context.focus_segment(None);
                            false
                        }
                    }
                    rune::Key::Tab if event.mods.shift => {
                        if index > 0 {
                            tx.focus(nav_keys[index - 1].as_str(), true);
                            s.current_mut().context.focus_segment(Some(index - 1));
                            true
                        } else {
                            s.current_mut().context.focus_segment(None);
                            false
                        }
                    }
                    rune::Key::Down | rune::Key::Right => {
                        let next = (index + 1) % count;
                        tx.focus(nav_keys[next].as_str(), true);
                        s.current_mut().context.focus_segment(Some(next));
                        true
                    }
                    rune::Key::Up | rune::Key::Left => {
                        let prev = (index + count - 1) % count;
                        tx.focus(nav_keys[prev].as_str(), true);
                        s.current_mut().context.focus_segment(Some(prev));
                        true
                    }
                    _ => context_escape(s, event),
                }
            })
            .child(swatch)
            .child(
                label(&item.label, SMALL.lh(16.0), p.muted)
                    .truncate()
                    .min_w(0.0)
                    .grow(1.0),
            )
            .child(label(count_label, TIMER, p.ink))
            .child(label(pct_label, TIMER, p.muted));
        rows = rows.child(legend_row);
    }

    let framed_bar = el()
        .w_full()
        .border(1.0, p.frame)
        .border_layout()
        .child(bar);
    col().w_full().gap(8.0).child(framed_bar).child(rows)
}

/// The usage popover, ported from KSG Code's `ContextWindowMeter` popup: a `w-64` (or `w-80`
/// with a segmented breakdown) tooltip-style card above the ring, aligned to its end, with
/// "Context Window" and `percent · used/limit` in `text-2xs tabular-nums`, the breakdown or
/// aggregate usage bar, total processed and cost rows, the compaction state, and a full-width
/// "Compact context" button.
pub(super) fn context_popover(s: &State, p: Palette) -> El {
    let t = s.current();
    let limit = context_limit(t);
    let usage = &t.context.usage;
    let legend = context_legend_for_thread(t);
    let last_legend_key = legend
        .as_deref()
        .and_then(|items| breakdown_keys(items).into_iter().last())
        .map(|(_, legend_key)| legend_key);
    // Figures use tabular digits; their names stay regular weight.
    let secondary = TIMER;
    let strong = TIMER;
    let mut figures = row().center_y().gap(8.0);
    if let Some(percent) = usage.percent {
        figures = figures.child(
            label(context_percent_text(percent), secondary, p.muted).key("context-details-percent"),
        );
    }
    let used = match (usage.tokens, limit) {
        (Some(tokens), Some(limit)) => {
            format!(
                "{}/{}",
                context_tokens_text(tokens),
                context_tokens_text(limit)
            )
        }
        (Some(tokens), None) => context_tokens_text(tokens),
        (None, Some(limit)) => format!("Limit {}", context_tokens_text(limit)),
        (None, None) => "Not reported".into(),
    };
    figures = figures.child(label(used, secondary, p.muted).key("context-details-usage"));
    let mut card = col()
        .key("context-details")
        .role("dialog")
        .label("Context window")
        .w(if legend.is_some() { 320.0 } else { 256.0 })
        .padding(12.0)
        .gap(8.0)
        .radius(12.0)
        .clip_rounded(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .anchored("context-usage-ring", rune::Side::Top, rune::Align::End, 4.0)
        .on_key(|s: &mut State, event, _| context_escape(s, event))
        .on_hover_change(|s: &mut State, inside, _| s.current_mut().context.hover(false, inside))
        .hover_delays(std::time::Duration::ZERO, CONTEXT_HOVER_DELAY)
        .child(
            row()
                .center_y()
                .gap(12.0)
                .child(meta_label("Context window", p.muted))
                .child(spacer())
                .child(figures),
        );
    if let Some(legend) = &legend {
        let approximate =
            crate::context_breakdown::is_approximate(usage.breakdown.as_deref(), Some(legend));
        card = card.child(context_breakdown_view(
            legend,
            approximate,
            t.context.active_segment(),
            p,
        ));
    } else if let Some(limit) = limit
        && limit > 0
    {
        let fraction = usage.percent.unwrap_or(0.0).clamp(0.0, 100.0) as f32 / 100.0;
        let ink = if fraction > 0.9 { p.danger } else { p.faint };
        let mut bar = row()
            .key("context-details-bar")
            .role("progressbar")
            .label("Context window usage")
            .w_full()
            .h(6.0)
            .radius(4.0)
            .clip_rounded(4.0)
            .fill(p.line);
        if usage.percent.is_some() {
            bar = bar.child(el().w_frac(fraction).h_full().radius(4.0).fill(ink));
        }
        card = card.child(bar);
    }
    let figure_row = |name: &str, value: String| {
        row()
            .center_y()
            .gap(12.0)
            .child(label(name, SMALL, p.muted))
            .child(spacer())
            .child(label(value, strong, p.ink))
    };
    if let Some(tokens) = t.native.stats["tokens"]["total"]
        .as_u64()
        .filter(|tokens| *tokens > 0)
    {
        card = card.child(figure_row("Total processed", context_tokens_text(tokens)));
    }
    if let Some(cost) = t.native.stats["cost"]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)
    {
        card = card.child(figure_row("Cost", context_cost_text(cost)));
    }
    // Ported from KSG Code's usage/UsageLimits.WindowBar. The context placement is
    // Shina-specific, requested here; it reuses this popover's rows and 6 px bars.
    if let Some(windows) = t.harness_state["usageLimits"]["windows"].as_array()
        && !windows.is_empty()
    {
        let mut block = col()
            .key("context-usage-limits")
            .gap(8.0)
            .child(rule(p))
            .child(meta_text("Usage", p.muted).key("usage-heading"));
        for (index, window) in windows.iter().take(16).enumerate() {
            let Some(percent) = window["usedPercent"].as_f64().filter(|n| n.is_finite()) else {
                continue;
            };
            let reset = window["resetsAt"]
                .as_i64()
                .and_then(crate::usage_limits::format_reset_clock_secs)
                .map(|clock| format!("Resets {clock}"))
                .unwrap_or_else(|| "Reset not reported".into());
            block = block.child(
                col()
                    .key(format!("usage-window-{index}"))
                    .gap(4.0)
                    .child(
                        row()
                            .gap(8.0)
                            .center_y()
                            .child(
                                label(window["name"].as_str().unwrap_or("Usage"), SMALL, p.muted)
                                    .truncate()
                                    .min_w(0.0)
                                    .grow(1.0),
                            )
                            .child(
                                meta_text(context_percent_text(percent), p.ink)
                                    .key(format!("usage-percent-{index}")),
                            ),
                    )
                    .child(usage_progress_bar(
                        format!("usage-bar-{index}"),
                        Some(format!("usage-fill-{index}")),
                        percent,
                        p,
                    ))
                    .child(
                        meta_text(reset, p.muted)
                            .key(format!("usage-reset-{index}"))
                            .wrap(),
                    ),
            );
        }
        card = card.child(block);
    }
    if let Some(state) = t.context.state_text() {
        card = card.child(
            label(
                t.context_harness_text(state),
                TextStyle::new(12.0, 500.0).lh(16.0),
                p.muted,
            )
            .key("context-details-state")
            .wrap()
            .mt(4.0),
        );
    }
    let can_compact = !s.demo && t.can_compact();
    let mut compact = row()
        .key("compact-context")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label("Compact context")
        .w_full()
        .h(24.0)
        .mt(4.0)
        .gap(6.0)
        .justify_center()
        .center_y()
        .radius(6.0)
        .border(1.0, p.frame)
        .hover(p.selected)
        .press(p.selected)
        .opacity(if can_compact { 1.0 } else { 0.55 })
        .disabled(!can_compact)
        .child(icon("minimize", p.ink).size(14.0, 14.0))
        .child(label("Compact context", TextStyle::new(12.0, 500.0), p.ink));
    if can_compact {
        let legend_count = legend.as_ref().map(|items| items.len()).unwrap_or(0);
        compact = compact
            .focusable(true)
            .on_press(|s: &mut State| {
                s.current_mut().context.close_popover();
                s.compact_context(None);
            })
            .on_key(move |s: &mut State, event, tx| {
                if event.key == rune::Key::Tab
                    && event.mods.shift
                    && let Some(last) = &last_legend_key
                {
                    tx.focus(last.as_str(), true);
                    s.current_mut()
                        .context
                        .focus_segment(Some(legend_count - 1));
                    return true;
                }
                context_escape(s, event)
            });
    }
    card = card.child(compact);
    if !can_compact {
        let reason = if s.demo {
            "Demo threads cannot compact."
        } else if t.session.is_none() {
            "Available once the conversation has a saved session."
        } else if !crate::harness::supports_compaction(&t.harness) {
            "This ACP agent has no native compaction API."
        } else if t.harness == "pi" && t.model_provider != "cpa" {
            "Compaction needs a CPA model."
        } else {
            "Available when the agent is idle."
        };
        card = card.child(label(reason, secondary, p.muted).wrap());
    }
    card
}
