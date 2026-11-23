use crate::attachments::Attachment;
use crate::canvas_layout;
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
use std::time::Duration;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
struct TimelineCounts {
    views: u64,
    keys: u64,
    row_builders: u64,
    built_turns: u64,
    built_slices: u64,
    index_builds: u64,
}

#[cfg(test)]
thread_local! {
    static TIMELINE_COUNTS: std::cell::Cell<TimelineCounts> = Default::default();
}

#[cfg(test)]
fn timeline_count(f: impl FnOnce(&mut TimelineCounts)) {
    TIMELINE_COUNTS.with(|cell| {
        let mut counts = cell.get();
        f(&mut counts);
        cell.set(counts);
    });
}

#[cfg(test)]
fn take_timeline_counts() -> TimelineCounts {
    TIMELINE_COUNTS.with(|cell| cell.replace(TimelineCounts::default()))
}
#[path = "agents_ui.rs"]
mod agents_ui;
#[path = "attachment_archive_preview.rs"]
mod attachment_archive_preview;
#[path = "attachment_notebook_preview.rs"]
mod attachment_notebook_preview;
#[path = "attachment_preview.rs"]
pub(crate) mod attachment_preview;
#[path = "attachment_source_highlight.rs"]
mod attachment_source_highlight;
#[path = "attachment_sqlite_preview.rs"]
mod attachment_sqlite_preview;
#[path = "attachment_table_preview.rs"]
mod attachment_table_preview;
#[path = "attachment_text_preview.rs"]
mod attachment_text_preview;
#[path = "captain_activity_ui.rs"]
mod captain_activity_ui;
#[path = "captain_docs_ui.rs"]
mod captain_docs_ui;
#[path = "captain_lifecycle_ui.rs"]
mod captain_lifecycle_ui;
#[path = "captain_ui.rs"]
mod captain_ui;
#[path = "changed_files.rs"]
pub(crate) mod changed_files;
#[path = "citation_comment.rs"]
pub(crate) mod citation_comment;
#[path = "composer_code_picker.rs"]
pub(crate) mod composer_code_picker;
#[path = "composer_menu.rs"]
pub(crate) mod composer_menu;
#[path = "composer_press.rs"]
pub(crate) mod composer_press;
#[path = "composer_stash.rs"]
pub(crate) mod composer_stash;
#[path = "composer_ui.rs"]
pub(crate) mod composer_ui;
#[path = "context_ui.rs"]
mod context_ui;
#[path = "crew_membership_ui.rs"]
mod crew_membership_ui;
#[path = "crew_questions_ui.rs"]
mod crew_questions_ui;
#[path = "crew_ui.rs"]
pub(crate) mod crew_ui;
#[path = "details_ui.rs"]
mod details_ui;
#[path = "diff_panel.rs"]
pub(crate) mod diff_panel;
#[path = "diff_tree.rs"]
pub(crate) mod diff_tree;
#[path = "icons.rs"]
mod icons;
#[path = "inspector_ui.rs"]
mod inspector_ui;
#[path = "markdown_ui.rs"]
mod markdown_ui;
#[path = "pi_ui.rs"]
mod pi_ui;
#[path = "quick_switcher.rs"]
mod quick_switcher;
#[path = "settings_ui.rs"]
pub(crate) mod settings_ui;
#[path = "sidebar_preview.rs"]
pub(crate) mod sidebar_preview;
#[path = "sidebar_row.rs"]
mod sidebar_row;
#[path = "sidebar_ui.rs"]
pub(crate) mod sidebar_ui;
#[path = "top_bar.rs"]
mod top_bar;
#[path = "transcript_context.rs"]
pub(crate) mod transcript_context;
#[path = "updates_ui.rs"]
mod updates_ui;
#[cfg(test)]
#[path = "wave3_tests.rs"]
mod wave3_tests;
#[path = "work_ui.rs"]
mod work_ui;
use composer_code_picker::CodeLanguageHeaders;
pub(crate) use pi_ui::questionnaire_key;

/// Keyboard navigation uses the same server-scoped order as the unified sidebar.
pub(crate) fn sidebar_navigation(s: &State) -> Vec<(String, usize)> {
    sidebar_ui::navigation(s)
}

