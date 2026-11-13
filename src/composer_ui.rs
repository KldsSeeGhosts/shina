//! Composer layout and focus policy, ported from KSG Code's composerFooterLayout,
//! composerFocusRedirect, composerEventScope and useFocusComposerOnThreadOpen.
use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ControlsLayout {
    pub hidden: usize,
    pub icons: usize,
    pub visible: bool,
}

pub(crate) struct Behavior {
    pub layout: Cell<ControlsLayout>,
    pub viewport: Cell<f32>,
    pub(super) measurement: Cell<Option<[f32; 7]>>,
    pub overflow: Cell<bool>,
    pub menu_expanded: bool,
    pub visible_blocks: Cell<usize>,
    pub show_access: Cell<bool>,
    paste_as_text_until: Option<std::time::Instant>,
    clock: Cell<f64>,
    natural_height: Cell<Option<f32>>,
    transition: Cell<RestingTransition>,
    expanded_width: Cell<f32>,
    wrapped_prompt: Cell<bool>,
    pending_focus: Cell<Option<FocusReason>>,
    pub scope_focused: Cell<bool>,
    /// KSG's forceExpandedOnMobile override for a host that requires the full editor.
    pub force_expanded: bool,
    permission_preview: Cell<Option<crate::model::AccessMode>>,
    opened_thread: Option<(String, usize)>,
    anchors: Cell<[AnchorMotion; 4]>,
    drawn_anchors: [rune::Live<[f32; 4]>; 4],
    prompt_height: Cell<f32>,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            layout: Cell::new(ControlsLayout {
                visible: true,
                ..Default::default()
            }),
            viewport: Cell::new(1280.0),
            measurement: Cell::new(None),
            overflow: Cell::new(false),
            menu_expanded: false,
            visible_blocks: Cell::new(3),
            show_access: Cell::new(true),
            paste_as_text_until: None,
            clock: Cell::new(0.0),
            natural_height: Cell::new(None),
            transition: Cell::new(RestingTransition::default()),
            expanded_width: Cell::new(700.0),
            wrapped_prompt: Cell::new(false),
            pending_focus: Cell::new(None),
            scope_focused: Cell::new(false),
            force_expanded: false,
            permission_preview: Cell::new(None),
            opened_thread: None,
            anchors: Cell::new([AnchorMotion::default(); 4]),
            drawn_anchors: std::array::from_fn(|_| rune::Live::new([0.0; 4])),
            prompt_height: Cell::new(COMPOSER_FIELD_MIN),
        }
    }
}

