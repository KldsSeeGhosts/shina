use crate::attachments::Attachment;
use crate::context::{ContextMeter, ContextPhase};
use crate::lifecycle::{self, Section};
use crate::model::{
    ProjectPicker, ProjectScope, RowTarget, SettingsPicker, State, Status, Thread, Tool, truncate,
};
use rune::{
    Color, Cx, El, Icon, LineCap, ScrollSpec, Shape, TextStyle, Tx, canvas, col, el, memo, row,
    text,
};
use rune_kit::{Markdown, MarkdownStyle};
use serde_json::json;
#[path = "captain_ui.rs"]
mod captain_ui;
#[path = "crew_ui.rs"]
mod crew_ui;
#[path = "markdown_ui.rs"]
mod markdown_ui;
#[path = "pi_ui.rs"]
mod pi_ui;
#[path = "pixel_icons.rs"]
mod pixel_icons;
#[path = "sidebar_ui.rs"]
mod sidebar_ui;
pub(crate) use pi_ui::questionnaire_key;

/// Keyboard navigation uses the same server-scoped order as the unified sidebar.
pub(crate) fn sidebar_navigation(s: &State) -> Vec<(String, usize)> {
    if s.client.is_none() {
        return s
            .sidebar_navigation()
            .into_iter()
            .map(|index| (s.machines.selected.clone(), s.threads[index].id))
            .collect();
    }
    let mut rows = Vec::new();
    let mut settled = 0;
    for entry in sidebar_ui::entries(s) {
        if !entry.captain && !entry.visible {
            continue;
        }
        if !entry.captain
            && lifecycle::section(&entry.life, lifecycle::now()) == Section::Settled
            && s.search.text.is_empty()
        {
            settled += 1;
            if settled > s.settled_visible {
                continue;
            }
        }
        rows.push((entry.server.to_owned(), entry.id));
        if entry.captain && entry.local.is_some() {
            rows.extend(
                s.visible_captain_workers(entry.id)
                    .into_iter()
                    .map(|index| (entry.server.to_owned(), s.threads[index].id)),
            );
        }
    }
    rows
}

// The two scenes of the icon (night sky, day sail) with a coral hull for the accent, set out
// in docs/ui/pixel-voyage.md. Coral is the primary accent, brass marks anything waiting on the
// user and sea marks running work and links.
#[derive(Clone, Copy)]
struct Palette {
    bg: Color,
    panel: Color,
    card: Color,
    line: Color,
    ink: Color,
    muted: Color,
    /// A quieter text colour for tool details, paths and machine names. Opaque, and 4.5:1 or
    /// better on every surface text sits on, so it is never `muted` with an alpha.
    faint: Color,
    /// Hover tint over any surface.
    selected: Color,
    /// Fill of the selected sidebar row.
    row: Color,
    coral: Color,
    coral_bg: Color,
    on_coral: Color,
    brass: Color,
    on_brass: Color,
    sea: Color,
    sea_bg: Color,
    danger: Color,
    /// The 2 px pixel border, and the hard shadow it casts down and to the right.
    frame: Color,
    /// The outline of portraits and project tiles. In the dark theme it stays the darker navy,
    /// so a face reads apart from the lighter frames of buttons, tabs and the composer.
    portrait_frame: Color,
    shadow: Color,
    field_line: Color,
    message_bg: Color,
    code_bg: Color,
}
fn color(hex: &str) -> Color {
    rune::color::hex(hex).expect("palette color")
}
fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: color("#101727"),
            panel: color("#162036"),
            card: color("#1d2a45"),
            line: color("#f4efe3").alpha(0.12),
            ink: color("#f4efe3"),
            // Muted and faint are two steps, 6.3:1 and 4.6:1 on a hovered card, so the quiet text
            // still reads as quieter than the secondary text.
            muted: color("#b8c7e0"),
            faint: color("#98aac5"),
            // Rune blends in linear light, so a small alpha still gives a visible highlight.
            selected: color("#9fb0cc").alpha(0.06),
            row: color("#1d2a45").alpha(0.85),
            coral: color("#e07a54"),
            coral_bg: color("#e07a54").alpha(0.16),
            on_coral: color("#1a0f0a"),
            brass: color("#d3ba6b"),
            on_brass: color("#1a0f0a"),
            sea: color("#7ab4d2"),
            sea_bg: color("#24406a"),
            danger: color("#f0a39c"),
            // The document's #0a0f1c frame vanishes on these surfaces, so night frames are a
            // lighter blue-grey and only the hard shadows stay near-black.
            frame: color("#52688e"),
            portrait_frame: color("#3a4a6c"),
            shadow: color("#060a13"),
            field_line: color("#3d507a"),
            message_bg: color("#24406a"),
            code_bg: color("#0c1220"),
        }
    } else {
        Palette {
            // The day sky of the icon, between its #b9e0ee and white: the window and sidebar
            // are sky, the sea strip is the horizon, and the chat pane is sailcloth.
            bg: color("#d5e8ef"),
            panel: color("#f8f4ea"),
            card: color("#fffdf7"),
            line: color("#1f2538").alpha(0.14),
            ink: color("#1f2538"),
            // The document's #5d6478 is 4.25:1 on a hovered sidebar row; this is 4.7:1.
            muted: color("#565d71"),
            faint: color("#595f73"),
            selected: color("#1f2538").alpha(0.075),
            row: color("#fffdf7").alpha(0.7),
            coral: color("#d2643c"),
            coral_bg: color("#d2643c").alpha(0.12),
            // Cream on #d2643c is 3.7:1. Near-black reaches 5.2:1, so coral fills keep it.
            on_coral: color("#1a0f0a"),
            // The document's #8a6a2f is 3.97:1 as a status word on the sky; the same hue at
            // 91% reaches 4.6:1 there.
            brass: color("#7e602b"),
            on_brass: color("#fffdf7"),
            sea: color("#1f6d62"),
            sea_bg: color("#d6ebe4"),
            danger: color("#a33e38"),
            frame: color("#1f2538"),
            portrait_frame: color("#1f2538"),
            shadow: color("#1f2538").alpha(0.30),
            field_line: color("#b5ac94"),
            message_bg: color("#e0d6ba"),
            code_bg: color("#ebe6d3"),
        }
    }
}
pub const SANS_FAMILY: &str = "Anthropic Sans Variable Text, Inter Variable";
/// The pixel face: section labels, counts, timers, status words, badges and the wordmark. It is
/// drawn for 11 px and its multiples, so use it at 11 or 22 only.
pub const PIXEL_FAMILY: &str = "Departure Mono";
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/InterVariable.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4Variable.ttf"),
    include_bytes!("../assets/fonts/DepartureMono-Regular.otf"),
];
/// A Departure Mono style at 11 px (the grid it is drawn on) or 22 px.
const fn pixel(size: f32) -> TextStyle {
    TextStyle::new(size, 400.0)
        .lh(size + 3.0)
        .family(rune::Family::Named(PIXEL_FAMILY))
}
/// Section labels, counts and status words.
const PIXEL: TextStyle = pixel(11.0);
/// Section labels are uppercase with a little air between the letters.
const PIXEL_LABEL: TextStyle = pixel(11.0).tracking(0.6);
const BODY: TextStyle = TextStyle::new(14.0, 400.0).lh(21.0).tracking(-0.10);
/// Assistant answers use Cursor's agent-window text: 15/24 sans with slightly tight tracking.
const PROSE: TextStyle = TextStyle::new(15.0, 400.0).lh(24.0).tracking(-0.20);
const COMPOSER: TextStyle = TextStyle::new(15.0, 400.0).lh(23.0).tracking(-0.10);
/// The new-thread greeting, ported from KSG Code's `DraftHeroHeadline`: centred sans,
/// `text-3xl font-normal tracking-tight`, or `text-2xl` when the pane is narrow.
/// Shina uses its bundled serif for greetings, the only typographic departure.
const HEADING: TextStyle = TextStyle::new(30.0, 400.0)
    .lh(36.0)
    .tracking(-0.45)
    .family(rune::Family::Named("Source Serif 4"));
/// Dialog titles: Source Serif 4 at 20/600, tight.
const DIALOG_TITLE: TextStyle = TextStyle::new(20.0, 600.0)
    .lh(26.0)
    .tracking(-0.2)
    .family(rune::Family::Named("Source Serif 4"));
/// A captain's name in the picker, the hero and the editor: Source Serif 4, tight.
const CAPTAIN_SERIF: TextStyle = TextStyle::new(16.0, 600.0)
    .lh(22.0)
    .tracking(-0.2)
    .family(rune::Family::Named("Source Serif 4"));
const HEADING_NARROW: TextStyle = TextStyle::new(24.0, 400.0)
    .lh(32.0)
    .tracking(-0.3)
    .family(rune::Family::Named("Source Serif 4"));
/// A chat pane this wide (a 1600 px window with the sidebar open) gets a longer measure and
/// larger body text. It follows the pane, not the window, so an open session panel or a wide
/// sidebar keeps the reading size that fits the lane.
const WIDE_PANE: f32 = 1200.0;
fn body_style(wide: bool) -> TextStyle {
    if wide {
        TextStyle::new(15.0, 400.0).lh(23.0).tracking(-0.10)
    } else {
        BODY
    }
}
fn prose_style(wide: bool) -> TextStyle {
    if wide {
        TextStyle::new(16.0, 400.0).lh(25.0).tracking(-0.20)
    } else {
        PROSE
    }
}
/// The chat lane's content width, ported from KSG Code's `.chat-content-lane`
/// (`max-width: var(--chat-content-max-width)`, 46 rem). Wide panes get 54 rem for the larger
/// text. Messages, tool rows, the queue and the composer all share it.
fn measure(wide: bool) -> f32 {
    if wide { 864.0 } else { 736.0 }
}
/// The lane's side padding, from KSG Code's `.chat-composer-lane` and
/// `.messages-timeline-scroll`: 0.75 rem below `sm` (640 px) and 3 rem from `sm` up. KSG reads
/// the viewport; Shina reads the chat pane, which is what the lane actually has to share.
fn lane_gutter(pane_width: f32) -> f32 {
    if pane_width >= 640.0 { 48.0 } else { 12.0 }
}
/// How wide the lane is in a pane of `pane_width`.
fn lane_width(pane_width: f32, wide: bool) -> f32 {
    (pane_width - 2.0 * lane_gutter(pane_width)).clamp(0.0, measure(wide))
}
/// Live timers and counts use tabular figures, so "12m 05s" never shifts as a digit changes.
const TABULAR: &str = "'tnum' 1";
const TIMER: TextStyle = PIXEL.features(TABULAR);
const LABEL: TextStyle = TextStyle::new(14.0, 430.0).tracking(-0.10);
const SMALL: TextStyle = TextStyle::new(12.0, 420.0);
/// The project line of a sidebar card.
const CARD_META: TextStyle = TextStyle::new(12.5, 430.0);
/// Sidebar rows sit 8 px in from the sidebar's edge, and their content 12 px inside that, so
/// icons and text share one left edge with the title strip and footer.
const ROW_INSET: f32 = 12.0;
/// Tool, thinking and status lines in the transcript.
const ACTIVITY: TextStyle = TextStyle::new(13.0, 420.0).tracking(-0.08);
/// Height of one-line sidebar rows: navigation, workers and shelved threads.
/// Workers sit a step below their captain: shorter rows on the captain's rail.
const CREW_ROW: f32 = 30.0;

/// A text field's focus look, shared by every field: a coral caret, and a border that is 1 px
/// `field_line` at rest and 2 px coral while the field has focus. Put it on the box that draws
/// the field's edge.
trait FieldFocus {
    fn field_focus(self, edit: &rune::TextEdit, p: Palette) -> Self;
}
impl FieldFocus for El {
    fn field_focus(self, edit: &rune::TextEdit, p: Palette) -> Self {
        self.caret(p.coral, p.coral_bg).border(
            if edit.focused { 2.0 } else { 1.0 },
            if edit.focused { p.coral } else { p.field_line },
        )
    }
}
/// The fill of a text field inside a dialog: the panel colour in light, the window colour in dark.
/// It stays the same whether the field has focus or not. Focus shows through `field_focus` only.
fn field_fill(s: &State, p: Palette) -> Color {
    if s.dark { p.bg } else { p.panel }
}
fn text_input(edit: &rune::TextEdit, style: TextStyle) -> El {
    rune::text_input(edit, style).on_input_key(|s: &mut State, event, _| s.undo_key(event, true))
}
fn label(value: impl Into<String>, style: TextStyle, ink: Color) -> El {
    text(value).font(style).color(ink)
}
fn spacer() -> El {
    el().grow(1.0)
}
fn rule(p: Palette) -> El {
    el().h(1.0).w_full().fill(p.line)
}
/// A hard 2 px shadow below and to the right, with no blur anywhere in the app.
fn hard_shadow(p: Palette) -> rune::Shadow {
    rune::Shadow {
        x: 2.0,
        y: 2.0,
        blur: 0.0,
        spread: 0.0,
        color: p.shadow,
        inset: false,
    }
}
/// The pixel frame: a 2 px `frame` border and the hard shadow.
fn framed(el: El, p: Palette) -> El {
    el.border(2.0, p.frame).box_shadow(hard_shadow(p))
}
/// The pixel frame for something that only informs (a keycap, a count, the Demo badge): the
/// outline without the hard shadow. Shadows belong to portraits, buttons you can press, the
/// composer, menus and dialogs.
fn outlined(el: El, p: Palette) -> El {
    el.border(1.0, p.frame)
}
/// An opaque colour from sRGB bytes, without going through a hex string.
fn rgb_bytes([red, green, blue]: [u8; 3]) -> Color {
    let linear = |byte: u8| rune::color::srgb_decode(f32::from(byte) / 255.0);
    Color {
        r: linear(red),
        g: linear(green),
        b: linear(blue),
        a: 1.0,
    }
}
/// A scene from `horizon` as one shape of flat rectangles.
fn scenery(rects: &[crate::horizon::Rect]) -> Shape {
    rects.iter().fold(Shape::new(), |shape, r| {
        shape.fill(
            vec![
                [r.x, r.y],
                [r.x + r.w, r.y],
                [r.x + r.w, r.y + r.h],
                [r.x, r.y + r.h],
            ],
            rgb_bytes(r.color),
        )
    })
}
/// The horizon band over the new-thread screen: `width` by `HORIZON_HEIGHT`, 6 px corners and
/// a pixel frame. The frame is an overlay, so the scenery cannot paint over it.
///
/// `fleet` is the number of working threads or tasks, one distant sail each (at most four).
///
/// The scenery is rebuilt only when the width, the theme, a captain's backdrop or the fleet
/// size changes. Its polygons are the costly part, and nothing else in the view moves them.
fn horizon_band(
    width: f32,
    dark: bool,
    subject: crate::horizon::Subject,
    fleet: usize,
    p: Palette,
) -> El {
    let fleet = fleet.min(crate::horizon::FLEET_MAX);
    let backdrop = match subject {
        crate::horizon::Subject::Boat => (false, [0u8; 3]),
        crate::horizon::Subject::Sky(rgb) => (true, rgb),
    };
    rune::memo(
        "horizon-band",
        (width.to_bits(), dark, backdrop, fleet),
        move || {
            let shape = scenery(&crate::horizon::band(
                width,
                HORIZON_HEIGHT,
                dark,
                subject,
                fleet,
            ));
            // The scenery is clipped in its own layer, because a clipped element also clips its
            // hard shadow.
            el().key("horizon-band")
                .w(width)
                .h(HORIZON_HEIGHT)
                .radius(6.0)
                .box_shadow(hard_shadow(p))
                .child(
                    el().size(width, HORIZON_HEIGHT)
                        .radius(6.0)
                        .clip_rounded(6.0)
                        .shape(shape),
                )
                .child(
                    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .radius(6.0)
                        .border(2.0, p.frame)
                        .a11y_hidden(),
                )
                .a11y_hidden()
        },
    )
}
/// The number of distant sails: the working count the captain header shows ("Working in the
/// background, 2 tasks"), threads running now, so the scenery and the words never disagree.
/// `horizon_band` caps it.
fn fleet_size(s: &State) -> usize {
    if let Some(captain) = captain_ui::brief_captain(s) {
        s.crew_working_now(s.captain_home(captain).unwrap_or(s.current().id))
    } else {
        s.working_now_count()
    }
}
/// Height of the horizon band, 36 cells of 3 pt.
const HORIZON_HEIGHT: f32 = 108.0;
/// Height of the sea strip at the foot of the sidebar, 9 cells of 3 pt.
const SEA_HEIGHT: f32 = 27.0;
/// A portrait in a pixel frame. The image sits under a border overlay, because Rune paints a
/// clipped image over its own border. Below 30 px the frame is 1 px so the face keeps its room.
fn pixel_portrait(s: &State, pixels: &rune::Pixels, size: f32, p: Palette) -> El {
    pixel_portrait_framed(s, pixels, size, p, None)
}
/// `pixel_portrait` with an accent for a selected portrait: the frame and the hard shadow both
/// take that colour. The frame keeps its width and the shadow lies outside the box, so the
/// outer bounds do not change.
fn pixel_portrait_framed(
    s: &State,
    pixels: &rune::Pixels,
    size: f32,
    p: Palette,
    accent: Option<Color>,
) -> El {
    let size = s.artwork.snap_points(size);
    let (radius, frame) = if size >= 30.0 { (4.0, 2.0) } else { (2.0, 1.0) };
    let edge = accent.unwrap_or(p.portrait_frame);
    let mut shadow = hard_shadow(p);
    if let Some(accent) = accent {
        shadow.color = accent;
    }
    el().size(size, size)
        .radius(radius)
        .box_shadow(shadow)
        .child(
            el().size(size, size)
                .radius(radius)
                .clip_rounded(radius)
                .pixels(pixels),
        )
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(radius)
                .border(frame, edge),
        )
}
/// Uppercase Departure Mono, for section labels and status words.
fn pixel_label(value: &str, ink: Color) -> El {
    label(value.to_uppercase(), PIXEL_LABEL, ink)
}
/// Departure Mono at 11 px as written, for counts, ages and timers.
fn pixel_text(value: impl Into<String>, ink: Color) -> El {
    label(value, PIXEL, ink)
}
fn button(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    button_in(key, title, TextStyle::new(12.0, 450.0), p, action)
}
/// A quiet button whose label is Departure Mono, for the rows of state under the composer.
fn pixel_button(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    button_in(key, title, PIXEL, p, action)
}
fn button_in(
    key: &str,
    title: &str,
    style: TextStyle,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .center_y()
        .gap(7.0)
        .px(10.0)
        .h(24.0)
        .radius(4.0)
        .hover(p.selected)
        .press(p.selected)
        .on_press(action)
        .child(label(title, style, p.ink).truncate().shrink())
}
/// Icon size in points. The glyphs sit on a 24 unit grid of 2 unit squares, so 18 pt puts each
/// square on 3 device pixels at 2x and keeps the edges hard. 16 pt lands on 2.67 and blurs.
const ICON: f32 = 18.0;
/// A Pixelarticons glyph (24 grid, 2 px squares).
fn icon(kind: &str, ink: Color) -> El {
    rune::icon(pixel_icons::glyph(kind), ICON)
        .color(ink)
        .a11y_hidden()
}
fn tool_icon(kind: &str, ink: Color) -> El {
    icon(kind, ink).size(ICON, ICON)
}
fn primary_button(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    framed(
        row()
            .key(key)
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(title)
            .focusable(true)
            .center_y()
            .h(32.0)
            .px(14.0)
            .radius(4.0)
            .fill(p.coral)
            // Rune cannot move a button on press, so a pressed key darkens instead.
            .press(p.coral.alpha(0.8))
            .on_press(action)
            .child(label(title, TextStyle::new(13.0, 600.0), p.on_coral)),
        p,
    )
}
/// The secondary action beside a primary one, such as Cancel: the same size and the 2 px frame,
/// with no shadow and no fill, so the pair reads as one set.
fn secondary_button(
    key: &str,
    title: &str,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .center_y()
        .h(32.0)
        .px(14.0)
        .radius(4.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .hover(p.selected)
        .press(p.selected)
        .on_press(action)
        .child(label(title, TextStyle::new(13.0, 600.0), p.ink))
}
fn icon_button(
    key: &str,
    title: &str,
    glyph: &str,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    el().key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .size(24.0, 24.0)
        .center()
        .radius(4.0)
        .hover(p.selected)
        .press(p.selected)
        .on_press(action)
        .child(tool_icon(glyph, p.muted))
}
/// An icon button that shows when its panel or state is on.
fn toggle_button(
    key: &str,
    title: &str,
    glyph: &str,
    on: bool,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    el().key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .size(24.0, 24.0)
        .center()
        .radius(4.0)
        .fill(if on { p.selected } else { Color::TRANSPARENT })
        .hover(p.selected)
        .on_press(action)
        .child(tool_icon(glyph, if on { p.ink } else { p.muted }))
}
/// The status a view should show for the open tab: a captain's home stands for the whole
/// captain, so it shows the most urgent of its threads; any other tab shows itself.
fn shown_status(s: &State) -> Status {
    let t = s.current();
    match t.captain {
        Some(_) if s.is_captain_home(t) => s.home_status(t.id),
        _ => t.status,
    }
}
fn status_ink(status: Status, p: Palette) -> Color {
    match status {
        Status::Failed => p.danger,
        Status::Waiting => p.brass,
        // Working and done both read as sea, so Done never turns grey in one list and blue in
        // the next. Every status word and mark takes its colour from this function.
        Status::Running | Status::Completed => p.sea,
        _ => p.muted,
    }
}
/// One status vocabulary everywhere, in squares: a filled one for working, a hollow one for
/// needs you, a smaller filled one for failed, and a check for settled. Ready and offline get
/// a faint square.
fn status_mark(status: Status, p: Palette, size: f32) -> El {
    let ink = status_ink(status, p);
    match status {
        Status::Running => el().size(size, size).fill(ink),
        Status::Waiting => el().size(size, size).border(2.0, ink),
        Status::Failed => el().size(size - 2.0, size - 2.0).fill(ink),
        Status::Completed => icon("check", ink).size(size + 7.0, size + 7.0),
        _ => el().size(size, size).fill(ink.alpha(0.6)),
    }
    .a11y_hidden()
}
/// A status chip for states that need attention. A finished or idle thread shows nothing:
/// the reply itself says it is done, and background completions use the unread badge.
fn status(status: Status, p: Palette) -> El {
    if matches!(status, Status::Ready | Status::Completed) {
        return el();
    }
    row()
        .center_y()
        .gap(6.0)
        .child(status_mark(status, p, 6.0))
        .child(pixel_text(status.label(), status_ink(status, p)))
}
/// Room for "Working for 59m 59s" in the chat header, so live status never reflows the row
/// around it.
const HEADER_STATUS_SLOT: f32 = 156.0;
/// A live status in a slot of fixed width. The text sits at the slot's end in tabular
/// figures, never shrinks or truncates, and fades between values on Shina's motion curve, so
/// "Ready" becoming "Working for 12m 05s" moves nothing beside it. Ready and Done show only
/// the empty slot, as `status` shows nothing for them.
fn status_slot(key: &str, status: Status, text: Option<String>, width: f32, p: Palette) -> El {
    let mut slot = row()
        .key(key)
        .w(width)
        .min_w(width)
        .h(20.0)
        .justify_end()
        .center_y();
    let text = match text {
        Some(text) => text,
        None if matches!(status, Status::Ready | Status::Completed) => return slot,
        None => status.label().into(),
    };
    slot = slot.label(text.clone()).child(
        row()
            .key(format!("{key}-{}", status.label()))
            .center_y()
            .gap(6.0)
            .opacity_motion(slider_curve(false))
            .child(status_mark(status, p, 6.0))
            .child(
                label(text, PIXEL.features(TABULAR), status_ink(status, p))
                    .key(format!("{key}-text")),
            ),
    );
    slot
}
/// A running timer for live status: "45s", "12m 05s", "1h 05m". Padding the trailing field
/// keeps its width steady from one tick to the next.
fn timer_label(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h {:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

/// The chat pane rounds its top corners and runs square to the window's bottom edge.
const PANE_CORNERS: [f32; 4] = [6.0, 6.0, 0.0, 0.0];

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let p = palette(s.dark);
    cx.foreground(p.ink);
    cx.focus_ring(2.0, 2.0, p.coral);
    // Every enabled button, tab, link and switch is a Tab stop in document order, as on the web,
    // even one that never said `focusable(true)`. Disabled controls stay out.
    cx.web_focus();
    cx.caption_color(p.muted);
    // The sidebar portraits' animation clock lives outside the view and needs this setting.
    s.portraits.set_reduced_motion(cx.is_reduced_motion());
    let narrow = cx
        .layout_with("workspace-root", |r| r[2] < 1050.0)
        .unwrap_or(false);
    // Below this width the sidebar stops docking and opens on demand instead.
    let compact = cx
        .layout_with("workspace-root", |r| r[2] < 760.0)
        .unwrap_or(false);
    let sidebar_width = if narrow {
        s.sidebar_width.min(216.0)
    } else {
        s.sidebar_width
    };
    let docked = (s.sidebar && !compact) || (compact && s.compact_sidebar);
    // The chat column's width from the last layout. The first frame guesses from the window,
    // less the gutters and a docked sidebar, so it already lays out close to its final lane.
    let main_width = cx.layout_with("chat-main", |r| r[2]).unwrap_or_else(|| {
        let sidebar = if docked { sidebar_width + 4.0 } else { 0.0 };
        cx.layout_with("workspace-root", move |r| r[2] - 16.0 - sidebar)
            .unwrap_or(900.0)
    });
    let wide = main_width >= WIDE_PANE;
    let mut context_popover_shown = false;
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
        // The header runs to the sidebar's edge, so the attention badge, which sits at its end,
        // has room beside the 22 px wordmark in a 232 px sidebar, even with the traffic lights.
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
    top = top.child(header(s, p, narrow, tabs_width).grow(1.0).min_w(0.0));
    // The pane and the sidebar's sea strip both run to the window's bottom edge, so the bottom
    // of the window is one edge, not a strip under the pane and another under the sidebar.
    let mut workspace = row().grow(1.0).min_h(0.0).px(8.0);
    if docked {
        workspace = workspace.child(sidebar(s, p, sidebar_width));
        if !compact {
            let max_width = (s.viewport_width - 480.0).clamp(200.0, 400.0);
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
                            s.sidebar_width = (drag.data + drag.dx / zoom).clamp(200.0, max_width);
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
                        s.sidebar_width = (s.sidebar_width + step).clamp(200.0, max_width);
                        s.checkpoint();
                        true
                    }),
            );
        }
    }
    let mut pane = col()
        .key("chat-pane")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .fill(p.panel)
        .corners(PANE_CORNERS)
        .clip_corners(PANE_CORNERS);
    // A captain home shows its Brief instead of session details: beside the chat, or as a
    // sheet over it on a narrow window.
    let brief = s.show_activity && captain_ui::brief_captain(s).is_some();
    if s.show_activity && !brief && (narrow || s.activity_full) {
        pane = pane.child(activity(s, p, true));
    } else {
        let mut body = row().grow(1.0).min_h(0.0);
        let main = col()
            .key("chat-main")
            .grow(1.0)
            .min_w(0.0)
            .min_h(0.0)
            .child(chat_header(s, p, narrow, compact))
            // Scrolled messages are cut under the header, so give them a boundary.
            .child(rule(p).key("chat-header-rule"));
        let main = if captain_ui::is_earlier(s) {
            main.child(conversation(s, p, wide, main_width))
                .child(captain_ui::earlier_bar(s, p, wide, main_width))
        } else if s.current().messages.is_empty()
            && s.current().tools.is_empty()
            && s.current().native.compactions.is_empty()
        {
            // Sizes from the last layout: the hero keeps its height only while the composer
            // and its crew tray still fit under it.
            let pane = cx.layout_with("empty-thread", |r| r[3]);
            let lane = cx.layout_with("composer-lane", |r| r[3]);
            let hero_fits = match (pane, lane) {
                (Some(pane), Some(lane)) => pane - lane >= CAPTAIN_HERO_HEIGHT,
                _ => true,
            };
            main.child(empty_thread(s, p, narrow, wide, main_width, hero_fits))
        } else {
            main.child(conversation(s, p, wide, main_width))
                .child(composer(s, p, narrow, wide, main_width))
        };
        body = body.child(main);
        context_popover_shown = true;
        if brief && !narrow {
            body = body
                .child(el().w(1.0).h_full().fill(p.line))
                .child(captain_ui::brief_side(s, p));
        } else if s.show_activity && !brief {
            body = body
                .child(el().w(1.0).h_full().fill(p.line))
                .child(activity(s, p, false));
        }
        pane = pane.child(body);
    }
    // The pane's 1 px `line` border sits over its content, which would otherwise paint across it.
    pane = pane.child(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .corners(PANE_CORNERS)
            .border(1.0, p.line)
            .a11y_hidden(),
    );
    let mut root = col()
        .key("workspace-root")
        .on_layout(|s: &mut State, rect| {
            s.viewport_width = rect[2];
            s.viewport_height = rect[3];
        })
        .on_key(|s: &mut State, ev, _| s.undo_key(ev, true))
        .fill(p.bg)
        .child(top)
        .child(workspace.child(pane));
    // The context popover floats at the root, after the ring it anchors to, so the composer
    // never makes room for it and no clipping ancestor cuts it off.
    if context_popover_shown
        && context_ring_shown(s.current())
        && s.current().context.popover_open()
    {
        root = root.child(context_popover(s, p));
    }
    if brief && narrow {
        let top = chrome.height.max(40.0 * zoom) / zoom;
        root = root.child(captain_ui::brief_sheet(s, p, top));
    }
    if s.rename_thread.is_some() || s.delete_thread.is_some() {
        root = root.child(thread_dialog(s, p));
    } else if s.project_picker == Some(ProjectPicker::AddProject) {
        root = root.child(add_project_palette(s, p));
    } else if s.project_picker.is_some() {
        root = root.child(project_picker(s, p));
    } else if s.captain_editor.is_some() {
        let height = cx.layout_with("workspace-root", |r| r[3]).unwrap_or(840.0);
        root = root.child(captain_editor(s, p, height));
    } else if s.settings_picker.is_some() {
        root = root.child(settings_picker(s, p));
    } else if s.captain_picker {
        root = root.child(captain_picker(s, p));
    } else if s.projects.clone_open {
        root = root.child(clone_dialog(s, p));
    } else if s.current().native.access_picker {
        root = root.child(pi_ui::access_picker(s, p));
    } else if s.current().native.conversation.is_some() {
        root = root.child(pi_ui::conversation_picker(s, p));
    } else if s.thread_menu {
        root = root.child(thread_menu(s, p));
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
        root = root.child(row_menu(s, p));
    }
    if s.snooze_menu.is_some() {
        root = root.child(snooze_menu(s, p));
    }
    if s.settings_menu {
        root = root.child(settings_menu(s, p));
    }
    if s.machines.picker {
        root = root.child(machine_picker(s, p));
    }
    if s.machines.dialog {
        root = root.child(machine_dialog(s, p));
    }
    for dialog in captain_ui::dialogs(s, p) {
        root = root.child(dialog);
    }
    if let Some(pill) = crew_ui::attention_row(s, p) {
        root = root.child(pill);
    }
    if s.attention_menu && !s.attention().is_empty() {
        // The pane's top left corner, worked out rather than read from the last layout, so the
        // popover is in place on its first frame: the workspace's 8 pt inset, the sidebar and
        // its 4 pt resize handle, under the top strip.
        let pane = docked.then(|| {
            let handle = if compact { 0.0 } else { 4.0 };
            (
                8.0 + sidebar_width + handle,
                chrome.height.max(40.0 * zoom) / zoom,
            )
        });
        root = root.child(crew_ui::attention_menu(s, p, pane));
    }
    root = match frame {
        Some((w, h)) if (zoom - 1.0).abs() > 0.001 => root.w(w / zoom).h(h / zoom).scale(zoom),
        _ => root.w_full().h_full(),
    };
    el().key("zoom-frame")
        .w_full()
        .h_full()
        .fill(p.bg)
        .center()
        .child(root)
}

/// A model's display name and its CPA route: "Gemini 3.8 Flash (CPA → Antigravity)" is
/// ("Gemini 3.8 Flash", Some("CPA → Antigravity")). Names without a route stay whole.
fn split_model_label(label: &str) -> (&str, Option<&str>) {
    let label = label.trim();
    if let Some(open) = label.rfind(" (")
        && label.ends_with(')')
    {
        return (&label[..open], Some(&label[open + 2..label.len() - 1]));
    }
    (label, None)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Vendor {
    OpenAi,
    Anthropic,
    Antigravity,
    Other,
}

/// The vendor behind a CPA model, from its route first and its id second, so the icon follows
/// KSG Code's provider marks: Codex is OpenAI, Claude is Anthropic, Antigravity is Google's.
fn model_vendor(id: &str, label: &str) -> Vendor {
    let (_, route) = split_model_label(label);
    let route = route.unwrap_or("").to_lowercase();
    let id = id.to_lowercase();
    let either = |words: &[&str]| words.iter().any(|w| route.contains(w) || id.contains(w));
    if either(&["codex", "openai"]) || id.starts_with("gpt") || id.starts_with("o3") {
        Vendor::OpenAi
    } else if either(&["anthropic", "claude"]) {
        Vendor::Anthropic
    } else if either(&["antigravity", "gemini", "google"]) {
        Vendor::Antigravity
    } else {
        Vendor::Other
    }
}

/// Provider marks ported from KSG Code's `Icons.tsx` (`OpenAI`, `ClaudeAI`, `AntigravityIcon`).
fn provider_icon(s: &State, p: Palette, vendor: Vendor, size: f32) -> El {
    static OPENAI: Icon = Icon {
        name: "OPENAI",
        view: 411.0,
        body: r#"<g transform="translate(-100 -100)"><path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd" d="M252.794 108.802C289.191 99.0484 326.265 110.305 351.148 135.135C385.113 126.072 422.85 134.862 449.492 161.505C476.136 188.149 484.925 225.888 475.862 259.85V259.854C500.696 284.735 511.95 321.81 502.198 358.207C492.447 394.602 464.161 421.084 430.215 430.217C421.083 464.162 394.603 492.448 358.206 502.199C321.812 511.951 284.734 500.693 259.852 475.864C225.887 484.927 188.15 476.137 161.507 449.495C134.864 422.851 126.073 385.111 135.136 351.149C110.304 326.266 99.0496 289.192 108.801 252.795C118.552 216.4 146.84 189.918 180.784 180.785C189.917 146.841 216.396 118.553 252.794 108.802ZM374.292 407.145C374.292 411.271 372.092 415.086 368.517 417.148L283.723 466.102C302.487 480.585 327.555 486.459 352.217 479.852C386.997 470.532 410.068 439.312 410.555 405.006V317.717C410.555 315.08 409.125 312.621 406.843 311.303L374.292 292.509V407.145ZM251.868 415.897C248.296 417.959 243.893 417.959 240.317 415.897L155.526 366.942C152.366 390.436 159.811 415.08 177.866 433.136H177.863C203.325 458.594 241.896 462.962 271.85 446.232L347.449 402.586C349.735 401.268 351.148 398.8 351.148 396.163V358.579L251.868 415.897ZM368.602 220.628C366.319 219.309 363.474 219.318 361.191 220.637L328.641 239.431L427.921 296.749C431.496 298.811 433.697 302.627 433.697 306.752V404.661C455.622 395.654 473.244 376.881 479.851 352.218C489.169 317.442 473.668 281.85 444.201 264.274L368.602 220.628ZM177.303 206.34C155.377 215.348 137.756 234.122 131.148 258.783C121.832 293.561 137.331 329.153 166.799 346.727L242.398 390.373C244.68 391.692 247.525 391.684 249.807 390.366L282.357 371.572L183.078 314.253C179.504 312.189 177.303 308.375 177.303 304.251V206.34ZM259.849 279.145V331.858L305.5 358.213L351.15 331.858V279.145L305.5 252.789L259.849 279.145ZM327.276 144.9C308.512 130.418 283.445 124.543 258.782 131.15C224.002 140.471 200.931 171.691 200.445 205.995V293.286C200.445 295.923 201.875 298.381 204.158 299.7L236.707 318.493V203.856C236.707 199.731 238.909 195.916 242.483 193.853L327.276 144.9ZM433.137 177.867C407.675 152.407 369.103 148.038 339.149 164.769L263.55 208.415C261.265 209.734 259.852 212.202 259.852 214.838V252.423L359.132 195.105C362.703 193.041 367.108 193.041 370.682 195.105L455.473 244.06C458.635 220.567 451.189 195.922 433.135 177.867H433.137Z"/></g>"#,
    };
    static CLAUDE: Icon = Icon {
        name: "CLAUDE",
        view: 257.0,
        body: r##"<path fill="#d97757" d="m50.228 170.321 50.357-28.257.843-2.463-.843-1.361h-2.462l-8.426-.518-28.775-.778-24.952-1.037-24.175-1.296-6.092-1.297L0 125.796l.583-3.759 5.12-3.434 7.324.648 16.202 1.101 24.304 1.685 17.629 1.037 26.118 2.722h4.148l.583-1.685-1.426-1.037-1.101-1.037-25.147-17.045-27.22-18.017-14.258-10.37-7.713-5.25-3.888-4.925-1.685-10.758 7-7.713 9.397.649 2.398.648 9.527 7.323 20.35 15.75L94.817 91.9l3.889 3.24 1.555-1.102.195-.777-1.75-2.917-14.453-26.118-15.425-26.572-6.87-11.018-1.814-6.61c-.648-2.723-1.102-4.991-1.102-7.778l7.972-10.823L71.42 0 82.05 1.426l4.472 3.888 6.61 15.101 10.694 23.786 16.591 32.34 4.861 9.592 2.592 8.879.973 2.722h1.685v-1.556l1.36-18.211 2.528-22.36 2.463-28.776.843-8.1 4.018-9.722 7.971-5.25 6.222 2.981 5.12 7.324-.713 4.73-3.046 19.768-5.962 30.98-3.889 20.739h2.268l2.593-2.593 10.499-13.934 17.628-22.036 7.778-8.749 9.073-9.657 5.833-4.601h11.018l8.1 12.055-3.628 12.443-11.342 14.388-9.398 12.184-13.48 18.147-8.426 14.518.778 1.166 2.01-.194 30.46-6.481 16.462-2.982 19.637-3.37 8.88 4.148.971 4.213-3.5 8.62-20.998 5.184-24.628 4.926-36.682 8.685-.454.324.519.648 16.526 1.555 7.065.389h17.304l32.21 2.398 8.426 5.574 5.055 6.805-.843 5.184-12.962 6.611-17.498-4.148-40.83-9.721-14-3.5h-1.944v1.167l11.666 11.406 21.387 19.314 26.767 24.887 1.36 6.157-3.434 4.86-3.63-.518-23.526-17.693-9.073-7.972-20.545-17.304h-1.36v1.814l4.73 6.935 25.017 37.59 1.296 11.536-1.814 3.76-6.481 2.268-7.13-1.297-14.647-20.544-15.1-23.138-12.185-20.739-1.49.843-7.194 77.448-3.37 3.953-7.778 2.981-6.48-4.925-3.436-7.972 3.435-15.749 4.148-20.544 3.37-16.333 3.046-20.285 1.815-6.74-.13-.454-1.49.194-15.295 20.999-23.267 31.433-18.406 19.702-4.407 1.75-7.648-3.954.713-7.064 4.277-6.286 25.47-32.405 15.36-20.092 9.917-11.6-.065-1.686h-.583L44.07 198.125l-12.055 1.555-5.185-4.86.648-7.972 2.463-2.593 20.35-13.999-.064.065Z"/>"##,
    };
    match vendor {
        Vendor::Antigravity => el()
            .size(size, size)
            .pixels_fit(s.artwork.antigravity.pixels(), rune::SurfaceFit::Fit)
            .a11y_hidden(),
        Vendor::OpenAi => rune::icon(&OPENAI, size).color(p.ink).a11y_hidden(),
        Vendor::Anthropic => rune::icon(&CLAUDE, size).a11y_hidden(),
        Vendor::Other => rune::icon(&pixel_icons::CHIP, size)
            .color(p.muted)
            .a11y_hidden(),
    }
}

/// Pi's level ids in the slider's words: "xhigh" reads "XHigh", "medium" reads "Medium".
fn effort_name(level: &str) -> String {
    match level {
        "" => "Default".into(),
        "xhigh" => "XHigh".into(),
        _ => {
            let mut chars = level.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    }
}

/// The composer's one model control, as KSG Code's `ComposerModelEffort` trigger: the
/// provider's mark, the model's name without its route, and the effort level.
fn model_effort_trigger(s: &State, p: Palette, narrow: bool, locked: bool) -> El {
    let t = s.current();
    let (name, _) = split_model_label(&t.model);
    let effort = effort_name(&t.thinking);
    let both_default = name == crate::model::DEFAULT_MODEL && t.thinking.is_empty();
    let mut trigger = row()
        .key("model-effort")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!(
            "Model {name}, reasoning {effort}. Change model or reasoning"
        ))
        .help(t.model.clone())
        .focusable(!locked)
        .center_y()
        .gap(7.0)
        .h(28.0)
        .px(8.0)
        .radius(4.0)
        .shrink()
        .min_w(0.0)
        .max_w(if narrow { 200.0 } else { 340.0 })
        .opacity(if locked { 0.65 } else { 1.0 })
        .disabled(locked)
        .hover(p.selected)
        .press(p.selected);
    if !locked {
        trigger = trigger.on_press(|s: &mut State| {
            s.settings_picker = Some(SettingsPicker::Thinking);
            s.thinking_drag = None;
            s.refresh_catalog();
        });
    }
    trigger
        .child(provider_icon(
            s,
            p,
            model_vendor(&t.model_id, &t.model),
            14.0,
        ))
        .child(
            label(name, TextStyle::new(13.0, 600.0), p.ink)
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        // "Default model" and a "Default" level say the same thing twice, so the level drops out.
        .child(if both_default {
            el()
        } else {
            pixel_text(effort, p.muted)
        })
}

const PICKER_WIDTH: f32 = 320.0;

/// The model and reasoning popover. Its effort half is Codex's slider in Shina's colours; its
/// model half drills into KSG Code's searchable, provider-grouped list in place.
fn settings_picker(s: &State, p: Palette) -> El {
    let t = s.current();
    let is_model = s.settings_picker == Some(SettingsPicker::Model);
    let can_choose = !t.status.busy() && !t.configuring;
    let mut card = col()
        .key("pi-settings-picker")
        .role("dialog")
        .label(if is_model {
            "Choose a model"
        } else {
            "Model and reasoning"
        })
        .w(PICKER_WIDTH)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .on_press(|_: &mut State| {});
    card = if is_model {
        model_list(s, p, can_choose, 320.0, card)
    } else {
        card.child(effort_slider(t, s.thinking_drag, p, can_choose))
            .child(rule(p))
            .child(model_row(s, p))
    };
    if let Some(error) = &t.error {
        card = card.child(
            label(error, SMALL, p.danger)
                .wrap_anywhere()
                .px(16.0)
                .pb(10.0),
        );
    }
    if let Some(error) = &s.catalog.error {
        card = card
            .child(label(error, SMALL, p.danger).wrap_anywhere().px(16.0))
            .child(
                button("retry-catalog", "Retry", p, |s| s.refresh_catalog())
                    .ml(10.0)
                    .mb(8.0),
            );
    }
    // Above the composer, right edges aligned, as KSG Code's popover opens. Rune's anchored
    // layer draws over the page and shifts the card to stay inside the window.
    card.anchored_fit("composer", rune::Side::Top, rune::Align::End, 8.0)
        .modal(true, |s: &mut State| s.settings_picker = None)
}

const SLIDER_H: f32 = 28.0;

/// The closed hand a held slider shows, as Codex's `cursor: grabbing`. winit draws
/// `Cursor::Grabbing` as the four-way move arrow on Windows, so Shina ships its own, drawn by
/// `tools/cursors.py` at 2x with the hot spot in the palm.
const GRABBING_CURSOR: rune::CursorImage = rune::CursorImage::png(
    include_bytes!("../assets/cursors/grabbing.png"),
    [16.0, 16.0],
);

/// Codex's top-level flourish: specks of light drift through a full track and twinkle.
/// Each speck rides a looping slide across the track and pulses on its own opacity loop.
fn sparkles(p: Palette) -> El {
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
fn slider_curve(dragging: bool) -> rune::Curve {
    rune::Curve::tween(
        if dragging { 0.15 } else { 0.3 },
        rune::Easing::cubic(0.23, 1.0, 0.32, 1.0),
    )
}

fn effort_slider(t: &Thread, dragging: Option<f32>, p: Palette, can_choose: bool) -> El {
    let levels = t.thinking_levels.clone();
    let count = levels.len();
    let current = levels.iter().position(|level| *level == t.thinking);
    // While held, the thumb follows the pointer and the title names the nearest level.
    let held = dragging.filter(|_| count > 1).map(|f| f.clamp(0.0, 1.0));
    let shown = held
        .map(|f| (f * (count - 1) as f32).round() as usize)
        .or(current);
    let title = shown
        .and_then(|index| levels.get(index))
        .map_or_else(|| effort_name(&t.thinking), |level| effort_name(level));
    let mut section = col()
        .key("effort-section")
        .px(16.0)
        .pt(14.0)
        .pb(16.0)
        .gap(12.0);
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
        .a11y_value(effort_name(&t.thinking))
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
                .radius(2.0)
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
                        // Codex's thumb in the pixel skin: a `card` disc in a 2 px frame.
                        el().key("effort-thumb-knob")
                            .absolute(-cap, 0.0)
                            .size(SLIDER_H, SLIDER_H)
                            .radius(cap)
                            .fill(p.card)
                            .border(2.0, p.frame),
                    ),
                )
                .child(el().key("effort-thumb-after").w(0.0).grow(1.0 - frac)),
        );
    }
    if can_choose {
        let keyed = levels.clone();
        let dragged = levels.clone();
        let ends = levels.clone();
        slider = slider
            // Rune's NSSlider keyboard: arrows and pages step, Home and End jump.
            .on_step(move |s: &mut State, delta, tx| {
                let last = keyed.len() as i32 - 1;
                let at = keyed
                    .iter()
                    .position(|l| *l == s.current().thinking)
                    .unwrap_or(0) as i32;
                let next = (at + delta).clamp(0, last) as usize;
                if next as i32 != at {
                    tx.motion_curve(slider_curve(false));
                    s.current_mut().set_thinking(keyed[next].clone());
                    s.checkpoint();
                }
            })
            .on_step_limit(move |s: &mut State, max, tx| {
                let level = if max { ends.last() } else { ends.first() };
                if let Some(level) = level.cloned()
                    && level != s.current().thinking
                {
                    tx.motion_curve(slider_curve(false));
                    s.current_mut().set_thinking(level);
                    s.checkpoint();
                }
            })
            .on_drag(move |s: &mut State, drag, tx| {
                // Where the pointer is between the end caps' centres, as Codex measures it.
                let along = ((drag.x - cap) / (drag.w - 2.0 * cap).max(1.0)).clamp(0.0, 1.0);
                let index = (along * (dragged.len() - 1) as f32).round() as usize;
                tx.animated();
                if drag.cancelled() {
                    tx.motion_curve(slider_curve(false));
                    s.thinking_drag = None;
                } else if drag.phase == rune::Phase::Up {
                    // Release glides onto the nearest level.
                    tx.motion_curve(slider_curve(false));
                    s.thinking_drag = None;
                    if dragged[index] != s.current().thinking {
                        s.current_mut().set_thinking(dragged[index].clone());
                        s.checkpoint();
                    }
                } else {
                    // A press glides to the pointer; a drag follows it closely.
                    tx.motion_curve(slider_curve(drag.phase == rune::Phase::Move));
                    s.thinking_drag = Some(along);
                }
            });
    }
    section.child(slider).child(
        row()
            .child(label(
                effort_name(&levels[0]),
                TextStyle::new(11.0, 450.0),
                p.muted,
            ))
            .child(spacer())
            .child(label(
                effort_name(&levels[count - 1]),
                TextStyle::new(11.0, 450.0),
                p.muted,
            )),
    )
}

fn model_row(s: &State, p: Palette) -> El {
    let t = s.current();
    let (name, route) = split_model_label(&t.model);
    let mut text = col().gap(1.0).grow(1.0).min_w(0.0).child(
        label(name, TextStyle::new(13.5, 500.0), p.ink)
            .truncate()
            .min_w(0.0),
    );
    if let Some(route) = route {
        text = text.child(label(route, SMALL, p.muted).truncate().min_w(0.0));
    }
    row()
        .key("open-model-list")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!("Model: {}. Choose another model", t.model))
        .focusable(true)
        .center_y()
        .gap(10.0)
        .pl(16.0)
        .pr(12.0)
        .py(10.0)
        .hover(p.selected)
        .press(p.selected)
        .on_press(|s: &mut State| {
            s.model_search.replace_text("");
            s.settings_picker = Some(SettingsPicker::Model);
        })
        .child(provider_icon(
            s,
            p,
            model_vendor(&t.model_id, &t.model),
            18.0,
        ))
        .child(text)
        .child(icon("open", p.muted))
}

/// Harness rows reuse KSG Code's ModelPickerContent groups and ModelListRow proportions.
fn model_list(s: &State, p: Palette, can_choose: bool, max_h: f32, card: El) -> El {
    let t = s.current();
    let header = row()
        .center_y()
        .gap(8.0)
        .px(12.0)
        .py(10.0)
        .child(icon_button(
            "back-to-effort",
            "Back to reasoning",
            "back",
            p,
            |s| s.settings_picker = Some(SettingsPicker::Thinking),
        ))
        .child(
            row()
                .center_y()
                .gap(6.0)
                .h(30.0)
                .px(10.0)
                .radius(4.0)
                .fill(p.selected)
                .field_focus(&s.model_search, p)
                .grow(1.0)
                .min_w(0.0)
                .child(icon("search", p.muted))
                .child(
                    text_input(&s.model_search, TextStyle::new(13.0, 420.0))
                        .key("model-search")
                        .label("Search models")
                        .placeholder("Search models", p.muted)
                        .color(p.ink)
                        .caret(p.coral, p.coral_bg)
                        .grow(1.0)
                        .min_w(0.0)
                        .on_edit(|s: &mut State| &mut s.model_search)
                        // Down from the search field goes into the list, as in KSG Code's
                        // combobox; the list's own group takes the arrows from there.
                        .on_input_key(|s: &mut State, event, tx| {
                            if matches!(event.key, rune::Key::Down) {
                                // The field's own focus flag would win over `tx.focus`.
                                s.model_search.focused = false;
                                tx.focus("model-list", true);
                                return true;
                            }
                            false
                        }),
                ),
        );
    let query = s.model_search.text.to_lowercase();
    let mut list = col()
        .key("model-list")
        .role("listbox")
        .label("Models")
        // Up, Down, Home and End move through the harnesses and models, Return chooses.
        .focus_group()
        .w_full()
        .max_h(max_h)
        .pb(6.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let mut group: Option<String> = None;
    let mut count = 0;
    list = list.child(
        pixel_label("Harness", p.muted)
            .key("harness-group")
            .px(16.0)
            .pt(8.0)
            .pb(4.0),
    );
    let harnesses = [("pi", "Pi", false), ("codex", "Codex", false)]
        .into_iter()
        .chain(
            s.harnesses
                .agents
                .iter()
                .map(|agent| (agent.id.as_str(), agent.name.as_str(), true)),
        );
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for (id, name, acp) in harnesses {
        let selected = t.harness == id;
        let enabled = t.can_choose_harness();
        let chosen = id.to_owned();
        // An ACP agent can share an id with a built-in harness. One with the same name too is
        // the same entry and is not listed twice. One with another name is listed with its
        // ACP tag, and its key gets a suffix so keys stay unique.
        if seen.contains(&(id, name)) {
            continue;
        }
        let repeats = seen.iter().filter(|&&(earlier, _)| earlier == id).count();
        seen.push((id, name));
        let key = if repeats == 0 {
            format!("choose-harness-{id}")
        } else {
            format!("choose-harness-{id}-{}", repeats + 1)
        };
        let mut item = row()
            .key(key)
            .role("option")
            .label(name)
            .checked(selected)
            .focusable(enabled)
            .center_y()
            .gap(10.0)
            .h(34.0)
            // Inset 4 px from the frame, with the same 16 px text edge as the headings.
            .mx(4.0)
            .pl(12.0)
            .pr(8.0)
            .radius(4.0)
            .fill(if selected {
                p.coral_bg
            } else {
                Color::TRANSPARENT
            })
            .hover(if selected { p.coral_bg } else { p.selected })
            // Arrow keys show where they are with the hover wash.
            .focus_fill(p.selected)
            .focus_selected(selected)
            .disabled(!enabled)
            .child(harness_icon(s, p, id, name))
            .child(
                label(name, TextStyle::new(13.5, 450.0), p.ink)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        if acp {
            // KSG Code's `ModelListRow` "New" tag, here naming the protocol the agent speaks.
            item = item.child(outlined(
                pixel_text("ACP", p.muted)
                    .key(format!("harness-tag-{id}"))
                    .px(4.0)
                    .py(1.0)
                    .radius(4.0),
                p,
            ));
        }
        item = item.child(spacer());
        if selected {
            item = item.child(icon("check", p.sea));
        }
        if enabled {
            item = item
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    s.choose_harness(&chosen);
                });
        }
        list = list.child(item);
    }
    if let Some(error) = &s.harnesses.error {
        list = list.child(label(error, SMALL, p.muted).wrap().px(16.0).py(6.0));
    }
    for (index, model) in t.models.iter().enumerate().filter(|(_, m)| {
        format!("{} {} {}", m.provider, m.id, m.name)
            .to_lowercase()
            .contains(&query)
    }) {
        count += 1;
        let (name, route) = split_model_label(&model.name);
        let heading = route.map_or_else(|| model.provider.to_uppercase(), str::to_string);
        if group.as_deref() != Some(heading.as_str()) {
            list = list.child(
                label(&heading, TextStyle::new(11.5, 600.0), p.muted)
                    .px(16.0)
                    .pt(8.0)
                    .pb(4.0),
            );
            group = Some(heading);
        }
        let provider = model.provider.clone();
        let id = model.id.clone();
        let selected = t.model_provider == model.provider && t.model_id == model.id;
        let mut item = row()
            .key(format!("choose-model-{index}"))
            .role("option")
            .label(&model.name)
            .checked(selected)
            .focusable(can_choose)
            .center_y()
            .gap(10.0)
            .h(34.0)
            // Inset 4 px from the frame, with the same 16 px text edge as the headings.
            .mx(4.0)
            .pl(12.0)
            .pr(8.0)
            .radius(4.0)
            .fill(if selected {
                p.coral_bg
            } else {
                Color::TRANSPARENT
            })
            .hover(if selected { p.coral_bg } else { p.selected })
            .focus_fill(p.selected)
            .focus_selected(selected)
            .disabled(!can_choose)
            .child(provider_icon(
                s,
                p,
                model_vendor(&model.id, &model.name),
                16.0,
            ))
            .child(
                label(name, TextStyle::new(13.5, 450.0), p.ink)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if selected {
            item = item.child(icon("check", p.sea));
        }
        if can_choose {
            item = item
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    s.current_mut().set_model(provider.clone(), id.clone());
                    s.settings_picker = Some(SettingsPicker::Thinking);
                    s.checkpoint();
                });
        }
        list = list.child(item);
    }
    if count == 0 {
        // The harnesses end here and the models begin: a rule and a heading, so the note is
        // plainly about models and not about the last harness.
        list = list
            .child(rule(p).mt(6.0))
            .child(
                pixel_label("Models", p.muted)
                    .key("models-group")
                    .px(16.0)
                    .pt(8.0)
                    .pb(4.0),
            )
            .child(
                label(
                    if t.harness != "pi" && t.models.is_empty() {
                        "This harness uses its configured model until it reports available models."
                    } else if t.models.is_empty() && s.catalog.remaining().is_some() {
                        "Reading the CPA catalog..."
                    } else if t.models.is_empty() {
                        "No CPA models are available."
                    } else {
                        "No matching models."
                    },
                    SMALL,
                    p.muted,
                )
                .wrap()
                .px(16.0)
                .py(10.0),
            );
    }
    card.child(header).child(rule(p)).child(list)
}

/// A harness row's mark, as KSG Code's `ProviderInstanceIcon` draws an ACP agent: the vendor's
/// mark when the agent names one, Pi's own glyph, or a generic agent glyph.
fn harness_icon(s: &State, p: Palette, id: &str, name: &str) -> El {
    static PI: Icon = Icon {
        name: "PI",
        view: 16.0,
        body: r#"<path d="M3 4.5h10M6 4.5V12M10.25 4.5v5.75a1.75 1.75 0 0 0 1.75 1.75h.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
    };
    static AGENT: Icon = Icon {
        name: "AGENT",
        view: 16.0,
        body: r#"<rect x="2" y="3" width="12" height="10" rx="2.5" fill="none" stroke="currentColor"/><path d="m5 6.5 2 1.5-2 1.5M8.5 10H11" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#,
    };
    if id == "pi" {
        return rune::icon(&PI, 16.0).color(p.ink).a11y_hidden();
    }
    match model_vendor(id, name) {
        Vendor::Other => rune::icon(&AGENT, 16.0).color(p.muted).a11y_hidden(),
        vendor => provider_icon(s, p, vendor, 16.0),
    }
}

fn sidebar_toggle(p: Palette, compact: bool) -> El {
    icon_button("sidebar-toggle", "Toggle sidebar", "sidebar", p, move |s| {
        if compact {
            s.compact_sidebar = !s.compact_sidebar;
        } else {
            s.sidebar = !s.sidebar;
        }
    })
}

/// The ship mark at 24 px with a 1 px `frame` outline.
fn shina_mark(s: &State, p: Palette) -> El {
    el().key("shina-logo")
        .size(
            s.artwork.snap_points(HEADER_MARK),
            s.artwork.snap_points(HEADER_MARK),
        )
        .radius(4.0)
        .clip_rounded(4.0)
        .pixels(&s.artwork.mark_pixels(s.dark, HEADER_MARK))
        // The day mark's sky is the chrome's sky, so a 1 px frame keeps its edge. The frame is
        // an overlay because Rune paints a clipped image over its border.
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(4.0)
                .border(1.0, p.frame)
                .a11y_hidden(),
        )
}

/// Side of the ship mark above an empty list. The mark is 32 cells, so 32 pt is one point per
/// cell and two device pixels at 2x: every cell lands on whole pixels.
const EMPTY_MARK: f32 = 32.0;

/// The empty state of a list: the ship mark at a whole multiple of its cells, above the
/// explanation.
fn empty_list(s: &State, p: Palette, key: &str, text: &str) -> El {
    col()
        .key(key)
        .role("status")
        .label(text)
        .items_center()
        .gap(8.0)
        .px(12.0)
        .py(12.0)
        .child(
            el().key(format!("{key}-mark"))
                .size(
                    s.artwork.snap_points(EMPTY_MARK),
                    s.artwork.snap_points(EMPTY_MARK),
                )
                .radius(4.0)
                .clip_rounded(4.0)
                .pixels(&s.artwork.mark_pixels(s.dark, EMPTY_MARK))
                .a11y_hidden(),
        )
        .child(
            label(text, SMALL, p.muted)
                .align(rune::Align::Center)
                .wrap()
                .w_full(),
        )
}

/// The wordmark's size in Departure Mono points, for a header `line` points wide after the
/// native controls and the gap before the tabs. 22 px whenever it clears the attention badge,
/// 11 px (the face's other native size) in a narrow sidebar, and none in a truly narrow one.
/// Width a macOS title strip reserves for the window buttons, which every platform assumes when
/// it sizes the wordmark.
const TITLE_CONTROLS: f32 = 72.0;

fn wordmark_size(line: f32, badge: bool) -> Option<f32> {
    let badge = if badge {
        BADGE_CLEARANCE + HEADER_BADGE
    } else {
        0.0
    };
    let before = HEADER_TOGGLE + HEADER_MARK;
    if line >= before + wordmark_gap(22.0) + WORDMARK_WIDTH + badge {
        Some(22.0)
    } else if line >= before + wordmark_gap(11.0) + WORDMARK_WIDTH / 2.0 + badge {
        Some(11.0)
    } else {
        None
    }
}
/// The space between the mark and the wordmark: 6 pt at 22 px, 4 pt at 11 px. A 200 pt sidebar
/// under macOS's real 76 pt traffic-light inset leaves 112 pt, and the 11 px wordmark with the
/// badge then takes 111.
fn wordmark_gap(size: f32) -> f32 {
    if size < 22.0 { 4.0 } else { WORDMARK_GAP }
}

/// The sidebar's half of the title strip, left-aligned as T3 Code lays it out: the sidebar
/// toggle, then Shina's mark and name. Search and actions sit in the sidebar below.
fn sidebar_header(s: &State, p: Palette, compact: bool, line: f32, wordmark_line: f32) -> El {
    let mut header = row().h(36.0).center_y().child(sidebar_toggle(p, compact));
    if line >= HEADER_TOGGLE + HEADER_MARK {
        header = header.child(shina_mark(s, p));
        if let Some(size) = wordmark_size(wordmark_line, !s.attention().is_empty()) {
            header = header.child(
                label("Shina", pixel(size), p.ink)
                    .key("shina-wordmark")
                    .ml(wordmark_gap(size)),
            );
        }
    }
    header
        .child(spacer())
        .child(el().key(crew_ui::ATTENTION_ANCHOR).size(0.0, 28.0))
}

/// The title strip's left end when the sidebar is not docked: the toggle, the 24 px mark and
/// the attention badge, then the gap before the tabs. The wordmark waits for the sidebar.
fn undocked_header(s: &State, p: Palette, compact: bool) -> El {
    let mut header = row()
        .h(36.0)
        .center_y()
        .child(sidebar_toggle(p, compact))
        .child(shina_mark(s, p));
    if !s.attention().is_empty() {
        header = header.child(el().w(BADGE_CLEARANCE + 28.0).h(1.0));
    }
    header
        .child(el().key(crew_ui::ATTENTION_ANCHOR).size(0.0, 28.0))
        .pr(HEADER_TAB_GAP)
}
/// The space between the attention badge, at the header's end, and the first tab.
const HEADER_TAB_GAP: f32 = 12.0;
/// The space between the mark and the 22 px wordmark. The toggle and the mark touch their boxes.
const WORDMARK_GAP: f32 = 6.0;
/// The least space between the wordmark (or the mark) and the attention badge.
const BADGE_CLEARANCE: f32 = 4.0;
const HEADER_TOGGLE: f32 = 24.0;
const HEADER_MARK: f32 = 24.0;
/// The attention badge's width.
const HEADER_BADGE: f32 = 20.0;
/// "Shina" at 22 px: five cells of 14 px.
const WORDMARK_WIDTH: f32 = 70.0;

fn sidebar(s: &State, p: Palette, width: f32) -> El {
    let mut side = col()
        .key("sidebar")
        .w(width)
        .h_full()
        .min_h(0.0)
        .pr(8.0)
        .gap(2.0);
    // One row holds search and the sidebar's actions, as in T3 Code: filter by workspace,
    // add a workspace, and start a thread.
    let scoped = !s.scope_additional.is_empty() || s.project_scope != ProjectScope::All;
    let mut scope_machine: Option<String> = None;
    let scope = if !s.scope_additional.is_empty()
        || !s.machines.scope_remote.is_empty() && s.project_scope != ProjectScope::All
    {
        format!(
            "{} workspaces",
            s.scope_additional.len() + s.machines.scope_remote.len() + 1
        )
    } else {
        match s.project_scope {
            ProjectScope::All => "All workspaces".into(),
            ProjectScope::NoProject => "No workspace".into(),
            ProjectScope::Project(id) => {
                let owner = s
                    .machines
                    .scope_server
                    .as_deref()
                    .unwrap_or(&s.machines.selected);
                let name = if owner == s.machines.selected {
                    s.project(Some(id)).map(|p| p.name.as_str())
                } else {
                    s.machines
                        .views
                        .get(owner)
                        .and_then(|view| view["index"]["projects"].as_array())
                        .and_then(|projects| {
                            projects
                                .iter()
                                .find(|project| project["id"].as_u64() == Some(id as u64))
                        })
                        .and_then(|project| project["name"].as_str())
                }
                .unwrap_or("Workspace");
                if s.client.is_some() {
                    scope_machine = Some(s.machine_label(owner));
                }
                name.into()
            }
        }
    };
    // The accessibility label and tooltip join the machine with a comma; the chip shows it as
    // its own label.
    let scope_words = match &scope_machine {
        Some(machine) => format!("{scope}, {machine}"),
        None => scope.clone(),
    };
    if let Some(notice) = crew_ui::recovery_banner(s, p) {
        side = side.child(notice);
    }
    side = side.child(
        row()
            .key("workspace-context")
            .h(36.0)
            .mb(6.0)
            .pr(4.0)
            .gap(2.0)
            .center_y()
            .child(
                row()
                    .key("thread-search-field")
                    .h(32.0)
                    .px(ROW_INSET)
                    .gap(8.0)
                    .center_y()
                    .radius(4.0)
                    .grow(1.0)
                    .min_w(0.0)
                    // A field: a 1 px `line` border, and a 2 px coral border while focused.
                    .field_focus(&s.search, p)
                    .hover(p.selected)
                    .child(icon("search", p.muted))
                    .child(
                        text_input(&s.search, LABEL)
                            .key("thread-search")
                            .label("Search conversations")
                            .placeholder("Search", p.muted)
                            .color(p.ink)
                            .single_line()
                            .grow(1.0)
                            .min_w(0.0)
                            .on_edit(|s: &mut State| &mut s.search),
                    ),
            )
            .child(
                toggle_button(
                    "project-scope",
                    &format!("Workspace: {scope_words}"),
                    "filter",
                    scoped,
                    p,
                    |s| s.open_picker(ProjectPicker::Scope),
                )
                .help(format!("Workspace: {scope_words}")),
            )
            .child(project_switcher(s, p).help("Add workspace"))
            .child(
                icon_button("new-thread", "New thread", "compose", p, |_| {})
                    .on_press_tx(|s: &mut State, tx| s.new_thread_click(tx.mods().shift))
                    .help("New thread. Shift-click: in the current workspace"),
            ),
    );
    // A workspace filter names itself and clears in one click.
    if scoped {
        side = side.child(
            row()
                .key("scope-chip")
                .h(28.0)
                .pl(ROW_INSET)
                .pr(4.0)
                .gap(6.0)
                .center_y()
                .child(icon("folder", p.muted))
                .child(label(&scope, SMALL, p.ink).truncate().shrink().min_w(0.0))
                .child(match &scope_machine {
                    Some(machine) => label(machine, SMALL, p.faint).truncate().max_w(MACHINE_CAP),
                    None => el(),
                })
                .child(spacer())
                .child(icon_button(
                    "clear-project-scope",
                    "Show all workspaces",
                    "close",
                    p,
                    |s| {
                        s.project_scope = ProjectScope::All;
                        s.scope_additional.clear();
                        s.machines.scope_remote.clear();
                        s.selected_threads.clear();
                        s.checkpoint();
                    },
                )),
        );
    }
    if s.show_archive {
        side = side.child(
            row()
                .key("archive-heading")
                .h(30.0)
                .pl(ROW_INSET)
                .pr(8.0)
                .gap(8.0)
                .center_y()
                .child(icon("archive", p.muted))
                .child(label("Archive", TextStyle::new(13.0, 560.0), p.ink))
                .child(spacer())
                .child(
                    label("Back", SMALL, p.muted)
                        .key("leave-archive")
                        .role("button")
                        .cursor(rune::Cursor::Pointer)
                        .focusable(true)
                        .on_press(|s: &mut State| s.show_archive = false),
                ),
        );
    }
    // The content grows to fill the scroll area so the shelves sit at the bottom. Layout does
    // that in the same pass that sizes the area, so no earlier frame's measurement (stale on the
    // frame where zoom changes) can make it taller than the area. Its automatic minimum is its
    // own content, so a long list still scrolls.
    let mut content = col().key("sidebar-content").pb(2.0).gap(2.0).grow(1.0);
    let entries = sidebar_ui::entries(s);
    let captains: Vec<_> = entries.iter().filter(|entry| entry.captain).collect();
    if !s.show_archive || !captains.is_empty() {
        let mut heading = section_title("Captains", p);
        if !captains.is_empty() {
            heading = heading.child(
                pixel_text(captains.len().to_string(), p.muted)
                    .key("captain-count")
                    .mr(6.0),
            );
        }
        if !s.show_archive {
            heading = heading.child(icon_button(
                "assign-captain",
                "Add a captain",
                "plus",
                p,
                // With no captains yet, this goes straight to the create form, which also
                // picks the workspace. Otherwise it picks a workspace, then a captain.
                |s| {
                    if s.captains.is_empty() {
                        s.captain_project = None;
                        s.begin_new_captain();
                    } else {
                        s.open_picker(ProjectPicker::AssignCaptain);
                    }
                },
            ));
        }
        content = content.child(heading);
        for entry in &captains {
            let home = entry.id;
            let workers = entry
                .local
                .map(|_| s.visible_captain_workers(home))
                .unwrap_or_default();
            let mut crew = col()
                .key(if s.client.is_some() {
                    format!("crew-{}-{home}", entry.server)
                } else {
                    format!("crew-{home}")
                })
                .gap(1.0)
                .child(sidebar_ui::row(s, p, entry));
            if !workers.is_empty() {
                // Both content and row highlights are inset beneath their owning captain, and
                // the rail runs down from the centre of the captain's portrait.
                let mut children = col()
                    .key(format!("crew-threads-{home}"))
                    .ml(6.0 + CAPTAIN_SLOT / 2.0)
                    .pl(6.0)
                    .gap(1.0)
                    .child(
                        el().key(format!("crew-rail-{home}"))
                            .pin(Some(0.0), Some(0.0), None, Some(CREW_ROW / 2.0))
                            .w(1.0)
                            .fill(p.line)
                            .a11y_hidden(),
                    );
                for index in workers {
                    children = children.child(thread_row(s, index, p, true));
                }
                crew = crew.child(children);
            }
            content = content.child(crew);
        }
    }
    let all: Vec<_> = entries.iter().filter(|entry| !entry.captain).collect();
    let visible: Vec<_> = all.iter().copied().filter(|entry| entry.visible).collect();
    let dragging = s.drag_thread.is_some();
    // Omit an empty thread list under captains; without captains it holds the empty state.
    let listed = if s.search.text.is_empty() {
        !all.is_empty()
    } else {
        !visible.is_empty()
    };
    // Threads follow the captains without a heading of their own. The captains' label and a
    // gap mark where the crews end.
    if (listed || dragging) && !captains.is_empty() {
        content = content.child(el().h(12.0));
    }
    let drafts: Vec<_> = visible
        .iter()
        .copied()
        .filter(|entry| entry.draft)
        .collect();
    if !drafts.is_empty() {
        if !captains.is_empty() {
            content = content.child(dotted_rule(p));
        }
        content = content.child(section_title("Drafts", p));
        for entry in drafts {
            content = content.child(sidebar_ui::row(s, p, entry));
        }
    }
    let mut first_shelf = true;
    for section in [
        Section::Pinned,
        Section::Active,
        Section::Snoozed,
        Section::Settled,
    ] {
        let rows: Vec<_> = all
            .iter()
            .copied()
            .filter(|entry| {
                !entry.draft && lifecycle::section(&entry.life, lifecycle::now()) == section
            })
            .collect();
        let shelf = match section {
            Section::Snoozed => Some(0),
            Section::Settled => Some(1),
            _ => None,
        };
        if let Some(shelf) = shelf {
            // Settled stays visible as a drop target while dragging, even when empty.
            if rows.is_empty() && !(section == Section::Settled && dragging) {
                continue;
            }
            // The first shelf takes the remaining height above it, so the shelves sit at the
            // bottom of the sidebar as in T3 Code.
            if first_shelf {
                content = content.child(spacer().min_h(12.0));
                first_shelf = false;
            }
            let key = format!("shelf-{section:?}");
            let title = format!("{section:?}");
            let marker = format!("drop-{section:?}");
            let open = s.shelves[shelf];
            // Each shelf heading sticks to the bottom of the scroll area, Snoozed one heading
            // above Settled, so both stay whole above the footer when the list is longer than
            // the area (as at 1.25x zoom). Scrolled into view, a heading sits in its own place.
            // The block carries its rule and the sidebar fill, so rows never show through.
            let settled_below = section == Section::Snoozed
                && (dragging
                    || all.iter().any(|entry| {
                        !entry.draft
                            && lifecycle::section(&entry.life, lifecycle::now()) == Section::Settled
                    }));
            let inset = if settled_below { SHELF_HEADING } else { 0.0 };
            content = content.child(
                col()
                    .key(format!("shelf-block-{section:?}"))
                    .h(SHELF_HEADING)
                    .fill(p.bg)
                    .sticky_bottom(inset)
                    .child(dotted_rule(p))
                    .child(
                        row()
                            .key(&key)
                            .role("button")
                            .cursor(rune::Cursor::Pointer)
                            .label(format!("{title}, {} threads", rows.len()))
                            .focusable(true)
                            .h(32.0)
                            .pl(ROW_INSET)
                            .pr(8.0)
                            .gap(6.0)
                            .center_y()
                            .radius(4.0)
                            .hover(p.selected)
                            .on_drawn_layout(move |s: &mut State, rect| {
                                s.drop_rects.insert(marker.clone(), rect);
                            })
                            .on_press(move |s: &mut State| {
                                s.shelves[shelf] = !s.shelves[shelf];
                                s.checkpoint();
                            })
                            .child(pixel_label(&title, p.muted))
                            .child(spacer())
                            .child(pixel_text(rows.len().to_string(), p.muted))
                            .child(icon(if open { "chevron" } else { "open" }, p.muted)),
                    ),
            );
        } else if dragging {
            let marker = format!("drop-{section:?}");
            let selected = s.drag_target.is_some_and(|(to, _)| to == section);
            content = content.child(
                row()
                    .key(&marker)
                    .h(28.0)
                    .px(8.0)
                    .center_y()
                    .fill(if selected { p.coral_bg } else { p.selected })
                    .on_drawn_layout(move |s: &mut State, rect| {
                        s.drop_rects.insert(marker.clone(), rect);
                    })
                    .child(pixel_label(&format!("{section:?}"), p.muted)),
            );
        }
        let limit = if section == Section::Settled && s.search.text.is_empty() {
            s.settled_visible
        } else {
            usize::MAX
        };
        for entry in rows
            .iter()
            .copied()
            .filter(|entry| entry.visible)
            .take(limit)
        {
            content = content.child(sidebar_ui::row(s, p, entry));
        }
        if section == Section::Settled && s.shelves[1] && rows.len() > s.settled_visible {
            content = content.child(button(
                "settled-more",
                &format!("Show {} more", (rows.len() - s.settled_visible).min(25)),
                p,
                |s| s.settled_visible += 25,
            ));
        }
    }
    if captains.is_empty() && (all.is_empty() || !s.search.text.is_empty() && visible.is_empty()) {
        content = content.child(empty_list(
            s,
            p,
            "sidebar-empty",
            if s.search.text.is_empty() {
                "No threads in this workspace yet"
            } else {
                "No matching conversations"
            },
        ));
    }
    side = side.child(
        col()
            .key("sidebar-scroll")
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(content),
    );
    if let Some(row) = update_row(s, p) {
        side = side.child(row);
    }
    if let Some(notice) = &s.machines.auto_notice
        && s.machines.notice.as_ref() == Some(notice)
    {
        side = side.child(machine_notice(notice, p));
    }
    if let Some(error) = &s.lifecycle_error {
        side = side.child(label(error, SMALL, p.danger).wrap().padding(8.0));
    }
    if let Some(notice) = &s.undo {
        let count = notice.entries.len();
        let title = if count > 1 {
            format!("{}, {count} threads", notice.action.undo_label())
        } else {
            notice.action.undo_label().to_string()
        };
        // An outlined card with the shortcut as a keycap at its right edge.
        side = side.child(
            row()
                .key("thread-undo")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!("{title}, Ctrl+Z"))
                .focusable(true)
                .h(32.0)
                .mx(8.0)
                .mb(4.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(4.0)
                .fill(p.card)
                .border(1.0, p.frame)
                .hover(p.selected)
                .press(p.selected)
                .on_press(|s: &mut State| s.undo_thread_action())
                .child(
                    label(title, TextStyle::new(12.0, 450.0), p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(keycap_chip("Ctrl+Z", p)),
        );
    }
    // The footer is a row of quiet icons, as in T3 Code: no divider and no wordmark. Settings
    // holds preferences such as the theme.
    side = side.child(
        row()
            .key("sidebar-footer")
            .h(40.0)
            .pl(8.0)
            .gap(6.0)
            .center_y()
            .child(
                toggle_button(
                    "sidebar-settings",
                    "Settings",
                    "settings",
                    s.settings_menu,
                    p,
                    |s| s.settings_menu = !s.settings_menu,
                )
                .help("Settings"),
            )
            .child(
                toggle_button(
                    "show-archive",
                    if s.show_archive {
                        "Back to threads"
                    } else {
                        "Archive"
                    },
                    "archive",
                    s.show_archive,
                    p,
                    |s| s.show_archive = !s.show_archive,
                )
                .help("Archive"),
            )
            .child(spacer()),
    );
    // The waterline of the app: a sea strip the footer icons sit above. It bleeds to the
    // window's left edge (the workspace inset is 8 px), sits on the window's bottom edge like
    // the chat pane, and runs the sidebar's full width, with square corners.
    let bleed = width + 8.0;
    let dark = s.dark;
    side = side.child(rune::memo(
        "sidebar-sea",
        (bleed.to_bits(), dark),
        move || {
            el().key("sidebar-sea")
                .w(bleed)
                .h(SEA_HEIGHT)
                .ml(-8.0)
                .shape(scenery(&crate::horizon::strip(bleed, SEA_HEIGHT, dark)))
                .a11y_hidden()
        },
    ));
    side
}

/// Preferences behind the footer's gear: the "You" profile, background work, and the theme.
/// The background rows are KSG Code's `SettingsRow` with a `Switch` (see `captain_ui`); a
/// captain's routines live in its Brief.
fn settings_menu(s: &State, p: Palette) -> El {
    let mut menu = col()
        .key("settings-menu")
        .role("menu")
        // Up, Down, Home and End move through the rows, Return presses one, Escape closes.
        .focus_group()
        .label("Settings")
        .w(320.0)
        .max_h(460.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(4.0)
        .gap(1.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .child(
            menu_item(
                "default-harness",
                &format!("Default harness: {}", s.default_harness),
                "settings",
                p,
            )
            .on_press(|s: &mut State| s.cycle_default_harness()),
        )
        .child(captain_ui::profile_item(s, p))
        .child(el().h(0.5).mx(8.0).my(3.0).fill(p.line));
    for item in captain_ui::background_settings(s, p) {
        menu = menu.child(item);
    }
    if s.client.is_some() {
        menu = menu.child(
            menu_item("machine-settings", "Linked machines", "monitor", p).on_press(
                |s: &mut State| {
                    s.settings_menu = false;
                    s.machines.dialog = true;
                },
            ),
        );
    }
    menu = menu.child(el().h(0.5).mx(8.0).my(3.0).fill(p.line));
    menu.anchored_fit("sidebar-settings", rune::Side::Top, rune::Align::Start, 6.0)
        .modal(true, |s: &mut State| s.settings_menu = false)
        .child(
            menu_item(
                "theme-toggle",
                if s.dark { "Light theme" } else { "Dark theme" },
                if s.dark { "sun" } else { "moon" },
                p,
            )
            .on_press(|s: &mut State| {
                s.dark = !s.dark;
                s.settings_menu = false;
                s.checkpoint();
            }),
        )
}

/// Add or restore a project. Project ownership stays on threads, not sidebar folders.
fn project_switcher(_: &State, p: Palette) -> El {
    icon_button("project-switcher", "Add workspace", "folder-plus", p, |s| {
        s.open_picker(ProjectPicker::AddProject)
    })
}

fn menu_item(key: &str, title: &str, glyph: &str, p: Palette) -> El {
    menu_item_in(key, title, glyph, p, p.ink, p.muted)
}
/// A menu item that destroys something: the title and icon use the danger colour.
fn danger_menu_item(key: &str, title: &str, glyph: &str, p: Palette) -> El {
    menu_item_in(key, title, glyph, p, p.danger, p.danger)
}
/// A menu item with a value at its right edge in pixel type, such as "3 days".
fn menu_value_item(key: &str, title: &str, value: &str, glyph: &str, p: Palette) -> El {
    menu_item(key, title, glyph, p)
        .label(format!("{title}, {value}"))
        .child(pixel_text(value, p.muted))
}
fn menu_item_in(
    key: &str,
    title: &str,
    glyph: &str,
    p: Palette,
    ink: Color,
    icon_ink: Color,
) -> El {
    row()
        .key(key)
        .role("menuitem")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .h(28.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(4.0)
        // Keys and the pointer share one highlight, as in an NSMenu. Every row asks for focus;
        // the menu's modal gives it to the first, so arrows work as soon as the menu opens.
        .focus_highlight(p.selected)
        .autofocus()
        .child(el().size(ICON, ICON).center().child(icon(glyph, icon_ink)))
        .child(
            label(title, TextStyle::new(13.0, 450.0), ink)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
}
/// Clone a repository into a new folder and open it.
fn clone_dialog(s: &State, p: Palette) -> El {
    let projects = &s.projects;
    let cloning = projects.cloning();
    let field = field_fill(s, p);
    let mut card = col()
        .key("clone-dialog")
        .role("dialog")
        .label("Clone repository")
        .w(460.0)
        .padding(16.0)
        .gap(12.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .center_y()
                .child(
                    col()
                        .gap(2.0)
                        .child(label("Clone repository", DIALOG_TITLE, p.ink))
                        .child(label(
                            "Uses your existing git and GitHub CLI login.",
                            SMALL,
                            p.muted,
                        )),
                )
                .child(spacer())
                .child(icon_button(
                    "close-clone",
                    if cloning { "Stop the clone" } else { "Close" },
                    "close",
                    p,
                    |s| s.close_clone_dialog(),
                )),
        )
        .child(
            col()
                .gap(5.0)
                .child(pixel_label("Repository", p.muted))
                .child(
                    text_input(&projects.clone_source, BODY)
                        .key("clone-source")
                        .label("Repository URL or owner/repo")
                        .placeholder("owner/repo or https://github.com/owner/repo", p.muted)
                        .single_line()
                        .color(p.ink)
                        .caret(p.coral, p.coral_bg)
                        .w_full()
                        .padding(8.0)
                        .radius(4.0)
                        .fill(field)
                        .field_focus(&projects.clone_source, p)
                        .disabled(cloning)
                        .on_edit(|s: &mut State| &mut s.projects.clone_source)
                        .on_submit(|s: &mut State, _| s.start_clone()),
                ),
        )
        .child(
            col()
                .gap(5.0)
                .child(pixel_label("Clone into", p.muted))
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            row()
                                .h(32.0)
                                .px(8.0)
                                .center_y()
                                .grow(1.0)
                                .min_w(0.0)
                                .radius(4.0)
                                .fill(field)
                                .border(1.0, p.field_line)
                                .child(
                                    label(projects.clone_parent.display().to_string(), BODY, p.ink)
                                        .truncate()
                                        .min_w(0.0),
                                ),
                        )
                        .child(
                            secondary_button("clone-choose-parent", "Choose...", p, |_| {})
                                .on_press_tx(crate::projects::pick_clone_parent)
                                .disabled(cloning),
                        ),
                ),
        );
    if s.machines.selected != "local" {
        card = card
            .child(pixel_label("Destination path on this server", p.muted))
            .child(
                text_input(&s.machines.project_path, BODY)
                    .key("remote-clone-path")
                    .label("Host destination path")
                    .single_line()
                    .w_full()
                    .padding(8.0)
                    .radius(4.0)
                    .fill(field)
                    .field_focus(&s.machines.project_path, p)
                    .color(p.ink)
                    .on_edit(|s: &mut State| &mut s.machines.project_path),
            );
    }
    let hint = match s.clone_target() {
        Ok((_, target)) => format!("Creates {}", target.display()),
        Err(_) if projects.clone_source.text.trim().is_empty() => {
            "Shorthand clones with gh. Other URLs clone with git.".into()
        }
        Err(error) => error,
    };
    card = card.child(label(hint, SMALL, p.muted).wrap_anywhere());
    if !projects.clone_progress.is_empty() {
        card = card.child(
            row()
                .key("clone-progress")
                .gap(6.0)
                .center_y()
                .child(status_mark(Status::Running, p, 8.0))
                .child(
                    label(&projects.clone_progress, SMALL, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    if let Some(error) = &projects.clone_error {
        card = card.child(
            label(error, SMALL, p.danger)
                .key("clone-error")
                .wrap_anywhere(),
        );
    }
    let mut actions = row().gap(8.0).center_y().child(spacer());
    actions = actions.child(secondary_button(
        "clone-cancel",
        if cloning { "Stop" } else { "Cancel" },
        p,
        |s| s.close_clone_dialog(),
    ));
    actions = actions.child(if cloning {
        primary_button("clone-start", "Cloning...", p, |_| {}).disabled(true)
    } else {
        primary_button("clone-start", "Clone", p, |s| s.start_clone())
    });
    card = card.child(actions);
    el().key("clone-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.28 }))
        .center()
        .modal(false, |s: &mut State| {
            if !s.projects.cloning() {
                s.close_clone_dialog();
            }
        })
        .child(card)
}

/// The one notice for linked machines this desktop cannot reach: a framed card with a brass
/// mark and a pixel-type label. It goes away when the connections are ready again.
fn machine_notice(text: &str, p: Palette) -> El {
    outlined(
        row()
            .key("machine-unreachable")
            .role("status")
            .label(text)
            .mx(8.0)
            .mb(8.0)
            .px(8.0)
            .py(7.0)
            .gap(8.0)
            .radius(4.0)
            .fill(p.card)
            .child(el().size(8.0, 8.0).mt(3.0).fill(p.brass).a11y_hidden())
            .child(pixel_text(text, p.ink).wrap().grow(1.0).min_w(0.0)),
        p,
    )
}

/// Shown only while an update is on its way or waits for a restart. Downloading is passive;
/// the only action is one restart, disabled while any thread still has work.
/// Ported from KSG Code's sidebar/SidebarUpdatePill.tsx.
fn update_row(s: &State, p: Palette) -> Option<El> {
    use crate::update::Status;
    match s.updater.status() {
        Status::Idle if s.updater.notice().is_some() => {
            let notice = s.updater.notice().unwrap();
            Some(
                col()
                    .mx(8.0)
                    .mb(8.0)
                    .gap(4.0)
                    .child(label(truncate(notice, 300), SMALL, p.muted).wrap_anywhere())
                    .child(button("retry-update", "Retry update check", p, |s| {
                        s.updater.retry()
                    })),
            )
        }
        Status::Disabled | Status::Idle | Status::Checking => None,
        Status::Downloading { tag } => {
            let title = format!("Downloading update {tag}");

            Some(
                row()
                    .key("update-progress")
                    .role("status")
                    .label(title.clone())
                    .h(30.0)
                    .mx(8.0)
                    .mb(8.0)
                    .px(10.0)
                    .gap(8.0)
                    .center_y()
                    .radius(6.0)
                    .fill(p.sea_bg)
                    .child(icon("send", p.sea))
                    .child(label(title, TextStyle::new(12.5, 520.0), p.sea)),
            )
        }
        Status::Ready { update } => {
            let title = format!("Restart to update to {}", update.tag);
            let blocker = s.update_restart_blocker();
            let description = blocker.or(s.restart_notice.as_deref()).map(str::to_owned);
            let mut button = row()
                .key("update-restart")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(title.clone())
                .description(description.clone().unwrap_or_default())
                .h(30.0)
                .mx(8.0)
                .mb(8.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .fill(p.sea_bg)
                .disabled(blocker.is_some())
                .a11y_value(description.unwrap_or_default())
                .child(icon("send", p.sea))
                .child(label(title, TextStyle::new(12.5, 520.0), p.sea));
            if blocker.is_none() {
                button = button
                    .focusable(true)
                    .hover(p.selected)
                    .on_press(|s: &mut State| s.restart_for_update = true);
            }
            Some(button)
        }
    }
}

/// A sidebar section label: Departure Mono, uppercase, muted. Counts and buttons follow it.
fn section_title(title: &str, p: Palette) -> El {
    row()
        .h(30.0)
        .pl(ROW_INSET)
        .pr(4.0)
        .center_y()
        .child(pixel_label(title, p.muted))
        .child(spacer())
}
/// The dotted divider (2 px on, 2 px off) between sidebar sections.
/// A shelf heading's block: its dotted rule (1 px and 4 px either side) and the 32 px row.
const SHELF_HEADING: f32 = 41.0;
fn dotted_rule(p: Palette) -> El {
    el().h(1.0)
        .mx(ROW_INSET)
        .my(4.0)
        .border(1.0, p.line)
        .border_dash(2.0, 2.0)
        .a11y_hidden()
}
/// A selected sidebar row: the `row` fill and a 3 px coral bar on the left edge, the full
/// height of the row, with square ends. Hover is a fill only.
fn select_bar(item: El, selected: bool, p: Palette) -> El {
    if !selected {
        return item;
    }
    item.fill(p.row).corners([0.0, 4.0, 4.0, 0.0]).child(
        el().w(3.0)
            .fill(p.coral)
            .pin(Some(0.0), Some(0.0), None, Some(0.0))
            .a11y_hidden(),
    )
}
fn face_for(status: Status) -> crate::visuals::Face {
    use crate::visuals::Face;
    match status {
        Status::Ready => Face::Normal,
        Status::Running => Face::Determined,
        Status::Waiting => Face::Surprised,
        Status::Completed => Face::Happy,
        Status::Failed | Status::Paused => Face::Dizzy,
    }
}
/// The captain's face beside its name in the transcript: 24 pt of the upper bust, a window of
/// 24 cells that is one pixel a cell at 1x and two at 2x, with a 1 px frame. The 20 pt head crop
/// it replaces had 28 cells, so it cropped or padded at both scales and lost Ember's goggles.
fn speaker_avatar(s: &State, p: Palette, captain: usize) -> El {
    const SIZE: f32 = 24.0;
    let portrait = s.captains.get(captain).map(|c| c.portrait).unwrap_or(0);
    let portrait = portrait.min(crate::visuals::CAPTAIN_NAMES.len() - 1);
    let pixels = s
        .artwork
        .bust_window_pixels(portrait, crate::visuals::Face::Normal, SIZE);
    pixel_portrait_framed(s, &pixels, SIZE, p, None)
        .key(format!("speaker-avatar-{portrait}"))
        .a11y_hidden()
}
fn avatar(s: &State, p: Palette, captain: usize, size: f32, status: Status) -> El {
    let portrait = s.captains.get(captain).map(|c| c.portrait).unwrap_or(0);
    portrait_avatar(s, p, portrait, size, status)
}
fn portrait_avatar(s: &State, p: Palette, captain: usize, size: f32, status: Status) -> El {
    portrait_face(s, p, captain, size, status, None)
}
fn portrait_face(
    s: &State,
    p: Palette,
    captain: usize,
    size: f32,
    status: Status,
    accent: Option<Color>,
) -> El {
    let captain = captain.min(crate::visuals::CAPTAIN_NAMES.len() - 1);
    let pixels = s.artwork.face_pixels(captain, face_for(status), size);
    let portrait = pixel_portrait_framed(s, &pixels, size, p, accent)
        .key(format!("avatar-{captain}-{}", size as u32))
        .a11y_hidden();
    status_badge(p, portrait, size, status)
}

/// The portrait in the captain picker and the editor's grid. The head art is 40 cells, so 40
/// points is a whole number of device pixels per cell at every integer window scale.
const PICKER_PORTRAIT: f32 = 40.0;
/// Every machine uses the same portrait size, including before a captain is selected.
const CAPTAIN_SLOT: f32 = 44.0;
/// A sidebar portrait is 40 px inside its 44 px slot, so the pixel frame's shadow fits in it.
const CAPTAIN_FACE: f32 = 40.0;
/// Room for one badge at the end of the name line, kept even when there is none, so a
/// badge arriving never moves the name.
/// The widest a machine name and its connection note grow beside a workspace or a detail.
const MACHINE_CAP: f32 = 110.0;
/// The least room left for a machine's name beside the workspace. Under it the tag is the icon
/// alone, because a name cut to two letters and an ellipsis says nothing.
const MACHINE_NAME_ROOM: f32 = 32.0;
const BADGE_SLOT: f32 = 28.0;
/// What a captain row spends beside its text: left padding, the portrait slot, the gap and the
/// right padding.
const CAPTAIN_TEXT_INSET: f32 = 6.0 + CAPTAIN_SLOT + 10.0 + 8.0;
/// Room for "59m 59s" at the end of the status line.
const TIMER_SLOT: f32 = 48.0;
const CAPTAIN_NAME: TextStyle = TextStyle::new(14.0, 560.0).tracking(-0.15);
const CAPTAIN_STATUS: TextStyle = TextStyle::new(12.5, 460.0);

/// A sidebar portrait. It plays its status's animation strip through the shared stage in
/// `crate::portraits`, which also learns here whether the row is on screen. Under reduced
/// motion, or for a set without strips, it is the static face.
fn sidebar_portrait(
    s: &State,
    p: Palette,
    row: usize,
    portrait: usize,
    size: f32,
    status: Status,
) -> El {
    let portrait = portrait.min(crate::visuals::CAPTAIN_NAMES.len() - 1);
    let face = face_for(status);
    let pixels = s
        .portraits
        .pixels(portrait, face, size)
        .unwrap_or_else(|| s.artwork.face_pixels(portrait, face, size));
    pixel_portrait(s, &pixels, size, p)
        .key(format!("portrait-{row}"))
        .a11y_hidden()
        .on_visible(move |s: &mut State, on, tx| {
            s.portraits.set_visible(row, on);
            // The view doesn't read this; the next poll does.
            tx.live_only();
            tx.wake_after(std::time::Duration::ZERO);
        })
}

/// What a count badge counts. Brass is for what needs you, and the neutral card fill with ink
/// digits for what is unread. Coral stays for the one primary action. The crew line counts its
/// states as status squares with pixel labels, not badges.
#[derive(Clone, Copy)]
pub(crate) enum CountKind {
    NeedsYou,
    Unread,
}

fn count_pill(count: usize, kind: CountKind, p: Palette) -> El {
    let text = if count > 9 {
        "9+".into()
    } else {
        count.to_string()
    };
    let (fill, ink, frame) = match kind {
        CountKind::NeedsYou => (p.brass, p.on_brass, p.frame),
        CountKind::Unread => (p.card, p.ink, p.frame),
    };
    // A square with Departure Mono digits and a 1 px frame, no shadow.
    row()
        .h(18.0)
        .min_w(18.0)
        .px(3.0)
        .radius(2.0)
        .fill(fill)
        .border(1.0, frame)
        .center()
        .child(label(text, PIXEL, ink))
}

/// A row's hover and selection washes, after Grok Bot's bot list: ink at about 6% on hover
/// and 8% when selected. Rune blends in linear light, so dark mode needs far less alpha.
fn row_washes(_: &State, p: Palette) -> (Color, Color) {
    (p.selected, p.row)
}

/// A captain in the sidebar, laid out like Grok Bot's bot rows: a larger portrait, the name,
/// and one status line under it. Every part has a fixed place: the name truncates before
/// anything else, the badge and timer keep reserved slots, and the status word never
/// shrinks, so a status change never moves the name, the portrait or the rows around it.
fn captain_row(
    s: &State,
    p: Palette,
    thread: &Thread,
    item: El,
    status: Status,
    approval: bool,
    selected: bool,
) -> El {
    let id = thread.id;
    let name = thread
        .captain
        .and_then(|c| s.captains.get(c))
        .map(|c| c.name.as_str())
        .unwrap_or(&thread.title);
    let crew: Vec<&Thread> = s
        .threads
        .iter()
        .filter(|t| t.parent == Some(id) && t.archived() == s.show_archive)
        .collect();
    let family = || std::iter::once(thread).chain(crew.iter().copied());
    let running: Vec<&Thread> = family().filter(|t| t.status == Status::Running).collect();
    let requests = family().filter(|t| t.request.is_some()).count();
    let unread = family().filter(|t| t.unread()).count();
    let plural = |n: usize, one: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    let mut detail = match status {
        Status::Running => plural(running.len().max(1), "thread"),
        Status::Waiting if approval => "approval".into(),
        Status::Waiting if requests > 0 => "a question".into(),
        _ => {
            let mut parts = vec![];
            if thread.renamed {
                parts.push(thread.title.clone());
            }
            let count = s.captain_worker_count(id);
            if count > 0 {
                parts.push(format!("{count} crew"));
            }
            parts.join(", ")
        }
    };
    if let Some(workspace) = row_workspace(s, thread) {
        detail = if detail.is_empty() {
            workspace
        } else {
            format!("{detail}, {workspace}")
        };
    }
    let timer = (status == Status::Running)
        .then(|| running.iter().filter_map(|t| t.started).min())
        .flatten();
    let status_ink = status_ink(status, p);
    let mut status_line = row()
        .key(format!("captain-status-{id}"))
        .h(18.0)
        .gap(8.0)
        .center_y()
        .child(pixel_text(status.label(), status_ink).key(format!("captain-state-{id}")));
    if !detail.is_empty() {
        status_line = status_line.child(
            label(detail.clone(), CAPTAIN_STATUS, p.muted)
                .key(format!("captain-detail-{id}"))
                .truncate()
                .shrink()
                .min_w(workspace_floor(
                    sidebar_row_width(s) - CAPTAIN_TEXT_INSET,
                    // The status word, three gaps, the machine icon and the timer slot.
                    status.label().chars().count() as f32 * 6.6
                        + 24.0
                        + if multiple_machines(s) { ICON } else { 0.0 }
                        + if timer.is_some() { TIMER_SLOT } else { 0.0 },
                    &detail,
                ))
                .beside_tag(
                    multiple_machines(s),
                    sidebar_row_width(s) - CAPTAIN_TEXT_INSET,
                    status.label().chars().count() as f32 * 6.6
                        + 24.0
                        + ICON
                        + if timer.is_some() { TIMER_SLOT } else { 0.0 },
                    &detail,
                ),
        );
    }
    if multiple_machines(s) {
        status_line = status_line.child(machine_tag(
            s,
            p,
            &s.machines.selected,
            format!("captain-machine-{id}"),
            CAPTAIN_STATUS,
            room_for_machine_name(
                sidebar_row_width(s) - CAPTAIN_TEXT_INSET,
                status.label().chars().count() as f32 * 6.6
                    + 24.0
                    + ICON
                    + if timer.is_some() { TIMER_SLOT } else { 0.0 },
                &detail,
            ),
        ));
    }
    // The machine sits 8 px after the workspace; the free space comes after it.
    status_line = status_line.child(spacer());
    if let Some(start) = timer {
        status_line = status_line.child(
            label(
                elapsed_label(start.elapsed().as_secs()),
                PIXEL.features(TABULAR),
                p.muted,
            )
            .key(format!("captain-timer-{id}"))
            .w(TIMER_SLOT)
            .align(rune::Align::End),
        );
    }
    let badge = if requests > 0 || status == Status::Waiting {
        Some(
            count_pill(requests.max(1), CountKind::NeedsYou, p)
                .key(format!("captain-attention-{id}")),
        )
    } else if unread > 0 {
        Some(count_pill(unread, CountKind::Unread, p).key(format!("captain-unread-{id}")))
    } else {
        None
    };
    let mut badges = row()
        .key(format!("captain-badges-{id}"))
        .w(BADGE_SLOT)
        .h(18.0)
        .center_y()
        .child(spacer());
    if let Some(badge) = badge {
        badges = badges.child(badge);
    }
    let name_line = row()
        .h(20.0)
        .gap(6.0)
        .center_y()
        .child(
            label(name, CAPTAIN_NAME, p.ink)
                .key(format!("captain-name-{id}"))
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(badges);
    let size = CAPTAIN_FACE;
    let portrait = captain_portrait(s, thread.captain);
    let (hover, active) = row_washes(s, p);
    let curve = |seconds| rune::Curve::tween(seconds, rune::Easing::cubic(0.2, 0.0, 0.0, 1.0));
    let mut description = format!("{name}, {}", status.label());
    if !detail.is_empty() {
        description.push_str(&format!(", {detail}"));
    }
    if multiple_machines(s) {
        description.push_str(&format!(", {}", machine_caption(s, &s.machines.selected)));
    }
    if unread > 0 {
        description.push_str(&format!(", {unread} unread"));
    }
    select_bar(
        item.label(description)
            .radius(4.0)
            .fill(Color::TRANSPARENT)
            .hover(hover)
            .press(active)
            .hover_motion(curve(0.15), curve(0.2)),
        selected,
        p,
    )
    .child(
        row()
            .grow(1.0)
            .min_w(0.0)
            .h(CAPTAIN_SLOT + 12.0)
            .pl(6.0)
            .pr(8.0)
            .gap(10.0)
            .center_y()
            .child(
                el().key(format!("portrait-slot-{id}"))
                    .size(CAPTAIN_SLOT, CAPTAIN_SLOT)
                    .center()
                    .child(sidebar_portrait(s, p, id, portrait, size, status)),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(1.0)
                    .child(name_line)
                    .child(status_line),
            ),
    )
}

/// The corner mark that shows a portrait's status in pickers and headers.
fn status_badge(p: Palette, portrait: El, size: f32, status: Status) -> El {
    if matches!(status, Status::Running | Status::Waiting | Status::Failed) {
        // A small square in the panel color, with a 1 px frame, keeps the mark readable over
        // the artwork. Every live avatar sits on the panel (chat turns, the hero, the brief).
        let (backing, mark) = if size <= 16.0 {
            (8.0, 5.0)
        } else if size >= 30.0 {
            (13.0, 9.0)
        } else {
            (10.0, 7.0)
        };
        let offset = size - backing + 2.0;
        return el().size(size, size).child(portrait).child(
            el().absolute(offset, offset)
                .size(backing, backing)
                .radius(2.0)
                .fill(p.panel)
                .border(1.0, p.frame)
                .center()
                .child(status_mark(status, p, mark))
                .a11y_hidden(),
        );
    }
    portrait
}
fn captain_status_chip(status: Status, p: Palette) -> El {
    row()
        .center_y()
        .gap(5.0)
        .child(status_mark(status, p, 6.0))
        .child(pixel_text(status.label(), status_ink(status, p)))
}
/// A sidebar row's status, and whether it waits on an approval: a pending request, or a
/// worker's approval, outranks the run. A captain's home stands for its whole crew.
fn row_status(s: &State, thread: &Thread) -> (Status, bool) {
    let id = thread.id;
    let confirm = |t: &Thread| t.request.as_ref().is_some_and(|r| r["method"] == "confirm");
    let approval = confirm(thread)
        || s.threads
            .iter()
            .any(|child| child.parent == Some(id) && confirm(child));
    let status = if approval || thread.request.is_some() {
        Status::Waiting
    } else if s.is_captain_home(thread) {
        s.home_status(id)
    } else {
        thread.status
    };
    (status, approval)
}

/// The portrait a captain wears; threads without a profile wear the first.
fn captain_portrait(s: &State, captain: Option<usize>) -> usize {
    captain
        .and_then(|c| s.captains.get(c))
        .map(|c| c.portrait)
        .unwrap_or(0)
}

/// Whether the sidebar is on screen, as `view` decides it.
fn sidebar_docked(s: &State) -> bool {
    let compact = s.viewport_width < 760.0;
    (s.sidebar && !compact) || (compact && s.compact_sidebar)
}

/// Advances the sidebar's portrait animations. Called from the app's poll; the answer is
/// when to wake for the next frame, or None when no portrait plays.
pub(crate) fn tick_portraits(
    s: &mut State,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    let rows: Vec<_> = if sidebar_docked(s) {
        s.sidebar_captains()
            .into_iter()
            .map(|i| {
                let t = &s.threads[i];
                let face = face_for(row_status(s, t).0);
                (t.id, captain_portrait(s, t.captain), face)
            })
            .collect()
    } else {
        vec![]
    };
    let mut rows = rows;
    let t = s.current();
    if let Some(captain) = t
        .captain
        .filter(|_| t.messages.is_empty() && t.tools.is_empty())
    {
        rows.push((
            HERO_PORTRAIT_ROW,
            captain_portrait(s, Some(captain)),
            face_for(shown_status(s)),
        ));
    }
    s.portraits.tick(now, rows)
}

fn thread_row(s: &State, index: usize, p: Palette, nested: bool) -> El {
    let thread = &s.threads[index];
    let id = thread.id;
    let (row_status, approval) = row_status(s, thread);
    let captain_home = s.is_captain_home(thread);
    let target = RowTarget::Thread(id);
    let selected = id == s.current().id || s.selected_threads.contains(&id);
    let section = s.thread_section(thread, lifecycle::now());
    let hovered = s.hovered_row == Some(target);
    let draft = thread.is_draft();
    let at = lifecycle::now();
    let state_label = if approval {
        "Approval"
    } else if thread.request.is_some() {
        "Input"
    } else if row_status == Status::Running {
        "Working"
    } else if thread.status == Status::Waiting {
        "Waiting"
    } else if thread.status == Status::Failed {
        "Failed"
    } else if thread.life.woke(at) {
        "Woke"
    } else if thread.unread() {
        "Done"
    } else {
        ""
    };
    let workspace = row_workspace(s, thread);
    let mut item = row()
        .key(if s.client.is_some() {
            format!("machine-thread-{}-{id}", s.machines.selected)
        } else {
            format!("thread-{id}")
        })
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!(
            "{}{}{}, {}",
            thread.title,
            workspace
                .as_ref()
                .map(|w| format!(", {w}"))
                .unwrap_or_default(),
            if multiple_machines(s) {
                format!(", {}", machine_caption(s, &s.machines.selected))
            } else {
                String::new()
            },
            if approval {
                "needs approval"
            } else if nested && (thread.request.is_some() || thread.status == Status::Waiting) {
                "needs input"
            } else if nested && thread.unread() && !thread.actionable() {
                "unread result"
            } else if state_label.is_empty() {
                row_status.label()
            } else {
                state_label
            },
        ))
        .focusable(true)
        .w_full()
        .radius(4.0)
        .fill(Color::TRANSPARENT)
        .hover(p.selected)
        .on_press_tx(move |s: &mut State, tx| {
            s.click_thread(index, tx.mods());
        })
        .on_double_press(move |s: &mut State| s.begin_rename(id))
        .on_key(move |s: &mut State, ev, _| {
            if ev.key == rune::Key::Function(2) {
                s.begin_rename(id);
                true
            } else {
                false
            }
        })
        .on_drawn_layout(move |s: &mut State, rect| {
            s.drop_rects.insert(format!("thread-{id}"), rect);
        })
        .on_secondary_press(move |s: &mut State, at, _| {
            s.row_menu = Some(target);
            s.row_menu_at = Some((at.x, at.y));
        })
        .on_hover_change(move |s: &mut State, hovered, _| {
            if hovered {
                s.hovered_row = Some(target);
            } else if s.hovered_row == Some(target) {
                s.hovered_row = None;
            }
        });
    if !nested && !captain_home && thread.machine == "local" {
        item = item.on_drag(move |s: &mut State, drag, _| s.drag_pointer(id, drag));
    }
    if captain_home && !nested {
        return captain_row(s, p, thread, item, row_status, approval, selected);
    }
    let item = select_bar(item, selected, p);

    // The mark only shows what deserves a glance: work in progress, something that needs the
    // user, a failure, an unread result, a draft or a pin. Quiet rows leave it out.
    let dot = |ink: Color| el().size(6.0, 6.0).fill(ink);
    let mark = if approval || thread.request.is_some() || row_status == Status::Waiting {
        Some(dot(p.brass))
    } else if row_status == Status::Running || row_status == Status::Failed {
        Some(status_mark(row_status, p, 7.0))
    } else if thread.unread() || thread.life.woke(at) {
        Some(dot(p.sea))
    } else if draft {
        Some(icon("edit", p.muted))
    } else if thread.pinned() && !nested {
        Some(icon("pin", p.muted))
    } else if let Some(status) = nested
        .then(|| crew_ui::worker_task_mark(s, thread))
        .flatten()
    {
        // Interrupted and queued workers carry their chip's mark, so they never look finished.
        Some(status_mark(status, p, 7.0).key(format!("worker-task-mark-{id}")))
    } else {
        None
    }
    .map(|mark| {
        let mark = el().size(14.0, 16.0).center().child(mark.a11y_hidden());
        if nested && (thread.actionable() || thread.unread()) {
            mark.key(format!("worker-alert-{id}"))
        } else {
            mark
        }
    });
    let quiet = !selected && !thread.unread() && !thread.actionable();
    let shelved = matches!(section, Section::Snoozed | Section::Settled) && !captain_home;
    let title = label(
        if draft && !thread.draft.text.is_empty() {
            thread.draft.text.lines().next().unwrap_or(&thread.title)
        } else {
            &thread.title
        },
        if nested || shelved {
            LABEL
        } else {
            TextStyle::new(14.0, 520.0).tracking(-0.10)
        },
        // Settled, snoozed and quiet worker rows recede.
        if quiet && (nested || shelved) {
            p.muted
        } else {
            p.ink
        },
    )
    .truncate()
    .grow(1.0)
    .shrink()
    .min_w(0.0);

    if nested {
        let mut line = row()
            .h(CREW_ROW)
            .w_full()
            .pl(10.0)
            .pr(8.0)
            .gap(8.0)
            .center_y();
        // Every worker row has the status slot, empty or not, so all titles share one left edge
        // and none looks nested under the row above.
        line = line.child(mark.unwrap_or_else(|| el().size(14.0, 16.0)));
        return item.child(line.child(title));
    }

    // The status slot: hover actions, then what needs attention, then how old the row is.
    // Hover puts Snooze and Settle where the age sat; everything else is in the right-click menu.
    let mut slot = row().gap(2.0).center_y().h(20.0);
    if hovered && !captain_home && thread.machine == "local" {
        if draft || !thread.draft.text.is_empty() {
            slot = slot.child(icon_button(
                &format!("thread-discard-{id}"),
                "Discard draft",
                "close",
                p,
                move |s| s.discard_draft(id),
            ));
        }
        if !draft {
            let text_action = |key: String, title: &str, glyph: &str| {
                row()
                    .key(key)
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label(title)
                    .focusable(true)
                    .h(22.0)
                    .px(6.0)
                    .gap(4.0)
                    .center_y()
                    .radius(4.0)
                    .hover(p.line)
                    .child(tool_icon(glyph, p.muted))
                    .child(label(title, SMALL, p.muted))
            };
            match section {
                Section::Snoozed => {
                    slot = slot.child(
                        text_action(format!("thread-park-{id}"), "Wake", "clock")
                            .on_press(move |s: &mut State| s.unsnooze_thread(id)),
                    )
                }
                Section::Settled => {
                    slot = slot.child(
                        text_action(format!("thread-park-{id}"), "Un-settle", "check")
                            .on_press(move |s: &mut State| s.settle_thread(id))
                            .disabled(s.lifecycle_busy(id)),
                    )
                }
                _ => {
                    slot = slot
                        .child(
                            icon_button(
                                &format!("thread-snooze-{id}"),
                                "Snooze",
                                "clock",
                                p,
                                move |s| s.open_snooze(id),
                            )
                            .disabled(!s.can_snooze(id)),
                        )
                        .child(
                            text_action(format!("thread-park-{id}"), "Settle", "check")
                                .on_press(move |s: &mut State| s.settle_thread(id))
                                .disabled(s.lifecycle_busy(id)),
                        )
                }
            }
        }
    } else if thread.life.woke(at) {
        slot = slot.child(
            pixel_text("Woke", p.sea)
                .key(format!("thread-woke-{id}"))
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .on_press(move |s: &mut State| {
                    s.threads[index].life.last_visited_at = Some(lifecycle::now());
                    s.checkpoint();
                }),
        );
    } else if !state_label.is_empty() {
        let ink = if approval || row_status == Status::Waiting {
            p.brass
        } else if thread.status == Status::Failed {
            p.danger
        } else {
            p.sea
        };
        // A working row counts up, as T3 Code's status slot does.
        let text = match (state_label, thread.started) {
            ("Working", Some(start)) => {
                format!("Working {}", elapsed_label(start.elapsed().as_secs()))
            }
            _ => state_label.to_string(),
        };
        slot = slot.child(label(text, PIXEL.features(TABULAR), ink));
    } else {
        let time = match section {
            Section::Snoozed => Some(lifecycle::wake_label(
                thread.life.snoozed_until.unwrap_or(0),
                at,
            )),
            Section::Settled => relative_time(thread.life.settled_at.unwrap_or(0) / 1000),
            _ => relative_time(thread.updated),
        };
        if let Some(time) = time {
            slot = slot.child(pixel_text(time, p.muted).key(format!("thread-age-{id}")));
        }
    }
    if thread.machine != "local" {
        slot = slot.child(icon("cloud", p.muted));
    }

    let project = s.project(thread.project);
    // Pins and drafts lead the line, as T3 Code puts its pin before the project icon.
    let lead = |line: El| match &mark {
        Some(_) if thread.pinned() || draft => line.child(mark.clone().unwrap()),
        _ => line,
    };

    // Snoozed and settled rows stay one line inside their shelves.
    if shelved {
        // The title truncates 8 px before the status slot. The slot never shrinks and keeps its
        // content at its right edge, so an age is a whole word, not a sliver.
        let line = lead(
            row()
                .h(34.0)
                .grow(1.0)
                .min_w(0.0)
                .px(ROW_INSET)
                .gap(8.0)
                .center_y(),
        )
        .child(project_tile(s, p, project, 16.0))
        .child(title)
        .child(slot.min_w(SHELF_SLOT).justify_end());
        return item.child(line);
    }

    // A thread card, as T3 Code lays it out: the project's icon and name with the status on
    // the right, then the title.
    let top = lead(row().h(18.0).gap(8.0).center_y())
        .child(project_tile(s, p, project, 16.0))
        .child(
            // The workspace and, on a linked machine, the machine: two labels, not one joined.
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(
                    label(
                        project
                            .map(|project| project.name.clone())
                            .unwrap_or_else(|| "No workspace".into()),
                        CARD_META,
                        p.muted,
                    )
                    .key(format!("thread-workspace-{id}"))
                    .truncate()
                    .shrink()
                    .min_w(0.0)
                    .beside_tag(
                        multiple_machines(s),
                        sidebar_row_width(s) - 2.0 * ROW_INSET,
                        16.0 + ICON
                            + slot_width(match state_label {
                                "" => None,
                                "Working" => Some("Working 59m 59s"),
                                other => Some(other),
                            })
                            + 32.0,
                        project.map_or("No workspace", |project| project.name.as_str()),
                    ),
                )
                .child(if multiple_machines(s) {
                    let workspace = project.map_or("No workspace", |project| project.name.as_str());
                    // The age or the status word, widest while it counts up.
                    let slot = match state_label {
                        "" => None,
                        "Working" => Some("Working 59m 59s"),
                        other => Some(other),
                    };
                    machine_tag(
                        s,
                        p,
                        &s.machines.selected,
                        format!("thread-machine-{id}"),
                        CARD_META,
                        room_for_machine_name(
                            sidebar_row_width(s) - 2.0 * ROW_INSET,
                            16.0 + ICON + slot_width(slot) + 32.0,
                            workspace,
                        ),
                    )
                } else {
                    el()
                })
                // The machine sits 8 px after the workspace; the free space comes after it.
                .child(spacer()),
        )
        .child(slot);
    item.child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .px(ROW_INSET)
            .pt(9.0)
            .pb(10.0)
            .gap(5.0)
            .child(top)
            .child(row().child(title)),
    )
}

/// The least width of a shelved row's status slot: room for "10m" and "1d".
const SHELF_SLOT: f32 = 40.0;
/// Tile colours for projects without an icon. Each project keeps one by its name.
const PROJECT_HUES: [&str; 6] = [
    "#6E8FE8", "#4FA58A", "#9A7BDD", "#C99A32", "#CF6A8C", "#3E9DBF",
];

/// A project's icon, as T3 Code shows each project's favicon: the repository's own PNG when
/// it has one, otherwise its initial on a tile in the project's colour.
fn project_tile(
    s: &State,
    p: Palette,
    project: Option<&crate::projects::Project>,
    size: f32,
) -> El {
    // A pixel-framed square. At 16 px the frame is 1 px, so the initial still has room.
    let tile = el()
        .size(size, size)
        .radius(2.0)
        .clip_rounded(2.0)
        .border(1.0, p.frame)
        .a11y_hidden();
    let Some(project) = project else {
        return tile.center().child(icon("thread", p.muted));
    };
    if let Some(path) = &project.icon {
        let handle = s.attachment_cache.file(path.clone(), size, 2.0);
        if rune::is_headless() {
            let _ = handle.wait();
        }
        return tile.pixels(handle.pixels());
    }
    let hash = project.name.bytes().fold(0usize, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as usize)
    });
    let hue = color(PROJECT_HUES[hash % PROJECT_HUES.len()]);
    let initial = project
        .name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    tile.center()
        .fill(hue.alpha(if s.dark { 0.45 } else { 0.35 }))
        .child(label(initial, PIXEL, p.ink))
}

/// A row's workspace name, unless the Workspace filter already names that one workspace.
fn row_workspace(s: &State, thread: &Thread) -> Option<String> {
    let named_by_filter = workspace_named_by_filter(s, &s.machines.selected, thread.project);
    (!named_by_filter).then(|| {
        s.project(thread.project)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "No workspace".into())
    })
}
fn workspace_named_by_filter(s: &State, server: &str, project: Option<usize>) -> bool {
    s.scope_additional.is_empty()
        && s.machines.scope_remote.is_empty()
        && match s.project_scope {
            ProjectScope::Project(id) => {
                project == Some(id)
                    && s.machines
                        .scope_server
                        .as_deref()
                        .unwrap_or(&s.machines.selected)
                        == server
            }
            ProjectScope::NoProject => project.is_none(),
            ProjectScope::All => false,
        }
}
/// A machine's name for labels and tooltips. Whether this desktop can reach it is not part of
/// a row; it shows as one notice and in the open thread (`State::unreachable_machines`).
fn machine_caption(s: &State, server: &str) -> String {
    s.machine_label(server)
}

/// Whether more than one machine is linked, so rows say which machine they belong to.
fn multiple_machines(s: &State) -> bool {
    s.client
        .as_ref()
        .is_some_and(|client| !client.linked.is_empty())
}

/// The least width a workspace name keeps beside a status word: ten characters of 12 px text.
const WORKSPACE_FLOOR: f32 = 64.0;

/// A sidebar row's width in points, from the sidebar setting and the last window width.
fn sidebar_row_width(s: &State) -> f32 {
    let sidebar = if s.viewport_width < 1050.0 {
        s.sidebar_width.min(216.0)
    } else {
        s.sidebar_width
    };
    sidebar - 8.0
}

/// About how wide a status or age slot is: Departure Mono 11 px cells of 6.6 pt, with room for
/// "1m" when the row shows nothing.
fn slot_width(text: Option<&str>) -> f32 {
    text.map_or(14.0, |text| text.chars().count() as f32 * 6.6)
}

/// About how wide `text` is at the sidebar's 12.5 px.
pub(super) fn text_width(text: &str) -> f32 {
    text.chars().count() as f32 * 6.2
}

/// The workspace floor for a row whose other items take `taken` of its `row` width: the full
/// ten characters when the row allows, and only what is left in a narrow sidebar. A name that
/// is shorter than the floor needs only its own width, so a short name leaves room for the
/// machine's. Nothing is held back from the status word, the machine icon or the timer, so a
/// row never overflows.
fn workspace_floor(row: f32, taken: f32, workspace: &str) -> f32 {
    (row - taken)
        .clamp(0.0, WORKSPACE_FLOOR)
        .min(text_width(workspace))
}

/// A linked machine on a row: its icon, then its name. The name gives way first and can drop
/// out, which leaves the icon alone. Rows never show whether the machine is reachable.
fn machine_tag(
    s: &State,
    p: Palette,
    server: &str,
    key: String,
    style: TextStyle,
    show_name: bool,
) -> El {
    let caption = machine_caption(s, server);
    let mut tag = row()
        .key(key)
        .role("img")
        .label(caption.clone())
        .help(caption)
        // 4 px between the icon and the name. The name is left out when the row lacks room for
        // it (see `room_for_machine_name`), so the 8 px between the row's groups never shrinks.
        .gap(4.0)
        .center_y()
        .shrink()
        .min_w(ICON)
        .max_w(MACHINE_CAP)
        .child(icon("monitor", p.faint));
    // A name with no room would show as a bare ellipsis, so it is left out instead.
    if show_name {
        tag = tag.child(
            label(s.machine_label(server), style, p.faint)
                .truncate()
                .shrink()
                .min_w(0.0)
                .a11y_hidden(),
        );
    }
    tag
}

/// A workspace name before a machine tag. When it is estimated to truncate it grows into the
/// row's real leftover space and keeps 2 px more after its ellipsis, 10 px from the tag in
/// all; otherwise it keeps its own width and the tag follows 8 px later. `taken` counts the
/// row's other items and the machine icon, as `room_for_machine_name` does.
pub(super) trait BesideTag {
    fn beside_tag(self, machine: bool, row: f32, taken: f32, workspace: &str) -> Self;
}
impl BesideTag for El {
    fn beside_tag(self, machine: bool, row: f32, taken: f32, workspace: &str) -> El {
        if machine && text_width(workspace) > row - taken {
            self.grow(1.0).mr(2.0)
        } else {
            self
        }
    }
}

/// Whether a row has 36 points to spare for a machine name after its other items and the
/// workspace, which takes its text (estimated at 6.2 points a character) or its floor,
/// whichever is wider.
fn room_for_machine_name(row: f32, taken: f32, workspace: &str) -> bool {
    let text = text_width(workspace);
    row - taken - text.max(workspace_floor(row, taken, workspace)) >= MACHINE_NAME_ROOM
}

fn relative_time(updated: u64) -> Option<String> {
    if updated == 0 {
        return None;
    }
    let seconds = crate::projects::now().saturating_sub(updated);
    Some(match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    })
}

/// The Captain screen: a centred modal that introduces each captain by portrait, role, state,
/// where they already work and how they are briefed, then offers a new one.
fn captain_picker(s: &State, p: Palette) -> El {
    let target = s.project(s.projects.captain_target);
    let workspace = target
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "No workspace".into());
    let mut card = popover("captain-picker", "Choose a captain", s, p)
        .role("dialog")
        .w(560.0)
        .max_h((s.viewport_height * 0.72).max(360.0))
        .padding(6.0)
        .radius(6.0)
        .on_press(|_: &mut State| {})
        .child(
            col()
                .px(10.0)
                .pt(12.0)
                .pb(10.0)
                .gap(4.0)
                .child(label("Choose a captain", DIALOG_TITLE, p.ink))
                .child(
                    row()
                        .gap(6.0)
                        .center_y()
                        .child(label("They will coordinate work in", SMALL, p.muted))
                        .child(project_tile(s, p, target, 14.0))
                        .child(label(workspace, TextStyle::new(12.0, 560.0), p.ink)),
                ),
        );
    // Up, Down, Home and End move through the captains; Return opens one.
    let mut list = col()
        .role("list")
        .label("Captains")
        .focus_group()
        .gap(2.0)
        .min_h(0.0)
        .shrink()
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    for captain in 0..s.captains.len() {
        let state = s.captain_status(captain);
        let open = s.current().captain == Some(captain) && s.is_captain_home(s.current());
        let hovered = s.hovered_captain == Some(captain);
        let profile = &s.captains[captain];
        // The workspaces this captain already has a home in.
        let mut homes: Vec<String> = s
            .threads
            .iter()
            .filter(|t| t.captain == Some(captain) && s.is_captain_home(t) && !t.archived())
            .map(|t| {
                s.project(t.project)
                    .map(|project| project.name.clone())
                    .unwrap_or_else(|| "No workspace".into())
            })
            .collect();
        homes.sort();
        homes.dedup();
        let brief = profile.instructions.lines().find(|l| !l.trim().is_empty());
        let mut text = col().gap(2.0).grow(1.0).min_w(0.0).child(
            row()
                .gap(8.0)
                .center_y()
                .child(label(&profile.name, CAPTAIN_SERIF, p.ink))
                .child(label(&profile.role, SMALL, p.muted))
                .child(spacer())
                .child(captain_status_chip(state, p)),
        );
        if let Some(brief) = brief {
            text = text.child(label(brief.trim(), SMALL, p.muted).truncate());
        }
        if !homes.is_empty() {
            text = text
                .child(label(format!("Works in {}", homes.join(", ")), SMALL, p.faint).truncate());
        }
        let mut item = row()
            .key(format!("choose-captain-{captain}"))
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("Open Captain {}, {}", profile.name, state.label()))
            .focusable(true)
            .autofocus()
            .focus_fill(p.selected)
            .px(10.0)
            .py(10.0)
            .gap(14.0)
            .radius(4.0)
            .fill(if open { p.selected } else { Color::TRANSPARENT })
            .hover(p.selected)
            .center_y()
            .on_hover_change(move |s: &mut State, on, _| {
                if on {
                    s.hovered_captain = Some(captain);
                } else if s.hovered_captain == Some(captain) {
                    s.hovered_captain = None;
                }
            })
            .on_press(move |s: &mut State| s.open_captain(captain))
            .child(avatar(s, p, captain, PICKER_PORTRAIT, state))
            .child(text);
        item = item.child(
            icon_button(
                &format!("edit-captain-{captain}"),
                &format!("Rename or edit {}", profile.name),
                "edit",
                p,
                move |s| {
                    s.captain_picker = false;
                    s.edit_captain(captain);
                },
            )
            .opacity(if hovered || open { 1.0 } else { 0.0 }),
        );
        list = list.child(item);
    }
    card = card
        .child(list)
        .child(el().h(0.5).w_full().my(4.0).fill(p.line))
        .child(
            row()
                .h(40.0)
                .pl(2.0)
                .pr(8.0)
                .gap(16.0)
                .center_y()
                .child(
                    menu_item("picker-new-captain", "New captain", "plus", p)
                        .on_press(|s: &mut State| s.begin_new_captain()),
                )
                .child(spacer())
                .child(keycap("Esc", "Close", p)),
        );
    modal_frame(
        "captain-picker-backdrop",
        s,
        |s| s.cancel_captain_flow(),
        card,
    )
}

/// A portrait's cell in the captain editor's grid: the 40 pt face and 4 pt each side.
const PICKER_CELL: f32 = 48.0;
fn captain_editor(s: &State, p: Palette, height: f32) -> El {
    let editor = s.captain_editor.as_ref().unwrap();
    let editing = editor.editing.is_some();
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
                    .focusable(true)
                    .size(PICKER_CELL, PICKER_CELL)
                    .center()
                    .radius(4.0)
                    .hover(p.selected)
                    .on_press(move |s: &mut State| {
                        s.captain_editor.as_mut().unwrap().portrait = portrait;
                    })
                    // The selected portrait's own 2 px frame and hard shadow turn coral. There is
                    // no ring or wash around it, so all twelve keep the same outer bounds.
                    .child(
                        portrait_face(
                            s,
                            p,
                            portrait,
                            PICKER_PORTRAIT,
                            Status::Ready,
                            selected.then_some(p.coral),
                        )
                        // The editor's header shows the chosen portrait at the same size.
                        .key(format!("captain-portrait-face-{portrait}")),
                    ),
            );
        }
        portraits = portraits.child(line);
    }
    let captain_project = s.captain_project.unwrap_or(s.current().project);
    let mut body = col()
        .key("captain-form-body")
        .gap(12.0)
        .min_h(0.0)
        .max_h((height - 180.0).max(180.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .gap(5.0)
                .child(pixel_label("Workspace", p.muted))
                .child(
                    row()
                        .key("captain-project-choice")
                        .role("button")
                        .cursor(rune::Cursor::Pointer)
                        .label(format!(
                            "Workspace: {}",
                            s.project(captain_project)
                                .map(|p| p.name.as_str())
                                .unwrap_or("No workspace")
                        ))
                        .focusable(!editing)
                        // The same height as the text inputs below it.
                        .h(37.0)
                        .px(8.0)
                        .gap(10.0)
                        .center_y()
                        .radius(4.0)
                        .fill(field)
                        .border(1.0, p.field_line)
                        .opacity(if editing { 0.6 } else { 1.0 })
                        .on_press(move |s: &mut State| {
                            if s.captain_editor
                                .as_ref()
                                .is_some_and(|e| e.editing.is_none())
                            {
                                s.open_picker(ProjectPicker::AssignCaptain);
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
                        .child(pixel_label("Portrait", p.muted))
                        .child(label(crate::visuals::PORTRAIT_NAMES[chosen], SMALL, p.ink)),
                )
                .child(portraits),
        )
        .child(
            col().gap(5.0).child(pixel_label("Name", p.muted)).child(
                text_input(&editor.name, BODY)
                    .key("captain-name")
                    .label("Captain name")
                    .placeholder("e.g. Atlas", p.muted)
                    .single_line()
                    .color(p.ink)
                    .caret(p.coral, p.coral_bg)
                    .w_full()
                    .padding(8.0)
                    .radius(4.0)
                    .fill(field)
                    .field_focus(&editor.name, p)
                    .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().name),
            ),
        )
        .child(
            col()
                .gap(5.0)
                .child(pixel_label("Role (optional)", p.muted))
                .child(
                    text_input(&editor.role, BODY)
                        .key("captain-role")
                        .label("Captain role")
                        .placeholder("e.g. Code review", p.muted)
                        .single_line()
                        .color(p.ink)
                        .caret(p.coral, p.coral_bg)
                        .w_full()
                        .padding(8.0)
                        .radius(4.0)
                        .fill(field)
                        .field_focus(&editor.role, p)
                        .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().role),
                ),
        )
        .child(
            col()
                .gap(5.0)
                .child(pixel_label("Instructions for Pi", p.muted))
                .child(
                    text_input(&editor.instructions, BODY)
                        .key("captain-instructions")
                        .label("Captain instructions")
                        .placeholder("What should this captain focus on? Optional.", p.muted)
                        .color(p.ink)
                        .caret(p.coral, p.coral_bg)
                        .w_full()
                        .min_h(76.0)
                        .max_h(140.0)
                        .padding(8.0)
                        .radius(4.0)
                        .fill(field)
                        .field_focus(&editor.instructions, p)
                        .on_edit(|s: &mut State| {
                            &mut s.captain_editor.as_mut().unwrap().instructions
                        }),
                ),
        )
        .child(
            label(
                "The captain and its crew use these instructions and Pi's configured CPA model.",
                SMALL,
                p.muted,
            )
            .wrap(),
        );
    if let Some(error) = &editor.error {
        body = body.child(label(error, SMALL, p.danger).wrap());
    }
    let title = if editing {
        "Edit captain"
    } else {
        "New captain"
    };
    let card = col()
        .key("captain-editor")
        .role("dialog")
        .label(title)
        .w(480.0)
        .max_h((height - 32.0).max(240.0))
        .padding(18.0)
        .gap(14.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .center_y()
                .gap(12.0)
                .child(portrait_avatar(s, p, editor.portrait, 40.0, Status::Ready))
                .child(
                    col()
                        .gap(3.0)
                        .child(label(title, DIALOG_TITLE, p.ink))
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
                .child(if profile.validate().is_ok() {
                    primary_button(
                        "save-captain",
                        if editing {
                            "Save changes"
                        } else {
                            "Create captain"
                        },
                        p,
                        |s| s.save_captain(),
                    )
                } else {
                    button(
                        "save-captain",
                        if editing {
                            "Save changes"
                        } else {
                            "Create captain"
                        },
                        p,
                        |_| {},
                    )
                    .h(32.0)
                    .px(14.0)
                    .opacity(0.5)
                    .disabled(true)
                }),
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

const TAB_GAP: f32 = 4.0;
/// A tab is 32 pt tall, so in the 40 pt top strip its bottom frame sits 4 pt above the chat
/// pane's top rule, the same distance as its top edge sits below the window edge.
const TAB_HEIGHT: f32 = 32.0;
/// Below this a tab drops its workspace prefix and keeps the title.
const TAB_PREFIX_WIDTH: f32 = 170.0;
/// The narrowest a tab gets before the strip scrolls.
const TAB_MIN_WIDTH: f32 = 120.0;
/// Tabs are packed from the left and stop growing here, so a few tabs do not sit in wide empty
/// slots.
const TAB_MAX_WIDTH: f32 = 260.0;
/// Inter 13/520, the title in a tab.
const TAB_TITLE: TextStyle = TextStyle::new(13.0, 520.0);
/// A tab's look. The active tab is a `card` with a pixel frame (no shadow) and a 2 px coral bar
/// along the bottom inside it. Others have no fill.
fn tab_frame(tab: El, active: bool, p: Palette) -> El {
    if !active {
        return tab.hover(p.selected);
    }
    tab.fill(p.card).border(2.0, p.frame).child(
        el().h(2.0)
            .fill(p.coral)
            .pin(Some(2.0), None, Some(2.0), Some(2.0))
            .a11y_hidden(),
    )
}

/// The new-tab plus: a framed 24 px square.
fn new_tab_button(p: Palette) -> El {
    framed(
        icon_button("new-tab", "New thread", "plus", p, |_| {})
            .fill(p.card)
            .on_press_tx(|s: &mut State, tx| s.new_thread_click(tx.mods().shift)),
        p,
    )
    .mr(2.0)
}

fn header(s: &State, p: Palette, narrow: bool, tabs_width: f32) -> El {
    let mut tabs = row()
        .key("thread-tabs")
        .min_w(0.0)
        .grow(1.0)
        .h(36.0)
        .center_y()
        .gap(TAB_GAP)
        .scroll_x()
        // Tabs hidden past an edge fade out, so a clipped strip reads as scrollable.
        .scroll_edge_fade(12.0);
    // Tabs share the strip evenly, down to a minimum. Only once every tab is at the minimum
    // does the strip scroll.
    let count = s
        .tabs
        .iter()
        .filter(|&&id| s.threads.iter().any(|t| t.id == id))
        .count()
        .max(1) as f32;
    let tab_width = ((tabs_width + TAB_GAP) / count - TAB_GAP)
        .clamp(TAB_MIN_WIDTH, if narrow { 210.0 } else { TAB_MAX_WIDTH });
    for &id in &s.tabs {
        let Some(index) = s.threads.iter().position(|t| t.id == id) else {
            continue;
        };
        let thread = &s.threads[index];
        let active = index == s.active;
        let project = s
            .project(thread.project)
            .map(|p| p.name.as_str())
            .unwrap_or("No workspace");
        let mut tab = tab_frame(
            row()
                .key(if active {
                    "active-tab".into()
                } else {
                    format!("tab-{id}")
                })
                .role("tab")
                .cursor(rune::Cursor::Pointer)
                .label(format!("{project} / {}", thread.title))
                .focusable(true)
                .center_y()
                .gap(8.0)
                .pl(8.0)
                .pr(if s.tabs.len() > 1 { 2.0 } else { 8.0 })
                .h(TAB_HEIGHT)
                .radius(4.0)
                .w(tab_width)
                .min_w(tab_width)
                .hover_group()
                .on_press_tx(move |s: &mut State, tx| {
                    s.select_thread(index);
                    tx.scroll_into_view("active-tab");
                }),
            active,
            p,
        )
        .child(if tab_width >= TAB_PREFIX_WIDTH {
            pixel_text(project, p.muted).max_w(85.0).truncate()
        } else {
            // A tab at its minimum gives the width to the title.
            el()
        })
        .child(label(&thread.title, TAB_TITLE, p.ink).truncate().grow(1.0));
        if s.tabs.len() > 1 {
            // The close button shows on the active tab and on hover.
            let close = el()
                .key(format!("close-tab-{id}"))
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label("Close tab, keep thread running")
                .focusable(true)
                .size(22.0, 22.0)
                .center()
                .radius(4.0)
                .hover(p.selected)
                .press(p.selected)
                .on_press(move |s: &mut State| s.close_tab(id))
                .child(tool_icon("close", p.muted));
            tab = tab.child(if active {
                close
            } else {
                close.group_opacity(0.0, 1.0)
            });
        }
        tabs = tabs.child(tab);
    }
    row()
        .h(36.0)
        .window_drag()
        .center_y()
        .gap(6.0)
        .child(tabs)
        .child(new_tab_button(p))
        .child(reveal_active_tab(s, tab_width, tabs_width))
}

/// Scrolls the strip so the active tab shows whole, each time the selection, the tab list or
/// the tab width changes. This is a 1 pt marker outside the strip whose key names that state.
/// A new key is a new element, and it reports once when it first shows, which runs the scroll.
/// (`on_visible` on the tab itself only fires once half the tab shows, too late for a tab
/// that starts off screen.)
fn reveal_active_tab(s: &State, tab_width: f32, tabs_width: f32) -> El {
    use std::hash::{Hash, Hasher};
    let mut signature = std::collections::hash_map::DefaultHasher::new();
    s.tabs.hash(&mut signature);
    s.threads.get(s.active).map(|t| t.id).hash(&mut signature);
    (tab_width.round() as i32).hash(&mut signature);
    // Tabs stop shrinking at their minimum, so the strip's own width must change the key too.
    (tabs_width.round() as i32).hash(&mut signature);
    el().key(format!("tab-reveal-{:x}", signature.finish()))
        .absolute(0.0, 0.0)
        .size(1.0, 1.0)
        .a11y_hidden()
        .on_visible(|_: &mut State, visible, tx| {
            if visible {
                tx.instant();
                tx.scroll_into_view("active-tab");
            }
        })
}

fn chat_header(s: &State, p: Palette, narrow: bool, _: bool) -> El {
    // The tab strip already names the thread, so the header only holds thread actions:
    // status, a labeled "More actions" menu and the session panel toggle.
    let mut header = row()
        .key("chat-header")
        .w_full()
        .h(36.0)
        .px(10.0)
        .center_y()
        .gap(4.0);
    if multiple_machines(s) {
        let server = &s.machines.selected;
        header = header.child(
            row()
                .key("chat-machine")
                .gap(5.0)
                .center_y()
                .min_w(0.0)
                .child(icon("monitor", p.muted))
                .child(
                    label(s.machine_label(server), SMALL, p.muted)
                        .truncate()
                        .min_w(0.0)
                        .shrink(),
                ),
        );
    }
    // Crew threads link back to the thread that started them; nothing else repeats the tab.
    if let Some(parent) = s
        .current()
        .parent
        .and_then(|id| s.threads.iter().position(|t| t.id == id))
    {
        header = header.child(label("From", SMALL, p.muted)).child(
            button("breadcrumb-parent", &s.threads[parent].title, p, move |s| {
                s.select_thread(parent)
            })
            .max_w(240.0)
            .shrink()
            .min_w(0.0),
        );
    }
    // The away status takes the header's free space, so its width from the last layout is
    // what the line has to fit in.
    let available = rune::memo::layout_size("captain-away-slot").map(|size| size[0]);
    if let Some(line) = captain_ui::away_status(s, p, available) {
        header = header.child(
            row()
                .key("captain-away-slot")
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .child(line),
        );
    } else {
        header = header.child(spacer());
    }
    // The thread on a machine this desktop cannot reach says so beside its status.
    if let Some(machine) = s.unreachable_selected() {
        let text = format!("{machine} is unreachable. Reconnecting.");
        header = header.child(
            row()
                .key("chat-unreachable")
                .role("status")
                .label(text.clone())
                .gap(6.0)
                .center_y()
                .min_w(0.0)
                .shrink()
                .mr(8.0)
                .child(status_mark(Status::Waiting, p, 6.0))
                .child(pixel_text(text, p.brass).truncate().shrink().min_w(0.0)),
        );
    }
    header = header.child(if s.demo {
        outlined(
            row()
                .key("demo-badge")
                .h(20.0)
                .px(6.0)
                .mr(6.0)
                .radius(4.0)
                .center_y()
                .fill(p.card)
                .label("Demo data, read-only")
                .child(pixel_label("Demo", p.brass)),
            p,
        )
    } else {
        let t = s.current();
        let text = match (t.status, t.started) {
            (Status::Running, Some(start)) => Some(format!(
                "Working for {}",
                timer_label(start.elapsed().as_secs())
            )),
            _ => None,
        };
        status_slot("chat-status", t.status, text, HEADER_STATUS_SLOT, p)
    });
    header = header.child(toggle_button(
        "thread-actions",
        "More actions",
        "more",
        s.thread_menu,
        p,
        |s| s.thread_menu = !s.thread_menu,
    ));
    if captain_ui::brief_captain(s).is_some() {
        // With the Brief docked open it has its own close control in the drawer's header, so
        // the chat header does not repeat it. Closed, or over the chat as a sheet, it stays.
        if !(s.show_activity && !narrow) {
            header = header.child(captain_ui::brief_button(s, p));
        }
    } else if !s.show_activity {
        header = header.child(icon_button(
            "open-session-panel",
            "Show session details",
            "panel",
            p,
            |s| {
                s.show_activity = true;
                s.activity_full = false;
            },
        ));
    }
    header
}

/// Pin and archive, with their names, under the header's "More actions" button.
fn thread_menu(s: &State, p: Palette) -> El {
    col()
        .key("thread-menu")
        .role("menu")
        .focus_group()
        .label("Thread actions")
        // Wide enough for "Auto-settle after" and its value on one row.
        .w(232.0)
        .padding(4.0)
        .gap(1.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .anchored_fit("thread-actions", rune::Side::Bottom, rune::Align::End, 4.0)
        .modal(true, |s: &mut State| s.thread_menu = false)
        .child(thread_actions(s, s.current().id, p))
}

fn thread_actions(s: &State, id: usize, p: Palette) -> El {
    let Some(thread) = s.threads.iter().find(|t| t.id == id) else {
        return el();
    };
    let captain = s.is_captain_home(thread);
    let mut actions = col()
        .gap(1.0)
        .shrink()
        .min_h(0.0)
        .max_h((s.viewport_height - 80.0).max(240.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    if id == s.current().id && thread.pi.is_some() {
        actions = actions.child(
            menu_item("menu-refresh-commands", "Refresh Pi commands", "refresh", p).on_press(
                |s: &mut State| {
                    s.thread_menu = false;
                    s.row_menu = None;
                    s.current_mut().refresh_commands();
                },
            ),
        );
    }
    if id == s.current().id && thread.session.is_some() {
        let locked = s.demo || thread.status.busy() || thread.configuring || thread.archived();
        actions = actions
            .child(
                menu_item("menu-fork-conversation", "Fork conversation", "thread", p)
                    .disabled(locked)
                    .on_press(|s: &mut State| {
                        s.thread_menu = false;
                        s.row_menu = None;
                        s.fork_conversation(None);
                    }),
            )
            .child(
                menu_item("menu-fork-message", "Fork from message...", "thread", p)
                    .disabled(locked)
                    .on_press(|s: &mut State| {
                        s.thread_menu = false;
                        s.row_menu = None;
                        s.conversation_points(crate::model::ConversationMode::Fork);
                    }),
            )
            .child(
                menu_item("menu-rewind", "Edit from here...", "edit", p)
                    .disabled(locked)
                    .on_press(|s: &mut State| {
                        s.thread_menu = false;
                        s.row_menu = None;
                        s.conversation_points(crate::model::ConversationMode::Rewind);
                    }),
            );
    }
    if !captain {
        actions = actions
            .child(
                menu_item(
                    "menu-pin-thread",
                    if thread.pinned() {
                        "Unpin thread"
                    } else {
                        "Pin thread"
                    },
                    "pin",
                    p,
                )
                .disabled(thread.machine != "local")
                .on_press(move |s: &mut State| {
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.pin_thread(id);
                }),
            )
            .child(
                menu_item(
                    "menu-settle-thread",
                    if thread.life.settled() {
                        "Un-settle thread"
                    } else {
                        "Settle thread"
                    },
                    "check",
                    p,
                )
                .disabled(
                    thread.machine != "local" || !thread.life.settled() && s.lifecycle_busy(id),
                )
                .on_press(move |s: &mut State| {
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.settle_thread(id);
                }),
            );
        if thread.life.snoozed(lifecycle::now()) {
            actions = actions.child(
                menu_item("menu-snooze-thread", "Wake thread", "clock", p).on_press(
                    move |s: &mut State| {
                        s.row_menu = None;
                        s.thread_menu = false;
                        s.unsnooze_thread(id);
                    },
                ),
            );
        } else {
            actions = actions.child(
                menu_item("menu-snooze-thread", "Snooze...", "clock", p)
                    .disabled(!s.can_snooze(id))
                    .on_press(move |s: &mut State| s.open_snooze(id)),
            );
        }
    }
    actions = actions
        .child(rule(p))
        .child(
            menu_item(
                "menu-rename-thread",
                if captain {
                    "Name this body of work"
                } else {
                    "Rename thread"
                },
                "edit",
                p,
            )
            .on_press(move |s: &mut State| s.begin_rename(id)),
        )
        .child(
            menu_item("menu-mark-unread", "Mark unread", "thread", p)
                .disabled(thread.life.last_completed_at.is_none())
                .on_press(move |s: &mut State| {
                    if let Some(t) = s.threads.iter_mut().find(|t| t.id == id) {
                        t.life.mark_unread();
                    }
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.checkpoint();
                }),
        );
    if !captain {
        let project = thread.project;
        actions = actions
            .child(
                menu_item(
                    "menu-filter-project",
                    if s.project_scope == ProjectScope::All {
                        "Filter by workspace"
                    } else {
                        "Show all workspaces"
                    },
                    "folder",
                    p,
                )
                .on_press(move |s: &mut State| {
                    s.project_scope = if s.project_scope != ProjectScope::All {
                        ProjectScope::All
                    } else {
                        project
                            .map(ProjectScope::Project)
                            .unwrap_or(ProjectScope::NoProject)
                    };
                    s.scope_additional.clear();
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.checkpoint();
                }),
            )
            .child(
                menu_item(
                    "menu-auto-settle",
                    if thread.life.auto_settle_disabled_at.is_some() {
                        "Enable auto-settle"
                    } else {
                        "Disable auto-settle"
                    },
                    "check",
                    p,
                )
                .on_press(move |s: &mut State| {
                    s.toggle_thread_auto_settle(id);
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.checkpoint();
                }),
            );
        actions = actions.child(
            menu_value_item(
                "menu-auto-settle-days",
                "Auto-settle after",
                &match s.auto_settle_days {
                    1 => "1 day".to_string(),
                    days => format!("{days} days"),
                },
                "clock",
                p,
            )
            .on_press(|s: &mut State| {
                s.cycle_auto_settle_days();
            }),
        );
        if !thread.draft.text.is_empty() {
            actions = actions.child(
                menu_item("menu-discard-draft", "Discard draft", "close", p).on_press(
                    move |s: &mut State| {
                        s.row_menu = None;
                        s.thread_menu = false;
                        s.discard_draft(id);
                    },
                ),
            );
        }
    }
    actions = actions
        .child(rule(p))
        .child(
            menu_item("menu-copy-thread-id", "Copy thread ID", "thread", p).on_press_tx(
                move |s: &mut State, tx| {
                    tx.copy(id.to_string());
                    s.row_menu = None;
                    s.thread_menu = false;
                },
            ),
        )
        .child(
            menu_item("menu-copy-path", "Copy path", "folder", p).on_press_tx(
                move |s: &mut State, tx| {
                    if let Some(i) = s.threads.iter().position(|t| t.id == id)
                        && let Ok(path) = s.thread_directory(i)
                    {
                        tx.copy(path.to_string_lossy().into_owned());
                    }
                    s.row_menu = None;
                    s.thread_menu = false;
                },
            ),
        );
    let archive = menu_item(
        "menu-archive-thread",
        if thread.archived() {
            "Restore thread"
        } else {
            "Archive thread"
        },
        "archive",
        p,
    );
    actions = actions.child(if s.lifecycle_busy(id) || thread.machine != "local" {
        archive
            .disabled(true)
            .child(label("Working", SMALL, p.muted))
    } else {
        archive.on_press(move |s: &mut State| {
            s.row_menu = None;
            s.thread_menu = false;
            s.archive_thread(id);
        })
    });
    if s.is_captain_home(thread)
        && let Some(captain) = thread.captain
    {
        actions = actions.child(
            menu_item("menu-edit-captain", "Edit captain profile", "edit", p).on_press(
                move |s: &mut State| {
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.edit_captain(captain);
                },
            ),
        );
    }
    actions.child(rule(p)).child(
        danger_menu_item("menu-delete-thread", "Delete thread...", "trash", p)
            .on_press(move |s: &mut State| s.request_delete(id)),
    )
}

fn snooze_menu(s: &State, p: Palette) -> El {
    let (_, choices) = s.snooze_menu.as_ref().unwrap();
    let mut card = col()
        .key("snooze-menu")
        .role("menu")
        .focus_group()
        .w(280.0)
        .padding(4.0)
        .gap(1.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        // The title is a pixel label, inset 8 px so it lines up with the rows' icons.
        .child(
            pixel_label("Snooze thread", p.muted)
                .px(8.0)
                .pt(6.0)
                .pb(4.0),
        );
    if s.custom_snooze_open {
        card = card
            .child(label("Local date and time, or duration", SMALL, p.muted).px(8.0))
            .child(
                text_input(&s.snooze_edit, LABEL)
                    .key("snooze-custom-input")
                    .label("Snooze time")
                    .placeholder("2026-11-12 09:00 or 90m, 3h, 2d", p.muted)
                    .single_line()
                    .color(p.ink)
                    .w_full()
                    .padding(8.0)
                    .mx(4.0)
                    .radius(4.0)
                    .fill(field_fill(s, p))
                    .field_focus(&s.snooze_edit, p)
                    .on_edit(|s: &mut State| &mut s.snooze_edit),
            )
            .child(button("snooze-custom-save", "Snooze", p, move |s| {
                if let Some(until) =
                    lifecycle::custom_snooze(&s.snooze_edit.text, chrono::Local::now())
                {
                    let ids = s.snooze_targets.clone();
                    for id in ids {
                        s.snooze_thread(id, until);
                    }
                } else {
                    s.lifecycle_error =
                        Some("Choose a valid future local time. DST gaps are not valid.".into());
                }
            }));
    } else {
        for (i, choice) in choices.iter().enumerate() {
            card = card.child(
                menu_item(&format!("snooze-preset-{i}"), &choice.label, "clock", p)
                    // The label already grows, so the time sits against the right edge on its
                    // own. A spacer here would split the row and clip the label.
                    .child(
                        label(&choice.time, TIMER, p.muted)
                            .w(72.0)
                            .align(rune::Align::End),
                    )
                    .on_press(move |s: &mut State| {
                        let until = s
                            .snooze_menu
                            .as_ref()
                            .and_then(|(_, c)| c.get(i))
                            .and_then(|c| lifecycle::pick_preset(c, lifecycle::now()));
                        if let Some(until) = until {
                            let ids = s.snooze_targets.clone();
                            for id in ids {
                                s.snooze_thread(id, until);
                            }
                        } else {
                            s.lifecycle_error =
                                Some("That snooze time has passed. Choose another time.".into());
                        }
                    }),
            );
        }
        card = card.child(rule(p)).child(
            menu_item("snooze-custom", "Custom...", "clock", p).on_press(|s: &mut State| {
                s.custom_snooze_open = true;
                s.snooze_edit.focused = true;
            }),
        );
    }
    if let Some(error) = &s.lifecycle_error {
        card = card.child(label(error, SMALL, p.danger).wrap());
    }
    // A popover on the thread's row, like the thread menu: no scrim, no Cancel row. Escape and a
    // press outside close it.
    card.anchored_fit(
        format!("thread-{}", s.snooze_menu.as_ref().unwrap().0),
        rune::Side::Right,
        rune::Align::Start,
        4.0,
    )
    .modal(true, |s: &mut State| {
        s.snooze_menu = None;
        s.lifecycle_error = None;
    })
}

fn bulk_thread_actions(s: &State, p: Palette) -> El {
    let count = s.selected_threads.len();
    let mut actions = col().gap(1.0);
    for (key, title, action) in [
        ("bulk-unpin", "Unpin", 0),
        ("bulk-settle", "Settle", 1),
        ("bulk-snooze", "Snooze...", 2),
        ("bulk-unread", "Mark unread", 3),
        ("bulk-delete", "Delete...", 4),
    ] {
        actions = actions.child(
            menu_item(
                key,
                &format!("{title} ({count})"),
                ["pin", "check", "clock", "dot", "trash"][action],
                p,
            )
            .on_press(move |s: &mut State| {
                let ids = s.selected_threads.clone();
                s.row_menu = None;
                if action == 2 {
                    if let Some(id) = ids.first() {
                        s.open_snooze(*id);
                    }
                    return;
                }
                if action == 4 {
                    if let Some(id) = ids.first() {
                        s.request_delete(*id);
                    }
                    return;
                }
                for id in ids {
                    if action == 0 {
                        if s.threads.iter().any(|t| t.id == id && t.pinned()) {
                            s.pin_thread(id);
                        }
                    } else if action == 1 {
                        if s.threads.iter().any(|t| t.id == id && !t.life.settled()) {
                            s.settle_thread(id);
                        }
                    } else if let Some(t) = s.threads.iter_mut().find(|t| t.id == id) {
                        t.life.mark_unread();
                    }
                }
                s.checkpoint();
            }),
        );
    }
    actions
}

fn row_menu(s: &State, p: Palette) -> El {
    let target = s.row_menu.expect("row menu target");
    let (anchor, actions) = match target {
        RowTarget::Thread(id) => (
            format!("thread-{id}"),
            if s.selected_threads.contains(&id) && s.selected_threads.len() > 1 {
                bulk_thread_actions(s, p)
            } else {
                thread_actions(s, id, p)
            },
        ),
        RowTarget::Project(id) => (
            format!("project-option-{id}"),
            col()
                .child(
                    menu_item("menu-project-new-thread", "New thread", "compose", p).on_press(
                        move |s: &mut State| {
                            s.row_menu = None;
                            s.new_thread_in(Some(id));
                        },
                    ),
                )
                .child(
                    menu_item("menu-project-captain", "Open Captain", "grid", p).on_press(
                        move |s: &mut State| {
                            s.row_menu = None;
                            s.project_picker = None;
                            s.choose_project_captain(id);
                        },
                    ),
                )
                .child(
                    menu_item(
                        "menu-remove-project",
                        "Remove workspace from list",
                        "close",
                        p,
                    )
                    .on_press(move |s: &mut State| {
                        s.row_menu = None;
                        s.project_picker = None;
                        s.remove_project(id);
                    }),
                ),
        ),
    };
    col()
        .key("row-context-menu")
        .instant_changes()
        .role("menu")
        .focus_group()
        .label("Row actions")
        .w(222.0)
        .padding(4.0)
        .gap(1.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .anchored_fit(
            if s.row_menu_at.is_some() {
                "row-menu-point".into()
            } else {
                anchor
            },
            // Beside the point, as a native context menu opens, so the window clamps it
            // vertically instead of cutting it short.
            if s.row_menu_at.is_some() || matches!(target, RowTarget::Thread(_)) {
                rune::Side::Right
            } else {
                rune::Side::Bottom
            },
            rune::Align::Start,
            if s.row_menu_at.is_some() { 2.0 } else { 4.0 },
        )
        .modal(true, |s: &mut State| s.row_menu = None)
        .child(actions)
}

fn thread_dialog(s: &State, p: Palette) -> El {
    let renaming = s.rename_thread.is_some();
    let id = s.rename_thread.or(s.delete_thread).expect("thread dialog");
    let captain = s
        .threads
        .iter()
        .any(|thread| thread.id == id && s.is_captain_home(thread));
    let title = if renaming && captain {
        "Name this body of work"
    } else if renaming {
        "Rename thread"
    } else {
        "Delete thread"
    };
    let mut card = col()
        .key("thread-management-dialog")
        .role("dialog")
        .label(title)
        .w(380.0)
        .max_w(540.0)
        .padding(16.0)
        .gap(12.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .on_press(|_: &mut State| {})
        .child(label(title, DIALOG_TITLE, p.ink));
    if renaming {
        if captain {
            card = card.child(
                label(
                    "Name the goal, feature, or ongoing work. The captain profile stays unchanged.",
                    SMALL,
                    p.muted,
                )
                .wrap(),
            );
        }
        card = card.child(
            text_input(&s.rename_edit, BODY)
                .key("thread-name-input")
                .label("Thread name")
                .single_line()
                .color(p.ink)
                .w_full()
                .padding(8.0)
                .radius(6.0)
                .fill(if s.dark { p.bg } else { p.panel })
                .field_focus(&s.rename_edit, p)
                .on_edit(|s: &mut State| &mut s.rename_edit)
                .on_submit(|s: &mut State, _| s.save_thread_name()),
        );
    } else {
        let roots = if s.delete_roots.is_empty() {
            vec![id]
        } else {
            s.delete_roots.clone()
        };
        let count = roots.len();
        let all: std::collections::HashSet<_> =
            roots.iter().flat_map(|id| s.deletion_ids(*id)).collect();
        let descendants = all.len().saturating_sub(count);
        let target = if count > 1 {
            format!("these {count} threads and their")
        } else {
            "this thread and its".into()
        };
        card = card.child(label(format!(
            "Delete {target} {descendants} delegated children? Running agents will stop. Project files and Pi session files stay on disk."
        ), BODY, p.ink).wrap());
    }
    card = card.child(
        row()
            .gap(8.0)
            .child(spacer())
            .child(secondary_button("cancel-thread-dialog", "Cancel", p, |s| {
                s.rename_thread = None;
                s.delete_thread = None;
            }))
            .child(if renaming {
                button("save-thread-name", "Save name", p, |s| s.save_thread_name())
                    .disabled(s.rename_edit.text.trim().is_empty())
            } else {
                button(
                    "confirm-delete-thread",
                    "Delete thread and children",
                    p,
                    |s| s.confirm_delete(),
                )
            }),
    );
    el().key("thread-dialog-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .padding(16.0)
        .fill(Color::BLACK.alpha(0.3))
        .modal(false, |s: &mut State| {
            s.rename_thread = None;
            s.delete_thread = None;
        })
        .child(card)
}

/// A centred modal over a dimmed window, as T3 Code opens its command palette: it sits a little
/// above the middle, and a press outside or Escape closes it.
fn modal_frame(key: &str, s: &State, close: fn(&mut State), card: El) -> El {
    col()
        .key(key)
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .items_center()
        .pt((s.viewport_height * 0.16).clamp(40.0, 160.0))
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.22 }))
        .modal(false, close)
        .on_press(close)
        // Escape closes it from the keyboard, so the scrim is no Tab stop.
        .focusable(false)
        .child(card)
}

/// A keyboard hint for a modal's footer: the key in a small cap, then what it does.
/// A key name in an outlined Departure Mono chip.
fn keycap_chip(keys: &str, p: Palette) -> El {
    outlined(
        row()
            .h(20.0)
            .px(6.0)
            .center_y()
            .radius(4.0)
            .fill(p.card)
            .child(pixel_text(keys, p.muted)),
        p,
    )
}
fn keycap(keys: &str, action: &str, p: Palette) -> El {
    row()
        .gap(8.0)
        .center_y()
        .child(keycap_chip(keys, p))
        .child(label(action, SMALL, p.muted))
}

/// A floating surface for menus and pickers, styled as T3 Code's popovers: one radius, a
/// hairline edge and a soft shadow that reads in both themes.
fn popover(key: &str, name: &str, _: &State, p: Palette) -> El {
    col()
        .key(key)
        .label(name)
        .padding(4.0)
        .gap(1.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
}

#[derive(Clone, PartialEq, Eq)]
enum PickerChoice {
    All,
    NoProject,
    Project(usize),
    MachineProject(String, usize),
}

/// The picker's rows that match its search, in display order.
/// Each choice with its name and, for a linked machine, the machine's name.
fn picker_choices(s: &State, mode: ProjectPicker) -> Vec<(PickerChoice, String, Option<String>)> {
    let query = s.picker_query.text.trim().to_lowercase();
    let mut choices = vec![];
    if mode == ProjectPicker::Scope {
        choices.push((PickerChoice::All, "All workspaces".to_string(), None));
    }
    if mode != ProjectPicker::CaptainScope {
        choices.push((PickerChoice::NoProject, "No workspace".to_string(), None));
    }
    for id in s.sorted_projects() {
        let project = s.project(Some(id)).expect("sorted project");
        if !project.hidden {
            choices.push((
                PickerChoice::Project(id),
                project.name.clone(),
                (mode == ProjectPicker::Scope && s.client.is_some())
                    .then(|| s.machine_label(&s.machines.selected)),
            ));
        }
    }
    if mode == ProjectPicker::Scope && s.client.is_some() {
        for (server, view) in &s.machines.views {
            if server == &s.machines.selected {
                continue;
            }
            for project in view["index"]["projects"].as_array().into_iter().flatten() {
                if project["hidden"] == true {
                    continue;
                }
                if let (Some(id), Some(name)) = (project["id"].as_u64(), project["name"].as_str()) {
                    choices.push((
                        PickerChoice::MachineProject(server.clone(), id as usize),
                        name.to_string(),
                        Some(s.machine_label(server)),
                    ));
                }
            }
        }
    }
    choices.retain(|(_, name, machine)| {
        query.is_empty()
            || format!("{name} {}", machine.as_deref().unwrap_or(""))
                .to_lowercase()
                .contains(&query)
    });
    choices
}

fn choose_picker(s: &mut State, mode: ProjectPicker, choice: PickerChoice, shift: bool) {
    let project = match choice {
        PickerChoice::All => {
            s.project_scope = ProjectScope::All;
            s.scope_additional.clear();
            s.machines.scope_remote.clear();
            s.selected_threads.clear();
            s.project_picker = None;
            s.checkpoint();
            return;
        }
        PickerChoice::NoProject => None,
        PickerChoice::Project(id) => Some(id),
        PickerChoice::MachineProject(server, id) => {
            if mode == ProjectPicker::Scope {
                s.set_machine_scope(server, Some(id), shift);
            }
            return;
        }
    };
    match mode {
        ProjectPicker::NewThread => s.new_thread_in(project),
        ProjectPicker::Scope => s.set_project_scope(project, shift),
        ProjectPicker::AddProject => {}
        ProjectPicker::CaptainScope => {
            if let Some(project) = project {
                captain_ui::toggle_scope(s, project);
            }
        }
        ProjectPicker::AssignCaptain => {
            s.project_picker = None;
            s.projects.captain_target = project;
            s.captain_project = Some(project);
            if s.captains.is_empty() && s.captain_editor.is_none() {
                s.begin_new_captain();
            } else {
                s.captain_picker = s.captain_editor.is_none();
            }
        }
    }
}

/// The workspace picker, as T3 Code's command palette: a centred modal that starts with a
/// search field and moves with the arrow keys. Enter picks the highlighted row; Shift adds a
/// workspace to the filter.
fn project_picker(s: &State, p: Palette) -> El {
    let mode = s.project_picker.expect("project picker");
    let choices = picker_choices(s, mode);
    let highlight = s.picker_highlight.min(choices.len().saturating_sub(1));
    let placeholder = match mode {
        ProjectPicker::Scope => "Filter by workspace...",
        ProjectPicker::NewThread => "New thread in...",
        ProjectPicker::AssignCaptain => "Assign a captain to...",
        ProjectPicker::AddProject => "Add workspace",
        ProjectPicker::CaptainScope => "Add to this captain's scope...",
    };
    let mut card = popover("project-picker", placeholder, s, p)
        .role("dialog")
        .w(520.0)
        .max_h(460.0)
        .padding(4.0)
        .radius(6.0)
        .on_press(|_: &mut State| {})
        .child(
            row()
                .h(46.0)
                // The search box and the rows below share one inset from the card's edge.
                .mx(4.0)
                .px(8.0)
                .gap(10.0)
                .center_y()
                .radius(4.0)
                .fill(field_fill(s, p))
                .field_focus(&s.picker_query, p)
                .child(
                    el().size(16.0, 16.0)
                        .center()
                        .child(icon("search", p.muted)),
                )
                .child(
                    rune::text_input(&s.picker_query, TextStyle::new(15.0, 420.0).tracking(-0.15))
                        .key("picker-search")
                        .label(placeholder)
                        .placeholder(placeholder, p.muted)
                        .color(p.ink)
                        .single_line()
                        .grow(1.0)
                        .min_w(0.0)
                        .on_edit(|s: &mut State| &mut s.picker_query)
                        .on_input_key(move |s: &mut State, event, _| {
                            if edits_text(event) {
                                s.picker_highlight = 0;
                            }
                            let count = s
                                .project_picker
                                .map_or(0, |mode| picker_choices(s, mode).len());
                            match event.key {
                                rune::Key::Down if count > 0 => {
                                    s.picker_highlight = (s.picker_highlight + 1).min(count - 1);
                                    true
                                }
                                // The list's ends, as cmdk's Home and End go in KSG Code's
                                // command palette. The field is one line, so its caret loses
                                // nothing it cannot reach with the arrows.
                                rune::Key::Home => {
                                    s.picker_highlight = 0;
                                    true
                                }
                                rune::Key::End if count > 0 => {
                                    s.picker_highlight = count - 1;
                                    true
                                }
                                rune::Key::Up => {
                                    s.picker_highlight = s.picker_highlight.saturating_sub(1);
                                    true
                                }
                                rune::Key::Enter => {
                                    if let Some(mode) = s.project_picker
                                        && let Some(choice) = picker_choices(s, mode)
                                            .get(s.picker_highlight.min(count.saturating_sub(1)))
                                            .map(|(choice, _, _)| choice.clone())
                                    {
                                        choose_picker(s, mode, choice, event.mods.shift);
                                    }
                                    true
                                }
                                rune::Key::Escape => {
                                    s.project_picker = None;
                                    true
                                }
                                _ => s.undo_key(event, true),
                            }
                        }),
                ),
        )
        // No hairline under the search box: its own frame already ends the field.
        .child(el().h(4.0));
    let mut list = col().gap(1.0).min_h(0.0).shrink().scroll(ScrollSpec {
        stick_bottom: false,
    });
    for (index, (choice, name, machine)) in choices.iter().enumerate() {
        let remote_project = if let PickerChoice::MachineProject(server, id) = choice {
            s.machines
                .views
                .get(server)
                .and_then(|view| view["index"]["projects"].as_array())
                .and_then(|projects| {
                    projects
                        .iter()
                        .find(|project| project["id"].as_u64() == Some(*id as u64))
                })
                .and_then(|project| crate::projects::Project::from_record(project, false))
        } else {
            None
        };
        let (key, project) = match choice {
            PickerChoice::All => ("scope-all-projects".to_string(), None),
            PickerChoice::NoProject => ("choose-no-project".to_string(), None),
            PickerChoice::Project(id) => (format!("choose-project-{id}"), s.project(Some(*id))),
            PickerChoice::MachineProject(server, id) => (
                format!("choose-project-{server}-{id}"),
                remote_project.as_ref(),
            ),
        };
        let checked = match (mode, choice) {
            (ProjectPicker::CaptainScope, PickerChoice::Project(id)) => {
                captain_ui::in_scope(s, *id)
            }
            (ProjectPicker::Scope, PickerChoice::All) => s.project_scope == ProjectScope::All,
            (ProjectPicker::Scope, PickerChoice::NoProject) => {
                s.project_scope == ProjectScope::NoProject
            }
            (ProjectPicker::Scope, PickerChoice::Project(id)) => {
                s.project_scope != ProjectScope::All
                    && s.scope_includes_on(&s.machines.selected, Some(*id))
            }
            (ProjectPicker::Scope, PickerChoice::MachineProject(server, id)) => {
                s.project_scope != ProjectScope::All && s.scope_includes_on(server, Some(*id))
            }
            _ => false,
        };
        let choice = choice.clone();
        let action_choice = choice.clone();
        let mut item = row()
            .key(key)
            .role("menuitem")
            .cursor(rune::Cursor::Pointer)
            .label(match machine {
                Some(machine) => format!("{name}, {machine}"),
                None => name.clone(),
            })
            .focusable(true)
            .h(38.0)
            // Inset 4 px from the frame so the highlight sits inside it.
            .mx(4.0)
            .pl(12.0)
            .pr(8.0)
            .gap(10.0)
            .center_y()
            .radius(4.0)
            .fill(if index == highlight {
                p.coral_bg
            } else {
                Color::TRANSPARENT
            })
            .hover(p.selected)
            .on_press_tx(move |s: &mut State, tx| {
                choose_picker(s, mode, action_choice.clone(), tx.mods().shift)
            })
            .child(match choice {
                PickerChoice::All => el().size(16.0, 16.0).center().child(icon("list", p.muted)),
                _ => project_tile(s, p, project, 16.0),
            })
            .child(
                label(name.as_str(), TextStyle::new(14.0, 480.0), p.ink)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            )
            .child(match machine {
                Some(machine) => label(machine, SMALL, p.muted).truncate().max_w(MACHINE_CAP),
                None => el(),
            })
            // Where the workspace lives, so two folders with one name stay distinct.
            .child(
                label(
                    project
                        .map(|project| project.path.display().to_string())
                        .unwrap_or_default(),
                    SMALL,
                    p.faint,
                )
                .truncate()
                .grow(1.0)
                .min_w(0.0),
            );
        if checked {
            item = item.child(icon("check", p.coral));
        }
        if let PickerChoice::Project(id) = choice {
            let target = RowTarget::Project(id);
            item = item.on_secondary_press(move |s: &mut State, _, _| {
                s.row_menu = Some(target);
                s.row_menu_at = None;
            });
            item = row()
                .key(format!("project-option-{id}"))
                .child(item.grow(1.0));
        }
        list = list.child(item);
    }
    if choices.is_empty() {
        list = list.child(
            label("No matching workspace", SMALL, p.muted)
                .px(8.0)
                .py(8.0),
        );
    }
    card = card
        .child(list)
        .child(el().h(0.5).w_full().my(4.0).fill(p.line))
        .child(
            menu_item("picker-add-project", "Add workspace...", "folder-plus", p)
                .on_press(|s: &mut State| s.open_picker(ProjectPicker::AddProject)),
        );
    let mut hints = row()
        .h(34.0)
        .px(8.0)
        .gap(16.0)
        .center_y()
        .child(keycap("↑↓", "Move", p))
        .child(keycap("Enter", "Choose", p));
    if mode == ProjectPicker::Scope {
        hints = hints.child(keycap("Shift", "Add to filter", p));
    }
    card = card.child(hints.child(spacer()).child(keycap("Esc", "Close", p)));
    modal_frame(
        "project-picker-backdrop",
        s,
        |s| s.project_picker = None,
        card,
    )
}

/// What the Add workspace palette offers before browsing, as T3 Code's Add project sources.
#[derive(Clone, PartialEq, Eq)]
enum AddSource {
    LocalFolder,
    GitUrl,
    SystemPicker,
    Restore(std::path::PathBuf),
}

fn add_sources(s: &State) -> Vec<(AddSource, String, String)> {
    let mut items = vec![
        (
            AddSource::LocalFolder,
            "Local folder".to_string(),
            "Browse a folder on disk".to_string(),
        ),
        (
            AddSource::GitUrl,
            "Git URL".into(),
            "Clone from a remote URL".into(),
        ),
        (
            AddSource::SystemPicker,
            "System folder picker".into(),
            "Choose a folder in the system dialog".into(),
        ),
    ];
    let mut removed: Vec<_> = s.projects.entries.iter().filter(|p| p.hidden).collect();
    removed.sort_by_key(|p| std::cmp::Reverse(p.updated));
    for project in removed {
        items.push((
            AddSource::Restore(project.path.clone()),
            format!("Restore {}", project.name),
            project.path.display().to_string(),
        ));
    }
    let query = s.picker_query.text.trim().to_lowercase();
    items.retain(|(_, title, detail)| {
        query.is_empty()
            || title.to_lowercase().contains(&query)
            || detail.to_lowercase().contains(&query)
    });
    items
}

fn run_add_source(s: &mut State, tx: &mut Tx, source: AddSource) {
    match source {
        AddSource::LocalFolder => s.browse_for_project(),
        AddSource::GitUrl => {
            s.project_picker = None;
            s.open_clone_dialog();
        }
        AddSource::SystemPicker => {
            s.project_picker = None;
            crate::projects::pick_folder(s, tx);
        }
        AddSource::Restore(path) => match s.add_project(path) {
            Ok(_) => {
                s.project_picker = None;
                s.projects.menu_error = None;
                s.checkpoint();
            }
            Err(error) => s.projects.menu_error = Some(error),
        },
    }
}

/// The folders the browse field lists, and the typed text up to the folder they are in.
fn browse_entries(query: &str) -> (String, Vec<String>) {
    let cut = query.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let (dir, filter) = crate::projects::browse_split(query);
    (
        query[..cut].to_string(),
        crate::projects::list_folders(&dir, &filter),
    )
}

/// Whether a key changes a search field's text, which sends a palette's highlight back to the
/// top. Rune also calls `on_edit` for keys that only move the caret or the highlight, so the
/// reset cannot live there.
fn edits_text(event: &rune::KeyEvent) -> bool {
    matches!(
        event.key,
        rune::Key::Text(_) | rune::Key::Backspace | rune::Key::Delete
    ) || matches!(event.key, rune::Key::Command('v' | 'x' | 'z'))
}
/// Move the browse field into a listed folder.
fn browse_into(s: &mut State, name: &str) {
    let (prefix, _) = browse_entries(&s.picker_query.text);
    let prefix = if prefix.is_empty() && s.picker_query.text.trim() == "~" {
        format!("~{}", std::path::MAIN_SEPARATOR)
    } else {
        prefix
    };
    s.picker_query
        .replace_text(format!("{prefix}{name}{}", std::path::MAIN_SEPARATOR));
    s.picker_highlight = 0;
}

/// A palette row with a title and a quieter description, as T3 Code lists its sources.
fn palette_row(key: String, glyph: &str, title: &str, detail: &str, lit: bool, p: Palette) -> El {
    row()
        .key(key)
        .role("menuitem")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .min_h(44.0)
        // Inset 4 px from the card's padding, in line with the search row above it.
        .mx(4.0)
        .px(8.0)
        .py(6.0)
        .gap(10.0)
        .center_y()
        .radius(4.0)
        .fill(if lit { p.coral_bg } else { Color::TRANSPARENT })
        .hover(p.selected)
        .child(el().size(16.0, 16.0).center().child(icon(glyph, p.muted)))
        .child({
            let text = col()
                .gap(1.0)
                .grow(1.0)
                .min_w(0.0)
                .child(label(title, TextStyle::new(14.0, 480.0), p.ink).truncate());
            if detail.is_empty() {
                text
            } else {
                text.child(label(detail, SMALL, p.muted).truncate())
            }
        })
}

/// The modifier the palette's add shortcut uses on this platform.
const SUBMIT_CHORD: &str = if cfg!(target_os = "macos") {
    "⌘ Enter"
} else {
    "Ctrl Enter"
};

/// Add workspace, as T3 Code's Add project palette: choose a source, then browse folders by
/// typing a path. Enter opens the highlighted folder, Ctrl+Enter adds the typed one, and a
/// path that does not exist yet is created first.
fn add_project_palette(s: &State, p: Palette) -> El {
    if s.machines.selected != "local" {
        return remote_project_dialog(s, p);
    }
    let browsing = s.picker_browsing;
    let query = s.picker_query.text.clone();
    let folders = if browsing {
        browse_entries(&query).1
    } else {
        vec![]
    };
    let sources = if browsing { vec![] } else { add_sources(s) };
    let count = if browsing {
        folders.len()
    } else {
        sources.len()
    };
    let highlight = s.picker_highlight.min(count.saturating_sub(1));
    let target = crate::projects::expand_path(&query);
    let creates = browsing && !query.trim().is_empty() && !target.exists();
    let submit = if creates { "Create & Add" } else { "Add" };
    let placeholder = if browsing {
        "Type a folder path"
    } else {
        "Add a workspace from..."
    };
    let mut field = row()
        .h(46.0)
        .mx(4.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(4.0)
        .fill(field_fill(s, p))
        .field_focus(&s.picker_query, p)
        .child(el().size(16.0, 16.0).center().child(icon(
            if browsing { "folder" } else { "folder-plus" },
            p.muted,
        )))
        .child(
            rune::text_input(&s.picker_query, TextStyle::new(15.0, 420.0).tracking(-0.15))
                .key("picker-search")
                .label(placeholder)
                .placeholder(placeholder, p.muted)
                .color(p.ink)
                .single_line()
                .grow(1.0)
                .min_w(0.0)
                .on_edit(|s: &mut State| {
                    s.projects.menu_error = None;
                    &mut s.picker_query
                })
                .on_input_key(|s: &mut State, event, tx| {
                    if edits_text(event) {
                        s.picker_highlight = 0;
                    }
                    let browsing = s.picker_browsing;
                    let (_, folders) = if browsing {
                        browse_entries(&s.picker_query.text)
                    } else {
                        (String::new(), vec![])
                    };
                    let sources = if browsing { vec![] } else { add_sources(s) };
                    let count = if browsing {
                        folders.len()
                    } else {
                        sources.len()
                    };
                    let at = s.picker_highlight.min(count.saturating_sub(1));
                    let command = event.mods.cmd || event.mods.ctrl;
                    match event.key {
                        rune::Key::Down if count > 0 => {
                            s.picker_highlight = (at + 1).min(count - 1);
                            true
                        }
                        rune::Key::Home => {
                            s.picker_highlight = 0;
                            true
                        }
                        rune::Key::End if count > 0 => {
                            s.picker_highlight = count - 1;
                            true
                        }
                        rune::Key::Up => {
                            s.picker_highlight = at.saturating_sub(1);
                            true
                        }
                        rune::Key::Tab if browsing => {
                            if let Some(name) = folders.get(at) {
                                browse_into(s, &name.clone());
                            }
                            true
                        }
                        rune::Key::Enter if browsing => {
                            match folders.get(at) {
                                Some(name) if !command => browse_into(s, &name.clone()),
                                _ => s.add_browsed_project(),
                            }
                            true
                        }
                        rune::Key::Enter => {
                            if let Some((source, _, _)) = sources.get(at).cloned() {
                                run_add_source(s, tx, source);
                            }
                            true
                        }
                        rune::Key::Escape => {
                            if browsing {
                                s.open_picker(ProjectPicker::AddProject);
                            } else {
                                s.project_picker = None;
                            }
                            true
                        }
                        _ => s.undo_key(event, true),
                    }
                }),
        );
    if browsing {
        field = field.child(
            row()
                .key("browse-add")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!("{submit} this folder"))
                .focusable(true)
                .h(28.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(4.0)
                .border(1.0, p.frame)
                .hover(p.selected)
                .on_press(|s: &mut State| s.add_browsed_project())
                .child(label(submit, TextStyle::new(12.5, 560.0), p.ink))
                .child(
                    pixel_text(
                        if folders.is_empty() {
                            "Enter"
                        } else {
                            SUBMIT_CHORD
                        },
                        p.muted,
                    )
                    .px(4.0)
                    .radius(4.0)
                    .fill(p.selected),
                ),
        );
    }
    let mut card = popover("project-picker", "Add workspace", s, p)
        .role("dialog")
        .w(560.0)
        .max_h(480.0)
        .padding(4.0)
        .radius(6.0)
        .on_press(|_: &mut State| {})
        .child(field)
        .child(el().h(4.0));
    let mut list = col().gap(1.0).min_h(0.0).shrink().scroll(ScrollSpec {
        stick_bottom: false,
    });
    if browsing {
        for (index, name) in folders.iter().enumerate() {
            let entry = name.clone();
            list = list.child(
                palette_row(
                    format!("browse-{index}"),
                    "folder",
                    name,
                    "",
                    index == highlight,
                    p,
                )
                .min_h(34.0)
                .on_press(move |s: &mut State| browse_into(s, &entry)),
            );
        }
        if folders.is_empty() {
            list = list.child(
                label(
                    if creates {
                        "No folder here yet. Add creates it."
                    } else {
                        "No folders inside this one."
                    },
                    SMALL,
                    p.muted,
                )
                .px(8.0)
                .py(10.0),
            );
        }
    } else {
        for (index, (source, title, detail)) in sources.iter().enumerate() {
            let glyph = match source {
                AddSource::LocalFolder => "folder-plus",
                AddSource::GitUrl => "branch",
                AddSource::SystemPicker => "folder",
                AddSource::Restore(_) => "archive",
            };
            let key = match source {
                AddSource::LocalFolder => "add-local-folder".to_string(),
                AddSource::GitUrl => "project-clone".to_string(),
                AddSource::SystemPicker => "project-open-folder".to_string(),
                AddSource::Restore(_) => format!("project-{index}"),
            };
            let source = source.clone();
            list = list.child(
                palette_row(key, glyph, title, detail, index == highlight, p)
                    .on_press_tx(move |s: &mut State, tx| run_add_source(s, tx, source.clone())),
            );
        }
        if sources.is_empty() {
            list = list.child(label("Nothing matches", SMALL, p.muted).px(8.0).py(10.0));
        }
    }
    card = card.child(list);
    if let Some(error) = &s.projects.menu_error {
        card = card.child(label(error, SMALL, p.danger).wrap().px(8.0).py(6.0));
    }
    let hints = if browsing {
        row()
            .h(34.0)
            .px(8.0)
            .gap(16.0)
            .center_y()
            .child(keycap("Enter", "Open folder", p))
            .child(keycap("Tab", "Complete", p))
            .child(keycap(SUBMIT_CHORD, submit, p))
            .child(spacer())
            .child(keycap("Esc", "Back", p))
    } else {
        row()
            .h(34.0)
            .px(8.0)
            .gap(16.0)
            .center_y()
            .child(keycap("↑↓", "Move", p))
            .child(keycap("Enter", "Choose", p))
            .child(spacer())
            .child(keycap("Esc", "Close", p))
    };
    card = card
        .child(el().h(0.5).w_full().mt(4.0).fill(p.line))
        .child(hints);
    modal_frame(
        "project-picker-backdrop",
        s,
        |s| s.project_picker = None,
        card,
    )
}

/// Ported from KSG Code's AddEnvironmentDialog host input; paths belong to the selected server.
fn remote_project_dialog(s: &State, p: Palette) -> El {
    let card = col()
        .key("remote-project-dialog")
        .role("dialog")
        .label("Add workspace on server")
        .w((s.viewport_width - 32.0).clamp(280.0, 480.0))
        .padding(16.0)
        .gap(12.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .child(
            label(
                format!("Workspace on {}", s.machine_label(&s.machines.selected)),
                BODY,
                p.ink,
            )
            .wrap(),
        )
        .child(
            label(
                "Enter an absolute path on that server. Shina does not copy or translate files.",
                SMALL,
                p.muted,
            )
            .wrap(),
        )
        .child(
            text_input(&s.machines.project_path, BODY)
                .key("remote-project-path")
                .label("Host workspace path")
                .single_line()
                .w_full()
                .padding(8.0)
                .field_focus(&s.machines.project_path, p)
                .color(p.ink)
                .on_edit(|s: &mut State| &mut s.machines.project_path),
        )
        .child(
            row()
                .gap(8.0)
                .child(spacer())
                .child(button("remote-project-cancel", "Cancel", p, |s| {
                    s.project_picker = None
                }))
                .child(
                    primary_button("remote-project-add", "Add workspace", p, |s| {
                        let path = std::path::PathBuf::from(&s.machines.project_path.text);
                        match s.add_project(path) {
                            Ok(_) => s.project_picker = None,
                            Err(error) => s.machines.notice = Some(error),
                        }
                    })
                    .disabled(!s.machine_connected(&s.machines.selected)),
                ),
        );
    el().key("remote-project-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .fill(Color::BLACK.alpha(0.28))
        .modal(false, |s: &mut State| s.project_picker = None)
        .child(card)
}

/// What a tool call does, for its one-line row: icon, past and present verb, and what it
/// acts on. Reads, listings and searches count as exploring when grouped, as Cursor does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    Read,
    List,
    Search,
    Edit,
    Run,
    Other,
}

fn tool_kind(tool: &Tool) -> ToolKind {
    use crate::work::ToolKind as Kind;
    match tool.resolved_kind() {
        Kind::Read => ToolKind::Read,
        Kind::FileChange => ToolKind::Edit,
        Kind::Command => ToolKind::Run,
        Kind::FileSearch | Kind::WebSearch => ToolKind::Search,
        Kind::List => ToolKind::List,
        _ => ToolKind::Other,
    }
}

/// The verb for a call, in the tense of its state: "Reading" while it runs, "Read" after.
fn tool_verb(tool: &Tool) -> String {
    let done = tool.done || tool.failed;
    match (tool_kind(tool), done) {
        (ToolKind::Read, true) => "Read".into(),
        (ToolKind::Read, false) => "Reading".into(),
        (ToolKind::List, true) => "Listed".into(),
        (ToolKind::List, false) => "Listing".into(),
        (ToolKind::Search, true) => "Searched".into(),
        (ToolKind::Search, false) => "Searching".into(),
        (ToolKind::Edit, true) => "Edited".into(),
        (ToolKind::Edit, false) => "Editing".into(),
        (ToolKind::Run, true) => "Ran".into(),
        (ToolKind::Run, false) => "Running".into(),
        (ToolKind::Other, true) => format!("Called {}", tool.name),
        (ToolKind::Other, false) => format!("Calling {}", tool.name),
    }
}

fn tool_title(tool: &Tool) -> String {
    let verb = tool_verb(tool);
    if tool.detail.is_empty() {
        verb
    } else {
        format!("{verb} {}", tool.detail)
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// One line for a batch: "Explored 3 files, 2 searches", "Edited 1 file", "Ran 2 commands".
fn tool_summary(tools: &[&Tool]) -> String {
    let count = |kind: ToolKind| tools.iter().filter(|t| tool_kind(t) == kind).count();
    let (read, list, search) = (
        count(ToolKind::Read),
        count(ToolKind::List),
        count(ToolKind::Search),
    );
    let mut parts = vec![];
    if read + list + search > 0 {
        let mut explored = vec![];
        if read > 0 {
            explored.push(plural(read, "file", "files"));
        }
        if list > 0 {
            explored.push(plural(list, "directory", "directories"));
        }
        if search > 0 {
            explored.push(plural(search, "search", "searches"));
        }
        // A batch of reads alone reads as plain "Read 2 files".
        let verb = if list + search == 0 {
            "Read"
        } else if read + search == 0 {
            "Listed"
        } else if read + list == 0 {
            "Searched"
        } else {
            "Explored"
        };
        parts.push(if verb == "Searched" {
            format!("Ran {}", explored.join(", "))
        } else {
            format!("{verb} {}", explored.join(", "))
        });
    }
    for (kind, verb, one, many) in [
        (ToolKind::Edit, "Edited", "file", "files"),
        (ToolKind::Run, "Ran", "command", "commands"),
        (ToolKind::Other, "Called", "tool", "tools"),
    ] {
        let n = count(kind);
        if n > 0 {
            parts.push(format!("{verb} {}", plural(n, one, many)));
        }
    }
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part.clone()
            } else {
                part.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Turn and thinking durations, as Cursor writes them: 45s, 2m 5s, 1h 3m.
fn elapsed_label(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TranscriptPart {
    Text {
        message: usize,
        from: usize,
        to: usize,
    },
    Tools(Vec<usize>),
    Thought(usize),
}

/// Content indices preserve thinking/tool order even when neither adds visible message text.
fn turn_parts(t: &Thread, start: usize, end: usize) -> Vec<TranscriptPart> {
    let mut events: Vec<_> = t
        .tools
        .iter()
        .enumerate()
        .filter(|(_, tool)| (start..end).contains(&tool.anchor.message))
        .map(|(index, tool)| (tool.anchor, tool.block, false, index))
        .collect();
    events.extend(
        t.thoughts
            .iter()
            .enumerate()
            .filter(|(_, thought)| (start..end).contains(&thought.anchor.message))
            .map(|(index, thought)| (thought.anchor, thought.block, true, index)),
    );
    events.sort_by_key(|(anchor, block, _, _)| (*anchor, *block));
    let mut events = events.into_iter().peekable();
    let mut parts = vec![];
    for message in start..end {
        let text = &t.messages[message].text;
        let mut from = 0;
        let mut previous_anchor = None;
        while events
            .peek()
            .is_some_and(|event| event.0.message == message)
        {
            let (anchor, _, thinking, index) = events.next().unwrap();
            let to = if anchor.offset >= from && text.is_char_boundary(anchor.offset) {
                anchor.offset
            } else {
                from
            };
            if to > from {
                parts.push(TranscriptPart::Text { message, from, to });
            }
            if thinking {
                parts.push(TranscriptPart::Thought(index));
            } else if previous_anchor == Some(anchor)
                && let Some(TranscriptPart::Tools(indices)) = parts.last_mut()
            {
                indices.push(index);
            } else {
                parts.push(TranscriptPart::Tools(vec![index]));
            }
            previous_anchor = Some(anchor);
            from = to;
        }
        if from < text.len() {
            parts.push(TranscriptPart::Text {
                message,
                from,
                to: text.len(),
            });
        }
    }
    parts
}

/// Secondary and tertiary transcript inks: a tool's verb, and what it acted on.
fn quiet_inks(p: Palette) -> (Color, Color) {
    (p.muted, p.faint)
}

/// Ported from KSG Code's WorkLogLine: a 24 px icon slot, a label and adjacent disclosure.
fn activity_line(
    key: String,
    a11y: String,
    verb: String,
    detail: &str,
    failed: bool,
    open: bool,
    glyph: &str,
    p: Palette,
) -> El {
    let (secondary, tertiary) = quiet_inks(p);
    let mut line = row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(a11y)
        .focusable(true)
        .center_y()
        .gap(5.0)
        .min_h(26.0)
        .px(6.0)
        .ml(-6.0)
        .radius(4.0)
        .hover(p.selected)
        .child(
            el().size(24.0, 24.0)
                .center()
                .child(tool_icon(glyph, secondary)),
        )
        // The verb is Departure Mono in sentence case; what it acted on stays Inter.
        .child(pixel_text(verb, if failed { p.danger } else { secondary }));
    if !detail.is_empty() {
        line = line.child(
            label(detail, ACTIVITY, tertiary)
                .truncate()
                .shrink()
                .min_w(0.0),
        );
    }
    if failed {
        line = line.child(pixel_text("failed", p.danger));
    }
    line.child(icon(if open { "chevron" } else { "open" }, tertiary).opacity(0.8))
        .child(spacer())
}

fn inline_tool(s: &State, p: Palette, index: usize) -> El {
    let t = s.current();
    let tool = &t.tools[index];
    let title = tool_title(tool);
    let mut content = col().gap(4.0).child(
        activity_line(
            format!("inline-tool-{}-{index}", t.id),
            format!(
                "{} output for {title}",
                if tool.expanded { "Hide" } else { "Show" }
            ),
            tool_verb(tool),
            &tool.detail,
            tool.failed,
            tool.expanded,
            match tool_kind(tool) {
                ToolKind::Read => "eye",
                ToolKind::List => "folder",
                ToolKind::Search => "search",
                ToolKind::Edit => "edit",
                ToolKind::Run => "terminal",
                ToolKind::Other => "wrench",
            },
            p,
        )
        .on_press(move |s: &mut State| {
            let tool = &mut s.current_mut().tools[index];
            tool.expanded = !tool.expanded;
        }),
    );
    if !tool.subagents.is_empty() {
        content = content.child(pi_ui::subagent_results(tool, p));
    }
    if tool.expanded {
        content = content.child(
            col()
                .key(format!("tool-output-{}-{index}", t.id))
                .role("group")
                .label(format!("Output for {title}"))
                .mb(4.0)
                .ml(28.0)
                .radius(6.0)
                .fill(p.code_bg)
                .border(1.0, p.line)
                .child(
                    label(&tool.detail, SMALL, p.muted)
                        .wrap_anywhere()
                        .selectable(true)
                        .px(12.0)
                        .pt(9.0)
                        .pb(7.0),
                )
                .child(rule(p))
                .child(
                    col()
                        .max_h(200.0)
                        .px(12.0)
                        .py(9.0)
                        .scroll(ScrollSpec {
                            stick_bottom: !tool.done,
                        })
                        .child(
                            label(
                                if tool.output.is_empty() {
                                    if tool.done {
                                        "No output"
                                    } else {
                                        "Waiting for output"
                                    }
                                } else {
                                    &tool.output
                                },
                                TextStyle::new(12.0, 400.0)
                                    .family(rune::Family::SystemMono)
                                    .lh(18.0),
                                if tool.output.is_empty() {
                                    p.muted
                                } else {
                                    p.ink
                                },
                            )
                            .wrap_anywhere()
                            .selectable(true),
                        ),
                ),
        );
    }
    if tool.expanded {
        if let Some(exit) = tool.exit_code {
            content = content.child(label(format!("Exit code {exit}"), SMALL, p.muted));
        }
        if let Some(parent) = &tool.parent_call {
            content =
                content.child(label(format!("Called by {parent}"), SMALL, p.muted).wrap_anywhere());
        }
        if !tool.images.is_empty() {
            content = content.child(message_images(
                s,
                p,
                &tool.images,
                &format!("tool-image-{}-{index}", t.id),
            ));
        }
        if tool.subagents.is_empty()
            && tool
                .structured
                .as_object()
                .is_some_and(|data| data.values().any(|v| !v.is_null()))
        {
            content = content.child(
                col()
                    .max_h(200.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(
                        label(
                            truncate(
                                &serde_json::to_string_pretty(&tool.structured).unwrap_or_default(),
                                50_000,
                            ),
                            SMALL,
                            p.ink,
                        )
                        .wrap_anywhere()
                        .selectable(true),
                    ),
            );
        }
    }
    content
}

fn inline_tools(s: &State, p: Palette, indices: &[usize]) -> El {
    if indices.len() == 1 {
        return inline_tool(s, p, indices[0]);
    }
    let t = s.current();
    let tools: Vec<_> = indices.iter().map(|index| &t.tools[*index]).collect();
    let first = indices[0];
    let expanded = tools[0].group_expanded;
    let failed = tools.iter().filter(|tool| tool.failed).count();
    let summary = tool_summary(&tools);
    let mut group = col()
        .key(format!("tool-group-{}-{first}", t.id))
        .gap(2.0)
        .child(
            activity_line(
                format!("toggle-tool-group-{}-{first}", t.id),
                format!("{} {summary}", if expanded { "Collapse" } else { "Expand" }),
                summary.clone(),
                "",
                failed > 0,
                expanded,
                "list",
                p,
            )
            .on_press(move |s: &mut State| {
                let tool = &mut s.current_mut().tools[first];
                tool.group_expanded = !tool.group_expanded;
            }),
        );
    if expanded {
        let mut rows = col().pl(14.0).gap(0.0);
        for index in indices {
            rows = rows.child(inline_tool(s, p, *index));
        }
        group = group.child(rows);
    }
    group
}

fn inline_thought(s: &State, p: Palette, index: usize) -> El {
    let t = s.current();
    let thought = &t.thoughts[index];
    let timing = thought.duration_ms.or_else(|| {
        (!thought.done)
            .then(|| {
                thought
                    .started
                    .map(|start| start.elapsed().as_millis() as u64)
            })
            .flatten()
    });
    let (verb, detail) = match (thought.done, timing) {
        (true, Some(ms)) if ms < 500 => ("Thought", "briefly".to_string()),
        (true, Some(ms)) => ("Thought", elapsed_label(ms / 1000)),
        (false, Some(ms)) => ("Thinking", elapsed_label(ms / 1000)),
        (true, None) => ("Thought", String::new()),
        (false, None) => ("Thinking", String::new()),
    };
    let mut content = col().gap(4.0).child(
        activity_line(
            format!("inline-thought-{}-{index}", t.id),
            format!(
                "{} thinking text",
                if thought.expanded { "Hide" } else { "Show" }
            ),
            verb.into(),
            &detail,
            false,
            thought.expanded,
            "brain",
            p,
        )
        .on_press(move |s: &mut State| {
            let thought = &mut s.current_mut().thoughts[index];
            thought.expanded = !thought.expanded;
        }),
    );
    if thought.expanded {
        // Thinking reads as a quiet aside, not a boxed output.
        content = content.child(
            col()
                .key(format!("thought-output-{}-{index}", t.id))
                .role("group")
                .label("Thinking text")
                .pl(28.0)
                .mb(4.0)
                .max_h(220.0)
                .scroll(ScrollSpec {
                    stick_bottom: !thought.done,
                })
                .child(
                    label(
                        if thought.text.is_empty() {
                            "Pi did not provide visible thinking text."
                        } else {
                            &thought.text
                        },
                        TextStyle::new(13.0, 400.0).lh(20.0),
                        p.faint,
                    )
                    .wrap_anywhere()
                    .selectable(true),
                ),
        );
    }
    content
}

fn transcript_part(s: &State, p: Palette, wide: bool, part: &TranscriptPart) -> El {
    let t = s.current();
    match part {
        TranscriptPart::Tools(indices) => crew_ui::tool_part(s, p, indices),
        TranscriptPart::Thought(index) => inline_thought(s, p, *index),
        TranscriptPart::Text { message, from, to } => {
            let chunk = &t.messages[*message].text[*from..*to];
            let message_index = *message;
            let offset = *from;
            let thread_id = t.id;
            let streaming = t.status == Status::Running && message + 1 == t.messages.len();
            memo(
                format!("message-{}-{message}-body-{from}", t.id),
                (chunk, s.dark, streaming, wide),
                || {
                    markdown_ui::render(
                        chunk,
                        p,
                        s.dark,
                        wide,
                        false,
                        streaming,
                        &format!("{thread_id}-{message_index}-{offset}"),
                    )
                },
            )
        }
    }
}

/// What the agent is doing right now, for the live status line at the end of a turn. A
/// running tool or thought already names itself in its own row, so the line says Working.
fn live_status(t: &Thread) -> String {
    if t.tools.iter().any(|tool| !tool.done && !tool.failed)
        || t.thoughts.last().is_some_and(|thought| !thought.done)
    {
        return "Working".into();
    }
    match t.phase.as_str() {
        "" | "Working" | "Waiting for the model" => "Planning next moves".into(),
        phase if phase.starts_with("Running ") => "Working".into(),
        phase => phase.into(),
    }
}

/// How far a turn's tool rows and its live status words sit in from the prose lane's edge. The
/// live status square is 6 px, and 8 px of gap puts its label on the same line.
const WORK_INDENT: f32 = 14.0;

fn assistant_turn(s: &State, p: Palette, wide: bool, start: usize, end: usize) -> El {
    let t = s.current();
    let parts = turn_parts(t, start, end);
    // A captain's delegations are its answer, not its working: their task cards stay in view
    // when the rest of the turn folds into "Worked for".
    let card = |part: &TranscriptPart| crew_ui::is_card_part(s, t, part);
    let last_work = parts
        .iter()
        .rposition(|part| !matches!(part, TranscriptPart::Text { .. }) && !card(part));
    // Initial thinking may precede the preamble on the wire. Keep that thinking in
    // the work section, and the first text before subsequent work above its fold.
    let preamble = last_work.and_then(|last| {
        parts[..last]
            .iter()
            .position(|part| matches!(part, TranscriptPart::Text { .. }))
    });
    let live = end == t.messages.len() && t.status.busy();
    let work = t.work.iter().find(|work| work.start == start);
    let mut response = col()
        .key(format!("assistant-turn-{}-{start}", t.id))
        .gap(12.0);
    // Captains sign their turns. A lone Pi thread needs no speaker label.
    if let Some(captain) = t.captain {
        response = response.child(
            row()
                .center_y()
                .gap(8.0)
                // The captain's resting face, as its sidebar portrait shows when idle. A turn's
                // own status ("Done", "Working") would change the expression and the mark.
                .child(speaker_avatar(s, p, captain))
                .child(label(
                    format!("Captain {}", s.captains[captain].name),
                    TextStyle::new(14.0, 600.0),
                    p.ink,
                )),
        );
    }
    if let Some(index) = preamble {
        response = response.child(transcript_part(s, p, wide, &parts[index]));
    } else if live && last_work.is_none() {
        // Until Pi emits work, streamed text is the preamble.
        for part in &parts {
            response = response.child(transcript_part(s, p, wide, part));
        }
    }
    let work_rows = |mut rows: El, lift_cards: bool| {
        if let Some(last) = last_work {
            for (index, part) in parts[..=last].iter().enumerate() {
                if Some(index) != preamble && !(lift_cards && card(part)) {
                    let row = transcript_part(s, p, wide, part);
                    // Prose between tool rows keeps its paragraph rhythm and the lane's one
                    // left margin. Tool rows are indented as a group by the same amount while
                    // the turn runs and after it ends, so nothing jumps when it finishes.
                    rows = rows.child(if matches!(part, TranscriptPart::Text { .. }) {
                        row.py(6.0)
                    } else {
                        row.ml(WORK_INDENT)
                    });
                }
            }
        }
        rows
    };
    if live {
        // While Pi works, its steps show as they happen, and one status line says what it
        // is doing now. The turn folds into "Worked for" when it ends.
        let mut rows = work_rows(
            col().key(format!("work-rows-{}-{start}", t.id)).gap(2.0),
            false,
        );
        let seconds = t.started.map(|start| start.elapsed().as_secs());
        rows = rows.child(
            row()
                .key(format!("live-status-{}", t.id))
                .min_h(26.0)
                // The square sits on the prose lane's left edge and the words start one
                // indent in, where the tool rows above do.
                .center_y()
                .gap(WORK_INDENT - 6.0)
                .child(status_mark(Status::Running, p, 6.0))
                .child(
                    row()
                        .gap(5.0)
                        .center_y()
                        .min_w(0.0)
                        .shrink()
                        .child(
                            pixel_text(live_status(t), p.muted)
                                .key(format!("live-status-text-{}", t.id))
                                .truncate()
                                .shrink()
                                .min_w(0.0),
                        )
                        // The phase gives way before the timer, whose tabular figures and
                        // padded seconds keep each tick the same width.
                        .child(match seconds {
                            Some(seconds) => label(timer_label(seconds), TIMER, p.sea)
                                .key(format!("live-timer-{}", t.id)),
                            None => el(),
                        }),
                ),
        );
        response = response.child(rows);
    } else if last_work.is_some() {
        let expanded = work.is_some_and(|work| work.expanded);
        let title = match work.and_then(|work| work.elapsed) {
            Some(seconds) => format!("Worked for {}", elapsed_label(seconds)),
            None => "Worked".into(),
        };
        let mut section = col()
            .key(format!("work-section-{}-{start}", t.id))
            .gap(4.0)
            .child(
                activity_line(
                    format!("toggle-work-{}-{start}", t.id),
                    format!(
                        "{} work section. {title}",
                        if expanded { "Collapse" } else { "Expand" }
                    ),
                    title.clone(),
                    "",
                    false,
                    expanded,
                    "clock",
                    p,
                )
                .on_press(move |s: &mut State| {
                    let t = s.current_mut();
                    match t.work.iter_mut().find(|work| work.start == start) {
                        Some(work) => work.expanded = !work.expanded,
                        // Turns from before work records existed fold too; opening one
                        // records it.
                        None => t.work.push(crate::model::AssistantWork {
                            start,
                            expanded: true,
                            elapsed: None,
                        }),
                    }
                }),
            );
        if expanded {
            // Tool rows nest under the "Worked for" line; prose stays on the lane's margin.
            section = section.child(work_rows(
                col().key(format!("work-rows-{}-{start}", t.id)).gap(2.0),
                true,
            ));
        }
        response = response.child(section);
        if let Some(last) = last_work {
            for part in parts[..=last].iter().filter(|part| card(part)) {
                response = response.child(transcript_part(s, p, wide, part));
            }
        }
    }
    if !live || last_work.is_some() {
        for part in &parts[last_work.map_or(0, |index| index + 1)..] {
            response = response.child(transcript_part(s, p, wide, part));
        }
    }
    response
}

/// The transcript, laid out in KSG Code's chat lane (`MessagesTimeline`'s
/// `.chat-content-lane` inside `.messages-timeline-scroll`): 46 rem of content, centred, with
/// 0.75 rem gutters below 640 px and 3 rem from there up. `pane_width` is the chat column's.
fn conversation(s: &State, p: Palette, wide: bool, pane_width: f32) -> El {
    let t = s.current();
    let gutter = lane_gutter(pane_width);
    let mut messages = col()
        .key("chat-lane")
        .w_full()
        .max_w(measure(wide))
        // 24 pt above the first message, counting the cap.
        .pt(24.0 - TRANSCRIPT_CAP)
        .pb(24.0)
        .gap(20.0);
    // A captain's delegations show as task cards inside a turn, so with no messages they have
    // nothing to hang from. The home counts as empty and greets above its crew tray.
    let visible_tools = t
        .tools
        .iter()
        .any(|tool| !(s.is_captain_home(t) && crew_ui::is_delegation(tool)));
    if t.messages.is_empty() && !visible_tools && !t.status.busy() {
        let mut introduction = col()
            .key("empty-state")
            .w_full()
            .max_w(measure(wide) + 2.0 * gutter)
            .px(gutter)
            .pb(12.0)
            .gap(16.0);
        if let Some(captain) = t.captain {
            introduction = introduction
                .items_center()
                .pb(16.0)
                .child(captain_hero(s, p, captain, wide, pane_width));
        } else {
            introduction = introduction
                .items_center()
                .px(0.0)
                .pb(16.0)
                .gap(24.0)
                .child(horizon_band(
                    lane_width(pane_width, wide),
                    s.dark,
                    crate::horizon::Subject::Boat,
                    fleet_size(s),
                    p,
                ))
                .child(draft_headline(s, p, pane_width));
        }
        if t.restoring_history() {
            introduction = introduction.child(label("Restoring conversation...", SMALL, p.muted));
        }
        return introduction;
    } else {
        let cutoff = t.messages.len().saturating_sub(100);
        // Do not cut through an assistant turn and hide the tools that led to its answer.
        let start = t.messages[..cutoff]
            .iter()
            .rposition(|message| message.role != "Pi")
            .unwrap_or(0);
        if start > 0 {
            messages = messages.child(label("Showing recent turns.", SMALL, p.muted));
        }
        let mut index = start;
        let mut hidden_images = t.messages[start..]
            .iter()
            .map(|message| message.images.len())
            .sum::<usize>()
            .saturating_sub(32);
        while index < t.messages.len() {
            let message = &t.messages[index];
            if message.role == "Pi" {
                let end = t.messages[index..]
                    .iter()
                    .position(|message| message.role != "Pi")
                    .map_or(t.messages.len(), |offset| index + offset);
                messages = messages.child(assistant_turn(s, p, wide, index, end));
                index = end;
                continue;
            }
            let key = format!("message-{}-{index}", t.id);
            let body = if message.role == "You" {
                // KSG Code's `TimelineUserMessageRow`: `max-w-[80%] rounded-2xl p-3`.
                let max_width = (lane_width(pane_width, wide) * 0.8).max(160.0);
                let mut user = col()
                    .items_end()
                    .gap(6.0)
                    .max_w(max_width)
                    .shrink()
                    .min_w(0.0);
                if !message.images.is_empty() {
                    let skip = hidden_images.min(message.images.len());
                    hidden_images -= skip;
                    if skip > 0 {
                        user = user.child(label(
                            format!("{skip} older image previews omitted."),
                            SMALL,
                            p.muted,
                        ));
                    }
                    user = user.child(message_images(s, p, &message.images[skip..], &key));
                }
                if !message.text.trim().is_empty() {
                    user = user.child(
                        memo(
                            format!("{key}-markdown"),
                            (&message.text, s.dark, wide),
                            || {
                                markdown_ui::render(
                                    &message.text,
                                    p,
                                    s.dark,
                                    wide,
                                    true,
                                    false,
                                    &format!("user-{}-{index}", t.id),
                                )
                            },
                        )
                        .key(format!("{key}-text"))
                        .max_w(max_width)
                        .min_w(0.0)
                        .padding(12.0)
                        .radius(6.0)
                        .fill(p.message_bg)
                        // Both themes outline the bubble in `frame`: 2 px at night, 1 px by day (the
                        // `line` colour disappeared on the `message_bg` fill). No shadow.
                        .border(if s.dark { 2.0 } else { 1.0 }, p.frame),
                    );
                }
                row().justify_end().child(user)
            } else if let Some(completion) = s
                .is_captain_home(t)
                .then(|| crew_ui::parse_completion(message.display_text()))
                .flatten()
            {
                crew_ui::result_block(s, p, wide, &completion)
            } else if let Some(brief) = (message.role == "Shina" && t.delegated)
                .then(|| crew_ui::parse_brief(message.display_text()))
                .flatten()
            {
                crew_ui::brief_block(s, p, wide, brief)
            } else {
                col()
                    .gap(4.0)
                    .px(12.0)
                    .py(8.0)
                    .radius(6.0)
                    .fill(p.coral_bg)
                    .child(label("Shina", TextStyle::new(13.0, 600.0), p.ink))
                    .child(
                        label(message.display_text(), body_style(wide), p.ink)
                            .wrap_anywhere()
                            .selectable(true),
                    )
            };
            messages = messages.child(body.key(key));
            index += 1;
        }
        messages = messages.child(pi_ui::compaction_activity(s, p));
        if t.status.busy() && t.messages.last().is_none_or(|message| message.role != "Pi") {
            messages = messages.child(assistant_turn(
                s,
                p,
                wide,
                t.messages.len(),
                t.messages.len(),
            ));
        }
    }
    col()
        .key(format!("conversation-{}", t.id))
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: !t.messages.is_empty() || !t.tools.is_empty(),
        })
        // A transcript longer than the pane rests stuck to its bottom, so its top is always
        // under the header's rule. An opaque 8 pt cap in the panel colour sits right under the
        // rule, stuck to the viewport's top, and the 28 pt fade starts below it, so nothing
        // scrolled shows through as a sliver under the rule.
        .scroll_fade(rune::ScrollFade::strength(28.0).below_sticky())
        .px(gutter)
        .items_center()
        .child(
            el().key("transcript-cap")
                .w_full()
                .h(TRANSCRIPT_CAP)
                .fill(p.panel)
                .sticky_top(0.0)
                .a11y_hidden(),
        )
        .child(messages)
}

/// The opaque strip under the chat header's rule, inside the transcript viewport.
const TRANSCRIPT_CAP: f32 = 8.0;

/// The sidebar portraits' animation stage also plays the captain hero, under this row id.
pub(crate) const HERO_PORTRAIT_ROW: usize = usize::MAX;
const HERO_PORTRAIT: f32 = 72.0;
/// The captain hero with a one-line heading: band, portrait overlap, heading, role and status.
const CAPTAIN_HERO_HEIGHT: f32 = 272.0;

/// An empty captain home greets like KSG Code's draft hero (`DraftHeroHeadline`: centred
/// `text-3xl font-normal tracking-tight`, `text-2xl` on a narrow pane), with the captain's
/// animated portrait above it. A home with a goal shows the goal; otherwise the captain asks
/// what to look after. The live status keeps one 20 px line, so it never moves the composer.
fn captain_hero(s: &State, p: Palette, captain: usize, wide: bool, pane_width: f32) -> El {
    let t = s.current();
    let profile = &s.captains[captain];
    let status = shown_status(s);
    let face = face_for(status);
    let portrait_index = captain_portrait(s, Some(captain));
    let pixels = s
        .portraits
        .pixels(portrait_index, face, HERO_PORTRAIT)
        .unwrap_or_else(|| s.artwork.face_pixels(portrait_index, face, HERO_PORTRAIT));
    let portrait = pixel_portrait(s, &pixels, HERO_PORTRAIT, p)
        .key("captain-hero-portrait")
        .a11y_hidden()
        .on_visible(|s: &mut State, on, tx| {
            s.portraits.set_visible(HERO_PORTRAIT_ROW, on);
            tx.live_only();
            tx.wake_after(std::time::Duration::ZERO);
        });
    let heading = if s.is_captain_home(t) && t.renamed {
        t.title.clone()
    } else {
        format!("What should {} look after?", profile.name)
    };
    let narrow = lane_width(pane_width, true) < 560.0;
    let working = s
        .threads
        .iter()
        .filter(|thread| {
            thread.parent == Some(t.id) && !thread.archived() && thread.status == Status::Running
        })
        .count();
    let (key, line_status, text) = if working > 0 {
        (
            "captain-working-count",
            Status::Running,
            Some(format!(
                "{working} {} working",
                if working == 1 { "thread" } else { "threads" }
            )),
        )
    } else if matches!(status, Status::Ready | Status::Completed) {
        ("captain-status", status, None)
    } else {
        ("captain-status", status, Some(status.label().to_string()))
    };
    let mut status_line = row().key("captain-status-line").h(20.0).center_y();
    if let Some(text) = text {
        status_line = status_line.child(
            row()
                .key(key)
                .label(text.clone())
                .center_y()
                .gap(6.0)
                .opacity_motion(slider_curve(false))
                .child(status_mark(line_status, p, 6.0))
                .child(label(
                    text,
                    PIXEL.features(TABULAR),
                    status_ink(line_status, p),
                )),
        );
    }
    let role = profile
        .instructions
        .lines()
        .next()
        .filter(|line| !line.trim().is_empty())
        .map_or_else(|| profile.role.clone(), str::to_owned);
    // A profile banner: the portrait overlaps the band's bottom edge, the band's sky takes
    // the portrait's backdrop colour, and the boat flies it as a pennant. One distant sail
    // shows for each crew thread that works.
    let band = horizon_band(
        lane_width(pane_width, wide),
        s.dark,
        crate::horizon::Subject::Sky(crate::visuals::portrait_backdrop(portrait_index)),
        fleet_size(s),
        p,
    );
    col()
        .key("captain-hero")
        .w_full()
        .items_center()
        .gap(10.0)
        .child(band)
        .child(
            status_badge(p, portrait, HERO_PORTRAIT, status)
                .mt(-(HERO_PORTRAIT / 2.0) - 10.0)
                .mb(6.0),
        )
        .child(
            label(
                heading,
                if narrow { HEADING_NARROW } else { HEADING },
                p.ink,
            )
            .key("captain-goal-heading")
            .align(rune::Align::Center)
            .wrap()
            .w_full()
            .max_w(measure(true)),
        )
        .child(
            // Who and what they look after, as two labels with a gap between them.
            row()
                .key("captain-role")
                .label(format!("Captain {}, {role}", profile.name))
                .max_w(520.0)
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(
                    label(format!("Captain {}", profile.name), SMALL.lh(18.0), p.ink)
                        .shrink()
                        .min_w(0.0),
                )
                .child(
                    label(role, SMALL.lh(18.0), p.muted)
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                ),
        )
        .child(status_line)
}

/// The new-thread screen, laid out like KSG Code's draft hero: the composer sits at the
/// vertical centre of the pane and the headline rests 16 px above it. Two equal zero-height
/// boxes do the centring, so attachments or a long draft grow the composer without moving
/// the headline off it.
fn empty_thread(
    s: &State,
    p: Palette,
    narrow: bool,
    wide: bool,
    pane_width: f32,
    hero_fits: bool,
) -> El {
    // A captain's hero keeps its height above a tall crew tray and takes the room from the
    // space below the composer. In a window too short for both, the composer stays on screen
    // and the clip keeps the hero out of the header.
    let floor = if s.current().captain.is_some() && hero_fits {
        CAPTAIN_HERO_HEIGHT
    } else {
        0.0
    };
    let above = col()
        .w_full()
        .h(0.0)
        .grow(1.0)
        .min_h(floor)
        .clip_rounded(0.0)
        .justify_end()
        .items_center()
        .child(conversation(s, p, wide, pane_width));
    let below = col().w_full().h(0.0).grow(1.0).min_h(0.0);
    col()
        .key("empty-thread")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .items_center()
        .child(above)
        .child(composer(s, p, narrow, wide, pane_width))
        .child(below)
}

/// "What should we build in <project>?" with the project as an inline picker, as in KSG Code.
/// When the sentence does not fit on one line, the project moves under it at body size.
fn draft_headline(s: &State, p: Palette, pane_width: f32) -> El {
    // The same words as the thread's tab, so the headline and the tab never disagree.
    let name = s
        .project(s.current().project)
        .map(|project| project.name.clone())
        .unwrap_or_else(|| "No workspace".into());
    // Sans at 30 px averages about 15 px a character, and the line stays inside the lane.
    let available = lane_width(pane_width, true);
    // With no workspace there is nothing to build "in", so the sentence stays whole and the
    // picker sits under it.
    let has_workspace = s.project(s.current().project).is_some();
    let inline = has_workspace
        && pane_width > 0.0
        && ("What should we build in ?".len() + name.chars().count()) as f32 * 15.0 <= available;
    let picker = |style: TextStyle| {
        let mut picker = row()
            .key("draft-project")
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("Project: {name}. Choose a project"))
            .help(name.clone())
            .center_y()
            .pl(4.0)
            .pr(1.0)
            .radius(6.0)
            .shrink()
            .min_w(0.0)
            .hover(p.selected)
            .press(p.selected);
        // With no workspace the picker is a choice still to make: a folder icon, then the
        // words at weight 400 in the muted colour, unlike a real project's name.
        if !has_workspace {
            picker = picker
                .gap(6.0)
                .px(6.0)
                .child(icon("folder", p.muted).key("draft-project-icon"));
        }
        picker = picker.child(
            label(
                name.clone(),
                TextStyle {
                    weight: if has_workspace { 500.0 } else { 400.0 },
                    ..style
                },
                if has_workspace { p.ink } else { p.muted },
            )
            .key("draft-project-name")
            .truncate()
            .shrink()
            .min_w(0.0),
        );
        if !s.demo {
            picker = picker
                .focusable(true)
                .on_press(|s: &mut State| s.open_picker(ProjectPicker::NewThread));
        }
        picker
    };
    if inline {
        row()
            .key("empty-heading")
            .w_full()
            .justify_center()
            .center_y()
            .child(label("What should we build in", HEADING, p.ink))
            .child(picker(HEADING))
            .child(label("?", HEADING, p.ink))
    } else {
        col()
            .key("empty-heading")
            .w_full()
            .items_center()
            .gap(6.0)
            .child(
                label("What should we build?", HEADING_NARROW, p.ink)
                    .align(rune::Align::Center)
                    .wrap()
                    .w_full(),
            )
            .child(picker(TextStyle::new(16.0, 500.0).lh(24.0)).max_w(measure(true)))
    }
}

fn image_preview(
    s: &State,
    p: Palette,
    image: &Attachment,
    width: f32,
    height: f32,
    key: String,
) -> El {
    let mut preview = el()
        .key(key)
        .size(width, height)
        .radius(4.0)
        .clip_rounded(4.0)
        .border(0.5, p.line);
    if let Some(path) = crate::attachments::preview_path_for(image) {
        let handle = s.attachment_cache.file(path, width.max(height), 2.0);
        // Headless fixtures have no window waker to rebuild after asynchronous decode.
        if rune::is_headless() {
            let _ = handle.wait();
        }
        // Cover and crop, as `object-cover` does, so square tiles never stretch a screenshot.
        preview = preview.pixels_fit(handle.pixels(), rune::SurfaceFit::Fill);
    } else {
        preview = preview
            .center()
            .fill(p.selected)
            .child(if image.is_image() {
                icon("image", p.muted)
            } else {
                label(&image.name, TextStyle::new(10.0, 400.0), p.muted)
                    .wrap_anywhere()
                    .align(rune::Align::Center)
                    .padding(4.0)
            });
    }
    preview
}

fn image_size(image: &Attachment, maximum: f32) -> (f32, f32) {
    let ratio = image.width.max(1) as f32 / image.height.max(1) as f32;
    if ratio >= 1.0 {
        (maximum, (maximum / ratio).max(40.0))
    } else {
        ((maximum * ratio).max(40.0), maximum)
    }
}

/// A draft image as a bare 64 px thumbnail with a close button in its corner, ported from
/// KSG Code's `ComposerAttachmentStrip`. The file name lives in the tooltip and the label.
const ATTACHMENT_THUMB: f32 = 64.0;
fn attachment_chip(s: &State, p: Palette, image: &Attachment) -> El {
    let id = image.id.clone();
    el().key(format!("attachment-{}", image.id))
        .role("group")
        .label(format!(
            "Attached {}: {}",
            if image.is_image() { "image" } else { "file" },
            image.name
        ))
        .help(image.name.clone())
        .size(ATTACHMENT_THUMB, ATTACHMENT_THUMB)
        .child(
            image_preview(
                s,
                p,
                image,
                ATTACHMENT_THUMB,
                ATTACHMENT_THUMB,
                format!("attachment-thumb-{id}"),
            )
            .radius(4.0)
            .clip_rounded(4.0)
            .a11y_hidden(),
        )
        .child(
            el().key(format!("remove-attachment-{id}"))
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!("Remove {}", image.name))
                .focusable(true)
                .pin(None, Some(4.0), Some(4.0), None)
                .size(18.0, 18.0)
                .center()
                .radius(4.0)
                .fill(Color::BLACK.alpha(0.6))
                .hover(Color::BLACK.alpha(0.8))
                .on_press(move |s: &mut State| s.remove_attachment(&id))
                .child(icon("close", Color::WHITE).size(12.0, 12.0)),
        )
}

fn message_images(s: &State, p: Palette, images: &[Attachment], key: &str) -> El {
    let mut strip = row()
        .justify_end()
        .items_end()
        .flex_wrap()
        .gap(6.0)
        .max_w(620.0);
    for (index, image) in images.iter().enumerate() {
        let (width, height) = image_size(image, if images.len() == 1 { 220.0 } else { 132.0 });
        strip = strip.child(
            image_preview(s, p, image, width, height, format!("{key}-image-{index}"))
                .role("group")
                .label(format!(
                    "Attached {}: {}",
                    if image.is_image() { "image" } else { "file" },
                    image.name
                )),
        );
    }
    strip
}

fn attach_control(locked: bool, p: Palette) -> El {
    let mut control = el()
        .key("attach-image")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label("Attach files or folders. Shift-click to choose a folder.")
        .help("Attach files. Shift-click for a folder.")
        .size(24.0, 24.0)
        .center()
        .radius(4.0)
        .opacity(if locked { 0.65 } else { 1.0 })
        .hover(p.selected)
        .disabled(locked)
        .child(icon("paperclip", p.muted));
    if !locked {
        control = control
            .focusable(true)
            .on_press_tx(|s: &mut State, tx: &mut Tx| pick_attachment(s, tx, tx.mods().shift));
    }
    control
}

fn pick_attachment(s: &mut State, tx: &mut Tx, folder: bool) {
    let thread = s.current().id;
    let options = rune::FileDialogOptions {
        title: if folder {
            "Attach a folder"
        } else {
            "Attach a file"
        }
        .into(),
        ..Default::default()
    };
    let callback = move |s: &mut State, result, _: &mut Tx| {
        match result {
            Ok(Some(path)) => s.attach_paths_to(thread, &[path]),
            Ok(None) => {}
            Err(error) => {
                if let Some(target) = s.threads.iter_mut().find(|t| t.id == thread) {
                    target.attachment_error =
                        Some(format!("Could not open the attachment picker: {error}"));
                }
            }
        }
        s.checkpoint();
    };
    if folder {
        tx.open_folder(options, callback);
    } else {
        tx.open_file(options, callback);
    }
}

const CONTEXT_RING_SIZE: f32 = 28.0;

fn context_limit(t: &Thread) -> Option<u64> {
    t.context.usage.limit(t.context_window)
}

/// The ring appears once Pi or the model catalog knows a capacity, or once Pi reports usage.
fn context_available(t: &Thread) -> bool {
    t.context.usage.has_usage() || context_limit(t).is_some()
}

/// A fresh draft has nothing to measure yet, so it shows no empty dashed ring.
fn context_ring_shown(t: &Thread) -> bool {
    context_available(t)
        && (t.context.usage.has_usage()
            || t.context.phase != ContextPhase::Idle
            || !t.messages.is_empty())
}

fn context_ink(percent: f64, p: Palette) -> Color {
    if percent >= 90.0 {
        p.danger
    } else if percent >= 70.0 {
        p.brass
    } else {
        p.sea
    }
}

/// Rune strokes a polyline, so a circle is a ring of points in the element's own points.
fn circle_points(cx: f32, cy: f32, radius: f32, steps: usize) -> Vec<[f32; 2]> {
    (0..steps.max(8))
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / steps.max(8) as f32;
            [cx + radius * angle.cos(), cy + radius * angle.sin()]
        })
        .collect()
}

fn arc_points(
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
fn context_ring_shape(context: &ContextMeter, p: Palette) -> Shape {
    let center = CONTEXT_RING_SIZE / 2.0;
    let radius = 7.5;
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
const CONTEXT_HOVER_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

/// Escape closes the usage popover from the ring or from inside it.
fn context_escape(s: &mut State, event: &rune::KeyEvent) -> bool {
    if event.key == rune::Key::Escape && s.current().context.popover_open() {
        s.current_mut().context.close_popover();
        true
    } else {
        false
    }
}

/// A press, Enter or Space on the ring opens the popover, or closes one it opened.
fn context_toggle(s: &mut State) {
    let context = &mut s.current_mut().context;
    if context.details_open {
        context.close_popover();
    } else {
        context.details_open = true;
    }
}

/// The composer's context meter, ported from KSG Code's `ContextWindowMeter` trigger: a
/// `size-7` ghost button that opens the usage popover on hover (150 ms in, 150 ms out), and on
/// a press, Enter or Space. The popover itself is drawn at the root by `context_popover`.
fn context_ring(t: &Thread, p: Palette) -> El {
    let limit = context_limit(t);
    let open = t.context.popover_open();
    el().key("context-usage-ring")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(t.context_harness_text(&t.context.accessible_label(limit)))
        .focusable(true)
        .expanded(open)
        .size(CONTEXT_RING_SIZE, CONTEXT_RING_SIZE)
        .center()
        .radius(CONTEXT_RING_SIZE / 2.0)
        .fill(if open { p.selected } else { Color::TRANSPARENT })
        .hover(p.selected)
        .on_press(context_toggle)
        .on_key(|s: &mut State, event, _| {
            if event.key == rune::Key::Enter {
                context_toggle(s);
                return true;
            }
            context_escape(s, event)
        })
        .on_hover_change(|s: &mut State, inside, _| s.current_mut().context.hover(true, inside))
        .hover_delays(CONTEXT_HOVER_DELAY, CONTEXT_HOVER_DELAY)
        .child(
            canvas(context_ring_shape(&t.context, p))
                .size(CONTEXT_RING_SIZE, CONTEXT_RING_SIZE)
                .a11y_hidden(),
        )
}

/// KSG Code's `formatPercentage`: one decimal under 10 %, whole numbers above.
fn context_percent_text(percent: f64) -> String {
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
fn context_tokens_text(value: u64) -> String {
    let trim = |text: String| text.strip_suffix(".0").map(str::to_owned).unwrap_or(text);
    match value {
        0..1_000 => value.to_string(),
        1_000..10_000 => format!("{}k", trim(format!("{:.1}", value as f64 / 1_000.0))),
        10_000..999_500 => format!("{}k", (value as f64 / 1_000.0).round() as u64),
        _ => format!("{}m", trim(format!("{:.1}", value as f64 / 1_000_000.0))),
    }
}

/// KSG Code's `formatContextWindowCost`: four decimals under a cent, two otherwise.
fn context_cost_text(cost: f64) -> String {
    if cost > 0.0 && cost < 0.01 {
        format!("USD {cost:.4}")
    } else {
        format!("USD {cost:.2}")
    }
}

/// The usage popover, ported from KSG Code's `ContextWindowMeter` popup (the branch without a
/// breakdown, which is what Pi reports): a `w-64` tooltip-style card above the ring, aligned to
/// its end, with "Context Window" and `percent · used/limit` in `text-2xs tabular-nums`, a
/// `h-1.5` usage bar, total processed and cost rows, the compaction state, and a full-width
/// "Compact context" button.
fn context_popover(s: &State, p: Palette) -> El {
    let t = s.current();
    let limit = context_limit(t);
    let usage = &t.context.usage;
    // Figures are Departure Mono with tabular digits; their names stay Inter.
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
        .w(256.0)
        .padding(12.0)
        .gap(8.0)
        .radius(6.0)
        .clip_rounded(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .anchored("context-usage-ring", rune::Side::Top, rune::Align::End, 4.0)
        .on_key(|s: &mut State, event, _| context_escape(s, event))
        .on_hover_change(|s: &mut State, inside, _| s.current_mut().context.hover(false, inside))
        .hover_delays(std::time::Duration::ZERO, CONTEXT_HOVER_DELAY)
        .child(
            row()
                .center_y()
                .gap(12.0)
                .child(pixel_label("Context window", p.muted))
                .child(spacer())
                .child(figures),
        );
    if let Some(limit) = limit
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
            .radius(2.0)
            .clip_rounded(2.0)
            .fill(p.line);
        if usage.percent.is_some() {
            bar = bar.child(el().w_frac(fraction).h_full().radius(2.0).fill(ink));
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
    if let Some(state) = t.context.state_text() {
        card = card.child(
            label(
                t.context_harness_text(state),
                TextStyle::new(11.0, 500.0).lh(16.0),
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
        .radius(4.0)
        .border(2.0, p.frame)
        .hover(p.selected)
        .press(p.selected)
        .opacity(if can_compact { 1.0 } else { 0.55 })
        .disabled(!can_compact)
        .child(icon("minimize", p.ink).size(14.0, 14.0))
        .child(label("Compact context", TextStyle::new(12.0, 500.0), p.ink));
    if can_compact {
        compact = compact.focusable(true).on_press(|s: &mut State| {
            s.current_mut().context.close_popover();
            s.compact_context(None);
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

/// The composer sits in KSG Code's `.chat-composer-lane`: the same gutters and 46 rem lane as
/// the messages above it, with `pb-4` under the footer.
/// The composer field's height with its 4 pt padding: one line at rest, so the body is 48 pt.
const COMPOSER_FIELD_MIN: f32 = 36.0;
/// KSG's `max-h-52`: a 200 pt text window inside the 4 pt padding, then the field scrolls.
const COMPOSER_FIELD_MAX: f32 = 208.0;
/// A composer that cannot send: demo data, an archived thread or an unreachable machine.
fn composer_disabled(s: &State) -> bool {
    s.demo || s.current().archived() || s.unreachable_selected().is_some()
}
/// The composer's 2 px frame: coral while its field has focus, from a click or from Tab, or a
/// file is over it; `frame` otherwise; and the lighter `line` when it cannot send.
fn composer_frame(s: &State, p: Palette) -> Color {
    if composer_disabled(s) {
        p.line
    } else if s.attachment_drop_hover || s.current().draft.focused {
        p.coral
    } else {
        p.frame
    }
}
fn composer(s: &State, p: Palette, narrow: bool, wide: bool, pane_width: f32) -> El {
    let t = s.current();
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
    }
    if let Some(error) = &t.attachment_error {
        area = area.child(
            label(error, SMALL, p.danger)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    if t.native.questionnaire.is_some() && t.request.is_some() {
        return area.child(pi_ui::questionnaire_drawer(s, p));
    }
    if t.request.is_some() {
        area = area.child(dialog(s, p));
    }
    let can_send = !s.demo
        && (s.client.is_none() || s.machine_connected(&s.machines.selected))
        && !t.initializing
        && !t.archived()
        && !t.configuring
        && t.request.is_none()
        && !t.native.operating()
        && t.phase != "Stopping"
        && t.has_draft_content();
    let mut send = el()
        .key("send-message")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(if t.status.busy() {
            if t.native.follow_up {
                "Queue message"
            } else {
                "Steer message"
            }
        } else {
            "Send message"
        })
        .size(32.0, 32.0)
        .radius(4.0)
        .center()
        // A framed key. Rune cannot move it on press, so a pressed key darkens instead.
        .press(if can_send { p.coral.alpha(0.8) } else { p.line })
        .fill(if can_send { p.coral } else { p.line })
        .child(icon("send", if can_send { p.on_coral } else { p.muted }))
        .disabled(!can_send);
    if can_send {
        send = framed(send, p);
    }
    if can_send {
        send = send.focusable(true).on_press_tx(|s: &mut State, tx| {
            let previous = s.current().native.follow_up;
            if tx.mods().cmd || tx.mods().ctrl {
                s.current_mut().native.follow_up = !previous;
            }
            s.send();
            s.current_mut().native.follow_up = previous;
        });
    }
    let action = if t.status.busy() && !s.demo && !t.has_draft_content() {
        framed(
            el().key("stop-agent")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label("Stop agent")
                .focusable(true)
                .size(32.0, 32.0)
                .radius(4.0)
                .center()
                .fill(p.coral)
                .press(p.coral.alpha(0.8))
                .on_press(|s: &mut State| s.current_mut().stop())
                .child(icon("stop", p.on_coral)),
            p,
        )
    } else {
        send
    };
    let locked = s.demo || t.archived() || t.status.busy() || t.configuring;
    let attach_locked =
        s.demo || t.archived() || t.configuring || t.native.operating() || t.request.is_some();
    // Delegation context sits above the composer, nearest first: who sent this worker, then
    // what the crew needs from the user, then what it is doing.
    let lane = lane_width(pane_width, wide);
    for part in [
        crew_ui::parent_bar(s, p, lane),
        crew_ui::needs_you(s, p),
        crew_ui::crew_tray(s, p, lane),
    ]
    .into_iter()
    .flatten()
    {
        area = area.child(part);
    }
    area = area.child(pi_ui::slash_menu(s, p));
    area = area.child(pi_ui::queued_messages(s, p));
    // KSG Code's composer: a 16 px card with a hairline outline and a 1 px shadow, text at
    // px-4 pt-4, and the controls in a px-2 pb-2 row so their hover fills sit near the edge.
    // A composer that cannot send has nothing to act on, so it drops the hard shadow. Its frame
    // keeps the same 2 px shape in the lighter `line` colour, so the box does not change size.
    let composer_disabled = composer_disabled(s);
    let mut card = col()
        .key("composer")
        .radius(6.0)
        .fill(p.card)
        .border(2.0, composer_frame(s, p));
    if !composer_disabled {
        card = card.box_shadow(hard_shadow(p));
    }
    card = card
        .on_drop_files(|s: &mut State, files, _| {
            s.attachment_drop_hover = false;
            s.attach_paths(files);
            s.checkpoint();
        })
        .on_drop_hover(|s: &mut State, over, _| s.attachment_drop_hover = over);
    if !t.draft_images.is_empty() {
        let mut tray = row()
            .key("composer-attachments")
            .gap(8.0)
            .flex_wrap()
            .px(12.0)
            .pt(12.0);
        for image in &t.draft_images {
            tray = tray.child(attachment_chip(s, p, image));
        }
        card = card.child(tray);
    }
    // Ported from KSG Code's composer body (`px-4 pt-4 pb-2`, `ChatComposer.tsx`) and its
    // editor (`max-h-52 overflow-y-auto p-1 -m-1`, `ComposerPromptEditorTiptap.tsx`): the field
    // grows with the draft up to a 200 pt text window, then scrolls inside its own viewport,
    // which Rune keeps on the caret after each edit. The viewport is what the card sizes to, so
    // the actions row always sits below the text and the card never grows past the cap. A bare
    // field with `max_h` laid out at the cap but let the card size to all of its text, which
    // drew the last lines under the actions and left an empty strip below them.
    let field = text_input(&t.draft, COMPOSER)
        .key("composer-input")
        .label("Message Pi")
        .placeholder(
            if s.demo {
                "Open a real workspace to send messages".to_string()
            } else if let Some(machine) = s.unreachable_selected() {
                format!("{machine} is unreachable. Reconnecting.")
            } else {
                "Ask anything. Shift Enter for a new line".to_string()
            },
            p.muted,
        )
        .color(p.ink)
        .caret(p.coral, p.coral_bg)
        .w_full()
        .min_h(COMPOSER_FIELD_MIN)
        .padding(4.0)
        .on_edit(|s: &mut State| {
            s.current_mut().native.slash_index = 0;
            &mut s.current_mut().draft
        })
        .on_input_key(|s: &mut State, event, _| pi_ui::composer_pi_key(s, event))
        .on_input_paste(|s: &mut State, content, _| {
            let has_image = content.image().is_some();
            if let Some(image) = content.image() {
                s.attach_image("Pasted image.png", image.png());
            }
            if !content.files().is_empty() {
                s.attach_paths(content.files());
            }
            (has_image || !content.files().is_empty()) && content.text().is_empty()
        })
        .on_submit(|s: &mut State, _| {
            if !s.demo && !s.current().archived() {
                s.send();
            }
        });
    card = card.child(
        col()
            .key("composer-body")
            .w_full()
            .px(12.0)
            .pt(10.0)
            .pb(2.0)
            // A press in the body's padding still lands in the field, as KSG's surface does.
            .on_press_tx(|_: &mut State, tx| tx.focus("composer-input", false))
            // The field inside is the Tab stop, not this.
            .focusable(false)
            .child(
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
                    .child(field),
            ),
    );
    let mut actions = row()
        .key("composer-actions")
        .center_y()
        .gap(2.0)
        .px(8.0)
        .pb(8.0)
        .child(
            attach_control(attach_locked, p)
                .size(28.0, 28.0)
                .radius(4.0),
        )
        .child(spacer())
        .child(model_effort_trigger(s, p, narrow, locked));
    if t.status.busy() && t.has_draft_content() {
        actions = actions
            .child(button(
                "dispatch-mode",
                if t.native.follow_up { "Queue" } else { "Steer" },
                p,
                |s| {
                    let n = &mut s.current_mut().native;
                    n.follow_up = !n.follow_up;
                },
            ))
            .child(icon_button("stop-agent", "Stop agent", "stop", p, |s| {
                s.current_mut().stop()
            }));
    }
    if context_ring_shown(t) {
        actions = actions.child(context_ring(t, p));
    }
    area = area.child(card.child(actions.child(action.ml(4.0))));
    // KSG Code's context strip: a bare muted row under the card, machine on the left and the
    // access mode on the right, inset to line up with the card's controls.
    let mut footer = row()
        .key("composer-footer")
        .min_h(if s.client.is_some() { 28.0 } else { 24.0 })
        .px(10.0)
        .flex_wrap()
        .center_y()
        .gap(6.0);
    footer = if s.demo {
        footer
            .child(pixel_text("Demo data", p.muted))
            .child(spacer())
            .child(
                row()
                    .key("leave-example")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Open a real workspace")
                    .focusable(true)
                    .center_y()
                    .h(20.0)
                    .px(6.0)
                    .radius(4.0)
                    .hover(p.coral_bg)
                    .on_press(|s: &mut State| {
                        s.leave_demo();
                    })
                    .child(pixel_text("Open a real workspace", p.sea)),
            )
    } else {
        footer.child(machine_selector(s, p)).child(spacer()).child(
            pixel_button("access-mode", t.native.access.label(), p, |s| {
                s.current_mut().native.access_picker = true
            })
            .disabled(t.status.busy() || t.configuring || t.archived()),
        )
    };
    area = area.child(footer);
    col()
        .key("composer-lane")
        .w_full()
        .px(lane_gutter(pane_width))
        .items_center()
        .child(area)
}

/// Ported from KSG Code's BranchToolbarEnvironmentSelector and ComposerControl size="xs".
fn machine_selector(s: &State, p: Palette) -> El {
    let title = s.machine_label(&s.machines.selected);
    row()
        .key("machine-selector")
        .role("button")
        .label("Run on")
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
        .radius(4.0)
        .hover(p.selected)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .disabled(s.client.is_none())
        .on_press(|s: &mut State| s.machines.picker = !s.machines.picker)
        .child(
            icon(
                if s.machines.selected == "local" {
                    "monitor"
                } else {
                    "cloud"
                },
                p.muted,
            )
            .size(12.0, 12.0),
        )
        .child(
            pixel_text(title, p.muted)
                .key("machine-selector-label")
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(icon("chevron", p.muted).size(12.0, 12.0))
}

/// Ported from KSG Code's BranchToolbarEnvironmentSelector SelectPopup/SelectGroup.
fn machine_picker(s: &State, p: Palette) -> El {
    let mut menu = col()
        .key("machine-picker")
        .role("menu")
        .focus_group()
        .label("Run on")
        .w(270.0)
        .max_h(360.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(4.0)
        .gap(1.0)
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .child(pixel_label("Run on", p.muted).px(8.0).py(6.0));
    let servers = std::iter::once("local".to_owned()).chain(
        s.client
            .iter()
            .flat_map(|client| client.linked.keys().cloned()),
    );
    for server in servers {
        let title = s.machine_label(&server);
        let key = format!("machine-option-{server}");
        menu = menu.child(
            menu_item(
                &key,
                &title,
                if server == "local" {
                    "monitor"
                } else {
                    "cloud"
                },
                p,
            )
            .on_press(move |s: &mut State| {
                s.select_machine(&server);
                if s.machines.selected == server && s.machine_connected(&server) {
                    s.new_thread_in(None);
                }
            }),
        );
    }
    menu = menu.child(
        menu_item("link-machine", "Add to another machine", "plus", p).on_press(|s: &mut State| {
            s.machines.picker = false;
            s.machines.dialog = true;
        }),
    );
    menu.anchored_fit("machine-selector", rune::Side::Top, rune::Align::Start, 6.0)
        .modal(true, |s: &mut State| s.machines.picker = false)
}

/// Ported from KSG Code's settings/AddEnvironmentDialog and AuthorizedClientsSettings.
fn machine_dialog(s: &State, p: Palette) -> El {
    let busy = s.machines.pending.is_some();
    let mut card = col().key("machine-dialog").role("dialog").label("Linked machines")
        .w((s.viewport_width-32.0).clamp(280.0,520.0)).max_h((s.viewport_height-64.0).max(240.0))
        .scroll(ScrollSpec {stick_bottom:false}).padding(16.0).gap(12.0).radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .child(row().center_y().child(label("Linked machines",DIALOG_TITLE,p.ink)).child(spacer())
            .child(icon_button("close-machines","Close","close",p,|s| {
                s.machines.dialog=false;s.machines.code.replace_text("");s.machines.notice=None;
            })))
        .child(label("Create a code on the other machine with shina pair. Paste its MagicDNS host and code here.",SMALL,p.muted).wrap())
        .child(pixel_label("Host",p.muted))
        .child(text_input(&s.machines.host,BODY).key("machine-host").label("Host").placeholder("studio-mac",p.muted)
            .single_line().w_full().padding(8.0).radius(6.0).field_focus(&s.machines.host, p).color(p.ink)
            .disabled(busy).on_edit(|s: &mut State| &mut s.machines.host))
        .child(pixel_label("Pairing code",p.muted))
        .child(text_input(&s.machines.code,BODY).key("machine-code").label("Pairing code").placeholder("PAIRCODE",p.muted)
            .single_line().w_full().padding(8.0).radius(6.0).field_focus(&s.machines.code, p).color(p.ink)
            .disabled(busy).on_edit(|s: &mut State| &mut s.machines.code))
        .child(primary_button("machine-link-submit","Link machine",p,|s| s.machine_action("link","")).disabled(busy))
        .child(row().flex_wrap().gap(8.0)
            .child(button("machine-pair","Create a code for this host",p,|s| s.machine_action("pair","")).disabled(busy))
            .child(button("machine-devices","Authorized devices",p,|s| s.machine_action("devices","")).disabled(busy)));
    if let Some(notice) = &s.machines.notice {
        card = card.child(
            label(notice, SMALL, p.muted)
                .key("machine-notice")
                .wrap_anywhere()
                .selectable(true),
        );
    }
    if let Some(client) = &s.client {
        for (id, remote) in &client.linked {
            let server = id.clone();
            card = card.child(
                row()
                    .center_y()
                    .gap(8.0)
                    .child(
                        label(
                            remote
                                .remote
                                .as_ref()
                                .map(|link| link.host.as_str())
                                .unwrap_or(id),
                            SMALL,
                            p.ink,
                        )
                        .grow(1.0)
                        .truncate()
                        .min_w(0.0),
                    )
                    .child(
                        button(&format!("unlink-{id}"), "Unlink and revoke", p, move |s| {
                            s.machine_action("unlink", &server)
                        })
                        .disabled(busy),
                    ),
            );
        }
    }
    for device in s.machines.devices.as_array().into_iter().flatten() {
        if !device["revokedAt"].is_null() {
            continue;
        }
        let id = device["deviceId"].as_str().unwrap_or("").to_owned();
        let title = format!("Device {}", &id[..id.len().min(8)]);
        card = card.child(
            row()
                .center_y()
                .gap(8.0)
                .child(label(title, SMALL, p.muted).grow(1.0))
                .child(
                    button(&format!("revoke-{id}"), "Revoke", p, move |s| {
                        s.machine_action("revoke", &id)
                    })
                    .disabled(busy),
                ),
        );
    }
    el().key("machines-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(0.28))
        .center()
        .modal(false, |s: &mut State| {
            s.machines.dialog = false;
            s.machines.code.replace_text("");
            s.machines.notice = None;
        })
        .child(card)
}

fn activity(s: &State, p: Palette, narrow: bool) -> El {
    let t = s.current();
    let mut pane = col().key("session-panel").min_h(0.0);
    pane = if narrow {
        pane.grow(1.0)
    } else {
        pane.w(300.0).h_full()
    };
    pane = pane.child(
        row()
            .key("session-panel-header")
            .h(36.0)
            .px(8.0)
            .center_y()
            .gap(4.0)
            .child(
                row()
                    .center_y()
                    .gap(6.0)
                    .px(8.0)
                    .h(25.0)
                    .radius(4.0)
                    .fill(p.selected)
                    .child(icon("thread", p.muted))
                    .child(label("Session", SMALL, p.ink)),
            )
            .child(spacer())
            .child(icon_button(
                "expand-session-panel",
                if s.activity_full {
                    "Restore split view"
                } else {
                    "Expand session details"
                },
                "expand",
                p,
                |s| s.activity_full = !s.activity_full,
            ))
            .child(icon_button(
                "close-session-panel",
                "Hide session details",
                "panel",
                p,
                |s| {
                    s.show_activity = false;
                    s.activity_full = false;
                },
            )),
    );
    // The same closing rule as the chat header, so one line runs across the pane.
    pane = pane.child(rule(p).key("session-header-rule"));
    let mut details = col()
        .padding(16.0)
        .gap(16.0)
        .child(label(&t.title, TextStyle::new(14.0, 560.0), p.ink).wrap())
        .child(status(t.status, p))
        .child(rule(p))
        .child(
            col()
                .gap(5.0)
                .child(pixel_label("Model", p.muted))
                .child(label(&t.model, LABEL, p.ink).wrap()),
        )
        .child(
            col()
                .gap(5.0)
                .child(pixel_label("Conversation", p.muted))
                .child(label(
                    if t.session.is_some() {
                        "Saved locally by Pi"
                    } else {
                        "No saved session yet"
                    },
                    LABEL,
                    p.ink,
                )),
        );
    if let Some(session) = &t.session {
        details = details.child(
            label(session, SMALL, p.muted)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    if !t.diagnostics.is_empty() {
        details = details
            .child(rule(p))
            .child(label("Session notices", LABEL, p.ink));
    }
    for diagnostic in &t.diagnostics {
        details = details.child(
            label(truncate(diagnostic, 600), SMALL, p.muted)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    for diagnostic in s.updater.diagnostics() {
        details = details.child(
            label(diagnostic, SMALL, p.muted)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    pane.child(
        col()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(details),
    )
}

fn dialog(s: &State, p: Palette) -> El {
    let t = s.current();
    let request = t.request.as_ref().expect("pending dialog");
    let mut card = col()
        .key("pi-dialog")
        .role("group")
        .label("Pi needs your input")
        .padding(13.0)
        .gap(10.0)
        .radius(6.0)
        .fill(p.coral_bg)
        .border(1.0, p.line)
        .child(label(crate::model::string(request, "title"), LABEL, p.ink).wrap())
        .child(label(crate::model::string(request, "message"), BODY, p.ink).wrap());
    let mut actions = row().gap(6.0).flex_wrap();
    match crate::model::string(request, "method") {
        "confirm" => {
            actions = actions
                .child(button("dialog-confirm", "Confirm", p, |s| {
                    s.current_mut().respond(json!({"confirmed":true}))
                }))
                .child(button("dialog-session", "Allow for session", p, |s| {
                    s.current_mut().approve_for_session()
                }));
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
                            move |s| s.current_mut().respond(json!({"value":value})),
                        ));
                    }
                }
            }
        }
        _ => {
            card = card.child(
                text_input(&t.reply, BODY)
                    .key("dialog-reply")
                    .label("Reply to Pi")
                    .placeholder(crate::model::string(request, "placeholder"), p.muted)
                    .color(p.ink)
                    .min_h(32.0)
                    .max_h(120.0)
                    .px(8.0)
                    .py(6.0)
                    .radius(6.0)
                    .fill(p.card)
                    .field_focus(&t.reply, p)
                    .on_edit(|s: &mut State| &mut s.current_mut().reply),
            );
            actions = actions.child(button("dialog-submit", "Submit reply", p, |s| {
                let value = s.current().reply.text.clone();
                s.current_mut().respond(json!({"value":value}));
            }));
        }
    }
    card.child(actions.child(button("dialog-cancel", "Cancel", p, |s| {
        s.current_mut().respond(json!({"cancelled":true}))
    })))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::context::ContextUsage;
    use rune::{Env, Runtime};
    fn runtime(width: f32) -> Runtime<State> {
        let mut rt = Runtime::with_text(
            State::new(".".into(), false),
            view,
            (width, 720.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
        );
        // As the app sets it in `main`.
        rt.set_tab_visits_controls(true);
        rt
    }
    /// Tab once and return the focused element.
    fn tab(rt: &mut Runtime<State>) -> rune::ElementInfo {
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(0.0);
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .expect("Tab focuses something")
    }
    /// Tab until `key` has focus.
    fn tab_to(rt: &mut Runtime<State>, key: &str) {
        for _ in 0..200 {
            if tab(rt).key.as_deref() == Some(key) {
                return;
            }
        }
        panic!("Tab never reaches {key}");
    }
    fn tap(rt: &mut Runtime<State>, key: rune::Key) {
        rt.key(rune::KeyEvent {
            key,
            mods: Default::default(),
        });
        rt.frame(0.0);
        rt.frame(1.0);
    }
    fn focused(rt: &Runtime<State>) -> rune::ElementInfo {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .expect("something has focus")
    }
    /// Walks an open menu or list from the keyboard: Down onto its first row and on to its last,
    /// then Home, End and Up, and Escape closes it.
    fn walk_menu(rt: &mut Runtime<State>, menu: &str, wraps: bool, at: &str) {
        let (frame, _) = rt
            .presented(menu)
            .unwrap_or_else(|| panic!("{at}: {menu} is not open"));
        // Down leaves a search field for the list; Home then starts from the top whether or
        // not the menu was opened from the keyboard and already shows its first row.
        tap(rt, rune::Key::Down);
        tap(rt, rune::Key::Home);
        let first = focused(rt);
        let centre = [
            first.rect[0] + first.rect[2] / 2.0,
            first.rect[1] + first.rect[3] / 2.0,
        ];
        assert!(
            centre[0] > frame[0]
                && centre[0] < frame[0] + frame[2]
                && centre[1] > frame[1]
                && centre[1] < frame[1] + frame[3],
            "{at}: Down focuses {:?} outside {menu}",
            first.key
        );
        let mut rows = vec![first];
        for _ in 0..60 {
            tap(rt, rune::Key::Down);
            let next = focused(rt);
            if next.path == rows[0].path || next.path == rows.last().unwrap().path {
                assert_eq!(
                    next.path == rows[0].path,
                    wraps,
                    "{at}: Down past the last row of {menu}"
                );
                break;
            }
            rows.push(next);
        }
        assert!(rows.len() >= 2, "{at}: {menu} has one row");
        for pair in rows.windows(2) {
            assert!(
                pair[1].rect[1] > pair[0].rect[1] - 0.5 || pair[1].scrolled != pair[0].scrolled,
                "{at}: Down goes up from {:?} to {:?}",
                pair[0].key,
                pair[1].key
            );
        }
        let (top, bottom) = (rows[0].path.clone(), rows.last().unwrap().path.clone());
        tap(rt, rune::Key::Home);
        assert_eq!(focused(rt).path, top, "{at}: Home in {menu}");
        tap(rt, rune::Key::End);
        assert_eq!(focused(rt).path, bottom, "{at}: End in {menu}");
        tap(rt, rune::Key::Home);
        tap(rt, rune::Key::Up);
        assert_eq!(
            focused(rt).path,
            if wraps { bottom } else { top },
            "{at}: Up from the top of {menu}"
        );
        tap(rt, rune::Key::Escape);
        assert!(
            rt.presented(menu).is_none(),
            "{at}: Escape leaves {menu} open"
        );
    }
    #[test]
    fn arrow_keys_home_end_return_and_escape_drive_every_menu_and_list() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let fresh = || {
                let mut rt = runtime(width);
                rt.state.populate_demo();
                rt.state.demo = false;
                rt.frame(0.0);
                rt.frame(1.0);
                rt
            };
            let at = |name: &str| format!("{width} {name}");
            // Opened from the keyboard.
            let mut rt = fresh();
            tab_to(&mut rt, "thread-actions");
            tap(&mut rt, rune::Key::Text(" ".into()));
            walk_menu(&mut rt, "thread-menu", true, &at("thread menu"));
            assert!(!rt.state.thread_menu);
            // Return presses the row the keys are on.
            tab_to(&mut rt, "thread-actions");
            tap(&mut rt, rune::Key::Enter);
            tap(&mut rt, rune::Key::Down);
            let row = focused(&rt).key;
            tap(&mut rt, rune::Key::Enter);
            assert!(
                !rt.state.thread_menu,
                "{width}: Return on {row:?} did nothing"
            );

            let mut rt = fresh();
            if rt.presented("sidebar-settings").is_some() {
                tab_to(&mut rt, "sidebar-settings");
                tap(&mut rt, rune::Key::Enter);
                walk_menu(&mut rt, "settings-menu", true, &at("settings menu"));
            }

            let mut rt = fresh();
            rt.state.thread_menu = true;
            rt.invalidate();
            rt.frame(0.0);
            press(&mut rt, "menu-snooze-thread");
            rt.frame(1.0);
            walk_menu(&mut rt, "snooze-menu", true, &at("snooze menu"));

            let mut rt = fresh();
            let id = rt.state.current().id;
            rt.state.row_menu = Some(crate::model::RowTarget::Thread(id));
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            walk_menu(&mut rt, "row-context-menu", true, &at("row menu"));

            let mut rt = fresh();
            rt.state.captain_picker = true;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            walk_menu(&mut rt, "captain-picker", false, &at("captain picker"));

            let mut rt = fresh();
            rt.state.current_mut().apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
                {"provider":"cpa","id":"model-a","name":"Model A","reasoning":true},
                {"provider":"cpa","id":"model-b","name":"Model B","reasoning":true},
                {"provider":"cpa","id":"model-c","name":"Model C","reasoning":true}
            ]}}));
            rt.state.settings_picker = Some(crate::model::SettingsPicker::Model);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            assert_eq!(focused(&rt).key.as_deref(), Some("model-search"));
            walk_menu(&mut rt, "model-list", true, &at("model list"));

            let mut rt = runtime(width);
            crate::captain_workspace_demo(&mut rt.state);
            rt.state.compact_sidebar = true;
            rt.state.demo_stop(19);
            rt.state.attention_menu = true;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            walk_menu(&mut rt, "attention-menu", true, &at("attention menu"));

            // A palette with a search field keeps its highlight in state; Home and End reach
            // its ends as well.
            let mut rt = fresh();
            rt.state
                .open_picker(crate::model::ProjectPicker::AddProject);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            tap(&mut rt, rune::Key::Down);
            assert_eq!(rt.state.picker_highlight, 1, "{width}: Down in the palette");
            tap(&mut rt, rune::Key::End);
            assert!(rt.state.picker_highlight > 0, "{width}: End in the palette");
            tap(&mut rt, rune::Key::Home);
            assert_eq!(rt.state.picker_highlight, 0, "{width}: Home in the palette");
        }
    }
    /// Whether a key names a slot that draws pixel art: a portrait, an avatar or the mark.
    fn draws_art(key: &str) -> bool {
        let numbered = |prefix: &str| {
            key.strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        };
        numbered("avatar-")
            || numbered("portrait-")
            || numbered("speaker-avatar-")
            || key == "captain-hero-portrait"
            || key == "shina-logo"
            || key.ends_with("-mark")
    }
    #[test]
    fn pixel_art_fills_whole_device_pixels_at_every_zoom_and_scale() {
        for scale in [1.0, 2.0] {
            for zoom in [0.9, 1.0, 1.1, 1.25, 1.5] {
                for scene in ["captain", "empty", "example"] {
                    for width in [960.0, 1280.0] {
                        let mut rt = Runtime::with_text(
                            State::new(".".into(), false),
                            view,
                            (width, 800.0),
                            Env::new(false, scale),
                            rune::text::TextSystem::new(
                                FONTS,
                                (SANS_FAMILY, "Cascadia Mono, Consolas"),
                            ),
                        );
                        rt.state.populate_demo();
                        match scene {
                            "captain" => rt.state.open_captain(0),
                            "empty" => rt.state.new_thread_in(None),
                            _ => {}
                        }
                        rt.state.zoom = zoom;
                        rt.frame(0.0);
                        rt.frame(1.0);
                        let mut checked = 0;
                        for e in rt.elements() {
                            let Some(key) = e.key.as_deref() else {
                                continue;
                            };
                            if !draws_art(key) || e.visible[2] <= 0.0 {
                                continue;
                            }
                            for side in [e.rect[2], e.rect[3]] {
                                let device = side * scale;
                                assert!(
                                    (device - device.round()).abs() < 0.02,
                                    "{scene} at {zoom}x zoom, {scale}x scale: {key} is {device} \
                                     device pixels"
                                );
                            }
                            checked += 1;
                        }
                        assert!(checked >= 1, "{scene} {zoom} {scale}: no art slots");
                    }
                }
            }
        }
    }
    #[test]
    fn the_composer_frame_turns_coral_when_tab_focuses_its_field() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            for dark in [false, true] {
                let mut rt = runtime(width);
                rt.state.populate_demo();
                rt.state.demo = false;
                rt.state.dark = dark;
                rt.frame(0.0);
                rt.frame(1.0);
                let p = palette(dark);
                assert!(!rt.state.current().draft.focused);
                assert_eq!(composer_frame(&rt.state, p), p.frame, "{width}");
                tab_to(&mut rt, "composer-input");
                assert!(rt.state.current().draft.focused, "{width}: Tab focus");
                assert_eq!(composer_frame(&rt.state, p), p.coral, "{width}");
                tab(&mut rt);
                assert!(!rt.state.current().draft.focused, "{width}: Tab away");
                assert_eq!(composer_frame(&rt.state, p), p.frame, "{width}");
            }
        }
    }
    #[test]
    fn tab_visits_every_enabled_control_in_visual_order_and_enter_or_space_presses_it() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            for empty in [false, true] {
                let mut rt = runtime(width);
                rt.state.populate_demo();
                rt.state.demo = false;
                if empty {
                    rt.state.new_thread_in(None);
                }
                rt.frame(0.0);
                rt.frame(1.0);
                let stops: Vec<String> = rt
                    .elements()
                    .into_iter()
                    .filter(|e| e.focusable && e.visible[2] > 0.0 && e.visible[3] > 0.0)
                    .filter_map(|e| e.key)
                    .collect();
                let mut seen: Vec<rune::ElementInfo> = Vec::new();
                for _ in 0..200 {
                    let focused = tab(&mut rt);
                    if seen.first().is_some_and(|first| first.path == focused.path) {
                        break;
                    }
                    assert!(
                        focused.focus_visible,
                        "{width}: {:?} shows no ring",
                        focused.key
                    );
                    seen.push(focused);
                }
                let keys: Vec<&str> = seen.iter().filter_map(|e| e.key.as_deref()).collect();
                for stop in &stops {
                    assert!(keys.contains(&stop.as_str()), "{width}: Tab skips {stop}");
                }
                // Disabled controls and the layers that only take pointer presses stay out.
                for key in ["send-message", "composer-body", "machine-selector"] {
                    assert!(!keys.contains(&key), "{width}/{empty}: Tab stops on {key}");
                }
                for key in ["composer-input", "attach-image", "model-effort"] {
                    assert!(keys.contains(&key), "{width}/{empty}: Tab misses {key}");
                }
                // Visual order: the title strip, then the sidebar, then the pane, each top to
                // bottom and left to right along a line.
                let region = |e: &rune::ElementInfo| {
                    if e.rect[1] < 40.0 {
                        0
                    } else if e.role == "separator" {
                        1
                    } else if rt
                        .presented("sidebar")
                        .is_some_and(|(r, _)| e.rect[0] < r[0] + r[2])
                    {
                        1
                    } else {
                        2
                    }
                };
                for pair in seen.windows(2) {
                    let (a, b) = (&pair[0], &pair[1]);
                    let (ra, rb) = (region(a), region(b));
                    assert!(ra <= rb, "{width}: {:?} comes before {:?}", a.key, b.key);
                    if ra == rb && b.role != "separator" && a.role != "separator" {
                        let same_line = (a.rect[1] - b.rect[1]).abs() < 12.0;
                        assert!(
                            if same_line {
                                b.rect[0] >= a.rect[0]
                            } else {
                                b.rect[1] >= a.rect[1]
                            },
                            "{width}: {:?} at {:?} comes after {:?} at {:?}",
                            b.key,
                            b.rect,
                            a.key,
                            a.rect
                        );
                    }
                }
            }
            // Space and Return press the focused control.
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.demo = false;
            rt.frame(0.0);
            rt.frame(1.0);
            tab_to(&mut rt, "thread-actions");
            tap(&mut rt, rune::Key::Text(" ".into()));
            assert!(rt.state.thread_menu, "{width}: Space");
            tap(&mut rt, rune::Key::Escape);
            assert!(!rt.state.thread_menu, "{width}: Escape");
            tab_to(&mut rt, "thread-actions");
            tap(&mut rt, rune::Key::Enter);
            assert!(rt.state.thread_menu, "{width}: Return");
        }
    }

    #[test]
    fn an_opaque_cap_sits_under_the_chat_header_rule_over_a_scrolled_transcript() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.compact_sidebar = true;
            rt.frame(0.0);
            rt.frame(1.0);
            // The demo transcript is longer than the pane and rests stuck to its bottom, so
            // content lies under the top edge.
            let (rule, _) = rt.presented("chat-header-rule").unwrap();
            let (lane, _) = rt.presented("chat-lane").unwrap();
            assert!(
                lane[1] < rule[1],
                "width {width}: the transcript is not scrolled"
            );
            let (cap, _) = rt.presented("transcript-cap").unwrap();
            assert!(
                (cap[1] - (rule[1] + rule[3])).abs() <= 0.5
                    && (cap[3] - TRANSCRIPT_CAP).abs() < 0.01,
                "width {width}: cap {cap:?} under rule {rule:?}"
            );
            let info = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("transcript-cap"))
                .unwrap();
            assert_eq!(info.fill, palette(false).panel, "width {width}");
            assert_eq!(info.opacity, 1.0, "width {width}");
            // Nothing scrolled draws over it: the topmost element across its strip is the cap.
            for x in [cap[0] + 4.0, cap[0] + cap[2] / 2.0, cap[0] + cap[2] - 4.0] {
                let picked = rt
                    .element_at_in("chat-main", x, cap[1] + cap[3] / 2.0)
                    .unwrap();
                assert_eq!(picked.key(), Some("transcript-cap"), "width {width}, x {x}");
            }
        }
    }

    #[test]
    fn a_hairline_closes_the_chat_header_and_the_brief_toggle_is_an_icon_when_open() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.compact_sidebar = width < 760.0;
            rt.state.open_captain(0);
            rt.frame(0.0);
            let (header, _) = rt.presented("chat-header").unwrap();
            let (rule, _) = rt.presented("chat-header-rule").unwrap();
            assert_eq!(rule[3], 1.0, "{width}");
            assert!((rule[1] - (header[1] + header[3])).abs() < 0.5, "{width}");
            let (closed, _) = rt.presented("open-brief").unwrap();
            assert!(closed[2] > 40.0, "{width}: the closed button names itself");
            press(&mut rt, "open-brief");
            rt.frame(1.0);
            // Over the chat as a sheet, the header keeps an icon-only toggle. Docked, the
            // drawer's own control closes it and the header drops its toggle.
            match rt.presented("open-brief") {
                Some((open, _)) => {
                    assert_eq!([open[2], open[3]], [24.0, 24.0], "{width}: icon only")
                }
                None => assert!(rt.presented("close-brief").is_some(), "{width}"),
            }
        }
    }
    #[test]
    fn dialog_scrims_cover_the_whole_window() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.begin_new_captain();
            rt.frame(0.0);
            rt.frame(1.0);
            let (scrim, _) = rt.presented("captain-editor-backdrop").unwrap();
            assert_eq!(scrim, [0.0, 0.0, width, 720.0], "{width}");
        }
    }
    #[test]
    fn a_selected_portrait_changes_its_own_frame_and_no_cell_or_bounds() {
        let mut rt = runtime(1280.0);
        rt.state.begin_new_captain();
        rt.frame(0.0);
        rt.frame(1.0);
        let faces = |rt: &Runtime<State>| -> Vec<[f32; 4]> {
            (0..crate::visuals::PORTRAITS)
                .map(|n| {
                    rt.presented(&format!("captain-portrait-face-{n}"))
                        .unwrap()
                        .0
                })
                .collect()
        };
        let cells = |rt: &Runtime<State>| -> Vec<[f32; 4]> {
            (0..crate::visuals::PORTRAITS)
                .map(|n| rt.presented(&format!("captain-portrait-{n}")).unwrap().0)
                .collect()
        };
        let (before_faces, before_cells) = (faces(&rt), cells(&rt));
        assert!(before_faces.iter().all(|f| f[2] == 40.0 && f[3] == 40.0));
        press(&mut rt, "captain-portrait-7");
        rt.frame(1.0);
        assert_eq!(faces(&rt), before_faces);
        assert_eq!(cells(&rt), before_cells);
    }
    #[test]
    fn remote_captain_status_lines_stay_inside_their_rows_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            crate::fleet_demo(&mut rt.state);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.dark = false;
            rt.frame(0.0);
            let mut checked = 0;
            for entry in sidebar_ui::entries(&rt.state)
                .iter()
                .filter(|entry| entry.captain)
            {
                let Some((row, _)) =
                    rt.presented(&format!("machine-thread-{}-{}", entry.server, entry.id))
                else {
                    continue;
                };
                let Some((line, _)) =
                    rt.presented(&format!("machine-status-{}-{}", entry.server, entry.id))
                else {
                    continue;
                };
                assert!(
                    line[0] + line[2] <= row[0] + row[2] + 0.5,
                    "{width}: the status line {line:?} runs past its row {row:?}"
                );
                checked += 1;
            }
            assert!(checked > 0, "{width}: no remote captain rows");
        }
    }

    #[test]
    fn fleet_rows_keep_the_workspace_and_end_in_a_machine_tag_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            for sidebar_width in [crate::model::DEFAULT_SIDEBAR_WIDTH, 200.0] {
                let mut rt = runtime(width);
                crate::fleet_demo(&mut rt.state);
                rt.state.sidebar_width = sidebar_width;
                rt.state.compact_sidebar = width < 760.0;
                rt.state.dark = false;
                rt.frame(0.0);
                rt.frame(1.0);
                let mut checked = 0;
                let mut named = 0;
                for entry in sidebar_ui::entries(&rt.state) {
                    let key = |kind: &str| format!("{kind}-{}-{}", entry.server, entry.id);
                    let Some((row, _)) = rt.presented(&key("machine-thread")) else {
                        continue;
                    };
                    let workspace = rt
                        .presented(&key(if entry.captain {
                            "machine-detail"
                        } else {
                            "machine-workspace"
                        }))
                        .map(|found| found.0);
                    let Some(workspace) = workspace else {
                        continue;
                    };
                    let (tag, _) = rt.presented(&key("machine-tag")).expect("a machine tag");
                    // Ten characters where the sidebar is its default width, or the whole name
                    // when it is shorter; the narrowest sidebars give up some of that but
                    // keep about four.
                    let text = rt
                        .elements()
                        .into_iter()
                        .find(|element| {
                            element.key.as_deref()
                                == Some(&format!(
                                    "{}-{}-{}",
                                    if entry.captain {
                                        "machine-detail"
                                    } else {
                                        "machine-workspace"
                                    },
                                    entry.server,
                                    entry.id
                                ))
                        })
                        .and_then(|element| element.text)
                        .unwrap_or_default();
                    let floor = if sidebar_row_width(&rt.state) >= 240.0 {
                        WORKSPACE_FLOOR.min(text_width(&text) * 0.9)
                    } else {
                        // A name shorter than that keeps its own width, with the tag 8 px later.
                        24.0_f32.min(text_width(&text) * 0.9)
                    };
                    assert!(
                        workspace[2] >= floor - 0.5,
                        "{width}/{sidebar_width}: the workspace is only {} wide",
                        workspace[2]
                    );
                    assert!(
                        tag[0] + tag[2] <= row[0] + row[2] + 0.5 && tag[2] >= ICON - 0.5,
                        "{width}/{sidebar_width}: the machine tag {tag:?} leaves its row {row:?}"
                    );
                    assert!(
                        workspace[0] + workspace[2] <= tag[0] + 0.5,
                        "{width}/{sidebar_width}: the workspace runs into the machine tag"
                    );
                    if tag[2] > ICON + 14.0 {
                        named += 1;
                    }
                    checked += 1;
                }
                assert!(checked > 0, "{width}/{sidebar_width}: no fleet rows");
                // Where the sidebar is the default width or wider, rows say which machine.
                if sidebar_row_width(&rt.state) >= 224.0 {
                    assert!(
                        named * 2 >= checked,
                        "{width}/{sidebar_width}: only {named} of {checked} rows name their machine"
                    );
                }
            }
        }
    }

    #[test]
    fn demo_machines_never_read_as_offline_and_rows_carry_no_connection_state() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            crate::fleet_demo(&mut rt.state);
            rt.state.compact_sidebar = width < 760.0;
            rt.frame(0.0);
            rt.frame(1.0);
            for server in ["local", "example-studio", "example-laptop"] {
                assert!(rt.state.machine_connected(server), "{server}");
            }
            let tree = rt.accessibility_json().to_string();
            assert!(!tree.contains("offline"), "{width}: a label says offline");
            assert!(!tree.contains("unreachable"), "{width}");
            assert!(rt.presented("machine-unreachable").is_none());
        }
    }

    #[test]
    fn unreachable_machines_raise_one_notice_and_the_open_thread_says_so() {
        let mut rt = runtime(1280.0);
        crate::fleet_demo(&mut rt.state);
        // Real machines, not demo data: their fixture clients are not connected.
        rt.state.demo = false;
        let poll = |rt: &mut Runtime<State>| {
            rt.update(|state, tx| {
                crate::model::poll(state, 0.0, tx);
            });
            rt.invalidate();
            rt.frame(1.0);
        };
        poll(&mut rt);
        let notice = rt.state.machines.notice.clone().expect("a notice");
        assert!(
            notice.contains("Studio")
                && notice.contains("Laptop")
                && notice.ends_with("unreachable. Reconnecting."),
            "{notice}"
        );
        assert!(notice.contains(" and "), "one notice lists both: {notice}");
        assert_eq!(
            rt.state.machines.auto_notice.as_deref(),
            Some(notice.as_str())
        );
        assert!(rt.presented("machine-unreachable").is_some());
        // Rows still say nothing about the connection.
        assert!(!rt.accessibility_json().to_string().contains("offline"));
        // The open thread on an unreachable machine says so.
        rt.state.open_machine_thread("example-studio", 3);
        poll(&mut rt);
        assert_eq!(rt.state.machines.selected, "example-studio");
        let tree = rt.accessibility_json();
        let header = find(&tree, "chat-unreachable").expect("the header says so");
        assert_eq!(header["name"], "Studio is unreachable. Reconnecting.");
        // Another message is never replaced or cleared by it.
        rt.state.machines.notice = Some("Something else".into());
        poll(&mut rt);
        assert_eq!(rt.state.machines.notice.as_deref(), Some("Something else"));
        // The notice clears itself once the machines are ready: none are linked any more.
        rt.state.machines.notice = rt.state.machines.auto_notice.clone();
        rt.state.client.as_mut().unwrap().linked.clear();
        poll(&mut rt);
        assert!(rt.state.machines.notice.is_none());
        assert!(rt.presented("machine-unreachable").is_none());
    }

    #[test]
    fn fleet_navigation_keeps_one_header_and_stable_captain_positions_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            crate::fleet_demo(&mut rt.state);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.dark = false;
            rt.state.zoom = 1.0;
            rt.frame(0.0);
            let positions = |rt: &Runtime<State>| {
                sidebar_ui::entries(&rt.state)
                    .iter()
                    .filter(|entry| entry.captain)
                    .map(|entry| {
                        let key = format!("machine-thread-{}-{}", entry.server, entry.id);
                        let rect = rt.presented(&key).unwrap().0;
                        ((entry.server.to_string(), entry.id), rect)
                    })
                    .collect::<Vec<_>>()
            };
            let before = positions(&rt);
            let header = rt.presented("thread-tabs").unwrap().0;
            for server in ["example-studio", "example-laptop", "local"] {
                let id = match server {
                    "example-studio" => 3,
                    "example-laptop" => 2,
                    _ => 1,
                };
                rt.state.open_machine_thread(server, id);
                rt.update(|state, tx| {
                    crate::model::poll(state, 0.0, tx);
                });
                rt.invalidate();
                rt.frame(1.0);
                assert_eq!(rt.state.machines.selected, server);
                assert!(
                    rt.state.storage_error.is_none(),
                    "{width}: {:?}",
                    rt.state.storage_error
                );
                assert!(!rt.state.dark);
                assert_eq!(rt.state.zoom, 1.0);
                assert_eq!(rt.presented("thread-tabs").unwrap().0, header);
                assert_eq!(
                    positions(&rt),
                    before,
                    "{width}: selecting {server} moved captains"
                );
                assert!(rt.find("tab-1").is_none());
                assert!(rt.find("tab-3").is_none());
                assert!(rt.find("sidebar-machine-label").is_none());
            }
        }
    }

    #[test]
    fn fleet_workspace_filter_is_server_scoped_and_does_not_follow_the_open_machine() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            crate::fleet_demo(&mut rt.state);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.open_picker(ProjectPicker::Scope);
            rt.frame(0.0);
            assert!(rt.presented("choose-project-example-laptop-1").is_some());
            choose_picker(
                &mut rt.state,
                ProjectPicker::Scope,
                PickerChoice::MachineProject("example-laptop".into(), 1),
                false,
            );
            assert_eq!(rt.state.machines.selected, "local");
            let entries = sidebar_ui::entries(&rt.state);
            assert!(!entries.is_empty());
            assert!(entries.iter().all(|entry| entry.server == "example-laptop"));
            rt.state.open_machine_thread("example-laptop", 2);
            assert_eq!(rt.state.project_scope, ProjectScope::Project(1));
            assert_eq!(
                rt.state.machines.scope_server.as_deref(),
                Some("example-laptop")
            );
            choose_picker(
                &mut rt.state,
                ProjectPicker::Scope,
                PickerChoice::All,
                false,
            );
            rt.state.open_machine_thread("local", 1);
            // The laptop now has a saved filter. Opening it must not restore that filter.
            rt.state.open_machine_thread("example-laptop", 2);
            assert_eq!(rt.state.project_scope, ProjectScope::All);
            assert!(
                sidebar_ui::entries(&rt.state)
                    .iter()
                    .any(|entry| entry.server == "local")
            );
            rt.state
                .set_machine_scope("example-laptop".into(), Some(1), false);
            rt.state.set_machine_scope("local".into(), Some(1), true);
            assert!(rt.state.scope_includes_on("example-laptop", Some(1)));
            assert!(rt.state.scope_includes_on("local", Some(1)));
            assert!(!rt.state.scope_includes_on("example-studio", Some(1)));
            rt.invalidate();
            rt.frame(1.0);
            let key = "machine-thread-local-7";
            let before = rt
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap()
                .path;
            rt.state.open_machine_thread("local", 1);
            rt.invalidate();
            rt.frame(2.0);
            let after = rt
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap()
                .path;
            assert_eq!(before, after, "{width}: captain button identity changed");
        }
    }

    #[test]
    fn user_markdown_keeps_line_breaks_and_long_paths_inside_the_bubble() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            let source = format!(
                "step one\nstep two\nstep three\n\n/{}",
                "segment".repeat(40)
            );
            rt.state.current_mut().messages = vec![crate::model::Message {
                role: "You",
                text: source.clone(),
                images: vec![],
            }];
            rt.frame(0.0);
            assert_eq!(rt.state.current().messages[0].text, source);
            let bubble = rt
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some("message-1-0-text"))
                .unwrap();
            let text: Vec<_> = rt
                .elements()
                .into_iter()
                .filter(|element| element.path.starts_with(&bubble.path))
                .filter_map(|element| element.text.map(|text| (text, element.rect)))
                .collect();
            assert!(
                text.iter()
                    .any(|(text, _)| text == "step one\nstep two\nstep three"),
                "{text:?}"
            );
            for (_, rect) in text {
                assert!(
                    rect[0] >= bubble.rect[0] - 0.5
                        && rect[0] + rect[2] <= bubble.rect[0] + bubble.rect[2] + 0.5,
                    "{width}: text {rect:?} escapes bubble {:?}",
                    bubble.rect
                );
            }
        }
    }

    #[test]
    fn markdown_code_headers_fit_and_copy_only_the_code_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            let thread = rt.state.current_mut();
            thread.tools.clear();
            thread.thoughts.clear();
            thread.work.clear();
            thread.captain = None;
            thread.messages = vec![
                crate::model::Message {
                    role: "You",
                    text: "Explain **this change**.".into(),
                    images: vec![],
                },
                crate::model::Message {
                    role: "Pi",
                    text: "```rust\nlet ready = true;\n```".into(),
                    images: vec![],
                },
            ];
            let clipboard = rune::clipboard::memory();
            rt.use_clipboard(&clipboard);
            rt.frame(0.0);
            let key = "copy-code-1-1-0-0";
            let (rect, _) = rt.presented(key).unwrap();
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
            press(&mut rt, key);
            assert_eq!(
                clipboard.text().as_deref().map(|text| text.as_str()),
                Some("let ready = true;")
            );
        }
    }

    #[test]
    fn assistant_diff_and_user_code_have_code_only_copy_headers_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            crate::markdown_demo(&mut rt.state);
            let diff_start = rt.state.current().messages[1].text.find("```diff").unwrap();
            let clipboard = rune::clipboard::memory();
            rt.use_clipboard(&clipboard);
            rt.frame(0.0);
            // Reveal the top of the conversation to exercise the user code header.
            rt.wheel(width - 80.0, 200.0, 2000.0);
            rt.frame(1.0);
            press(&mut rt, "copy-code-user-1-0-0");
            assert_eq!(
                clipboard.text().as_deref().map(|text| text.as_str()),
                Some("let ready = true;")
            );
            let key = format!("copy-code-1-1-0-diff-{diff_start}");
            rt.wheel(width - 80.0, 400.0, -2000.0);
            rt.frame(2.0);
            let (rect, _) = rt.presented(&key).unwrap();
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
            press(&mut rt, &key);
            assert_eq!(
                clipboard.text().as_deref().map(|text| text.as_str()),
                Some("- socket.open();\n  socket.on(\"reconnect\", reconnect);\n+ socket.open();")
            );
        }
    }

    #[test]
    fn machine_selector_labels_and_link_dialog_fit_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            let mut client = crate::client::Client::new(rt.state.data_dir.clone());
            let mut remote = crate::client::Client::new(rt.state.data_dir.clone());
            remote.remote = Some(crate::peer::Link {
                server_id: "fixture-server".into(),
                host: "studio-mac".into(),
                device_id: "fixture-device".into(),
                token: "fixture".into(),
            });
            client.linked.insert("fixture-server".into(), remote);
            rt.state.client = Some(client);
            rt.state.machines.views.insert(
                "fixture-server".into(),
                json!({"index":{"threads":[
                    {"id":1,"title":"Remote thread","session":"fixture","captainHome":false,"lifecycle":{}},
                    {"id":2,"title":"Remote captain","captainHome":true,"lifecycle":{}}
                ]}}),
            );
            rt.frame(0.0);
            for key in ["machine-selector", "machine-selector-label"] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width}: {rect:?}"
                );
                assert!(
                    rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                    "{width}: {rect:?}"
                );
            }
            for key in [
                "machine-thread-fixture-server-1",
                "machine-thread-fixture-server-2",
                "machine-portrait-fixture-server-2",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width}: {rect:?}"
                );
                assert!(
                    rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                    "{width}: {rect:?}"
                );
            }
            assert!(rt.find("sidebar-machine-label").is_none());
            assert!(rt.find("machine-heading-fixture-server").is_none());
            press(&mut rt, "machine-selector");
            for key in [
                "machine-picker",
                "machine-option-local",
                "machine-option-fixture-server",
                "link-machine",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width}: {rect:?}"
                );
            }
            press(&mut rt, "link-machine");
            for key in [
                "machine-dialog",
                "machine-host",
                "machine-code",
                "machine-link-submit",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width}: {rect:?}"
                );
                assert!(
                    rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                    "{width}: {rect:?}"
                );
            }
            press(&mut rt, "close-machines");
            rt.state.machines.selected = "fixture-server".into();
            rt.state.project_picker = Some(ProjectPicker::AddProject);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(0.25);
            assert!(rt.presented("remote-project-path").is_some());
            assert!(
                rt.find("send-message")
                    .is_none_or(|_| !rt.state.machine_connected("fixture-server"))
            );
        }
    }
    #[test]
    fn background_settings_fit_the_existing_menu_at_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.settings_menu = true;
            rt.frame(0.0);
            let (menu, _) = rt.presented("settings-menu").unwrap();
            for key in [
                "default-harness",
                "background-toggle",
                "keep-awake-toggle",
                "theme-toggle",
            ] {
                let (item, _) = rt.presented(key).unwrap();
                assert!(item[0] >= menu[0] && item[0] + item[2] <= menu[0] + menu[2] + 0.5);
                assert!(item[1] >= 0.0 && item[1] + item[3] <= 720.5);
            }
        }
    }
    #[test]
    fn server_clients_have_the_existing_update_action_at_all_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.client = Some(crate::client::Client::new(rt.state.data_dir.clone()));
            rt.state.updater = crate::update::Updater::test_ready(crate::update::PreparedUpdate {
                tag: "v99.0.0".into(),
                target: crate::install::InstalledTarget::Windows {
                    exe: "shina.exe".into(),
                },
                asset: "shina.exe".into(),
                size: 1,
                sha256: "0".repeat(64),
                download: "download.exe".into(),
                prepared: "prepared.exe".into(),
            });
            rt.frame(0.0);
            let (notice, _) = rt
                .presented("update-restart")
                .expect("coordinated restart action");
            assert!(notice[0] >= 0.0 && notice[1] >= 0.0);
            assert!(notice[2] <= rt.state.sidebar_width, "{width}: {notice:?}");
            assert!(rt.find("server-update-disabled").is_none());
        }
    }
    #[test]
    fn the_brief_lists_and_removes_routines_without_starting_a_prompt() {
        let mut rt = runtime(1280.0);
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.open_captain(0);
        let home = rt.state.current().id;
        rt.state.background.routines = crate::routines::Routines::load(
            crate::store::directory().unwrap().join("routines.json"),
        );
        let id = rt
            .state
            .background
            .routines
            .create(
                0,
                home,
                "Review".into(),
                "Read status".into(),
                crate::routines::Schedule::Every { minutes: 15 },
                crate::routines::MissedPolicy::Skip,
                chrono::Local::now(),
            )
            .unwrap();
        rt.state.show_activity = true;
        rt.frame(0.0);
        assert!(rt.presented(&format!("routine-delete-{id}")).is_some());
        press(&mut rt, &format!("routine-pause-{id}"));
        assert!(rt.state.routines(0)[0].paused);
        press(&mut rt, &format!("routine-delete-{id}"));
        assert!(rt.state.routines(0).is_empty());
        assert!(rt.state.current().pi.is_none());
    }
    fn press(rt: &mut Runtime<State>, key: &str) {
        rt.frame(0.25);
        let (x, y) = rt
            .find(key)
            .unwrap_or_else(|| panic!("{key} is not on screen"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
    }

    #[test]
    fn questionnaire_replaces_composer_and_supports_keyboard_and_previous_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.on_key = Some(crate::key);
            rt.state.current_mut().draft.replace_text("Preserved draft");
            rt.state.current_mut().apply(json!({"type":"extension_ui_request","id":"form","method":"input",
                "questions":[
                    {"id":"one","header":"Choose storage","question":"Where should the working data go?",
                     "multiSelect":true,"options":[{"label":"Disk","description":"Keep it on this computer"},{"label":"Server","description":"Use a remote server"}]},
                    {"id":"two","header":"Details","question":"Anything else?","options":[]}
                ]}));
            rt.frame(0.0);
            for key in [
                "questionnaire-drawer",
                "dialog-option-0",
                "dialog-option-1",
                "dialog-reply",
                "questionnaire-next",
                "dialog-cancel",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 1.0,
                    "{key}: {rect:?}"
                );
                assert!(
                    rect[1] >= 0.0 && rect[1] + rect[3] <= 721.0,
                    "{key}: {rect:?}"
                );
            }
            assert!(rt.find("composer-input").is_none());
            assert!(rt.find("send-message").is_none());
            // Digit shortcuts work before any drawer control has focus.
            rt.key(rune::KeyEvent {
                key: rune::Key::Text("1".into()),
                mods: Default::default(),
            });
            rt.frame(0.0);
            rt.key(rune::KeyEvent {
                key: rune::Key::Text("2".into()),
                mods: Default::default(),
            });
            rt.frame(0.0);
            assert_eq!(
                rt.state
                    .current()
                    .native
                    .questionnaire
                    .as_ref()
                    .unwrap()
                    .answers[0]
                    .selected,
                vec!["Disk", "Server"]
            );
            press(&mut rt, "questionnaire-next");
            assert_eq!(
                rt.state
                    .current()
                    .native
                    .questionnaire
                    .as_ref()
                    .unwrap()
                    .index,
                1
            );
            assert!(rt.find("dialog-submit").is_some());
            rt.state.current_mut().reply.replace_text("Free text");
            press(&mut rt, "questionnaire-previous");
            assert_eq!(
                rt.state
                    .current()
                    .native
                    .questionnaire
                    .as_ref()
                    .unwrap()
                    .index,
                0
            );
            press(&mut rt, "questionnaire-next");
            assert_eq!(rt.state.current().reply.text, "Free text");
            assert_eq!(rt.state.current().draft.text, "Preserved draft");
            rt.state.current_mut().reply.focused = true;
            assert!(!pi_ui::questionnaire_key(
                &mut rt.state,
                &rune::KeyEvent {
                    key: rune::Key::Text("1".into()),
                    mods: Default::default()
                }
            ));
        }
    }

    #[test]
    fn ordinary_file_and_folder_chips_fit_at_all_widths() {
        let path = crate::store::directory()
            .unwrap()
            .join("a long file name.txt");
        std::fs::write(&path, "Fixture file").unwrap();
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state
                .attach_paths(&[path.clone(), crate::store::directory().unwrap()]);
            rt.frame(0.0);
            let (composer, _) = rt.presented("composer").unwrap();
            for attachment in &rt.state.current().draft_images {
                let (rect, _) = rt
                    .presented(&format!("attachment-{}", attachment.id))
                    .unwrap();
                assert_eq!((rect[2], rect[3]), (64.0, 64.0));
                assert!(
                    rect[0] >= composer[0] && rect[0] + rect[2] <= composer[0] + composer[2] + 1.0
                );
                assert!(rect[0] >= 0.0 && rect[0] + rect[2] <= width);
            }
            assert_eq!(rt.state.current().draft_images.len(), 2);
        }
    }

    #[test]
    fn an_acp_agent_sharing_a_built_in_id_gets_its_own_key() {
        let mut rt = runtime(1280.0);
        rt.state.harnesses.agents = vec![crate::harness::Agent {
            id: "codex".into(),
            name: "Codex ACP".into(),
            command: "codex-acp".into(),
            args: vec![],
            env: vec![],
        }];
        rt.state.settings_picker = Some(SettingsPicker::Model);
        rt.frame(0.0);
        let keyed = |key: &str| {
            rt.elements()
                .into_iter()
                .filter(|element| element.key.as_deref() == Some(key))
                .count()
        };
        assert_eq!(keyed("choose-harness-codex"), 1);
        assert_eq!(keyed("choose-harness-codex-2"), 1);
    }

    #[test]
    fn the_empty_thread_headline_names_the_workspace_its_tab_names() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.state.new_thread_in(None);
        rt.frame(0.0);
        let (tab, _) = rt.presented("active-tab").unwrap();
        assert!(tab[2] > 0.0);
        let picker = rt
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("draft-project"))
            .expect("the headline picker");
        assert!(
            picker
                .label
                .as_deref()
                .is_some_and(|label| label.contains("No workspace")),
            "{:?}",
            picker.label
        );
    }

    #[test]
    fn a_linked_client_uses_the_standard_tab_strip() {
        let mut rt = runtime(1280.0);
        crate::fleet_demo(&mut rt.state);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(rt.state.client.is_some());
        let tabs = rt
            .elements()
            .into_iter()
            .filter(|element| element.role == "tab")
            .count();
        assert!(tabs >= 1, "the standard tab strip has role=tab entries");
        assert!(rt.presented("active-tab").is_some());
    }

    #[test]
    fn a_docked_brief_has_one_control_and_routine_schedules_are_whole() {
        let mut rt = runtime(1440.0);
        crate::captain_brief_demo(&mut rt.state);
        rt.state.show_activity = true;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(
            rt.presented("close-brief").is_some(),
            "the drawer's own control"
        );
        assert!(
            rt.presented("open-brief").is_none(),
            "the chat header does not repeat it"
        );
        let mut checked = 0;
        for element in rt.elements() {
            let Some(key) = element.key.as_deref() else {
                continue;
            };
            if key.starts_with("routine-schedule-") {
                // "Weekdays at 9:00" is 16 cells of about 6 pt at 11.5 px.
                let text = element.text.clone().unwrap_or_default();
                assert!(
                    element.rect[2] >= text.chars().count() as f32 * 5.0,
                    "{key}: {text} is cut to {}",
                    element.rect[2]
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no routine rows on screen");
        // The memory controls overlay the row instead of reserving width at rest.
        let mut memories = 0;
        for element in rt.elements() {
            let Some(key) = element.key.as_deref() else {
                continue;
            };
            let Some(id) = key.strip_prefix("memory-text-") else {
                continue;
            };
            let (row, _) = rt.presented(&format!("memory-{id}")).unwrap();
            let (actions, _) = rt.presented(&format!("memory-actions-{id}")).unwrap();
            assert!(
                element.rect[0] + element.rect[2] >= row[0] + row[2] - 8.5,
                "{key}: the text stops at {} in a row ending at {}",
                element.rect[0] + element.rect[2],
                row[0] + row[2]
            );
            assert!(actions[0] + actions[2] <= row[0] + row[2] + 0.5);
            memories += 1;
        }
        assert!(memories > 0, "no memory rows on screen");
        rt.state.show_activity = false;
        rt.invalidate();
        rt.frame(1.0);
        assert!(
            rt.presented("open-brief").is_some(),
            "closed, the header opens it"
        );
    }

    #[test]
    fn an_acp_agent_that_repeats_a_built_in_entry_is_listed_once() {
        let mut rt = runtime(1280.0);
        rt.state.harnesses.agents = vec![
            crate::harness::Agent {
                id: "codex".into(),
                name: "Codex".into(),
                command: "codex-acp".into(),
                args: vec![],
                env: vec![],
            },
            crate::harness::Agent {
                id: "codex".into(),
                name: "Codex ACP".into(),
                command: "codex-acp".into(),
                args: vec![],
                env: vec![],
            },
        ];
        rt.state.settings_picker = Some(SettingsPicker::Model);
        rt.frame(0.0);
        let listed = |key: &str| rt.find(key).is_some();
        assert!(listed("choose-harness-codex"));
        assert!(
            listed("choose-harness-codex-2"),
            "a differently named agent stays"
        );
        assert!(
            !listed("choose-harness-codex-3"),
            "the same name and id is not repeated"
        );
        assert!(
            listed("harness-tag-codex"),
            "the second entry carries the ACP tag"
        );
    }

    #[test]
    fn harness_picker_is_available_before_first_send_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.harnesses.agents = vec![crate::harness::Agent {
                id: "fixture".into(),
                name: "Fixture ACP".into(),
                command: "fixture-agent".into(),
                args: vec![],
                env: vec![],
            }];
            rt.state.settings_picker = Some(SettingsPicker::Model);
            rt.frame(0.0);
            for key in [
                "harness-group",
                "choose-harness-pi",
                "choose-harness-codex",
                "choose-harness-fixture",
                "harness-tag-fixture",
                "model-list",
                "model-search",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 1.0,
                    "{key}: {rect:?}"
                );
            }
            assert!(
                rt.presented("harness-tag-pi").is_none(),
                "Pi is built in, not an ACP agent"
            );
            assert!(rt.presented("harness-tag-codex").is_none());
            press(&mut rt, "choose-harness-fixture");
            assert_eq!(rt.state.current().harness, "fixture");
            assert!(rt.state.current().selected_model.is_none());
            rt.state.current_mut().life.latest_user_message_at = Some(1);
            rt.invalidate();
            rt.frame(0.0);
            press(&mut rt, "choose-harness-pi");
            assert_eq!(
                rt.state.current().harness,
                "fixture",
                "harness is fixed after first send"
            );
        }
    }
    fn find<'a>(tree: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
        if tree["key"] == key {
            return Some(tree);
        }
        tree["children"]
            .as_array()?
            .iter()
            .find_map(|c| find(c, key))
    }

    #[test]
    fn native_subagent_cards_and_queued_messages_fit_at_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            let (pi, _, _) = crate::backend::Pi::test_connection();
            let t = rt.state.current_mut();
            t.pi = Some(pi);
            t.model_provider = "cpa".into();
            t.status = Status::Running;
            t.apply(json!({"type":"tool_execution_start","toolCallId":"native","toolName":"subagent","args":{}}));
            t.apply(json!({"type":"tool_execution_update","toolCallId":"native","partialResult":{"content":[],"details":{"results":[{"agent":"Research","task":"Read the source","model":"Fixture"}]}}}));
            t.tools[0].expanded = true;
            t.native.follow_up = true;
            t.draft.replace_text(
                "A queued message with a long enough sentence to wrap on narrow windows.",
            );
            t.send_during_run();
            rt.frame(0.0);
            for key in ["queued-messages", "pi-subagents-native", "composer"] {
                let (r, _) = rt.presented(key).unwrap_or_else(|| panic!("{width} {key}"));
                assert!(
                    r[0] >= 0.0 && r[0] + r[2] <= width + 1.0,
                    "{width} {key} {r:?}"
                );
            }
        }
    }

    #[test]
    fn pi_controls_fit_and_native_compaction_is_an_action_at_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            let (pi, commands, _) = crate::backend::Pi::test_connection();
            let t = rt.state.current_mut();
            t.pi = Some(pi);
            t.session = Some("fixture.jsonl".into());
            t.model_provider = "cpa".into();
            t.context_window = Some(200000);
            t.context.details_open = true;
            t.context.absorb(
                &json!({"contextUsage":{"tokens":100000,"contextWindow":200000,"percent":50}}),
            );
            rt.frame(0.0);
            for key in ["composer", "compact-context", "access-mode"] {
                let (r, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    r[0] >= 0.0 && r[0] + r[2] <= width + 1.0,
                    "{width}: {key} {r:?}"
                );
            }
            press(&mut rt, "compact-context");
            assert!(commands.try_iter().any(|r| r["type"] == "compact"));
            assert_eq!(rt.state.current().phase, "Compacting context");
            rt.state.current_mut().stop();
            assert!(!rt.state.current().status.busy());
        }
    }

    #[test]
    fn codex_compact_button_and_progress_rows_fit_at_supported_widths() {
        // Uses the existing KSG ContextWindowMeter and context-compaction row layout.
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            let (pi, commands, _) = crate::backend::Pi::test_connection();
            let t = rt.state.current_mut();
            t.pi = Some(pi);
            t.harness = "codex".into();
            t.session = Some("codex-fixture-thread".into());
            t.model_provider = "codex".into();
            t.context_window = Some(200000);
            t.context
                .absorb(&json!({"contextUsage":{"tokens":100000,"contextWindow":200000}}));
            t.context.details_open = true;
            rt.frame(0.0);
            press(&mut rt, "compact-context");
            assert!(commands.try_iter().any(|r| r["type"] == "compact"));
            rt.state
                .current_mut()
                .apply(json!({"type":"compaction_start"}));
            rt.state.current_mut().context.details_open = true;
            rt.invalidate();
            rt.frame(0.0);
            for key in ["compact-context", "compaction-0"] {
                let (r, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    r[0] >= 0.0 && r[0] + r[2] <= width + 1.0,
                    "{width}: {key} {r:?}"
                );
            }
            rt.state
                .current_mut()
                .apply(json!({"type":"compaction_end"}));
            rt.state.current_mut().native.compaction_expanded = true;
            rt.frame(0.0);
            assert!(rt.presented("compaction-0").is_some());
        }
    }

    #[test]
    fn slash_menu_supports_keyboard_selection_and_provider_commands_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            rt.state.current_mut().apply(json!({"type":"response","command":"get_commands","success":true,"data":{"commands":[{"name":"skill:review","description":"Review the patch","source":"skill"}]}}));
            rt.state.current_mut().draft.replace_text("/skill:");
            rt.invalidate();
            rt.frame(0.0);
            let (r, _) = rt.presented("slash-menu").unwrap();
            assert!(r[0] >= 0.0 && r[0] + r[2] <= width + 1.0);
            rt.key(rune::KeyEvent {
                key: rune::Key::Tab,
                mods: Default::default(),
            });
            rt.frame(0.0);
            assert_eq!(rt.state.current().draft.text, "/skill:review ");
            assert!(rt.find("slash-menu").is_none());
        }
    }

    #[test]
    fn steering_and_rewind_controls_send_native_commands_without_overflow() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            let (pi, commands, _) = crate::backend::Pi::test_connection();
            let t = rt.state.current_mut();
            t.pi = Some(pi);
            t.model_provider = "cpa".into();
            t.status = Status::Running;
            t.draft.replace_text("Steer now");
            rt.frame(0.0);
            for key in ["dispatch-mode", "send-message", "stop-agent"] {
                let (r, _) = rt.presented(key).unwrap();
                assert!(r[0] + r[2] <= width + 1.0, "{key}");
            }
            press(&mut rt, "send-message");
            let send = commands.try_iter().find(|r| r["type"] == "prompt").unwrap();
            assert_eq!(send["streamingBehavior"], "steer");
            assert_eq!(rt.state.current().status, Status::Running);
            let t = rt.state.current_mut();
            t.status = Status::Ready;
            t.session = Some("source.jsonl".into());
            t.native.conversation = Some(crate::model::ConversationMode::Rewind);
            t.native.fork_points = vec![json!({"entryId":"point","text":"Repeated prompt"})];
            rt.invalidate();
            rt.frame(0.0);
            press(&mut rt, "fork-point-0");
            assert!(rt.find("rewind-confirm").is_some());
            press(&mut rt, "rewind-confirm");
            let rewind = commands.try_iter().find(|r| r["type"] == "fork").unwrap();
            assert_eq!(rewind["entryId"], "point");
        }
    }

    #[test]
    fn reading_and_control_text_keep_accessible_contrast_in_both_themes() {
        for dark in [false, true] {
            let p = palette(dark);
            for (index, (foreground, background)) in [
                (p.ink, p.panel),
                (p.ink, p.bg),
                (p.muted, p.panel),
                (p.muted, p.bg),
                (p.muted, p.card),
                (p.faint, p.panel),
                (p.faint, p.card),
                (p.faint, p.bg),
                (p.faint, p.bg.mix(p.selected, p.selected.a)),
                (p.faint, p.card.mix(p.selected, p.selected.a)),
                (p.faint, p.code_bg),
                (p.ink, p.message_bg),
                // The "Shina" role label sits on the coral wash.
                (p.ink, p.panel.mix(p.coral_bg, p.coral_bg.a)),
                (p.muted, p.code_bg),
                (p.muted, p.bg.mix(p.selected, p.selected.a)),
                (p.muted, p.card.mix(p.selected, p.selected.a)),
                (p.sea, p.panel),
                (p.sea, p.card),
                (p.sea, p.bg),
                (p.sea, p.sea_bg),
                (p.on_coral, p.coral),
                (p.on_brass, p.brass),
                (p.brass, p.panel),
                (p.on_brass, crew_ui::brass_darkened(p, 0.2)),
                (p.on_brass, crew_ui::brass_darkened(p, 0.32)),
            ]
            .into_iter()
            .enumerate()
            {
                let a = foreground.luminance();
                let b = background.luminance();
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(
                    contrast >= 4.5,
                    "text contrast {contrast:.2} in pair {index}, dark={dark}"
                );
            }
        }
        // Night's two quiet inks are different steps, not one colour under two names.
        let p = palette(true);
        assert!(
            (p.muted.luminance() + 0.05) / (p.faint.luminance() + 0.05) >= 1.25,
            "dark muted and faint text read as the same colour"
        );
    }

    #[test]
    fn composer_controls_fit_with_long_model_names_at_minimum_height() {
        for (width, sidebar) in [(620.0, false), (620.0, true), (800.0, true), (1920.0, true)] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 480.0), 1.0);
            rt.state.compact_sidebar = sidebar;
            let t = rt.state.current_mut();
            t.model = "A deliberately long model name for layout validation".into();
            t.context.usage = ContextUsage {
                tokens: Some(60_000),
                context_window: Some(200_000),
                percent: Some(30.0),
            };
            t.draft.replace_text("A short draft");
            rt.frame(0.0);
            rt.frame(1.0);
            let (card, _) = rt.presented("composer").unwrap();
            assert!(card[1] >= 0.0 && card[1] + card[3] <= 480.5);
            let mut previous_right = card[0];
            for key in [
                "attach-image",
                "model-effort",
                "context-usage-ring",
                "send-message",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{key} is missing"));
                assert!(
                    rect[0] >= previous_right - 0.5
                        && rect[0] + rect[2] <= card[0] + card[2] + 0.5
                        && rect[1] >= card[1]
                        && rect[1] + rect[3] <= card[1] + card[3] + 0.5,
                    "{key} overlaps or overflows at {width}, sidebar={sidebar}: {rect:?} in {card:?}"
                );
                previous_right = rect[0] + rect[2];
            }
            let (footer, _) = rt.presented("composer-footer").unwrap();
            assert!(footer[1] + footer[3] <= 480.5);
        }
    }

    #[test]
    fn sidebar_and_composer_remain_usable_with_system_reduce_motion() {
        let mut rt = runtime(620.0);
        rt.set_system_reduced_motion(true);
        rt.frame(0.0);
        press(&mut rt, "sidebar-toggle");
        rt.frame(1.0);
        assert!(rt.reduced_motion);
        assert!(rt.presented("sidebar").is_some());
        press(&mut rt, "sidebar-toggle");
        rt.frame(1.0);
        assert!(rt.presented("sidebar").is_none());
        press(&mut rt, "composer-input");
        assert!(rt.state.current().draft.focused);
    }

    #[test]
    fn the_update_row_offers_one_restart_and_a_passive_download() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        // A development build shows nothing, and never a download button.
        assert!(find(&rt.accessibility_json(), "update-restart").is_none());
        assert!(find(&rt.accessibility_json(), "update-download").is_none());
        rt.state.updater = crate::update::Updater::test_downloading("v9.0.0");
        rt.invalidate();
        rt.frame(0.0);
        let tree = rt.accessibility_json();
        let progress = match find(&tree, "update-progress") {
            Some(node) => node,
            None => panic!(
                "a downloading row; status={:?} tree={}",
                rt.state.updater.status(),
                serde_json::to_string(&tree).unwrap()
            ),
        };
        assert_eq!(progress["role"], "status");
        assert_eq!(progress["name"], "Downloading update v9.0.0");
        assert!(find(&tree, "update-restart").is_none());
        assert!(find(&tree, "update-download").is_none());
        rt.state.updater = crate::update::Updater::test_ready(crate::update::PreparedUpdate {
            tag: "v9.1.0".into(),
            target: crate::install::InstalledTarget::Windows {
                exe: "shina.exe".into(),
            },
            asset: "shina.exe".into(),
            size: 1,
            sha256: "0".repeat(64),
            download: "download.exe".into(),
            prepared: "prepared.exe".into(),
        });
        rt.invalidate();
        rt.frame(0.0);
        let tree = rt.accessibility_json();
        let restart = find(&tree, "update-restart").expect("one restart action");
        assert_eq!(restart["name"], "Restart to update to v9.1.0");
        assert!(
            restart["actions"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x == "click"))
        );
        press(&mut rt, "update-restart");
        assert!(
            rt.state.restart_for_update,
            "the click only requests a restart"
        );
        // A running thread disables the button and says why.
        rt.state.restart_for_update = false;
        rt.state.threads[0].status = Status::Running;
        rt.invalidate();
        rt.frame(0.0);
        let tree = rt.accessibility_json();
        let restart = find(&tree, "update-restart").expect("the row stays");
        assert!(
            restart["states"]
                .as_array()
                .is_some_and(|s| s.iter().any(|x| x == "disabled")),
            "a busy app cannot restart: {restart}"
        );
        // A disabled control takes no press: the click only requests a restart when ready.
        rt.pointer_down(120.0, 640.0);
        rt.pointer_up(120.0, 640.0);
        rt.frame(0.0);
        assert!(
            !rt.state.restart_for_update,
            "a disabled restart cannot be clicked"
        );
        rt.state.threads[0].status = Status::Ready;
        rt.state.updater = crate::update::Updater::test_idle();
        rt.invalidate();
        rt.frame(0.0);
        assert!(find(&rt.accessibility_json(), "update-restart").is_none());
    }

    #[test]
    fn an_update_authentication_failure_has_a_visible_manual_remedy_and_retry() {
        let mut rt = runtime(1280.0);
        rt.state.updater = crate::update::Updater::test_suspended();
        rt.frame(0.0);
        assert!(rt.find("retry-update").is_some());
        assert!(rt.state.updater.notice().unwrap().contains("gh auth login"));
        press(&mut rt, "retry-update");
        assert!(rt.state.updater.suspended().is_none());
    }

    #[test]
    fn image_heavy_transcripts_render_at_most_thirty_two_previews() {
        let mut rt = Runtime::with_text(
            State::new(".".into(), false),
            view,
            (1280.0, 8000.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
        );
        for message in 0..6 {
            rt.state.current_mut().messages.push(crate::model::Message {
                role: "You",
                text: format!("Screenshot batch {message}"),
                images: (0..8)
                    .map(|image| Attachment {
                        id: format!("missing-{message}-{image}.png"),
                        name: format!("image {message}-{image}"),
                        mime: "image/png".into(),
                        width: 512,
                        height: 512,
                        bytes: 1,
                        path: None,
                    })
                    .collect(),
            });
        }
        rt.frame(0.0);
        let tree = rt.accessibility_json().to_string();
        assert_eq!(tree.matches("Attached image:").count(), 32);
        assert!(tree.contains("older image previews omitted"));
    }

    #[test]
    fn a_shelved_row_keeps_its_age_whole_at_the_right_edge_at_every_width() {
        for (width, sidebar) in [
            (620.0, 200.0),
            (960.0, 216.0),
            (1280.0, 232.0),
            (1920.0, 300.0),
        ] {
            let mut rt = Runtime::with_text(
                State::new(".".into(), false),
                view,
                (width, 1400.0),
                Env::new(false, 1.0),
                rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
            );
            rt.state.populate_demo();
            rt.state.shelves = [true; 2];
            rt.state.compact_sidebar = width < 760.0;
            rt.state.sidebar_width = sidebar;
            rt.frame(0.0);
            rt.frame(1.0);
            let mut checked = 0;
            for id in [4, 5] {
                let Some((row, _)) = rt.presented(&format!("thread-{id}")) else {
                    continue;
                };
                let (age, _) = rt
                    .presented(&format!("thread-age-{id}"))
                    .unwrap_or_else(|| panic!("{width}: thread {id} has no age"));
                // A whole word: "3h" and "1d" are two 6.6 pt cells.
                assert!(
                    age[2] >= 12.0,
                    "{width}/{id}: the age is only {} wide",
                    age[2]
                );
                assert!(
                    (age[0] + age[2] - (row[0] + row[2] - ROW_INSET)).abs() <= 2.5,
                    "{width}/{id}: the age {age:?} is not at the row's right edge {row:?}"
                );
                checked += 1;
            }
            assert!(checked > 0, "{width}: no shelved row was on screen");
        }
    }

    #[test]
    fn shelves_start_collapsed_keep_the_open_row_and_page_settled_threads() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        // A running thread stays in its own place in the list; there is no Working shelf.
        assert!(rt.find("shelf-Working").is_none());
        // Shelves sit at the bottom of the sidebar, just above its footer.
        let mut tall = Runtime::with_text(
            State::new(".".into(), false),
            view,
            (1280.0, 1400.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
        );
        tall.state.populate_demo();
        tall.frame(0.0);
        tall.frame(0.5);
        let (settled, _) = tall.presented("shelf-Settled").unwrap();
        let (footer, _) = tall.presented("sidebar-footer").unwrap();
        assert!(
            footer[1] - (settled[1] + settled[3]) < 40.0,
            "Settled {settled:?} is not above the footer {footer:?}"
        );
        assert!(rt.find("thread-2").is_some());
        assert!(rt.find("shelf-Snoozed").is_some());
        assert!(rt.find("shelf-Settled").is_some());
        assert!(rt.find("thread-4").is_none());
        assert!(rt.find("thread-5").is_none());
        rt.update(|_, tx| tx.scroll_into_view("shelf-Snoozed"));
        rt.frame(0.5);
        press(&mut rt, "shelf-Snoozed");
        assert!(rt.state.shelves[0]);
        assert!(rt.find("thread-4").is_some());
        rt.state.select_thread(3);
        rt.frame(0.0);
        rt.update(|_, tx| tx.scroll_into_view("shelf-Snoozed"));
        rt.frame(0.5);
        press(&mut rt, "shelf-Snoozed");
        assert!(
            rt.find("thread-4").is_some(),
            "collapsed shelf retains the open thread"
        );
        for id in 11..51 {
            let mut t = Thread::new(id);
            t.life.settle(lifecycle::now() - id as u64);
            rt.state.threads.push(t);
        }
        rt.state.shelves[1] = true;
        rt.invalidate();
        rt.frame(0.0);
        let count = |s: &State| {
            s.visible_sidebar_threads()
                .iter()
                .filter(|i| s.threads[**i].life.settled())
                .count()
        };
        assert_eq!(count(&rt.state), 10);
        assert!(rt.find("settled-more").is_some());
        rt.update(|_, tx| tx.scroll_into_view("settled-more"));
        rt.frame(0.5);
        press(&mut rt, "settled-more");
        assert_eq!(count(&rt.state), 35);
    }

    const LONG_DRAFT: &str = "Go ahead and make this an issue on GitHub, and then I want you to deeply analyze specific features from KSG Code missing from Shina. I am looking for things like:\n1. Gaps in our agent output composition. I think the way KSG formats tool calls, thinking and agent output is far superior.\n2. In the input box, we lack a lot of functionality like the $ for skills and slash commands from the ACP integrations from providers.\n3. On that note, even ACP providers.\n4. I suspect agents don't have the deep knowledge of Shina's tooling and application like they do in KSG Code.\n5. Generally speaking there appear to be bugs.\nWhen you write the issue, list each gap with the KSG Code component it comes from, what Shina does today, and what a port would take, so we can plan the work in slices and review each one on its own.\n6. And bugs.";

    /// A conversation whose composer holds `LONG_DRAFT`, focused, with the caret at its end.
    fn long_draft(width: f32, zoom: f32) -> Runtime<State> {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        rt.state.demo = false;
        rt.state.compact_sidebar = true;
        rt.state.zoom = zoom;
        rt.frame(0.0);
        rt.state.current_mut().draft = rune::TextEdit::new(LONG_DRAFT);
        rt.invalidate();
        rt.frame(0.0);
        // Click near the top of the field's viewport: a long field's centre can lie below it.
        rt.frame(0.25);
        let (view, _) = rt.presented("composer-scroll").unwrap();
        rt.pointer_down(view[0] + 24.0, view[1] + 12.0);
        rt.pointer_up(view[0] + 24.0, view[1] + 12.0);
        rt.frame(0.0);
        // The press put the caret where it landed. Type at the end, as the user does, so the
        // field follows the caret.
        let draft = &mut rt.state.current_mut().draft;
        draft.set_caret(draft.text.len());
        rt.invalidate();
        rt.frame(0.0);
        rt.key(rune::KeyEvent {
            key: rune::Key::Text(".".into()),
            mods: Default::default(),
        });
        rt.frame(0.5);
        rt.frame(0.5);
        rt
    }

    #[test]
    fn a_long_draft_scrolls_inside_the_composer_above_its_actions() {
        for zoom in [1.0, 1.25] {
            for width in [620.0, 960.0, 1280.0, 1920.0] {
                let rt = long_draft(width, zoom);
                let at = format!("width {width}, zoom {zoom}");
                assert!(rt.state.current().draft.focused, "{at}");
                assert!(
                    rt.state.current().draft.text.ends_with("And bugs.."),
                    "{at}"
                );
                let (card, _) = rt.presented("composer").unwrap();
                // The input's visible box is its body: the padding around the field's viewport.
                let (input, _) = rt.presented("composer-body").unwrap();
                let (actions, _) = rt.presented("composer-actions").unwrap();
                // The field scrolled to its end, as typing there keeps the caret in view: its
                // text ends in its 4 pt padding at the viewport's bottom, above the actions.
                // (Rune's reported caret rect ignores zoom, so the field's own rect is used.)
                let (view, _) = rt.presented("composer-scroll").unwrap();
                let (field, _) = rt.presented("composer-input").unwrap();
                let text_bottom = field[1] + field[3] - 4.0 * zoom;
                assert!(
                    (text_bottom - (view[1] + view[3] - 4.0 * zoom)).abs() <= 1.0,
                    "{at}: the field {field:?} is not scrolled to its end in {view:?}"
                );
                assert!(
                    text_bottom <= actions[1] + 0.5,
                    "{at}: the text ends at {text_bottom}, under the actions {actions:?}"
                );
                assert!(field[3] > view[3], "{at}: the draft fits; it should not");
                // The actions (with their own 8 pt bottom padding) sit at the card's bottom, and
                // the card is the input over the actions. Rune draws the frame over the edge
                // without insetting layout, so no frame term enters.
                assert!(
                    (card[1] + card[3] - (actions[1] + actions[3])).abs() <= 1.0,
                    "{at}: a strip under the actions {actions:?} in the card {card:?}"
                );
                assert!(
                    (card[3] - (input[3] + actions[3])).abs() <= 1.0,
                    "{at}: card {card:?} is not input {input:?} plus actions {actions:?}"
                );
                assert!(
                    input[1] + input[3] <= actions[1] + 0.5,
                    "{at}: the input {input:?} runs under the actions {actions:?}"
                );
                // A press on the last visual line lands in the field and places the caret there,
                // not on the actions row.
                // Near the start of the last line, where the caret is not already.
                let (x, y) = (view[0] + 8.0 * zoom, text_bottom - 10.0 * zoom);
                let picked = rt
                    .element_at_in("composer", x, y)
                    .unwrap_or_else(|| panic!("{at}: nothing at ({x}, {y})"));
                assert!(
                    picked.path().contains("composer-input")
                        || picked.key() == Some("composer-input"),
                    "{at}: the last line at ({x}, {y}) reaches {picked:?}"
                );
                let mut rt = rt;
                let end = rt.state.current().draft.focus;
                rt.pointer_down(x, y);
                rt.pointer_up(x, y);
                rt.frame(0.0);
                let draft = &rt.state.current().draft;
                let last = draft.text.rfind('\n').unwrap();
                assert!(
                    draft.focus > last && draft.focus < end,
                    "{at}: a press on the last line put the caret at {}, not after {last} and before {end}",
                    draft.focus
                );
            }
        }
    }

    #[test]
    fn typing_in_the_composer_never_moves_a_scrolled_transcript_or_grows_past_the_cap() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.demo = false;
            rt.state.compact_sidebar = true;
            rt.frame(0.0);
            let id = rt.state.current().id;
            let viewport = format!("conversation-{id}");
            rt.update(|_, tx| tx.scroll_to(viewport.clone(), 0.0, 0.0));
            rt.frame(0.5);
            press(&mut rt, "composer-input");
            let (lane, _) = rt.presented("chat-lane").unwrap();
            let mut heights = Vec::new();
            for _ in 0..14 {
                rt.key(rune::KeyEvent {
                    key: rune::Key::Text("A line of the draft that is long enough to wrap".into()),
                    mods: Default::default(),
                });
                rt.key(rune::KeyEvent {
                    key: rune::Key::Enter,
                    mods: rune::Mods {
                        shift: true,
                        ..Default::default()
                    },
                });
                rt.frame(0.0);
                // The transcript keeps its place while the user reads back and types.
                assert_eq!(
                    rt.presented("chat-lane").unwrap().0[1],
                    lane[1],
                    "width {width}: the transcript moved while typing"
                );
                heights.push(rt.presented("composer").unwrap().0[3]);
            }
            // The card grows with the draft, then stops at KSG's cap.
            let cap = heights.last().copied().unwrap();
            assert!(heights[0] < cap, "width {width}: {heights:?}");
            assert!(
                (cap - (10.0 + COMPOSER_FIELD_MAX + 2.0 + 40.0)).abs() <= 0.5,
                "width {width}: {heights:?}"
            );
        }
    }

    fn element_text(rt: &Runtime<State>, key: &str) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .and_then(|e| e.text.or(e.label))
    }

    #[test]
    fn the_header_and_the_crew_line_count_only_work_running_now() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            crate::captain_workspace_demo(&mut rt.state);
            rt.state.compact_sidebar = true;
            rt.frame(0.0);
            rt.frame(1.0);
            // Two workers run. The review waits on an approval, and one more waits for a slot.
            let line = |key| element_text(&rt, key);
            assert!(
                rt.find("crew-tray-working").is_some(),
                "width {width}: no working count"
            );
            let words: Vec<String> = rt
                .elements()
                .into_iter()
                .filter(|e| {
                    e.key.as_deref().is_none() && e.text.is_some() && {
                        let parent = e.parent.map(|i| rt.elements()[i].key.clone());
                        matches!(
                            parent.flatten().as_deref(),
                            Some("crew-tray-working" | "crew-tray-needs-you" | "crew-tray-queued")
                        )
                    }
                })
                .filter_map(|e| e.text)
                .collect();
            assert_eq!(
                words,
                ["2 working", "1 needs you", "1 queued"],
                "width {width}"
            );
            assert_eq!(rt.state.crew_working_now(7), 2, "width {width}");
            if let Some(status) = line("captain-away-status") {
                assert!(status.ends_with("2 tasks"), "width {width}: {status}");
            }
            // The waiting worker's chip shows the word, not a running timer.
            assert_eq!(
                line("crew-chip-clock-2").as_deref(),
                Some("Needs you"),
                "width {width}"
            );
            assert_ne!(line("crew-chip-clock-16").as_deref(), Some("Needs you"));
        }
    }

    #[test]
    fn interrupted_and_queued_workers_carry_their_chip_marks_in_the_sidebar() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            crate::captain_workspace_demo(&mut rt.state);
            rt.state.compact_sidebar = true;
            rt.frame(0.0);
            let fill = |rt: &Runtime<State>, key: &str| {
                rt.elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some(key))
                    .unwrap_or_else(|| panic!("width {width}: no {key}"))
            };
            let p = palette(false);
            // Interrupted (18): a hollow brass square. Queued (19): a muted square.
            let interrupted = fill(&rt, "worker-task-mark-18");
            assert_eq!(interrupted.fill.a, 0.0, "width {width}: hollow");
            let queued = fill(&rt, "worker-task-mark-19");
            assert_eq!(queued.fill, p.muted.alpha(0.6), "width {width}");
            assert!(rt.presented("worker-task-mark-18").is_some());
            // The finished worker (1) keeps an empty slot.
            assert!(
                rt.elements()
                    .iter()
                    .all(|e| e.key.as_deref() != Some("worker-task-mark-1")),
                "width {width}"
            );
        }
    }

    #[test]
    fn the_away_count_never_shrinks_and_the_phrase_shortens_to_background() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            for sidebar in [200.0, 232.0, 320.0] {
                let mut rt = runtime(width);
                crate::captain_workspace_demo(&mut rt.state);
                rt.state.compact_sidebar = true;
                rt.state.sidebar_width = sidebar;
                rt.state.brief.demo_status = Some(crate::model::BackgroundStatus {
                    hidden_to_tray: true,
                    working: 6,
                    ..Default::default()
                });
                rt.frame(0.0);
                rt.frame(1.0);
                let at = format!("width {width}, sidebar {sidebar}");
                let (count, _) = rt.presented("captain-away-count").unwrap();
                let natural = rune::text::measure("6 tasks", &[], PIXEL, None).0;
                assert!(
                    count[2] >= natural - 0.5,
                    "{at}: the count shrank to {count:?}"
                );
                let (line, _) = rt.presented("captain-away-status").unwrap();
                let (actions, _) = rt.presented("thread-actions").unwrap();
                assert!(
                    line[0] + line[2] <= actions[0],
                    "{at}: {line:?} under the buttons"
                );
                // The phrase is whole, or it is the short form; never a truncated long one.
                let words: Vec<String> = rt
                    .elements()
                    .into_iter()
                    .filter(|e| {
                        e.text.is_some()
                            && e.parent.is_some_and(|i| {
                                rt.elements()[i].key.as_deref() == Some("captain-away-status")
                            })
                    })
                    .filter_map(|e| e.text)
                    .collect();
                assert!(
                    words[0] == "Working in the background" || words[0] == "Background",
                    "{at}: {words:?}"
                );
                if words[0] == "Working in the background" {
                    let needed = rune::text::measure(&words[0], &[], PIXEL, None).0;
                    let phrase = rt
                        .elements()
                        .into_iter()
                        .find(|e| e.text.as_deref() == Some(words[0].as_str()))
                        .unwrap();
                    assert!(
                        phrase.rect[2] >= needed - 0.5,
                        "{at}: the phrase is truncated"
                    );
                }
            }
        }
    }

    #[test]
    fn a_machine_tag_follows_its_workspace_8_px_later_in_every_fleet_row() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            for sidebar in [200.0, 232.0, 320.0] {
                let mut rt = runtime(width);
                crate::fleet_demo(&mut rt.state);
                rt.state.compact_sidebar = true;
                rt.state.sidebar_width = sidebar;
                rt.frame(0.0);
                rt.frame(1.0);
                let elements = rt.elements();
                let rect = |key: &str| {
                    elements
                        .iter()
                        .find(|e| e.key.as_deref() == Some(key))
                        .map(|e| e.rect)
                };
                let mut pairs = 0;
                for e in &elements {
                    let Some(key) = e.key.as_deref() else {
                        continue;
                    };
                    let tag = if let Some(rest) = key.strip_prefix("captain-detail-") {
                        format!("captain-machine-{rest}")
                    } else if let Some(rest) = key.strip_prefix("thread-workspace-") {
                        format!("thread-machine-{rest}")
                    } else if let Some(rest) = key.strip_prefix("machine-detail-") {
                        format!("machine-tag-{rest}")
                    } else if let Some(rest) = key.strip_prefix("machine-workspace-") {
                        format!("machine-tag-{rest}")
                    } else {
                        continue;
                    };
                    let Some(tag) = rect(&tag) else { continue };
                    let workspace = e.rect;
                    let gap = tag[0] - (workspace[0] + workspace[2]);
                    let at = format!("width {width}, sidebar {sidebar}, {key}");
                    // Exactly 8 px, or 10 when the name truncates.
                    assert!(
                        (gap - 8.0).abs() < 0.5 || (gap - 10.0).abs() < 0.5,
                        "{at}: {gap} px between {workspace:?} and {tag:?}"
                    );
                    pairs += 1;
                }
                assert!(pairs >= 4, "width {width}, sidebar {sidebar}: {pairs} rows");
            }
        }
    }

    #[test]
    fn the_no_workspace_picker_reads_as_a_choice_to_make() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.compact_sidebar = true;
            rt.state.new_thread_in(None);
            rt.frame(0.0);
            rt.frame(1.0);
            let name = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("draft-project-name"))
                .unwrap();
            assert_eq!(name.text.as_deref(), Some("No workspace"));
            assert_eq!(name.text_color, Some(palette(false).muted), "width {width}");
            let (icon, _) = rt.presented("draft-project-icon").unwrap();
            assert!(
                icon[0] + icon[2] <= name.rect[0],
                "width {width}: the icon comes first"
            );
            // A real project keeps the ink name and no icon.
            let project = rt.state.projects.entries[0].id;
            rt.state.new_thread_in(Some(project));
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            assert!(
                rt.presented("draft-project-icon").is_none(),
                "width {width}"
            );
            let name = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("draft-project-name"))
                .unwrap();
            assert_eq!(name.text_color, Some(palette(false).ink), "width {width}");
        }
    }

    #[test]
    fn the_shelf_headings_stay_whole_inside_the_sidebar_at_every_zoom() {
        // At 1.25x the demo list is taller than the scroll area, and the Snoozed and Settled
        // headings at its end were cut to slivers above the footer. They stick to the area's
        // bottom edge instead, from the frame where the zoom takes effect.
        for width in [960.0, 1280.0, 1920.0] {
            for zoom in [0.5, 1.0, 1.25, 1.5, 2.0] {
                let mut rt = runtime(width);
                rt.state.populate_demo();
                // A window narrower than 760 layout points docks the sidebar only in compact mode.
                rt.state.compact_sidebar = true;
                rt.frame(0.0);
                rt.state.zoom = zoom;
                for step in 0..3 {
                    rt.frame(step as f64 * 0.5);
                    let (scroll, _) = rt.presented("sidebar-scroll").unwrap();
                    for key in ["shelf-Snoozed", "shelf-Settled"] {
                        let (rect, _) = rt
                            .presented(key)
                            .unwrap_or_else(|| panic!("{key} missing at zoom {zoom}"));
                        assert!(
                            rect[1] >= scroll[1] - 0.5
                                && rect[1] + rect[3] <= scroll[1] + scroll[3] + 0.5,
                            "width {width}, zoom {zoom}, frame {step}: {key} {rect:?} is not inside \
                             the scroll area {scroll:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_snooze_menu_is_a_popover_with_no_scrim_or_cancel_row() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            rt.frame(0.0);
            let id = rt.state.threads[0].id;
            rt.state.open_snooze(id);
            rt.frame(0.0);
            rt.frame(1.0);
            assert!(rt.presented("snooze-backdrop").is_none(), "{width}");
            assert!(rt.presented("snooze-cancel").is_none(), "{width}");
            let (menu, _) = rt.presented("snooze-menu").unwrap();
            assert!(
                menu[0] >= 0.0 && menu[0] + menu[2] <= width + 0.5,
                "{width}: {menu:?}"
            );
            assert!(menu[1] >= 0.0 && menu[1] + menu[3] <= 720.5, "{width}");
        }
    }

    #[test]
    fn done_is_sea_everywhere_and_the_thread_menu_names_its_values() {
        for dark in [false, true] {
            let p = palette(dark);
            assert_eq!(status_ink(Status::Completed, p), p.sea);
            assert_eq!(status_ink(Status::Running, p), p.sea);
            assert_ne!(status_ink(Status::Ready, p), p.sea);
        }
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        press(&mut rt, "thread-actions");
        let elements = rt.elements();
        let days = elements
            .iter()
            .find(|e| e.key.as_deref() == Some("menu-auto-settle-days"))
            .expect("the auto-settle row");
        assert!(
            days.label
                .as_deref()
                .unwrap_or("")
                .starts_with("Auto-settle after, "),
            "{:?}",
            days.label
        );
        assert!(rt.presented("menu-delete-thread").is_some());
    }

    #[test]
    fn an_empty_search_shows_the_ship_mark_at_a_whole_multiple_of_its_cells() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.search.replace_text("zzzz-no-such-thread");
            rt.frame(0.0);
            rt.frame(1.0);
            let (mark, _) = rt.presented("sidebar-empty-mark").unwrap();
            assert_eq!([mark[2], mark[3]], [32.0, 32.0], "{width}");
            let (note, _) = rt.presented("sidebar-empty").unwrap();
            assert!(mark[1] >= note[1] && mark[1] + mark[3] <= note[1] + note[3]);
        }
    }

    #[test]
    fn the_captain_editor_packs_its_portraits_and_matches_its_field_heights() {
        let mut rt = runtime(1280.0);
        rt.state.begin_new_captain();
        rt.frame(0.0);
        rt.frame(1.0);
        let height = |key: &str| rt.presented(key).unwrap().0[3];
        assert_eq!(height("captain-portrait-0"), 48.0);
        assert_eq!(height("captain-portrait-face-0"), 40.0);
        assert_eq!(height("captain-project-choice"), height("captain-name"));
        assert_eq!(height("captain-role"), height("captain-name"));
    }

    #[test]
    fn dialog_fields_keep_one_fill_focused_or_not_and_cancel_sits_beside_the_primary() {
        let mut rt = runtime(1280.0);
        rt.state.begin_new_captain();
        rt.frame(0.0);
        rt.frame(1.0);
        let (close, _) = rt.presented("close-captain-editor").unwrap();
        let (card, _) = rt.presented("captain-editor").unwrap();
        assert!(
            close[0] + close[2] <= card[0] + card[2],
            "close stays inside"
        );
        assert!(close[1] < card[1] + 60.0, "close is at the top");
        let (cancel, _) = rt.presented("cancel-captain").unwrap();
        let (save, _) = rt.presented("save-captain").unwrap();
        assert!(
            cancel[0] > card[0] + card[2] / 2.0,
            "Cancel is right-aligned"
        );
        assert!(cancel[0] < save[0], "Cancel comes before the primary");
        assert!(save[0] + save[2] <= card[0] + card[2]);
        for dark in [false, true] {
            let p = palette(dark);
            let mut state = State::new(".".into(), false);
            state.dark = dark;
            assert_eq!(field_fill(&state, p), if dark { p.bg } else { p.panel });
        }
    }

    #[test]
    fn snooze_menu_custom_validation_and_footer_undo_use_real_controls() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        press(&mut rt, "thread-actions");
        press(&mut rt, "menu-snooze-thread");
        assert!(rt.find("snooze-menu").is_some());
        assert!(rt.find("snooze-preset-0").is_some());
        press(&mut rt, "snooze-custom");
        rt.state.snooze_edit.replace_text("not a date");
        rt.frame(0.0);
        press(&mut rt, "snooze-custom-save");
        assert!(rt.state.lifecycle_error.is_some());
        rt.state.snooze_edit.replace_text("90m");
        rt.frame(0.0);
        press(&mut rt, "snooze-custom-save");
        assert!(rt.state.threads[0].life.snoozed(lifecycle::now()));
        assert!(rt.find("thread-undo").is_some());
        press(&mut rt, "thread-undo");
        assert!(!rt.state.threads[0].life.snoozed(lifecycle::now()));
        assert!(rt.state.undo.is_none());
    }

    #[test]
    fn footer_undo_claims_ctrl_z_in_a_focused_composer_only_until_typing() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        rt.state.pin_thread(1);
        rt.state.pin_thread(1);
        rt.invalidate();
        rt.frame(0.0);
        press(&mut rt, "composer-input");
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('z'),
            mods: rune::Mods {
                cmd: true,
                ctrl: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert!(rt.state.threads[0].pinned());
        rt.state.pin_thread(1);
        rt.invalidate();
        rt.frame(0.0);
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("x".into()),
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert!(!rt.state.undo.as_ref().unwrap().owns_text_key);
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('z'),
            mods: rune::Mods {
                cmd: true,
                ctrl: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert!(!rt.state.threads[0].pinned());
    }

    #[test]
    fn captain_home_menus_keep_lifecycle_controls_separate_from_threads() {
        let mut rt = runtime(1280.0);
        // Rosters start empty; these tests exercise the earlier named presets.
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.open_captain(0);
        let home = rt.state.current().id;
        rt.state
            .search
            .replace_text(rt.state.current().title.clone());
        assert!(rt.state.visible_sidebar_threads().is_empty());
        rt.frame(0.0);
        press(&mut rt, "thread-actions");
        for key in [
            "menu-pin-thread",
            "menu-settle-thread",
            "menu-snooze-thread",
            "menu-auto-settle",
        ] {
            assert!(rt.find(key).is_none());
        }
        assert!(rt.find(&format!("thread-{home}")).is_some());
    }

    #[test]
    fn native_pointer_drag_pins_at_the_target_position_and_resizes_the_sidebar() {
        let mut rt = runtime(1280.0);
        rt.state.current_mut().draft.replace_text("A prompt");
        rt.state.new_thread();
        rt.state.current_mut().draft.replace_text("Another prompt");
        rt.state.select_thread(0);
        rt.state.pin_thread(1);
        rt.invalidate();
        rt.frame(0.0);
        let (x, y) = rt.find("thread-2").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_move(x, y - 20.0);
        rt.frame(0.0);
        assert_eq!(rt.state.drag_thread, Some(2));
        let (rect, _) = rt.presented("thread-1").unwrap();
        rt.pointer_move(rect[0] + 40.0, rect[1] + 3.0);
        rt.frame(0.0);
        rt.pointer_up(rect[0] + 40.0, rect[1] + 3.0);
        rt.frame(0.0);
        assert!(rt.state.threads[1].pinned());
        assert!(rt.state.threads[1].life.pin_order_key < rt.state.threads[0].life.pin_order_key);
        let width = rt.state.sidebar_width;
        let (x, y) = rt.find("sidebar-resize").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_move(x + 30.0, y);
        rt.pointer_up(x + 30.0, y);
        rt.frame(0.0);
        assert!((rt.state.sidebar_width - width - 30.0).abs() < 0.1);
    }

    /// Exercise the actual renderer with the messages from an opt-in live Pi run.
    pub(crate) fn verify_tool_rows(thread: Thread) -> Thread {
        let mut rt = runtime(1280.0);
        rt.state.threads[0] = thread;
        rt.state.demo = true;
        rt.frame(0.0);
        let id = rt.state.current().id;
        for work in &mut rt.state.current_mut().work {
            assert!(
                work.expanded,
                "settled work rows must be discoverable by default"
            );
        }
        // Each batch summary must exist before its constituent rows are requested.
        let t = rt.state.current();
        for work in &t.work {
            let end = t.messages[work.start..]
                .iter()
                .position(|message| message.role != "Pi")
                .map_or(t.messages.len(), |offset| work.start + offset);
            for part in turn_parts(t, work.start, end) {
                if let TranscriptPart::Tools(indices) = part {
                    let key = if indices.len() > 1 {
                        format!("toggle-tool-group-{id}-{}", indices[0])
                    } else {
                        format!("inline-tool-{id}-{}", indices[0])
                    };
                    assert!(
                        rt.presented(&key).is_some(),
                        "missing visible activity {key}"
                    );
                }
            }
        }
        for tool in &mut rt.state.current_mut().tools {
            tool.group_expanded = true;
        }
        rt.invalidate();
        rt.frame(1.0);
        for index in 0..rt.state.current().tools.len() {
            let (rect, _) = rt.presented(&format!("inline-tool-{id}-{index}")).unwrap();
            assert!(
                rect[2] > 100.0 && rect[3] >= 28.0,
                "tool row has no usable layout"
            );
        }
        for index in 0..rt.state.current().thoughts.len() {
            assert!(
                rt.presented(&format!("inline-thought-{id}-{index}"))
                    .is_some()
            );
        }
        std::mem::replace(&mut rt.state.threads[0], Thread::new(1))
    }
    #[test]
    fn new_thread_is_a_native_interaction() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        // A blank draft is reused, so give this one text before asking for another.
        rt.state
            .current_mut()
            .draft
            .replace_text("Read this project");
        assert!(rt.state.current().pi.is_none());
        // One workspace in view: New thread starts there at once, as in T3 Code.
        press(&mut rt, "new-thread");
        assert!(rt.state.project_picker.is_none());
        assert_eq!(rt.state.threads.len(), 2);
        assert_eq!(rt.state.current().project, Some(1));
        assert!(rt.state.current().draft.text.is_empty());
    }
    #[test]
    fn narrow_workspace_keeps_the_composer_inside_the_window() {
        let mut rt = runtime(620.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        let (rect, _) = rt.presented("composer").unwrap();
        assert!(rect[0] >= 0.0 && rect[0] + rect[2] <= 620.5);
        assert!(rect[2] > 200.0);
        for key in ["open-session-panel", "thread-actions", "leave-example"] {
            let (rect, _) = rt.presented(key).unwrap();
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= 620.5,
                "{key} overflows the window"
            );
        }
        let (bubble, _) = rt.presented("message-1-0-text").unwrap();
        assert!(
            bubble[0] >= 0.0 && bubble[0] + bubble[2] <= 620.0,
            "user message overflows its pane: {bubble:?}"
        );
        // A compact window gives the conversation the full width until the sidebar is asked for.
        assert!(rt.presented("sidebar").is_none());
        press(&mut rt, "sidebar-toggle");
        // Layout changes animate; let them settle before hit-testing the new sidebar.
        rt.frame(2.0);
        assert!(
            rt.state.compact_sidebar,
            "toggle did not request the sidebar"
        );
        assert!(rt.presented("sidebar").is_some());
        press(&mut rt, "sidebar-toggle");
        rt.frame(4.0);
        assert!(!rt.state.compact_sidebar, "sidebar toggle did not close it");
        assert!(rt.presented("sidebar").is_none());
    }
    #[test]
    fn context_ring_reports_real_usage_and_opens_details_by_keyboard() {
        let mut rt = runtime(1280.0);
        let thread = rt.state.current_mut();
        thread.model = "Vision".into();
        thread.context_window = Some(200_000);
        thread.supports_images = Some(true);
        thread.context.usage = ContextUsage {
            tokens: Some(60_000),
            context_window: Some(200_000),
            percent: Some(30.0),
        };
        thread.draft.replace_text("Ready");
        rt.invalidate();
        rt.frame(0.0);

        let ring = rt
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("context-usage-ring"))
            .expect("context ring");
        assert_eq!(ring.role, "button");
        assert!(ring.focusable);
        assert_eq!(ring.expanded, Some(false));
        let label = ring.label.as_deref().unwrap();
        assert!(
            label.contains("60,000 of 200,000 tokens") && label.contains("30%"),
            "{label}"
        );
        assert!(
            ring.help.is_none(),
            "the popover replaces the native tooltip"
        );

        let (ring_rect, _) = rt.presented("context-usage-ring").unwrap();
        let (send_rect, _) = rt.presented("send-message").unwrap();
        assert!(
            ring_rect[0] + ring_rect[2] <= send_rect[0],
            "ring must sit beside, not over, send"
        );
        assert!(
            (ring_rect[1] + ring_rect[3] / 2.0 - (send_rect[1] + send_rect[3] / 2.0)).abs() < 2.0,
            "ring and send share a centre line"
        );

        let mut tabs = 0;
        while tabs < 120 {
            rt.key(rune::KeyEvent {
                key: rune::Key::Tab,
                mods: Default::default(),
            });
            rt.frame(0.0);
            tabs += 1;
            if rt.elements().into_iter().any(|element| {
                element.key.as_deref() == Some("context-usage-ring") && element.focused
            }) {
                break;
            }
        }
        assert!(
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some("context-usage-ring"))
                .unwrap()
                .focused,
            "the ring must be reachable by keyboard after {tabs} tabs"
        );
        rt.key(rune::KeyEvent {
            key: rune::Key::Text(" ".into()),
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert_eq!(
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some("context-usage-ring"))
                .unwrap()
                .expanded,
            Some(true)
        );
        assert!(rt.find("context-details").is_some());
        assert_eq!(
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some("context-details-usage"))
                .unwrap()
                .text
                .as_deref(),
            Some("60k/200k")
        );
        assert_eq!(
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some("context-details-percent"))
                .unwrap()
                .text
                .as_deref(),
            Some("30%")
        );
        assert_eq!(
            rt.elements()
                .into_iter()
                .find(|element| element.focused)
                .and_then(|element| element.key),
            Some("context-usage-ring".to_owned()),
            "opening details keeps keyboard focus on the ring"
        );
        rt.key(rune::KeyEvent {
            key: rune::Key::Text(" ".into()),
            mods: Default::default(),
        });
        rt.frame(0.5);
        assert!(
            rt.find("context-details").is_none(),
            "Space toggles the same disclosure as a press"
        );
        assert!(!rt.state.current().context.details_open);

        press(&mut rt, "context-usage-ring");
        rt.frame(0.0);
        assert!(
            rt.state.current().context.details_open,
            "pointer activation remains available too"
        );
    }

    #[test]
    fn unknown_after_compaction_stays_inside_a_narrow_composer() {
        let mut rt = runtime(620.0);
        let thread = rt.state.current_mut();
        thread.model = "Vision".into();
        thread.context_window = Some(200_000);
        thread.context.usage = ContextUsage {
            tokens: None,
            context_window: Some(200_000),
            percent: None,
        };
        thread.context.phase = ContextPhase::AwaitingUsage;
        thread.draft.replace_text("Ready");
        rt.invalidate();
        rt.frame(0.0);

        let ring = rt
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("context-usage-ring"))
            .expect("unknown context still shows its track");
        assert!(
            ring.label
                .as_deref()
                .unwrap()
                .contains("unknown after compaction")
        );
        let (ring_rect, _) = rt.presented("context-usage-ring").unwrap();
        let (send_rect, _) = rt.presented("send-message").unwrap();
        assert!(ring_rect[0] >= 0.0 && ring_rect[0] + ring_rect[2] <= 620.0);
        assert!(ring_rect[0] + ring_rect[2] <= send_rect[0]);
        assert!(
            rt.elements()
                .into_iter()
                .any(
                    |element| element.key.as_deref() == Some("context-usage-ring")
                        && element.role == "button"
                ),
            "the unknown state keeps its dashed ring"
        );

        press(&mut rt, "context-usage-ring");
        rt.frame(0.5);
        let (details, _) = rt.presented("context-details").unwrap();
        assert!(
            details[0] >= 0.0 && details[0] + details[2] <= 620.5,
            "the popover stays in the window: {details:?}"
        );
        assert!(
            details[1] + details[3] <= ring_rect[1] + 0.5,
            "the popover sits above the ring: {details:?} {ring_rect:?}"
        );
        assert_eq!(
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some("context-details-usage"))
                .unwrap()
                .text
                .as_deref(),
            Some("Limit 200k")
        );
        assert!(rt.find("context-details-percent").is_none());
        assert!(
            rt.find("context-details-bar").is_some(),
            "the bar keeps its empty track while usage is unknown"
        );
        assert_eq!(
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some("context-details-state"))
                .unwrap()
                .text
                .as_deref(),
            Some("Waiting for Pi to report usage")
        );
    }

    fn context_fixture(width: f32) -> Runtime<State> {
        let mut rt = runtime(width);
        let thread = rt.state.current_mut();
        thread.context_window = Some(200_000);
        thread.context.usage = ContextUsage {
            tokens: Some(61_400),
            context_window: Some(200_000),
            percent: Some(30.7),
        };
        thread.native.stats = json!({"tokens": {"total": 1_284_000}, "cost": 0.4183});
        thread.draft.replace_text("Ready");
        rt.invalidate();
        rt.frame(0.0);
        rt
    }

    fn focused_key(rt: &Runtime<State>) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|element| element.focused)
            .and_then(|element| element.key)
    }

    #[test]
    fn context_popover_opens_on_hover_and_stays_while_the_pointer_is_inside() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = context_fixture(width);
            let (composer, _) = rt.presented("composer").unwrap();
            let (ring, _) = rt.presented("context-usage-ring").unwrap();
            let (rx, ry) = (ring[0] + ring[2] / 2.0, ring[1] + ring[3] / 2.0);

            // A brief pass opens nothing; resting past KSG's 150 ms delay does.
            rt.pointer_move(rx, ry);
            rt.frame(0.05);
            assert!(
                rt.find("context-details").is_none(),
                "{width}: opened early"
            );
            rt.frame(0.2);
            rt.frame(0.3);
            let (popover, _) = rt
                .presented("context-details")
                .unwrap_or_else(|| panic!("{width}: hover did not open the popover"));
            assert!(
                popover[0] >= 0.0 && popover[0] + popover[2] <= width + 0.5,
                "{width}: popover leaves the window {popover:?}"
            );
            assert!(
                popover[1] + popover[3] <= ring[1] + 0.5,
                "{width}: popover must sit above the ring {popover:?} {ring:?}"
            );
            assert!(
                (popover[0] + popover[2] - (ring[0] + ring[2])).abs() < 1.0
                    || popover[0] + popover[2] >= width - 9.0,
                "{width}: popover aligns to the ring's end"
            );
            let (after, _) = rt.presented("composer").unwrap();
            assert_eq!(
                composer, after,
                "{width}: the composer moved when it opened"
            );
            for (key, text) in [
                ("context-details-percent", "31%"),
                ("context-details-usage", "61k/200k"),
            ] {
                let element = rt
                    .elements()
                    .into_iter()
                    .find(|element| element.key.as_deref() == Some(key))
                    .unwrap();
                assert_eq!(element.text.as_deref(), Some(text), "{width}: {key}");
            }

            // Crossing the gap into the popover keeps it open past the close delay.
            rt.pointer_move(rx, ring[1] - 2.0);
            rt.frame(0.02);
            rt.pointer_move(popover[0] + 20.0, popover[1] + 20.0);
            rt.frame(0.02);
            rt.frame(0.4);
            rt.frame(0.4);
            assert!(
                rt.find("context-details").is_some(),
                "{width}: moving into the popover closed it"
            );

            // Leaving both closes it after the delay.
            rt.pointer_move(4.0, 200.0);
            rt.frame(0.05);
            assert!(
                rt.state.current().context.popover_open(),
                "{width}: closed before the delay"
            );
            rt.frame(0.3);
            rt.frame(0.5);
            assert!(!rt.state.current().context.popover_open(), "{width}");
            assert!(rt.find("context-details").is_none(), "{width}: still shown");
        }
    }

    #[test]
    fn context_popover_opens_from_the_keyboard_and_escape_closes_it() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = context_fixture(width);
            let (composer, _) = rt.presented("composer").unwrap();
            for _ in 0..120 {
                if focused_key(&rt).as_deref() == Some("context-usage-ring") {
                    break;
                }
                rt.key(rune::KeyEvent {
                    key: rune::Key::Tab,
                    mods: Default::default(),
                });
                rt.frame(0.0);
            }
            assert_eq!(focused_key(&rt).as_deref(), Some("context-usage-ring"));
            rt.key(rune::KeyEvent {
                key: rune::Key::Enter,
                mods: Default::default(),
            });
            rt.frame(0.3);
            assert!(rt.find("context-details").is_some(), "{width}: Enter");
            assert_eq!(rt.presented("composer").unwrap().0, composer, "{width}");
            assert_eq!(focused_key(&rt).as_deref(), Some("context-usage-ring"));
            rt.key(rune::KeyEvent {
                key: rune::Key::Escape,
                mods: Default::default(),
            });
            rt.frame(0.5);
            assert!(rt.find("context-details").is_none(), "{width}: Escape");
            assert!(!rt.state.current().context.popover_open());
        }
    }

    #[test]
    fn context_popover_shows_ksg_figures_and_compaction_state() {
        assert_eq!(context_percent_text(0.0), "0%");
        assert_eq!(context_percent_text(4.26), "4.3%");
        assert_eq!(context_percent_text(9.0), "9%");
        assert_eq!(context_percent_text(30.7), "31%");
        assert_eq!(context_tokens_text(950), "950");
        assert_eq!(context_tokens_text(9_500), "9.5k");
        assert_eq!(context_tokens_text(120_400), "120k");
        assert_eq!(context_tokens_text(1_200_000), "1.2m");
        assert_eq!(context_cost_text(0.004), "USD 0.0040");
        assert_eq!(context_cost_text(0.4183), "USD 0.42");

        let mut rt = context_fixture(960.0);
        let t = rt.state.current_mut();
        t.context.begin_compaction();
        t.context.details_open = true;
        rt.invalidate();
        rt.frame(0.3);
        let text = |rt: &Runtime<State>, key: &str| {
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .and_then(|element| element.text)
        };
        assert_eq!(
            text(&rt, "context-details-state").as_deref(),
            Some("Compacting context")
        );
        assert_eq!(
            text(&rt, "context-details-usage").as_deref(),
            Some("Limit 200k")
        );
        let compact = rt
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("compact-context"))
            .unwrap();
        assert!(!compact.focusable, "no second compaction while one runs");
        press(&mut rt, "compact-context");
        assert_eq!(rt.state.current().context.phase, ContextPhase::Compacting);
    }

    #[test]
    fn chat_lane_follows_ksg_gutters_and_width_at_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.demo = false;
            rt.frame(0.0);
            rt.frame(0.0);
            let (main, _) = rt.presented("chat-main").unwrap();
            let (lane, _) = rt.presented("chat-lane").unwrap();
            let (card, _) = rt.presented("composer").unwrap();
            let gutter = lane_gutter(main[2]);
            assert!(
                lane[0] - main[0] >= gutter - 0.5
                    && main[0] + main[2] - (lane[0] + lane[2]) >= gutter - 0.5,
                "{width}: lane {lane:?} crowds its pane {main:?}"
            );
            assert!(lane[2] <= measure(main[2] >= WIDE_PANE) + 0.5, "{width}");
            assert!(
                (card[0] - lane[0]).abs() < 0.5 && (card[2] - lane[2]).abs() < 0.5,
                "{width}: the composer {card:?} and transcript {lane:?} share one lane"
            );
            let bubble =
                rt.elements()
                    .into_iter()
                    .filter(|element| {
                        element.key.as_deref().is_some_and(|key| {
                            key.starts_with("message-") && key.ends_with("-text")
                        })
                    })
                    .filter_map(|element| rt.presented(element.key.as_deref()?))
                    .map(|(rect, _)| rect)
                    .next();
            if let Some(bubble) = bubble {
                assert!(
                    bubble[2] <= lane[2] * 0.8 + 0.5,
                    "{width}: bubble {bubble:?}"
                );
                assert!(bubble[0] + bubble[2] <= lane[0] + lane[2] + 0.5, "{width}");
            }
        }
    }

    #[test]
    fn header_status_keeps_its_slot_and_siblings_when_it_starts_working() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.frame(0.0);
            rt.frame(0.0);
            let (header, _) = rt.presented("chat-header").unwrap();
            let (slot, _) = rt.presented("chat-status").unwrap();
            let (actions, _) = rt.presented("thread-actions").unwrap();
            let t = rt.state.current_mut();
            t.status = Status::Running;
            t.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(725));
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            let label = rt
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some("chat-status-text"))
                .unwrap();
            assert_eq!(label.text.as_deref(), Some("Working for 12m 05s"));
            let (text, _) = rt.presented("chat-status-text").unwrap();
            let (after_slot, _) = rt.presented("chat-status").unwrap();
            assert_eq!(
                slot, after_slot,
                "{width}: the status slot moved or resized"
            );
            assert!(
                text[0] >= after_slot[0] - 0.5
                    && text[0] + text[2] <= after_slot[0] + after_slot[2] + 0.5
                    && text[0] >= header[0]
                    && text[0] + text[2] <= header[0] + header[2],
                "{width}: status text {text:?} escapes {after_slot:?} in {header:?}"
            );
            assert_eq!(
                rt.presented("thread-actions").unwrap().0[0],
                actions[0],
                "{width}: More actions moved"
            );
        }
    }

    #[test]
    fn live_timer_ticks_without_moving_the_status_line() {
        assert_eq!(timer_label(45), "45s");
        assert_eq!(timer_label(725), "12m 05s");
        assert_eq!(timer_label(3_900), "1h 05m");
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            let t = rt.state.current_mut();
            t.messages.push(crate::model::Message {
                role: "You",
                text: "Go".into(),
                images: vec![],
            });
            t.status = Status::Running;
            t.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(721));
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            let id = rt.state.current().id;
            let (line, _) = rt.presented(&format!("live-status-{id}")).unwrap();
            let (timer, _) = rt.presented(&format!("live-timer-{id}")).unwrap();
            let (lane, _) = rt.presented("chat-lane").unwrap();
            assert!(line[0] >= 0.0 && timer[0] + timer[2] <= lane[0] + lane[2] + 0.5);
            rt.state.current_mut().started =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(728));
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            let (later, _) = rt.presented(&format!("live-timer-{id}")).unwrap();
            assert_eq!(later[0], timer[0], "{width}: the timer moved");
            assert!(
                (later[2] - timer[2]).abs() < 0.5,
                "{width}: tabular figures keep the width, {timer:?} then {later:?}"
            );
        }
    }

    #[test]
    fn context_ring_geometry_is_a_real_circle_and_arc() {
        let circle = circle_points(13.0, 13.0, 9.4, 64);
        assert_eq!(circle.len(), 64);
        for point in &circle {
            let distance = (point[0] - 13.0).hypot(point[1] - 13.0);
            assert!((distance - 9.4).abs() < 0.001, "{point:?}");
        }
        let quarter = arc_points(13.0, 13.0, 9.4, -90.0, 90.0, 16);
        assert!((quarter[0][0] - 13.0).abs() < 0.001 && quarter[0][1] < 4.0);
        let last = quarter.last().unwrap();
        assert!(last[0] > 22.0 && (last[1] - 13.0).abs() < 0.001);
    }

    #[test]
    fn tool_groups_and_individual_outputs_expand_independently() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        assert!(rt.find("toggle-tool-group-1-0").is_some());
        assert!(rt.find("inline-tool-1-0").is_none());
        // Larger reply text can put the first tool group above the pinned-bottom viewport.
        rt.wheel(800.0, 200.0, 1000.0);
        rt.frame(1.0);
        press(&mut rt, "toggle-tool-group-1-0");
        rt.frame(2.0);
        assert!(rt.state.current().tools[0].group_expanded);
        assert!(rt.find("inline-tool-1-0").is_some());
        press(&mut rt, "inline-tool-1-0");
        rt.frame(4.0);
        assert!(rt.state.current().tools[0].expanded);
        assert!(rt.presented("tool-output-1-0").is_some());
        assert!(rt.presented("tool-output-1-1").is_none());
        assert!(
            !rt.state.show_activity,
            "tool output never opens a side panel"
        );
    }
    #[test]
    fn shell_controls_align_in_the_title_strip_and_panel_controls_stay_inside() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        let (strip, _) = rt.presented("top-strip").unwrap();
        let (toggle, _) = rt.presented("sidebar-toggle").unwrap();
        let (logo, _) = rt.presented("shina-logo").unwrap();
        assert!(
            toggle[0] < logo[0],
            "the sidebar toggle leads the title strip"
        );
        // Without native window buttons on the left, the strip starts at the window's edge,
        // on the same 20 px line as the search icon.
        assert!(
            toggle[0] <= 20.0,
            "the title strip is left-aligned: {toggle:?}"
        );
        assert!(logo[0] - (toggle[0] + toggle[2]) <= 10.0);
        let (search, _) = rt.presented("thread-search").unwrap();
        assert!(
            search[1] >= strip[1] + strip[3],
            "search sits in the sidebar"
        );
        for key in ["sidebar-toggle", "shina-logo", "active-tab"] {
            let (rect, _) = rt.presented(key).unwrap();
            assert!(
                rect[1] >= strip[1] && rect[1] + rect[3] <= strip[1] + strip[3] + 0.5,
                "{key} is not inside the title strip: {rect:?}"
            );
        }
        for key in ["thread-actions", "open-session-panel"] {
            let (rect, _) = rt.presented(key).unwrap();
            assert!(
                rect[1] >= strip[1] + strip[3],
                "{key} is still in the window title strip"
            );
        }
        assert!(rt.find("toggle-activity").is_none());
        press(&mut rt, "open-session-panel");
        rt.frame(2.0);
        assert!(rt.state.show_activity);
        let (panel, _) = rt.presented("session-panel").unwrap();
        for key in ["expand-session-panel", "close-session-panel"] {
            let (rect, _) = rt.presented(key).unwrap();
            assert!(rect[0] >= panel[0] && rect[0] + rect[2] <= panel[0] + panel[2] + 0.5);
        }
        press(&mut rt, "expand-session-panel");
        rt.frame(4.0);
        assert!(rt.state.activity_full);
        assert!(rt.presented("composer").is_none());
        press(&mut rt, "close-session-panel");
        rt.frame(6.0);
        assert!(!rt.state.show_activity && !rt.state.activity_full);
        assert!(rt.presented("composer").is_some());
        press(&mut rt, "sidebar-toggle");
        rt.frame(8.0);
        let (toggle, _) = rt.presented("sidebar-toggle").unwrap();
        let (strip, _) = rt.presented("top-strip").unwrap();
        assert!(toggle[0] < 40.0 && toggle[1] + toggle[3] <= strip[1] + strip[3]);
        assert!(
            rt.find("toggle-sidebar").is_none(),
            "there is no pane-header toggle"
        );
        assert!(rt.presented("sidebar").is_none());
        press(&mut rt, "sidebar-toggle");
        rt.frame(10.0);
        assert!(rt.presented("sidebar").is_some());
    }

    #[test]
    fn native_traffic_light_space_is_reserved_across_sidebar_modes_and_zoom() {
        for width in [620.0, 1280.0] {
            for zoom in [0.67, 1.0, 1.25, 2.0] {
                for sidebar in [false, true] {
                    let mut rt = runtime(width);
                    rt.state.populate_demo();
                    rt.state.zoom = zoom;
                    rt.state.sidebar = sidebar;
                    rt.state.compact_sidebar = sidebar;
                    let mut chrome = rune::ChromeInsets::default();
                    chrome.leading = 80.0;
                    chrome.height = 44.0;
                    rt.set_chrome_insets(chrome);
                    rt.frame(0.0);
                    rt.frame(0.25);
                    for key in ["shina-logo", "sidebar-toggle", "active-tab"] {
                        if let Some((rect, _)) = rt.presented(key) {
                            assert!(
                                rect[0] >= 87.5,
                                "{key} overlaps traffic lights at width {width}, zoom {zoom}, sidebar {sidebar}: {rect:?}"
                            );
                        }
                    }
                    assert!(rt.presented("sidebar-toggle").is_some());
                }
            }
        }
    }

    #[test]
    fn native_controls_keep_docked_tabs_aligned_and_release_space_in_fullscreen() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        let original_logo = rt.presented("shina-logo").unwrap().0;
        let original_tab = rt.presented("active-tab").unwrap().0;
        let original_pane = rt.presented("chat-pane").unwrap().0;
        let mut chrome = rune::ChromeInsets::default();
        chrome.leading = 80.0;
        chrome.height = 44.0;
        rt.set_chrome_insets(chrome);
        rt.frame(0.25);
        assert!(rt.presented("shina-logo").unwrap().0[0] >= 88.0);
        assert_eq!(rt.presented("active-tab").unwrap().0[0], original_tab[0]);
        assert_eq!(rt.presented("chat-pane").unwrap().0[0], original_pane[0]);
        rt.set_chrome_insets(rune::ChromeInsets::default());
        rt.frame(0.5);
        assert_eq!(rt.presented("shina-logo").unwrap().0[0], original_logo[0]);
    }

    #[test]
    fn narrow_session_panel_can_close_without_hiding_the_composer_permanently() {
        let mut rt = runtime(620.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        press(&mut rt, "open-session-panel");
        rt.frame(2.0);
        assert!(rt.presented("session-panel").is_some());
        press(&mut rt, "close-session-panel");
        rt.frame(4.0);
        assert!(rt.presented("composer").is_some());
    }
    #[test]
    fn summaries_group_calls_at_the_same_transcript_position_only() {
        let mut s = State::new(".".into(), false);
        s.populate_demo();
        let t = s.current();
        assert!(
            matches!(&turn_parts(t, 2, 3)[0], TranscriptPart::Tools(indices) if *indices == vec![0, 1])
        );
        assert!(
            matches!(&turn_parts(t, 3, 4)[0], TranscriptPart::Tools(indices) if *indices == vec![2, 3])
        );
        assert_eq!(
            tool_summary(&t.tools[..2].iter().collect::<Vec<_>>()),
            "Read 2 files"
        );
        assert_eq!(
            tool_summary(&t.tools[2..].iter().collect::<Vec<_>>()),
            "Edited 1 file, ran 1 command"
        );
        let explored = [&t.tools[0], &t.tools[0]];
        let mut search = t.tools[0].clone();
        search.name = "grep".into();
        assert_eq!(
            tool_summary(&[explored[0], explored[1], &search]),
            "Explored 2 files, 1 search"
        );
        assert_eq!(elapsed_label(125), "2m 5s");
        assert_eq!(tool_title(&t.tools[3]), "Ran npm test -- reconnect.test.ts");
    }

    #[test]
    fn thinking_between_tools_at_the_same_offset_separates_the_groups() {
        let mut thread = Thread::new(1);
        thread.apply(
            json!({"type":"response","id":"history","command":"get_messages","success":true,
                "data":{"messages":[{"role":"assistant","content":[
                    {"type":"toolCall","id":"a","name":"read","arguments":{}},
                    {"type":"thinking","thinking":"Then check the second file."},
                    {"type":"toolCall","id":"b","name":"read","arguments":{}},
                    {"type":"text","text":"Done"}
                ]}]}
            }),
        );
        assert_eq!(
            turn_parts(&thread, 0, 1),
            vec![
                TranscriptPart::Tools(vec![0]),
                TranscriptPart::Thought(0),
                TranscriptPart::Tools(vec![1]),
                TranscriptPart::Text {
                    message: 0,
                    from: 0,
                    to: 4
                },
            ]
        );
    }
    #[test]
    fn tool_only_activity_remains_visible_before_a_shina_notice() {
        let mut rt = runtime(1280.0);
        rt.state.current_mut().apply(json!({
            "type":"tool_execution_start","toolCallId":"read","toolName":"read",
            "args":{"path":"README.md"}
        }));
        rt.state.current_mut().messages.push(crate::model::Message {
            role: "Shina",
            text: "A delegated task finished.".into(),
            images: Vec::new(),
        });
        rt.frame(0.0);
        // The finished turn folds its tool into the work line, which still opens.
        assert!(rt.presented("inline-tool-1-0").is_none());
        press(&mut rt, "toggle-work-1-0");
        assert!(rt.presented("inline-tool-1-0").is_some());
    }

    #[test]
    fn settled_work_heads_activity_and_leaves_the_answer_visible_when_collapsed() {
        // Tall enough that expanding a thought cannot scroll the work toggle under the header.
        let mut rt = Runtime::new(
            State::new(".".into(), false),
            view,
            (1280.0, 1000.0),
            Env::new(false, 1.0),
        );
        rt.state.populate_demo();
        rt.frame(0.0);
        let y = |rt: &Runtime<State>, key: &str| rt.presented(key).unwrap().0[1];
        assert!(y(&rt, "message-1-1-body-0") < y(&rt, "toggle-work-1-1"));
        assert!(y(&rt, "toggle-work-1-1") < y(&rt, "toggle-tool-group-1-0"));
        assert!(y(&rt, "toggle-tool-group-1-0") < y(&rt, "inline-thought-1-0"));
        assert!(y(&rt, "inline-thought-1-0") < y(&rt, "message-1-3-body-0"));
        press(&mut rt, "inline-thought-1-0");
        assert!(rt.presented("thought-output-1-0").is_some());
        press(&mut rt, "toggle-work-1-1");
        rt.frame(2.0);
        assert!(!rt.state.current().work[0].expanded);
        assert!(rt.presented("work-rows-1-1").is_none());
        assert!(rt.presented("message-1-3-body-0").is_some());
        press(&mut rt, "toggle-work-1-1");
        rt.frame(4.0);
        assert!(rt.state.current().thoughts[0].expanded);
        assert!(rt.presented("thought-output-1-0").is_some());
    }

    #[test]
    fn plain_answers_have_no_trailing_work_divider() {
        let mut rt = runtime(1280.0);
        rt.state.current_mut().apply(json!({"type":"agent_start"}));
        rt.state
            .current_mut()
            .apply(json!({"type":"message_end","message":{
                "role":"assistant","content":[{"type":"text","text":"Hello."}]
            }}));
        rt.state
            .current_mut()
            .apply(json!({"type":"agent_settled"}));
        rt.frame(0.0);
        assert!(rt.presented("message-1-0-body-0").is_some());
        assert!(rt.presented("work-section-1-0").is_none());
    }

    #[test]
    fn streamed_preamble_stays_put_when_the_first_tool_arrives() {
        let mut rt = runtime(1280.0);
        rt.state.current_mut().apply(json!({"type":"agent_start"}));
        rt.state
            .current_mut()
            .apply(json!({"type":"message_update","assistantMessageEvent":{
                "type":"text_delta","contentIndex":0,"delta":"I'll check the file."
            }}));
        rt.frame(0.0);
        let text_y = rt.presented("message-1-0-body-0").unwrap().0[1];
        assert!(text_y < rt.presented("live-status-1").unwrap().0[1]);
        assert!(
            rt.presented("toggle-work-1-0").is_none(),
            "no fold while live"
        );
        rt.state
            .current_mut()
            .apply(json!({"type":"message_end","message":{
                "role":"assistant","content":[
                    {"type":"text","text":"I'll check the file."},
                    {"type":"toolCall","id":"read","name":"read","arguments":{"path":"file.txt"}}
                ]
            }}));
        rt.invalidate();
        rt.frame(1.0);
        rt.frame(2.0);
        assert_eq!(rt.presented("message-1-0-body-0").unwrap().0[1], text_y);
        let tool_y = rt.presented("inline-tool-1-0").unwrap().0[1];
        assert!(tool_y < rt.presented("live-status-1").unwrap().0[1]);
    }

    #[test]
    fn tool_rows_keep_one_indent_while_the_turn_runs_and_after_it_ends() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.state.current_mut().apply(json!({"type":"agent_start"}));
            rt.state
                .current_mut()
                .apply(json!({"type":"message_end","message":{
                    "role":"assistant","content":[
                        {"type":"text","text":"I'll check the file."},
                        {"type":"toolCall","id":"read","name":"read","arguments":{"path":"file.txt"}}
                    ]
                }}));
            rt.invalidate();
            rt.frame(1.0);
            rt.frame(2.0);
            let prose = rt.presented("message-1-0-body-0").unwrap().0;
            let live_tool = rt.presented("inline-tool-1-0").unwrap().0;
            let (status, _) = rt.presented("live-status-1").unwrap();
            assert_eq!(
                status[0], prose[0],
                "{width}: the status square is on the prose edge"
            );
            assert!(
                live_tool[0] > prose[0],
                "{width}: {live_tool:?} against {prose:?}"
            );
            rt.state
                .current_mut()
                .apply(json!({"type":"agent_settled"}));
            rt.state.current_mut().work[0].expanded = true;
            rt.invalidate();
            rt.frame(1.0);
            rt.frame(2.0);
            let done_tool = rt.presented("inline-tool-1-0").unwrap().0;
            assert_eq!(done_tool[0], live_tool[0], "{width}: the indent jumped");
        }
    }

    #[test]
    fn a_running_turn_shows_its_steps_then_folds_into_worked_for() {
        let mut rt = runtime(620.0);
        rt.state.current_mut().apply(json!({"type":"agent_start"}));
        rt.state
            .current_mut()
            .apply(json!({"type":"message_update","assistantMessageEvent":{
                "type":"thinking_start","contentIndex":0
            }}));
        rt.state
            .current_mut()
            .apply(json!({"type":"message_update","assistantMessageEvent":{
                "type":"thinking_delta","contentIndex":0,"delta":"Checking the file."
            }}));
        rt.frame(0.0);
        let (thought, _) = rt.presented("inline-thought-1-0").unwrap();
        let (status, _) = rt.presented("live-status-1").unwrap();
        assert!(thought[1] < status[1]);
        rt.state
            .current_mut()
            .apply(json!({"type":"agent_settled"}));
        rt.invalidate();
        rt.frame(1.0);
        rt.frame(2.0);
        assert!(rt.presented("live-status-1").is_none());
        assert!(rt.presented("toggle-work-1-0").is_some());
        assert!(!rt.state.current().work[0].expanded);
        assert!(rt.presented("inline-thought-1-0").is_none());
    }
    #[test]
    fn captain_assignment_has_one_control_and_no_per_project_placeholders() {
        let mut rt = runtime(1280.0);
        // Rosters start empty; these tests exercise the earlier named presets.
        rt.state.captains = crate::captains::Captain::presets();
        for id in 2..=8 {
            rt.state
                .projects
                .entries
                .push(crate::projects::Project::new(
                    id,
                    rt.state.data_dir.join(format!("project-{id}")),
                ));
        }
        rt.state.projects.next_id = 9;
        rt.frame(0.0);
        assert!(rt.find("assign-captain").is_some());
        for id in 1..=8 {
            assert!(rt.find(&format!("choose-project-captain-{id}")).is_none());
        }
        press(&mut rt, "assign-captain");
        assert_eq!(rt.state.project_picker, Some(ProjectPicker::AssignCaptain));
        assert!(rt.find("choose-no-project").is_some());
        press(&mut rt, "choose-project-3");
        press(&mut rt, "choose-captain-0");
        assert_eq!(rt.state.project(Some(3)).unwrap().captain, Some(0));
        assert_eq!(rt.state.current().project, Some(3));
        assert!(rt.find("thread-2").is_some());
        press(&mut rt, "assign-captain");
        assert!(
            rt.find("choose-project-3").is_some(),
            "several captains can share a project"
        );
        assert!(rt.find("choose-project-1").is_some());
    }

    #[test]
    fn settings_gear_holds_the_theme_and_projects_show_their_own_icon() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        assert!(
            rt.find("theme-toggle").is_none(),
            "the theme lives in settings"
        );
        let dark = rt.state.dark;
        press(&mut rt, "sidebar-settings");
        press(&mut rt, "theme-toggle");
        assert_ne!(rt.state.dark, dark);
        assert!(!rt.state.settings_menu);
        // This repository ships assets/project-icon.png.
        let project = crate::projects::Project::new(99, std::env::current_dir().unwrap());
        let icon = project
            .icon
            .as_ref()
            .expect("the project icon or its thumbnail");
        let dimensions = image::ImageReader::open(icon)
            .unwrap()
            .into_dimensions()
            .unwrap();
        assert!(dimensions.0 <= 256 && dimensions.1 <= 256);
    }

    #[test]
    fn controls_show_a_hand_and_the_workspace_picker_searches_by_keyboard() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        let (x, y) = rt.find("new-thread").unwrap();
        rt.pointer_move(x, y);
        rt.frame(0.1);
        assert_eq!(rt.cursor(), rune::Cursor::Pointer);
        let (x, y) = rt.find("thread-search").unwrap();
        rt.pointer_move(x, y);
        rt.frame(0.2);
        assert_ne!(
            rt.cursor(),
            rune::Cursor::Pointer,
            "fields keep their text cursor"
        );

        press(&mut rt, "new-thread");
        let (picker, _) = rt.presented("project-picker").unwrap();
        let middle = picker[0] + picker[2] / 2.0;
        assert!(
            (middle - 640.0).abs() < 2.0,
            "the picker is a centred modal: {picker:?}"
        );
        assert!(
            rt.find("close-project-picker").is_none(),
            "no title bar or close button"
        );
        rt.state.picker_query.replace_text("run");
        rt.invalidate();
        rt.frame(0.5);
        assert!(rt.find("choose-no-project").is_none());
        let threads = rt.state.threads.len();
        let key = |key| rune::KeyEvent {
            key,
            mods: rune::Mods::default(),
        };
        rt.key(key(rune::Key::Down));
        rt.key(key(rune::Key::Enter));
        rt.frame(1.0);
        assert!(rt.state.project_picker.is_none());
        assert_eq!(rt.state.threads.len(), threads + 1);
        let project = rt.state.current().project;
        assert_eq!(rt.state.project(project).unwrap().name, "rune");
    }

    #[test]
    fn a_new_roster_is_empty_and_adding_a_captain_opens_the_create_form() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        assert!(rt.state.captains.is_empty());
        press(&mut rt, "assign-captain");
        assert!(
            rt.state.captain_editor.is_some(),
            "no preset list to choose from"
        );
        assert!(rt.state.project_picker.is_none());
        let editor = rt.state.captain_editor.as_mut().unwrap();
        assert!(editor.role.text.is_empty(), "no default role");
        editor.name.replace_text("Riku");
        rt.invalidate();
        rt.frame(0.5);
        press(&mut rt, "save-captain");
        assert_eq!(rt.state.captains.len(), 1);
        assert_eq!(rt.state.captains[0].role, "");
        assert!(!rt.state.captains[0].prompt().contains("Role:"));
        assert_eq!(rt.state.current().captain, Some(0));
    }

    #[test]
    fn add_workspace_is_a_palette_that_browses_adds_and_creates_folders() {
        let root = std::env::temp_dir().join(format!("shina-add-{}", std::process::id()));
        std::fs::create_dir_all(root.join("alpha").join("inner")).unwrap();
        std::fs::create_dir_all(root.join("beta")).unwrap();
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        // Add workspace inside the New thread picker opens the palette, not a menu.
        rt.state.open_picker(ProjectPicker::NewThread);
        rt.invalidate();
        rt.frame(0.5);
        press(&mut rt, "picker-add-project");
        assert_eq!(rt.state.project_picker, Some(ProjectPicker::AddProject));
        assert!(rt.find("project-open-folder").is_some());
        press(&mut rt, "add-local-folder");
        assert!(rt.state.picker_browsing);
        let sep = std::path::MAIN_SEPARATOR;
        rt.state
            .picker_query
            .replace_text(format!("{}{sep}al", root.display()));
        rt.invalidate();
        rt.frame(1.0);
        assert!(rt.find("browse-0").is_some() && rt.find("browse-1").is_none());
        let key = |key, cmd| rune::KeyEvent {
            key,
            mods: rune::Mods {
                cmd,
                ctrl: cmd,
                ..Default::default()
            },
        };
        rt.key(key(rune::Key::Enter, false));
        rt.frame(1.5);
        assert!(rt.state.picker_query.text.ends_with(&format!("alpha{sep}")));
        rt.key(key(rune::Key::Enter, true));
        rt.frame(2.0);
        assert!(rt.state.project_picker.is_none());
        let added = |s: &State, name: &str| {
            s.projects
                .entries
                .iter()
                .any(|p| p.path.file_name().is_some_and(|n| n == name))
        };
        assert!(added(&rt.state, "alpha"));
        // A path that does not exist yet is created, then added.
        press(&mut rt, "project-switcher");
        press(&mut rt, "add-local-folder");
        rt.state
            .picker_query
            .replace_text(format!("{}{sep}gamma", root.display()));
        rt.invalidate();
        rt.frame(3.0);
        press(&mut rt, "browse-add");
        assert!(root.join("gamma").is_dir());
        assert!(added(&rt.state, "gamma"));
        // Several workspaces in view: New thread asks which one.
        press(&mut rt, "new-thread");
        assert_eq!(rt.state.project_picker, Some(ProjectPicker::NewThread));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_filter_precedes_and_filters_both_captains_and_threads() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        let (filter, _) = rt.presented("project-scope").unwrap();
        let (captain, _) = rt.presented("thread-7").unwrap();
        assert!(filter[1] < captain[1]);
        press(&mut rt, "project-scope");
        press(&mut rt, "choose-project-1");
        rt.frame(1.0);
        assert!(rt.find("thread-7").is_some());
        assert!(rt.find("thread-8").is_none());
        assert!(rt.find("thread-9").is_none());
        assert!(rt.find("thread-3").is_none());
        rt.state.search.replace_text("no such conversation");
        rt.invalidate();
        rt.frame(1.0);
        assert!(rt.find("thread-7").is_none());
        assert!(rt.find("assign-captain").is_some());
    }

    #[test]
    fn unknown_activity_does_not_render_an_epoch_age() {
        assert_eq!(relative_time(0), None);
        assert_eq!(relative_time(crate::projects::now()), Some("now".into()));
        let mut rt = runtime(1280.0);
        rt.state.current_mut().draft.replace_text("A prompt");
        rt.state.current_mut().updated = 0;
        rt.frame(0.0);
        assert!(rt.find("thread-age-1").is_none());
        rt.state.current_mut().updated = crate::projects::now();
        rt.invalidate();
        rt.frame(0.0);
        assert!(rt.find("thread-age-1").is_some());
    }

    #[test]
    fn captain_picker_opens_a_separate_conversation_without_starting_pi() {
        let mut rt = runtime(1280.0);
        // Rosters start empty; these tests exercise the earlier named presets.
        rt.state.captains = crate::captains::Captain::presets();
        rt.frame(0.0);
        press(&mut rt, "assign-captain");
        press(&mut rt, "choose-project-1");
        assert!(rt.state.captain_picker);
        rt.frame(1.0 / 60.0);
        press(&mut rt, "choose-captain-2");
        assert!(!rt.state.captain_picker);
        assert_eq!(rt.state.current().captain, Some(2));
        assert!(rt.state.current().captain_home);
        assert!(rt.state.current().pi.is_none());
        press(&mut rt, "new-thread");
        rt.frame(2.0);
        assert_eq!(rt.state.current().project, Some(1));
        assert_eq!(rt.state.current().captain, None);
        assert!(!rt.state.current().captain_home);
    }
    #[test]
    fn fourth_captain_opens_from_the_picker() {
        let mut rt = runtime(1280.0);
        // Rosters start empty; these tests exercise the earlier named presets.
        rt.state.captains = crate::captains::Captain::presets();
        rt.frame(0.0);
        press(&mut rt, "assign-captain");
        press(&mut rt, "choose-project-1");
        rt.frame(1.0 / 60.0);
        press(&mut rt, "choose-captain-3");
        assert_eq!(rt.state.current().captain, Some(3));
        assert!(rt.state.current().captain_home);
        assert_eq!(rt.state.current().title, "Pip");
    }
    #[test]
    fn new_captain_button_creates_a_profile_instead_of_reopening_a_preset() {
        let mut rt = runtime(1280.0);
        // Rosters start empty; these tests exercise the earlier named presets.
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.current_mut().draft.replace_text("Keep this draft");
        rt.frame(0.0);
        press(&mut rt, "assign-captain");
        press(&mut rt, "choose-project-1");
        rt.frame(1.0);
        press(&mut rt, "picker-new-captain");
        assert!(rt.state.captain_editor.is_some());
        assert!(!rt.state.captain_picker);
        let editor = rt.state.captain_editor.as_mut().unwrap();
        editor.name.replace_text("Atlas");
        editor.role.replace_text("Code review");
        rt.frame(1.0 / 60.0);
        press(&mut rt, "captain-portrait-2");
        press(&mut rt, "save-captain");
        assert!(rt.state.captain_editor.is_none());
        assert_eq!(rt.state.captains[4].portrait, 2);
        assert_eq!(rt.state.current().captain, Some(4));
        assert_eq!(rt.state.current().title, "Atlas");
        assert!(rt.state.current().pi.is_none());
        assert_eq!(rt.state.threads[0].draft.text, "Keep this draft");
        assert!(rt.find("thread-2").is_some());
    }
    #[test]
    fn captain_editor_shows_every_portrait_in_a_grid_of_six_at_all_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.begin_new_captain();
            rt.frame(0.0);
            rt.frame(1.0 / 60.0);
            let (card, _) = rt.presented("captain-editor").unwrap();
            let cells: Vec<[f32; 4]> = (0..crate::visuals::PORTRAITS)
                .map(|i| rt.presented(&format!("captain-portrait-{i}")).unwrap().0)
                .collect();
            for (i, cell) in cells.iter().enumerate() {
                let (row, col) = (i / 6, i % 6);
                assert!(
                    cell[0] >= card[0] && cell[0] + cell[2] <= card[0] + card[2] + 0.5,
                    "portrait {i} leaves the card at {width}: {cell:?}"
                );
                assert!(
                    (cell[2] - PICKER_CELL).abs() < 0.5 && (cell[3] - PICKER_CELL).abs() < 0.5,
                    "portrait {i}: {cell:?}"
                );
                assert!((cell[2] - cells[0][2]).abs() < 0.5, "equal columns");
                assert!(
                    (cell[1] - cells[row * 6][1]).abs() < 0.5,
                    "row {row} aligns"
                );
                assert!((cell[0] - cells[col][0]).abs() < 0.5, "column {col} aligns");
                if col > 0 {
                    assert!(cell[0] >= cells[i - 1][0] + cells[i - 1][2]);
                }
            }
            assert!(cells[6][1] >= cells[0][1] + cells[0][3]);
            // The outer faces line up with the name field's edges, so the grid is the fields'
            // width.
            let (name, _) = rt.presented("captain-name").unwrap();
            let face = |i: usize| {
                rt.presented(&format!("captain-portrait-face-{i}"))
                    .unwrap()
                    .0
            };
            let (first, last) = (face(0), face(5));
            assert!(
                (first[0] - name[0]).abs() < 1.0,
                "{width}: {first:?} vs {name:?}"
            );
            assert!(
                (last[0] + last[2] - name[0] - name[2]).abs() < 1.0,
                "{width}: {last:?} vs {name:?}"
            );
            press(&mut rt, "captain-portrait-11");
            assert_eq!(rt.state.captain_editor.as_ref().unwrap().portrait, 11);
        }
    }
    #[test]
    fn captain_editor_footer_stays_visible_at_minimum_window_size() {
        let mut rt = Runtime::new(
            State::new(".".into(), false),
            view,
            (620.0, 480.0),
            Env::new(false, 1.0),
        );
        rt.state.begin_new_captain();
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);
        for key in ["captain-editor", "save-captain", "cancel-captain"] {
            let (rect, _) = rt.presented(key).unwrap();
            assert!(
                rect[0] >= 0.0
                    && rect[1] >= 0.0
                    && rect[0] + rect[2] <= 620.5
                    && rect[1] + rect[3] <= 480.5,
                "{key} is outside the window: {rect:?}"
            );
        }
    }
    #[test]
    fn captain_crew_follows_its_home_and_never_enters_the_lone_thread_list() {
        let mut rt = runtime(1280.0);
        // Rosters start empty; these tests exercise the earlier named presets.
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.threads[0].title = "Parent".into();
        rt.state.threads[0].captain_home = true;
        rt.state.threads[0].captain = Some(2);
        rt.state.threads[0].life.pinned_at = Some(1);
        let mut child = crate::model::Thread::new(2);
        child.title = "Delegated child".into();
        child.parent = Some(1);
        child.project = Some(1);
        child.delegated = true;
        child.captain = Some(2);
        child.life.pinned_at = Some(1);
        rt.state.threads.push(child);
        rt.frame(0.0);
        let (parent, _) = rt.presented("thread-1").unwrap();
        let (child, _) = rt.presented("thread-2").unwrap();
        assert!(child[0] >= parent[0] + 24.0 && child[1] > parent[1]);
        assert!(child[0] + child[2] <= parent[0] + parent[2] + 0.5);
        rt.state.search.replace_text("Delegated");
        rt.frame(0.0);
        assert!(
            rt.presented("thread-1").is_some(),
            "matching workers retain their captain as context during search"
        );
        press(&mut rt, "thread-2");
        assert_eq!(rt.state.current().id, 2);
        assert!(rt.presented("thread-2").is_some());
        assert!(rt.state.sidebar_threads().is_empty());
    }

    #[test]
    fn archived_workers_remain_under_their_captain_and_can_be_restored() {
        let mut rt = runtime(1280.0);
        // Rosters start empty; these tests exercise the earlier named presets.
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.open_captain(0);
        let parent = rt.state.current().id;
        let mut child = crate::model::Thread::new(3);
        child.title = "Cross captain child".into();
        child.parent = Some(parent);
        child.project = Some(1);
        child.delegated = true;
        child.captain = Some(1);
        rt.state.threads.push(child);
        rt.frame(0.0);
        assert!(rt.presented("thread-3").is_some());
        rt.state.threads.last_mut().unwrap().life.archived_at = Some(1);
        rt.state.show_archive = true;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let (home, _) = rt.presented(&format!("thread-{parent}")).unwrap();
        let (worker, _) = rt.presented("thread-3").unwrap();
        assert!(worker[0] >= home[0] + 24.0 && worker[1] > home[1]);
        assert!(
            !rt.state
                .sidebar_threads()
                .iter()
                .any(|i| rt.state.threads[*i].id == 3)
        );
        press(&mut rt, "thread-3");
        press(&mut rt, "thread-actions");
        press(&mut rt, "menu-archive-thread");
        assert!(!rt.state.current().archived());
    }

    #[test]
    fn nested_worker_alerts_cover_input_failure_and_unread_results_without_auto_expanding() {
        let mut rt = runtime(1280.0);
        // Rosters start empty; these tests exercise the earlier named presets.
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.open_captain(0);
        let mut child = crate::model::Thread::new(99);
        child.parent = Some(rt.state.current().id);
        child.delegated = true;
        rt.state.threads.push(child);
        rt.frame(0.0);
        assert!(rt.presented("worker-alert-99").is_none());
        for state in 0..3 {
            let child = rt.state.threads.last_mut().unwrap();
            child.request = (state == 0).then(|| json!({"method":"input"}));
            child.status = if state == 1 {
                Status::Failed
            } else {
                Status::Completed
            };
            child.life.last_completed_at = Some(2);
            child.life.last_visited_at = Some(if state == 2 { 1 } else { 3 });
            rt.invalidate();
            rt.frame(1.0);
            assert!(rt.presented("worker-alert-99").is_some());
        }
        rt.state.select_thread(0);
        rt.invalidate();
        rt.frame(1.0);
        assert!(rt.find("thread-99").is_none());
    }

    #[test]
    fn pinned_captain_child_has_one_row_and_retains_its_draft() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        rt.state.current_mut().draft.replace_text("Keep this draft");
        press(&mut rt, "thread-actions");
        rt.frame(1.0 / 60.0);
        assert!(rt.state.thread_menu, "the menu opens");
        assert!(
            rt.find("menu-pin-thread").is_some(),
            "the pin item is shown"
        );
        press(&mut rt, "menu-pin-thread");
        assert!(rt.state.current().pinned());
        assert_eq!(rt.state.current().draft.text, "Keep this draft");
        assert!(rt.find("thread-1").is_some());
    }
    #[test]
    fn sidebar_context_and_header_share_rename_pin_archive_and_delete_actions() {
        let mut rt = runtime(1280.0);
        rt.state.current_mut().draft.replace_text("A prompt");
        rt.frame(0.0);
        let (x, y) = rt.find("thread-1").unwrap();
        rt.secondary_down(x, y);
        rt.secondary_up(x, y);
        rt.frame(0.0);
        assert_eq!(rt.state.row_menu, Some(RowTarget::Thread(1)));
        for key in [
            "menu-rename-thread",
            "menu-pin-thread",
            "menu-archive-thread",
            "menu-delete-thread",
        ] {
            assert!(rt.find(key).is_some(), "{key} missing from row menu");
        }
        press(&mut rt, "menu-rename-thread");
        rt.state.rename_edit.replace_text("Renamed in sidebar");
        rt.frame(1.0);
        press(&mut rt, "save-thread-name");
        assert_eq!(rt.state.current().title, "Renamed in sidebar");
        press(&mut rt, "thread-actions");
        press(&mut rt, "menu-pin-thread");
        assert!(rt.state.current().pinned());
        // The row has no actions button; a right-click opens its menu at the pointer.
        assert!(rt.find("thread-more-1").is_none());
        let (x, y) = rt.find("thread-1").unwrap();
        rt.secondary_down(x + 30.0, y);
        rt.secondary_up(x + 30.0, y);
        rt.frame(0.0);
        assert!(rt.state.row_menu_at.is_some());
        press(&mut rt, "menu-delete-thread");
        assert_eq!(
            rt.state.delete_thread,
            Some(1),
            "delete menu opens confirmation"
        );
        assert_eq!(
            rt.state.threads.len(),
            1,
            "confirmation does not delete yet"
        );
        press(&mut rt, "cancel-thread-dialog");
        assert_eq!(rt.state.threads.len(), 1);
        press(&mut rt, "thread-actions");
        press(&mut rt, "menu-delete-thread");
        press(&mut rt, "confirm-delete-thread");
        assert!(rt.state.threads.iter().all(|t| t.id != 1));
        assert!(rt.state.projects.entries[0].path.is_dir());
    }

    #[test]
    fn project_picker_removes_only_the_project_entry_and_can_create_projectless_threads() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        rt.state.open_picker(ProjectPicker::NewThread);
        rt.invalidate();
        rt.frame(1.0);
        let (x, y) = rt.find("project-option-1").unwrap();
        rt.secondary_down(x, y);
        rt.secondary_up(x, y);
        rt.frame(0.0);
        assert_eq!(rt.state.row_menu, Some(RowTarget::Project(1)));
        press(&mut rt, "menu-remove-project");
        assert!(rt.state.projects.entries[0].hidden);
        assert_eq!(rt.state.threads[0].project, Some(1));
        // With no workspace in view, New thread asks where.
        press(&mut rt, "new-thread");
        assert_eq!(rt.state.project_picker, Some(ProjectPicker::NewThread));
        press(&mut rt, "choose-no-project");
        assert!(rt.state.current().project.is_none());
        assert!(rt.state.current().captain.is_none());
    }
    fn assert_inside(rt: &mut Runtime<State>, width: f32, keys: &[&str]) {
        for key in keys {
            let (r, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key} is missing"));
            assert!(
                r[0] >= 0.0 && r[0] + r[2] <= width + 1.0 && r[2] > 0.0,
                "{width}: {key} {r:?}"
            );
        }
    }
    fn crew_runtime(width: f32) -> Runtime<State> {
        let mut rt = runtime(width);
        crate::captain_workspace_demo(&mut rt.state);
        rt.invalidate();
        rt.frame(0.0);
        rt
    }

    #[test]
    fn captain_crew_surfaces_fit_and_open_workers_at_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = crew_runtime(width);
            assert_eq!(rt.state.current().id, 7);
            assert_inside(
                &mut rt,
                width,
                &[
                    "task-card-16",
                    "task-clock-16",
                    "resume-task-18",
                    "dismiss-task-18",
                    "result-open-1",
                    "needs-you",
                    "needs-you-2",
                    "approve-2",
                    "decline-2",
                    "crew-tray",
                    "crew-chip-19",
                    "composer",
                ],
            );
            // Delegations are task cards, not generic tool rows, and finished tasks stay off
            // the tray.
            assert!(rt.presented("inline-tool-7-0").is_none());
            assert!(rt.presented("crew-chip-1").is_none());
            assert!(rt.presented("crew-chip-18").is_none());
            // The approval stack scrolls inside KSG's max-h-80.
            assert!(rt.presented("needs-you").unwrap().0[3] <= 320.5);
            press(&mut rt, "crew-chip-16");
            rt.frame(1.0);
            assert_eq!(rt.state.current().id, 16, "{width}");
            assert_inside(&mut rt, width, &["parent-bar", "open-captain", "composer"]);
            assert!(rt.presented("crew-tray").is_none());
            press(&mut rt, "open-captain");
            assert_eq!(rt.state.current().id, 7, "{width}");
            press(&mut rt, "open-request-2");
            assert_eq!(rt.state.current().id, 2, "{width}");
        }
    }

    #[test]
    fn the_crew_tray_is_two_equal_columns_from_560_and_one_column_below() {
        let chips = |rt: &Runtime<State>| -> Vec<[f32; 4]> {
            ["crew-chip-16", "crew-chip-17", "crew-chip-19"]
                .iter()
                .map(|key| rt.presented(key).unwrap().0)
                .collect()
        };
        let rt = crew_runtime(1280.0);
        let wide = chips(&rt);
        // Three chips fill two rows of two columns, so two of them share a row.
        let mut rows: Vec<f32> = wide.iter().map(|c| c[1]).collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup_by(|a, b| (*a - *b).abs() < 1.0);
        assert_eq!(rows.len(), 2, "two rows: {wide:?}");
        let mut columns: Vec<f32> = wide.iter().map(|c| c[0]).collect();
        columns.sort_by(f32::total_cmp);
        columns.dedup_by(|a, b| (*a - *b).abs() < 1.0);
        assert_eq!(columns.len(), 2, "two columns: {wide:?}");
        for chip in &wide {
            assert!(
                (chip[2] - wide[0][2]).abs() < 1.0,
                "equal columns: {wide:?}"
            );
        }
        assert!(
            (columns[1] - columns[0] - wide[0][2] - 8.0).abs() < 1.0,
            "8 px between columns"
        );
        assert!(
            (rows[1] - rows[0] - 30.0 - 6.0).abs() < 1.0,
            "6 px between rows"
        );
        // The timer keeps its width beside the longest title.
        for chip in ["16", "17", "19"] {
            let clock = rt.presented(&format!("crew-chip-clock-{chip}")).unwrap().0;
            assert!(clock[2] >= 20.0, "timer {chip}: {clock:?}");
        }
        let rt = crew_runtime(560.0);
        let narrow = chips(&rt);
        for pair in narrow.windows(2) {
            assert!(
                (pair[0][0] - pair[1][0]).abs() < 1.0 && (pair[0][1] - pair[1][1]).abs() > 20.0,
                "narrow lanes stack in one column: {narrow:?}"
            );
            assert!(
                (pair[0][2] - pair[1][2]).abs() < 1.0,
                "stacked chips share a width"
            );
        }
    }

    #[test]
    fn attention_row_keeps_one_height_and_opens_threads_from_its_popover() {
        for width in [960.0, 1280.0, 1920.0] {
            let mut rt = crew_runtime(width);
            let (row, _) = rt.presented("attention-row").unwrap();
            assert_eq!(row[3], 24.0);
            assert_inside(&mut rt, width, &["attention-row", "attention-count"]);
            press(&mut rt, "attention-row");
            assert!(rt.state.attention_menu);
            assert_inside(&mut rt, width, &["attention-menu", "attention-item-18"]);
            press(&mut rt, "attention-item-18");
            assert_eq!(rt.state.current().id, 18);
            assert!(!rt.state.attention_menu);
        }
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        for t in &mut rt.state.threads {
            t.request = None;
            if t.status == Status::Failed {
                t.status = Status::Ready;
            }
        }
        rt.invalidate();
        rt.frame(1.0);
        assert!(rt.state.attention().is_empty());
        assert!(rt.presented("attention-row").is_none());
        let keys = ["workspace-context", "thread-search-field", "sidebar"];
        let before: Vec<_> = keys.iter().map(|k| rt.presented(k).unwrap().0).collect();
        rt.state.threads[0].request = Some(json!({"method":"confirm"}));
        rt.invalidate();
        rt.frame(1.0);
        assert!(rt.presented("attention-row").is_some());
        let after: Vec<_> = keys.iter().map(|k| rt.presented(k).unwrap().0).collect();
        assert_eq!(before, after, "the attention pill never moves the sidebar");
    }

    #[test]
    fn recovery_notice_reviews_the_first_interrupted_task_or_dismisses() {
        for width in [960.0, 1280.0, 1920.0] {
            let mut rt = crew_runtime(width);
            rt.state.recovery_notice = Some("Shina was closed while 2 tasks were running.".into());
            rt.invalidate();
            rt.frame(0.0);
            assert_inside(&mut rt, width, &["recovery-notice", "review-recovery"]);
            press(&mut rt, "review-recovery");
            assert_eq!(rt.state.current().id, 18);
            assert!(rt.state.recovery_notice.is_none());
            rt.state.recovery_notice = Some("Notice".into());
            rt.invalidate();
            press(&mut rt, "dismiss-recovery");
            assert!(rt.state.recovery_notice.is_none());
        }
    }

    #[test]
    fn the_recovery_notice_puts_dismiss_at_the_top_right_and_counts_what_review_shows() {
        for width in [960.0, 1280.0, 1920.0] {
            let mut rt = crew_runtime(width);
            rt.state.recovery_notice = Some("Shina was closed while 2 tasks were running.".into());
            rt.invalidate();
            rt.frame(0.0);
            let (card, _) = rt.presented("recovery-notice").unwrap();
            let (dismiss, _) = rt.presented("dismiss-recovery").unwrap();
            let (review, _) = rt.presented("review-recovery").unwrap();
            assert!(
                dismiss[1] < card[1] + 20.0 && dismiss[0] + dismiss[2] > card[0] + card[2] - 20.0,
                "{width}: dismiss {dismiss:?} is not at the top right of {card:?}"
            );
            assert!(
                review[1] > dismiss[1],
                "{width}: Review is below the text row"
            );
        }
    }

    #[test]
    fn a_goal_less_captain_with_no_tools_shows_the_hero_above_the_crew_tray() {
        let mut rt = runtime(1280.0);
        crate::captain_workspace_demo(&mut rt.state);
        let home = rt.state.current_mut();
        home.messages.clear();
        home.tools.clear();
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let (hero, _) = rt
            .presented("captain-hero-portrait")
            .expect("the hero draws");
        let (tray, _) = rt.presented("crew-tray").expect("the crew tray draws");
        assert!(hero[1] + hero[3] <= tray[1], "{hero:?} {tray:?}");
    }

    #[test]
    fn the_empty_thread_headline_without_a_workspace_reads_what_should_we_build() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.state.new_thread_in(None);
        rt.frame(0.0);
        let labels: Vec<String> = rt
            .elements()
            .into_iter()
            .filter_map(|element| element.text)
            .collect();
        assert!(
            labels
                .iter()
                .any(|text| text.contains("What should we build?")),
            "{labels:?}"
        );
    }

    #[test]
    fn the_narrow_scene_docks_the_compact_sidebar_with_the_small_wordmark() {
        let mut rt = runtime(620.0);
        crate::captain_workspace_demo(&mut rt.state);
        rt.state.compact_sidebar = true;
        rt.state.sidebar_width = 200.0;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let (side, _) = rt.presented("sidebar").expect("the sidebar is docked");
        assert!(side[2] <= 216.5, "{side:?}");
        let (word, _) = rt.presented("shina-wordmark").unwrap();
        assert!(word[2] < 40.0, "the wordmark drops to 11 px: {word:?}");
    }

    #[test]
    fn the_clone_destination_and_its_button_are_both_32_tall() {
        for width in [620.0, 960.0, 1280.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.open_clone_dialog();
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            let (choose, _) = rt.presented("clone-choose-parent").unwrap();
            assert!((choose[3] - 32.0).abs() < 0.5, "{width}: {choose:?}");
        }
    }

    #[test]
    fn the_snooze_popover_has_four_pixels_of_padding_and_a_titled_first_row() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        let id = rt.state.threads[0].id;
        rt.state.open_snooze(id);
        rt.frame(0.0);
        rt.frame(1.0);
        let (menu, _) = rt.presented("snooze-menu").unwrap();
        assert!(menu[2] > 0.0 && menu[3] > 0.0, "{menu:?}");
    }

    #[test]
    fn the_session_drawer_header_has_a_rule_on_the_chat_headers_line() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.state.show_activity = true;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let (rule, _) = rt.presented("session-header-rule").unwrap();
        let (chat, _) = rt.presented("chat-header-rule").unwrap();
        assert!((rule[1] - chat[1]).abs() < 1.0, "{rule:?} {chat:?}");
    }

    #[test]
    fn an_empty_captain_greets_with_a_large_portrait_at_supported_widths() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.captains = crate::captains::Captain::presets();
            rt.state.open_captain(0);
            rt.frame(0.0);
            assert_inside(
                &mut rt,
                width,
                &["captain-hero-portrait", "captain-goal-heading", "composer"],
            );
            let (portrait, _) = rt.presented("captain-hero-portrait").unwrap();
            let (heading, _) = rt.presented("captain-goal-heading").unwrap();
            let (composer, _) = rt.presented("composer").unwrap();
            assert_eq!(portrait[2], HERO_PORTRAIT);
            assert!(portrait[1] + portrait[3] < heading[1]);
            assert!(heading[1] + heading[3] < composer[1]);
        }
    }

    #[test]
    fn the_horizon_band_crowns_the_greeting_and_the_portrait_overlaps_its_edge() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            rt.frame(0.0);
            rt.frame(1.0);
            let (pane, _) = rt.presented("chat-pane").unwrap();
            let (band, _) = rt.presented("horizon-band").unwrap();
            let (heading, _) = rt.presented("empty-heading").unwrap();
            assert_eq!(band[3], HORIZON_HEIGHT);
            assert!(band[3] >= 96.0 && band[3] <= 120.0, "96 to 120 pt tall");
            assert!(band[2] <= measure(true) + 0.5, "inside the content lane");
            assert!(
                band[0] >= pane[0] - 0.5 && band[0] + band[2] <= pane[0] + pane[2] + 0.5,
                "band overflows the pane at {width}: {band:?} in {pane:?}"
            );
            assert!(
                band[1] + band[3] <= heading[1],
                "the band never sits behind text at {width}: {band:?} {heading:?}"
            );

            let mut rt = runtime(width);
            rt.state.captains = crate::captains::Captain::presets();
            rt.state.open_captain(0);
            rt.frame(0.0);
            let (band, _) = rt.presented("horizon-band").unwrap();
            let (portrait, _) = rt.presented("captain-hero-portrait").unwrap();
            let (heading, _) = rt.presented("captain-goal-heading").unwrap();
            assert!(
                portrait[1] < band[1] + band[3] && portrait[1] + portrait[3] > band[1] + band[3],
                "the portrait crosses the band's bottom edge at {width}"
            );
            assert!(
                portrait[1] > band[1],
                "the portrait stays inside the band's top"
            );
            assert!(portrait[1] + portrait[3] < heading[1]);
        }
    }

    #[test]
    fn the_sidebar_sea_strip_sits_under_the_footer_icons() {
        for width in [960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.frame(0.0);
            let (sidebar, _) = rt.presented("sidebar").unwrap();
            let (footer, _) = rt.presented("sidebar-footer").unwrap();
            let (sea, _) = rt.presented("sidebar-sea").unwrap();
            assert_eq!(sea[3], SEA_HEIGHT);
            assert!(
                sea[1] >= footer[1] + footer[3] - 0.5,
                "icons sit above the sea"
            );
            // It bleeds: flush to the window's left and bottom edges, and as wide as the sidebar.
            let (root, _) = rt.presented("workspace-root").unwrap();
            assert!(sea[0].abs() < 0.5, "flush left at {width}: {sea:?}");
            assert!(
                (sea[1] + sea[3] - (root[1] + root[3])).abs() < 0.5,
                "flush to the bottom at {width}: {sea:?} in {root:?}"
            );
            assert!(
                (sea[0] + sea[2] - (sidebar[0] + sidebar[2])).abs() < 0.5,
                "the sidebar's full width at {width}: {sea:?} {sidebar:?}"
            );
        }
    }

    /// Opens `count` distinct threads. Each has a title and a sent message, so none is a blank
    /// draft that `new_thread` would reuse.
    fn open_distinct_threads(rt: &mut Runtime<State>, count: usize) {
        for n in 0..count {
            rt.state.new_thread();
            let thread = rt.state.current_mut();
            thread.title = format!("Thread number {n} with a longer title");
            thread.life.latest_user_message_at = Some(lifecycle::now());
        }
    }

    fn settle_frames(rt: &mut Runtime<State>) {
        rt.invalidate();
        rt.frame(0.0);
        for _ in 0..4 {
            rt.frame(1.0);
        }
    }

    fn tab_key(rt: &Runtime<State>, id: usize) -> String {
        if rt.state.current().id == id {
            "active-tab".to_string()
        } else {
            format!("tab-{id}")
        }
    }

    #[test]
    fn tabs_share_the_strip_and_the_active_tab_stays_whole() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            for extra in [0usize, 2, 6, 12] {
                let mut rt = runtime(width);
                open_distinct_threads(&mut rt, extra);
                rt.set_viewport((width, 800.0), 1.0);
                settle_frames(&mut rt);
                let mut stages: Vec<(&str, Box<dyn Fn(&mut Runtime<State>)>)> = vec![
                    ("the last tab", Box::new(|_| {})),
                    (
                        "the first tab",
                        Box::new(|rt| {
                            let first = rt.state.tabs[0];
                            let index = rt.state.threads.iter().position(|t| t.id == first);
                            rt.state.select_thread(index.unwrap());
                        }),
                    ),
                    (
                        "the last tab again",
                        Box::new(|rt| {
                            let last = *rt.state.tabs.last().unwrap();
                            let index = rt.state.threads.iter().position(|t| t.id == last);
                            rt.state.select_thread(index.unwrap());
                        }),
                    ),
                    (
                        "after closing the last tab",
                        Box::new(|rt| {
                            let last = *rt.state.tabs.last().unwrap();
                            rt.state.close_tab(last);
                        }),
                    ),
                ];
                for (stage, step) in stages.drain(..) {
                    step(&mut rt);
                    settle_frames(&mut rt);
                    let (strip, _) = rt.presented("thread-tabs").unwrap();
                    let (plus, _) = rt.presented("new-tab").unwrap();
                    let active = rt.presented("active-tab").map(|(rect, _)| rect).unwrap();
                    let label = format!("{width}/{extra}/{stage}");
                    assert!(
                        plus[0] >= strip[0] + strip[2] - 0.5,
                        "{label}: the plus overlaps the strip {strip:?} {plus:?}"
                    );
                    assert!(
                        active[0] >= strip[0] - 0.5
                            && active[0] + active[2] <= strip[0] + strip[2] + 0.5,
                        "{label}: the active tab {active:?} is clipped by {strip:?}"
                    );
                    for id in rt.state.tabs.clone() {
                        let (tab, _) = rt.presented(&tab_key(&rt, id)).unwrap();
                        assert!(
                            tab[2] >= TAB_MIN_WIDTH - 0.5 && tab[2] <= TAB_MAX_WIDTH + 0.5,
                            "{label}: {tab:?}"
                        );
                    }
                    let count = rt.state.tabs.len() as f32;
                    if count * (TAB_MIN_WIDTH + TAB_GAP) <= strip[2] {
                        // Everything fits at the minimum, so nothing scrolls and every tab is whole.
                        for id in rt.state.tabs.clone() {
                            let (tab, _) = rt.presented(&tab_key(&rt, id)).unwrap();
                            assert!(
                                tab[0] >= strip[0] - 0.5
                                    && tab[0] + tab[2] <= strip[0] + strip[2] + 0.5,
                                "{label}: tab {tab:?} escapes {strip:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn narrowing_the_strip_scrolls_a_middle_active_tab_back_into_view() {
        let mut rt = runtime(1280.0);
        open_distinct_threads(&mut rt, 12);
        let middle = rt.state.tabs[rt.state.tabs.len() / 2];
        let index = rt
            .state
            .threads
            .iter()
            .position(|t| t.id == middle)
            .unwrap();
        rt.state.select_thread(index);
        // Tabs are already at their minimum width at these sizes, so only the strip changes.
        for width in [1100.0, 980.0, 860.0, 760.0, 700.0] {
            rt.set_viewport((width, 800.0), 1.0);
            settle_frames(&mut rt);
            let (strip, _) = rt.presented("thread-tabs").unwrap();
            let (active, _) = rt.presented("active-tab").unwrap();
            assert_eq!(rt.presented("active-tab").unwrap().0[2], TAB_MIN_WIDTH);
            assert!(
                active[0] >= strip[0] - 0.5 && active[0] + active[2] <= strip[0] + strip[2] + 0.5,
                "{width}: the active tab {active:?} is out of view in {strip:?}"
            );
        }
    }

    #[test]
    fn the_enabled_send_button_is_coral_and_the_stop_button_is_the_same_key() {
        for dark in [false, true] {
            let p = palette(dark);
            let fill_of = |rt: &Runtime<State>, key: &str| {
                rt.elements()
                    .into_iter()
                    .find(|element| element.key.as_deref() == Some(key))
                    .unwrap_or_else(|| panic!("{key} is missing"))
                    .fill
            };
            let mut rt = runtime(1280.0);
            rt.state.dark = dark;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            assert_eq!(
                fill_of(&rt, "send-message"),
                p.line,
                "disabled, dark={dark}"
            );
            rt.state.current_mut().draft.replace_text("Hello");
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            assert_eq!(
                fill_of(&rt, "send-message"),
                p.coral,
                "enabled, dark={dark}"
            );
            let (send, _) = rt.presented("send-message").unwrap();
            assert_eq!((send[2], send[3]), (32.0, 32.0));
            // While a run is busy and the draft is empty, the stop button takes its place with
            // the same size and the same coral fill.
            let t = rt.state.current_mut();
            t.draft.replace_text("");
            t.status = Status::Running;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            assert_eq!(fill_of(&rt, "stop-agent"), p.coral, "stop, dark={dark}");
            let (stop, _) = rt.presented("stop-agent").unwrap();
            assert_eq!((stop[2], stop[3]), (32.0, 32.0));
        }
    }

    #[test]
    fn the_wordmark_stays_beside_the_attention_badge_in_a_docked_sidebar() {
        for width in [960.0, 1280.0, 1920.0] {
            let rt = crew_runtime(width);
            let (word, _) = rt.presented("shina-wordmark").unwrap_or_else(|| {
                panic!("{width}: the wordmark is missing beside the attention badge")
            });
            let (badge, _) = rt.presented("attention-row").unwrap();
            assert!(
                word[0] + word[2] + BADGE_CLEARANCE <= badge[0] + 0.5,
                "{width}: the badge crowds the wordmark {word:?} {badge:?}"
            );
            // At 22 px the five cells are 70 px wide; the narrow fallback is 11 px.
            assert!(
                (word[2] - WORDMARK_WIDTH).abs() < 1.0 || word[2] < 40.0,
                "{width}: {word:?}"
            );
        }
    }

    #[test]
    fn the_attention_popover_is_340_wide_keeps_its_reasons_and_sits_under_its_badge() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            crate::captain_workspace_demo(&mut rt.state);
            rt.state.compact_sidebar = true;
            rt.state.demo_stop(19);
            rt.state.recovery_notice = Some("Two tasks stopped.".into());
            rt.state.brief.demo_status = Some(crate::model::BackgroundStatus {
                hidden_to_tray: true,
                working: 2,
                ..Default::default()
            });
            rt.state.attention_menu = true;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            let (menu, _) = rt.presented("attention-menu").unwrap();
            assert!(
                (menu[2] - 340.0f32.min(width - 16.0)).abs() < 0.5,
                "{width}: {menu:?}"
            );
            // With the sidebar docked, the popover hangs from the top bar with its left edge on
            // the chat pane's, so it covers neither the sidebar nor the recovery banner's dismiss
            // control at the lane's right. Closed, it opens 6 px under its badge.
            let (pane, _) = rt.presented("chat-pane").unwrap();
            let (sidebar, _) = rt.presented("sidebar").unwrap();
            assert!(
                (menu[0] - pane[0]).abs() < 0.5 && (menu[1] - pane[1]).abs() < 0.5,
                "{width}: {menu:?} {pane:?}"
            );
            assert!(
                menu[0] >= sidebar[0] + sidebar[2],
                "{width}: over the sidebar"
            );
            let (dismiss, _) = rt.presented("dismiss-recovery").unwrap();
            let apart = menu[0] + menu[2] <= dismiss[0]
                || dismiss[0] + dismiss[2] <= menu[0]
                || menu[1] + menu[3] <= dismiss[1]
                || dismiss[1] + dismiss[3] <= menu[1];
            assert!(
                apart,
                "{width}: {menu:?} covers the dismiss control {dismiss:?}"
            );
            assert!(
                menu[0] >= 0.0 && menu[0] + menu[2] <= width + 0.5,
                "{width}: {menu:?}"
            );
            for item in rt.state.attention() {
                let (reason, _) = rt
                    .presented(&format!("attention-reason-{}", item.thread_id))
                    .unwrap();
                assert!(reason[2] >= 80.0 - 0.5, "{width}: {reason:?}");
                assert!(reason[0] + reason[2] <= menu[0] + menu[2], "{width}");
            }
        }
    }

    #[test]
    fn the_wordmark_keeps_22_px_in_a_normal_sidebar_with_the_traffic_lights_and_a_badge() {
        // A macOS title strip reserves about 72 pt for the window buttons on the left, and the
        // header keeps HEADER_TAB_GAP at its end.
        let line = |sidebar: f32| sidebar - TITLE_CONTROLS - HEADER_TAB_GAP;
        assert_eq!(wordmark_size(line(232.0), true), Some(22.0));
        assert_eq!(wordmark_size(line(232.0), false), Some(22.0));
        assert_eq!(wordmark_size(line(216.0), true), Some(11.0));
        assert_eq!(wordmark_size(line(200.0), true), Some(11.0));
        assert_eq!(wordmark_size(line(120.0), true), None);
    }

    #[test]
    fn a_narrow_sidebar_shows_the_11_px_wordmark_under_the_real_traffic_light_inset() {
        // macOS reports a 76 pt leading inset; Windows and Linux report none.
        for leading in [76.0, 0.0] {
            for sidebar in [200.0, 208.0, 216.0] {
                let mut rt = runtime(1280.0);
                crate::captain_workspace_demo(&mut rt.state);
                rt.state.sidebar_width = sidebar;
                let mut chrome = rune::ChromeInsets::default();
                chrome.leading = leading;
                rt.set_chrome_insets(chrome);
                rt.frame(0.0);
                rt.frame(0.5);
                let at = format!("leading {leading}, sidebar {sidebar}");
                let (mark, _) = rt
                    .presented("shina-wordmark")
                    .unwrap_or_else(|| panic!("{at}: no wordmark"));
                let size = rt
                    .elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some("shina-wordmark"))
                    .unwrap()
                    .rect[3];
                assert!(size < 20.0, "{at}: the wordmark is not 11 px ({size})");
                let (badge, _) = rt.presented("attention-row").unwrap();
                assert!(
                    mark[0] + mark[2] + BADGE_CLEARANCE <= badge[0] + 0.5,
                    "{at}: wordmark {mark:?} runs into the badge {badge:?}"
                );
            }
        }
    }

    #[test]
    fn an_undocked_sidebar_keeps_the_mark_and_the_attention_badge_in_the_top_bar() {
        for width in [620.0, 960.0] {
            let mut rt = runtime(width);
            crate::captain_workspace_demo(&mut rt.state);
            rt.state.compact_sidebar = false;
            rt.state.sidebar = false;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            let (toggle, _) = rt.presented("sidebar-toggle").unwrap();
            let (logo, _) = rt.presented("shina-logo").unwrap();
            assert_eq!((logo[2], logo[3]), (24.0, 24.0));
            assert!(logo[0] >= toggle[0] + toggle[2] - 0.5, "{width}");
            let (badge, _) = rt
                .presented("attention-row")
                .unwrap_or_else(|| panic!("{width}: the attention badge is missing"));
            assert!(
                badge[0] >= logo[0] + logo[2] + BADGE_CLEARANCE - 0.5,
                "{width}"
            );
            let (tab, _) = rt.presented("active-tab").unwrap();
            assert!(
                (tab[0] - (badge[0] + badge[2]) - HEADER_TAB_GAP).abs() < 0.5,
                "{width}: {badge:?} {tab:?}"
            );
        }
    }

    #[test]
    fn the_narrowest_docked_sidebar_keeps_the_mark_the_badge_and_the_tab_apart() {
        let mut rt = runtime(1280.0);
        crate::captain_workspace_demo(&mut rt.state);
        rt.state.sidebar_width = 200.0;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let (sidebar, _) = rt.presented("sidebar").unwrap();
        assert_eq!(sidebar[2], 200.0);
        let (logo, _) = rt.presented("shina-logo").unwrap();
        let (badge, _) = rt.presented("attention-row").unwrap();
        let (tab, _) = rt.presented("active-tab").unwrap();
        assert!(logo[0] + logo[2] <= badge[0] + 0.5, "{logo:?} {badge:?}");
        if let Some((word, _)) = rt.presented("shina-wordmark") {
            assert!(
                word[0] + word[2] + BADGE_CLEARANCE <= badge[0] + 0.5,
                "{word:?} {badge:?}"
            );
        }
        assert!((tab[0] - (badge[0] + badge[2]) - HEADER_TAB_GAP).abs() < 0.5);
    }

    #[test]
    fn the_first_tab_clears_the_badge_by_12_and_the_pane_rule_by_4() {
        for width in [960.0, 1280.0, 1920.0] {
            let rt = crew_runtime(width);
            let (badge, _) = rt.presented("attention-row").unwrap();
            let (tab, _) = rt.presented("active-tab").unwrap();
            assert!(
                (tab[0] - (badge[0] + badge[2]) - HEADER_TAB_GAP).abs() < 0.5,
                "{width}: the badge sits {} pt before the first tab",
                tab[0] - (badge[0] + badge[2])
            );
            let (pane, _) = rt.presented("chat-pane").unwrap();
            assert!(
                (pane[1] - (tab[1] + tab[3]) - 4.0).abs() < 0.5,
                "{width}: the tab ends {} pt above the pane",
                pane[1] - (tab[1] + tab[3])
            );
        }
    }

    #[test]
    fn an_empty_captain_keeps_the_reference_composer_width() {
        let mut rt = runtime(1280.0);
        rt.state.open_captain(0);
        rt.frame(0.0);
        let (rect, _) = rt.presented("composer").unwrap();
        assert!(
            rect[2] > 700.0 && rect[2] < 770.0,
            "composer width was {}",
            rect[2]
        );
    }
    #[test]
    fn a_named_captain_goal_wraps_without_pushing_the_composer_out_of_the_window() {
        let mut state = State::new(".".into(), false);
        // Rosters start empty; these tests exercise the earlier named presets.
        state.captains = crate::captains::Captain::presets();
        state.open_captain(0);
        state.current_mut().title = "Polish the Shina workspace and keyboard navigation".into();
        state.current_mut().renamed = true;
        let mut rt = Runtime::with_text(
            state,
            view,
            (620.0, 480.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
        );
        rt.frame(0.0);
        for key in ["captain-goal-heading", "composer", "send-message"] {
            let (rect, _) = rt.presented(key).unwrap();
            assert!(
                rect[0] >= 0.0
                    && rect[1] >= 0.0
                    && rect[0] + rect[2] <= 620.5
                    && rect[1] + rect[3] <= 480.5,
                "{key} is outside the minimum window: {rect:?}",
            );
        }
        assert!(rt.presented("captain-goal-heading").unwrap().0[3] > 32.0);
    }
    #[test]
    fn an_empty_captain_does_not_count_a_waiting_worker_as_running() {
        let mut rt = runtime(1280.0);
        // Rosters start empty; these tests exercise the earlier named presets.
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.open_captain(0);
        let mut worker = crate::model::Thread::new(99);
        worker.parent = Some(rt.state.current().id);
        worker.status = Status::Waiting;
        rt.state.threads.push(worker);
        rt.frame(0.0);
        assert!(rt.presented("captain-working-count").is_none());
        rt.state.threads.last_mut().unwrap().status = Status::Running;
        rt.invalidate();
        rt.frame(1.0);
        assert!(rt.presented("captain-working-count").is_some());
    }
    #[test]
    fn composer_has_no_connection_controls_and_pickers_work_without_a_process() {
        let mut rt = runtime(1280.0);
        rt.state.current_mut().apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"provider":"cpa","id":"model-a","name":"Model A","reasoning":true,
             "thinkingLevelMap":{"minimal":null,"low":null,"medium":null}}
        ]}}));
        rt.frame(0.0);
        assert!(rt.find("connect-pi").is_none());
        assert!(rt.find("disconnect-pi").is_none());
        press(&mut rt, "model-effort");
        rt.frame(1.0 / 60.0);
        press(&mut rt, "open-model-list");
        rt.frame(1.0 / 60.0);
        press(&mut rt, "choose-model-0");
        assert!(rt.state.current().pi.is_none());
        assert_eq!(
            rt.state.settings_picker,
            Some(SettingsPicker::Thinking),
            "picking a model returns to the slider"
        );
        assert_eq!(rt.state.current().model_id, "model-a");
        rt.frame(0.0);
        press(&mut rt, "choose-thinking-1");
        assert_eq!(rt.state.current().thinking, "high");
        assert!(rt.state.current().pi.is_none());
        assert!(
            rt.state.settings_picker.is_some(),
            "the popover stays open after a level change"
        );
    }

    #[test]
    fn native_model_and_reasoning_pickers_send_real_rpc_commands() {
        let mut rt = runtime(1280.0);
        let (pi, commands, _events) = crate::backend::Pi::test_connection();
        rt.state.current_mut().pi = Some(pi);
        rt.state.current_mut().status = Status::Ready;
        rt.state.current_mut().apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"provider":"cpa","id":"model-a","name":"Model A","reasoning":true,
             "thinkingLevelMap":{"minimal":null,"low":null,"medium":null}}
        ]}}));
        rt.state.current_mut().apply(json!({"type":"response","command":"get_available_thinking_levels","success":true,"data":{"levels":["off","high"]}}));
        rt.frame(0.0);
        press(&mut rt, "model-effort");
        rt.frame(1.0 / 60.0);
        assert!(rt.find("pi-settings-picker").is_some());
        press(&mut rt, "open-model-list");
        rt.frame(1.0 / 60.0);
        press(&mut rt, "choose-model-0");
        let model = commands.try_recv().unwrap();
        assert_eq!(model["type"], "set_model");
        assert_eq!(model["provider"], "cpa");
        assert_eq!(model["modelId"], "model-a");
        assert!(rt.state.current().configuring);
        rt.state.current_mut().apply(json!({"type":"response","command":"set_model","success":true,"data":{"provider":"cpa","id":"model-a","name":"Model A"}}));
        assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
        assert_eq!(
            commands.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        assert_eq!(commands.try_recv().unwrap()["type"], "get_session_stats");
        rt.frame(0.0);
        // Pi's replies arrive outside a frame here, so redraw the open popover with them.
        rt.invalidate();
        press(&mut rt, "choose-thinking-1");
        let thinking = commands.try_recv().unwrap();
        assert_eq!(thinking["type"], "set_thinking_level");
        assert_eq!(thinking["level"], "high");
        rt.state
            .current_mut()
            .apply(json!({"type":"response","command":"set_thinking_level","success":true}));
        assert_eq!(rt.state.current().thinking, "high");
    }
    fn with_levels(rt: &mut Runtime<State>) {
        rt.state.current_mut().apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"provider":"cpa","id":"gpt-6.1-sol","name":"GPT 6.1 Sol (CPA → Codex)","reasoning":true,
             "thinkingLevelMap":{"off":null,"low":null,"medium":null,"high":null}}
        ]}}));
        let t = rt.state.current_mut();
        t.model = "GPT 6.1 Sol (CPA → Codex)".into();
        t.model_provider = "cpa".into();
        t.model_id = "gpt-6.1-sol".into();
        t.thinking_levels = vec!["off".into(), "low".into(), "medium".into(), "high".into()];
        t.thinking = "medium".into();
        rt.invalidate();
    }

    #[test]
    fn model_popover_opens_above_its_trigger_inside_the_window() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            with_levels(&mut rt);
            rt.frame(0.0);
            press(&mut rt, "model-effort");
            rt.frame(1.0);
            let (trigger, _) = rt.presented("model-effort").unwrap();
            let (card, _) = rt.presented("pi-settings-picker").unwrap();
            assert!(
                card[1] + card[3] <= trigger[1] + 0.5,
                "popover must open above the trigger at {width}: {card:?} {trigger:?}"
            );
            assert!(
                card[0] >= 0.0 && card[0] + card[2] <= width + 0.5 && card[1] >= 0.0,
                "popover leaves the window at {width}: {card:?}"
            );
            press(&mut rt, "open-model-list");
            rt.frame(2.0);
            let (list, _) = rt.presented("pi-settings-picker").unwrap();
            assert!(
                list[1] >= 0.0,
                "model list leaves the window at {width}: {list:?}"
            );
        }
    }

    #[test]
    fn effort_slider_moves_by_keyboard_click_and_drag() {
        let mut rt = runtime(1280.0);
        with_levels(&mut rt);
        rt.frame(0.0);
        press(&mut rt, "model-effort");
        rt.frame(1.0);
        let focused = |rt: &Runtime<State>| {
            rt.elements()
                .into_iter()
                .any(|element| element.key.as_deref() == Some("effort-slider") && element.focused)
        };
        for _ in 0..40 {
            if focused(&rt) {
                break;
            }
            rt.key(rune::KeyEvent {
                key: rune::Key::Tab,
                mods: Default::default(),
            });
            rt.frame(1.0);
        }
        assert!(focused(&rt), "Tab reaches the effort slider");
        rt.key(rune::KeyEvent {
            key: rune::Key::Right,
            mods: Default::default(),
        });
        rt.frame(1.5);
        assert_eq!(rt.state.current().thinking, "high");
        rt.key(rune::KeyEvent {
            key: rune::Key::Home,
            mods: Default::default(),
        });
        rt.frame(1.5);
        assert_eq!(rt.state.current().thinking, "off");
        rt.frame(2.0);
        press(&mut rt, "choose-thinking-2");
        assert_eq!(rt.state.current().thinking, "medium");
        rt.frame(3.0);
        let (track, _) = rt.presented("effort-slider").unwrap();
        let y = track[1] + track[3] / 2.0;
        rt.pointer_down(track[0] + 4.0, y);
        rt.pointer_move(track[0] + track[2] * 0.3, y);
        rt.frame(3.5);
        let held = rt.state.thinking_drag.expect("the drag holds the thumb");
        assert!(
            (0.2..0.4).contains(&held),
            "the thumb follows the pointer, not the nearest stop: {held}"
        );
        rt.pointer_move(track[0] + track[2] - 4.0, y);
        rt.pointer_up(track[0] + track[2] - 4.0, y);
        rt.frame(4.0);
        assert_eq!(rt.state.current().thinking, "high");
        assert_eq!(rt.state.thinking_drag, None);
        assert!(rt.state.settings_picker.is_some(), "the popover stays open");
    }

    #[test]
    fn effort_track_fills_to_its_end_and_sparkles_only_at_the_top_level() {
        let mut rt = runtime(1280.0);
        with_levels(&mut rt);
        rt.state.current_mut().thinking = "high".into();
        rt.invalidate();
        rt.frame(0.0);
        press(&mut rt, "model-effort");
        rt.frame(5.0);
        let (track, _) = rt.presented("effort-slider").unwrap();
        let (thumb, _) = rt.presented("effort-thumb").unwrap();
        assert!(
            (thumb[0] - (track[0] + track[2] - SLIDER_H / 2.0)).abs() < 1.0,
            "the top level puts the thumb at the end cap: {thumb:?} in {track:?}"
        );
        assert!(
            rt.find("effort-sparkles").is_some(),
            "the top level sparkles"
        );
        rt.state.current_mut().thinking = "off".into();
        rt.invalidate();
        rt.frame(10.0);
        let (thumb, _) = rt.presented("effort-thumb").unwrap();
        assert!(
            (thumb[0] - (track[0] + SLIDER_H / 2.0)).abs() < 1.0,
            "the lowest level puts the thumb at the start cap: {thumb:?} in {track:?}"
        );
        assert!(rt.find("effort-sparkles").is_none());
    }

    #[test]
    fn model_labels_split_their_route_and_pick_ksg_provider_marks() {
        assert_eq!(
            split_model_label("Gemini 3.8 Flash (CPA → Antigravity)"),
            ("Gemini 3.8 Flash", Some("CPA → Antigravity"))
        );
        assert_eq!(split_model_label("Model A"), ("Model A", None));
        assert_eq!(
            model_vendor("gpt-6.1-sol", "GPT 6.1 Sol (CPA → Codex)"),
            Vendor::OpenAi
        );
        assert_eq!(
            model_vendor("x", "Claude Opus 5.5 (CPA → Anthropic)"),
            Vendor::Anthropic
        );
        assert_eq!(
            model_vendor("gemini-3.8-flash", "Gemini 3.8 Flash"),
            Vendor::Antigravity
        );
        assert_eq!(model_vendor("model-a", "Model A"), Vendor::Other);
        assert_eq!(effort_name("xhigh"), "XHigh");
    }

    fn ui_png(width: u32) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            1,
            image::Rgba([20, 40, 60, 255]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn composer_attachment_control_adds_and_removes_named_chips_without_losing_text() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        assert!(rt.find("attach-image").is_some());
        rt.state.current_mut().draft.replace_text("Keep this text");
        rt.state.attach_image("pasted.png", &ui_png(1));
        rt.frame(0.0);
        let id = rt.state.current().draft_images[0].id.clone();
        assert!(rt.find(&format!("attachment-{id}")).is_some());
        assert!(rt.find(&format!("attachment-thumb-{id}")).is_some());
        assert_eq!(rt.state.current().draft.text, "Keep this text");
        press(&mut rt, &format!("remove-attachment-{id}"));
        assert!(rt.state.current().draft_images.is_empty());
        assert_eq!(rt.state.current().draft.text, "Keep this text");
        assert!(rt.find(&format!("attachment-{id}")).is_none());
    }

    #[test]
    fn paste_attaches_clipboard_images_without_inserting_placeholder_text() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        press(&mut rt, "composer-input");
        let image = rune::ClipboardImage::from_png(ui_png(1)).unwrap();
        rt.paste_content(rune::ClipboardContent::new("").with_image(image));
        rt.frame(0.0);
        assert_eq!(rt.state.current().draft_images.len(), 1);
        assert!(rt.state.current().draft.text.is_empty());
        assert!(rt.state.current().has_draft_content());
        assert!(rt.find("send-message").is_some());
        let id = rt.state.current().draft_images[0].id.clone();
        assert!(rt.find(&format!("attachment-{id}")).is_some());
    }

    #[test]
    fn dropping_an_image_file_attaches_it_from_the_composer() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        let path = std::env::temp_dir().join(format!(
            "shina-ui-drop-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, ui_png(2)).unwrap();
        let (rect, _) = rt.presented("composer").unwrap();
        rt.drop_files(
            vec![path.clone()],
            Some((rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0)),
        );
        rt.frame(0.0);
        let _ = std::fs::remove_file(&path);
        assert_eq!(rt.state.current().draft_images.len(), 1);
        assert_eq!(
            rt.state.current().draft_images[0].name,
            path.file_name().unwrap().to_string_lossy().into_owned()
        );
    }

    #[test]
    fn new_thread_centres_the_composer_with_the_headline_above_it() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            rt.frame(0.0);
            rt.frame(1.0);
            let (pane, _) = rt.presented("chat-pane").unwrap();
            let (card, _) = rt.presented("composer").unwrap();
            let (heading, _) = rt.presented("empty-heading").unwrap();
            let (project, _) = rt.presented("draft-project").unwrap();
            let centre = card[1] + card[3] / 2.0;
            let pane_centre = pane[1] + pane[3] / 2.0;
            assert!(
                (centre - pane_centre).abs() < 60.0,
                "composer is off centre at {width}: {card:?} in {pane:?}"
            );
            assert!(
                heading[1] + heading[3] <= card[1] && card[1] - (heading[1] + heading[3]) < 40.0,
                "headline must sit just above the composer at {width}: {heading:?} {card:?}"
            );
            for rect in [heading, project] {
                assert!(
                    rect[0] >= pane[0] - 0.5 && rect[0] + rect[2] <= pane[0] + pane[2] + 0.5,
                    "headline overflows the pane at {width}: {rect:?} in {pane:?}"
                );
            }
            assert!(rt.find("suggest-review").is_none(), "no starter prompts");
            assert!(
                rt.find("context-usage-ring").is_none(),
                "no empty ring on a fresh draft"
            );
        }
    }

    #[test]
    fn draft_images_are_square_thumbnails_with_a_corner_close_button() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        rt.state.attach_image("Pasted image.png", &ui_png(40));
        rt.frame(0.0);
        let id = rt.state.current().draft_images[0].id.clone();
        let (tile, _) = rt.presented(&format!("attachment-{id}")).unwrap();
        assert_eq!((tile[2], tile[3]), (ATTACHMENT_THUMB, ATTACHMENT_THUMB));
        let (close, _) = rt.presented(&format!("remove-attachment-{id}")).unwrap();
        assert!(
            close[0] >= tile[0] && close[0] + close[2] <= tile[0] + tile[2] + 0.5,
            "close button must sit inside the thumbnail: {close:?} in {tile:?}"
        );
        assert!(
            !rt.elements()
                .into_iter()
                .any(|element| element.text.as_deref() == Some("Pasted image.png")),
            "the file name is not drawn as text"
        );
    }

    #[test]
    fn attachment_chips_stay_inside_a_narrow_composer() {
        let mut rt = runtime(620.0);
        rt.frame(0.0);
        rt.state.attach_image("wide screenshot.png", &ui_png(40));
        rt.frame(0.0);
        let (rect, _) = rt.presented("composer").unwrap();
        assert!(rect[0] >= 0.0 && rect[0] + rect[2] <= 620.5);
        let id = rt.state.current().draft_images[0].id.clone();
        let (chip, _) = rt.presented(&format!("attachment-{id}")).unwrap();
        assert!(
            chip[0] >= rect[0] && chip[0] + chip[2] <= rect[0] + rect[2] + 0.5,
            "attachment chip overflows the composer: {chip:?} in {rect:?}"
        );
    }

    fn rect(rt: &Runtime<State>, key: &str) -> [f32; 4] {
        rt.presented(key)
            .unwrap_or_else(|| panic!("{key} is not presented"))
            .0
    }

    #[test]
    fn captain_status_stays_inside_its_row_and_never_moves_siblings() {
        let mut status_widths = vec![];
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            for sidebar_width in [crate::model::DEFAULT_SIDEBAR_WIDTH, 200.0] {
                let mut rt = runtime(width);
                rt.state.populate_demo();
                rt.state.sidebar_width = sidebar_width;
                rt.state.compact_sidebar = width < 760.0;
                for captain in &mut rt.state.captains {
                    captain.name = format!("{} of the Extraordinarily Long Name", captain.name);
                }
                let homes: Vec<usize> = rt
                    .state
                    .sidebar_captains()
                    .iter()
                    .map(|i| rt.state.threads[*i].id)
                    .collect();
                assert!(homes.len() >= 2);
                let (first, next) = (homes[0], homes[1]);
                let i = rt.state.threads.iter().position(|t| t.id == first).unwrap();
                rt.state.threads[i].title =
                    "Coordinate a long migration across every workspace and release train".into();
                rt.state.threads[i].renamed = true;
                rt.invalidate();
                rt.frame(0.0);
                rt.frame(1.0);
                let siblings = [
                    format!("captain-name-{first}"),
                    format!("portrait-slot-{first}"),
                    format!("captain-badges-{first}"),
                    format!("thread-{next}"),
                ];
                let before: Vec<_> = siblings.iter().map(|k| rect(&rt, k)).collect();
                for step in 0..4 {
                    let t = &mut rt.state.threads[i];
                    t.request = None;
                    t.started = None;
                    match step {
                        0 => {
                            t.status = Status::Running;
                            t.started = Some(
                                std::time::Instant::now() - std::time::Duration::from_secs(3599),
                            );
                        }
                        1 => {
                            t.status = Status::Waiting;
                            t.request = Some(json!({"method":"confirm"}));
                        }
                        2 => t.status = Status::Failed,
                        _ => t.status = Status::Completed,
                    }
                    rt.invalidate();
                    rt.frame(1.0);
                    rt.frame(1.0);
                    let row = rect(&rt, &format!("thread-{first}"));
                    let mut parts = vec![
                        format!("captain-state-{first}"),
                        format!("captain-status-{first}"),
                        format!("captain-name-{first}"),
                    ];
                    if step == 0 {
                        parts.push(format!("captain-timer-{first}"));
                    }
                    for key in parts {
                        let r = rect(&rt, &key);
                        assert!(
                            r[2] > 0.0
                                && r[0] >= row[0] - 0.5
                                && r[0] + r[2] <= row[0] + row[2] + 0.5
                                && r[1] >= row[1] - 0.5
                                && r[1] + r[3] <= row[1] + row[3] + 0.5,
                            "{width}/{sidebar_width} step {step}: {key} {r:?} leaves {row:?}"
                        );
                    }
                    if step == 1 {
                        status_widths.push(rect(&rt, &format!("captain-state-{first}"))[2]);
                    }
                    let after: Vec<_> = siblings.iter().map(|k| rect(&rt, k)).collect();
                    for (key, (a, b)) in siblings.iter().zip(before.iter().zip(&after)) {
                        assert!(
                            a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.5),
                            "{width}/{sidebar_width} step {step}: {key} moved from {a:?} to {b:?}"
                        );
                    }
                }
            }
        }
        // "Needs you" is never squeezed: it has its full width at the narrowest sidebar too.
        let widest = status_widths.iter().cloned().fold(0.0, f32::max);
        assert!(
            status_widths.iter().all(|w| (widest - w).abs() < 0.5),
            "{status_widths:?}"
        );
    }

    #[test]
    fn thread_card_timers_stay_inside_narrow_rows() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.sidebar_width = 200.0;
            rt.state.compact_sidebar = width < 760.0;
            let i = rt
                .state
                .threads
                .iter()
                .position(|t| t.status == Status::Running && !rt.state.is_captain_home(t))
                .unwrap();
            let id = rt.state.threads[i].id;
            rt.state.threads[i].title = "An extremely long thread title that cannot fit".into();
            rt.state.threads[i].started =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(3000));
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            let row = rect(&rt, &format!("thread-{id}"));
            let side = rect(&rt, "sidebar");
            assert!(
                row[0] + row[2] <= side[0] + side[2] + 0.5,
                "{width}: {row:?}"
            );
            let label = rt
                .accessibility_json()
                .to_string()
                .contains("Working 50m 0s");
            assert!(label, "{width}: the running timer is shown");
        }
    }

    #[test]
    fn sidebar_portraits_play_only_while_shown_focused_and_allowed_to_move() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        rt.frame(0.1);
        rt.frame(0.1);
        let now = std::time::Instant::now();
        assert!(
            tick_portraits(&mut rt.state, now).is_some(),
            "on-screen captains animate"
        );
        rt.state.sidebar = false;
        assert!(
            tick_portraits(&mut rt.state, now).is_none(),
            "hidden sidebar"
        );
        rt.state.sidebar = true;
        rt.state.portraits.set_focused(false);
        assert!(
            tick_portraits(&mut rt.state, now).is_none(),
            "unfocused window"
        );
        rt.state.portraits.set_focused(true);
        rt.set_system_reduced_motion(true);
        rt.invalidate();
        rt.frame(0.1);
        assert!(
            tick_portraits(&mut rt.state, now).is_none(),
            "reduced motion"
        );
    }
}