pub(crate) fn sidebar_target_key(s: &State, server: &str, id: usize) -> String {
    sidebar_ui::target_key(s, server, id)
}

// KSG Code's default theme (`apps/web/src/index.css`, the dark and light palettes after the
// 2026 redesign): calm neutral chrome, a slightly lighter chat pane, raised cards for the
// composer and menus, hairline borders and one soft blue for links, focus and running work.
// The field names are older than the theme: `coral` is the primary key (KSG's light `surface-lit`
// button in dark, near-black in light), `sea` is KSG's info blue and `brass` its warning.
#[derive(Clone, Copy)]
struct Palette {
    bg: Color,
    panel: Color,
    card: Color,
    lift: Color,
    /// KSG's `surface-tray`: the composer shell around the card and the context strip.
    tray: Color,
    /// KSG's `--pill-fill`: the recessed send pill inside the composer card, a step below
    /// the card's gradient in both themes.
    pill: Color,
    line: Color,
    /// KSG's `--strong-foreground`: the selected sidebar row's title and other active labels.
    strong: Color,
    /// KSG's receding sidebar thread title: `--sidebar-thread-title-foreground` at 75% over
    /// the chrome, premixed so it does not pick up Rune's linear-light blend.
    recede: Color,
    /// KSG's `--foreground`: prose, titles and values.
    ink: Color,
    /// KSG's `--muted-foreground` (`--secondary-label`): tool rows, labels and meta text.
    muted: Color,
    /// KSG's `--placeholder`, the step between its muted and dim text: carets, timestamps,
    /// counts and other text that can recede. It reads 3:1 or better on every surface, not
    /// 4.5:1, so nothing a reader needs is drawn in it.
    faint: Color,
    /// Hover tint over any surface.
    selected: Color,
    /// Fill of the selected sidebar row.
    row: Color,
    coral: Color,
    coral_bg: Color,
    on_coral: Color,
    brass: Color,
    brass_bg: Color,
    on_brass: Color,
    sea: Color,
    sea_bg: Color,
    /// KSG's `--sidebar-status-input`: a thread waiting on user input.
    input: Color,
    danger: Color,
    danger_bg: Color,
    ok: Color,
    ok_bg: Color,
    /// A control's 1 px border.
    frame: Color,
    /// The outline of portraits and project tiles.
    portrait_frame: Color,
    /// The colour of the soft drop shadows under cards, menus and dialogs.
    shadow: Color,
    field_line: Color,
    /// The focus ring and a focused field's border.
    ring: Color,
    message_bg: Color,
    code_bg: Color,
    /// KSG's `--composer-fill` gradient ends: the composer card runs `card_top`
    /// into `card_bottom`.
    card_top: Color,
    card_bottom: Color,
}
fn color(hex: &str) -> Color {
    rune::color::hex(hex).expect("palette color")
}
fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: color("#000000"),
            panel: color("#151515"),
            card: color("#1a1a1a"),
            lift: color("#262626"),
            tray: color("#191919"),
            pill: color("#0e0e0e"),
            line: color("#242424"),
            strong: color("#f0f0f0"),
            recede: color("#b5b6b9"),
            ink: color("#d4d4d4"),
            muted: color("#8a8a8a"),
            faint: color("#777777"),
            // Rune blends in linear light, so a small alpha still gives a visible highlight.
            selected: color("#ffffff").alpha(0.006),
            row: color("#1e1e1e"),
            coral: color("#eceef2"),
            coral_bg: color("#8aa4ff").alpha(0.16),
            on_coral: color("#111215"),
            brass: color("#d9a02a"),
            brass_bg: color("#2a2213"),
            on_brass: color("#111215"),
            sea: color("#8aa4ff"),
            sea_bg: color("#1d2338"),
            input: color("#a5b4fc"),
            danger: color("#ff6b63"),
            danger_bg: color("#2e1a1b"),
            ok: color("#4cc35f"),
            ok_bg: color("#16281b"),
            frame: color("#2c2c2c"),
            portrait_frame: color("#2c2c2c"),
            shadow: color("#000000").alpha(0.45),
            field_line: color("#707070"),
            ring: color("#8aa4ff"),
            message_bg: color("#222222"),
            code_bg: color("#131313"),
            card_top: color("#202020"),
            card_bottom: color("#1c1c1c"),
        }
    } else {
        Palette {
            bg: color("#ebebeb"),
            panel: color("#f5f5f5"),
            card: color("#ffffff"),
            lift: color("#ececec"),
            tray: color("#fafafa"),
            pill: color("#efefef"),
            line: color("#e6e6e6"),
            strong: color("#0f0f0f"),
            recede: color("#4d4d4f"),
            ink: color("#252525"),
            muted: color("#636363"),
            faint: color("#747474"),
            selected: color("#000000").alpha(0.045),
            row: color("#ffffff"),
            coral: color("#141518"),
            coral_bg: color("#1d4ed8").alpha(0.10),
            on_coral: color("#f7f7f8"),
            brass: color("#7d5200"),
            brass_bg: color("#f6ecd6"),
            on_brass: color("#ffffff"),
            sea: color("#1d4ed8"),
            sea_bg: color("#e3e9fb"),
            input: color("#4f46e5"),
            danger: color("#b42318"),
            danger_bg: color("#fbe7e4"),
            ok: color("#176f31"),
            ok_bg: color("#ddf3e3"),
            frame: color("#e1e1e1"),
            portrait_frame: color("#e3e3e3"),
            shadow: color("#000000").alpha(0.10),
            field_line: color("#7d7d7d"),
            ring: color("#1d4ed8"),
            message_bg: color("#ececec"),
            code_bg: color("#f0f0f0"),
            card_top: color("#ffffff"),
            card_bottom: color("#f7f7f7"),
        }
    }
}

