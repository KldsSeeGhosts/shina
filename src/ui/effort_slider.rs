use super::*;

pub(super) const SLIDER_H: f32 = 28.0;

/// The closed hand a held slider shows, as Codex's `cursor: grabbing`. winit draws
/// `Cursor::Grabbing` as the four-way move arrow on Windows, so Shina ships its own, drawn by
/// `tools/cursors.py` at 2x with the hot spot in the palm.
pub(super) const GRABBING_CURSOR: rune::CursorImage = rune::CursorImage::png(
    include_bytes!("../../assets/cursors/grabbing.png"),
    [16.0, 16.0],
);

/// Codex's top-level flourish: specks of light drift through a full track and twinkle.
/// Each speck rides a looping slide across the track and pulses on its own opacity loop.
pub(super) fn sparkles(p: Palette) -> El {
    // (height, rise, period, delay, size), fixed so the pattern never jumps on a redraw.
    const SPECKS: [(f32, f32, f64, f64, f32); 9] = [
        (9.0, -0.10, 2.6, 0.0, 2.0),
        (17.0, -0.18, 3.1, 0.4, 1.5),
        (13.0, 0.08, 2.2, 0.9, 2.5),
        (6.0, 0.12, 3.6, 1.3, 1.5),
        (20.0, -0.22, 2.8, 1.7, 2.0),
        (11.0, -0.06, 3.3, 2.1, 1.5),
        (15.0, 0.16, 2.4, 0.2, 2.0),
        (8.0, 0.20, 3.9, 2.6, 1.5),
        (18.0, -0.12, 2.9, 1.1, 2.5),
    ];
    let mut layer = el()
        .key("effort-sparkles")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .a11y_hidden();
    for (index, (y, rise, period, delay, size)) in SPECKS.into_iter().enumerate() {
        let mut drift =
            rune::LoopSpec::slide(period, [-0.08, 0.0], [1.0, rise]).easing(rune::Easing::LINEAR);
        drift.delay = delay;
        layer = layer.child(
            el().key(format!("speck-{index}"))
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .looping(drift)
                .child(
                    el().absolute(0.0, y - size / 2.0)
                        .size(size, size)
                        .radius(size / 2.0)
                        .fill(p.brass)
                        .looping(rune::LoopSpec::opacity(period / 2.0, delay, 0.15, 0.95)),
                ),
        );
    }
    layer
}

/// Codex's slider motion, from its `--model-picker-power-slider-motion-*` variables: 0.3 s on
/// `cubic-bezier(.23, 1, .32, 1)`, a decisive ease-out with no overshoot, and 0.15 s while the
/// thumb is dragged. Rune lands drag commits at once by default, so each drag phase asks for
/// this curve explicitly; otherwise a release jumps to its level instead of gliding there.
pub(super) fn slider_curve(dragging: bool) -> rune::Curve {
    if dragging {
        rune::Curve::tween(0.15, rune::Easing::cubic(0.23, 1.0, 0.32, 1.0))
    } else {
        MOTION_SLOW
    }
}

