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
#[path = "pi_ui.rs"]
mod pi_ui;
pub(crate) use pi_ui::questionnaire_key;

// Warm neutral chrome, a paper-like reading area, and Shina's teal action color.
#[derive(Clone, Copy)]
struct Palette {
    bg: Color,
    panel: Color,
    card: Color,
    line: Color,
    ink: Color,
    muted: Color,
    selected: Color,
    teal: Color,
    teal_bg: Color,
    gold: Color,
    danger: Color,
    on_teal: Color,
    badge_ring: Color,
    teal_text: Color,
    field_line: Color,
}
fn color(hex: &str) -> Color {
    rune::color::hex(hex).expect("palette color")
}
fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: color("#242422"),
            panel: color("#1c1c1a"),
            card: color("#30302d"),
            line: color("#e3dfd5").alpha(0.12),
            ink: color("#efede6"),
            muted: color("#aaa79e"),
            // Rune blends in linear light, so a small alpha still gives a visible highlight.
            selected: color("#e3dfd5").alpha(0.02),
            teal: color("#70b9ad"),
            teal_bg: color("#263f39"),
            gold: color("#d3ba6b"),
            danger: color("#f0a39c"),
            on_teal: color("#0d1f1e"),
            badge_ring: color("#242422"),
            teal_text: color("#89c8bc"),
            field_line: color("#5e5c55"),
        }
    } else {
        Palette {
            bg: color("#eeece5"),
            panel: color("#faf9f5"),
            card: color("#fffefa"),
            line: color("#34332d").alpha(0.12),
            ink: color("#292923"),
            muted: color("#66645b"),
            selected: color("#34332d").alpha(0.075),
            teal: color("#23776b"),
            teal_bg: color("#e0ede7"),
            gold: color("#8a701f"),
            danger: color("#a33e38"),
            on_teal: Color::WHITE,
            badge_ring: color("#eeece5"),
            teal_text: color("#23776b"),
            field_line: color("#bcb9ae"),
        }
    }
}
pub const SANS_FAMILY: &str = "Anthropic Sans Variable Text, Inter Variable";
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/InterVariable.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4Variable.ttf"),
];
const BODY: TextStyle = TextStyle::new(14.0, 400.0).lh(21.0).tracking(-0.10);
/// Assistant answers use Cursor's agent-window text: 15/24 sans with slightly tight tracking.
const PROSE: TextStyle = TextStyle::new(15.0, 400.0).lh(24.0).tracking(-0.20);
const COMPOSER: TextStyle = TextStyle::new(15.0, 400.0).lh(23.0).tracking(-0.10);
/// The new-thread greeting, ported from KSG Code's `DraftHeroHeadline`: centred sans,
/// `text-3xl font-normal tracking-tight`, or `text-2xl` when the pane is narrow.
const HEADING: TextStyle = TextStyle::new(30.0, 400.0).lh(36.0).tracking(-0.75);
const HEADING_NARROW: TextStyle = TextStyle::new(24.0, 400.0).lh(32.0).tracking(-0.6);
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
const TIMER: TextStyle = ACTIVITY.features(TABULAR);
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
fn button(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
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
        .radius(5.0)
        .hover(p.selected)
        .press(p.selected)
        .on_press(action)
        .child(
            label(title, TextStyle::new(12.0, 450.0), p.ink)
                .truncate()
                .shrink(),
        )
}
fn icon(kind: &str, ink: Color) -> El {
    macro_rules! glyph {
        ($name:ident,$body:expr) => {
            static $name: Icon = Icon {
                name: stringify!($name),
                view: 16.0,
                body: $body,
            };
        };
    }
    glyph!(
        PLUS,
        r#"<path d="M8 3v10M3 8h10" stroke="currentColor" stroke-linecap="round"/>"#
    );
    glyph!(
        SIDEBAR,
        r#"<rect x="2" y="2.5" width="12" height="11" rx="1.5" stroke="currentColor"/><path d="M6 2.5v11" stroke="currentColor"/>"#
    );
    glyph!(
        COMPOSE,
        r#"<path d="M8.5 3H4a1.5 1.5 0 0 0-1.5 1.5V12A1.5 1.5 0 0 0 4 13.5h7.5A1.5 1.5 0 0 0 13 12V7.5M8 8l5-5a1.1 1.1 0 0 0-1.5-1.5L6.5 6.5 6 9z" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        CHEVRON_RIGHT,
        r#"<path d="m6.5 4.5 3.5 3.5-3.5 3.5" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        SEND,
        r#"<path d="m4 7 4-4 4 4M8 3v10" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        CLOUD,
        r#"<path d="M4.5 12.5a3 3 0 0 1-.6-5.95 4.2 4.2 0 0 1 8-.6 3.35 3.35 0 0 1 .1 6.55z" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        PIN,
        r#"<path d="m6 2 7 7-2.5.5L8 12 4 8l2.5-2.5zM6 10l-4 4" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        CLOCK,
        r#"<circle cx="8" cy="8.5" r="5" stroke="currentColor" fill="none"/><path d="M8 5.5v3.3l2 1.2M2 3l2-1M12 2l2 1" stroke="currentColor" stroke-linecap="round"/>"#
    );
    glyph!(
        MORE,
        r#"<circle cx="3" cy="8" r=".75" fill="currentColor"/><circle cx="8" cy="8" r=".75" fill="currentColor"/><circle cx="13" cy="8" r=".75" fill="currentColor"/>"#
    );
    glyph!(
        LIST,
        r#"<path d="M6 4h8M6 8h8M6 12h8M2 4h.5M2 8h.5M2 12h.5" stroke="currentColor" stroke-linecap="round"/>"#
    );
    glyph!(
        BRANCH,
        r#"<circle cx="4" cy="3" r="1.5" stroke="currentColor"/><circle cx="12" cy="3" r="1.5" stroke="currentColor"/><circle cx="4" cy="13" r="1.5" stroke="currentColor"/><path d="M4 4.5v7M12 4.5V6a3 3 0 0 1-3 3H4" stroke="currentColor" fill="none"/>"#
    );
    glyph!(
        STOP,
        r#"<rect x="4.5" y="4.5" width="7" height="7" rx=".5" fill="currentColor"/>"#
    );
    glyph!(
        GRID,
        r#"<rect x="2.5" y="3" width="11" height="10" rx="1" stroke="currentColor"/><path d="M2.5 6h11M7 3v10" stroke="currentColor"/>"#
    );
    glyph!(
        ARCHIVE,
        r#"<rect x="2.5" y="3" width="11" height="3" rx=".5" stroke="currentColor"/><path d="M3.5 6v7h9V6M6.5 9h3" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        BUBBLE,
        r#"<path d="M3 3.5h10a1 1 0 0 1 1 1V10a1 1 0 0 1-1 1H7l-3 2.5V11H3a1 1 0 0 1-1-1V4.5a1 1 0 0 1 1-1z" fill="none" stroke="currentColor" stroke-linejoin="round"/>"#
    );
    glyph!(
        PANEL,
        r#"<rect x="2" y="2.5" width="12" height="11" rx="1.5" stroke="currentColor" fill="none"/><path d="M10 2.5v11" stroke="currentColor"/>"#
    );
    glyph!(
        ISSUE,
        r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" fill="none"/><circle cx="8" cy="8" r="1.5" fill="currentColor"/>"#
    );
    glyph!(
        BOLT,
        r#"<path d="M9 1.5 3.5 9H8l-1 5.5L12.5 7H8z" fill="none" stroke="currentColor" stroke-linejoin="round"/>"#
    );
    glyph!(DOT, r#"<circle cx="8" cy="8" r="3" fill="currentColor"/>"#);
    glyph!(
        THEME,
        r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" fill="none"/><path d="M8 2.5a5.5 5.5 0 0 1 0 11z" fill="currentColor"/>"#
    );
    glyph!(
        PENCIL,
        r#"<path d="M10.5 2.5 13.5 5.5 6 13H3v-3z" fill="none" stroke="currentColor" stroke-linejoin="round"/><path d="m9 4 3 3" stroke="currentColor"/>"#
    );
    glyph!(
        FILTER,
        r#"<path d="M2.5 4.5h11M4.5 8h7M6.5 11.5h3" stroke="currentColor" stroke-linecap="round"/>"#
    );
    glyph!(
        FOLDER_PLUS,
        r#"<path d="M2 4.5c0-.83.67-1.5 1.5-1.5h2.6l1.4 1.5h5c.83 0 1.5.67 1.5 1.5v5.5c0 .83-.67 1.5-1.5 1.5h-9c-.83 0-1.5-.67-1.5-1.5z" stroke="currentColor" stroke-linejoin="round" fill="none"/><path d="M8 7v3.5M6.25 8.75h3.5" stroke="currentColor" stroke-linecap="round"/>"#
    );
    glyph!(
        SETTINGS,
        r#"<circle cx="8" cy="8" r="2" stroke="currentColor" fill="none"/><path d="M6.9 1.75h2.2l.35 1.6 1.2.7 1.55-.5 1.1 1.9-1.2 1.1v1.4l1.2 1.1-1.1 1.9-1.55-.5-1.2.7-.35 1.6H6.9l-.35-1.6-1.2-.7-1.55.5-1.1-1.9 1.2-1.1v-1.4L2.7 5.45l1.1-1.9 1.55.5 1.2-.7z" stroke="currentColor" stroke-linejoin="round" fill="none"/>"#
    );
    glyph!(
        SUN,
        r#"<circle cx="8" cy="8" r="2.75" stroke="currentColor" fill="none"/><path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.06 1.06M11.54 11.54l1.06 1.06M3.4 12.6l1.06-1.06M11.54 4.46l1.06-1.06" stroke="currentColor" stroke-linecap="round"/>"#
    );
    glyph!(
        MOON,
        r#"<path d="M13 9.5A5.5 5.5 0 0 1 6.5 3a5.5 5.5 0 1 0 6.5 6.5z" fill="none" stroke="currentColor" stroke-linejoin="round"/>"#
    );
    glyph!(
        ALERT,
        r#"<circle cx="8" cy="8" r="5.75" stroke="currentColor" fill="none"/><path d="M8 5v3.5" stroke="currentColor" stroke-linecap="round"/><circle cx="8" cy="11" r=".8" fill="currentColor"/>"#
    );
    glyph!(
        FILE,
        r#"<path d="M9.5 1.5H4a1 1 0 0 0-1 1v11a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V5zM9 1.5V5h4M5.5 8h5M5.5 10.5h3" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        TERMINAL,
        r#"<rect x="1.5" y="2.5" width="13" height="11" rx="1.5" fill="none" stroke="currentColor"/><path d="m4 6 2 2-2 2M8.5 10H12" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        EXPAND,
        r#"<path d="M9.5 2.5h4v4M13.5 2.5 9 7M6.5 13.5h-4v-4M2.5 13.5 7 9" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        IMAGE,
        r#"<rect x="2" y="2.5" width="12" height="11" rx="1.5" fill="none" stroke="currentColor"/><circle cx="5.5" cy="6" r="1.3" fill="currentColor"/><path d="m2.5 11 3-3 2.5 2.5 2-2 3 3" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        MONITOR,
        r#"<rect x="1.5" y="2.5" width="13" height="8.5" rx="1.25" fill="none" stroke="currentColor"/><path d="M5.5 13.5h5M8 11v2.5" stroke="currentColor" stroke-linecap="round"/>"#
    );
    glyph!(
        PAPERCLIP,
        r#"<path d="M13.5 7.5 8 13a3.5 3.5 0 0 1-5-5l5.75-5.75a2.25 2.25 0 0 1 3.2 3.2L6.2 11.2a1 1 0 0 1-1.4-1.4L10 4.6" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    glyph!(
        CHEVRON_LEFT,
        r#"<path d="m9.5 4.5-3.5 3.5 3.5 3.5" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    // Lucide's `Minimize2`, the icon on KSG Code's Compact button.
    glyph!(
        MINIMIZE,
        r#"<path d="M9.33 2.67v4h4M2.67 9.33h4v4M9.33 6.67 14 2M2 14l4.67-4.67" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
    let glyph = match kind {
        "plus" => &PLUS,
        "minimize" => &MINIMIZE,
        "paperclip" => &PAPERCLIP,
        "back" => &CHEVRON_LEFT,
        "monitor" => &MONITOR,
        "sidebar" => &SIDEBAR,
        "compose" => &COMPOSE,
        "edit" => &PENCIL,
        "send" => &SEND,
        "thread" => &BUBBLE,
        "cloud" => &CLOUD,
        "panel" => &PANEL,
        "issue" => &ISSUE,
        "bolt" => &BOLT,
        "dot" => &DOT,
        "theme" => &THEME,
        "settings" => &SETTINGS,
        "filter" => &FILTER,
        "folder-plus" => &FOLDER_PLUS,
        "sun" => &SUN,
        "moon" => &MOON,
        "alert" => &ALERT,
        "pin" => &PIN,
        "clock" => &CLOCK,
        "more" => &MORE,
        "list" => &LIST,
        "branch" => &BRANCH,
        "stop" => &STOP,
        "grid" => &GRID,
        "archive" => &ARCHIVE,
        "folder" => &rune_kit::glyphs::FOLDER,
        "search" => &rune_kit::glyphs::SEARCH,
        "chevron" => &rune_kit::glyphs::CHEVRON_DOWN,
        "open" => &CHEVRON_RIGHT,
        "lock" => &rune_kit::glyphs::LOCK,
        "close" => &rune_kit::glyphs::CLOSE,
        "file" => &FILE,
        "terminal" => &TERMINAL,
        "expand" => &EXPAND,
        "image" => &IMAGE,
        _ => &rune_kit::glyphs::CHECK_SMALL,
    };
    rune::icon(glyph, 14.0).color(ink).a11y_hidden()
}
fn tool_icon(kind: &str, ink: Color) -> El {
    icon(kind, ink).size(16.0, 16.0)
}
fn primary_button(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .center_y()
        .h(32.0)
        .px(14.0)
        .radius(6.0)
        .fill(p.teal)
        .on_press(action)
        .child(label(title, TextStyle::new(13.0, 560.0), p.on_teal))
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
        .radius(7.0)
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
        .radius(5.0)
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
        Status::Waiting => p.gold,
        Status::Running => p.teal_text,
        _ => p.muted,
    }
}
/// One status vocabulary everywhere: a dot for working, a ring for needs you, a square for
/// failed, and a check for settled. Ready and offline have no mark.
fn status_mark(status: Status, p: Palette, size: f32) -> El {
    let ink = status_ink(status, p);
    match status {
        Status::Running => el().size(size, size).radius(size / 2.0).fill(ink),
        Status::Waiting => el()
            .size(size, size)
            .radius(size / 2.0)
            .border((size / 3.5).max(1.5), ink),
        Status::Failed => el().size(size - 1.0, size - 1.0).radius(1.5).fill(ink),
        Status::Completed => icon("check", ink).size(size + 5.0, size + 5.0),
        _ => el()
            .size(size, size)
            .radius(size / 2.0)
            .fill(ink.alpha(0.6)),
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
        .child(label(status.label(), SMALL, status_ink(status, p)))
}
/// Room for "Working for 59m 59s" in the chat header, so live status never reflows the row
/// around it.
const HEADER_STATUS_SLOT: f32 = 132.0;
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
                label(text, SMALL.features(TABULAR), status_ink(status, p))
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

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let p = palette(s.dark);
    cx.foreground(p.ink);
    cx.focus_ring(2.0, 2.0, p.teal);
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
        let show_logo = sidebar_width - padding - 16.0 >= 72.0;
        top = top.child(
            sidebar_header(s, p, compact, show_logo)
                .w(sidebar_width)
                .pl(padding)
                .pr(16.0),
        );
    } else {
        top = top.pl(leading.max(8.0));
        top = top.child(sidebar_toggle(p, compact));
    }
    top = top.child(header(s, p, narrow, compact).grow(1.0).min_w(0.0));
    let mut workspace = row().grow(1.0).min_h(0.0).px(8.0).pb(8.0);
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
        .radius(10.0)
        .clip_rounded(10.0);
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
            .child(chat_header(s, p, narrow, compact));
        let main = if captain_ui::is_earlier(s) {
            main.child(conversation(s, p, wide, main_width))
                .child(captain_ui::earlier_bar(s, p, wide, main_width))
        } else if s.current().messages.is_empty()
            && s.current().tools.is_empty()
            && s.current().native.compactions.is_empty()
        {
            main.child(empty_thread(s, p, narrow, wide, main_width))
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
    for dialog in captain_ui::dialogs(s, p) {
        root = root.child(dialog);
    }
    if docked && let Some(pill) = crew_ui::attention_row(s, p, sidebar_width < 240.0) {
        root = root.child(pill);
    }
    if s.attention_menu && docked && !s.attention().is_empty() {
        root = root.child(crew_ui::attention_menu(s, p));
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
    static CHIP: Icon = Icon {
        name: "CHIP",
        view: 16.0,
        body: r#"<rect x="4" y="4" width="8" height="8" rx="1.5" fill="none" stroke="currentColor"/><path d="M6.5 2v2M9.5 2v2M6.5 12v2M9.5 12v2M2 6.5h2M2 9.5h2M12 6.5h2M12 9.5h2" stroke="currentColor" stroke-linecap="round"/>"#,
    };
    match vendor {
        Vendor::Antigravity => el()
            .size(size, size)
            .pixels_fit(s.artwork.antigravity.pixels(), rune::SurfaceFit::Fit)
            .a11y_hidden(),
        Vendor::OpenAi => rune::icon(&OPENAI, size).color(p.ink).a11y_hidden(),
        Vendor::Anthropic => rune::icon(&CLAUDE, size).a11y_hidden(),
        Vendor::Other => rune::icon(&CHIP, size).color(p.muted).a11y_hidden(),
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
        .radius(8.0)
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
            label(name, TextStyle::new(13.0, 500.0), p.ink)
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(label(effort, TextStyle::new(13.0, 500.0), p.muted))
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
        .radius(14.0)
        .clip_rounded(14.0)
        .fill(p.card)
        .border(1.0, p.line)
        .shadow(
            12.0,
            32.0,
            Color::BLACK.alpha(if s.dark { 0.45 } else { 0.18 }),
        )
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
                        .fill(p.on_teal)
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
                    if count == 0 { p.muted } else { p.teal_text },
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
        .fill(p.selected)
        .border(0.5, p.line);
    if count == 0 {
        return section.child(slider.child(track).opacity(0.5)).child(label(
            "This model has no reasoning levels.",
            SMALL,
            p.muted,
        ));
    }
    let fill = |on: bool| if on { p.teal } else { Color::TRANSPARENT };
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
                    p.on_teal.alpha(0.7)
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
                        // Codex's thumb: white, a hairline edge, and a soft 2 px shadow.
                        el().key("effort-thumb-knob")
                            .absolute(-cap, 0.0)
                            .size(SLIDER_H, SLIDER_H)
                            .radius(cap)
                            .fill(Color::WHITE)
                            .border(0.5, p.field_line)
                            .shadow(0.0, 2.0, Color::BLACK.alpha(0.12)),
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
                .radius(8.0)
                .fill(p.selected)
                .grow(1.0)
                .min_w(0.0)
                .child(icon("search", p.muted))
                .child(
                    text_input(&s.model_search, TextStyle::new(13.0, 420.0))
                        .key("model-search")
                        .label("Search models")
                        .placeholder("Search models", p.muted)
                        .color(p.ink)
                        .caret(p.teal, p.teal_bg)
                        .grow(1.0)
                        .min_w(0.0)
                        .on_edit(|s: &mut State| &mut s.model_search),
                ),
        );
    let query = s.model_search.text.to_lowercase();
    let mut list = col()
        .key("model-list")
        .w_full()
        .max_h(max_h)
        .pb(6.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let mut group: Option<String> = None;
    let mut count = 0;
    list = list.child(
        label("Harness", TextStyle::new(11.5, 600.0), p.muted)
            .key("harness-group")
            .px(16.0)
            .pt(8.0)
            .pb(4.0),
    );
    let harnesses = [("pi", "Pi"), ("codex", "Codex")].into_iter().chain(
        s.harnesses
            .agents
            .iter()
            .map(|agent| (agent.id.as_str(), agent.name.as_str())),
    );
    for (id, name) in harnesses {
        let selected = t.harness == id;
        let enabled = t.can_choose_harness();
        let chosen = id.to_owned();
        let mut item = row()
            .key(format!("choose-harness-{id}"))
            .role("option")
            .label(name)
            .checked(selected)
            .focusable(enabled)
            .center_y()
            .gap(10.0)
            .h(34.0)
            .pl(16.0)
            .pr(12.0)
            .fill(if selected {
                p.teal_bg
            } else {
                Color::TRANSPARENT
            })
            .hover(if selected { p.teal_bg } else { p.selected })
            .disabled(!enabled)
            .child(harness_icon(s, p, id, name))
            .child(
                label(name, TextStyle::new(13.5, 450.0), p.ink)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        if !matches!(id, "pi" | "codex") {
            // KSG Code's `ModelListRow` "New" tag, here naming the protocol the agent speaks.
            item = item.child(
                label("ACP", TextStyle::new(9.5, 650.0).tracking(0.4), p.muted)
                    .key(format!("harness-tag-{id}"))
                    .px(4.0)
                    .py(1.0)
                    .radius(4.0)
                    .border(1.0, p.ink.alpha(0.12)),
            );
        }
        item = item.child(spacer());
        if selected {
            item = item.child(icon("check", p.teal_text));
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
            .pl(16.0)
            .pr(12.0)
            .fill(if selected {
                p.teal_bg
            } else {
                Color::TRANSPARENT
            })
            .hover(if selected { p.teal_bg } else { p.selected })
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
            item = item.child(icon("check", p.teal_text));
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
        list = list.child(
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

/// The sidebar's half of the title strip, left-aligned as T3 Code lays it out: the sidebar
/// toggle, then Shina's mark and name. Search and actions sit in the sidebar below.
fn sidebar_header(s: &State, p: Palette, compact: bool, show_logo: bool) -> El {
    let mut header = row()
        .h(36.0)
        .center_y()
        .gap(8.0)
        .child(sidebar_toggle(p, compact));
    if show_logo {
        header = header
            .child(
                // The 32-cell sail stays crisp at 24 px, an exact three-quarter scale on 2x displays.
                el().key("shina-logo")
                    .size(24.0, 24.0)
                    .radius(5.0)
                    .clip_rounded(5.0)
                    .pixels(
                        if s.dark {
                            &s.artwork.mark_dark
                        } else {
                            &s.artwork.mark
                        }
                        .pixels(),
                    ),
            )
            .child(label(
                "Shina",
                TextStyle::new(15.0, 650.0).tracking(-0.25),
                p.ink,
            ));
    }
    header
        .child(spacer())
        .child(el().key(crew_ui::ATTENTION_ANCHOR).size(0.0, 28.0))
}

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
    let scope = if !s.scope_additional.is_empty() {
        format!("{} workspaces", s.scope_additional.len() + 1)
    } else {
        match s.project_scope {
            ProjectScope::All => "All workspaces".into(),
            ProjectScope::NoProject => "No workspace".into(),
            ProjectScope::Project(id) => s
                .project(Some(id))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "All workspaces".into()),
        }
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
                    .radius(8.0)
                    .grow(1.0)
                    .min_w(0.0)
                    .fill(if s.search.focused || !s.search.text.is_empty() {
                        p.selected
                    } else {
                        Color::TRANSPARENT
                    })
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
                    &format!("Workspace: {scope}"),
                    "filter",
                    scoped,
                    p,
                    |s| s.open_picker(ProjectPicker::Scope),
                )
                .help(format!("Workspace: {scope}")),
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
                .child(spacer())
                .child(icon_button(
                    "clear-project-scope",
                    "Show all workspaces",
                    "close",
                    p,
                    |s| {
                        s.project_scope = ProjectScope::All;
                        s.scope_additional.clear();
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
    let mut content = col().pb(2.0).gap(2.0).min_h(s.sidebar_viewport_height);
    let captains = s.sidebar_captains();
    if !s.show_archive || !captains.is_empty() {
        let mut heading = section_title("Captains", p);
        if !captains.is_empty() {
            heading = heading.child(
                label(captains.len().to_string(), SMALL, p.muted)
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
        for index in captains.iter().copied() {
            let home = s.threads[index].id;
            let workers = s.visible_captain_workers(home);
            let mut crew = col()
                .key(format!("crew-{home}"))
                .gap(1.0)
                .child(thread_row(s, index, p, false));
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
    let all = s.sidebar_threads();
    let visible = s.visible_sidebar_threads();
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
        .filter(|i| s.draft_listed(&s.threads[*i]))
        .collect();
    if !drafts.is_empty() {
        content = content.child(section_title("Drafts", p));
        for i in drafts {
            content = content.child(thread_row(s, i, p, false));
        }
        content = content.child(rule(p));
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
            .filter(|i| {
                !s.threads[*i].is_draft()
                    && s.thread_section(&s.threads[*i], lifecycle::now()) == section
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
            content = content.child(
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
                    .radius(6.0)
                    .hover(p.selected)
                    .on_drawn_layout(move |s: &mut State, rect| {
                        s.drop_rects.insert(marker.clone(), rect);
                    })
                    .on_press(move |s: &mut State| {
                        s.shelves[shelf] = !s.shelves[shelf];
                        s.checkpoint();
                    })
                    .child(label(title, TextStyle::new(13.0, 460.0), p.muted))
                    .child(spacer())
                    .child(label(rows.len().to_string(), SMALL, p.muted))
                    .child(icon(if open { "chevron" } else { "open" }, p.muted)),
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
                    .fill(if selected { p.teal_bg } else { p.selected })
                    .on_drawn_layout(move |s: &mut State, rect| {
                        s.drop_rects.insert(marker.clone(), rect);
                    })
                    .child(label(format!("{section:?}"), SMALL, p.muted)),
            );
        }
        for index in rows.iter().copied().filter(|i| visible.contains(i)) {
            content = content.child(thread_row(s, index, p, false));
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
        content = content.child(
            label(
                if s.search.text.is_empty() {
                    "No threads in this workspace yet"
                } else {
                    "No matching conversations"
                },
                SMALL,
                p.muted,
            )
            .wrap()
            .padding(8.0),
        );
    }
    side = side.child(
        col()
            .key("sidebar-scroll")
            .on_layout(|s: &mut State, rect| s.sidebar_viewport_height = rect[3])
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
    if let Some(error) = &s.lifecycle_error {
        side = side.child(label(error, SMALL, p.danger).wrap().padding(8.0));
    }
    if let Some(notice) = &s.undo {
        let count = notice.entries.len();
        let title = format!(
            "{}{} · Ctrl+Z",
            notice.action.label(),
            if count > 1 {
                format!(" {count}")
            } else {
                String::new()
            }
        );
        side = side.child(button("thread-undo", &title, p, |s| s.undo_thread_action()).h(32.0));
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
    side
}

/// Preferences behind the footer's gear: the "You" profile, background work, and the theme.
/// The background rows are KSG Code's `SettingsRow` with a `Switch` (see `captain_ui`); a
/// captain's routines live in its Brief.
fn settings_menu(s: &State, p: Palette) -> El {
    let mut menu = col()
        .key("settings-menu")
        .role("menu")
        .label("Settings")
        .w(320.0)
        .max_h(460.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .padding(4.0)
        .gap(1.0)
        .radius(10.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(
            10.0,
            30.0,
            Color::BLACK.alpha(if s.dark { 0.38 } else { 0.14 }),
        )
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
    row()
        .key(key)
        .role("menuitem")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .h(32.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(6.0)
        .hover(p.selected)
        .child(el().size(16.0, 16.0).center().child(icon(glyph, p.muted)))
        .child(
            label(title, TextStyle::new(13.0, 450.0), p.ink)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
}

/// Clone a repository into a new folder and open it.
fn clone_dialog(s: &State, p: Palette) -> El {
    let projects = &s.projects;
    let cloning = projects.cloning();
    let field = if s.dark { p.bg } else { p.panel };
    let mut card = col()
        .key("clone-dialog")
        .role("dialog")
        .label("Clone repository")
        .w(460.0)
        .padding(16.0)
        .gap(12.0)
        .radius(10.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(8.0, 24.0, Color::BLACK.alpha(0.15))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .center_y()
                .child(
                    col()
                        .gap(2.0)
                        .child(label(
                            "Clone repository",
                            TextStyle::new(14.0, 560.0),
                            p.ink,
                        ))
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
                .child(label("Repository", SMALL, p.muted))
                .child(
                    text_input(&projects.clone_source, BODY)
                        .key("clone-source")
                        .label("Repository URL or owner/repo")
                        .placeholder("owner/repo or https://github.com/owner/repo", p.muted)
                        .single_line()
                        .color(p.ink)
                        .caret(p.teal, p.teal_bg)
                        .w_full()
                        .padding(8.0)
                        .radius(6.0)
                        .fill(field)
                        .border(1.0, p.field_line)
                        .disabled(cloning)
                        .on_edit(|s: &mut State| &mut s.projects.clone_source)
                        .on_submit(|s: &mut State, _| s.start_clone()),
                ),
        )
        .child(
            col()
                .gap(5.0)
                .child(label("Clone into", SMALL, p.muted))
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            row()
                                .h(34.0)
                                .px(8.0)
                                .center_y()
                                .grow(1.0)
                                .min_w(0.0)
                                .radius(6.0)
                                .fill(field)
                                .border(1.0, p.field_line)
                                .child(
                                    label(projects.clone_parent.display().to_string(), BODY, p.ink)
                                        .truncate()
                                        .min_w(0.0),
                                ),
                        )
                        .child(
                            button("clone-choose-parent", "Choose...", p, |_| {})
                                .h(34.0)
                                .border(0.5, p.line)
                                .on_press_tx(crate::projects::pick_clone_parent)
                                .disabled(cloning),
                        ),
                ),
        );
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
    actions = actions.child(
        button(
            "clone-cancel",
            if cloning { "Stop" } else { "Cancel" },
            p,
            |s| s.close_clone_dialog(),
        )
        .h(32.0)
        .border(0.5, p.line),
    );
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
                    .fill(p.teal_bg)
                    .child(icon("send", p.teal_text))
                    .child(label(title, TextStyle::new(12.5, 520.0), p.teal_text)),
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
                .fill(p.teal_bg)
                .disabled(blocker.is_some())
                .a11y_value(description.unwrap_or_default())
                .child(icon("send", p.teal_text))
                .child(label(title, TextStyle::new(12.5, 520.0), p.teal_text));
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

fn section_title(title: &str, p: Palette) -> El {
    row()
        .h(30.0)
        .pl(ROW_INSET)
        .pr(4.0)
        .center_y()
        .child(label(title, TextStyle::new(12.5, 500.0), p.muted))
        .child(spacer())
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
fn avatar(s: &State, p: Palette, captain: usize, size: f32, status: Status) -> El {
    let portrait = s.captains.get(captain).map(|c| c.portrait).unwrap_or(0);
    portrait_avatar(s, p, portrait, size, status)
}
fn portrait_avatar(s: &State, p: Palette, captain: usize, size: f32, status: Status) -> El {
    let captain = captain.min(crate::visuals::CAPTAIN_NAMES.len() - 1);
    let radius = if size >= 30.0 { 9.0 } else { 4.0 };
    let portrait = el()
        .key(format!("avatar-{captain}-{}", size as u32))
        .size(size, size)
        .radius(radius)
        .clip_rounded(radius)
        .border(0.5, p.line)
        .pixels(
            if size < 30.0 {
                s.artwork.small_face(captain, face_for(status))
            } else {
                s.artwork.face(captain, face_for(status))
            }
            .pixels(),
        )
        .a11y_hidden();
    status_badge(p, portrait, size, status)
}

/// Captain rows: a portrait slot that holds the focal size, so rows never change height.
const CAPTAIN_SLOT: f32 = 44.0;
const CAPTAIN_PORTRAIT: f32 = 36.0;
/// Room for one badge at the end of the name line, kept even when there is none, so a
/// badge arriving never moves the name.
const BADGE_SLOT: f32 = 28.0;
/// Room for "59m 59s" at the end of the status line.
const TIMER_SLOT: f32 = 48.0;
const CAPTAIN_NAME: TextStyle = TextStyle::new(14.0, 560.0).tracking(-0.15);
const CAPTAIN_STATUS: TextStyle = TextStyle::new(12.5, 460.0);
/// Running clocks use tabular digits so a ticking timer keeps its width.
const SIDEBAR_TIMER: TextStyle = TextStyle::new(12.0, 520.0).features("'tnum' 1");

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
        .pixels(portrait, face)
        .unwrap_or_else(|| s.artwork.face(portrait, face).pixels());
    let radius = (size * 0.27).round();
    el().key(format!("portrait-{row}"))
        .size(size, size)
        .radius(radius)
        .clip_rounded(radius)
        .border(0.5, p.line)
        .pixels(pixels)
        .a11y_hidden()
        .on_visible(move |s: &mut State, on, tx| {
            s.portraits.set_visible(row, on);
            // The view doesn't read this; the next poll does.
            tx.live_only();
            tx.wake_after(std::time::Duration::ZERO);
        })
}

fn count_pill(count: usize, fill: Color, ink: Color) -> El {
    let text = if count > 9 {
        "9+".into()
    } else {
        count.to_string()
    };
    row()
        .h(16.0)
        .min_w(16.0)
        .px(5.0)
        .radius(8.0)
        .fill(fill)
        .center()
        .child(label(text, TextStyle::new(10.5, 650.0), ink))
}

/// A row's hover and selection washes, after Grok Bot's bot list: ink at about 6% on hover
/// and 8% when selected. Rune blends in linear light, so dark mode needs far less alpha.
fn row_washes(s: &State, p: Palette) -> (Color, Color) {
    if s.dark {
        (p.ink.alpha(0.014), p.selected)
    } else {
        (p.ink.alpha(0.055), p.selected)
    }
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
    let detail = match status {
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
            parts.extend(row_workspace(s, thread));
            parts.join(" · ")
        }
    };
    let timer = (status == Status::Running)
        .then(|| running.iter().filter_map(|t| t.started).min())
        .flatten();
    let status_ink = match status {
        Status::Ready => p.muted,
        Status::Completed => p.teal_text,
        other => status_ink(other, p),
    };
    let mut status_line = row()
        .key(format!("captain-status-{id}"))
        .h(18.0)
        .gap(4.0)
        .center_y()
        .child(
            el().size(10.0, 10.0)
                .center()
                .child(status_mark(status, p, 6.0)),
        )
        .child(
            label(status.label(), CAPTAIN_STATUS, status_ink).key(format!("captain-state-{id}")),
        );
    if !detail.is_empty() {
        status_line = status_line.child(
            label(format!("· {detail}"), CAPTAIN_STATUS, p.muted)
                .truncate()
                .shrink()
                .grow(1.0)
                .min_w(0.0),
        );
    } else {
        status_line = status_line.child(spacer());
    }
    if let Some(start) = timer {
        status_line = status_line.child(
            label(
                elapsed_label(start.elapsed().as_secs()),
                SIDEBAR_TIMER,
                p.muted,
            )
            .key(format!("captain-timer-{id}"))
            .w(TIMER_SLOT)
            .align(rune::Align::End),
        );
    }
    let badge = if requests > 0 || status == Status::Waiting {
        Some(count_pill(requests.max(1), p.gold, p.on_teal).key(format!("captain-attention-{id}")))
    } else if unread > 0 {
        Some(count_pill(unread, p.teal, p.on_teal).key(format!("captain-unread-{id}")))
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
    let size = if focal_captain(s) == Some(id) {
        CAPTAIN_SLOT
    } else {
        CAPTAIN_PORTRAIT
    };
    let portrait = captain_portrait(s, thread.captain);
    let (hover, active) = row_washes(s, p);
    let curve = |seconds| rune::Curve::tween(seconds, rune::Easing::cubic(0.2, 0.0, 0.0, 1.0));
    let mut description = format!("{name}, {}", status.label());
    if !detail.is_empty() {
        description.push_str(&format!(", {detail}"));
    }
    if unread > 0 {
        description.push_str(&format!(", {unread} unread"));
    }
    item.label(description)
        .radius(10.0)
        .fill(if selected { active } else { Color::TRANSPARENT })
        .hover(hover)
        .press(active)
        .hover_motion(curve(0.15), curve(0.2))
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
        // A backing disc in the surface color keeps the mark readable over the artwork.
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
                .radius(backing / 2.0)
                .fill(p.badge_ring)
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
        .child(label(status.label(), SMALL, status_ink(status, p)))
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

/// The captain that gets the larger portrait: the one whose crew is open, otherwise the most
/// urgent one that is working or needs the user. The portrait's slot has the same size either
/// way, so moving the focus never moves a row.
fn focal_captain(s: &State) -> Option<usize> {
    let captains = s.sidebar_captains();
    let open = s.selected_row();
    if let Some(i) = captains.iter().find(|i| s.threads[**i].id == open) {
        return Some(s.threads[*i].id);
    }
    captains
        .iter()
        .map(|i| (s.threads[*i].id, row_status(s, &s.threads[*i]).0))
        .filter(|(_, status)| status.urgency() >= Status::Running.urgency())
        .max_by_key(|(id, status)| (status.urgency(), std::cmp::Reverse(*id)))
        .map(|(id, _)| id)
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
        .key(format!("thread-{}", thread.id))
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!(
            "{}{}, {}",
            thread.title,
            workspace
                .as_ref()
                .map(|w| format!(", {w}"))
                .unwrap_or_default(),
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
        .radius(8.0)
        .fill(if selected {
            p.selected
        } else {
            Color::TRANSPARENT
        })
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

    // The mark only shows what deserves a glance: work in progress, something that needs the
    // user, a failure, an unread result, a draft or a pin. Quiet rows leave it out.
    let dot = |ink: Color| el().size(7.0, 7.0).radius(3.5).fill(ink);
    let mark = if approval || thread.request.is_some() || row_status == Status::Waiting {
        Some(dot(p.gold))
    } else if row_status == Status::Running || row_status == Status::Failed {
        Some(status_mark(row_status, p, 7.0))
    } else if thread.unread() || thread.life.woke(at) {
        Some(dot(p.teal_text))
    } else if draft {
        Some(icon("edit", p.muted))
    } else if thread.pinned() && !nested {
        Some(icon("pin", p.muted))
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
    .min_w(0.0);

    if nested {
        let mut line = row().h(CREW_ROW).px(10.0).gap(8.0).center_y();
        if let Some(mark) = mark {
            line = line.child(mark);
        }
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
                    .radius(6.0)
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
            label("Woke", SMALL, p.teal_text)
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
            p.gold
        } else if thread.status == Status::Failed {
            p.danger
        } else {
            p.teal_text
        };
        // A working row counts up, as T3 Code's status slot does.
        let text = match (state_label, thread.started) {
            ("Working", Some(start)) => {
                format!("Working {}", elapsed_label(start.elapsed().as_secs()))
            }
            _ => state_label.to_string(),
        };
        slot = slot.child(label(text, SIDEBAR_TIMER, ink));
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
            slot = slot.child(label(time, SMALL, p.muted).key(format!("thread-age-{id}")));
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
        let line = lead(row().h(34.0).px(ROW_INSET).gap(8.0).center_y())
            .child(project_tile(s, p, project, 16.0))
            .child(title)
            .child(slot);
        return item.child(line);
    }

    // A thread card, as T3 Code lays it out: the project's icon and name with the status on
    // the right, then the title.
    let top = lead(row().h(18.0).gap(8.0).center_y())
        .child(project_tile(s, p, project, 16.0))
        .child(
            label(
                project
                    .map(|project| project.name.clone())
                    .unwrap_or_else(|| "No workspace".into()),
                CARD_META,
                p.muted,
            )
            .truncate()
            .grow(1.0)
            .min_w(0.0),
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
    let tile = el()
        .size(size, size)
        .radius(4.0)
        .clip_rounded(4.0)
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
        .fill(hue.alpha(if s.dark { 0.24 } else { 0.18 }))
        .child(label(initial, TextStyle::new(size * 0.62, 650.0), hue))
}

/// A row's workspace name, unless the Workspace filter already names that one workspace.
fn row_workspace(s: &State, thread: &Thread) -> Option<String> {
    let named_by_filter = s.scope_additional.is_empty()
        && match s.project_scope {
            ProjectScope::Project(id) => thread.project == Some(id),
            ProjectScope::NoProject => thread.project.is_none(),
            ProjectScope::All => false,
        };
    (!named_by_filter).then(|| {
        s.project(thread.project)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "No workspace".into())
    })
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
        .radius(14.0)
        .on_press(|_: &mut State| {})
        .child(
            col()
                .px(10.0)
                .pt(12.0)
                .pb(10.0)
                .gap(4.0)
                .child(label(
                    "Choose a captain",
                    TextStyle::new(16.0, 620.0).tracking(-0.2),
                    p.ink,
                ))
                .child(
                    row()
                        .gap(6.0)
                        .center_y()
                        .child(label("They will coordinate work in", SMALL, p.muted))
                        .child(project_tile(s, p, target, 14.0))
                        .child(label(workspace, TextStyle::new(12.0, 560.0), p.ink)),
                ),
        );
    let mut list = col().gap(2.0).min_h(0.0).shrink().scroll(ScrollSpec {
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
                .child(label(&profile.name, TextStyle::new(14.0, 600.0), p.ink))
                .child(label(&profile.role, SMALL, p.muted))
                .child(spacer())
                .child(captain_status_chip(state, p)),
        );
        if let Some(brief) = brief {
            text = text.child(label(brief.trim(), SMALL, p.muted).truncate());
        }
        if !homes.is_empty() {
            text = text.child(
                label(
                    format!("Works in {}", homes.join(", ")),
                    SMALL,
                    p.muted.alpha(0.8),
                )
                .truncate(),
            );
        }
        let mut item = row()
            .key(format!("choose-captain-{captain}"))
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("Open Captain {}, {}", profile.name, state.label()))
            .focusable(true)
            .px(10.0)
            .py(10.0)
            .gap(14.0)
            .radius(10.0)
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
            .child(avatar(s, p, captain, 48.0, state))
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

fn captain_editor(s: &State, p: Palette, height: f32) -> El {
    let editor = s.captain_editor.as_ref().unwrap();
    let editing = editor.editing.is_some();
    let profile = editor.profile();
    let field = if s.dark { p.bg } else { p.panel };
    // The portrait grid is ported from KSG Code's `ProjectIconPickerDialog` emoji grid: square
    // cells with `gap-1`, `rounded-md` and a transparent border, the chosen one bordered on an
    // accent fill. Names stay in the accessible label, and the chosen one shows by the heading.
    // Six columns fit the editor's fixed 480 px card, so the grid is the same at every width.
    const COLUMNS: usize = 6;
    let chosen = editor.portrait.min(crate::visuals::PORTRAITS - 1);
    let mut portraits = col().key("captain-portraits").gap(4.0).w_full();
    for first in (0..crate::visuals::PORTRAITS).step_by(COLUMNS) {
        let mut line = row().gap(4.0).w_full();
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
                    .w(0.0)
                    .grow(1.0)
                    .min_w(0.0)
                    .h(64.0)
                    .center()
                    .radius(8.0)
                    .border(
                        if selected { 2.0 } else { 1.0 },
                        if selected { p.teal } else { Color::TRANSPARENT },
                    )
                    .fill(if selected {
                        p.teal_bg
                    } else {
                        Color::TRANSPARENT
                    })
                    .hover(p.selected)
                    .on_press(move |s: &mut State| {
                        s.captain_editor.as_mut().unwrap().portrait = portrait;
                    })
                    .child(portrait_avatar(s, p, portrait, 48.0, Status::Ready)),
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
                .child(label("Workspace", SMALL, p.muted))
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
                        .h(38.0)
                        .px(8.0)
                        .gap(10.0)
                        .center_y()
                        .radius(6.0)
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
                        .child(label("Portrait", SMALL, p.muted))
                        .child(label(crate::visuals::PORTRAIT_NAMES[chosen], SMALL, p.ink)),
                )
                .child(portraits),
        )
        .child(
            col().gap(5.0).child(label("Name", SMALL, p.muted)).child(
                text_input(&editor.name, BODY)
                    .key("captain-name")
                    .label("Captain name")
                    .placeholder("e.g. Atlas", p.muted)
                    .single_line()
                    .color(p.ink)
                    .caret(p.teal, p.teal_bg)
                    .w_full()
                    .padding(8.0)
                    .radius(6.0)
                    .fill(field)
                    .border(1.0, p.field_line)
                    .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().name),
            ),
        )
        .child(
            col()
                .gap(5.0)
                .child(label("Role (optional)", SMALL, p.muted))
                .child(
                    text_input(&editor.role, BODY)
                        .key("captain-role")
                        .label("Captain role")
                        .placeholder("e.g. Code review", p.muted)
                        .single_line()
                        .color(p.ink)
                        .caret(p.teal, p.teal_bg)
                        .w_full()
                        .padding(8.0)
                        .radius(6.0)
                        .fill(field)
                        .border(1.0, p.field_line)
                        .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().role),
                ),
        )
        .child(
            col()
                .gap(5.0)
                .child(label("Instructions for Pi", SMALL, p.muted))
                .child(
                    text_input(&editor.instructions, BODY)
                        .key("captain-instructions")
                        .label("Captain instructions")
                        .placeholder("What should this captain focus on? Optional.", p.muted)
                        .color(p.ink)
                        .caret(p.teal, p.teal_bg)
                        .w_full()
                        .min_h(76.0)
                        .max_h(140.0)
                        .padding(8.0)
                        .radius(6.0)
                        .fill(field)
                        .border(1.0, p.field_line)
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
        .radius(14.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(
            10.0,
            30.0,
            Color::BLACK.alpha(if s.dark { 0.38 } else { 0.14 }),
        )
        .on_press(|_: &mut State| {})
        .child(
            row()
                .center_y()
                .gap(12.0)
                .child(portrait_avatar(s, p, editor.portrait, 40.0, Status::Ready))
                .child(
                    col()
                        .gap(3.0)
                        .child(label(title, TextStyle::new(16.0, 560.0), p.ink))
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
        .child(rule(p))
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    button("cancel-captain", "Cancel", p, |s| s.cancel_captain_flow())
                        .h(32.0)
                        .px(14.0)
                        .border(0.5, p.line),
                )
                .child(spacer())
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
        .child(card)
}

fn header(s: &State, p: Palette, narrow: bool, _: bool) -> El {
    let mut tabs = row()
        .key("thread-tabs")
        .min_w(0.0)
        .grow(1.0)
        .h(36.0)
        .center_y()
        .gap(4.0)
        .scroll_x();
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
        let mut tab = row()
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
            .gap(6.0)
            .px(8.0)
            .h(28.0)
            .radius(6.0)
            .w(if narrow { 210.0 } else { 285.0 })
            .min_w(120.0)
            .fill(if active { p.card } else { Color::TRANSPARENT })
            .hover(p.selected)
            .on_press(move |s: &mut State| s.select_thread(index))
            .child(label(project, SMALL, p.muted).max_w(85.0).truncate())
            .child(label("/", SMALL, p.muted))
            .child(
                label(&thread.title, TextStyle::new(12.0, 520.0), p.ink)
                    .truncate()
                    .grow(1.0),
            );
        if s.tabs.len() > 1 {
            tab = tab.child(icon_button(
                &format!("close-tab-{id}"),
                "Close tab, keep thread running",
                "close",
                p,
                move |s| s.close_tab(id),
            ));
        }
        tabs = tabs.child(tab);
    }
    row()
        .h(36.0)
        .window_drag()
        .center_y()
        .gap(6.0)
        .child(tabs)
        .child(
            icon_button("new-tab", "New thread", "plus", p, |_| {})
                .on_press_tx(|s: &mut State, tx| s.new_thread_click(tx.mods().shift)),
        )
}

fn chat_header(s: &State, p: Palette, _: bool, _: bool) -> El {
    // The tab strip already names the thread, so the header only holds thread actions:
    // status, a labeled "More actions" menu and the session panel toggle.
    let mut header = row()
        .key("chat-header")
        .w_full()
        .h(36.0)
        .px(10.0)
        .center_y()
        .gap(4.0);
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
    if let Some(line) = captain_ui::away_status(s, p) {
        header = header.child(line);
    }
    header = header.child(spacer()).child(if s.demo {
        row()
            .key("demo-badge")
            .h(20.0)
            .px(8.0)
            .mr(4.0)
            .radius(10.0)
            .center_y()
            .fill(p.gold.alpha(0.12))
            .label("Demo data, read-only")
            .child(label("Demo", TextStyle::new(11.0, 560.0), p.gold))
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
        header = header.child(captain_ui::brief_button(s, p));
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
        .label("Thread actions")
        .w(200.0)
        .padding(4.0)
        .gap(1.0)
        .radius(10.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(
            10.0,
            30.0,
            Color::BLACK.alpha(if s.dark { 0.38 } else { 0.14 }),
        )
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
            menu_item(
                "menu-auto-settle-days",
                &format!("Inactivity: {} days, click to change", s.auto_settle_days),
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
        menu_item("menu-delete-thread", "Delete thread...", "close", p)
            .on_press(move |s: &mut State| s.request_delete(id)),
    )
}

fn snooze_menu(s: &State, p: Palette) -> El {
    let (_, choices) = s.snooze_menu.as_ref().unwrap();
    let mut card = col()
        .key("snooze-menu")
        .w(280.0)
        .padding(12.0)
        .gap(4.0)
        .radius(8.0)
        .fill(p.card)
        .border(0.5, p.line)
        .on_press(|_: &mut State| {})
        .child(label("Snooze thread", LABEL, p.ink));
    if s.custom_snooze_open {
        card = card
            .child(label("Local date and time, or duration", SMALL, p.muted))
            .child(
                text_input(&s.snooze_edit, LABEL)
                    .key("snooze-custom-input")
                    .label("Snooze time")
                    .placeholder("2026-11-12 09:00 or 90m, 3h, 2d", p.muted)
                    .single_line()
                    .color(p.ink)
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
                    .child(spacer())
                    .child(label(&choice.time, SMALL, p.muted))
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
    card = card.child(button("snooze-cancel", "Cancel", p, |s| {
        s.snooze_menu = None;
        s.lifecycle_error = None;
    }));
    el().key("snooze-backdrop")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .fill(Color::BLACK.alpha(0.2))
        .modal(false, |s: &mut State| s.snooze_menu = None)
        .child(card)
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
            menu_item(key, &format!("{title} ({count})"), "thread", p).on_press(
                move |s: &mut State| {
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
                },
            ),
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
        .label("Row actions")
        .w(222.0)
        .padding(4.0)
        .gap(1.0)
        .radius(10.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(
            10.0,
            30.0,
            Color::BLACK.alpha(if s.dark { 0.38 } else { 0.14 }),
        )
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
        .radius(10.0)
        .fill(p.card)
        .border(0.5, p.line)
        .on_press(|_: &mut State| {})
        .child(label(title, TextStyle::new(15.0, 560.0), p.ink));
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
            .child(button("cancel-thread-dialog", "Cancel", p, |s| {
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
        .child(card)
}

/// A keyboard hint for a modal's footer: the key in a small cap, then what it does.
fn keycap(keys: &str, action: &str, p: Palette) -> El {
    row()
        .gap(6.0)
        .center_y()
        .child(
            row()
                .h(18.0)
                .px(5.0)
                .center_y()
                .radius(4.0)
                .border(0.5, p.line)
                .fill(p.selected)
                .child(label(keys, TextStyle::new(11.0, 520.0), p.muted)),
        )
        .child(label(action, SMALL, p.muted))
}

/// A floating surface for menus and pickers, styled as T3 Code's popovers: one radius, a
/// hairline edge and a soft shadow that reads in both themes.
fn popover(key: &str, name: &str, s: &State, p: Palette) -> El {
    col()
        .key(key)
        .label(name)
        .padding(4.0)
        .gap(1.0)
        .radius(10.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(
            10.0,
            30.0,
            Color::BLACK.alpha(if s.dark { 0.38 } else { 0.14 }),
        )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerChoice {
    All,
    NoProject,
    Project(usize),
}

/// The picker's rows that match its search, in display order.
fn picker_choices(s: &State, mode: ProjectPicker) -> Vec<(PickerChoice, String)> {
    let query = s.picker_query.text.trim().to_lowercase();
    let mut choices = vec![];
    if mode == ProjectPicker::Scope {
        choices.push((PickerChoice::All, "All workspaces".to_string()));
    }
    if mode != ProjectPicker::CaptainScope {
        choices.push((PickerChoice::NoProject, "No workspace".to_string()));
    }
    for id in s.sorted_projects() {
        let project = s.project(Some(id)).expect("sorted project");
        if !project.hidden {
            choices.push((PickerChoice::Project(id), project.name.clone()));
        }
    }
    choices.retain(|(_, name)| query.is_empty() || name.to_lowercase().contains(&query));
    choices
}

fn choose_picker(s: &mut State, mode: ProjectPicker, choice: PickerChoice, shift: bool) {
    let project = match choice {
        PickerChoice::All => {
            s.project_scope = ProjectScope::All;
            s.scope_additional.clear();
            s.selected_threads.clear();
            s.project_picker = None;
            s.checkpoint();
            return;
        }
        PickerChoice::NoProject => None,
        PickerChoice::Project(id) => Some(id),
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
        .padding(6.0)
        .radius(14.0)
        .on_press(|_: &mut State| {})
        .child(
            row()
                .h(46.0)
                .px(8.0)
                .gap(10.0)
                .center_y()
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
                        .on_edit(|s: &mut State| {
                            s.picker_highlight = 0;
                            &mut s.picker_query
                        })
                        .on_input_key(move |s: &mut State, event, _| {
                            let count = s
                                .project_picker
                                .map_or(0, |mode| picker_choices(s, mode).len());
                            match event.key {
                                rune::Key::Down if count > 0 => {
                                    s.picker_highlight = (s.picker_highlight + 1).min(count - 1);
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
                                            .map(|(choice, _)| *choice)
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
        .child(el().h(0.5).w_full().mb(4.0).fill(p.line));
    let mut list = col().gap(1.0).min_h(0.0).shrink().scroll(ScrollSpec {
        stick_bottom: false,
    });
    for (index, (choice, name)) in choices.iter().enumerate() {
        let (key, project) = match choice {
            PickerChoice::All => ("scope-all-projects".to_string(), None),
            PickerChoice::NoProject => ("choose-no-project".to_string(), None),
            PickerChoice::Project(id) => (format!("choose-project-{id}"), s.project(Some(*id))),
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
                s.project_scope == ProjectScope::Project(*id) || s.scope_additional.contains(id)
            }
            _ => false,
        };
        let choice = *choice;
        let mut item = row()
            .key(key)
            .role("menuitem")
            .cursor(rune::Cursor::Pointer)
            .label(name.as_str())
            .focusable(true)
            .h(38.0)
            .px(8.0)
            .gap(10.0)
            .center_y()
            .radius(8.0)
            .fill(if index == highlight {
                p.selected
            } else {
                Color::TRANSPARENT
            })
            .hover(p.selected)
            .on_press_tx(move |s: &mut State, tx| choose_picker(s, mode, choice, tx.mods().shift))
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
            // Where the workspace lives, so two folders with one name stay distinct.
            .child(
                label(
                    project
                        .map(|project| project.path.display().to_string())
                        .unwrap_or_default(),
                    SMALL,
                    p.muted.alpha(0.8),
                )
                .truncate()
                .grow(1.0)
                .min_w(0.0),
            );
        if checked {
            item = item.child(icon("check", p.teal));
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
        .px(8.0)
        .py(6.0)
        .gap(10.0)
        .center_y()
        .radius(8.0)
        .fill(if lit { p.selected } else { Color::TRANSPARENT })
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
        .px(8.0)
        .gap(10.0)
        .center_y()
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
                    s.picker_highlight = 0;
                    s.projects.menu_error = None;
                    &mut s.picker_query
                })
                .on_input_key(|s: &mut State, event, tx| {
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
                .radius(6.0)
                .border(0.5, p.line)
                .hover(p.selected)
                .on_press(|s: &mut State| s.add_browsed_project())
                .child(label(submit, TextStyle::new(12.5, 560.0), p.ink))
                .child(
                    label(
                        if folders.is_empty() {
                            "Enter"
                        } else {
                            SUBMIT_CHORD
                        },
                        TextStyle::new(11.0, 520.0),
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
        .padding(6.0)
        .radius(14.0)
        .on_press(|_: &mut State| {})
        .child(field)
        .child(el().h(0.5).w_full().mb(4.0).fill(p.line));
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

fn tool_kind(name: &str) -> ToolKind {
    match name.to_ascii_lowercase().as_str() {
        "read" | "read_file" => ToolKind::Read,
        "edit" | "write" | "edit_file" | "write_file" | "apply_patch" => ToolKind::Edit,
        "bash" | "powershell" | "shell" | "exec" | "exec_command" => ToolKind::Run,
        "grep" | "rg" | "search" | "find" => ToolKind::Search,
        "ls" | "list" | "list_directory" => ToolKind::List,
        _ => ToolKind::Other,
    }
}

/// The verb for a call, in the tense of its state: "Reading" while it runs, "Read" after.
fn tool_verb(tool: &Tool) -> String {
    let done = tool.done || tool.failed;
    match (tool_kind(&tool.name), done) {
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
    let count = |kind: ToolKind| tools.iter().filter(|t| tool_kind(&t.name) == kind).count();
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
    (p.muted, p.muted.alpha(0.62))
}

/// A one-line activity row. The verb leads in the secondary ink, its target trails in the
/// tertiary ink, and the whole line truncates. A failure says so in red at the end.
fn activity_line(
    key: String,
    a11y: String,
    verb: String,
    detail: &str,
    failed: bool,
    open: bool,
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
        .radius(6.0)
        .hover(p.selected)
        .child(label(
            verb,
            ACTIVITY,
            if failed { p.danger } else { secondary },
        ));
    if !detail.is_empty() {
        line = line.child(
            label(detail, ACTIVITY, tertiary)
                .truncate()
                .shrink()
                .min_w(0.0),
        );
    }
    if failed {
        line = line.child(label("failed", ACTIVITY, p.danger));
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
                .radius(10.0)
                .fill(p.card)
                .border(0.5, p.line)
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
                                TextStyle::new(11.5, 400.0)
                                    .family(rune::Family::SystemMono)
                                    .lh(17.0),
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
                .pl(12.0)
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
                        p.muted.alpha(0.8),
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
            let streaming = t.status == Status::Running && message + 1 == t.messages.len();
            let style = MarkdownStyle::neutral(s.dark)
                .text(p.ink)
                .muted(p.muted)
                .body_font(prose_style(wide))
                .inline_code_bg(if s.dark {
                    Color::WHITE.alpha(0.07)
                } else {
                    Color::BLACK.alpha(0.055)
                })
                .selection(p.teal_bg)
                .code_block(p.card, p.line)
                .diff(p.teal.alpha(0.12), p.danger.alpha(0.10))
                .code_block_font(
                    TextStyle::new(12.0, 400.0)
                        .family(rune::Family::SystemMono)
                        .lh(18.0),
                );
            memo(
                format!("message-{}-{message}-body-{from}", t.id),
                (chunk, s.dark, streaming, wide),
                || {
                    Markdown::new(chunk)
                        .streaming(streaming)
                        .scroll_wide(true)
                        .render(&style)
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
                .gap(7.0)
                .child(avatar(s, p, captain, 18.0, t.status))
                .child(label(
                    format!("Captain {}", s.captains[captain].name),
                    TextStyle::new(13.0, 600.0),
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
                    // Prose between tool rows keeps its paragraph rhythm.
                    rows = rows.child(if matches!(part, TranscriptPart::Text { .. }) {
                        row.py(6.0)
                    } else {
                        row
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
                // The mark hangs in the lane's gutter so the words line up with the tool
                // rows above; 12 px fits even the narrow 0.75 rem gutter.
                .ml(-12.0)
                .center_y()
                .gap(6.0)
                .child(status_mark(Status::Running, p, 6.0))
                .child(
                    row()
                        .gap(5.0)
                        .center_y()
                        .min_w(0.0)
                        .shrink()
                        .child(
                            label(live_status(t), ACTIVITY, p.muted)
                                .key(format!("live-status-text-{}", t.id))
                                .truncate()
                                .shrink()
                                .min_w(0.0),
                        )
                        // The phase gives way before the timer, whose tabular figures and
                        // padded seconds keep each tick the same width.
                        .child(match seconds {
                            Some(seconds) => label(timer_label(seconds), TIMER, quiet_inks(p).1)
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
            section = section.child(work_rows(
                col()
                    .key(format!("work-rows-{}-{start}", t.id))
                    .pl(14.0)
                    .gap(2.0),
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
        .pt(20.0)
        .pb(24.0)
        .gap(20.0);
    if t.messages.is_empty() && t.tools.is_empty() && !t.status.busy() {
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
                .child(captain_hero(s, p, captain, pane_width));
        } else {
            introduction = introduction
                .items_center()
                .px(0.0)
                .pb(16.0)
                .gap(8.0)
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
                        label(&message.text, body_style(wide), p.ink)
                            .key(format!("{key}-text"))
                            .wrap_anywhere()
                            .selectable(true)
                            .max_w(max_width)
                            .padding(12.0)
                            .radius(16.0)
                            .fill(if s.dark { p.card } else { p.selected }),
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
                    .fill(p.teal_bg)
                    .child(label("Shina", TextStyle::new(13.0, 600.0), p.teal))
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
        .px(gutter)
        .items_center()
        .child(messages)
}

/// The sidebar portraits' animation stage also plays the captain hero, under this row id.
pub(crate) const HERO_PORTRAIT_ROW: usize = usize::MAX;
const HERO_PORTRAIT: f32 = 72.0;

/// An empty captain home greets like KSG Code's draft hero (`DraftHeroHeadline`: centred
/// `text-3xl font-normal tracking-tight`, `text-2xl` on a narrow pane), with the captain's
/// animated portrait above it. A home with a goal shows the goal; otherwise the captain asks
/// what to look after. The live status keeps one 20 px line, so it never moves the composer.
fn captain_hero(s: &State, p: Palette, captain: usize, pane_width: f32) -> El {
    let t = s.current();
    let profile = &s.captains[captain];
    let status = shown_status(s);
    let face = face_for(status);
    let portrait_index = captain_portrait(s, Some(captain));
    let pixels = s
        .portraits
        .pixels(portrait_index, face)
        .unwrap_or_else(|| s.artwork.face(portrait_index, face).pixels());
    let portrait = el()
        .key("captain-hero-portrait")
        .size(HERO_PORTRAIT, HERO_PORTRAIT)
        .radius(20.0)
        .clip_rounded(20.0)
        .border(0.5, p.line)
        .pixels(pixels)
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
                    SMALL.features(TABULAR),
                    status_ink(line_status, p),
                )),
        );
    }
    let role = profile
        .instructions
        .lines()
        .next()
        .filter(|line| !line.trim().is_empty())
        .map_or_else(
            || format!("Captain {} · {}", profile.name, profile.role),
            |line| format!("Captain {} · {line}", profile.name),
        );
    col()
        .key("captain-hero")
        .w_full()
        .items_center()
        .gap(10.0)
        .child(status_badge(p, portrait, HERO_PORTRAIT, status).mb(6.0))
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
            label(role, SMALL.lh(18.0), p.muted)
                .align(rune::Align::Center)
                .wrap()
                .w_full()
                .max_w(520.0),
        )
        .child(status_line)
}

/// The new-thread screen, laid out like KSG Code's draft hero: the composer sits at the
/// vertical centre of the pane and the headline rests 16 px above it. Two equal zero-height
/// boxes do the centring, so attachments or a long draft grow the composer without moving
/// the headline off it.
fn empty_thread(s: &State, p: Palette, narrow: bool, wide: bool, pane_width: f32) -> El {
    let above = col()
        .w_full()
        .h(0.0)
        .grow(1.0)
        .min_h(0.0)
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
    let name = if s.demo {
        "native-workspace".to_string()
    } else {
        s.project_name()
    };
    // Sans at 30 px averages about 15 px a character, and the line stays inside the lane.
    let available = lane_width(pane_width, true);
    let inline = pane_width > 0.0
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
            .press(p.selected)
            .child(
                label(
                    name.clone(),
                    TextStyle {
                        weight: 500.0,
                        ..style
                    },
                    p.ink,
                )
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
            .child(label("What should we build in", HEADING, p.ink).pr(4.0))
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
        .radius((width.min(height) / 6.0).clamp(4.0, 10.0))
        .clip_rounded((width.min(height) / 6.0).clamp(4.0, 10.0))
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
            .radius(8.0)
            .clip_rounded(8.0)
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
                .radius(9.0)
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
        .radius(5.0)
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
        p.gold
    } else {
        p.teal_text
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
        p.gold.alpha(0.65)
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
    let secondary = TextStyle::new(11.0, 400.0).lh(16.0).features(TABULAR);
    let strong = TextStyle::new(11.0, 500.0).lh(16.0).features(TABULAR);
    let mut figures = row().center_y().gap(4.0);
    if let Some(percent) = usage.percent {
        figures = figures
            .child(
                label(context_percent_text(percent), secondary, p.muted)
                    .key("context-details-percent"),
            )
            .child(label("·", secondary, p.muted));
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
        .radius(8.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(
            8.0,
            24.0,
            Color::BLACK.alpha(if s.dark { 0.36 } else { 0.10 }),
        )
        .anchored("context-usage-ring", rune::Side::Top, rune::Align::End, 4.0)
        .on_key(|s: &mut State, event, _| context_escape(s, event))
        .on_hover_change(|s: &mut State, inside, _| s.current_mut().context.hover(false, inside))
        .hover_delays(std::time::Duration::ZERO, CONTEXT_HOVER_DELAY)
        .child(
            row()
                .center_y()
                .gap(12.0)
                .child(label(
                    "Context Window",
                    TextStyle::new(12.0, 500.0).lh(16.0),
                    p.muted,
                ))
                .child(spacer())
                .child(figures),
        );
    if let Some(limit) = limit
        && limit > 0
    {
        let fraction = usage.percent.unwrap_or(0.0).clamp(0.0, 100.0) as f32 / 100.0;
        let ink = if fraction > 0.9 {
            p.danger
        } else {
            p.muted.alpha(0.72)
        };
        let mut bar = row()
            .key("context-details-bar")
            .role("progressbar")
            .label("Context window usage")
            .w_full()
            .h(6.0)
            .radius(3.0)
            .clip_rounded(3.0)
            .fill(p.line);
        if usage.percent.is_some() {
            bar = bar.child(el().w_frac(fraction).h_full().radius(3.0).fill(ink));
        }
        card = card.child(bar);
    }
    let figure_row = |name: &str, value: String| {
        row()
            .center_y()
            .gap(12.0)
            .child(label(name, secondary, p.muted))
            .child(spacer())
            .child(label(value, strong, p.muted))
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
        .radius(6.0)
        .border(1.0, p.line)
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
        .radius(16.0)
        .center()
        .hover(if can_send {
            p.teal.alpha(0.85)
        } else {
            p.selected
        })
        .fill(if can_send {
            p.teal
        } else if s.dark {
            Color::WHITE.alpha(0.10)
        } else {
            Color::BLACK.alpha(0.06)
        })
        .child(icon("send", if can_send { p.on_teal } else { p.muted }))
        .disabled(!can_send);
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
        icon_button("stop-agent", "Stop agent", "stop", p, |s| {
            s.current_mut().stop()
        })
        .size(32.0, 32.0)
        .radius(16.0)
        .border(1.0, p.line)
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
    let mut card = col()
        .key("composer")
        .radius(16.0)
        .fill(p.card)
        .border(
            1.0,
            if s.attachment_drop_hover {
                p.teal
            } else if t.draft.focused {
                p.field_line
            } else {
                p.line
            },
        )
        .shadow(
            1.0,
            2.0,
            Color::BLACK.alpha(if s.dark { 0.0 } else { 0.05 }),
        )
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
    card = card.child(
        text_input(&t.draft, COMPOSER)
            .key("composer-input")
            .label("Message Pi")
            .placeholder(
                if s.demo {
                    "Open a real workspace to send messages"
                } else {
                    "Ask anything. Shift Enter for a new line"
                },
                p.muted,
            )
            .color(p.ink)
            .caret(p.teal, p.teal_bg)
            .w_full()
            .min_h(48.0)
            .max_h(200.0)
            .px(16.0)
            .pt(14.0)
            .pb(6.0)
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
            }),
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
                .radius(8.0),
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
        .min_h(24.0)
        .px(10.0)
        .flex_wrap()
        .center_y()
        .gap(6.0);
    footer = if s.demo {
        footer
            .child(label("Demo data", SMALL, p.muted))
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
                    .radius(5.0)
                    .hover(p.teal_bg)
                    .on_press(|s: &mut State| {
                        s.leave_demo();
                    })
                    .child(label(
                        "Open a real workspace",
                        TextStyle::new(11.5, 500.0),
                        p.teal,
                    )),
            )
    } else {
        let local = t.machine == "local";
        footer
            .child(icon(if local { "monitor" } else { "cloud" }, p.muted))
            .child(
                label(
                    if local {
                        "Local".to_string()
                    } else {
                        t.machine.clone()
                    },
                    SMALL,
                    p.muted,
                )
                .truncate()
                .shrink()
                .min_w(0.0),
            )
            .child(spacer())
            .child(
                button("access-mode", t.native.access.label(), p, |s| {
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
                    .radius(5.0)
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
    let mut details = col()
        .padding(16.0)
        .gap(16.0)
        .child(label(&t.title, TextStyle::new(14.0, 560.0), p.ink).wrap())
        .child(status(t.status, p))
        .child(rule(p))
        .child(
            col()
                .gap(5.0)
                .child(label("Model", SMALL, p.muted))
                .child(label(&t.model, LABEL, p.ink).wrap()),
        )
        .child(
            col()
                .gap(5.0)
                .child(label("Conversation", SMALL, p.muted))
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
        .radius(7.0)
        .fill(p.teal_bg)
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
        Runtime::with_text(
            State::new(".".into(), false),
            view,
            (width, 720.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
        )
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
            for (foreground, background) in [
                (p.ink, p.panel),
                (p.ink, p.bg),
                (p.muted, p.panel),
                (p.muted, p.bg),
                (p.muted, p.card),
                (p.muted, p.bg.mix(p.selected, p.selected.a)),
                (p.muted, p.card.mix(p.selected, p.selected.a)),
                (p.teal_text, p.panel),
                (p.on_teal, p.teal),
            ] {
                let a = foreground.luminance();
                let b = background.luminance();
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(contrast >= 4.5, "text contrast {contrast:.2}, dark={dark}");
            }
        }
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
                assert!(cell[2] >= 60.0 && cell[3] >= 60.0, "portrait {i}: {cell:?}");
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
    fn the_crew_tray_stacks_on_a_narrow_lane() {
        let rt = crew_runtime(1280.0);
        let first = rt.presented("crew-chip-16").unwrap().0;
        let second = rt.presented("crew-chip-17").unwrap().0;
        assert!(
            (first[1] - second[1]).abs() < 1.0,
            "wide lanes keep one row"
        );
        let rt = crew_runtime(560.0);
        let first = rt.presented("crew-chip-16").unwrap().0;
        let second = rt.presented("crew-chip-17").unwrap().0;
        assert!(second[1] > first[1] + 20.0, "narrow lanes stack");
        assert!(
            (first[2] - second[2]).abs() < 1.0,
            "stacked chips share a width"
        );
    }

    #[test]
    fn attention_row_keeps_one_height_and_opens_threads_from_its_popover() {
        for width in [960.0, 1280.0, 1920.0] {
            let mut rt = crew_runtime(width);
            let (row, _) = rt.presented("attention-row").unwrap();
            assert_eq!(row[3], 28.0);
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