/// KSG's htmlRenderTheme variables, mapped from the same palette as Rune's UI.
pub(crate) fn html_theme(dark: bool) -> serde_json::Value {
    let p = palette(dark);
    let css = |color: Color| {
        let [r, g, b, a] = color.premul_srgb(1.0);
        if a < 0.999 {
            format!(
                "rgba({},{},{},{a})",
                (r / a * 255.0).round() as u8,
                (g / a * 255.0).round() as u8,
                (b / a * 255.0).round() as u8
            )
        } else {
            format!(
                "#{:02x}{:02x}{:02x}",
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8
            )
        }
    };
    let mut variables = serde_json::Map::new();
    for (name, color) in [
        ("background", p.panel),
        ("foreground", p.ink),
        ("muted", p.selected),
        ("muted-foreground", p.muted),
        ("card", p.card),
        ("card-foreground", p.ink),
        ("popover", p.card),
        ("popover-foreground", p.ink),
        ("secondary", p.sea_bg),
        ("secondary-foreground", p.ink),
        ("border", p.frame),
        ("input", p.field_line),
        ("ring", p.ring),
        ("primary", p.coral),
        ("primary-foreground", p.on_coral),
        ("accent", p.coral),
        ("accent-foreground", p.on_coral),
        ("accent-surface", p.coral_bg),
        ("accent-surface-foreground", p.ink),
        ("destructive", p.danger),
        ("destructive-foreground", p.danger),
        ("destructive-surface", p.danger_bg),
        ("warning", p.brass),
        ("warning-foreground", p.brass),
        ("warning-surface", p.brass_bg),
        ("success", p.ok),
        ("success-foreground", p.ok),
        ("info", p.sea),
        ("info-foreground", p.sea),
        ("code-background", p.code_bg),
        ("code-foreground", p.ink),
        ("chart-1", p.sea),
        ("chart-2", p.coral),
        ("chart-3", p.brass),
        ("chart-4", p.danger),
        ("chart-5", p.muted),
        ("chart-6", p.faint),
    ] {
        variables.insert(format!("--{name}"), css(color).into());
    }
    variables.insert("--radius".into(), "8px".into());
    variables.insert(
        "--font-sans".into(),
        r#"Inter,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif"#.into(),
    );
    variables.insert(
        "--font-mono".into(),
        r#""SF Mono",Menlo,Consolas,monospace"#.into(),
    );
    serde_json::json!({"appearance":if dark {"dark"} else {"light"},"variables":variables})
}
/// Every piece of UI text uses this one stack, as KSG Code's `--font-sans` does. The user's KSG
/// Appearance setting names Anthropic Sans, so an installed copy comes first. The font lists
/// its family twice ("Anthropic Sans Variable Text" in name ID 1 and "Anthropic Sans Variable"
/// in ID 16), and Rune registers the file under the second, so both names are here. Without the
/// font installed, text falls back to the bundled Inter (KSG's default). Tests lay out in the
/// bundled Inter alone, so a measurement is the same on every machine.
#[cfg(not(test))]
pub const SANS_FAMILY: &str =
    "Anthropic Sans Variable Text, Anthropic Sans Variable, Inter Variable";