/// Which control an effort drag belongs to: the thread, model, option id and levels the
/// press started on. A remounted slider reuses the same element key, so this hash is how a
/// held gesture knows its release still lands on the control it began in.
pub(super) fn effort_drag_key(context: &str, option_id: &str, levels: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    context.hash(&mut hasher);
    option_id.hash(&mut hasher);
    levels.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn effort_slider_with_levels(
    levels: &[String],
    active_level: &str,
    dragging: Option<crate::model::ThinkingDrag>,
    p: Palette,
    can_choose: bool,
    option_id: String,
    context: String,
    locks_when_body: bool,
) -> El {
    let levels = levels.to_vec();
    let count = levels.len();
    let current = levels.iter().position(|level| *level == active_level);
    // The gesture signature: the thread, model, option and levels this slider was built
    // for. A drag whose stored key differs belongs to a control that has since been
    // rebuilt for another context, and its events are stale.
    let gesture_key = effort_drag_key(&context, &option_id, &levels);
    // While held, the thumb follows the pointer and the title names the nearest level.
    let held = dragging
        .filter(|d| d.key == gesture_key && count > 1)
        .map(|d| d.frac.clamp(0.0, 1.0));
    let shown = held
        .map(|f| (f * (count - 1) as f32).round() as usize)
        .or(current);
    let title = shown
        .and_then(|index| levels.get(index))
        .map_or_else(|| effort_name(active_level), |level| effort_name(level));
    // Under the reasoning menu's header, so a little tighter than Codex's standalone slider.
    let mut section = col()
        .key("effort-section")
        .px(12.0)
        .pt(8.0)
        .pb(14.0)
        .gap(10.0);
    section = section.child(
        row()
            .center_y()
            .child(icon("bolt", p.muted).w(16.0))
            .child(
                label(
                    if count == 0 {
                        "Fixed reasoning".to_string()
                    } else {
                        title
                    },
                    TextStyle::new(15.0, 600.0),
                    if count == 0 { p.muted } else { p.sea },
                )
                .key("effort-title")
                .align(rune::Align::Center)
                .wrap()
                .grow(1.0),
            )
            .child(el().w(16.0)),
    );
    // Codex's geometry: a 24 px track inside a 28 px row, and a 28 px thumb whose centre runs
    // between the row's end caps. Fill, dots and thumb are fractions of the row.
    let at = shown.unwrap_or(0);
    let frac = held.unwrap_or(if count > 1 {
        at as f32 / (count - 1) as f32
    } else {
        1.0
    });
    let max = shown.is_some() && at + 1 == count && count > 1;
    let cap = SLIDER_H / 2.0;
    let mut slider = el()
        .key("effort-slider")
        .role("slider")
        .label("Reasoning effort")
        .a11y_value(effort_name(active_level))
        .opacity(if can_choose { 1.0 } else { 0.5 })
        .h(SLIDER_H)
        .w_full()
        .motion(rune::Spring::new(0.3, 0.0))
        // Only a press or drag shows the closed hand, as Codex does; hovering keeps the arrow.
        .cursor(if can_choose && held.is_some() {
            rune::Cursor::Image(GRABBING_CURSOR)
        } else {
            rune::Cursor::Default
        });
    let mut track = el()
        .key("effort-track")
        .pin(Some(0.0), Some(2.0), Some(0.0), Some(2.0))
        .radius(12.0)
        .clip_rounded(12.0)
        .fill(p.sea_bg)
        .border(1.0, p.line);
    if count == 0 {
        return section.child(slider.child(track).opacity(0.5)).child(label(
            "This model has no reasoning levels.",
            SMALL,
            p.muted,
        ));
    }
    let fill = |on: bool| if on { p.coral } else { Color::TRANSPARENT };
    // One fill, ending at cap + frac * (width - cap): under the thumb at the lowest level and
    // at the track's far end at the top. The thumb covers the part past its centre.
    track = track.child(
        row()
            .key("effort-fill")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .child(el().w(cap).h_full().fill(fill(shown.is_some())))
            .child(
                el().key("effort-fill-run")
                    .w(0.0)
                    .grow(frac)
                    .h_full()
                    .fill(fill(shown.is_some())),
            )
            .child(
                el().key("effort-fill-rest")
                    .w(0.0)
                    .grow(1.0 - frac)
                    .h_full(),
            ),
    );
    if max {
        track = track.child(sparkles(p));
    }
    slider = slider.child(track);
    let mut dots = row()
        .key("effort-dots")
        .pin(Some(cap - 2.0), Some(0.0), Some(cap - 2.0), Some(0.0))
        .center_y()
        .justify_between();
    for index in 0..count {
        let lit = shown.is_some_and(|at| index <= at);
        dots = dots.child(
            el().key(format!("choose-thinking-{index}"))
                .size(4.0, 4.0)
                .radius(4.0)
                .fill(if Some(index) == shown {
                    Color::TRANSPARENT
                } else if lit {
                    p.on_coral.alpha(0.7)
                } else {
                    p.muted.alpha(0.55)
                }),
        );
    }
    slider = slider.child(dots);
    if shown.is_some() {
        slider = slider.child(
            row()
                .key("effort-thumb-row")
                .pin(Some(cap), Some(0.0), Some(cap), Some(0.0))
                .center_y()
                .child(el().key("effort-thumb-before").w(0.0).grow(frac))
                .child(
                    el().key("effort-thumb").w(0.0).h(SLIDER_H).child(
                        // Codex's white thumb with a hairline edge.
                        el().key("effort-thumb-knob")
                            .absolute(-cap, 0.0)
                            .size(SLIDER_H, SLIDER_H)
                            .radius(cap)
                            .fill(Color::WHITE)
                            .border(1.0, p.frame),
                    ),
                )
                .child(el().key("effort-thumb-after").w(0.0).grow(1.0 - frac)),
        );
    }
    {
        let keyed = levels.clone();
        let dragged = levels.clone();
        let ends = levels.clone();
        let oid_step = option_id.clone();
        let oid_limit = option_id.clone();
        let oid_drag = option_id;
        // Use the same stop the thumb renders, including a descriptor's current value.
        let effective_at = move |s: &State, levels: &[String]| {
            s.current()
                .selected_thinking
                .as_ref()
                .and_then(|level| levels.iter().position(|l| l == level))
                .or(current)
        };
        let can_pick = move |s: &State| {
            let t = s.current();
            t.can_choose_thinking()
                && !(locks_when_body
                    && crate::descriptors::read_prompt_ultrathink(&t.draft.text)
                        == crate::descriptors::PromptUltrathink::Body)
        };
        if can_choose {
            slider = slider
                // Rune's NSSlider keyboard: arrows and pages step, Home and End jump.
                .on_step(move |s: &mut State, delta, tx| {
                    if !can_pick(s) {
                        return;
                    }
                    let last = keyed.len() as i32 - 1;
                    let at = effective_at(s, &keyed).unwrap_or(0) as i32;
                    let next = (at + delta).clamp(0, last) as usize;
                    if next as i32 != at {
                        tx.motion_curve(MOTION_SLOW);
                        s.set_composer_option_sticky(
                            oid_step.clone(),
                            crate::model_selection::OptionValue::Str(keyed[next].clone()),
                        );
                    }
                })
                .on_step_limit(move |s: &mut State, max, tx| {
                    let index = if max { ends.len() - 1 } else { 0 };
                    if can_pick(s) && effective_at(s, &ends) != Some(index) {
                        tx.motion_curve(MOTION_SLOW);
                        s.set_composer_option_sticky(
                            oid_limit.clone(),
                            crate::model_selection::OptionValue::Str(ends[index].clone()),
                        );
                    }
                });
        }
        // The drag handler stays mounted while `can_choose` flips, so a press that began
        // while the control was enabled still hears its Up or cancel even if a turn
        // started meanwhile.
        slider = slider.on_drag(move |s: &mut State, drag, tx| {
            // The position a pointer x maps to between the end caps' centres, minus the
            // grab offset a knob press saved in `drag.data`.
            let along_at = |x: f32, grab: f32| {
                ((x - grab - cap) / (drag.w - 2.0 * cap).max(1.0)).clamp(0.0, 1.0)
            };
            let held_key = s.thinking_drag.map(|d| d.key);
            let stale = held_key.is_some_and(|key| key != gesture_key);
            tx.animated();
            if drag.cancelled() {
                if !stale {
                    tx.motion_curve(MOTION_SLOW);
                }
                s.thinking_drag = None;
                return;
            }
            if drag.phase == rune::Phase::Down {
                // A press only begins a gesture while the control answers choices. When
                // the turn itself is running the press previews nothing and commits
                // nothing. A configure in flight still accepts the pick: the model
                // serializes it behind the pending acknowledgement. An ultrathink in the
                // draft body locks the control the same way.
                s.thinking_drag = None;
                if !can_pick(s) || dragged.len() < 2 {
                    return;
                }
                // Where the thumb centre sits for the effective level, so a knob press
                // keeps the grab offset instead of snapping the centre to the pointer.
                let last = (dragged.len() - 1).max(1) as f32;
                let at = effective_at(s, &dragged).unwrap_or(0);
                let thumb_x =
                    cap + (at as f32 / last).clamp(0.0, 1.0) * (drag.w - 2.0 * cap).max(0.0);
                let on_knob = (drag.x - thumb_x).abs() <= cap;
                drag.data = if on_knob { drag.x - thumb_x } else { 0.0 };
                // A track press glides the thumb to the pointer's nearest stop; a knob
                // press starts from where the thumb already is.
                let along = if on_knob {
                    at as f32 / last
                } else {
                    along_at(drag.x, 0.0)
                };
                tx.motion_curve(MOTION_SLOW);
                s.thinking_drag = Some(crate::model::ThinkingDrag {
                    frac: along.clamp(0.0, 1.0),
                    key: gesture_key,
                });
                return;
            }
            if stale || !can_pick(s) {
                // The slider was rebuilt for another thread, model or option mid-gesture.
                // The release or move still clears the stale hold but commits nothing.
                if drag.phase == rune::Phase::Up {
                    s.thinking_drag = None;
                }
                return;
            }
            if s.thinking_drag.is_none() {
                return;
            }
            let along = along_at(drag.x, drag.data);
            let index = (along * (dragged.len() - 1) as f32).round() as usize;
            if drag.phase == rune::Phase::Up {
                // Release glides onto the nearest level.
                tx.motion_curve(MOTION_SLOW);
                s.thinking_drag = None;
                if effective_at(s, &dragged) != Some(index) {
                    s.set_composer_option_sticky(
                        oid_drag.clone(),
                        crate::model_selection::OptionValue::Str(dragged[index].clone()),
                    );
                }
            } else {
                tx.motion_curve(slider_curve(true));
                s.thinking_drag = Some(crate::model::ThinkingDrag {
                    frac: along,
                    key: gesture_key,
                });
            }
        });
    }
    // Codex ends its track with the first and last level names, so the endpoints read as
    // named stops rather than bare caps. A single level gets one label.
    let mut labels = row().key("effort-ends").w_full().child(
        label(
            effort_name(&levels[0]),
            TextStyle::new(12.0, 500.0),
            p.muted,
        )
        .key("effort-end-low"),
    );
    if count > 1 {
        labels = labels.child(spacer()).child(
            label(
                effort_name(&levels[count - 1]),
                TextStyle::new(12.0, 500.0),
                p.muted,
            )
            .key("effort-end-high"),
        );
    }
    section.child(slider).child(labels)
}