/// Read the unconstrained controls, including controls currently in overflow. Cx tracks these
/// dependencies, so resizing, a new model, a new captain or font metrics reruns the resolver.
pub(super) fn measure(s: &State, cx: &Cx<State>) {
    s.composer_ui
        .natural_height
        .set(cx.layout_with("composer-natural", |r| r[3]));
    if let Some(height) = cx.layout_with("measure-composer-prompt", |r| r[3]) {
        s.composer_ui.wrapped_prompt.set(height > 23.0 + 8.0 + 1.0);
        s.composer_ui
            .prompt_height
            .set(height.clamp(COMPOSER_FIELD_MIN, COMPOSER_FIELD_MAX));
    }
    if let Some(width) = cx.layout_with("workspace-root", |r| r[2]) {
        s.composer_ui.viewport.set(width);
    }
    let keys = [
        "measure-model",
        "measure-machine",
        "measure-captain",
        "measure-access",
        "measure-machine-icon",
        "measure-captain-icon",
        "measure-access-icon",
    ];
    let mut widths = [0.0; 7];
    let mut complete = true;
    for (index, key) in keys.iter().enumerate() {
        if let Some(width) = cx.layout_with(*key, |r| r[2]) {
            widths[index] = width;
        } else {
            complete = false;
        }
    }
    if complete {
        s.composer_ui.measurement.set(Some(widths));
    }
    let is_collapsed = collapsed(
        s,
        s.composer_ui.viewport.get(),
        s.composer_ui.expanded_width.get(),
    );
    if let Some(height) = s.composer_ui.natural_height.get() {
        let now = s.composer_ui.clock.get();
        let duration = if cx.is_reduced_motion() {
            0
        } else {
            s.preferences.panel_animation_duration_ms.min(400)
        };
        let mut transition = s.composer_ui.transition.get();
        transition.retarget(now, height, is_collapsed, duration);
        s.composer_ui.transition.set(transition);
        let frame_height = cx.layout_with("composer", |r| r[3]).unwrap_or(height);
        let mut anchors = s.composer_ui.anchors.get();
        for (index, key) in [
            "composer-transition-prompt",
            "composer-transition-actions",
            "composer-footer",
            "model-effort",
        ]
        .iter()
        .enumerate()
        {
            if let Some(mut rect) = cx.window_layout(key) {
                // Read destination coordinates, independently of the animated card's height.
                if index != 2 || is_collapsed {
                    rect[1] += frame_height - height;
                }
                let drawn = s.composer_ui.drawn_anchors[index].get().map(|v| v / s.zoom);
                anchors[index].observe(now, rect, transition, drawn);
            }
        }
        s.composer_ui.anchors.set(anchors);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct AnchorMotion {
    from: [f32; 2],
    target: [f32; 2],
    from_height: f32,
    target_height: f32,
    timing: RestingTransition,
    initialized: bool,
}

impl AnchorMotion {
    fn position(self, now: f64) -> [f32; 2] {
        let progress = self.timing.progress(now);
        std::array::from_fn(|i| self.from[i] + (self.target[i] - self.from[i]) * progress)
    }

    fn observe(
        &mut self,
        now: f64,
        rect: [f32; 4],
        transition: RestingTransition,
        drawn: [f32; 4],
    ) {
        let target = [rect[0], rect[1]];
        if transition.duration == 0.0 {
            *self = Self {
                from: target,
                target,
                from_height: rect[3],
                target_height: rect[3],
                timing: transition,
                initialized: true,
            };
            return;
        }
        if self.initialized
            && (0..2).all(|i| (target[i] - self.target[i]).abs() < 0.5)
            && (rect[3] - self.target_height).abs() < 0.5
        {
            return;
        }
        let remaining = (transition.start + transition.duration - now).max(0.0);
        let animate = self.initialized && remaining > 0.0;
        self.from = if animate {
            if self.timing.duration > 0.0 && now < self.timing.start + self.timing.duration {
                self.position(now)
            } else if drawn[2] > 0.0 && drawn[3] > 0.0 {
                [drawn[0], drawn[1]]
            } else {
                self.position(now)
            }
        } else {
            target
        };
        self.from_height = if animate { self.target_height } else { rect[3] };
        self.target = target;
        self.target_height = rect[3];
        self.timing = RestingTransition {
            start: now,
            duration: if animate { remaining } else { 0.0 },
            ..transition
        };
        self.initialized = true;
    }
}

/// KSG's resting transition: cubic-bezier(.32,.72,0,1), optional 0..400 ms duration,
/// 50 ms cleanup buffer. A retarget starts at the displayed height, not the old destination.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RestingTransition {
    pub from: f32,
    pub target: f32,
    pub start: f64,
    pub duration: f64,
    pub collapsed: bool,
    pub initialized: bool,
}

impl RestingTransition {
    fn progress(self, now: f64) -> f32 {
        if self.duration == 0.0 {
            return 1.0;
        }
        let x = ((now - self.start) / self.duration).clamp(0.0, 1.0);
        if x == 0.0 || x == 1.0 {
            return x as f32;
        }
        // Invert the cubic's x coordinate, then evaluate y. Bisection also handles x2 = 0.
        let cubic = |t: f64, a: f64, b: f64| {
            3.0 * (1.0 - t).powi(2) * t * a + 3.0 * (1.0 - t) * t * t * b + t.powi(3)
        };
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..24 {
            let t = (low + high) / 2.0;
            if cubic(t, 0.32, 0.0) < x {
                low = t;
            } else {
                high = t;
            }
        }
        cubic((low + high) / 2.0, 0.72, 1.0) as f32
    }

    pub fn height(self, now: f64) -> f32 {
        self.from + (self.target - self.from) * self.progress(now)
    }

    pub fn retarget(&mut self, now: f64, height: f32, collapsed: bool, duration_ms: u32) {
        if duration_ms == 0 {
            *self = Self {
                from: height,
                target: height,
                start: now,
                duration: 0.0,
                collapsed,
                initialized: true,
            };
            return;
        }
        if (height - self.target).abs() < 0.5 && self.initialized {
            return;
        }
        let interrupted = self.duration > 0.0 && now < self.start + self.duration;
        let animate = self.initialized && (collapsed != self.collapsed || interrupted);
        self.from = if animate { self.height(now) } else { height };
        self.target = height;
        self.start = now;
        self.duration = if animate {
            duration_ms.min(400) as f64 / 1000.0
        } else {
            0.0
        };
        self.collapsed = collapsed;
        self.initialized = true;
    }
}

pub(crate) fn tick(s: &mut State, now: f64, tx: &mut Tx) -> bool {
    s.composer_ui.clock.set(now);
    let restore = s.composer_ui.pending_focus.take();
    if let Some(reason) = restore {
        s.restore_composer_focus(reason);
    }
    let transition = s.composer_ui.transition.get();
    let active = transition.duration > 0.0 && now < transition.start + transition.duration + 0.050;
    if active {
        tx.wake_after(std::time::Duration::from_millis(16));
    }
    active || restore.is_some()
}

/// Keep the expanded form mounted while Tab moves among its controls. Rune routes a field's
/// input key before focus traversal, and a control's ancestor key before its own traversal.
pub(super) fn control_key(s: &mut State, event: &rune::KeyEvent, tx: &mut Tx) -> bool {
    if !s.current().draft.focused && !s.composer_ui.scope_focused.get() {
        return false;
    }
    if event.key == rune::Key::Tab && !event.mods.cmd && !event.mods.ctrl && !event.mods.alt {
        let last = if s.demo {
            "leave-example"
        } else if s.composer_ui.layout.get().hidden > 0 {
            "composer-overflow"
        } else {
            "access-mode"
        };
        let leaving = if event.mods.shift {
            if let Some(image) = super::tray_attachments(s.current()).first() {
                tx.focused() == Some(format!("remove-attachment-{}", image.id).as_str())
            } else {
                tx.focused() == Some("composer-input")
            }
        } else {
            tx.focused() == Some(last)
        };
        s.composer_ui.scope_focused.set(!leaving);
    }
    false
}

pub(super) fn frame_card(s: &State, p: Palette, card: El) -> El {
    let now = s.composer_ui.clock.get();
    let transition = s.composer_ui.transition.get();
    let mut frame = col()
        .key("composer")
        .w_full()
        .radius(6.0)
        .fill(p.card)
        .border(2.0, composer_frame(s, p))
        // The numerical transition owns geometry. Rune must not spring each sampled height.
        .instant_changes();
    if !composer_disabled(s) {
        frame = frame.box_shadow(hard_shadow(p));
    }
    let mut content = col().key("composer-clip").w_full().child(card);
    let mut slot = col()
        .key("composer-transition-slot")
        .instant_changes()
        .w_full()
        .justify_end();
    if transition.duration > 0.0 && now < transition.start + transition.duration + 0.050 {
        (s.wake)();
        frame = frame
            .h(transition.height(now))
            .min_h(transition.height(now));
        // Clip only the content, never the frame's hard shadow.
        content = content.h_full().clip_rounded(4.0);
        slot = slot.h(transition.target);
    }
    slot.child(frame.child(content))
}

/// KSG publishes destination overlay geometry once. Land Rune's ancestor growth stacks
/// during this tween, otherwise they add a second spring to the same height change.
pub(super) fn motion_host(s: &State, element: El) -> El {
    let transition = s.composer_ui.transition.get();
    if transition.duration > 0.0
        && s.composer_ui.clock.get() < transition.start + transition.duration + 0.050
    {
        element.instant_changes()
    } else {
        element
    }
}

pub(super) fn anchor_offset(s: &State, index: usize, inside_card: bool) -> [f32; 2] {
    let anchor = s.composer_ui.anchors.get()[index];
    if !anchor.initialized {
        return [0.0; 2];
    }
    let position = anchor.position(s.composer_ui.clock.get());
    [
        position[0] - anchor.target[0],
        position[1] - anchor.target[1] + if inside_card { footer_offset(s) } else { 0.0 },
    ]
}

pub(super) fn bind_anchor(s: &State, index: usize, element: El) -> El {
    element.drawn_rect_into(&s.composer_ui.drawn_anchors[index])
}

#[cfg(test)]
pub(super) fn motion_debug(s: &State) -> String {
    format!(
        "clock {}, height {:?}, action {:?}, drawn {:?}",
        s.composer_ui.clock.get(),
        s.composer_ui.transition.get(),
        s.composer_ui.anchors.get()[1],
        s.composer_ui.drawn_anchors[1].get()
    )
}

pub(super) fn prompt_surface(s: &State, content: El, collapsed: bool) -> El {
    let [x, y] = anchor_offset(s, 0, true);
    let mut surface = col()
        .key("composer-transition-prompt")
        .w_full()
        .offset(x, y);
    let clip = col().key("composer-prompt-clip").w_full();
    let anchor = s.composer_ui.anchors.get()[0];
    if !collapsed
        && anchor.timing.duration > 0.0
        && anchor.target_height > anchor.from_height + 0.5
        && s.composer_ui.clock.get() < anchor.timing.start + anchor.timing.duration
    {
        let height = s.composer_ui.prompt_height.get();
        let hidden = (anchor.target_height - anchor.from_height)
            * (1.0 - anchor.timing.progress(s.composer_ui.clock.get()));
        surface = surface.h(height).child(
            clip.h((height - hidden).max(0.0))
                .clip_rounded(0.0)
                .child(content.h(height)),
        );
    } else {
        surface = surface.child(clip.child(content));
    }
    bind_anchor(s, 0, surface)
}

pub(super) fn footer_offset(s: &State) -> f32 {
    let transition = s.composer_ui.transition.get();
    if transition.duration > 0.0 {
        transition.height(s.composer_ui.clock.get()) - transition.target
    } else {
        0.0
    }
}

pub(super) fn image_arrival(s: &State, collapsed: bool) -> f32 {
    let mut transition = s.composer_ui.transition.get();
    if transition.duration == 0.0 || transition.collapsed != collapsed {
        return 1.0;
    }
    if !collapsed {
        transition.start += transition.duration / 2.0;
        transition.duration /= 2.0;
    }
    transition.progress(s.composer_ui.clock.get())
}

pub(super) fn measurement_probes(s: &State, p: Palette) -> El {
    col()
        .key("composer-measurements")
        .absolute(0.0, 0.0)
        .w(10000.0)
        .inert()
        .opacity(0.0)
        .a11y_hidden()
        .child(row().child(model_effort_trigger(s, p, true, false).key("measure-model")))
        .child(row().child(machine_selector(s, p, false).key("measure-machine")))
        .child(row().child(captain_selector_trigger(s, p, false).key("measure-captain")))
        .child(row().child(pixel_button(
            "measure-access",
            s.current().native.access.label(),
            p,
            |_| {},
        )))
        .child(row().child(machine_selector(s, p, true).key("measure-machine-icon")))
        .child(row().child(captain_selector_trigger(s, p, true).key("measure-captain-icon")))
        .child(row().child(
            icon_button("measure-access-icon", "Permissions", "shield", p, |_| {}).size(28.0, 24.0),
        ))
        .child(
            text_input(
                &s.current().draft,
                COMPOSER.family(rune::Family::Named("Inter Variable")),
            )
            .key("measure-composer-prompt")
            .w(s.composer_ui.expanded_width.get().max(1.0) + 8.0)
            .padding(4.0)
            .pill_colors(p.selected, Some(p.ink))
            .pill_icon(composer_menu::pill_icon)
            .inert(),
        )
}

/// KSG's trailing blocks first lose labels, then enter overflow, last block first.
/// Natural widths never depend on what the last layout hid. Promotion has 1 px slack.
pub(crate) fn resolve(
    host: f32,
    fixed: f32,
    minimum: f32,
    blocks: &[f32],
    icons: &[f32],
    overflow: f32,
    gap: f32,
    previous: ControlsLayout,
) -> ControlsLayout {
    let n = blocks.len();
    let width = |step: usize, fixed: f32| {
        let hidden = step.saturating_sub(n);
        let icon_count = step.min(n);
        fixed
            + (0..n - hidden)
                .map(|i| {
                    if i >= n - icon_count {
                        icons[i]
                    } else {
                        blocks[i]
                    }
                })
                .sum::<f32>()
            + if hidden > 0 { overflow } else { 0.0 }
            + gap * (n - hidden + usize::from(hidden > 0)) as f32
    };
    let previous_step = if previous.hidden > 0 {
        n + previous.hidden.min(n)
    } else {
        previous.icons.min(n)
    };
    let mut step = 0;
    while step < 2 * n && width(step, fixed) > host - if step < previous_step { 1.0 } else { 0.0 } {
        step += 1;
    }
    ControlsLayout {
        hidden: step.saturating_sub(n),
        icons: step.min(n),
        visible: width(step, minimum) <= host - if previous.visible { 0.0 } else { 1.0 },
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum FocusReason {
    ThreadOpened,
    AttachmentInserted,
    MenuClosed,
    Inserted,
    Explicit,
}

impl State {
    pub(crate) fn composer_thread_opened(&mut self) {
        let key = (self.machines.selected.clone(), self.current().id);
        if self.composer_ui.opened_thread.as_ref() == Some(&key) {
            return;
        }
        self.composer_ui.opened_thread = Some(key);
        self.composer_ui.overflow.set(false);
        self.composer_ui.scope_focused.set(false);
        self.composer_ui.menu_expanded = false;
        self.composer_ui
            .transition
            .set(RestingTransition::default());
        self.composer_ui.anchors.set([AnchorMotion::default(); 4]);
        self.composer_ui.pending_focus.set(None);
        self.restore_composer_focus(FocusReason::ThreadOpened);
    }

    /// One restoration policy. A model picker keeps its own focus through selections.
    /// Thread-open restoration must also leave a simultaneous rename field alone.
    pub(crate) fn restore_composer_focus(&mut self, reason: FocusReason) {
        if self.headless
            || self.settings_picker.is_some()
            || self.captain_picker
            || self.project_picker.is_some()
            || self.rename_thread.is_some()
            || self.captain_editor.is_some()
            || self.settings_menu
            || self.current().native.access_picker
            || self.current().request.is_some()
            || self.current().native.questionnaire.is_some()
            || self.current().native.conversation.is_some()
            || self.machines.dialog
            || self.projects.clone_open
            || self.fleet.open
            || self.settings.archive.is_some()
            || self.brief.profile_open
            || self.brief.confirm_reset
        {
            return;
        }
        let edit = &mut self.current_mut().draft;
        // KSG's menu and thread restorations use focusAtEnd. Insertion owns its selection.
        if matches!(reason, FocusReason::ThreadOpened | FocusReason::MenuClosed) {
            edit.anchor = edit.text.len();
            edit.focus = edit.text.len();
        }
        edit.focused = true;
        self.composer_ui.permission_preview.set(None);
    }

    pub(crate) fn begin_composer_menu(&mut self) {
        self.composer_ui.menu_expanded =
            self.current().draft.focused || self.composer_ui.scope_focused.get();
        self.composer_ui.overflow.set(false);
        self.composer_ui.scope_focused.set(false);
    }
}

/// Rune delivers keys to an editable field before the window handler. Accept only an
/// unowned event or the timeline's non-interactive scroll region, never a control or field.
/// Popup checks also cover an unfocused popup. There is no embedded terminal or side chat
/// input in Shina; their separate native windows never deliver keys to this window.
pub(crate) fn redirect_allowed(s: &State, focused: Option<&str>) -> bool {
    focused.is_none_or(|key| key == format!("conversation-{}", s.current().id))
        && !s.thread_find.query.focused
        && !s.current().draft.is_composing()
        && !s.settings_menu
        && s.settings_picker.is_none()
        && !s.captain_picker
        && s.project_picker.is_none()
        && s.captain_editor.is_none()
        && s.rename_thread.is_none()
        && s.delete_thread.is_none()
        && !s.thread_menu
        && s.row_menu.is_none()
        && s.snooze_menu.is_none()
        && !s.custom_snooze_open
        && !s.attention_menu
        && !s.compact_sidebar
        && !s.machines.picker
        && !s.machines.dialog
        && !s.projects.clone_open
        && !s.fleet.open
        && s.settings.archive.is_none()
        && !s.brief.profile_open
        && !s.brief.confirm_reset
        && !s.composer_ui.overflow.get()
        && !s.current().native.access_picker
        && s.current().native.conversation.is_none()
        && s.current().native.questionnaire.is_none()
        && s.current().request.is_none()
        && !s.current().context.popover_open()
        && s.confirm_remove_attachment.is_none()
        && !captain_ui::is_earlier(s)
}

impl Behavior {
    pub(crate) fn consume_paste_as_text(&self) -> bool {
        // Keep a short window after Mod+Shift+V so on_input_paste sees the bypass.
        self.paste_as_text_until
            .is_some_and(|until| std::time::Instant::now() <= until)
    }
}

pub(crate) fn key(s: &mut State, event: &rune::KeyEvent, tx: &mut Tx) -> bool {
    if event.is_command('v') && event.mods.shift && !event.mods.alt {
        s.composer_ui.paste_as_text_until =
            Some(std::time::Instant::now() + std::time::Duration::from_millis(1000));
        return false;
    }
    if event.mods.cmd || event.mods.ctrl || event.mods.alt || !redirect_allowed(s, tx.focused()) {
        return false;
    }
    let rune::Key::Text(value) = &event.key else {
        return false;
    };
    if value.encode_utf16().count() != 1 || value.chars().any(char::is_control) {
        return false;
    }
    s.restore_composer_focus(FocusReason::Inserted);
    let t = s.current_mut();
    t.draft.anchor = t.draft.text.len();
    t.draft.focus = t.draft.text.len();
    t.draft.insert(value);
    changed(s);
    s.checkpoint();
    true
}

pub(super) fn changed(s: &mut State) {
    composer_menu::clear_dismissed_if_changed(s);
    let t = s.current_mut();
    t.native.slash_index = 0;
    t.composer_rules.changed(&mut t.draft);
    t.reconcile_draft_context();
    t.typing_guard.on_type(std::time::Instant::now());
    t.update_submission_error();
    s.ensure_composer_commands();
}

pub(crate) fn paste(s: &mut State, content: &rune::ClipboardContent, tx: &mut Tx) {
    if !redirect_allowed(s, tx.focused())
        || content.text().is_empty() && content.files().is_empty() && content.image().is_none()
    {
        return;
    }
    s.restore_composer_focus(FocusReason::Inserted);
    let edit = &mut s.current_mut().draft;
    edit.anchor = edit.text.len();
    edit.focus = edit.text.len();
    let bypass = s
        .composer_ui
        .paste_as_text_until
        .take()
        .is_some_and(|until| std::time::Instant::now() <= until);
    s.paste_composer_content(content, bypass);
    changed(s);
    s.checkpoint();
}

pub(super) fn collapsed(s: &State, width: f32, expanded_width: f32) -> bool {
    s.composer_ui.expanded_width.set(expanded_width);
    let t = s.current();
    let style = COMPOSER.family(rune::Family::Named("Inter Variable"));
    // Measure at the expanded width, not beside resting actions, to avoid oscillation.
    let text = t.draft.display().0;
    let (natural, _) = rune::text::measure(&text, &[], style, None);
    width < 640.0
        && !s.composer_ui.force_expanded
        && !t.draft.focused
        && !s.composer_ui.scope_focused.get()
        && !(s.composer_ui.menu_expanded
            && (s.settings_picker.is_some()
                || s.captain_picker
                || s.machines.picker
                || s.current().native.access_picker
                || s.composer_ui.overflow.get()))
        && !text.contains('\n')
        && natural <= expanded_width + 1.0
        && !s.composer_ui.wrapped_prompt.get()
        && t.request.is_none()
        && t.native.questionnaire.is_none()
}

/// Natural widths use Rune's font metrics and the controls' actual padding, gaps and icons.
/// This is the native equivalent of restingComposerControlsMeasurement's scrollWidth reads.
pub(super) fn controls(s: &State, p: Palette, width: f32, resting: bool, locked: bool) -> El {
    if s.demo {
        return row()
            .key("composer-footer")
            .h(32.0)
            .px(10.0)
            .center_y()
            .child(pixel_text("Demo data", p.muted))
            .child(spacer())
            .child(pixel_button(
                "leave-example",
                "Open a real workspace",
                p,
                |s| s.leave_demo(),
            ));
    }
    let t = s.current();
    let text_width = |value: &str, style| rune::text::measure(value, &[], style, None).0;
    let machine = text_width(&s.machine_label(&s.machines.selected), SMALL) + 50.0;
    let captain = t.captain.and_then(|id| s.captains.get(id));
    let captain_width = text_width(captain.map_or("No captain", |c| c.name.as_str()), SMALL)
        + if captain.is_some() { 52.0 } else { 50.0 };
    let display_access = s.effective_access_mode(t);
    let access = text_width(display_access.label(), PIXEL) + 20.0;
    let (name, _) = split_model_label(&t.model);
    let fixed = if resting {
        (text_width(name, TextStyle::new(13.0, 600.0))
            + 37.0
            + if name == crate::model::DEFAULT_MODEL && t.thinking.is_empty() {
                0.0
            } else {
                7.0 + text_width(&effort_name(&t.thinking), PIXEL)
            })
        .min(200.0)
    } else {
        0.0
    };
    let measurement = s.composer_ui.measurement.get();
    let show_access = !resting || display_access.id() != s.thread_defaults.access;
    s.composer_ui.show_access.set(show_access);
    let mut blocks = measurement.map_or([machine, captain_width, access], |m| [m[1], m[2], m[3]]);
    let icon_widths = measurement.map_or(
        [38.0, if captain.is_some() { 40.0 } else { 38.0 }, 28.0],
        |m| [m[4], m[5], m[6]],
    );
    // KSG uses an icon, never a label, for a non-default access chip in the resting row.
    if resting {
        blocks[2] = icon_widths[2];
    }
    let count = if show_access { 3 } else { 2 };
    let fixed = if resting {
        measurement.map_or(fixed, |m| m[0])
    } else {
        0.0
    };
    let layout = resolve(
        width,
        fixed,
        if resting { 52.0 } else { 0.0 },
        &blocks[..count],
        &icon_widths[..count],
        28.0,
        4.0,
        s.composer_ui.layout.get(),
    );
    s.composer_ui.layout.set(layout);
    if layout.hidden == 0 || !layout.visible {
        if s.composer_ui.overflow.replace(false) {
            s.composer_ui
                .pending_focus
                .set(Some(FocusReason::MenuClosed));
            (s.wake)();
        }
    }
    let visible = count - layout.hidden;
    s.composer_ui.visible_blocks.set(visible);
    let icon_only = |index| index >= count - layout.icons;
    let [controls_x, controls_y] = anchor_offset(s, 2, resting);
    let mut controls = row()
        .key("composer-footer")
        .instant_changes()
        .on_key(control_key)
        .keeps_text_selection()
        .w_full()
        .center_y()
        .gap(4.0)
        .min_w(0.0)
        .h(32.0)
        .px(if resting { 16.0 } else { 10.0 });
    controls = bind_anchor(s, 2, controls.offset(controls_x, controls_y));
    if resting {
        let [model_x, model_y] = anchor_offset(s, 3, true);
        controls = controls.child(bind_anchor(
            s,
            3,
            model_effort_trigger(s, p, true, locked)
                .min_w(52.0)
                .offset(model_x - controls_x, model_y - controls_y),
        ));
    }
    if visible > 0 {
        controls = controls.child(machine_selector(s, p, icon_only(0)));
    }
    if visible > 1 {
        controls = controls.child(captain_selector_trigger(s, p, icon_only(1)));
    }
    if !resting {
        controls = controls.child(spacer());
    }
    if visible > 2 {
        let mut access_control = if resting || icon_only(2) {
            icon_button("access-mode", display_access.label(), "shield", p, |s| {
                s.begin_composer_menu();
                s.open_access_picker();
            })
            .size(28.0, 24.0)
        } else {
            pixel_button("access-mode", display_access.label(), p, |s| {
                s.begin_composer_menu();
                s.open_access_picker();
            })
        };
        access_control = access_control
            .preserve_focus_on_press(true)
            .disabled(t.status.busy() || t.configuring || t.archived());
        controls = controls.child(access_control);
    }
    if layout.hidden > 0 {
        controls = controls.child(
            icon_button(
                "composer-overflow",
                "More composer controls",
                "more",
                p,
                |s| {
                    s.begin_composer_menu();
                    s.reconcile_current_access_mode();
                    s.composer_ui.overflow.set(true);
                },
            )
            .size(28.0, 24.0)
            .preserve_focus_on_press(true),
        );
    }
    if resting && !layout.visible {
        controls = controls.opacity(0.0).inert();
    }
    controls
}

pub(super) fn overflow_menu(s: &State, p: Palette) -> El {
    let visible = s.composer_ui.visible_blocks.get();
    let mut menu = col()
        .key("composer-overflow-menu")
        .role("menu")
        .focus_group()
        .w(288.0)
        .padding(4.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p));
    // Machine, captain, permissions. Hiding is from the end of this order.
    if visible == 0 {
        menu = menu.child(
            menu_item("overflow-machine", "Run on", "monitor", p)
                .disabled(s.client.is_none() || !s.can_retarget_draft())
                .on_press(|s: &mut State| {
                    s.composer_ui.overflow.set(false);
                    s.machines.picker = true;
                }),
        );
    }
    if visible <= 1 {
        menu = menu.child(
            menu_item("overflow-captain", "Captain", "user", p)
                .disabled(
                    s.current().status.busy()
                        || s.current().configuring
                        || s.current().archived()
                        || !s.current().is_draft(),
                )
                .on_press(|s: &mut State| {
                    s.composer_ui.overflow.set(false);
                    s.open_captain_selector();
                }),
        );
    }
    if s.composer_ui.show_access.get() {
        menu = menu.child(permissions_section(s, p));
    }
    menu.anchored_fit(
        "composer-overflow",
        rune::Side::Top,
        rune::Align::Start,
        8.0,
    )
    .focus_return("composer-input")
    .modal(true, |s: &mut State| {
        s.composer_ui.overflow.set(false);
        s.restore_composer_focus(FocusReason::MenuClosed);
    })
}

/// KSG's ComposerPermissionsSection. Read only is Shina's additional access option.
pub(super) fn permissions_section(s: &State, p: Palette) -> El {
    let t = s.current();
    let supported = crate::model::supported_access_modes(&t.harness);
    let (modes, current_mode) =
        crate::model::resolve_compatible_access_mode(t.native.access, supported);
    let shown = crate::model::resolve_compatible_access_mode(
        s.composer_ui
            .permission_preview
            .get()
            .unwrap_or(current_mode),
        supported,
    )
    .1;
    let mut menu = col()
        .key("composer-permissions")
        .on_key(|s: &mut State, event, tx| {
            let direction = match event.key {
                rune::Key::Left => -1,
                rune::Key::Right => 1,
                _ => return false,
            };
            let modes = crate::model::resolve_compatible_access_mode(
                s.current().native.access,
                crate::model::supported_access_modes(&s.current().harness),
            )
            .0;
            let Some(at) = modes.iter().position(|mode| {
                tx.focused() == Some(format!("overflow-access-{}", mode.id()).as_str())
            }) else {
                return false;
            };
            let next = at as i32 + direction;
            if !(0..modes.len() as i32).contains(&next) {
                return false;
            }
            let mode = modes[next as usize];
            s.composer_ui.permission_preview.set(Some(mode));
            tx.focus(format!("overflow-access-{}", mode.id()), true);
            true
        })
        .px(8.0)
        .py(6.0)
        .gap(6.0)
        .child(pixel_label("Permissions", p.muted));
    let mut options = row().gap(2.0).padding(2.0).radius(4.0).fill(p.selected);
    let mut description = "";
    for mode in modes {
        let text = mode.description();
        if mode == shown {
            description = text;
        }
        options = options.child(
            el().key(format!("overflow-access-{}", mode.id()))
                .role("menuitemradio")
                .checked(mode == current_mode)
                .label(mode.label())
                .help(text)
                .h(24.0)
                .grow(1.0)
                .center()
                .radius(4.0)
                .fill(if mode == current_mode {
                    p.card
                } else {
                    Color::TRANSPARENT
                })
                .hover(p.selected)
                .focus_fill(p.selected)
                .focus_selected(mode == current_mode)
                .on_hover_change(move |s: &mut State, inside, _| {
                    s.composer_ui.permission_preview.set(inside.then_some(mode));
                })
                .child(
                    icon(
                        if mode == current_mode {
                            "check"
                        } else {
                            "shield"
                        },
                        p.muted,
                    )
                    .size(14.0, 14.0),
                )
                .disabled(
                    s.current().status.busy() || s.current().configuring || s.current().archived(),
                )
                .on_press(move |s: &mut State| {
                    s.composer_ui.overflow.set(false);
                    s.settings_picker = None;
                    s.current_mut().native.access_picker = false;
                    s.set_access_mode(mode);
                    s.restore_composer_focus(FocusReason::MenuClosed);
                }),
        );
    }
    menu = menu
        .child(options)
        .child(label(shown.label(), SMALL, p.ink))
        .child(label(description, SMALL, p.muted).wrap().min_h(32.0));
    menu
}