#[cfg(test)]
pub const SANS_FAMILY: &str = "Inter Variable";
/// Code, inline code, keycaps and diff gutters: Anthropic Mono when installed (the user's KSG
/// code font), else the system's.
#[cfg(not(test))]
pub const MONO_FAMILY: &str = "Anthropic Mono Web, Cascadia Mono, Consolas";
#[cfg(test)]
pub const MONO_FAMILY: &str = "Cascadia Mono, Consolas";
pub const FONTS: &[&[u8]] = &[include_bytes!("../assets/fonts/InterVariable.ttf")];
/// KSG Code's small UI text: `size` at weight 500, for section labels, counts, timers,
/// status words, badges and the controls under the composer. The wordmark uses it at 15.
const fn meta(size: f32) -> TextStyle {
    TextStyle::new(size, 500.0).lh(size + 5.0)
}
/// Section labels, counts and status words.
const META: TextStyle = meta(12.0);
/// Compact 12/500 meta rows in Brief, Context, and crew cards (16 pt line height).
const META_COMPACT: TextStyle = TextStyle::new(12.0, 500.0).lh(16.0);
/// Section labels: KSG's sidebar group headings, sentence case with a little air.
const META_LABEL: TextStyle = meta(12.0).tracking(0.1);
/// Inter 13/500, thread titles in the bar and highlighted sidebar rows.
const TITLE: TextStyle = TextStyle::new(13.0, 500.0).lh(19.0).tracking(-0.08);
/// Inter 13/400, quiet read sidebar rows and breadcrumb workspace names.
const ROW_QUIET: TextStyle = TextStyle::new(13.0, 400.0).lh(19.0).tracking(-0.08);
const ROW_STRONG: TextStyle = TextStyle::new(13.0, 600.0).lh(19.0).tracking(-0.08);
const CARD_TITLE: TextStyle = TextStyle::new(14.0, 600.0).lh(20.0).tracking(-0.10);
const MONO_META: TextStyle = TextStyle::new(12.0, 400.0)
    .lh(17.0)
    .family(rune::Family::SystemMono);
const BODY: TextStyle = TextStyle::new(14.0, 400.0).lh(21.0).tracking(-0.10);
/// Assistant answers use KSG Code's `.chat-markdown`: `text-sm leading-relaxed`, 14/22.5.
const PROSE: TextStyle = TextStyle::new(14.0, 400.0).lh(22.5).tracking(-0.10);
/// The composer's text, as KSG Code's `ComposerPromptEditorTiptap` sets it: `text-sm` on
/// `.composer-text`'s 1.25 rem lines.
const COMPOSER: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0).tracking(-0.10);
/// The new-thread greeting, ported from KSG Code's `DraftHeroHeadline`: centred Inter,
/// `text-3xl font-medium tracking-tight`, or `text-2xl` when the pane is narrow.
const HEADING: TextStyle = TextStyle::new(30.0, 500.0).lh(36.0).tracking(-0.75);
const DIALOG_TITLE: TextStyle = IDENTITY;
/// The captain's name in the Inspector's Overview (spec §0.1 `IDENTITY`): Inter 16/600,
/// tight tracking. Replaces the old Source Serif treatment, per the all-Inter rule.
const IDENTITY: TextStyle = TextStyle::new(16.0, 600.0).lh(22.0).tracking(-0.16);
const HEADING_NARROW: TextStyle = TextStyle::new(24.0, 500.0).lh(32.0).tracking(-0.6);
/// A chat pane this wide (a 1600 px window with the sidebar open) gets a longer measure and
/// larger body text. It follows the pane, not the window, so an open session panel or a wide
/// sidebar keeps the reading size that fits the lane.
const WIDE_PANE: f32 = 1200.0;
/// KSG Code keeps one text size at every pane width, so a wide pane no longer steps up to 15.
fn body_style(_wide: bool) -> TextStyle {
    BODY
}
fn prose_style(_wide: bool) -> TextStyle {
    PROSE
}
/// The chat lane's content width, ported from KSG Code's `.chat-content-lane`
/// (`max-width: var(--chat-content-max-width)`, 45 rem at every pane width). Messages, tool
/// rows, the queue and the composer all share it.
fn measure(_wide: bool) -> f32 {
    720.0
}
/// The lane's side padding, from KSG Code's `.chat-composer-lane` and
/// `.messages-timeline-scroll`: 0.75 rem below `sm` (640 px) and 3 rem from `sm` up. KSG reads
/// the viewport; Shina reads the chat pane, which is what the lane actually has to share.
fn lane_gutter(pane_width: f32) -> f32 {
    if pane_width >= 640.0 { 48.0 } else { 12.0 }
}
/// The chat lane the conversation and the composer share, resolved through KSG Code's
/// `resolveChatCanvasLayout` (`chatCanvasLayout.ts`): centred at `measure` until an inline
/// Details card docks beside it, when `inset_start`/`inset_end` carry the shift (KSG's
/// `--chat-lane-inset-start`/`--chat-lane-inset-end`). `details_card` stays `None` until C5
/// mounts the card; the resolver is the single source for both lanes already.
fn chat_lane(
    pane_width: f32,
    wide: bool,
    details_card: Option<canvas_layout::DetailsCardEdges>,
) -> canvas_layout::ChatLane {
    canvas_layout::resolve_chat_canvas_layout(canvas_layout::ChatCanvasInput {
        container_width: pane_width,
        padding: lane_gutter(pane_width),
        max_chat_width: measure(wide),
        details_card,
    })
    .chat
}
/// How wide the lane is in a pane of `pane_width`.
fn lane_width(pane_width: f32, wide: bool) -> f32 {
    chat_lane(pane_width, wide, None).width
}
/// Live timers and counts use tabular figures, so "12m 05s" never shifts as a digit changes.
const TABULAR: &str = "'tnum' 1";
const TIMER: TextStyle = META.features(TABULAR);
const LABEL: TextStyle = TextStyle::new(14.0, 400.0).tracking(-0.10);
const SMALL: TextStyle = TextStyle::new(12.0, 400.0);
/// The project line of a sidebar card.
/// Sidebar rows sit 8 px in from the sidebar's edge, and their content 12 px inside that, so
/// icons and text share one left edge with the title strip and footer.
const ROW_INSET: f32 = 12.0;
/// Tool, thinking and status lines in the transcript.
const ACTIVITY: TextStyle = TextStyle::new(13.0, 400.0).tracking(-0.08);
/// A text field's focus look, shared by every field: KSG's 1 px `--input` border that turns the
/// `--ring` blue while the field has focus. Put it on the box that draws the field's edge.
trait FieldFocus {
    fn field_focus(self, edit: &rune::TextEdit, p: Palette) -> Self;
}
impl FieldFocus for El {
    fn field_focus(self, edit: &rune::TextEdit, p: Palette) -> Self {
        self.caret(p.ink, p.coral_bg)
            .border(1.0, if edit.focused { p.ring } else { p.field_line })
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
    el().h(1.0).w_full().fill(p.line).hairline()
}
/// KSG Code's soft control shadow (`shadow-xs`): one point down, a few points of blur.
fn control_shadow(p: Palette) -> rune::Shadow {
    rune::Shadow {
        x: 0.0,
        y: 1.0,
        blur: 3.0,
        spread: 0.0,
        color: p.shadow.alpha(p.shadow.a * 0.6),
        inset: false,
    }
}
/// KSG Code's popover, menu and dialog shadow (`shadow-lg`): deeper and diffuse.
fn soft_shadow(p: Palette) -> rune::Shadow {
    rune::Shadow {
        x: 0.0,
        y: 8.0,
        blur: 24.0,
        spread: 0.0,
        color: p.shadow,
        inset: false,
    }
}
fn dialog_shadow(p: Palette) -> rune::Shadow {
    rune::Shadow {
        y: 16.0,
        blur: 48.0,
        ..soft_shadow(p)
    }
}

const MOTION_FAST: rune::Curve = rune::Curve::tween(0.12, rune::Easing::cubic(0.2, 0.0, 0.0, 1.0));
const MOTION_BASE: rune::Curve = rune::Curve::tween(0.18, rune::Easing::cubic(0.2, 0.0, 0.0, 1.0));
const MOTION_SLOW: rune::Curve =
    rune::Curve::tween(0.30, rune::Easing::cubic(0.23, 1.0, 0.32, 1.0));
/// A raised surface you act on: KSG's 1 px `--border` and the soft control shadow.
fn framed(el: El, p: Palette) -> El {
    el.border(1.0, p.frame).box_shadow(control_shadow(p))
}
/// A surface that only informs (a keycap, a count, a badge): the 1 px border, no shadow.
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
/// A portrait with rounded corners and a 1 px hairline, as KSG's avatars. The image sits under
/// a border overlay, because Rune paints a clipped image over its own border.
fn pixel_portrait(s: &State, pixels: &rune::Pixels, size: f32, p: Palette) -> El {
    pixel_portrait_framed(s, pixels, size, p, None)
}
/// `pixel_portrait` with an accent ring for a selected portrait. The ring keeps the 1 px width,
/// so the outer bounds do not change.
fn pixel_portrait_framed(
    s: &State,
    pixels: &rune::Pixels,
    size: f32,
    p: Palette,
    accent: Option<Color>,
) -> El {
    portrait(pixels, s.artwork.snap_points(size), p, accent)
}

fn portrait_radius(size: f32) -> f32 {
    (size * 0.25).round().clamp(5.0, 20.0)
}

fn captain_wash(dark: bool, slot: usize, width: f32, height: f32) -> El {
    const SIGNALS: [(&str, &str); 12] = [
        ("#e0793a", "#b4541a"),
        ("#45c49b", "#167a5b"),
        ("#8b84f0", "#5148c9"),
        ("#d9b23a", "#8a6a00"),
        ("#5b8fd6", "#2c5fae"),
        ("#b97ad0", "#8a3fa6"),
        ("#93a84c", "#5f7320"),
        ("#f0685c", "#c2392e"),
        ("#ee5fa0", "#c22a74"),
        ("#a3acba", "#5d6675"),
        ("#38bdf8", "#0b77ad"),
        ("#5fbf62", "#2b7f2f"),
    ];
    let (night, day) = SIGNALS[slot.min(SIGNALS.len() - 1)];
    let signal = color(if dark { night } else { day });
    el().size(width, height).a11y_hidden().radial_gradient(
        [0.5, 0.5],
        [width / 2.0, height / 2.0],
        [
            (0.0, signal.alpha(if dark { 0.10 } else { 0.08 })),
            (1.0, signal.alpha(0.0)),
        ],
    )
}

fn portrait(pixels: &rune::Pixels, size: f32, p: Palette, accent: Option<Color>) -> El {
    let radius = portrait_radius(size);
    let edge = accent.unwrap_or(p.portrait_frame);
    el().size(size, size)
        .radius(radius)
        .child(
            el().size(size, size)
                .radius(radius)
                .clip_rounded(radius)
                .pixels_fit(pixels, rune::SurfaceFit::Fill),
        )
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(radius)
                .border(if accent.is_some() { 2.0 } else { 1.0 }, edge),
        )
}

fn captain_face(
    s: &State,
    p: Palette,
    captain: usize,
    face: crate::visuals::Face,
    size: f32,
) -> El {
    let pixels = s.artwork.face_pixels(captain, face, size);
    if s.artwork.is_pixel_art(captain) {
        pixel_portrait(s, &pixels, size, p)
    } else {
        portrait(&pixels, size, p, None)
    }
}

fn haloed(s: &State, p: Palette, image: El, size: f32, status: Status) -> El {
    if size < 30.0 {
        return status_badge(p, image, size, status);
    }
    if matches!(status, Status::Running | Status::Waiting | Status::Failed) {
        let mut frame = el()
            .size(size, size)
            .radius(portrait_radius(size))
            .outline(1.5, 2.0, status_ink(status, p))
            .child(image);
        if status == Status::Running && s.portraits.motion_allowed() {
            frame = frame.child(halo_sweep(size, p));
        }
        frame
    } else {
        image
    }
}

// Native opacity cycles share a clock, so the highlight travels around the rounded
// perimeter without a view rebuild or one timer per portrait.
fn halo_sweep(size: f32, p: Palette) -> El {
    const SEGMENTS: usize = 12;
    let span = size + 5.5;
    let radius = portrait_radius(size) + 2.75;
    let edge = span - radius * 2.0;
    let arc = radius * std::f32::consts::FRAC_PI_2;
    let quarter = edge + arc;
    let point = |position: f32| {
        let distance = position.rem_euclid(1.0) * quarter * 4.0;
        let corner = (distance / quarter).floor() as usize;
        let distance = distance % quarter;
        let [x, y] = if distance < edge {
            [radius + distance, 0.0]
        } else {
            let angle = (distance - edge) / radius;
            [
                span - radius + radius * angle.sin(),
                radius - radius * angle.cos(),
            ]
        };
        match corner {
            0 => [x, y],
            1 => [span - y, x],
            2 => [span - x, span - y],
            _ => [y, span - x],
        }
    };
    let mut sweep = el().absolute(-2.75, -2.75).size(span, span).a11y_hidden();
    for i in 0..SEGMENTS {
        let start = i as f32 / SEGMENTS as f32;
        let points: Vec<_> = (0..=4)
            .map(|j| point(start + j as f32 / (SEGMENTS as f32 * 4.0)))
            .collect();
        sweep = sweep.child(
            el().absolute(0.0, 0.0)
                .size(span, span)
                .shape(rune::Shape::new().stroke(points, 1.5, p.sea.mix(p.ink, 0.72)))
                .cycle(rune::Cycle {
                    period: 1.6,
                    phase: start,
                    stops: &[(0.0, 0.0), (0.09, 1.0), (0.23, 0.0), (1.0, 0.0)],
                    still: 0.0,
                    fps: 30.0,
                }),
        );
    }
    sweep
}
/// A section label: Inter 12/500 in sentence case, as KSG's sidebar group headings.
fn meta_label(value: &str, ink: Color) -> El {
    let mut chars = value.chars();
    let value = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars.as_str().chars()).collect(),
        None => String::new(),
    };
    label(value, META_LABEL, ink)
}
/// Inter 12/500 as written, for counts, ages and timers.
fn meta_text(value: impl Into<String>, ink: Color) -> El {
    label(value, META, ink)
}
fn button(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    button_in(key, title, TextStyle::new(12.0, 500.0), p, action)
}
/// A quiet button with a 12 px Inter label, for the rows of state under the composer.
fn meta_button(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    button_in(key, title, META, p, action)
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
        .px(8.0)
        .h(24.0)
        .radius(12.0)
        .hover(p.selected)
        .press(p.selected)
        .on_press(action)
        .child(label(title, style, p.ink).truncate().shrink())
}
/// Icon size in points: KSG's `size-4` Lucide icons.
const ICON: f32 = 16.0;
/// A Lucide outline glyph from `icons`.
fn icon(kind: &str, ink: Color) -> El {
    rune::icon(icons::glyph(kind), ICON)
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
            .radius(8.0)
            .fill(p.coral)
            .press(p.coral.alpha(0.9))
            .on_press(action)
            .child(label(title, TextStyle::new(13.0, 600.0), p.on_coral)),
        p,
    )
}
/// The secondary action beside a primary one, such as Cancel: the same size with a 1 px border
/// on the card fill, as KSG's `outline` button.
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
        .radius(8.0)
        .fill(p.card)
        .border(1.0, p.frame)
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
        .radius(6.0)
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
    let active_fill = if p.panel.luminance() < 0.5 {
        p.lift
    } else {
        p.card
    };
    let mut button = el()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .checked(on)
        .focusable(true)
        .size(24.0, 24.0)
        .center()
        .radius(6.0)
        .fill(if on { active_fill } else { Color::TRANSPARENT })
        .hover(p.selected)
        .on_press(action)
        .child(tool_icon(glyph, if on { p.ink } else { p.muted }));
    if on {
        button = button.border(1.0, p.field_line);
    }
    button
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
        Status::Running => p.sea,
        Status::Completed => p.ok,
        _ => p.muted,
    }
}
/// Status uses shape as well as colour, so it also reads in monochrome.
fn status_mark(status: Status, p: Palette, size: f32) -> El {
    let ink = status_ink(status, p);
    let dot = |side: f32| el().size(side, side).radius(side / 2.0);
    match status {
        Status::Running => dot(size).fill(ink),
        Status::Waiting => dot(size).border(1.5, ink),
        Status::Failed => el()
            .size(6.0_f32.min(size), 6.0_f32.min(size))
            .radius(1.0)
            .rotate(std::f32::consts::FRAC_PI_4)
            .fill(ink),
        Status::Completed => icon("check", ink).size(12.0, 12.0),
        Status::Ready => el(),
        _ => dot(size).border(1.0, p.faint),
    }
    .a11y_hidden()
}
/// A status slot in the top bar, intrinsic width. Fades between states on
/// Shina's motion curve; the live elapsed timer lives in the timeline's working bar.
fn status_slot(key: &str, status: Status, p: Palette) -> El {
    let mut slot = row().key(key).h(20.0).shrink().center_y();
    if matches!(status, Status::Ready | Status::Completed) {
        return slot;
    }
    let text = status.label();
    slot = slot.label(text).child(
        row()
            .key(format!("{key}-{text}"))
            .center_y()
            .gap(6.0)
            .opacity_motion(MOTION_SLOW)
            .child(status_mark(status, p, 6.0))
            .child(label(text, META, status_ink(status, p)).key(format!("{key}-text"))),
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

#[path = "ui/approval_dialog.rs"]
mod approval_dialog;
#[path = "ui/assistant_turn.rs"]
mod assistant_turn;
#[path = "ui/attachments_ui.rs"]
mod attachments_ui;
#[path = "ui/captain_picker_ui.rs"]
mod captain_picker_ui;
#[path = "ui/chrome.rs"]
mod chrome;
#[path = "ui/composer_view.rs"]
mod composer_view;
#[path = "ui/context_and_notices.rs"]
mod context_and_notices;
#[path = "ui/effort_slider.rs"]
mod effort_slider;
#[path = "ui/empty_hero.rs"]
mod empty_hero;
#[path = "ui/environment_pickers.rs"]
mod environment_pickers;
#[path = "ui/model_picker_ui.rs"]
mod model_picker_ui;
#[path = "ui/project_picker_ui.rs"]
mod project_picker_ui;
#[path = "ui/root_view.rs"]
mod root_view;
#[path = "ui/tabs_and_menus.rs"]
mod tabs_and_menus;
#[path = "ui/timeline.rs"]
mod timeline;
#[path = "ui/work_rows.rs"]
mod work_rows;

use approval_dialog::*;
use assistant_turn::*;
use attachments_ui::*;
use captain_picker_ui::*;
use chrome::*;
use composer_view::*;
use context_and_notices::*;
use effort_slider::*;
use empty_hero::*;
use environment_pickers::*;
use model_picker_ui::*;
use project_picker_ui::*;
use tabs_and_menus::*;
use timeline::*;
use work_rows::*;

pub(crate) use approval_dialog::long_turn_fixture;
pub(crate) use attachments_ui::tray_attachments;
pub(crate) use chrome::tick_portraits;
pub(crate) use empty_hero::HERO_PORTRAIT_ROW;
pub(crate) use environment_pickers::{
    cycle_model_picker_harness, jump_model_picker_index, step_reasoning_effort,
};
pub use root_view::view;
pub(crate) use timeline::{
    TimelineCache, restore_timeline_position, timeline_text_row, timeline_tool_row,
};

#[cfg(test)]
#[path = "ui/tests.rs"]
pub(crate) mod tests;
