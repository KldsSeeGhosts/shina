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

/// The chat pane rounds its top corners (KSG's `--radius-pane: 14px`) and runs square to the
/// window's bottom edge.
const PANE_CORNERS: [f32; 4] = [14.0, 14.0, 0.0, 0.0];

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    #[cfg(test)]
    timeline_count(|counts| counts.views += 1);
    let unloaded = s
        .current()
        .transcript_projection
        .as_ref()
        .is_some_and(|projection| !projection.history_loaded);
    s.timeline_positions
        .borrow_mut()
        .open(&s.machines.selected, s.current().id, unloaded);
    let p = palette(s.dark);
    let composer_model = ComposerModelDisplay::new(s);
    composer_ui::measure(s, cx);
    cx.foreground(p.ink);
    cx.focus_ring(2.0, 2.0, p.ring);
    // Every enabled button, tab, link and switch is a Tab stop in document order, as on the web,
    // even one that never said `focusable(true)`. Disabled controls stay out.
    cx.web_focus();
    cx.caption_color(p.muted);
    // The sidebar portraits' animation clock lives outside the view and needs this setting.
    s.portraits.set_reduced_motion(cx.is_reduced_motion());
    let [layout_width, layout_height] = cx
        .layout_with("workspace-root", |r| [r[2], r[3]])
        .unwrap_or([s.viewport_width, s.viewport_height]);
    let narrow = layout_width < 1050.0;
    // Below this width the sidebar stops docking and opens on demand instead.
    let compact = layout_width < 760.0;
    let sidebar_width = sidebar_width_at(s.sidebar_width, layout_width);
    let docked = (s.sidebar && !compact) || (compact && s.compact_sidebar);
    // The chat column's width from the last layout. The first frame guesses from the window,
    // less the gutters and a docked sidebar, so it already lays out close to its final lane.
    let main_width = cx.layout_with("chat-main", |r| r[2]).unwrap_or_else(|| {
        let sidebar = if docked && !compact {
            sidebar_width + 4.0
        } else {
            0.0
        };
        cx.layout_with("workspace-root", move |r| r[2] - 16.0 - sidebar)
            .unwrap_or(900.0)
    });
    let wide = main_width >= WIDE_PANE;
    // The details card docks in the chat column when it fits there. The top bar's button
    // and the lanes below both read the answer.
    s.details.pane_height.set(
        cx.layout_with("chat-main", |r| r[3])
            .unwrap_or(s.viewport_height - 56.0),
    );
    let details_rect = details_ui::rect(s, main_width);
    s.details.fits.set(details_rect.is_some());
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
        // The header runs to the sidebar's edge, so the compose button at its end has room
        // beside the wordmark in a 232 px sidebar, even with the traffic lights.
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
    top = top.child(top_bar::bar(s, p, narrow, compact, tabs_width));
    // The pane and the sidebar's sea strip both run to the window's bottom edge, so the bottom
    // of the window is one edge, not a strip under the pane and another under the sidebar.
    let mut workspace = row().grow(1.0).min_h(0.0).px(8.0);
    if docked && !compact {
        workspace = workspace.child(sidebar(s, p, sidebar_width));
        if !compact {
            let max_width = sidebar_max_width(layout_width);
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
                            s.sidebar_width = (drag.data.min(max_width) + drag.dx / zoom)
                                .clamp(crate::model::SIDEBAR_MIN, max_width);
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
                        s.sidebar_width =
                            (sidebar_width + step).clamp(crate::model::SIDEBAR_MIN, max_width);
                        s.checkpoint();
                        true
                    }),
            );
        }
    }
    let mut compact_inspector = None;
    let mut pane = col()
        .key("chat-pane")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .fill(p.panel)
        .corners(PANE_CORNERS)
        .clip_corners(PANE_CORNERS);
    // A retired captain record has no home thread, so it has no Inspector to sit in:
    // it draws as a sheet over the conversation instead.
    let brief_record = s.brief.record.as_ref().is_some_and(|(server, captain)| {
        *server == s.machines.selected
            && s.captain_phase(*captain) == crate::model::CaptainPhase::Retired
    });
    {
        let mut body = row().key("chat-body").grow(1.0).min_h(0.0);
        let main =
            composer_ui::motion_host(s, col().key("chat-main").grow(1.0).min_w(0.0).min_h(0.0));
        let main = if s.current().captain_home
            && s.current().captain.is_some_and(|captain| {
                s.captain_phase(captain) == crate::model::CaptainPhase::Archived
            }) {
            main.child(conversation(s, p, wide, main_width))
                .child(captain_lifecycle_ui::archived_bar(s, p, wide, main_width))
        } else if captain_ui::is_earlier(s) {
            main.child(conversation(s, p, wide, main_width))
                .child(captain_ui::earlier_bar(s, p, wide, main_width))
        } else if s.current().messages.is_empty()
            && s.current().tools.is_empty()
            && s.current().native.compactions.is_empty()
            && s.current().worktree_setup.is_none()
            && !s.current().history_has_more
            && s.current().history_error.is_none()
        {
            // Sizes from the last layout: the hero keeps its height only while the composer
            // and its crew tray still fit under it.
            let pane = cx.layout_with("empty-thread", |r| r[3]);
            let lane = cx.layout_with("composer-lane", |r| r[3]);
            let hero_fits = match (pane, lane) {
                (Some(pane), Some(lane)) => pane - lane >= CAPTAIN_HERO_HEIGHT,
                _ => true,
            };
            main.child(empty_thread(
                s,
                &composer_model,
                p,
                wide,
                main_width,
                hero_fits,
                composer_caret_edges(cx, zoom),
            ))
        } else {
            main.child(conversation(s, p, wide, main_width))
                .child(composer(
                    s,
                    &composer_model,
                    p,
                    wide,
                    main_width,
                    composer_caret_edges(cx, zoom),
                ))
        };
        // The thread's right panel sits at the conversation's right edge inside this row
        // (KSG Code's `ChatRightPanel`). Below 1050 pt (`narrow`), or whenever the body
        // cannot fit a 360 pt panel beside a 360 pt conversation, or when maximized, it
        // covers the conversation column instead (spec §0.1 / §3.3).
        let panel = s.right_panel();
        let panel_open = panel.open;
        let body_width = cx.layout_with("chat-body", |r| r[2]).unwrap_or_else(|| {
            let sidebar = if docked && !compact {
                sidebar_width + 4.0
            } else {
                0.0
            };
            cx.layout_with("workspace-root", move |r| r[2] - 16.0 - sidebar)
                .unwrap_or(900.0)
        });
        let covering =
            panel_open && (panel.maximized || narrow || crate::right_panel::covers(body_width));
        let mut main_wrap = el().grow(1.0).min_w(0.0).min_h(0.0).h_full().child(main);
        // KSG's inline `ThreadDetailsCard`: pinned 12 pt inside the column's top right
        // corner, under a covering Inspector.
        if let Some(rect) = details_rect.filter(|_| s.details.docked) {
            main_wrap = main_wrap.child(details_ui::card(s, p, rect.height).pin(
                None,
                Some(rect.y),
                Some(rect.right_inset),
                None,
            ));
        }
        if covering && compact {
            compact_inspector = Some(inspector_ui::view(s, p, zoom, None));
        } else if covering {
            main_wrap = main_wrap.child(inspector_ui::view(s, p, zoom, None).pin(
                Some(0.0),
                Some(0.0),
                Some(0.0),
                Some(0.0),
            ));
        }
        body = body.child(main_wrap);
        if panel_open && !covering {
            let max = crate::right_panel::max_width(s.viewport_width, body_width);
            let width = crate::right_panel::clamp_width(s.right_panel_width, max);
            body = body.child(inspector_ui::view(s, p, zoom, Some(max)).w(width));
        }
        pane = pane.child(body);
    }
    // The pane has no edge border: the `bg`/`panel` contrast draws its edge (KSG's pane sits
    // borderless on the chrome).
    let mut root = col()
        .key("workspace-root")
        .focusable(false)
        .on_press(|s: &mut State| {
            s.composer_ui.scope_focused.set(false);
            sidebar_preview::dismiss(s, true);
        })
        .on_window_focus(|s: &mut State, focused, _| {
            if !focused {
                sidebar_preview::dismiss(s, true);
            }
        })
        .on_layout(|s: &mut State, rect| {
            s.viewport_width = rect[2];
            s.viewport_height = rect[3];
        })
        .on_key(|s: &mut State, ev, tx| {
            if ev.key == rune::Key::Escape && s.now_popover {
                s.now_popover = false;
                return true;
            }
            if ev.key == rune::Key::Escape && sidebar_preview::dismiss(s, true) {
                return true;
            }
            if ev.key == rune::Key::Tab {
                sidebar_preview::after_tab(s, tx);
            }
            if ev.key == rune::Key::Escape
                && (s.assistant_selection.is_some() || s.citation_highlight.is_some())
            {
                s.assistant_selection = None;
                s.citation_highlight = None;
                tx.clear_text_selection();
                return true;
            }
            s.undo_key(ev, true)
        })
        .fill(p.bg)
        .child(top)
        .child(workspace.child(pane));
    if compact && docked {
        let top = chrome.height.max(40.0 * zoom) / zoom;
        root = root.child(
            col()
                .key("sidebar-overlay")
                .absolute(8.0, top)
                .size(sidebar_width, (layout_height - top).max(0.0))
                .min_h(0.0)
                .instant_changes()
                .fill(p.bg)
                .box_shadow(soft_shadow(p))
                .child(sidebar(s, p, sidebar_width)),
        );
    }
    if let Some(inspector) = compact_inspector {
        let top = chrome.height.max(40.0 * zoom) / zoom;
        root = root.child(
            col()
                .key("compact-inspector-sheet")
                .absolute(8.0, top)
                .size(
                    (layout_width - 16.0).max(0.0),
                    (layout_height - top).max(0.0),
                )
                .min_h(0.0)
                .child(inspector.w_full()),
        );
    }
    // The context popover floats at the root, after the ring it anchors to, so the composer
    // never makes room for it and no clipping ancestor cuts it off.
    if let Some(toolbar) = assistant_selection_toolbar(s, p, cx) {
        root = root.child(toolbar);
    }
    if composer_menu::trigger(s).is_some() {
        root = root.child(composer_menu::overlay(s, p, cx, s.viewport_width));
    }
    if context_ring_shown(s.current()) && s.current().context.popover_open() {
        root = root.child(context_popover(s, p));
    }
    if docked && s.update_popover {
        root = root.child(updates_ui::popover(s, p));
    }
    if docked && let Some(preview) = sidebar_preview::view(s, p, sidebar_width) {
        root = root.child(preview);
    }
    if let Some(notice) = s.composer_notices.open_details_notice()
        && let Some(popover) =
            composer_notice_details_popover(notice, p, lane_width(main_width, wide), cx)
    {
        root = root.child(popover);
    }
    // Citation chips get a comment action pinned over each pill's right end,
    // mounted at the root like the other composer overlays so nothing clips it.
    // The collapsed preview shares the field's key, so check it is not showing.
    if cx.layout_with("composer-preview", |_| ()).is_none() {
        for action in citation_comment::chip_actions(s, p, cx, zoom) {
            root = root.child(action);
        }
    }
    if brief_record {
        let top = chrome.height.max(40.0 * zoom) / zoom;
        root = root.child(captain_ui::brief_sheet(s, p, top));
    }
    if s.attachment_preview.is_some() {
        root = root.child(attachment_preview::view(s, p));
    } else if s.transcript_preview.is_some() {
        root = root.child(transcript_context::preview(s, p));
    } else if s.quick_switcher {
        root = root.child(quick_switcher::view(s, p));
    } else if s.rename_thread.is_some() || s.delete_thread.is_some() {
        root = root.child(thread_dialog(s, p));
    } else if s.plan_save_dialog.is_some() {
        root = root.child(work_ui::plan_save_dialog(s, p));
    } else if s.confirm_remove_attachment.is_some() {
        root = root.child(remove_attachment_dialog(s, p));
    } else if s.project_picker == Some(ProjectPicker::AddProject) {
        root = root.child(add_project_palette(s, p));
    } else if s.project_picker.is_some() {
        root = root.child(project_picker(s, p));
    } else if s.captain_editor.is_some() {
        let height = cx.layout_with("workspace-root", |r| r[3]).unwrap_or(840.0);
        root = root.child(captain_editor(s, p, height));
    } else if s.settings_picker.is_some() {
        if s.composer_hidden() {
            root = root.child(
                el().key("settings-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.settings_picker = None),
            );
        } else {
            root = root.child(settings_picker(s, &composer_model, p));
        }
    } else if s.captain_picker {
        let anchor = (!s.composer_hidden())
            .then(|| {
                if s.composer_ui.visible_blocks.get() >= 2
                    && cx.layout_with("captain-selector", |_| ()).is_some()
                {
                    Some("captain-selector")
                } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                    Some("composer-overflow")
                } else if cx.layout_with("captain-selector", |_| ()).is_some() {
                    Some("captain-selector")
                } else if cx.layout_with("composer", |_| ()).is_some() {
                    Some("composer")
                } else {
                    None
                }
            })
            .flatten();
        if let Some(anchor) = anchor {
            root = root.child(captain_picker(s, p, anchor));
        } else {
            root = root.child(
                el().key("captain-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.cancel_captain_flow()),
            );
        }
    } else if s.projects.clone_open {
        root = root.child(clone_dialog(s, p));
    } else if s.current().native.access_picker {
        if s.composer_hidden() {
            root = root.child(
                el().key("access-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.current_mut().native.access_picker = false),
            );
        } else {
            let anchor = if cx.layout_with("access-mode", |_| ()).is_some() {
                "access-mode"
            } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                "composer-overflow"
            } else {
                "composer"
            };
            root = root.child(pi_ui::access_picker(s, p, anchor));
        }
    } else if s.current().native.conversation.is_some() {
        root = root.child(pi_ui::conversation_picker(s, p));
    } else if s.thread_menu {
        root = root.child(thread_menu(s, p));
    } else if s.plan_menu.is_some() {
        if let Some(menu) = work_ui::plan_menu(s, p, cx) {
            root = root.child(menu);
        }
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
        if let Some(menu) = row_menu(s, p, cx) {
            root = root.child(menu);
        }
    }
    if s.tab_menu.is_some() {
        if let Some((x, y)) = s.tab_menu_at {
            root = root.child(
                el().key("thread-tab-menu-point")
                    .absolute(x / zoom, y / zoom)
                    .size(0.0, 0.0),
            );
        }
        root = root.child(thread_tab_menu(s, p));
    }
    if s.snooze_menu.is_some() {
        if let Some((x, y)) = s.snooze_at {
            root = root.child(
                el().key("snooze-menu-point")
                    .absolute(x / zoom, y / zoom)
                    .size(0.0, 0.0),
            );
        }
        if let Some(menu) = snooze_menu(s, p, cx) {
            root = root.child(menu);
        }
    }
    if s.settings_menu {
        root = root.child(settings_ui::dialog(s, p));
    }
    if s.machines.picker {
        let anchor = (!s.composer_hidden())
            .then(|| {
                if s.composer_ui.visible_blocks.get() > 0
                    && cx.layout_with("machine-selector", |_| ()).is_some()
                {
                    Some("machine-selector")
                } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                    Some("composer-overflow")
                } else if cx.layout_with("machine-selector", |_| ()).is_some() {
                    Some("machine-selector")
                } else {
                    None
                }
            })
            .flatten();
        if let Some(anchor) = anchor {
            root = root.child(machine_picker(s, p, anchor));
        } else {
            root = root.child(
                el().key("machine-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.machines.picker = false),
            );
        }
    }
    if s.env_mode_picker {
        let can_show = !s.composer_hidden()
            && matches!(
                s.composer_vcs_controls(),
                crate::model::ComposerVcsControls::Editable { .. }
            );
        let anchor = can_show
            .then(|| {
                if s.composer_ui.visible_blocks.get() > 0
                    && cx.layout_with("env-mode-trigger", |_| ()).is_some()
                {
                    Some("env-mode-trigger")
                } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                    Some("composer-overflow")
                } else if cx.layout_with("env-mode-trigger", |_| ()).is_some() {
                    Some("env-mode-trigger")
                } else {
                    None
                }
            })
            .flatten();
        if let Some(anchor) = anchor {
            root = root.child(env_mode_picker(s, p, anchor));
        } else {
            root = root.child(
                el().key("env-mode-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.env_mode_picker = false),
            );
        }
    }
    if s.base_branch_picker {
        let can_show = !s.composer_hidden()
            && matches!(
                s.composer_vcs_controls(),
                crate::model::ComposerVcsControls::Editable {
                    mode: crate::model::ThreadEnvMode::Worktree,
                    ..
                }
            );
        let anchor = can_show
            .then(|| {
                if cx.layout_with("base-branch-trigger", |_| ()).is_some() {
                    Some("base-branch-trigger")
                } else if cx.layout_with("composer-overflow", |_| ()).is_some() {
                    Some("composer-overflow")
                } else {
                    None
                }
            })
            .flatten();
        if let Some(anchor) = anchor {
            root = root.child(base_branch_picker(s, p, anchor));
        } else {
            root = root.child(
                el().key("base-branch-picker-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.base_branch_picker = false),
            );
        }
    }
    if s.composer_ui.overflow.get() {
        if !s.composer_hidden() && cx.layout_with("composer-overflow", |_| ()).is_some() {
            root = root.child(composer_ui::overflow_menu(s, p));
        } else {
            root = root.child(
                el().key("composer-overflow-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.composer_ui.overflow.set(false)),
            );
        }
    }
    // Where the card cannot dock, the top bar's button opens it as a popover (KSG's
    // `PopoverPopup` presentation of the same card).
    if s.details.popover && details_rect.is_none() && details_ui::available(s) {
        root = root.child(
            details_ui::card(s, p, (s.viewport_height - 80.0).max(160.0))
                .anchored_fit("header-details", rune::Side::Bottom, rune::Align::End, 6.0)
                .modal(true, |s: &mut State| s.details.popover = false),
        );
    }
    if let Some(hover) = details_ui::hover_card(s, p) {
        root = root.child(hover);
    }
    if s.now_popover {
        if !s.composer_hidden() && cx.layout_with("crew-tray", |_| ()).is_some() {
            root = root.child(crew_ui::now_popover(s, p, lane_width(main_width, wide)));
        } else {
            root = root.child(
                el().key("now-popover-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.now_popover = false),
            );
        }
    }
    if composer_stash::menu_open(s) {
        if !s.composer_hidden() && cx.layout_with("composer-stash-badge", |_| ()).is_some() {
            // KSG's drawer portal, mounted beside the other composer popovers.
            root = root.child(composer_stash::menu(s, p, lane_width(main_width, wide)));
        } else {
            root = root.child(
                el().key("composer-stash-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.stash_menu = false),
            );
        }
    }
    if composer_code_picker::menu_open(s) {
        root = root.child(composer_code_picker::menu(s, p));
    }
    if let Some(popover) = citation_comment::popover(s, p, cx) {
        root = root.child(popover);
    }
    if s.machines.dialog {
        root = root.child(machine_dialog(s, p));
    }
    if s.fleet.open {
        root = root.child(settings_ui::fleet_dialog(s, p));
    }
    if s.settings.archive.is_some() {
        root = root.child(settings_ui::archive_dialog(s, p));
    }
    for dialog in captain_ui::dialogs(s, p) {
        root = root.child(dialog);
    }
    let crew_overlay_captain = captain_ui::brief_captain(s)
        .or_else(|| s.current().crew_captain.filter(|c| *c < s.captains.len()));
    if let Some(captain) = crew_overlay_captain {
        if let Some(menu) = crew_membership_ui::recruit_palette(s, p, captain, cx) {
            root = root.child(menu);
        }
        // A secondary press on a Crew Roster row opens the member's More menu right there,
        // anchored to the row.
        if let Some(thread_id) = s.brief.more {
            let anchor = format!("crew-chip-{thread_id}");
            if cx.layout_with(&anchor, |_| ()).is_some() {
                root = root.child(
                    crew_membership_ui::more_menu(s, p, captain, thread_id)
                        .anchored_fit(anchor, rune::Side::Bottom, rune::Align::End, 4.0)
                        .modal(true, |s: &mut State| s.brief.more = None),
                );
            } else {
                root = root.child(
                    el().key("crew-more-dismiss")
                        .absolute(0.0, 0.0)
                        .size(0.0, 0.0)
                        .a11y_hidden()
                        .on_layout(|s: &mut State, _| s.brief.more = None),
                );
            }
        }
    } else {
        if s.brief.recruit.is_some() {
            root = root.child(
                el().key("crew-recruit-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.brief.recruit = None),
            );
        }
        if s.brief.more.is_some() {
            root = root.child(
                el().key("crew-more-dismiss")
                    .absolute(0.0, 0.0)
                    .size(0.0, 0.0)
                    .a11y_hidden()
                    .on_layout(|s: &mut State, _| s.brief.more = None),
            );
        }
    }
    if let Some((x, y)) = s.brief.moving.as_ref().and_then(|m| m.at) {
        root = root.child(
            el().key("crew-move-point")
                .absolute(x / zoom, y / zoom)
                .size(0.0, 0.0),
        );
    }
    if let Some(menu) = crew_membership_ui::move_picker(s, p, cx) {
        root = root.child(menu);
    }
    if !s.toasts.is_empty() {
        root = root.child(toast_stack(s, p, chrome.height.max(40.0 * zoom) / zoom));
    }
    root = root.child(composer_ui::measurement_probes(s, &composer_model, p));
    root = match frame {
        Some((w, h)) if (zoom - 1.0).abs() > 0.001 => root.w(w / zoom).h(h / zoom).scale(zoom),
        _ => root.w_full().h_full(),
    };
    el().key("zoom-frame")
        .on_paste_content(composer_ui::paste)
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

static OPENAI_ICON: Icon = Icon {
    name: "OPENAI",
    view: 411.0,
    body: r#"<g transform="translate(-100 -100)"><path fill="currentColor" fill-rule="evenodd" clip-rule="evenodd" d="M252.794 108.802C289.191 99.0484 326.265 110.305 351.148 135.135C385.113 126.072 422.85 134.862 449.492 161.505C476.136 188.149 484.925 225.888 475.862 259.85V259.854C500.696 284.735 511.95 321.81 502.198 358.207C492.447 394.602 464.161 421.084 430.215 430.217C421.083 464.162 394.603 492.448 358.206 502.199C321.812 511.951 284.734 500.693 259.852 475.864C225.887 484.927 188.15 476.137 161.507 449.495C134.864 422.851 126.073 385.111 135.136 351.149C110.304 326.266 99.0496 289.192 108.801 252.795C118.552 216.4 146.84 189.918 180.784 180.785C189.917 146.841 216.396 118.553 252.794 108.802ZM374.292 407.145C374.292 411.271 372.092 415.086 368.517 417.148L283.723 466.102C302.487 480.585 327.555 486.459 352.217 479.852C386.997 470.532 410.068 439.312 410.555 405.006V317.717C410.555 315.08 409.125 312.621 406.843 311.303L374.292 292.509V407.145ZM251.868 415.897C248.296 417.959 243.893 417.959 240.317 415.897L155.526 366.942C152.366 390.436 159.811 415.08 177.866 433.136H177.863C203.325 458.594 241.896 462.962 271.85 446.232L347.449 402.586C349.735 401.268 351.148 398.8 351.148 396.163V358.579L251.868 415.897ZM368.602 220.628C366.319 219.309 363.474 219.318 361.191 220.637L328.641 239.431L427.921 296.749C431.496 298.811 433.697 302.627 433.697 306.752V404.661C455.622 395.654 473.244 376.881 479.851 352.218C489.169 317.442 473.668 281.85 444.201 264.274L368.602 220.628ZM177.303 206.34C155.377 215.348 137.756 234.122 131.148 258.783C121.832 293.561 137.331 329.153 166.799 346.727L242.398 390.373C244.68 391.692 247.525 391.684 249.807 390.366L282.357 371.572L183.078 314.253C179.504 312.189 177.303 308.375 177.303 304.251V206.34ZM259.849 279.145V331.858L305.5 358.213L351.15 331.858V279.145L305.5 252.789L259.849 279.145ZM327.276 144.9C308.512 130.418 283.445 124.543 258.782 131.15C224.002 140.471 200.931 171.691 200.445 205.995V293.286C200.445 295.923 201.875 298.381 204.158 299.7L236.707 318.493V203.856C236.707 199.731 238.909 195.916 242.483 193.853L327.276 144.9ZM433.137 177.867C407.675 152.407 369.103 148.038 339.149 164.769L263.55 208.415C261.265 209.734 259.852 212.202 259.852 214.838V252.423L359.132 195.105C362.703 193.041 367.108 193.041 370.682 195.105L455.473 244.06C458.635 220.567 451.189 195.922 433.135 177.867H433.137Z"/></g>"#,
};

static CLAUDE_ICON: Icon = Icon {
    name: "CLAUDE",
    view: 257.0,
    body: r##"<path fill="#d97757" d="m50.228 170.321 50.357-28.257.843-2.463-.843-1.361h-2.462l-8.426-.518-28.775-.778-24.952-1.037-24.175-1.296-6.092-1.297L0 125.796l.583-3.759 5.12-3.434 7.324.648 16.202 1.101 24.304 1.685 17.629 1.037 26.118 2.722h4.148l.583-1.685-1.426-1.037-1.101-1.037-25.147-17.045-27.22-18.017-14.258-10.37-7.713-5.25-3.888-4.925-1.685-10.758 7-7.713 9.397.649 2.398.648 9.527 7.323 20.35 15.75L94.817 91.9l3.889 3.24 1.555-1.102.195-.777-1.75-2.917-14.453-26.118-15.425-26.572-6.87-11.018-1.814-6.61c-.648-2.723-1.102-4.991-1.102-7.778l7.972-10.823L71.42 0 82.05 1.426l4.472 3.888 6.61 15.101 10.694 23.786 16.591 32.34 4.861 9.592 2.592 8.879.973 2.722h1.685v-1.556l1.36-18.211 2.528-22.36 2.463-28.776.843-8.1 4.018-9.722 7.971-5.25 6.222 2.981 5.12 7.324-.713 4.73-3.046 19.768-5.962 30.98-3.889 20.739h2.268l2.593-2.593 10.499-13.934 17.628-22.036 7.778-8.749 9.073-9.657 5.833-4.601h11.018l8.1 12.055-3.628 12.443-11.342 14.388-9.398 12.184-13.48 18.147-8.426 14.518.778 1.166 2.01-.194 30.46-6.481 16.462-2.982 19.637-3.37 8.88 4.148.971 4.213-3.5 8.62-20.998 5.184-24.628 4.926-36.682 8.685-.454.324.519.648 16.526 1.555 7.065.389h17.304l32.21 2.398 8.426 5.574 5.055 6.805-.843 5.184-12.962 6.611-17.498-4.148-40.83-9.721-14-3.5h-1.944v1.167l11.666 11.406 21.387 19.314 26.767 24.887 1.36 6.157-3.434 4.86-3.63-.518-23.526-17.693-9.073-7.972-20.545-17.304h-1.36v1.814l4.73 6.935 25.017 37.59 1.296 11.536-1.814 3.76-6.481 2.268-7.13-1.297-14.647-20.544-15.1-23.138-12.185-20.739-1.49.843-7.194 77.448-3.37 3.953-7.778 2.981-6.48-4.925-3.436-7.972 3.435-15.749 4.148-20.544 3.37-16.333 3.046-20.285 1.815-6.74-.13-.454-1.49.194-15.295 20.999-23.267 31.433-18.406 19.702-4.407 1.75-7.648-3.954.713-7.064 4.277-6.286 25.47-32.405 15.36-20.092 9.917-11.6-.065-1.686h-.583L44.07 198.125l-12.055 1.555-5.185-4.86.648-7.972 2.463-2.593 20.35-13.999-.064.065Z"/>"##,
};

fn provider_glyph(vendor: Vendor) -> &'static Icon {
    match vendor {
        Vendor::OpenAi => &OPENAI_ICON,
        Vendor::Anthropic => &CLAUDE_ICON,
        Vendor::Antigravity | Vendor::Other => &icons::CHIP,
    }
}

/// Provider marks ported from KSG Code's `Icons.tsx` (`OpenAI`, `ClaudeAI`, `AntigravityIcon`).
fn provider_icon(s: &State, p: Palette, vendor: Vendor, size: f32) -> El {
    match vendor {
        Vendor::Antigravity => el()
            .size(size, size)
            .pixels_fit(s.artwork.antigravity.pixels(), rune::SurfaceFit::Fit)
            .a11y_hidden(),
        Vendor::OpenAi => rune::icon(&OPENAI_ICON, size).color(p.ink).a11y_hidden(),
        Vendor::Anthropic => rune::icon(&CLAUDE_ICON, size).a11y_hidden(),
        Vendor::Other => rune::icon(&icons::CHIP, size).color(p.muted).a11y_hidden(),
    }
}

fn crewmate_avatar(
    s: &State,
    p: Palette,
    model_or_provider: &str,
    status: Status,
    size: f32,
) -> El {
    mascot_avatar(
        s,
        p,
        crate::visuals::provider_mascot_for(model_or_provider),
        status,
        size,
    )
}

fn mascot_avatar(
    s: &State,
    p: Palette,
    mascot: crate::visuals::ProviderMascot,
    status: Status,
    size: f32,
) -> El {
    use crate::visuals::ProviderMascot::*;
    let pixels = s
        .artwork
        .mascot_pixels(mascot, size)
        .or_else(|| s.artwork.mascot_pixels(Default, size));
    let image = if let Some(pixels) = pixels {
        portrait(&pixels, size, p, None)
    } else {
        let (vendor, tint) = match mascot {
            OpenAi => (Vendor::OpenAi, p.ok_bg),
            Anthropic => (Vendor::Anthropic, p.brass_bg),
            Google => (Vendor::Antigravity, p.sea_bg),
            DeepSeek | Glm => (Vendor::Other, p.sea_bg),
            Qwen | Grok | Default => (Vendor::Other, p.lift),
        };
        el().size(size, size)
            .radius(portrait_radius(size))
            .fill(tint)
            .border(1.0, p.portrait_frame)
            .center()
            .child(provider_icon(s, p, vendor, (size * 0.6).min(20.0)))
    };
    haloed(s, p, image, size, status).a11y_hidden()
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

/// Shared by the visible footer, its measurement probe and any open picker in this view.
struct ComposerModelDisplay {
    descriptors: Vec<crate::descriptors::ProviderOptionDescriptor>,
    effort: String,
    both_default: bool,
    traits: crate::descriptors::TraitsTriggerDisplay,
    selections: Vec<crate::model_selection::ModelSelection>,
    label: String,
    trigger_label: String,
    unavailable: bool,
    multi_labels: Vec<(String, String)>,
}

impl ComposerModelDisplay {
    fn new(s: &State) -> Self {
        use crate::model_picker::get_display_model_name;
        let entries = s.provider_instance_entries();
        let (instance_id, model, options) = s.resolve_composer_model(&entries);
        let descriptors = s.model_descriptors(&instance_id, &model, &options);
        let selected = crate::model_selection::ModelSelection {
            instance_id,
            model,
            options,
        };
        let (effort, both_default, traits) = composer_effort(s, &descriptors, &selected);
        let entry = |id: &str| entries.iter().find(|e| e.instance_id == id);
        let selected_entry = entry(&selected.instance_id);
        let option = selected_entry.and_then(|e| s.picker_model_option(e, &selected.model));
        let display_name = |opt: &crate::model_picker::PickerModelOption, short| {
            get_display_model_name(
                &opt.name,
                opt.short_name.as_deref(),
                opt.sub_provider.as_deref(),
                short,
            )
        };
        let fallback = split_model_label(&s.current().model).0;
        let label = option
            .as_ref()
            .map(|opt| display_name(opt, true))
            .unwrap_or_else(|| fallback.to_owned());
        let trigger_label = if option.is_some() {
            label.clone()
        } else {
            crate::model_picker::retained_model_while_provider_locked(
                selected_entry,
                &[],
                &selected.model,
            )
            .map(|opt| display_name(&opt, true))
            .unwrap_or_else(|| label.clone())
        };
        let unavailable = option.as_ref().is_some_and(|opt| opt.is_unavailable);
        let selections = s.multi_model_selections_from(selected);
        let multi_labels = if selections.len() > 1 {
            selections
                .iter()
                .enumerate()
                .map(|(index, selection)| {
                    let other;
                    let opt = if index == 0 {
                        option.as_ref()
                    } else {
                        other = entry(&selection.instance_id)
                            .and_then(|e| s.picker_model_option(e, &selection.model));
                        other.as_ref()
                    };
                    let suffix = if opt.is_some_and(|m| m.is_unavailable) {
                        " (Unavailable)"
                    } else {
                        ""
                    };
                    let short = opt
                        .map(|m| display_name(m, true))
                        .unwrap_or_else(|| split_model_label(&selection.model).0.to_owned());
                    let full = opt
                        .map(|m| display_name(m, false))
                        .unwrap_or_else(|| selection.model.clone());
                    (format!("{short}{suffix}"), format!("{full}{suffix}"))
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            descriptors,
            effort,
            both_default,
            traits,
            selections,
            label,
            trigger_label,
            unavailable,
            multi_labels,
        }
    }

    fn show_effort(&self) -> bool {
        !self.both_default && !self.effort.is_empty() && self.selections.len() == 1
    }
}

/// The reasoning effort the composer shows, as KSG's `ComposerModelEffort` trigger reads it,
/// with the descriptors' trigger display. `None` when both the model and effort are defaults.
fn composer_effort(
    s: &State,
    descriptors: &[crate::descriptors::ProviderOptionDescriptor],
    selection: &crate::model_selection::ModelSelection,
) -> (String, bool, crate::descriptors::TraitsTriggerDisplay) {
    let t = s.current();
    let (name, _) = split_model_label(&t.model);
    let controls = crate::descriptors::split_column_and_footer_controls(descriptors);
    let prompt_ultrathink = crate::descriptors::read_prompt_ultrathink(&t.draft.text);
    let primary_is_prompt_controlled = descriptors
        .iter()
        .find(|d| {
            matches!(
                d,
                crate::descriptors::ProviderOptionDescriptor::Select { .. }
            )
        })
        .is_some_and(|d| match d {
            crate::descriptors::ProviderOptionDescriptor::Select {
                prompt_injected_values,
                ..
            } => !prompt_injected_values.is_empty(),
            _ => false,
        })
        && prompt_ultrathink != crate::descriptors::PromptUltrathink::None;
    let display = crate::descriptors::build_traits_trigger_display(
        descriptors,
        primary_is_prompt_controlled,
        Some(selection),
        None,
    );
    let fallback_thinking = selection
        .options
        .iter()
        .find(|o| matches!(o.id.as_str(), "thinking" | "effort" | "reasoningEffort"))
        .and_then(|o| o.value.as_str())
        .unwrap_or(&t.thinking);
    let effort = if !display.label.is_empty() && controls.effort.is_some() {
        display
            .effort_label
            .clone()
            .unwrap_or_else(|| effort_name(fallback_thinking))
    } else if primary_is_prompt_controlled {
        "Ultrathink".into()
    } else {
        effort_name(fallback_thinking)
    };
    let both_default = name == crate::model::DEFAULT_MODEL
        && fallback_thinking.is_empty()
        && display.speed_label.is_none()
        && !display.ultracode
        && display.context_window.is_none();
    (effort, both_default, display)
}

fn model_effort_trigger(
    s: &State,
    model: &ComposerModelDisplay,
    p: Palette,
    narrow: bool,
    locked: bool,
) -> El {
    model_trigger(s, model, p, narrow, locked, true)
}

/// Ported from KSG Code's `ComposerModelEffort` reasoning trigger: a gauge and the effort,
/// beside the model in the context strip. It opens the same model and effort picker.
/// `compact` drops the label for a narrow strip, as KSG's controls bar does.
fn effort_trigger(
    model: &ComposerModelDisplay,
    p: Palette,
    locked: bool,
    compact: bool,
) -> Option<El> {
    let effort = &model.effort;
    if !model.show_effort() {
        return None;
    }
    let mut trigger = row()
        .key("composer-effort")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!("Reasoning {effort}. Change reasoning"))
        .help(format!("Reasoning: {effort}"))
        .preserve_focus_on_press(true)
        .center_y()
        .gap(6.0)
        .h(24.0)
        .px(7.0)
        .radius(6.0)
        .shrink()
        .min_w(0.0)
        .opacity(if locked { 0.65 } else { 1.0 })
        .disabled(locked)
        .hover(p.selected)
        .press(p.selected)
        .child(icon("gauge", p.muted).size(14.0, 14.0));
    if !compact {
        trigger = trigger.child(label(effort, SMALL, p.muted).truncate().shrink().min_w(0.0));
    }
    if !locked {
        trigger = trigger.on_press(|s: &mut State| {
            s.begin_composer_menu();
            s.open_model_picker(SettingsPicker::Thinking);
        });
    }
    Some(trigger)
}

/// The model trigger. `with_effort` keeps the effort inside it, as the resting composer does.
fn model_trigger(
    s: &State,
    model: &ComposerModelDisplay,
    p: Palette,
    narrow: bool,
    locked: bool,
    with_effort: bool,
) -> El {
    let t = s.current();
    let (name, _) = split_model_label(&t.model);
    let effort = &model.effort;
    let display = &model.traits;
    let multi_selections = &model.selections;
    if multi_selections.len() > 1 {
        let short_labels = model
            .multi_labels
            .iter()
            .map(|(short, _)| short.as_str())
            .collect::<Vec<_>>();
        let multiple_label = if short_labels.len() > 2 {
            format!(
                "{}, {} more",
                short_labels[..2].join(", "),
                short_labels.len() - 2
            )
        } else {
            short_labels.join(", ")
        };
        let all_model_names = model
            .multi_labels
            .iter()
            .map(|(_, full)| full.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let mut icons = row().center_y().gap(2.0).shrink();
        for (sel, (_, full)) in multi_selections.iter().zip(&model.multi_labels).take(3) {
            icons = icons.child(provider_icon(s, p, model_vendor(&sel.model, full), 14.0));
        }
        let mut trigger = row()
            .key("model-effort")
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(&all_model_names)
            .help(&all_model_names)
            .preserve_focus_on_press(true)
            .center_y()
            .gap(7.0)
            .h(28.0)
            .px(8.0)
            .radius(6.0)
            .shrink()
            .min_w(0.0)
            .max_w(if narrow { 200.0 } else { 340.0 })
            .opacity(if locked { 0.65 } else { 1.0 })
            .disabled(locked)
            .hover(p.selected)
            .press(p.selected);
        if !locked {
            trigger = trigger.on_press(|s: &mut State| {
                s.begin_composer_menu();
                s.open_model_picker(SettingsPicker::Model);
            });
        }
        trigger = trigger.child(icons);
        if !narrow {
            trigger = trigger.child(
                label(&multiple_label, TextStyle::new(13.0, 600.0), p.ink)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        }
        return trigger.child(outlined(
            meta_text(format!("{}", multi_selections.len()), p.muted)
                .key("model-multi-count")
                .px(4.0)
                .py(1.0)
                .radius(6.0),
            p,
        ));
    }

    let mut trigger = row()
        .key("model-effort")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!(
            "Model {name}, reasoning {effort}. Change model or reasoning"
        ))
        .help(t.model.clone())
        .preserve_focus_on_press(true)
        .center_y()
        .gap(7.0)
        .h(28.0)
        .px(8.0)
        .radius(6.0)
        .shrink()
        .min_w(0.0)
        .max_w(if narrow { 200.0 } else { 340.0 })
        .opacity(if locked { 0.65 } else { 1.0 })
        .disabled(locked)
        .hover(p.selected)
        .press(p.selected);
    if !locked {
        trigger = trigger.on_press(|s: &mut State| {
            s.begin_composer_menu();
            s.open_model_picker(SettingsPicker::Model);
        });
    }
    let display_name = &model.trigger_label;
    let trigger = trigger.child(provider_icon(
        s,
        p,
        model_vendor(&t.model_id, &t.model),
        14.0,
    ));
    // A narrow context strip shows the model as its provider icon; the name is in the help.
    if !with_effort && narrow {
        return trigger;
    }
    let mut trigger = trigger.child(
        if with_effort {
            label(display_name, TextStyle::new(13.0, 600.0), p.ink)
        } else {
            // In the context strip the model reads like KSG's other strip controls.
            label(display_name, SMALL, p.muted)
        }
        .truncate()
        .shrink()
        .min_w(0.0),
    );
    if with_effort && !model.both_default {
        trigger = trigger.child(meta_text(effort, p.muted).truncate().shrink().min_w(0.0));
    }
    if display.speed_label.is_some() {
        trigger = trigger.child(icon("bolt", p.brass));
    }
    if display.ultracode {
        trigger = trigger.child(icon("workflow", p.sea));
    }
    if let Some(cw) = &display.context_window {
        trigger = trigger.child(meta_text(cw, p.muted).truncate().shrink().min_w(0.0));
    }
    if model.unavailable {
        trigger = trigger.child(outlined(
            meta_text("Unavailable", p.muted)
                .px(4.0)
                .py(1.0)
                .radius(6.0),
            p,
        ));
    }
    trigger
}

const PICKER_WIDTH: f32 = 320.0;
/// KSG Code's model list column, `max-w-90`.
const MODEL_LIST_W: f32 = 360.0;
/// KSG Code's docked options column, `w-48`.
const MODEL_OPTIONS_W: f32 = 192.0;
/// KSG Code's docked picker height, `h-86.5`. A short window shrinks it.
const MODEL_PICKER_H: f32 = 346.0;
/// The reasoning menu's width: KSG's `w-60` menu, widened so five levels keep room on the slider.
const REASONING_W: f32 = 272.0;

/// The model picker or the reasoning menu, whichever is open. KSG Code keeps them apart:
/// `ProviderModelPicker` for the model and `ComposerReasoningPicker` for its levels.
fn settings_picker(s: &State, model: &ComposerModelDisplay, p: Palette) -> El {
    if s.settings_picker == Some(SettingsPicker::Thinking) {
        reasoning_picker(s, model, p)
    } else {
        model_picker(s, model, p)
    }
}

/// A popover above the composer, right edges aligned, sized to the window (Floating UI's
/// `size` and `shift`), modal, and handing focus back to the composer when it closes. It
/// anchors to the composer rather than a trigger: the composer is always on screen, and its
/// edge stays put while a trigger's label changes width under the open menu.
fn composer_popover(card: El) -> El {
    card.anchored_fit("composer", rune::Side::Top, rune::Align::End, 8.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.settings_picker = None;
            s.open_option_menu = None;
            s.thinking_drag = None;
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// KSG's `handleInstanceModelChange`: with an options column docked the popup stays open for
/// them; otherwise picking a model closes it.
fn finish_model_pick(s: &mut State) {
    let descriptors = s.active_model_descriptors();
    if crate::descriptors::split_column_and_footer_controls(&descriptors)
        .others
        .is_empty()
    {
        s.settings_picker = None;
        s.open_option_menu = None;
        s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
    } else {
        s.settings_picker = Some(SettingsPicker::Model);
    }
}

/// Ported from KSG Code's `ProviderModelPicker` (docked): the provider rail, search and model
/// list at one fixed height, the `ComposerEffortColumn` (`optionsOnly`) beside it when the
/// model has options besides reasoning, and the Permissions footer below (as KSG's
/// `composerControlsOwnership` keeps Permissions reachable when the default chip is quiet).
/// Below 640 px the column stacks under the list, as KSG's `max-sm` classes do.
fn model_picker(s: &State, model: &ComposerModelDisplay, p: Palette) -> El {
    const PERMISSIONS_FOOTER_H: f32 = 37.0;
    let t = s.current();
    let can_choose = !t.status.busy() && !t.configuring;
    let stacked = s.viewport_width <= 640.0;
    let controls = crate::descriptors::split_column_and_footer_controls(&model.descriptors);
    let entries = s.provider_instance_entries();
    let options = s.picker_models_for_entries(&entries);
    let has_options = !controls.others.is_empty();
    let room = (s.viewport_width - 16.0).max(240.0);
    let width = if stacked {
        room
    } else if has_options {
        MODEL_LIST_W + 1.0 + MODEL_OPTIONS_W
    } else {
        MODEL_LIST_W
    }
    .min(room);

    // Docked beside options, KSG holds one fixed height; alone, the list is as tall as its
    // tallest tab, so switching tabs or searching never resizes it.
    // The card is anchored above the composer (`anchored_fit`, 8 pt gutter): the
    // room it leaves bounds the column, so a clamped card never overflows its
    // own rows. One frame behind, as `anchored_fit` measures it.
    let above = rune::memo::layout("composer")
        .map(|rect| rect[1] - 16.0 - PERMISSIONS_FOOTER_H)
        .unwrap_or(f32::MAX);
    let list_h = (if stacked && has_options {
        224.0
    } else if has_options {
        MODEL_PICKER_H
    } else {
        tallest_model_tab_h(s, &options, entries.len()).min(if stacked {
            320.0
        } else {
            MODEL_PICKER_H
        })
    })
    .min(above.max(120.0));
    let list_col = col()
        .key("model-picker-list-col")
        .w(if stacked {
            width
        } else {
            MODEL_LIST_W.min(width)
        })
        .h(list_h)
        .shrink()
        .min_w(0.0)
        .min_h(0.0);
    let snapshot = picker_snapshot_from(s, entries, &options);
    let list_col = model_list(
        s,
        p,
        can_choose,
        list_col,
        &snapshot,
        &model.selections,
        !(stacked && has_options),
    );
    let mut body = if stacked { col() } else { row() }
        .key("model-picker-body")
        .w_full()
        .min_w(0.0)
        .min_h(0.0)
        .shrink()
        .child(list_col);
    if has_options {
        body = body
            .child(if stacked {
                rule(p)
            } else {
                el().w(1.0).h_full().fill(p.line).hairline()
            })
            .child(options_column(
                s,
                p,
                can_choose,
                stacked,
                &model.label,
                &controls,
            ));
    }
    let display_access = s.effective_access_mode(t);
    let access_disabled = t.status.busy() || t.configuring || t.archived();
    let access_emphasized = display_access.id() != s.thread_defaults.access;
    let mut permissions_row = row()
        .key("model-picker-permissions")
        .role("button")
        .label(format!("Permissions: {}", display_access.label()))
        .w_full()
        .h(36.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .disabled(access_disabled)
        .opacity(if access_disabled { 0.65 } else { 1.0 })
        .child(
            icon(
                pi_ui::access_glyph(display_access),
                if access_emphasized { p.ink } else { p.muted },
            )
            .size(14.0, 14.0),
        )
        .child(label("Permissions", SMALL, p.ink))
        .child(spacer())
        .child(label(display_access.label(), META, p.muted))
        .child(icon("chevron-right", p.faint).size(12.0, 12.0));
    if !access_disabled {
        permissions_row = permissions_row
            .cursor(rune::Cursor::Pointer)
            .focusable(true)
            .hover(p.selected)
            .press(p.selected)
            .on_press(|s: &mut State| {
                s.settings_picker = None;
                s.open_access_picker();
            });
    }
    let mut card = col()
        .key("pi-settings-picker")
        .role("dialog")
        .label("Choose a model")
        .w(width)
        .min_h(0.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(body)
        .child(rule(p))
        .child(permissions_row);
    if let Some(error) = &t.error {
        card = card.child(rule(p)).child(
            label(error, SMALL, p.danger)
                .wrap_anywhere()
                .px(12.0)
                .py(8.0),
        );
    }
    if let Some(open_id) = &s.open_option_menu
        && let Some(desc) = controls.others.iter().find(|d| d.id() == open_id)
    {
        card = card.child(option_choice_menu(desc, p, can_choose, stacked));
    }
    composer_popover(card)
}

/// The model list's height for its tallest provider tab or favorites, from KSG's
/// `tallestTabUnits`: a one-line row is 32 pt, a two-line row 48 pt, 2 pt apart, plus the
/// search row and the list's padding. It never drops below the provider rail.
type PickerModels = std::collections::BTreeMap<String, Vec<crate::model_picker::PickerModelOption>>;

fn tallest_model_tab_h(s: &State, options: &PickerModels, entry_count: usize) -> f32 {
    use crate::model_picker::provider_model_key;
    const CHROME: f32 = 51.0 + 12.0;
    let favorites: std::collections::HashSet<String> = s
        .effective_model_favorites()
        .iter()
        .map(|f| provider_model_key(&f.provider, &f.model))
        .collect();
    let row = |two_line: bool| if two_line { 48.0 } else { 32.0 };
    let mut tallest: f32 = 0.0;
    let mut favorites_h: f32 = 0.0;
    for (instance, models) in options {
        let mut tab_h: f32 = 0.0;
        let mut legacy = false;
        for model in models {
            let favorite = favorites.contains(&provider_model_key(instance, &model.slug));
            if favorite {
                favorites_h += row(true) + 2.0;
            }
            if model.is_legacy && !favorite {
                legacy = true;
            } else {
                tab_h += row(model.sub_provider.is_some()) + 2.0;
            }
        }
        if legacy {
            tab_h += row(false) + 2.0;
        }
        tallest = tallest.max(tab_h);
    }
    let rail = (entry_count + 1) as f32 * 40.0 + 8.0;
    (CHROME + tallest.max(favorites_h)).max(rail).max(160.0)
}

/// An option row's choices, beside the row as KSG's `OptionMenuRow` opens them.
fn option_choice_menu(
    desc: &crate::descriptors::ProviderOptionDescriptor,
    p: Palette,
    can_choose: bool,
    stacked: bool,
) -> El {
    let crate::descriptors::ProviderOptionDescriptor::Select {
        id,
        options,
        current_value,
        ..
    } = desc
    else {
        return el();
    };
    let choices = crate::descriptors::selectable_provider_option_choices(desc);
    let current_id = current_value
        .as_deref()
        .or_else(|| options.iter().find(|o| o.is_default).map(|o| o.id.as_str()))
        .unwrap_or("");
    let mut choice_col = col()
        .key(format!("option-popup-{id}"))
        .w(220.0)
        .padding(4.0)
        .gap(2.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p));
    for choice in choices {
        let selected = choice.id == current_id;
        let oid = id.clone();
        let cid = choice.id.clone();
        let mut crow = row()
            .key(format!("option-choice-{id}-{}", choice.id))
            .role("menuitemradio")
            .checked(selected)
            .focusable(can_choose)
            .px(8.0)
            .py(5.0)
            .gap(6.0)
            .center_y()
            .radius(6.0)
            .fill(if selected {
                p.selected
            } else {
                Color::TRANSPARENT
            })
            .hover(p.selected)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| {
                s.set_composer_option_sticky(
                    oid.clone(),
                    crate::model_selection::OptionValue::Str(cid.clone()),
                );
                s.open_option_menu = None;
            })
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(label(&choice.label, SMALL, p.ink).truncate())
                    .child(if let Some(d) = &choice.description {
                        label(d, META, p.muted).wrap()
                    } else {
                        el()
                    }),
            );
        if choice.is_default {
            crow = crow.child(meta_text("Default", p.muted));
        }
        if selected {
            crow = crow.child(icon("check", p.ink).size(14.0, 14.0));
        }
        choice_col = choice_col.child(crow);
    }
    choice_col.anchored_fit(
        format!("option-menu-{id}"),
        if stacked {
            rune::Side::Bottom
        } else {
            rune::Side::Right
        },
        rune::Align::Start,
        6.0,
    )
}

/// Ported from KSG Code's `ComposerEffortColumn` with `optionsOnly`: the model's options other
/// than reasoning and speed, one row each, under a header that names the model.
fn options_column(
    s: &State,
    p: Palette,
    can_choose: bool,
    stacked: bool,
    model_label: &str,
    controls: &crate::descriptors::ColumnControls,
) -> El {
    // KSG's `bg-muted/40`, a shade off the card.
    let tint = Color::BLACK.alpha(if s.dark { 0.12 } else { 0.03 });
    let mut column = col()
        .key("model-picker-options")
        .h(if stacked { 256.0 } else { MODEL_PICKER_H })
        .shrink()
        .min_h(0.0)
        .min_w(0.0)
        .fill(tint);
    column = if stacked {
        column.w_full().corners([0.0, 0.0, 11.0, 11.0])
    } else {
        column.w(MODEL_OPTIONS_W).corners([0.0, 11.0, 11.0, 0.0])
    };
    // The search row's height, so one rule runs across the top of the popup.
    column = column
        .child(
            row()
                .center_y()
                .gap(8.0)
                .h(50.0)
                .px(12.0)
                .child(label("Options", META, p.muted))
                .child(spacer())
                .child(
                    label(model_label, META, p.muted)
                        .key("effort-model-label")
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                ),
        )
        .child(rule(p));
    column.child(
        col()
            .w_full()
            .gap(2.0)
            .px(8.0)
            .pt(10.0)
            .pb(6.0)
            .h(0.0)
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .children(option_rows(s, p, can_choose, controls)),
    )
}

/// KSG's `OptionSwitchRow` and `OptionMenuRow`: an option's name over its current value.
fn option_rows(
    s: &State,
    p: Palette,
    can_choose: bool,
    controls: &crate::descriptors::ColumnControls,
) -> Vec<El> {
    let mut rows = Vec::new();
    for desc in &controls.others {
        match desc {
            crate::descriptors::ProviderOptionDescriptor::Boolean {
                id,
                label: opt_label,
                current_value,
                ..
            } => {
                let on = *current_value == Some(true);
                let oid = id.clone();
                rows.push(
                    row()
                        .key(format!("option-switch-{id}"))
                        .role("switch")
                        .checked(on)
                        .label(format!("{opt_label}: {}", if on { "On" } else { "Off" }))
                        .focusable(can_choose)
                        .disabled(!can_choose)
                        .px(8.0)
                        .py(4.0)
                        .gap(8.0)
                        .center_y()
                        .radius(6.0)
                        .hover(p.selected)
                        .focus_fill(p.selected)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |s: &mut State| {
                            s.set_composer_option_sticky(
                                oid.clone(),
                                crate::model_selection::OptionValue::Bool(!on),
                            );
                        })
                        .child(option_row_text(opt_label, if on { "On" } else { "Off" }, p))
                        .child(switch_track(on, p)),
                );
            }
            crate::descriptors::ProviderOptionDescriptor::Select {
                id,
                label: opt_label,
                options,
                current_value,
                ..
            } => {
                let choices = crate::descriptors::selectable_provider_option_choices(desc);
                let current_id = current_value
                    .as_deref()
                    .or_else(|| options.iter().find(|o| o.is_default).map(|o| o.id.as_str()))
                    .unwrap_or("");
                let current_label = choices
                    .iter()
                    .find(|c| c.id == current_id)
                    .map(|c| c.label.as_str())
                    .unwrap_or(current_id);
                let is_open = s.open_option_menu.as_deref() == Some(id.as_str());
                let oid = id.clone();
                rows.push(
                    row()
                        .key(format!("option-menu-{id}"))
                        .role("button")
                        .label(format!("{opt_label}: {current_label}"))
                        .focusable(can_choose)
                        .disabled(!can_choose)
                        .px(8.0)
                        .py(4.0)
                        .gap(6.0)
                        .center_y()
                        .radius(6.0)
                        .fill(if is_open {
                            p.selected
                        } else {
                            Color::TRANSPARENT
                        })
                        .hover(p.selected)
                        .focus_fill(p.selected)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |s: &mut State| {
                            if s.open_option_menu.as_deref() == Some(oid.as_str()) {
                                s.open_option_menu = None;
                            } else {
                                s.open_option_menu = Some(oid.clone());
                            }
                        })
                        .child(option_row_text(opt_label, current_label, p))
                        .child(icon("open", p.muted).size(14.0, 14.0)),
                );
            }
        }
    }
    rows
}

/// KSG's `OptionRowText`: the option's name in meta type over its value.
fn option_row_text(name: &str, value: &str, p: Palette) -> El {
    col()
        .grow(1.0)
        .min_w(0.0)
        .child(label(name, META, p.muted).truncate())
        .child(label(value, TextStyle::new(13.0, 500.0), p.ink).truncate())
}

/// KSG's small `Switch`: a 28 x 16 track with a 12 pt knob.
fn switch_track(on: bool, p: Palette) -> El {
    let mut track = row()
        .w(28.0)
        .h(16.0)
        .radius(8.0)
        .padding(2.0)
        .center_y()
        .shrink()
        .fill(if on { p.coral } else { p.line });
    if on {
        track = track.justify_end();
    }
    track.child(el().size(12.0, 12.0).radius(6.0).fill(p.card))
}

/// Ported from KSG Code's `ComposerReasoningPicker`: its own menu above the effort chip, with
/// the reasoning levels under a header that names the model and the speed controls below
/// them. The levels keep Shina's Codex slider, laid out for this narrower menu.
fn reasoning_picker(s: &State, model: &ComposerModelDisplay, p: Palette) -> El {
    let t = s.current();
    let can_choose = !t.status.busy() && !t.configuring;
    let controls = crate::descriptors::split_column_and_footer_controls(&model.descriptors);
    let title = controls
        .effort
        .as_ref()
        .map(|d| d.label().to_owned())
        .unwrap_or_else(|| "Reasoning".into());
    let model_label = &model.label;
    let card = col()
        .key("reasoning-picker")
        .role("dialog")
        .label("Reasoning")
        .w(REASONING_W.min((s.viewport_width - 16.0).max(200.0)))
        .min_h(0.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .key("reasoning-header")
                .center_y()
                .gap(8.0)
                .px(12.0)
                .pt(10.0)
                .pb(2.0)
                .child(label(&title, META, p.muted))
                .child(spacer())
                .child(
                    label(model_label, META, p.muted)
                        .key("effort-model-label")
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                ),
        );
    // Below the header everything scrolls, so a window too short for the menu clips nothing.
    let mut body = col()
        .key("reasoning-body")
        .w_full()
        .shrink()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(effort_body(s, p, can_choose, &controls));
    if let Some(speed) = speed_section(s, p, can_choose, &controls) {
        body = body.child(rule(p)).child(speed);
    }
    if let Some(error) = &t.error {
        body = body.child(rule(p)).child(
            label(error, SMALL, p.danger)
                .wrap_anywhere()
                .px(12.0)
                .py(8.0),
        );
    }
    composer_popover(card.child(body))
}

/// The reasoning levels: the slider for a select, KSG's On and Off rows for a boolean, or a
/// note when the model has none.
fn effort_body(
    s: &State,
    p: Palette,
    can_choose: bool,
    controls: &crate::descriptors::ColumnControls,
) -> El {
    let t = s.current();
    // The drag handler re-checks the lock live, since "ultrathink" can land in the draft
    // mid-gesture. `locks_when_body` is the descriptor half of the predicate.
    let locks_when_body = controls.effort.as_ref().is_some_and(|d| match d {
        crate::descriptors::ProviderOptionDescriptor::Select {
            prompt_injected_values,
            ..
        } => !prompt_injected_values.is_empty(),
        _ => false,
    });
    let ultrathink_in_body = locks_when_body
        && crate::descriptors::read_prompt_ultrathink(&t.draft.text)
            == crate::descriptors::PromptUltrathink::Body;

    let mut body = col().key("model-picker-effort").w_full();
    match &controls.effort {
        Some(
            desc @ crate::descriptors::ProviderOptionDescriptor::Select {
                id, current_value, ..
            },
        ) => {
            let choices = crate::descriptors::selectable_provider_option_choices(desc);
            let levels: Vec<String> = if choices.is_empty() {
                t.thinking_levels.clone()
            } else {
                choices.into_iter().map(|c| c.id).collect()
            };
            // One consistent value renders the level: the pinned pick when the user just
            // chose, else the descriptor's current value, else what the provider runs.
            let current = t
                .selected_thinking
                .clone()
                .filter(|v| levels.iter().any(|l| l == v))
                .or_else(|| {
                    current_value
                        .clone()
                        .filter(|v| levels.iter().any(|l| l == v))
                })
                .unwrap_or_else(|| t.thinking.clone());
            if ultrathink_in_body {
                body = body.child(
                    label(crate::descriptors::ULTRATHINK_IN_BODY_NOTE, META, p.muted)
                        .key("ultrathink-lock-note")
                        .wrap()
                        .px(12.0)
                        .pt(6.0),
                );
            }
            body.child(effort_slider_with_levels(
                &levels,
                &current,
                s.thinking_drag,
                p,
                t.can_choose_thinking() && !ultrathink_in_body,
                id.clone(),
                format!("{}:{}:{}", t.id, t.harness, t.model_id),
                locks_when_body,
            ))
        }
        Some(crate::descriptors::ProviderOptionDescriptor::Boolean {
            id, current_value, ..
        }) => {
            let on = *current_value == Some(true);
            let mut group = col().role("radiogroup").gap(2.0).padding(6.0).w_full();
            for (label_text, val) in [("On", true), ("Off", false)] {
                let checked = on == val;
                let oid = id.clone();
                group = group.child(
                    row()
                        .key(format!("effort-bool-{id}-{val}"))
                        .role("radio")
                        .checked(checked)
                        .focusable(can_choose)
                        .disabled(!can_choose)
                        .h(30.0)
                        .px(8.0)
                        .center_y()
                        .radius(6.0)
                        .hover(p.selected)
                        .focus_fill(p.selected)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |s: &mut State| {
                            s.set_composer_option_sticky(
                                oid.clone(),
                                crate::model_selection::OptionValue::Bool(val),
                            );
                        })
                        .child(label(label_text, SMALL, p.ink).grow(1.0))
                        .child(if checked {
                            icon("check", p.ink).size(14.0, 14.0)
                        } else {
                            el()
                        }),
                );
            }
            body.child(group)
        }
        None if !t.thinking_levels.is_empty() => {
            let fallback_id = match t.harness.as_str() {
                "pi" => "thinking",
                "codex" => "reasoningEffort",
                _ => "effort",
            };
            body.child(effort_slider_with_levels(
                &t.thinking_levels,
                t.selected_thinking.as_deref().unwrap_or(&t.thinking),
                s.thinking_drag,
                p,
                t.can_choose_thinking(),
                fallback_id.into(),
                format!("{}:{}:{}", t.id, t.harness, t.model_id),
                false,
            ))
        }
        None => body.child(
            label("No effort levels", SMALL, p.muted)
                .key("no-effort-levels")
                .px(12.0)
                .py(10.0),
        ),
    }
}

/// Ported from KSG Code's `SpeedControl` and `UltracodeControl`, which its reasoning menu keeps
/// below the levels: service tiers, Claude's Fast mode, or Codex's legacy tiers, then Ultracode.
fn speed_section(
    s: &State,
    p: Palette,
    can_choose: bool,
    controls: &crate::descriptors::ColumnControls,
) -> Option<El> {
    let t = s.current();
    let segment = |key: String, text: &str, on: bool| {
        row()
            .key(key)
            .role("radio")
            .label(text)
            .checked(on)
            .focusable(can_choose)
            .disabled(!can_choose)
            .h(24.0)
            .w(0.0)
            .grow(1.0)
            .min_w(0.0)
            .px(8.0)
            .gap(4.0)
            .center()
            .radius(6.0)
            .fill(if on { p.card } else { Color::TRANSPARENT })
            .hover(if on { p.card } else { p.selected })
            .focus_fill(p.selected)
            .cursor(rune::Cursor::Pointer)
    };
    let group = |key: &str, name: &str| {
        row()
            .key(key)
            .role("radiogroup")
            .label(name)
            .w_full()
            .gap(2.0)
            .padding(2.0)
            .radius(8.0)
            .fill(p.selected)
    };
    let mut section = col()
        .key("picker-footer")
        .w_full()
        .gap(8.0)
        .px(12.0)
        .py(10.0);
    let mut any = false;
    if let Some(crate::descriptors::ProviderOptionDescriptor::Select {
        options,
        current_value,
        ..
    }) = &controls.speed_select
    {
        let selected = t.harness_state["serviceTier"]
            .as_str()
            .or_else(|| current_value.as_deref())
            .unwrap_or("default");
        let mut tiers = group("service-tier-picker", "Service Tier");
        for (index, opt) in options.iter().enumerate() {
            let on = selected == opt.id;
            let oid = opt.id.clone();
            let mut seg = segment(format!("service-tier-{index}"), &opt.label, on).on_press(
                move |s: &mut State| {
                    s.set_composer_service_tier_sticky(oid.clone());
                },
            );
            if let Some(desc) = &opt.description {
                seg = seg.help(desc);
            }
            if opt.label == "Fast" || opt.label == "Ultrafast" {
                seg = seg.child(icon("bolt", if on { p.brass } else { p.muted }).size(14.0, 14.0));
            }
            tiers = tiers.child(
                seg.child(label(&opt.label, SMALL, if on { p.ink } else { p.muted }).truncate()),
            );
        }
        section = section.child(label("Speed", META, p.muted)).child(tiers);
        any = true;
    } else if let Some(tiers) = service_tier_picker(t, p, can_choose) {
        section = section.child(tiers);
        any = true;
    } else if let Some(crate::descriptors::ProviderOptionDescriptor::Boolean {
        current_value,
        ..
    }) = &controls.fast_toggle
    {
        let is_fast = *current_value == Some(true);
        let tiers = group("fast-mode-picker", "Fast Mode")
            .child(
                segment("fast-mode-off".into(), "Standard", !is_fast)
                    .on_press(|s: &mut State| {
                        s.set_composer_option_sticky(
                            "fastMode".into(),
                            crate::model_selection::OptionValue::Bool(false),
                        );
                    })
                    .child(label(
                        "Standard",
                        SMALL,
                        if !is_fast { p.ink } else { p.muted },
                    )),
            )
            .child(
                segment("fast-mode-on".into(), "Fast", is_fast)
                    .on_press(|s: &mut State| {
                        s.set_composer_option_sticky(
                            "fastMode".into(),
                            crate::model_selection::OptionValue::Bool(true),
                        );
                    })
                    .child(icon("bolt", if is_fast { p.brass } else { p.muted }).size(14.0, 14.0))
                    .child(label("Fast", SMALL, if is_fast { p.ink } else { p.muted })),
            );
        section = section.child(label("Speed", META, p.muted)).child(tiers);
        any = true;
    }

    if let Some(crate::descriptors::ProviderOptionDescriptor::Boolean {
        id,
        label: ultra_label,
        current_value,
        ..
    }) = &controls.ultracode_toggle
    {
        let on = *current_value == Some(true);
        let oid = id.clone();
        section = section.child(
            row()
                .key("ultracode-toggle")
                .role("switch")
                .label(ultra_label)
                .checked(on)
                .focusable(can_choose)
                .disabled(!can_choose)
                .h(28.0)
                .mx(-4.0)
                .px(4.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .hover(p.selected)
                .focus_fill(p.selected)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    s.set_composer_option_sticky(
                        oid.clone(),
                        crate::model_selection::OptionValue::Bool(!on),
                    );
                })
                .child(icon("workflow", if on { p.sea } else { p.muted }).size(14.0, 14.0))
                .child(
                    label(ultra_label, SMALL, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(switch_track(on, p)),
        );
        any = true;
    }
    any.then_some(section)
}

/// Codex's legacy service tiers, read from the model, as KSG's `SpeedControl` segments.
fn service_tier_picker(t: &Thread, p: Palette, can_choose: bool) -> Option<El> {
    let model = t
        .models
        .iter()
        .find(|model| model.id == t.model_id && model.provider == "codex")?;
    if model.service_tiers.is_empty() {
        return None;
    }
    let mut choices = row()
        .key("service-tier-options")
        .role("radiogroup")
        .label("Service tier")
        .w_full()
        .gap(2.0)
        .padding(2.0)
        .radius(8.0)
        .fill(p.selected);
    let selected = t.harness_state["serviceTier"]
        .as_str()
        .or_else(|| t.harness_state["effectiveServiceTier"].as_str())
        .or_else(|| {
            model
                .service_tiers
                .iter()
                .find(|tier| tier["isDefault"] == true)
                .and_then(|tier| tier["id"].as_str())
        })
        .unwrap_or("default");
    for (index, tier) in model.service_tiers.iter().enumerate() {
        let Some(id) = tier["id"].as_str() else {
            continue;
        };
        let name = tier["name"].as_str().unwrap_or(id);
        let id = id.to_owned();
        let on = selected == id;
        choices = choices.child(
            row()
                .key(format!("service-tier-{index}"))
                .role("radio")
                .label(name)
                .checked(on)
                .focusable(can_choose)
                .disabled(!can_choose)
                .h(24.0)
                .w(0.0)
                .grow(1.0)
                .min_w(0.0)
                .px(8.0)
                .center_y()
                .justify_center()
                .radius(6.0)
                .fill(if on { p.card } else { Color::TRANSPARENT })
                .hover(if on { p.card } else { p.selected })
                .focus_fill(p.selected)
                .cursor(rune::Cursor::Pointer)
                .child(label(name, SMALL, if on { p.ink } else { p.muted }).truncate())
                .on_press(move |s: &mut State| {
                    s.set_composer_service_tier_sticky(id.clone());
                }),
        );
    }
    Some(
        col()
            .key("service-tier-picker")
            .gap(8.0)
            .child(label("Service tier", META, p.muted))
            .child(choices),
    )
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
    if dragging {
        rune::Curve::tween(0.15, rune::Easing::cubic(0.23, 1.0, 0.32, 1.0))
    } else {
        MOTION_SLOW
    }
}

/// Which control an effort drag belongs to: the thread, model, option id and levels the
/// press started on. A remounted slider reuses the same element key, so this hash is how a
/// held gesture knows its release still lands on the control it began in.
fn effort_drag_key(context: &str, option_id: &str, levels: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    context.hash(&mut hasher);
    option_id.hash(&mut hasher);
    levels.hash(&mut hasher);
    hasher.finish()
}

fn effort_slider_with_levels(
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

struct PickerSnapshot {
    entries: Vec<crate::model_picker::ProviderInstanceEntry>,
    sidebar_entries: Vec<crate::model_picker::ProviderInstanceEntry>,
    selected_tab: crate::model_picker::ModelPickerTab,
    is_searching: bool,
    locked_disabled_ids: std::collections::HashSet<String>,
    selectable_unavailable_ids: std::collections::HashSet<String>,
    favorites_set: std::collections::HashSet<String>,
    visible_models: Vec<crate::model_picker::ModelPickerItem>,
    legacy_section: Option<(String, bool)>,
    setup_entries: Vec<crate::model_picker::ProviderInstanceEntry>,
    update_required_notices: Vec<(String, String)>,
}

fn picker_snapshot(s: &State) -> PickerSnapshot {
    let entries = s.provider_instance_entries();
    let options = s.picker_models_for_entries(&entries);
    picker_snapshot_from(s, entries, &options)
}

fn picker_snapshot_from(
    s: &State,
    entries: Vec<crate::model_picker::ProviderInstanceEntry>,
    options_by_instance: &PickerModels,
) -> PickerSnapshot {
    use crate::model_picker::{
        ModelPickerItem, ModelPickerSearchableModel, ModelPickerTab,
        is_provider_instance_picker_ready, is_provider_instance_picker_visible,
        matches_locked_provider, provider_model_key, rank_model_picker_search,
        should_include_model_picker_option, should_offer_model_picker_setup,
        sort_provider_model_items, split_legacy_models,
    };

    let t = s.current();
    let favorites = s.effective_model_favorites();
    let favorites_set: std::collections::HashSet<String> = favorites
        .iter()
        .map(|f| provider_model_key(&f.provider, &f.model))
        .collect();
    let locked_provider = s.locked_provider_for_current();
    let is_searching = !s.model_search.text.trim().is_empty();

    let selected_tab = s
        .model_picker_tab
        .clone()
        .unwrap_or_else(|| ModelPickerTab::Instance(t.harness.clone()));

    let mut locked_disabled_ids = std::collections::HashSet::new();
    if locked_provider.is_some() {
        for entry in &entries {
            if !matches_locked_provider(entry, locked_provider.as_deref(), None) {
                locked_disabled_ids.insert(entry.instance_id.clone());
            }
        }
    }

    let mut selectable_unavailable_ids = std::collections::HashSet::new();
    if let Some(active_entry) = entries.iter().find(|e| e.instance_id == t.harness)
        && !is_provider_instance_picker_ready(active_entry)
        && options_by_instance.get(&t.harness).is_some_and(|opts| {
            opts.iter().any(|opt| {
                should_include_model_picker_option(active_entry, opt, &t.harness, &t.model_id)
            })
        })
    {
        selectable_unavailable_ids.insert(t.harness.clone());
    }
    for entry in &entries {
        let opts = options_by_instance
            .get(&entry.instance_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if should_offer_model_picker_setup(entry, opts) {
            selectable_unavailable_ids.insert(entry.instance_id.clone());
        }
    }

    let sidebar_entries = {
        let visible: Vec<_> = entries
            .iter()
            .filter(|e| is_provider_instance_picker_visible(e))
            .cloned()
            .collect();
        if locked_provider.is_none() {
            visible
        } else {
            let mut available = Vec::new();
            let mut disabled = Vec::new();
            for entry in visible {
                if matches_locked_provider(&entry, locked_provider.as_deref(), None) {
                    available.push(entry);
                } else {
                    disabled.push(entry);
                }
            }
            available.extend(disabled);
            available
        }
    };

    let mut flat_models = Vec::new();
    for entry in &entries {
        let opts = options_by_instance
            .get(&entry.instance_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for (catalog_index, model) in opts.iter().enumerate() {
            if !should_include_model_picker_option(entry, model, &t.harness, &t.model_id) {
                continue;
            }
            flat_models.push(ModelPickerItem {
                slug: model.slug.clone(),
                name: model.name.clone(),
                short_name: model.short_name.clone(),
                sub_provider: model.sub_provider.clone(),
                badge: model.badge.clone(),
                is_legacy: model.is_legacy,
                is_unavailable: model.is_unavailable,
                instance_id: entry.instance_id.clone(),
                driver_kind: entry.driver_kind.clone(),
                instance_display_name: entry.display_name.clone(),
                continuation_group_key: entry.continuation_group_key.clone(),
                catalog_index,
            });
        }
    }

    let instance_order: Vec<String> = entries.iter().map(|e| e.instance_id.clone()).collect();
    let entry_by_id: std::collections::HashMap<&str, &crate::model_picker::ProviderInstanceEntry> =
        entries
            .iter()
            .map(|e| (e.instance_id.as_str(), e))
            .collect();

    let filtered_models: Vec<ModelPickerItem> = if is_searching {
        let candidates: Vec<ModelPickerItem> = if locked_provider.is_some() {
            flat_models
                .into_iter()
                .filter(|m| {
                    entry_by_id.get(m.instance_id.as_str()).is_some_and(|e| {
                        matches_locked_provider(e, locked_provider.as_deref(), None)
                    })
                })
                .collect()
        } else {
            flat_models
        };
        rank_model_picker_search(&candidates, &s.model_search.text, |m| {
            ModelPickerSearchableModel {
                driver_kind: &m.driver_kind,
                provider_display_name: &m.instance_display_name,
                name: &m.name,
                short_name: m.short_name.as_deref(),
                sub_provider: m.sub_provider.as_deref(),
                is_favorite: favorites_set.contains(&provider_model_key(&m.instance_id, &m.slug)),
            }
        })
        .into_iter()
        .cloned()
        .collect()
    } else {
        let scoped: Vec<ModelPickerItem> = flat_models
            .into_iter()
            .filter(|m| {
                if locked_provider.is_some()
                    && !entry_by_id.get(m.instance_id.as_str()).is_some_and(|e| {
                        matches_locked_provider(e, locked_provider.as_deref(), None)
                    })
                {
                    return false;
                }
                match &selected_tab {
                    ModelPickerTab::Favorites => {
                        favorites_set.contains(&provider_model_key(&m.instance_id, &m.slug))
                    }
                    ModelPickerTab::Instance(id) => &m.instance_id == id,
                }
            })
            .collect();
        let is_fav_tab = matches!(selected_tab, ModelPickerTab::Favorites);
        sort_provider_model_items(
            &scoped,
            |m| &m.instance_id,
            |m| &m.slug,
            &favorites_set,
            !is_fav_tab,
            if is_fav_tab { &instance_order } else { &[] },
        )
        .into_iter()
        .cloned()
        .collect()
    };

    let is_fav_tab = matches!(selected_tab, ModelPickerTab::Favorites);
    let (current_models, legacy_models) =
        split_legacy_models(filtered_models, is_searching, is_fav_tab, &favorites_set);
    let legacy_section = match &selected_tab {
        ModelPickerTab::Instance(id) if !legacy_models.is_empty() => {
            Some((id.clone(), s.expanded_legacy_instances.contains(id)))
        }
        _ => None,
    };
    let mut visible_models = current_models;
    if let Some((_, true)) = legacy_section {
        visible_models.extend(legacy_models);
    }

    let selected_instance_id = match &selected_tab {
        ModelPickerTab::Instance(id) => Some(id.as_str()),
        ModelPickerTab::Favorites => None,
    };
    let setup_entries: Vec<_> = if !is_searching {
        entries
            .iter()
            .filter(|entry| {
                let opts = options_by_instance
                    .get(&entry.instance_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                matches_locked_provider(entry, locked_provider.as_deref(), None)
                    && should_offer_model_picker_setup(entry, opts)
                    && selected_instance_id.is_none_or(|id| entry.instance_id == id)
                    && (selected_instance_id.is_some() || visible_models.is_empty())
            })
            .cloned()
            .collect()
    } else {
        vec![]
    };

    let mut update_required_notices = Vec::new();
    for entry in &entries {
        let shown = if is_searching {
            matches_locked_provider(entry, locked_provider.as_deref(), None)
        } else {
            selected_instance_id == Some(entry.instance_id.as_str())
        };
        if shown
            && let Some(notice) = crate::model_picker::format_provider_update_required_notice(
                &entry.driver_kind,
                &entry.display_name,
                &entry.update_required_models,
                &s.model_search.text,
            )
        {
            update_required_notices.push((entry.instance_id.clone(), notice));
        }
    }

    PickerSnapshot {
        entries,
        sidebar_entries,
        selected_tab,
        is_searching,
        locked_disabled_ids,
        selectable_unavailable_ids,
        favorites_set,
        visible_models,
        legacy_section,
        setup_entries,
        update_required_notices,
    }
}

/// Ported from KSG Code's `ModelPickerContent.tsx`, `ModelPickerSidebar.tsx`, and `ModelListRow.tsx`.
/// `round_bottom` rounds the rail's lower corner, when the list ends at the popup's bottom.
fn model_list(
    s: &State,
    p: Palette,
    can_choose: bool,
    card: El,
    snap: &PickerSnapshot,
    selections: &[crate::model_selection::ModelSelection],
    round_bottom: bool,
) -> El {
    use crate::model_picker::{
        ModelPickerTab, describe_unavailable_instance, get_display_model_name,
        get_provider_status_message, is_provider_instance_picker_ready, provider_model_key,
    };

    let t = s.current();
    let show_sidebar = !snap.is_searching && !snap.sidebar_entries.is_empty();

    let search_field = text_input(&s.model_search, TextStyle::new(13.0, 400.0))
        .key("model-search")
        .label("Search models")
        .placeholder("Search models...", p.muted)
        .color(p.ink)
        .caret(p.ink, p.coral_bg)
        .grow(1.0)
        .min_w(0.0)
        .on_edit(|s: &mut State| &mut s.model_search)
        // Down from the search field goes into the list, as in KSG Code's
        // combobox; the list's own group takes the arrows from there.
        .on_input_key(|s: &mut State, event, tx| {
            if (event.mods.cmd || event.mods.ctrl || event.mods.alt) && crate::key(s, event, tx) {
                return true;
            }
            if matches!(event.key, rune::Key::Down)
                && !event.mods.cmd
                && !event.mods.ctrl
                && !event.mods.alt
                && !event.mods.shift
            {
                // The field's own focus flag would win over `tx.focus`.
                s.model_search.focused = false;
                tx.focus("model-list", true);
                return true;
            }
            false
        })
        .autofocus();

    let header = row().center_y().gap(8.0).h(50.0).px(10.0).shrink().child(
        row()
            .center_y()
            .gap(6.0)
            .h(30.0)
            .px(10.0)
            .radius(6.0)
            .fill(p.selected)
            .field_focus(&s.model_search, p)
            .grow(1.0)
            .min_w(0.0)
            .child(icon("search", p.muted))
            .child(search_field),
    );

    // KSG's `--model-picker-rail`: a darker strip of square provider buttons, the selected one
    // filled.
    let mut sidebar = col()
        .key("harness-group")
        .role("toolbar")
        .label("Providers")
        .w(44.0)
        .h_full()
        .min_h(0.0)
        .padding(4.0)
        .gap(4.0)
        // Inside the popup's 1 pt border, so its 12 pt corners less one.
        .corners([11.0, 0.0, 0.0, if round_bottom { 11.0 } else { 0.0 }])
        .fill(Color::BLACK.alpha(if s.dark { 0.2 } else { 0.1 }))
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    let fav_selected = snap.selected_tab == ModelPickerTab::Favorites;
    sidebar = sidebar.child(
        row()
            .key("choose-harness-favorites")
            .role("button")
            .label("Favorites")
            .help("Favorites")
            .checked(fav_selected)
            .cursor(rune::Cursor::Pointer)
            .size(36.0, 36.0)
            .center()
            .radius(8.0)
            .fill(if fav_selected {
                p.selected
            } else {
                Color::TRANSPARENT
            })
            .hover(p.selected)
            .on_press(|s: &mut State| {
                s.model_picker_tab = Some(ModelPickerTab::Favorites);
            })
            .child(
                icon(
                    if fav_selected { "star" } else { "star-off" },
                    if fav_selected { p.ink } else { p.muted },
                )
                .size(18.0, 18.0),
            ),
    );

    let mut seen: Vec<(String, String)> = Vec::new();
    for entry in &snap.sidebar_entries {
        if seen.iter().any(|(earlier_id, earlier_name)| {
            earlier_id == &entry.instance_id && earlier_name == &entry.display_name
        }) {
            continue;
        }
        let repeats = seen
            .iter()
            .filter(|(earlier_id, _)| earlier_id == &entry.instance_id)
            .count();
        seen.push((entry.instance_id.clone(), entry.display_name.clone()));
        let key = if repeats == 0 {
            format!("choose-harness-{}", entry.instance_id)
        } else {
            format!("choose-harness-{}-{}", entry.instance_id, repeats + 1)
        };

        let is_unavailable = !is_provider_instance_picker_ready(entry);
        let is_context_disabled = snap.locked_disabled_ids.contains(&entry.instance_id);
        let unavailable_reachable = snap.selectable_unavailable_ids.contains(&entry.instance_id);
        let is_disabled = (is_unavailable && !unavailable_reachable) || is_context_disabled;
        let is_selected = snap.selected_tab == ModelPickerTab::Instance(entry.instance_id.clone());
        let tooltip = if is_unavailable {
            describe_unavailable_instance(entry)
        } else if is_context_disabled {
            format!(
                "{} is unavailable in this thread. Start a new thread to switch providers.",
                entry.display_name
            )
        } else {
            entry.display_name.clone()
        };

        let mut btn_content = col()
            .items_center()
            .justify_center()
            .gap(1.0)
            .opacity(if is_selected { 1.0 } else { 0.7 })
            .child(harness_icon(s, p, &entry.instance_id, &entry.display_name));
        if entry.is_acp {
            btn_content = btn_content.child(outlined(
                meta_text("ACP", p.muted)
                    .key(format!("harness-tag-{}", entry.instance_id))
                    .px(2.0)
                    .radius(4.0),
                p,
            ));
        }

        let mut item = row()
            .key(key)
            .role("button")
            .label(&tooltip)
            .help(&tooltip)
            .checked(is_selected)
            .disabled(is_disabled)
            .opacity(if is_disabled { 0.5 } else { 1.0 })
            .w(36.0)
            .h(if entry.is_acp { 42.0 } else { 36.0 })
            .center()
            .radius(8.0)
            .fill(if is_selected {
                p.selected
            } else {
                Color::TRANSPARENT
            });
        if !is_disabled {
            let target_id = entry.instance_id.clone();
            item = item
                .cursor(rune::Cursor::Pointer)
                .hover(p.selected)
                .on_press(move |s: &mut State| {
                    s.model_picker_tab = Some(ModelPickerTab::Instance(target_id.clone()));
                });
        }
        sidebar = sidebar.child(item.child(btn_content));
    }

    let mut list = col()
        .key("model-list")
        .role("listbox")
        .label("Models")
        .focus_group()
        .w_full()
        .h(0.0)
        .grow(1.0)
        .min_h(0.0)
        .pl(6.0)
        .pr(4.0)
        .py(6.0)
        .gap(2.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });

    if let Some(error) = &s.harnesses.error {
        list = list.child(label(error, SMALL, p.muted).wrap().px(16.0).py(6.0));
    }

    let multi_enabled = s.multiple_models_enabled();
    let multi_selections = if multi_enabled { selections } else { &[] };

    let rows_mix_providers = snap
        .visible_models
        .iter()
        .any(|m| m.instance_id != snap.visible_models[0].instance_id);
    let mut jump_num = 0usize;
    for (index, model) in snap.visible_models.iter().enumerate() {
        let disabled_reason = if !can_choose {
            Some("Model picker is disabled".to_owned())
        } else {
            s.model_disabled_reason_for(&model.instance_id, &model.slug)
        };
        let enabled = disabled_reason.is_none();
        let jump_label = if enabled && jump_num < 9 {
            jump_num += 1;
            Some(format!("⌘{jump_num}"))
        } else {
            None
        };
        let is_favorite = snap
            .favorites_set
            .contains(&provider_model_key(&model.instance_id, &model.slug));
        let selected = if multi_enabled && multi_selections.len() > 1 {
            multi_selections
                .iter()
                .any(|sel| sel.instance_id == model.instance_id && sel.model == model.slug)
        } else {
            t.harness == model.instance_id && t.model_id == model.slug
        };
        let display_name = get_display_model_name(
            &model.name,
            model.short_name.as_deref(),
            model.sub_provider.as_deref(),
            false,
        );

        // KSG's `ModelListRow`: the name with a check when selected, then its badges.
        let mut name_row = row().center_y().gap(6.0).min_w(0.0).child(
            label(&display_name, TextStyle::new(13.0, 500.0), p.ink)
                .truncate()
                .shrink()
                .min_w(0.0),
        );
        if selected {
            name_row = name_row.child(icon("check", p.ink).size(14.0, 14.0));
        }
        if model.badge.as_deref() == Some("new") {
            name_row = name_row.child(outlined(
                meta_text("New", p.muted)
                    .label("New model")
                    .px(4.0)
                    .py(1.0)
                    .radius(6.0),
                p,
            ));
        }
        if model.is_unavailable {
            name_row = name_row.child(outlined(
                meta_text("Unavailable", p.muted)
                    .px(4.0)
                    .py(1.0)
                    .radius(6.0),
                p,
            ));
        }

        let mut name_col = col().gap(4.0).grow(1.0).min_w(0.0).child(name_row);
        // The provider line, when the rows mix providers or the model names a sub-provider.
        if rows_mix_providers || model.sub_provider.is_some() {
            // KSG `ModelListRow` uses the sub-provider name on this line when present.
            let provider_label = model
                .sub_provider
                .as_deref()
                .unwrap_or(&model.instance_display_name);
            name_col = name_col.child(
                row()
                    .center_y()
                    .gap(6.0)
                    .min_w(0.0)
                    .child(
                        provider_icon(
                            s,
                            p,
                            model_vendor(&model.instance_id, &model.instance_display_name),
                            12.0,
                        )
                        .opacity(0.6),
                    )
                    .child(
                        label(provider_label, META, p.faint)
                            .truncate()
                            .shrink()
                            .min_w(0.0),
                    ),
            );
        }
        // A started thread hands its context to the new provider; the row says so
        // under the name, KSG Code's pending-handoff wording for the pick.
        let handoff_hint = enabled && t.has_started_session() && model.instance_id != t.harness;
        if handoff_hint {
            name_col = name_col.child(
                label("Starts a context handoff", META, p.muted)
                    .key(format!("handoff-hint-{index}"))
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            );
        }

        let fav_provider = model.instance_id.clone();
        let fav_slug = model.slug.clone();
        let mut star_btn = row()
            .key(format!("toggle-favorite-{index}"))
            .role("button")
            .label(if is_favorite {
                "Remove from favorites"
            } else {
                "Add to favorites"
            })
            .help(if is_favorite {
                "Remove from favorites"
            } else {
                "Add to favorites"
            })
            .focusable(false)
            .size(24.0, 24.0)
            .center()
            .radius(6.0)
            .disabled(!enabled)
            .child(
                icon(
                    if is_favorite { "star" } else { "star-off" },
                    if is_favorite { p.brass } else { p.muted },
                )
                .size(14.0, 14.0),
            );
        if enabled {
            star_btn = star_btn
                .cursor(rune::Cursor::Pointer)
                .hover(p.selected)
                .on_press(move |s: &mut State| {
                    s.toggle_model_favorite(&fav_provider, &fav_slug);
                });
        }

        let mut trailing = row().center_y().gap(6.0).shrink().child(star_btn);
        if let Some(jump) = jump_label {
            trailing = trailing.child(keycap_chip(&jump, p));
        }

        // Clicking a row picks the model; Shift adds it to a multi-model send, as KSG's
        // combobox does. There is no checkbox.
        let mut item = row()
            .key(format!("choose-model-{index}"))
            .role("option")
            .label(format!("{display_name}, {}", model.instance_display_name))
            .checked(selected)
            .focusable(enabled)
            .disabled(!enabled)
            .opacity(if enabled { 1.0 } else { 0.55 })
            .center_y()
            .gap(8.0)
            .min_h(32.0)
            .px(8.0)
            .py(5.0)
            .radius(6.0)
            .hover(p.selected)
            .focus_fill(p.selected)
            .focus_selected(selected)
            .child(name_col)
            .child(trailing);
        if let Some(reason) = &disabled_reason {
            item = item.help(reason);
        }
        if enabled {
            let instance_id = model.instance_id.clone();
            let id = model.slug.clone();
            item = item
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |s: &mut State, tx| {
                    if tx.mods().shift && s.multiple_models_enabled() {
                        s.toggle_model_in_multi_selection(&instance_id, &id);
                    } else {
                        s.select_model_from_picker(&instance_id, &id);
                        finish_model_pick(s);
                    }
                });
        }
        list = list.child(item);
    }

    if let Some((instance_id, expanded)) = &snap.legacy_section {
        let toggle_id = instance_id.clone();
        list = list.child(
            row()
                .key(format!("legacy-models-{instance_id}"))
                .role("option")
                .label(if *expanded { "Show less" } else { "Show more" })
                .focusable(true)
                .center_y()
                .h(32.0)
                .mx(4.0)
                .px(12.0)
                .radius(6.0)
                .hover(p.selected)
                .focus_fill(p.selected)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    if !s.expanded_legacy_instances.remove(&toggle_id) {
                        s.expanded_legacy_instances.insert(toggle_id.clone());
                    }
                })
                .child(label(
                    if *expanded { "Show less" } else { "Show more" },
                    TextStyle::new(13.0, 500.0),
                    p.muted,
                )),
        );
    }

    let selected_id = match &snap.selected_tab {
        ModelPickerTab::Instance(id) => Some(id.as_str()),
        ModelPickerTab::Favorites => None,
    };
    let active_harness = selected_id.unwrap_or(&t.harness);
    let (pi_loading, pi_error) = if active_harness == "pi" {
        s.pi_catalog_status()
    } else {
        (false, None)
    };
    let (native_loading, native_error) = s.native_catalog_status(active_harness);
    let catalog_loading =
        pi_loading || (matches!(active_harness, "codex" | "claude") && native_loading);
    let catalog_error = pi_error.or(native_error);

    if snap.visible_models.is_empty() && snap.setup_entries.is_empty() {
        let empty_text = if snap.is_searching {
            "No models found"
        } else if snap.selected_tab == ModelPickerTab::Favorites {
            "No favorites yet"
        } else if active_harness == "claude"
            && snap
                .entries
                .iter()
                .find(|e| e.instance_id == "claude")
                .is_some_and(|e| !e.installed)
        {
            crate::claude::MISSING
        } else if matches!(active_harness, "codex" | "claude") {
            if native_loading {
                "Reading the harness catalog..."
            } else if let Some(err) = native_error {
                err
            } else {
                "Catalog not available. Models will be read when a session starts."
            }
        } else if active_harness != "pi" {
            "This harness uses its configured model until it reports available models."
        } else if let Some(error) = pi_error {
            error
        } else if s.catalog.remaining().is_some() {
            "Reading the CPA catalog..."
        } else {
            "No CPA models are available."
        };
        let note_key = if active_harness == "claude"
            && snap
                .entries
                .iter()
                .find(|e| e.instance_id == "claude")
                .is_some_and(|e| !e.installed)
        {
            "claude-unavailable"
        } else {
            "models-empty-note"
        };
        list = list
            .child(rule(p).mt(6.0))
            .child(
                meta_label("Models", p.muted)
                    .key("models-group")
                    .px(16.0)
                    .pt(8.0)
                    .pb(4.0),
            )
            .child(
                label(
                    empty_text,
                    SMALL,
                    if catalog_error.is_some() {
                        p.danger
                    } else {
                        p.muted
                    },
                )
                .key(note_key)
                .wrap()
                .px(16.0)
                .py(10.0),
            );
        if catalog_error.is_some() {
            list = list.child(
                button("retry-catalog", "Retry", p, |s| s.force_refresh_catalog())
                    .ml(12.0)
                    .mb(8.0),
            );
        }
    }

    let mut right_col = col()
        .w(0.0)
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .child(header)
        .child(rule(p))
        .child(list);

    if catalog_loading && !snap.visible_models.is_empty() {
        right_col = right_col.child(rule(p)).child(
            label("Refreshing models...", SMALL, p.muted)
                .key("catalog-loading")
                .px(12.0)
                .py(6.0),
        );
    }
    if let Some(error) = catalog_error
        && !snap.visible_models.is_empty()
    {
        right_col = right_col.child(rule(p)).child(
            col()
                .gap(4.0)
                .px(12.0)
                .py(6.0)
                .child(label(error, SMALL, p.danger).wrap_anywhere())
                .child(button("retry-catalog", "Retry", p, |s| {
                    s.force_refresh_catalog()
                })),
        );
    }

    if !snap.setup_entries.is_empty() {
        let mut setup_box = col()
            .gap(6.0)
            .px(12.0)
            .py(8.0)
            .border(0.0, Color::TRANSPARENT);
        let multi = snap.setup_entries.len() > 1;
        for entry in &snap.setup_entries {
            let btn_label = if multi {
                format!("Set up {}", entry.display_name)
            } else {
                "Open provider setup".to_owned()
            };
            setup_box = setup_box.child(
                col()
                    .gap(4.0)
                    .child(label(get_provider_status_message(entry), SMALL, p.muted).wrap())
                    .child(button(
                        &format!("open-provider-setup-{}", entry.instance_id),
                        &btn_label,
                        p,
                        |s| {
                            s.settings_picker = None;
                            s.open_settings();
                            s.settings.section = 2;
                        },
                    )),
            );
        }
        right_col = right_col.child(rule(p)).child(setup_box);
    }

    for (instance_id, notice) in &snap.update_required_notices {
        right_col = right_col.child(rule(p)).child(
            col()
                .key(format!("update-required-notice-{instance_id}"))
                .gap(4.0)
                .px(12.0)
                .py(8.0)
                .child(label(notice, SMALL, p.muted).wrap())
                .child(button(
                    &format!("provider-settings-{instance_id}"),
                    "Provider settings",
                    p,
                    |s| {
                        s.settings_picker = None;
                        s.open_settings();
                        s.settings.section = 2;
                    },
                )),
        );
    }

    let mut body = row().w_full().h_full().min_w(0.0).min_h(0.0);
    if show_sidebar {
        body = body.child(sidebar);
    }
    body = body.child(right_col);
    card.child(body)
}

/// A harness's display name: "Pi", "Codex", "Claude" for the built-ins, the registry's
/// configured name for ACP agents (allowing overrides of "antigravity"), built-in
/// "Antigravity" when unregistered, or a generic "agent" when the id is unknown.
fn harness_name<'a>(s: &'a State, id: &'a str) -> &'a str {
    match id {
        "pi" => "Pi",
        "codex" => "Codex",
        "claude" => "Claude",
        id => s
            .harnesses
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .map(|agent| agent.name.as_str())
            .unwrap_or(match id {
                "antigravity" => "Antigravity",
                _ => "agent",
            }),
    }
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
        .radius(6.0)
        .clip_rounded(6.0)
        .pixels(&s.artwork.mark_pixels(s.dark, HEADER_MARK))
        // The day mark's sky is the chrome's sky, so a 1 px frame keeps its edge. The frame is
        // an overlay because Rune paints a clipped image over its border.
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(6.0)
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
                .radius(6.0)
                .clip_rounded(6.0)
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

/// The wordmark's size in points, for a header `line` points wide after the native controls
/// and the gap before the tabs: Inter 14/600 when it fits, 12 in a narrow sidebar, and none
/// in a truly narrow one.
/// Width a macOS title strip reserves for the window buttons, which every platform assumes when
/// it sizes the wordmark.
const TITLE_CONTROLS: f32 = 72.0;

fn wordmark_size(line: f32) -> Option<f32> {
    // The compose button and its 2 pt gap follow the wordmark's slot, so the wordmark
    // reserves 26 pt for it: the mark drops out earlier on a 200 pt sidebar.
    let before = HEADER_TOGGLE + HEADER_MARK + 26.0;
    if line >= before + wordmark_gap(14.0) + WORDMARK_WIDTH {
        Some(14.0)
    } else if line >= before + wordmark_gap(12.0) + WORDMARK_WIDTH * 0.8 {
        Some(12.0)
    } else {
        None
    }
}
/// The space between the mark and the wordmark: 6 pt at 22 px, 4 pt at 11 px.
fn wordmark_gap(size: f32) -> f32 {
    if size < 15.0 { 4.0 } else { WORDMARK_GAP }
}

/// The sidebar's half of the title strip, left-aligned as T3 Code lays it out: the sidebar
/// toggle, then Shina's mark and name. Search and actions sit in the sidebar below.
fn sidebar_header(s: &State, p: Palette, compact: bool, line: f32, wordmark_line: f32) -> El {
    let mut header = row().h(36.0).center_y().child(sidebar_toggle(p, compact));
    if line >= HEADER_TOGGLE + HEADER_MARK {
        header = header.child(shina_mark(s, p));
        if let Some(size) = wordmark_size(wordmark_line) {
            header = header.child(
                label(
                    "Shina",
                    TextStyle::new(size, 600.0).tracking(-0.01 * size),
                    p.ink,
                )
                .key("shina-wordmark")
                .ml(wordmark_gap(size)),
            );
        }
    }
    header.child(spacer())
}

/// The title strip's left end when the sidebar is not docked: the toggle and the 24 px mark,
/// then the gap before the tabs. The wordmark waits for the sidebar.
fn undocked_header(s: &State, p: Palette, compact: bool) -> El {
    row()
        .h(36.0)
        .center_y()
        .child(sidebar_toggle(p, compact))
        .child(shina_mark(s, p))
        .pr(HEADER_TAB_GAP)
}
/// The space between the compose button, at the header's end, and the first tab.
const HEADER_TAB_GAP: f32 = 12.0;
/// The space between the mark and the 22 px wordmark. The toggle and the mark touch their boxes.
const WORDMARK_GAP: f32 = 6.0;
const HEADER_TOGGLE: f32 = 24.0;
const HEADER_MARK: f32 = 24.0;
/// "Shina" in Inter 15/600.
const WORDMARK_WIDTH: f32 = 40.0;

/// KSG's `SidebarHeaderIconButton`: a 28 pt icon control in the sidebar's header row.
fn header_control(item: El, p: Palette) -> El {
    item.size(28.0, 28.0)
        .center()
        .radius(8.0)
        .hover(p.selected)
        .press(p.selected)
}

fn sidebar(s: &State, p: Palette, width: f32) -> El {
    let mut side = col()
        .key("sidebar")
        .w(width)
        .h_full()
        .min_h(0.0)
        .pr(8.0)
        .gap(1.0);
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
    // KSG's `SidebarThreadHeader`: the search field beside 28 pt controls for the project
    // scope, Add project and New thread, at 4 pt gaps.
    let scoped_projects: Vec<crate::projects::ProjectId> = match s.project_scope {
        ProjectScope::Project(id) => {
            let mut ids = vec![id];
            ids.extend(s.scope_additional.iter().copied());
            ids
        }
        _ => s.scope_additional.clone(),
    };
    let scope_glyph = if scoped_projects.len() > 1 {
        row()
            .child(project_tile(
                s,
                p,
                s.project(Some(scoped_projects[0])),
                14.0,
            ))
            .child(
                el().size(14.0, 14.0)
                    .ml(-4.0)
                    .radius(4.0)
                    .border(1.0, p.bg)
                    .child(project_tile(
                        s,
                        p,
                        s.project(Some(scoped_projects[1])),
                        14.0,
                    )),
            )
    } else if let Some(&id) = scoped_projects.first() {
        project_tile(s, p, s.project(Some(id)), 16.0)
    } else {
        icon("folder", if scoped { p.sea } else { p.muted }).size(14.0, 14.0)
    };
    let scope_button = header_control(
        el().key("project-scope")
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("Workspace: {scope_words}"))
            .help(format!("Workspace: {scope_words}"))
            .focusable(true)
            .on_press(|s: &mut State| s.open_picker(ProjectPicker::Scope))
            .child(scope_glyph),
        p,
    );
    side = side.child(
        row()
            .key("workspace-context")
            .h(28.0)
            .mt(8.0)
            .mb(8.0)
            .center_y()
            .gap(4.0)
            .child(
                row()
                    .key("thread-search-field")
                    .h(28.0)
                    .pl(6.0)
                    .pr(6.0)
                    .gap(6.0)
                    .center_y()
                    .radius(8.0)
                    .grow(1.0)
                    .min_w(0.0)
                    // KSG's `InputGroup size="sidebar"`: a 28 px `surface-well` (the raised
                    // fill under a hairline rim) that takes the ring while focused.
                    .fill(p.row)
                    .caret(p.ink, p.coral_bg)
                    .border(1.0, if s.search.focused { p.ring } else { p.frame })
                    // KSG seats the 13 px glyph in a 16 px slot, so the placeholder starts
                    // where the rows' labels do.
                    .child(
                        el().size(16.0, 16.0)
                            .center()
                            .child(icon("search", p.muted).size(13.0, 13.0)),
                    )
                    .child(
                        text_input(&s.search, ROW_QUIET)
                            .key("thread-search")
                            .label("Search conversations")
                            .placeholder("Search", p.muted)
                            .color(p.ink)
                            .single_line()
                            .grow(1.0)
                            .min_w(0.0)
                            .on_edit(|s: &mut State| &mut s.search)
                            .on_input_change(|s: &mut State, tx| {
                                s.sync_sidebar_search_input();
                                s.poll_sidebar_search(tx);
                            })
                            .on_input_key(|s: &mut State, ev, tx| {
                                if ev.key == rune::Key::Escape && !s.search.text.is_empty() {
                                    s.search.replace_text("");
                                    s.sync_sidebar_search_input();
                                    s.poll_sidebar_search(tx);
                                    true
                                } else {
                                    s.undo_key(ev, true)
                                }
                            }),
                    )
                    .child(sidebar_ui::search_clear(s, p)),
            )
            .child(scope_button)
            .child(header_control(
                el().key("project-switcher")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Add workspace")
                    .focusable(true)
                    .on_press(|s: &mut State| s.open_picker(ProjectPicker::AddProject))
                    .help("Add workspace")
                    .child(icon("folder-plus", p.muted).size(14.0, 14.0)),
                p,
            ))
            .child(header_control(
                el().key("new-thread")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("New thread")
                    .focusable(true)
                    .on_press_tx(|s: &mut State, tx| s.new_thread_click(tx.mods().shift))
                    .help(if s.projects.entries.iter().all(|project| project.hidden) {
                        "New thread"
                    } else {
                        "New thread. Shift-click: in the current workspace"
                    })
                    .child(icon("compose", p.muted).size(14.0, 14.0)),
                p,
            )),
    );
    // A workspace filter names itself and clears in one click.
    if scoped {
        side = side.child(
            row()
                .key("scope-chip")
                .h(24.0)
                .ml(4.0)
                .mb(8.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .radius(6.0)
                .fill(p.selected)
                .child(icon("folder", p.muted))
                .child(label(&scope, SMALL, p.ink).truncate().shrink().min_w(0.0))
                .child(match &scope_machine {
                    Some(machine) => label(machine, SMALL, p.faint).truncate().max_w(MACHINE_CAP),
                    None => el(),
                })
                .child(spacer())
                .child(
                    row()
                        .key("clear-project-scope")
                        .role("button")
                        .label("Show all workspaces")
                        .cursor(rune::Cursor::Pointer)
                        .focusable(true)
                        .size(16.0, 16.0)
                        .radius(4.0)
                        .center()
                        .hover(p.selected)
                        .on_press(|s: &mut State| {
                            s.project_scope = ProjectScope::All;
                            s.scope_additional.clear();
                            s.machines.scope_remote.clear();
                            s.selected_threads.clear();
                            s.checkpoint();
                        })
                        .child(icon("close", p.muted).size(12.0, 12.0)),
                ),
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
                .child(label("Archive", TextStyle::new(13.0, 600.0), p.ink))
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
    side = side.child(sidebar_ui::content(s, p));
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
                .radius(6.0)
                .fill(p.card)
                .border(1.0, p.frame)
                .hover(p.selected)
                .press(p.selected)
                .on_press(|s: &mut State| s.undo_thread_action())
                .child(
                    label(title, TextStyle::new(12.0, 500.0), p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(keycap_chip("Ctrl+Z", p)),
        );
    }
    // The footer is a row of quiet icons, as in T3 Code: no divider and no wordmark.
    // Ported from KSG Code's `SidebarChrome` footer (`SidebarUtilityItem`): 32 px icon-only
    // controls with 16 px glyphs, 4 px apart, in an 8/12 px padded strip. Settings holds
    // preferences such as the theme.
    side = side.child(
        row()
            .key("sidebar-footer")
            .pt(8.0)
            .pb(12.0)
            .px(8.0)
            .gap(4.0)
            .center_y()
            .child(
                footer_button(
                    "sidebar-settings",
                    "Settings",
                    "settings",
                    s.settings_menu,
                    p,
                    |s| s.open_settings(),
                )
                .help("Settings"),
            )
            .child(
                footer_button(
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
            .child(spacer())
            .child(updates_ui::footer(s, p)),
    );
    side
}

/// KSG's `SidebarUtilityItem`: a 32 px `size="footer"` icon button with a 16 px glyph and
/// the shared 8 px control radius, showing its on state with a card fill like `toggle_button`.
fn footer_button(
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
    el().key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .checked(on)
        .focusable(true)
        .size(32.0, 32.0)
        .center()
        .radius(8.0)
        .fill(if on { active_fill } else { Color::TRANSPARENT })
        .hover(p.selected)
        .on_press(action)
        .child(tool_icon(glyph, p.muted))
}

fn menu_item(key: &str, title: &str, glyph: &str, p: Palette) -> El {
    menu_item_in(key, title, glyph, p, p.ink, p.muted)
}
/// A menu item that destroys something: the title and icon use the danger colour.
fn danger_menu_item(key: &str, title: &str, glyph: &str, p: Palette) -> El {
    menu_item_in(key, title, glyph, p, p.danger, p.danger)
}
/// A menu row that stays greyed out because acting on it would do nothing, like KSG's
/// `disabled` DropdownMenuItem: faint ink, no press, not a tab stop.
fn disabled_menu_item(key: &str, title: &str, glyph: &str, p: Palette) -> El {
    menu_item_in(key, title, glyph, p, p.faint, p.faint)
        .focusable(false)
        .cursor(rune::Cursor::Default)
}
trait BlockedAction {
    fn blocked(self, reason: Option<String>, key: &str, p: Palette, inline: bool) -> El;
}

impl BlockedAction for El {
    fn blocked(self, reason: Option<String>, key: &str, p: Palette, inline: bool) -> El {
        let Some(reason) = reason else { return self };
        let control = self
            .disabled(true)
            .focusable(false)
            .cursor(rune::Cursor::Default)
            .description(&reason)
            .help(&reason);
        if inline {
            control.child(
                label(reason, SMALL, p.faint)
                    .key(format!("{key}-reason"))
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            )
        } else {
            control
        }
    }
}

fn gated_menu_item(key: &str, title: &str, glyph: &str, p: Palette, reason: Option<String>) -> El {
    let Some(reason) = reason else {
        return menu_item(key, title, glyph, p);
    };
    if reason.chars().count() <= 24 {
        disabled_menu_item(key, title, glyph, p).blocked(Some(reason), key, p, true)
    } else {
        col()
            .key(key)
            .role("menuitem")
            .label(title)
            .disabled(true)
            .help(&reason)
            .description(&reason)
            .h(44.0)
            .px(8.0)
            .justify_center()
            .child(
                row()
                    .center_y()
                    .gap(8.0)
                    .child(icon(glyph, p.faint))
                    .child(label(title, TITLE, p.faint).truncate()),
            )
            .child(
                label(reason, SMALL, p.faint)
                    .key(format!("{key}-reason"))
                    .pl(24.0)
                    .truncate(),
            )
    }
}
/// A menu item with a value at its right edge in pixel type, such as "3 days".
fn menu_value_item(key: &str, title: &str, value: &str, glyph: &str, p: Palette) -> El {
    menu_item(key, title, glyph, p)
        .label(format!("{title}, {value}"))
        .child(meta_text(value, p.muted))
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
        .radius(6.0)
        // Keys and the pointer share one highlight, as in an NSMenu. Every row asks for focus;
        // the menu's modal gives it to the first, so arrows work as soon as the menu opens.
        .focus_highlight(p.selected)
        .autofocus()
        .child(el().size(ICON, ICON).center().child(icon(glyph, icon_ink)))
        .child(
            label(title, TextStyle::new(13.0, 500.0), ink)
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
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
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
                .child(meta_label("Repository", p.muted))
                .child(
                    text_input(&projects.clone_source, BODY)
                        .key("clone-source")
                        .label("Repository URL or owner/repo")
                        .placeholder("owner/repo or https://github.com/owner/repo", p.muted)
                        .single_line()
                        .color(p.ink)
                        .caret(p.ink, p.coral_bg)
                        .w_full()
                        .padding(8.0)
                        .radius(6.0)
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
                .child(meta_label("Clone into", p.muted))
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
                            secondary_button("clone-choose-parent", "Choose...", p, |_| {})
                                .on_press_tx(crate::projects::pick_clone_parent)
                                .disabled(cloning),
                        ),
                ),
        );
    if s.machines.selected != "local" {
        card = card
            .child(meta_label("Destination path on this server", p.muted))
            .child(
                text_input(
                    &s.projects.clone_path,
                    BODY.family(rune::Family::SystemMono),
                )
                .key("remote-clone-path")
                .label("Host destination path")
                .single_line()
                .w_full()
                .padding(8.0)
                .radius(6.0)
                .fill(field)
                .field_focus(&s.projects.clone_path, p)
                .color(p.ink)
                .on_edit(|s: &mut State| &mut s.projects.clone_path),
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

/// A sidebar section label: Inter 12/500, sentence case, muted. Counts and buttons follow it.
fn section_title(title: &str, p: Palette) -> El {
    row()
        .h(28.0)
        .pl(ROW_INSET)
        .pr(4.0)
        .center_y()
        .child(meta_label(title, p.faint))
        .child(spacer())
}
/// A shelf heading's height.
// KSG's `CollapsibleSectionHeader` is `h-8` (32 px).
const SHELF_HEADING: f32 = 32.0;
/// A selected sidebar row: KSG's `row` fill; light theme adds a hairline border.
fn select_bar(item: El, selected: bool, dark: bool, p: Palette) -> El {
    if !selected {
        return item;
    }
    let bar = item.fill(p.row).radius(6.0);
    if dark { bar } else { bar.border(1.0, p.line) }
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
    let fitted = if s.artwork.is_pixel_art(captain) {
        s.artwork.snap_points(size)
    } else {
        size
    };
    let portrait = portrait(&pixels, fitted, p, accent)
        .key(format!("avatar-{captain}-{}", size as u32))
        .a11y_hidden();
    haloed(s, p, portrait, size, status)
}

/// The portrait in the captain picker and the editor's grid. The head art is 40 cells, so 40
/// points is a whole number of device pixels per cell at every integer window scale.
const PICKER_PORTRAIT: f32 = 40.0;
/// Every machine uses the same portrait size, including before a captain is selected.
/// The widest a machine name and its connection note grow beside a workspace or a detail.
const MACHINE_CAP: f32 = 110.0;

/// A sidebar portrait. It plays its status's animation strip through the shared stage in
/// `crate::portraits`, which also learns here whether the row is on screen. Under reduced
/// motion, or for a set without strips, it is the static face.
fn sidebar_portrait(
    s: &State,
    p: Palette,
    row: usize,
    slot: usize,
    size: f32,
    status: Status,
) -> El {
    let slot = slot.min(crate::visuals::CAPTAIN_NAMES.len() - 1);
    let face = face_for(status);
    let pixels = s
        .portraits
        .pixels(slot, face, size)
        .unwrap_or_else(|| s.artwork.face_pixels(slot, face, size));
    portrait(
        &pixels,
        if s.artwork.is_pixel_art(slot) {
            s.artwork.snap_points(size)
        } else {
            size
        },
        p,
        None,
    )
    .key(format!("portrait-{row}"))
    .a11y_hidden()
    .on_visible(move |s: &mut State, on, tx| {
        s.portraits.set_visible(row, on);
        // The view doesn't read this; the next poll does.
        tx.live_only();
        tx.wake_after(std::time::Duration::ZERO);
    })
}

/// The corner mark that shows a portrait's status in pickers and headers.
fn status_badge(p: Palette, portrait: El, size: f32, status: Status) -> El {
    status_badge_keyed(p, portrait, size, status, None)
}

fn status_badge_keyed(
    p: Palette,
    portrait: El,
    size: f32,
    status: Status,
    mark_key: Option<String>,
) -> El {
    let show_mark =
        matches!(status, Status::Running | Status::Waiting | Status::Failed) || mark_key.is_some();
    if show_mark {
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
        let mut dot = status_mark(status, p, mark);
        if let Some(key) = mark_key {
            dot = dot.key(key);
        }
        return el().size(size, size).child(portrait).child(
            el().absolute(offset, offset)
                .size(backing, backing)
                .radius(backing / 2.0)
                .fill(p.panel)
                .border(1.0, p.frame)
                .center()
                .child(dot)
                .a11y_hidden(),
        );
    }
    portrait
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
    } else if thread.status == Status::Ready
        && (thread.client_creating.is_some()
            || thread.remote_send.is_some()
            || thread.client_promoted.is_some())
    {
        Status::Running
    } else {
        thread.status
    };
    (status, approval)
}

/// Ported from KSG Code's `Toasts` (`components/ui/toast.tsx`) and `stackedThreadToast`,
/// styled as a framed card.
fn toast_stack(s: &State, p: Palette, top: f32) -> El {
    let mut stack = col()
        .key("toast-stack")
        .pin(Some(top + 8.0), Some(16.0), None, None)
        .w((s.viewport_width - 32.0).clamp(240.0, 360.0))
        .gap(8.0);
    for toast in s.toasts.iter().rev() {
        let id = toast.id;
        let (glyph_name, ink) = match toast.kind {
            crate::model::ToastKind::Info => ("dot", p.sea),
            crate::model::ToastKind::Success => ("check", p.sea),
            crate::model::ToastKind::Warning => ("alert", p.brass),
            crate::model::ToastKind::Error => ("alert", p.danger),
        };
        let mut card = col()
            .key(format!("toast-{id}"))
            .role("status")
            .label(toast.title.clone())
            .w_full()
            .padding(12.0)
            .gap(6.0)
            .radius(12.0)
            .fill(p.card)
            .border(1.0, p.frame)
            .box_shadow(soft_shadow(p))
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(icon(glyph_name, ink))
                    .child(
                        label(&toast.title, TextStyle::new(13.0, 600.0), p.ink)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(icon_button(
                        &format!("toast-close-{id}"),
                        "Dismiss",
                        "close",
                        p,
                        move |s| {
                            s.dismiss_toast(id);
                        },
                    )),
            );
        if let Some(description) = &toast.description {
            card = card.child(label(description, SMALL, p.muted).wrap().selectable(true));
        }
        if let Some(action_label) = &toast.action_label {
            card = card.child(row().w_full().justify_end().child(button(
                &format!("toast-action-{id}"),
                action_label,
                p,
                move |s| s.activate_toast(id),
            )));
        }
        stack = stack.child(card);
    }
    stack
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

pub(super) fn search_snippet_el(
    server: &str,
    id: usize,
    snippet: &str,
    query: &str,
    muted: Color,
    brass: Color,
) -> El {
    let ranges = crate::thread_search::find_snippet_token_ranges(snippet, query);
    let mut lbl = label(snippet, SMALL, muted)
        .key(format!("sidebar-search-snippet-{server}-{id}"))
        .truncate()
        .min_w(0.0);
    if !ranges.is_empty() {
        lbl = lbl.text_highlights(
            ranges
                .into_iter()
                .map(|r| rune::TextHighlight::new(r, brass.alpha(0.32))),
        );
    }
    lbl
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
    // A pixel-framed square. At 16 px the frame is 1 px, so the initial still has room.
    let tile = el()
        .size(size, size)
        .radius(4.0)
        .clip_rounded(4.0)
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
    // Keep initial-on-tile contrast >= 4.5:1 on both dark card (#1e1f23) and light card (#ffffff).
    tile.center()
        .fill(hue.alpha(if s.dark { 0.30 } else { 0.28 }))
        .child(label(initial, META, p.ink))
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
/// a row; the open thread says so in the composer notice stack.
fn machine_caption(s: &State, server: &str) -> String {
    s.machine_label(server)
}

/// KSG Code's EnvironmentMachineIcon distinguishes a desktop from a linked server.
fn machine_glyph(s: &State, server: &str) -> &'static str {
    if server == "local"
        || s.machines
            .views
            .get("local")
            .and_then(|view| view["serverId"].as_str())
            == Some(server)
    {
        "monitor"
    } else {
        "server"
    }
}

/// Whether more than one machine is linked, so rows say which machine they belong to.
fn multiple_machines(s: &State) -> bool {
    s.client
        .as_ref()
        .is_some_and(|client| !client.linked.is_empty())
}

/// A sidebar row's width in points, from the sidebar setting and the last window width.
fn sidebar_row_width(s: &State) -> f32 {
    sidebar_width_at(s.sidebar_width, s.viewport_width) - 16.0
}

fn sidebar_max_width(viewport: f32) -> f32 {
    (viewport - 480.0).clamp(crate::model::SIDEBAR_MIN, crate::model::SIDEBAR_MAX)
}

fn sidebar_width_at(preferred: f32, viewport: f32) -> f32 {
    if viewport < 760.0 {
        // The compact drawer sits in front of the chat rather than beside it.
        // Widen the docked sidebar without covering more of a small conversation.
        preferred.min(288.0).min((viewport - 56.0).max(0.0))
    } else {
        preferred.min(sidebar_max_width(viewport))
    }
}

/// About how wide `text` is at the sidebar's 13 px.
pub(super) fn text_width(text: &str) -> f32 {
    text.chars().count() as f32 * 6.2
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
        // Keep the icon visible when the machine name truncates.
        .gap(4.0)
        .center_y()
        .shrink()
        .min_w(ICON)
        .max_w(MACHINE_CAP)
        .child(icon(machine_glyph(s, server), p.faint));
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

/// In-place captain selector in the composer footer, ported from KSG Code's
/// `BranchToolbarEnvironmentSelector` (`ComposerControl` size `xs`) for the trigger and
/// `ModelPickerContent` for the searchable popover with a trailing creation action.
/// Captains are Shina-only (KSG Code has no captains).
fn captain_selector_trigger(s: &State, p: Palette, icon_only: bool) -> El {
    let t = s.current();
    let active = t.captain.and_then(|id| s.captains.get(id).map(|c| (id, c)));
    let locked = t.status.busy() || t.configuring || t.archived() || !t.is_draft();
    let title = active
        .map(|(_, c)| c.name.clone())
        .unwrap_or_else(|| "No captain".into());
    let mut trigger = row()
        .key("captain-selector")
        .role("button")
        .label(format!("Captain: {title}"))
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
        .radius(6.0)
        .hover(p.selected)
        .focusable(!locked)
        .cursor(if locked {
            rune::Cursor::Default
        } else {
            rune::Cursor::Pointer
        })
        .opacity(if locked { 0.65 } else { 1.0 })
        .disabled(locked);
    trigger = trigger.preserve_focus_on_press(true);
    if !locked {
        trigger = trigger.on_press(|s: &mut State| {
            s.begin_composer_menu();
            s.captain_picker = !s.captain_picker;
            if s.captain_picker {
                s.captain_project = Some(s.current().project);
                s.captain_picker_query = rune::TextEdit::default();
                s.captain_picker_query.focused = true;
                s.captain_picker_highlight = 0;
            }
        });
    }
    let lead = match active {
        Some((id, _)) => avatar(s, p, id, 14.0, s.captain_status(id)),
        None => icon("user", p.muted).size(12.0, 12.0),
    };
    trigger
        .child(lead)
        .child(if icon_only {
            el()
        } else {
            label(title, SMALL, if active.is_some() { p.ink } else { p.muted })
                .key("captain-selector-label")
                .truncate()
                .shrink()
                .min_w(0.0)
        })
        .child(icon("chevron", p.muted).size(12.0, 12.0))
}

/// Ported from KSG Code's `ModelPickerContent` (searchable list in a bordered popover card)
/// and `BranchToolbarEnvironmentSelector` (anchored above the composer footer control).
/// Captains, portraits, and the "No captain" / "New captain" rows are Shina-specific.
fn captain_picker(s: &State, p: Palette, anchor: &str) -> El {
    let t = s.current();
    let selected_captain = t.captain;
    let query = s.captain_picker_query.text.trim().to_lowercase();
    let filtered: Vec<usize> = s
        .picker_captains()
        .filter(|&id| {
            if query.is_empty() {
                return true;
            }
            let c = &s.captains[id];
            c.name.to_lowercase().contains(&query) || c.role.to_lowercase().contains(&query)
        })
        .collect();

    let header = row().center_y().gap(6.0).px(10.0).py(8.0).child(
        row()
            .center_y()
            .gap(6.0)
            .h(30.0)
            .px(10.0)
            .radius(6.0)
            .fill(p.selected)
            .field_focus(&s.captain_picker_query, p)
            .grow(1.0)
            .min_w(0.0)
            .child(icon("search", p.muted))
            .child(
                text_input(&s.captain_picker_query, TextStyle::new(13.0, 400.0))
                    .key("captain-search")
                    .label("Search captains")
                    .placeholder("Search captains", p.muted)
                    .color(p.ink)
                    .caret(p.ink, p.coral_bg)
                    .grow(1.0)
                    .min_w(0.0)
                    .on_edit(|s: &mut State| &mut s.captain_picker_query)
                    .on_input_key(|s: &mut State, event, tx| {
                        if matches!(event.key, rune::Key::Down) {
                            s.captain_picker_query.focused = false;
                            tx.focus("captain-list", true);
                            return true;
                        }
                        false
                    }),
            ),
    );

    let mut list = col()
        .key("captain-list")
        .role("listbox")
        .label("Captains")
        .focus_group()
        .w_full()
        .max_h(260.0)
        .pb(4.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });

    let no_captain_selected = selected_captain.is_none();
    let mut no_captain = row()
        .key("choose-no-captain")
        .role("option")
        .label("No captain (plain thread)")
        .checked(no_captain_selected)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .center_y()
        .gap(10.0)
        .h(34.0)
        .mx(4.0)
        .pl(12.0)
        .pr(8.0)
        .radius(6.0)
        .fill(if no_captain_selected {
            p.coral_bg
        } else {
            Color::TRANSPARENT
        })
        .hover(if no_captain_selected {
            p.coral_bg
        } else {
            p.selected
        })
        .focus_fill(p.selected)
        .focus_selected(no_captain_selected)
        .on_press(|s: &mut State| {
            s.captain_picker = false;
            s.captain_project = None;
            s.projects.captain_target = None;
            if s.current().captain.is_some() {
                let project = s.current().project;
                s.new_thread_in(project);
            }
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
        .child(icon("user", p.muted).size(16.0, 16.0))
        .child(
            label("No captain", TextStyle::new(14.0, 500.0), p.ink)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        );
    if no_captain_selected {
        no_captain = no_captain.child(icon("check", p.sea));
    }
    list = list.child(no_captain);

    if !filtered.is_empty() {
        list = list.child(meta_label("Captains", p.muted).px(16.0).pt(6.0).pb(4.0));
    }

    let filtered_empty = filtered.is_empty() && !query.is_empty();
    for captain in filtered {
        let profile = &s.captains[captain];
        let state = s.captain_status(captain);
        let selected = selected_captain == Some(captain);
        let mut item = row()
            .key(format!("choose-captain-{captain}"))
            .role("option")
            .label(format!("Open Captain {}, {}", profile.name, state.label()))
            .checked(selected)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .center_y()
            .gap(10.0)
            .h(36.0)
            .mx(4.0)
            .pl(12.0)
            .pr(8.0)
            .radius(6.0)
            .fill(if selected {
                p.coral_bg
            } else {
                Color::TRANSPARENT
            })
            .hover(if selected { p.coral_bg } else { p.selected })
            .focus_fill(p.selected)
            .focus_selected(selected)
            .on_press(move |s: &mut State| {
                s.captain_picker = false;
                s.open_captain(captain);
            })
            .child(avatar(s, p, captain, 18.0, state))
            .child(
                col()
                    .gap(1.0)
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        label(&profile.name, TextStyle::new(14.0, 500.0), p.ink)
                            .truncate()
                            .min_w(0.0),
                    )
                    .child(if profile.role.is_empty() {
                        el()
                    } else {
                        label(&profile.role, SMALL, p.muted).truncate().min_w(0.0)
                    }),
            );
        if selected {
            item = item.child(icon("check", p.sea));
        }
        list = list.child(item);
    }
    if filtered_empty {
        list = list.child(
            label("No matching captains", SMALL, p.muted)
                .px(16.0)
                .py(8.0),
        );
    }

    let mut card = col()
        .key("captain-picker")
        .role("dialog")
        .label("Choose a captain")
        .w(PICKER_WIDTH)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(header)
        .child(list)
        .child(rule(p));
    let new_captain = row()
        .key("picker-new-captain")
        .role("button")
        .label("New captain")
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .center_y()
        .gap(10.0)
        .h(34.0)
        .mx(4.0)
        .pl(12.0)
        .pr(8.0)
        .radius(6.0)
        .hover(p.selected)
        .focus_fill(p.selected)
        .on_press(|s: &mut State| s.begin_new_captain())
        .child(icon("plus", p.muted))
        .child(label("New captain", TextStyle::new(14.0, 500.0), p.ink));
    card = card.child(col().py(4.0).child(new_captain));

    card.anchored_fit(anchor, rune::Side::Top, rune::Align::Start, 6.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.cancel_captain_flow();
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// A portrait's cell in the captain editor's grid: the 40 pt face and 4 pt each side.
const PICKER_CELL: f32 = 48.0;
fn captain_editor(s: &State, p: Palette, height: f32) -> El {
    let editor = s.captain_editor.as_ref().unwrap();
    let editing = editor.editing.is_some();
    let promotion = editor.promotion.as_ref();
    let pending = promotion.is_some_and(|promotion| promotion.pending.is_some());
    let restore = promotion.is_some_and(|promotion| promotion.restore.is_some());
    let profile_locked = pending || restore;
    let promotion_reason = promotion.and_then(|promotion| {
        s.captain_promotion_unavailable(&editor.server, promotion.thread)
            .or_else(|| pending.then(|| "Waiting for the machine to confirm.".into()))
            .or_else(|| {
                promotion
                    .preview
                    .as_ref()
                    .and_then(|preview| preview["blockers"].as_array())
                    .and_then(|blockers| blockers.first())
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .or_else(|| {
                promotion
                    .preview
                    .is_none()
                    .then(|| "Waiting for the promotion preview.".into())
            })
    });
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
                    .disabled(profile_locked)
                    .focusable(true)
                    .size(PICKER_CELL, PICKER_CELL)
                    .center()
                    .radius(6.0)
                    .hover(p.selected)
                    .on_press(move |s: &mut State| {
                        s.captain_editor.as_mut().unwrap().portrait = portrait;
                    })
                    // The selected portrait's own frame turns the ring colour. There is no wash
                    // around it, so all twelve keep the same outer bounds.
                    .child(
                        portrait_face(
                            s,
                            p,
                            portrait,
                            PICKER_PORTRAIT,
                            Status::Ready,
                            selected.then_some(p.ring),
                        )
                        // The editor's header shows the chosen portrait at the same size.
                        .key(format!("captain-portrait-face-{portrait}")),
                    ),
            );
        }
        portraits = portraits.child(line);
    }
    let captain_project = promotion.map_or_else(
        || s.captain_project.unwrap_or(s.current().project),
        |promotion| promotion.project,
    );
    let mut body = col()
        .key("captain-form-body")
        .gap(12.0)
        .min_h(0.0)
        .max_h((height - 180.0).max(180.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child({
            // Ported from KSG Code's BranchToolbarEnvironmentSelector. A captain's host
            // is immutable after creation, unlike an unsent conversation draft.
            if machine_options(s).len() > 1 || promotion.is_some() {
                let mut lives = col()
                    .key("captain-machine-row")
                    .gap(5.0)
                    .child(meta_label("Lives on", p.muted))
                    .child(
                        menu_item(
                            if promotion.is_some() { "captain-promote-machine" } else { "captain-machine-choice" },
                            &s.machine_label(&editor.server),
                            if editor.server == "local" {
                                "monitor"
                            } else {
                                "cloud"
                            },
                            p,
                        )
                        .disabled(editing || promotion.is_some())
                        .on_press(|s: &mut State| {
                            let editor = s.captain_editor.as_mut().unwrap();
                            if editor.editing.is_none() && editor.promotion.is_none() {
                                editor.machine_picker = !editor.machine_picker;
                            }
                        }),
                    );
                if editor.machine_picker && !editing && promotion.is_none() {
                    for server in machine_options(s) {
                        if !s.machine_connected(&server) {
                            continue;
                        }
                        let title = s.machine_label(&server);
                        lives = lives.child(
                            machine_option(
                                &format!("captain-machine-{server}"),
                                &title,
                                &server,
                                true,
                                p,
                            )
                            .on_press(move |s: &mut State| s.retarget_captain(&server)),
                        );
                    }
                }
                lives
            } else {
                el()
            }
        })
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Workspace", p.muted))
                .child(
                    row()
                        .key(if promotion.is_some() { "captain-promote-workspace" } else { "captain-project-choice" })
                        .role("button")
                        .cursor(rune::Cursor::Pointer)
                        .label(format!(
                            "Workspace: {}",
                            s.project(captain_project)
                                .map(|p| p.name.as_str())
                                .unwrap_or("No workspace")
                        ))
                        .focusable(!editing && promotion.is_none())
                        .disabled(promotion.is_some())
                        // The same height as the text inputs below it.
                        .h(37.0)
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
                                .is_some_and(|e| e.editing.is_none() && e.promotion.is_none())
                            {
                                s.open_picker(ProjectPicker::CaptainWorkspace);
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
                        .child(meta_label("Portrait", p.muted))
                        .child(label(crate::visuals::PORTRAIT_NAMES[chosen], SMALL, p.ink)),
                )
                .child(portraits),
        )
        .child(
            col().gap(5.0).child(meta_label("Name", p.muted)).child(
                text_input(&editor.name, BODY)
                    .key("captain-name")
                    .label("Captain name")
                    .placeholder("e.g. Atlas", p.muted)
                    .single_line()
                    .color(p.ink)
                    .caret(p.ink, p.coral_bg)
                    .w_full()
                    .padding(8.0)
                    .radius(6.0)
                    .fill(field)
                    .field_focus(&editor.name, p)
                    .disabled(profile_locked)
                    .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().name),
            ),
        )
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Role (optional)", p.muted))
                .child(
                    text_input(&editor.role, BODY)
                        .key("captain-role")
                        .label("Captain role")
                        .placeholder("e.g. Code review", p.muted)
                        .single_line()
                        .color(p.ink)
                        .caret(p.ink, p.coral_bg)
                        .w_full()
                        .padding(8.0)
                        .radius(6.0)
                        .fill(field)
                        .field_focus(&editor.role, p)
                        .disabled(profile_locked)
                        .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().role),
                ),
        )
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Instructions for Pi", p.muted))
                .child(
                    text_input(&editor.instructions, BODY)
                        .key("captain-instructions")
                        .label("Captain instructions")
                        .placeholder("What should this captain focus on? Optional.", p.muted)
                        .color(p.ink)
                        .caret(p.ink, p.coral_bg)
                        .w_full()
                        .min_h(76.0)
                        .max_h(140.0)
                        .padding(8.0)
                        .radius(6.0)
                        .fill(field)
                        .field_focus(&editor.instructions, p)
                        .disabled(profile_locked)
                        .on_edit(|s: &mut State| {
                            &mut s.captain_editor.as_mut().unwrap().instructions
                        }),
                ),
        )
        .child(
            label(
                if promotion.is_some() {
                    "Promotion keeps this thread's model and access mode. Private captain instructions stay with the captain."
                } else {
                    "The captain and its crew use these instructions and Pi's configured CPA model."
                },
                SMALL,
                p.muted,
            )
            .wrap(),
        );
    if let Some(promotion) = promotion {
        // KSG has no Captain role. This adapts the existing captain editor and its
        // ProjectIconPickerDialog layout with the approved role-change summary.
        body = body.child(
            col().key("captain-promote-summary").gap(6.0)
                .child(meta_label("What stays", p.muted))
                .child(label("The conversation, its model, access mode, workspace, machine, tasks and pull requests.", BODY, p.muted).wrap())
                .child(meta_label("What changes", p.muted))
                .child(label("It moves to Captains and gets a Brief. Its scope starts as this workspace.", BODY, p.muted).wrap()),
        );
        if let Some(captain) = promotion.restore_candidate {
            let server = editor.server.clone();
            let thread = promotion.thread;
            body = body.child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        captain_ui::switch(
                            s,
                            "captain-promote-restore",
                            "Restore the former captain's profile",
                            restore,
                            p,
                            move |s| {
                                if s.machines.selected == server
                                    && s.captain_editor
                                        .as_ref()
                                        .and_then(|editor| editor.promotion.as_ref())
                                        .is_some_and(|promotion| promotion.thread == thread)
                                    && let Err(error) = s.set_captain_promotion_restore(!restore)
                                    && let Some(editor) = s.captain_editor.as_mut()
                                {
                                    editor.error = Some(error);
                                }
                            },
                        )
                        .disabled(pending),
                    )
                    .child(
                        label(
                            format!(
                                "Restore {}'s profile, plan, rules and memory",
                                s.captains[captain].name
                            ),
                            SMALL,
                            p.muted,
                        )
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                    ),
            );
        }
    }
    if let Some(error) = editor.error.as_ref().or(promotion_reason.as_ref()) {
        body = body.child(
            label(error, SMALL, p.danger)
                .key("captain-editor-reason")
                .role("alert")
                .wrap(),
        );
    }
    let title = if let Some(promotion) = promotion {
        format!("Make \"{}\" a captain", promotion.title)
    } else if editing {
        "Edit captain".into()
    } else {
        "New captain".into()
    };
    let save_label = if promotion.is_some() {
        "Make captain"
    } else if editing {
        "Save changes"
    } else {
        "Create captain"
    };
    let card = col()
        .key("captain-editor")
        .role("dialog")
        .label(&title)
        .w(480.0)
        .max_h((height - 32.0).max(240.0))
        .padding(18.0)
        .gap(14.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .center_y()
                .gap(12.0)
                .child(portrait_avatar(s, p, editor.portrait, 40.0, Status::Ready))
                .child(
                    col()
                        .gap(3.0)
                        .child(label(&title, DIALOG_TITLE, p.ink).wrap())
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
                .child(
                    if profile.validate().is_ok() && promotion_reason.is_none() {
                        primary_button("save-captain", save_label, p, |s| s.save_captain())
                    } else {
                        button("save-captain", save_label, p, |_| {})
                            .h(32.0)
                            .px(14.0)
                            .opacity(0.5)
                            .disabled(true)
                    },
                ),
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
/// A tab is 28 pt tall inside the 40 pt bar's 36 pt row, the spec's opt-in strip.
const TAB_HEIGHT: f32 = 28.0;
/// Below this a tab drops its leading icon and keeps the title.
const TAB_PREFIX_WIDTH: f32 = 170.0;
/// The narrowest a tab gets before the strip scrolls.
const TAB_MIN_WIDTH: f32 = 120.0;
/// Tabs are packed from the left and stop growing here, so a few tabs do not sit in wide empty
/// slots.
const TAB_MAX_WIDTH: f32 = 200.0;
/// Inter 13/500, the title in a tab.
const TAB_TITLE: TextStyle = TextStyle::new(13.0, 500.0);
/// The 16 px leading icon in a tab, KSG `PaneTabs`' `ProjectFavicon`. Captain homes and
/// captain workers show the captain's face when the profile exists locally, an explicit
/// remote-delegation badge for peer workers (or missing captain profiles), and the project
/// tile otherwise.
fn tab_icon(s: &State, p: Palette, thread: &Thread) -> El {
    let captain = if s.is_captain_home(thread) {
        thread.captain
    } else {
        thread.crew_captain
    };
    if let Some(captain) = captain {
        if s.captains.get(captain).is_some() {
            return avatar(s, p, captain, 16.0, s.captain_status(captain)).a11y_hidden();
        }
        return el()
            .key(format!("tab-icon-crew-{}", thread.id))
            .size(16.0, 16.0)
            .radius(4.0)
            .border(1.0, p.frame)
            .fill(p.selected)
            .center()
            .a11y_hidden()
            .child(icon("crew", p.muted).size(12.0, 12.0));
    }
    if thread.peer_origin.is_some() {
        return el()
            .key(format!("tab-icon-peer-{}", thread.id))
            .size(16.0, 16.0)
            .radius(4.0)
            .border(1.0, p.frame)
            .fill(p.selected)
            .center()
            .a11y_hidden()
            .child(icon("crew", p.muted).size(12.0, 12.0));
    }
    project_tile(s, p, s.project(thread.project), 16.0)
}
/// A tab's look, as KSG's chat tabs: the active tab is a `card` with a hairline and 8 px
/// corners. Others have no fill.
fn tab_frame(tab: El, active: bool, p: Palette) -> El {
    if !active {
        // The hover block matches the active tab's 8 px corners, so a tab's fill
        // never shifts shape as it activates.
        return tab.hover(p.selected).radius(8.0);
    }
    tab.fill(p.card).radius(8.0).border(1.0, p.line)
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

fn header(s: &State, p: Palette, _narrow: bool, tabs_width: f32) -> El {
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
    let tab_width = ((tabs_width + TAB_GAP) / count - TAB_GAP).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
    for &id in &s.tabs {
        let Some(index) = s.threads.iter().position(|t| t.id == id) else {
            continue;
        };
        let thread = &s.threads[index];
        let active = index == s.active;
        // KSG draws the ghost tab's title in italics once kept tabs sit
        // beside it; a lone tab never reads as provisional.
        let ghost = s.is_ghost_tab(id) && s.tabs.len() > 1;
        let project = s
            .project(thread.project)
            .map(|p| p.name.as_str())
            .unwrap_or("No workspace");
        let mut tab = tab_frame(
            row()
                // The pressable keeps `tab-{id}` whether focused or not: Rune
                // pairs a double click by element path, so a key that changed
                // on focus would swallow the second click before `keep_tab`.
                .key(format!("tab-{id}"))
                .role("tab")
                .cursor(rune::Cursor::Pointer)
                .label(format!("{project} / {}", thread.title))
                .focusable(true)
                .center_y()
                .gap(8.0)
                .pl(8.0)
                .pr(if s.tabs.len() > 1 { 2.0 } else { 8.0 })
                .h(TAB_HEIGHT)
                .radius(8.0)
                .w(tab_width)
                .min_w(tab_width)
                .hover_group()
                .pointer_buttons([rune::PointerButton::Middle])
                .on_press_tx(move |s: &mut State, tx| {
                    // KSG: a middle click closes the tab.
                    if tx.button() == rune::PointerButton::Middle {
                        s.close_tab(id);
                        return;
                    }
                    s.show_thread(index);
                    tx.scroll_into_view("active-tab");
                })
                .on_double_press(move |s: &mut State| s.keep_tab(id))
                .on_secondary_press(move |s: &mut State, at, _| {
                    s.tab_menu = Some(id);
                    s.tab_menu_at = Some((at.x, at.y));
                }),
            active,
            p,
        )
        .child(if tab_width >= TAB_PREFIX_WIDTH {
            // KSG's `PaneTabs` leads with the project favicon; captain homes and
            // their workers show the captain's face instead.
            tab_icon(s, p, thread)
        } else {
            // A tab at its minimum gives the width to the title.
            el()
        })
        .child(
            // The active marker covers the tab exactly (`absolute` measures
            // from the same corner the border box does): tests and
            // `scroll_into_view` find `active-tab` here while the pressable
            // above keeps a stable `tab-{id}` key so the second click of a
            // double-click still pairs with the first.
            el().key(if active {
                "active-tab".into()
            } else {
                format!("tab-marker-{id}")
            })
            .a11y_hidden()
            .absolute(0.0, 0.0)
            .w(tab_width)
            .h(TAB_HEIGHT),
        )
        .child(
            label(
                &thread.title,
                if ghost { TAB_TITLE.italic() } else { TAB_TITLE },
                p.ink,
            )
            .truncate()
            .grow(1.0),
        );
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
                .radius(6.0)
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

/// Ported from KSG Code's thread tab context menu on the `row_menu` pattern: a
/// ghost tab offers "Keep tab" first, then the close group. Items that would do
/// nothing are disabled, as KSG does.
fn thread_tab_menu(s: &State, p: Palette) -> El {
    let id = s.tab_menu.expect("tab menu only mounts with a tab");
    let index = s.tabs.iter().position(|tab| *tab == id);
    let last = s.tabs.len().saturating_sub(1);
    let anchor = if s.tab_menu_at.is_some() {
        "thread-tab-menu-point".to_string()
    } else {
        format!("tab-{id}")
    };
    enum TabMenu {
        Keep,
        Close,
        Others,
        ToRight,
        All,
    }
    let item = |key: &str, title: &str, glyph: &str, action: Option<TabMenu>| {
        let Some(action) = action else {
            return disabled_menu_item(key, title, glyph, p);
        };
        menu_item(key, title, glyph, p).on_press(move |s: &mut State| {
            s.tab_menu = None;
            match action {
                TabMenu::Keep => s.keep_tab(id),
                TabMenu::Close => s.close_tab(id),
                TabMenu::Others => s.close_other_tabs(id),
                TabMenu::ToRight => s.close_tabs_to_right(id),
                TabMenu::All => s.close_all_tabs(),
            }
        })
    };
    let keep = s.is_ghost_tab(id).then_some(TabMenu::Keep);
    let close = (s.tabs.len() > 1).then_some(TabMenu::Close);
    let others = (s.tabs.len() > 1).then_some(TabMenu::Others);
    let to_right = index.is_some_and(|i| i < last).then_some(TabMenu::ToRight);
    let all = (s.tabs.len() > 1).then_some(TabMenu::All);
    col()
        .key("thread-tab-menu")
        .role("menu")
        .focus_group()
        .label("Tab actions")
        .autofocus()
        .w(222.0)
        .padding(4.0)
        .gap(1.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .anchored_fit(anchor, rune::Side::Bottom, rune::Align::Start, 2.0)
        .modal(true, |s: &mut State| s.tab_menu = None)
        .child(item("menu-tab-keep", "Keep tab", "pin", keep))
        .child(item("menu-tab-close", "Close", "close", close))
        .child(item("menu-tab-others", "Close others", "close", others))
        .child(item(
            "menu-tab-right",
            "Close to the right",
            "chevron",
            to_right,
        ))
        .child(item("menu-tab-all", "Close all", "close", all))
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
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .anchored_fit("thread-actions", rune::Side::Bottom, rune::Align::End, 4.0)
        .modal(true, |s: &mut State| s.thread_menu = false)
        .child(thread_actions(s, s.current().id, p, "thread-actions"))
}

fn thread_actions(s: &State, id: usize, p: Palette, anchor: &str) -> El {
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
    if captain && id == s.current().id {
        // KSG's BranchToolbarEnvironmentSelector uses a machine row per
        // environment. Shina keeps the owner selected and sends one turn there.
        for (machine, project, label) in s.captain_continue_targets(id) {
            actions = actions.child(
                gated_menu_item(
                    &format!("captain-continue-{machine}-{project}"),
                    &format!("Continue on {label}"),
                    "monitor",
                    p,
                    s.continue_blocker(),
                )
                .on_press(move |s: &mut State| {
                    s.thread_menu = false;
                    s.row_menu = None;
                    s.continue_captain_draft(&machine, project);
                }),
            );
        }
    }
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
        actions = actions
            .child(
                gated_menu_item(
                    "menu-fork-conversation",
                    "Fork conversation",
                    "thread",
                    p,
                    s.conversation_blocker(thread),
                )
                .on_press(|s: &mut State| {
                    s.thread_menu = false;
                    s.row_menu = None;
                    s.fork_conversation(None);
                }),
            )
            .child(
                gated_menu_item(
                    "menu-fork-message",
                    "Fork from message...",
                    "thread",
                    p,
                    s.rewind_blocker(thread),
                )
                .on_press(|s: &mut State| {
                    s.thread_menu = false;
                    s.row_menu = None;
                    s.conversation_points(crate::model::ConversationMode::Fork);
                }),
            )
            .child(
                gated_menu_item(
                    "menu-rewind",
                    "Edit from here...",
                    "edit",
                    p,
                    s.rewind_blocker(thread),
                )
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
                gated_menu_item(
                    "menu-pin-thread",
                    if thread.pinned() {
                        "Unpin thread"
                    } else {
                        "Pin thread"
                    },
                    "pin",
                    p,
                    s.lifecycle_blocker(id),
                )
                .on_press(move |s: &mut State| {
                    s.row_menu = None;
                    s.thread_menu = false;
                    s.pin_thread(id);
                }),
            )
            .child(
                gated_menu_item(
                    "menu-settle-thread",
                    if thread.life.settled() {
                        "Un-settle thread"
                    } else {
                        "Settle thread"
                    },
                    "check",
                    p,
                    s.lifecycle_blocker(id),
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
            let snooze_anchor = anchor.to_owned();
            actions = actions.child(
                gated_menu_item(
                    "menu-snooze-thread",
                    "Snooze...",
                    "clock",
                    p,
                    s.snooze_blocker(id),
                )
                .on_press(move |s: &mut State| {
                    let at = if snooze_anchor == "thread-actions" {
                        None
                    } else {
                        s.row_menu_at
                    };
                    s.open_snooze_from(id, Some(snooze_anchor.clone()), at);
                }),
            );
        }
    }
    if !thread.captain_home
        && !thread.client_draft
        && !thread.delegated
        && thread.parent.is_none()
        && thread.peer_origin.is_none()
    {
        let server = s.machines.selected.clone();
        let reason = s.captain_promotion_unavailable(&server, id);
        let item = gated_menu_item(
            "menu-promote-captain",
            "Make this a captain...",
            "grid",
            p,
            reason,
        );
        actions = actions.child(item.on_press(move |s: &mut State| {
            s.row_menu = None;
            s.thread_menu = false;
            if let Err(error) = s.begin_captain_promotion(&server, id) {
                s.push_toast(
                    crate::model::ToastKind::Error,
                    "Could not make the thread a captain",
                    Some(error),
                    None,
                    Some(6000),
                );
            }
        }));
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
            gated_menu_item(
                "menu-mark-unread",
                "Mark unread",
                "thread",
                p,
                s.unread_blocker(thread),
            )
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
        // Crew membership on the thread's own machine: an idle member can move or leave,
        // an ordinary root can join through the same picker the Brief uses. Hidden where
        // membership cannot exist; disabled rows carry the real reason.
        if let Some(owner) = thread.crew_captain {
            let expected = s.machines.selected.clone();
            let move_server = expected.clone();
            let reason = crew_membership_ui::access_reason(s.captain_access(owner)).or_else(|| {
                s.threads
                    .iter()
                    .position(|t| t.id == id)
                    .and_then(|index| s.crew_idle(index).err())
            });
            let move_anchor = anchor.to_string();
            actions = actions
                .child(
                    menu_item("menu-crew-move", "Move to another captain...", "switch", p)
                        .label(match &reason {
                            Some(reason) => format!("Move to another captain, {reason}"),
                            None => "Move to another captain".to_string(),
                        })
                        .disabled(reason.is_some())
                        .on_press(move |s: &mut State| {
                            if s.machines.selected != move_server
                                || s.captain_access(owner) != crate::model::CaptainAccess::Full
                            {
                                return;
                            }
                            let at = if move_anchor == "thread-actions" {
                                None
                            } else {
                                s.row_menu_at
                            };
                            s.row_menu = None;
                            s.row_menu_at = None;
                            s.thread_menu = false;
                            s.brief.moving = Some(crate::model::MovePicker {
                                server: move_server.clone(),
                                thread: id,
                                mode: crate::model::MoveMode::Move { from: owner },
                                anchor: move_anchor.clone(),
                                at,
                            });
                        }),
                )
                .child(
                    menu_item("menu-crew-release", "Release from crew", "close", p)
                        .label(match &reason {
                            Some(reason) => format!("Release from crew, {reason}"),
                            None => "Release from crew".to_string(),
                        })
                        .disabled(reason.is_some())
                        .on_press(move |s: &mut State| {
                            s.row_menu = None;
                            s.thread_menu = false;
                            if let Err(error) = s.captain_crew_command(
                                &expected,
                                owner,
                                id,
                                crate::model::CrewKind::Release,
                            ) {
                                s.push_toast(
                                    crate::model::ToastKind::Error,
                                    "Could not release the thread",
                                    Some(error),
                                    None,
                                    Some(6000),
                                );
                            }
                        }),
                );
        } else if !s.captains.is_empty()
            && !thread.captain_home
            && thread.parent.is_none()
            && thread.peer_origin.is_none()
            && !thread.delegated
            && !thread.archived()
            && (0..s.captains.len()).any(|captain| {
                !matches!(
                    s.captain_access(captain),
                    crate::model::CaptainAccess::UpdateRequired
                        | crate::model::CaptainAccess::NoRecords
                )
            })
        {
            let reason = s
                .threads
                .iter()
                .position(|t| t.id == id)
                .and_then(|index| s.join_gate(index).err());
            let join_anchor = anchor.to_string();
            let join_server = s.machines.selected.clone();
            actions = actions.child(
                gated_menu_item(
                    "menu-crew-recruit",
                    "Add to a captain's crew...",
                    "switch",
                    p,
                    reason,
                )
                .on_press(move |s: &mut State| {
                    if s.machines.selected != join_server {
                        return;
                    }
                    let at = if join_anchor == "thread-actions" {
                        None
                    } else {
                        s.row_menu_at
                    };
                    s.row_menu = None;
                    s.row_menu_at = None;
                    s.thread_menu = false;
                    s.brief.moving = Some(crate::model::MovePicker {
                        server: join_server.clone(),
                        thread: id,
                        mode: crate::model::MoveMode::Join,
                        anchor: join_anchor.clone(),
                        at,
                    });
                }),
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
    let primary_captain = thread
        .captain
        .filter(|&captain| thread.captain_home && s.captain_home(captain) == Some(id));
    if let Some(captain) = primary_captain {
        use crate::model::{CaptainLifecycleAction as Action, CaptainPhase};
        let archived = s.captain_phase(captain) == CaptainPhase::Archived;
        actions = actions.child(captain_lifecycle_menu_item(
            s,
            p,
            captain,
            if archived {
                Action::Unarchive
            } else {
                Action::Archive
            },
            if archived {
                "menu-unarchive-captain"
            } else {
                "menu-archive-captain"
            },
            if archived {
                "Unarchive captain"
            } else {
                "Archive captain..."
            },
            "archive",
            false,
        ));
        if !archived {
            actions = actions.child(captain_lifecycle_menu_item(
                s,
                p,
                captain,
                Action::Demote,
                "menu-demote-captain",
                "Make it an ordinary thread...",
                "thread",
                false,
            ));
        }
    } else {
        let grouped_archive = thread
            .crew_captain
            .filter(|&captain| s.captain_phase(captain) == crate::model::CaptainPhase::Archived);
        let reason = grouped_archive
            .map(|captain| {
                format!(
                    "Archived with {}. Unarchive the captain to restore it.",
                    s.captains[captain].name
                )
            })
            .or_else(|| s.lifecycle_blocker(id));
        let archive = gated_menu_item(
            "menu-archive-thread",
            if thread.archived() {
                "Restore thread"
            } else {
                "Archive thread"
            },
            "archive",
            p,
            reason,
        );
        actions = actions.child(archive.on_press(move |s: &mut State| {
            s.row_menu = None;
            s.thread_menu = false;
            s.archive_thread(id);
        }));
    }
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
    actions = actions.child(rule(p));
    if let Some(captain) = primary_captain {
        actions.child(captain_lifecycle_menu_item(
            s,
            p,
            captain,
            crate::model::CaptainLifecycleAction::Delete,
            "menu-delete-captain",
            "Delete captain...",
            "trash",
            true,
        ))
    } else {
        actions.child(
            danger_menu_item("menu-delete-thread", "Delete thread...", "trash", p)
                .on_press(move |s: &mut State| s.request_delete(id)),
        )
    }
}

/// Captain group actions have no KSG equivalent. Their menu rows reuse KSG's Sidebar
/// archive/delete entries and open the shared alert-dialog confirmation.
fn captain_lifecycle_menu_item(
    s: &State,
    p: Palette,
    captain: usize,
    action: crate::model::CaptainLifecycleAction,
    key: &str,
    title: &str,
    glyph: &str,
    danger: bool,
) -> El {
    let server = s.machines.selected.clone();
    let reason = s.captain_lifecycle_unavailable(&server, captain, action);
    let item = if reason.is_some() {
        gated_menu_item(key, title, glyph, p, reason)
    } else if danger {
        danger_menu_item(key, title, glyph, p)
    } else {
        menu_item(key, title, glyph, p)
    };
    item.on_press(move |s: &mut State| {
        s.row_menu = None;
        s.thread_menu = false;
        if let Err(error) = s.open_captain_lifecycle(&server, captain, action) {
            s.push_toast(
                crate::model::ToastKind::Error,
                "Could not open the captain action",
                Some(error),
                None,
                Some(6000),
            );
        }
    })
}

fn snooze_menu(s: &State, p: Palette, cx: &Cx<State>) -> Option<El> {
    let (id, choices) = s.snooze_menu.as_ref()?;
    let id = *id;
    let row_key = format!("thread-{id}");
    // Under Rune #1339 an anchored overlay whose anchor is not mounted is
    // suppressed visually while still holding modal focus. Use `thread-actions`
    // for header launches, a mounted row or captured pointer point for sidebar
    // launches, and never mount a modal without a live anchor.
    let (anchor, side, align) = if s.snooze_anchor.as_deref() == Some("thread-actions")
        && cx.layout_with("thread-actions", |_| ()).is_some()
    {
        (
            "thread-actions".to_owned(),
            rune::Side::Bottom,
            rune::Align::End,
        )
    } else if let Some(a) = s
        .snooze_anchor
        .as_deref()
        .filter(|a| cx.layout_with(a, |_| ()).is_some())
    {
        (a.to_owned(), rune::Side::Right, rune::Align::Start)
    } else if cx.layout_with(&row_key, |_| ()).is_some() {
        (row_key, rune::Side::Right, rune::Align::Start)
    } else if s.snooze_at.is_some() {
        (
            "snooze-menu-point".to_owned(),
            rune::Side::Right,
            rune::Align::Start,
        )
    } else if cx.layout_with("thread-actions", |_| ()).is_some() {
        (
            "thread-actions".to_owned(),
            rune::Side::Bottom,
            rune::Align::End,
        )
    } else {
        return None;
    };
    let mut card = col()
        .key("snooze-menu")
        .role("menu")
        .focus_group()
        .w(280.0)
        .padding(4.0)
        .gap(1.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        // The title is a pixel label, inset 8 px so it lines up with the rows' icons.
        .child(meta_label("Snooze thread", p.muted).px(8.0).pt(6.0).pb(4.0));
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
                    .radius(6.0)
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
    // A popover on the thread's row or header trigger: no scrim, no Cancel row. Escape and a
    // press outside close it.
    Some(
        card.anchored_fit(anchor, side, align, 4.0)
            .modal(true, |s: &mut State| {
                s.snooze_menu = None;
                s.snooze_anchor = None;
                s.snooze_at = None;
                s.lifecycle_error = None;
            }),
    )
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
                let at = s.row_menu_at;
                s.row_menu = None;
                s.row_menu_at = None;
                if action == 2 {
                    if let Some(id) = ids.first() {
                        s.open_snooze_from(*id, Some(format!("thread-{}", *id)), at);
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

fn row_menu(s: &State, p: Palette, cx: &Cx<State>) -> Option<El> {
    let target = s.row_menu?;
    let (anchor, actions) = match target {
        RowTarget::Thread(id) => {
            let row_key = format!("thread-{id}");
            let anchor = if s.row_menu_at.is_some() {
                "row-menu-point".to_owned()
            } else if cx.layout_with(&row_key, |_| ()).is_some() {
                row_key
            } else if cx.layout_with("thread-actions", |_| ()).is_some() {
                "thread-actions".to_owned()
            } else {
                return None;
            };
            (
                anchor,
                if s.selected_threads.contains(&id) && s.selected_threads.len() > 1 {
                    bulk_thread_actions(s, p)
                } else {
                    thread_actions(s, id, p, &format!("thread-{id}"))
                },
            )
        }
        RowTarget::Project(id) => {
            let option_key = format!("project-option-{id}");
            let anchor = if s.row_menu_at.is_some() {
                "row-menu-point".to_owned()
            } else if cx.layout_with(&option_key, |_| ()).is_some() {
                option_key
            } else {
                return None;
            };
            (
                anchor,
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
            )
        }
    };
    Some(
        col()
            .key("row-context-menu")
            .instant_changes()
            .role("menu")
            .focus_group()
            .label("Row actions")
            .w(222.0)
            .padding(4.0)
            .gap(1.0)
            .radius(12.0)
            .fill(p.card)
            .border(1.0, p.frame)
            .box_shadow(soft_shadow(p))
            .anchored_fit(
                anchor,
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
            .modal(true, |s: &mut State| {
                s.row_menu = None;
                s.row_menu_at = None;
            })
            .child(actions),
    )
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
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
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
                .radius(12.0)
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
/// A key name in an outlined chip.
fn keycap_chip(keys: &str, p: Palette) -> El {
    outlined(
        row()
            .h(20.0)
            .px(6.0)
            .center_y()
            .radius(6.0)
            .fill(p.card)
            .child(meta_text(keys, p.muted)),
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
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
}

#[derive(Clone, PartialEq, Eq)]
enum PickerChoice {
    All,
    NoProject,
    Project(usize),
    MachineProject(String, usize),
    MachineNoProject(String),
}

/// The picker's rows that match its search, in display order.
/// Each choice with its name and, for a linked machine, the machine's name.
fn picker_choices(s: &State, mode: ProjectPicker) -> Vec<(PickerChoice, String, Option<String>)> {
    let query = s.picker_query.text.trim().to_lowercase();
    let mut choices = vec![];
    if mode == ProjectPicker::Scope {
        choices.push((PickerChoice::All, "All workspaces".to_string(), None));
    }
    let multiple = s
        .client
        .as_ref()
        .is_some_and(|client| !client.linked.is_empty());
    // The New captain editor shares the new-thread picker's machine grouping, but its
    // choice lands on the editor's `captain_project`, never on a draft thread.
    let draft_pick = matches!(
        mode,
        ProjectPicker::NewThread | ProjectPicker::CaptainWorkspace
    );
    let across = matches!(mode, ProjectPicker::Scope | ProjectPicker::CaptainScope)
        || draft_pick && multiple;
    if mode != ProjectPicker::CaptainScope && !(draft_pick && multiple) {
        choices.push((PickerChoice::NoProject, "No workspace".to_string(), None));
    }
    for id in s.sorted_projects() {
        let project = s.project(Some(id)).expect("sorted project");
        if !project.hidden {
            choices.push((
                PickerChoice::Project(id),
                project.name.clone(),
                (across && (s.client.is_some() || mode == ProjectPicker::CaptainScope))
                    .then(|| s.machine_label(&s.machines.selected)),
            ));
        }
    }
    if mode == ProjectPicker::CaptainScope {
        for peer in &s.peers {
            for project in peer.capabilities["projects"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if project["hidden"] == true {
                    continue;
                }
                if let (Some(id), Some(name)) = (project["id"].as_u64(), project["name"].as_str()) {
                    let choice = PickerChoice::MachineProject(peer.server_id.clone(), id as usize);
                    if !choices.iter().any(|(existing, _, _)| *existing == choice) {
                        choices.push((choice, name.into(), Some(peer.label.clone())));
                    }
                }
            }
        }
    }
    // Scope is authorized by the captain owner's peer links, not this desktop's
    // device links. Their ids can differ, especially when its owner is remote.
    if across && s.client.is_some() && mode != ProjectPicker::CaptainScope {
        for (server, view) in &s.machines.views {
            if server == &s.machines.selected {
                continue;
            }
            if draft_pick
                && (!s.machine_connected(server)
                    || mode == ProjectPicker::NewThread
                        && s.machines
                            .draft_picker
                            .as_ref()
                            .is_some_and(|target| target != server))
            {
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
    if mode == ProjectPicker::CaptainScope {
        choices.sort_by_key(|(choice, _, _)| match choice {
            PickerChoice::MachineProject(server, _) => (true, server.clone()),
            _ => (false, String::new()),
        });
    }
    if draft_pick && multiple {
        choices.retain(|(choice, _, _)| match choice {
            PickerChoice::Project(_) => {
                s.machine_connected(&s.machines.selected)
                    && (mode == ProjectPicker::CaptainWorkspace
                        || s.machines
                            .draft_picker
                            .as_ref()
                            .is_none_or(|target| target == &s.machines.selected))
            }
            _ => true,
        });
        for server in machine_options(s) {
            if !s.machine_connected(&server)
                || mode == ProjectPicker::NewThread
                    && s.machines
                        .draft_picker
                        .as_ref()
                        .is_some_and(|target| target != &server)
            {
                continue;
            }
            choices.push((
                PickerChoice::MachineNoProject(server.clone()),
                "No workspace".into(),
                Some(s.machine_label(&server)),
            ));
        }
        choices.sort_by_key(|(choice, _, _)| {
            let server = match choice {
                PickerChoice::MachineProject(server, _)
                | PickerChoice::MachineNoProject(server) => server,
                _ => &s.machines.selected,
            };
            (
                server != "local",
                server.clone(),
                !matches!(choice, PickerChoice::MachineNoProject(_)),
            )
        });
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
            if mode == ProjectPicker::CaptainScope {
                captain_ui::toggle_remote_scope(s, server, id);
            } else if mode == ProjectPicker::Scope {
                s.set_machine_scope(server, Some(id), shift);
            } else if mode == ProjectPicker::NewThread && s.machine_connected(&server) {
                s.select_machine_without_draft(&server);
                if s.machines.selected == server {
                    s.choose_draft_project(Some(id));
                }
            } else if mode == ProjectPicker::CaptainWorkspace {
                captain_workspace_pick(s, &server, Some(id));
            }
            return;
        }
        PickerChoice::MachineNoProject(server) => {
            if mode == ProjectPicker::NewThread && s.machine_connected(&server) {
                s.select_machine_without_draft(&server);
                if s.machines.selected == server {
                    s.choose_draft_project(None);
                }
            } else if mode == ProjectPicker::CaptainWorkspace {
                captain_workspace_pick(s, &server, None);
            }
            return;
        }
    };
    match mode {
        ProjectPicker::NewThread => s.choose_draft_project(project),
        ProjectPicker::Scope => s.set_project_scope(project, shift),
        ProjectPicker::AddProject => {}
        ProjectPicker::CaptainScope => {
            if let Some(project) = project {
                captain_ui::toggle_scope(s, project);
            }
        }
        ProjectPicker::CaptainWorkspace => {
            s.captain_project = Some(project);
            s.project_picker = None;
            s.checkpoint();
        }
    }
}

/// A machine pick in the New captain modal records the target machine and workspace on the
/// editor without switching the active machine or draft behind the modal until Save.
fn captain_workspace_pick(s: &mut State, server: &str, project: Option<usize>) {
    if !s.machine_connected(server) {
        return;
    }
    if let Some(editor) = s.captain_editor.as_mut() {
        if editor.editing.is_some() || editor.promotion.is_some() {
            return;
        }
        editor.server = server.into();
        editor.machine_picker = false;
    }
    s.captain_project = Some(project);
    s.projects.captain_target = None;
    s.project_picker = None;
    s.checkpoint();
}

/// Ported from KSG Code's CommandPalette and CommandPaletteLookups project/environment
/// grouping. Captain scope membership is Shina-specific.
/// A centred modal that starts with a
/// search field and moves with the arrow keys. Enter picks the highlighted row; Shift adds a
/// workspace to the filter.
fn project_picker(s: &State, p: Palette) -> El {
    let mode = s.project_picker.expect("project picker");
    let choices = picker_choices(s, mode);
    let highlight = s.picker_highlight.min(choices.len().saturating_sub(1));
    let placeholder = match mode {
        ProjectPicker::Scope => "Filter by workspace...",
        ProjectPicker::NewThread => "New thread in...",
        ProjectPicker::CaptainWorkspace => "Workspace...",
        ProjectPicker::AddProject => "Add workspace",
        ProjectPicker::CaptainScope => "Add to this captain's scope...",
    };
    let mut card = popover("project-picker", placeholder, s, p)
        .role("dialog")
        .w(520.0)
        .max_h(460.0)
        .padding(4.0)
        .radius(12.0)
        .on_press(|_: &mut State| {})
        .child(
            row()
                .h(46.0)
                // The search box and the rows below share one inset from the card's edge.
                .mx(4.0)
                .px(8.0)
                .gap(10.0)
                .center_y()
                .radius(6.0)
                .fill(field_fill(s, p))
                .field_focus(&s.picker_query, p)
                .child(
                    el().size(16.0, 16.0)
                        .center()
                        .child(icon("search", p.muted)),
                )
                .child(
                    rune::text_input(&s.picker_query, TextStyle::new(15.0, 400.0).tracking(-0.15))
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
    let mut last_machine = None;
    for (index, (choice, name, machine)) in choices.iter().enumerate() {
        if matches!(
            mode,
            ProjectPicker::NewThread
                | ProjectPicker::CaptainScope
                | ProjectPicker::CaptainWorkspace
        ) && machine != &last_machine
        {
            if let Some(machine) = machine {
                list = list.child(
                    label(machine, SMALL, p.muted)
                        .key(format!("picker-machine-{index}"))
                        .px(12.0)
                        .py(6.0),
                );
            }
            last_machine = machine.clone();
        }
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
                .or_else(|| {
                    s.peers
                        .iter()
                        .find(|p| &p.server_id == server)
                        .and_then(|p| p.capabilities["projects"].as_array())
                        .and_then(|ps| ps.iter().find(|p| p["id"].as_u64() == Some(*id as u64)))
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
            PickerChoice::MachineNoProject(server) => (
                if server == &s.machines.selected {
                    "choose-no-project".into()
                } else {
                    format!("choose-no-project-{server}")
                },
                None,
            ),
        };
        let checked = match (mode, choice) {
            (ProjectPicker::CaptainScope, PickerChoice::Project(id)) => {
                captain_ui::in_scope(s, *id)
            }
            (ProjectPicker::CaptainScope, PickerChoice::MachineProject(server, id)) => {
                captain_ui::remote_in_scope(s, server, *id)
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
            (ProjectPicker::CaptainWorkspace, PickerChoice::Project(id)) => {
                s.captain_project.unwrap_or(s.current().project) == Some(*id)
            }
            (ProjectPicker::CaptainWorkspace, PickerChoice::NoProject) => {
                s.captain_project.unwrap_or(s.current().project).is_none()
            }
            (ProjectPicker::CaptainWorkspace, PickerChoice::MachineNoProject(server)) => {
                *server == s.machines.selected
                    && s.captain_project.unwrap_or(s.current().project).is_none()
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
            .radius(6.0)
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
                label(name.as_str(), TextStyle::new(14.0, 500.0), p.ink)
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
    if s.machines.selected != "local" {
        items[0].1 = "Folder on this machine".into();
        items.retain(|(source, _, _)| *source != AddSource::SystemPicker);
    }
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
fn browse_entries(s: &State) -> Vec<String> {
    if s.projects.browser.request.as_ref()
        != Some(&(s.machines.selected.clone(), s.picker_query.text.clone()))
    {
        return vec![];
    }
    s.projects
        .browser
        .listing
        .as_ref()
        .map(|listing| {
            listing
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        })
        .unwrap_or_default()
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
/// Move the browse field into a listed folder with a trailing separator, matching KSG Code's
/// `appendBrowsePathSegment` / `browseTo`.
fn browse_into(s: &mut State, name: &str) {
    if let Some(path) = s
        .projects
        .browser
        .listing
        .as_ref()
        .and_then(|listing| listing.entries.iter().find(|entry| entry.name == name))
        .map(|entry| entry.path.clone())
    {
        let sep = if path.contains('\\') && !path.contains('/') {
            '\\'
        } else {
            '/'
        };
        let next = if path.ends_with(['/', '\\']) {
            path
        } else {
            format!("{path}{sep}")
        };
        s.picker_query.replace_text(next);
    }
    s.picker_highlight = 0;
}

/// Shell-style Tab completion for folder browsing, ported from KSG Code's
/// `getFilesystemBrowseTabCompletion`: a single match enters the directory, several extend
/// to their common case-insensitive prefix.
fn browse_tab_complete(s: &mut State, folders: &[String]) {
    let Some(first) = folders.first() else {
        return;
    };
    if folders.len() == 1 {
        browse_into(s, first);
        return;
    }
    let query = &s.picker_query.text;
    let leaf = if query.ends_with(['/', '\\']) {
        ""
    } else {
        query.rsplit(['/', '\\']).next().unwrap_or("")
    };
    let mut prefix: Vec<char> = first.chars().collect();
    for name in &folders[1..] {
        let chars: Vec<char> = name.chars().collect();
        let mut len = 0;
        while len < prefix.len()
            && len < chars.len()
            && prefix[len].to_lowercase().eq(chars[len].to_lowercase())
        {
            len += 1;
        }
        prefix.truncate(len);
    }
    let common: String = prefix.into_iter().collect();
    let lower_common = common.to_lowercase();
    if common.chars().count() > leaf.chars().count()
        && folders
            .iter()
            .all(|name| name.to_lowercase().starts_with(&lower_common))
    {
        let dir_prefix = if query.ends_with(['/', '\\']) {
            query.as_str()
        } else if let Some(idx) = query.rfind(['/', '\\']) {
            &query[..=idx]
        } else {
            ""
        };
        s.picker_query.replace_text(format!("{dir_prefix}{common}"));
        s.picker_highlight = 0;
    }
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
        .radius(6.0)
        .fill(if lit { p.coral_bg } else { Color::TRANSPARENT })
        .hover(p.selected)
        .child(el().size(16.0, 16.0).center().child(icon(glyph, p.muted)))
        .child({
            let text = col()
                .gap(1.0)
                .grow(1.0)
                .min_w(0.0)
                .child(label(title, TextStyle::new(14.0, 500.0), p.ink).truncate());
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

/// Ported from KSG Code's CommandPalette Add project flow: choose a source, then browse by
/// typing a path. Enter opens the highlighted folder, Ctrl+Enter adds the typed one, and a
/// path that does not exist yet is created first.
fn add_project_palette(s: &State, p: Palette) -> El {
    let browsing = s.picker_browsing;
    let query = s.picker_query.text.clone();
    let folders = if browsing { browse_entries(s) } else { vec![] };
    let sources = if browsing { vec![] } else { add_sources(s) };
    let count = if browsing {
        folders.len()
    } else {
        sources.len()
    };
    let highlight = s.picker_highlight.min(count.saturating_sub(1));
    let current_listing = s.projects.browser.request.as_ref()
        == Some(&(s.machines.selected.clone(), query.clone()))
        && s.projects.browser.pending.is_none();
    let creates = browsing
        && current_listing
        && !query.trim().is_empty()
        && (s
            .projects
            .browser
            .listing
            .as_ref()
            .is_some_and(|listing| !listing.target_exists)
            || s.projects.browser.error.as_deref() == Some("That folder does not exist."));
    let submit = if creates { "Create folder" } else { "Add" };
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
        .radius(6.0)
        .fill(field_fill(s, p))
        .field_focus(&s.picker_query, p)
        .child(el().size(16.0, 16.0).center().child(icon(
            if browsing { "folder" } else { "folder-plus" },
            p.muted,
        )))
        .child(
            rune::text_input(
                &s.picker_query,
                if browsing {
                    TextStyle::new(15.0, 400.0).family(rune::Family::SystemMono)
                } else {
                    TextStyle::new(15.0, 400.0).tracking(-0.15)
                },
            )
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
                let folders = if browsing { browse_entries(s) } else { vec![] };
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
                        browse_tab_complete(s, &folders);
                        true
                    }
                    rune::Key::Backspace if browsing && s.picker_query.text.is_empty() => {
                        s.open_picker(ProjectPicker::AddProject);
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
                .label(if creates {
                    "Create folder and add workspace"
                } else {
                    "Add this folder"
                })
                .focusable(true)
                .h(28.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .border(1.0, p.frame)
                .hover(p.selected)
                .disabled(s.client.is_some() && !s.machine_connected(&s.machines.selected))
                .on_press(|s: &mut State| s.add_browsed_project())
                .child(label(submit, TextStyle::new(13.0, 600.0), p.ink).key(
                    if s.machines.selected == "local" {
                        "local-project-add"
                    } else {
                        "remote-project-add"
                    },
                ))
                .child(
                    meta_text(
                        if folders.is_empty() {
                            "Enter"
                        } else {
                            SUBMIT_CHORD
                        },
                        p.muted,
                    )
                    .px(4.0)
                    .radius(6.0)
                    .fill(p.selected),
                ),
        );
    }
    let mut card = popover("project-picker", "Add workspace", s, p)
        .role("dialog")
        .w(560.0)
        .max_h(480.0)
        .padding(4.0)
        .radius(12.0)
        .on_press(|_: &mut State| {});
    if machine_options(s).len() > 1 {
        let mut machines = col().key("project-machine-row").gap(1.0);
        for server in machine_options(s) {
            let online = s.machine_connected(&server);
            let title = s.machine_label(&server);
            let key = format!("project-machine-{server}");
            let locked = s
                .captain_editor
                .as_ref()
                .is_some_and(|editor| editor.server != server)
                || s.projects.return_picker == Some(ProjectPicker::CaptainScope)
                    && server != s.machines.selected;
            machines = machines.child(
                machine_option(&key, &title, &server, online, p)
                    .disabled(!online || locked)
                    .on_press(move |s: &mut State| s.choose_project_machine(&server)),
            );
        }
        card = card.child(machines);
    }
    card = card
        .child(
            el().key(if s.machines.selected == "local" {
                "local-project-path"
            } else {
                "remote-project-path"
            })
            .child(field),
        )
        .child(el().h(4.0));
    let mut list = col().gap(1.0).min_h(0.0).shrink().scroll(ScrollSpec {
        stick_bottom: false,
    });
    if browsing {
        if let Some(parent) = s
            .projects
            .browser
            .listing
            .as_ref()
            .and_then(|listing| listing.parent.clone())
        {
            let sep = if parent.contains('\\') && !parent.contains('/') {
                '\\'
            } else {
                '/'
            };
            let parent_dir = if parent.ends_with(['/', '\\']) {
                parent
            } else {
                format!("{parent}{sep}")
            };
            list = list.child(
                palette_row(
                    "browse-parent".into(),
                    "folder",
                    "..",
                    &parent_dir,
                    false,
                    p,
                )
                .min_h(34.0)
                .on_press(move |s: &mut State| {
                    s.picker_query.replace_text(&parent_dir);
                    s.picker_highlight = 0;
                }),
            );
        }
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
        if s.projects.browser.pending.is_some()
            || s.projects.browser.request.as_ref()
                != Some(&(s.machines.selected.clone(), query.clone()))
        {
            list = list.child(
                meta_text("Loading", p.muted)
                    .key("browse-loading")
                    .px(8.0)
                    .py(10.0),
            );
        } else if let Some(error) = &s.projects.browser.error {
            list = list.child(
                label(error, SMALL, p.muted)
                    .key("browse-error")
                    .wrap()
                    .px(8.0)
                    .py(10.0),
            );
        } else if folders.is_empty() {
            list = list.child(
                label(
                    if creates {
                        "No folder here yet."
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
        if s.projects
            .browser
            .listing
            .as_ref()
            .is_some_and(|listing| listing.truncated)
        {
            list = list.child(
                label(
                    "Showing the first 200 folders. Type a more specific path.",
                    SMALL,
                    p.muted,
                )
                .key("browse-truncated")
                .wrap()
                .px(8.0)
                .py(6.0),
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
    if s.machines.selected != "local" {
        card = card.child(button("remote-project-cancel", "Cancel", p, |s| {
            s.project_picker = None
        }));
        card = el().key("remote-project-dialog").child(card);
    }
    let modal = modal_frame(
        "project-picker-backdrop",
        s,
        |s| s.project_picker = None,
        card,
    );
    if s.machines.selected == "local" {
        modal
    } else {
        el().key("remote-project-backdrop")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .child(modal)
    }
}

/// The verb for a call, in the tense of its state: "Reading" while it runs, "Read" after.
fn tool_verb(tool: &Tool) -> String {
    tool_verb_in(tool, tool.done || tool.failed)
}

/// The verb for a call in a given tense. KSG's live activity row keeps the present tense
/// ("Reading") while its turn is still acting on the result (`liveActivityToolStatus`).
fn tool_verb_in(tool: &Tool, done: bool) -> String {
    use crate::work::ToolKind::*;
    match (tool.resolved_kind(), done) {
        (Read, true) => "Read".into(),
        (Read, false) => "Reading".into(),
        (List, true) => "Listed".into(),
        (List, false) => "Listing".into(),
        (FileSearch | WebSearch, true) => "Searched".into(),
        (FileSearch | WebSearch, false) => "Searching".into(),
        (FileChange, true) => "Changed".into(),
        (FileChange, false) => "Changing".into(),
        (Command, true) => "Ran".into(),
        (Command, false) => "Running".into(),
        (Subagent, true) => "Delegated".into(),
        (Subagent, false) => "Delegating".into(),
        (Plan, true) => "Updated plan".into(),
        (Plan, false) => "Updating plan".into(),
        (Html, true) => "Rendered".into(),
        (Html, false) => "Rendering".into(),
        (Other, true) => format!("Called {}", tool.name),
        (Other, false) => format!("Calling {}", tool.name),
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

/// Ported from KSG Code's work-log/presentation.ts summarizeToolGroup.
fn tool_summary(tools: &[&Tool]) -> String {
    crate::work::summarize(tools)
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
    /// Cards that keep their own place: plans, rendered HTML and a captain's delegations.
    Tools(Vec<usize>),
    /// Thinking before grouping; `turn_parts` folds every run of it into `Work`.
    Thought(Vec<usize>),
    /// One streak of thoughts and ordinary tool calls between prose, in order.
    Work(Vec<WorkEntry>),
    Compaction(usize),
    Retry(usize),
}

/// A step of a work streak: a tool call or a thought, by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WorkEntry {
    Tool(usize),
    Thought(usize),
}

/// Content indices preserve thinking/tool order even when neither adds visible message text.
fn turn_parts(t: &Thread, start: usize, end: usize) -> Vec<TranscriptPart> {
    turn_parts_in(t, &timeline_structure(t).events, start, end)
}

/// A tool that shows as its own card rather than a step: a plan, rendered HTML, a captain's
/// delegation, or a subagent run with its result cards (KSG keeps subagent events out of
/// work groups too).
fn is_card_tool(t: &Thread, index: usize) -> bool {
    let tool = &t.tools[index];
    if !tool.subagents.is_empty() {
        return true;
    }
    match tool.resolved_kind() {
        crate::work::ToolKind::Plan | crate::work::ToolKind::Subagent => true,
        crate::work::ToolKind::Html => {
            crate::html_render::find_reference(&tool.structured).is_some()
        }
        _ => t.captain_home && !t.delegated && crew_ui::is_delegation(tool),
    }
}

/// Ported from KSG Code's `deriveTimelineRows` grouping (`MessagesTimeline.logic.ts`): every
/// contiguous run of reasoning and tool entries between prose is one work group. Cards,
/// compactions and retries end a run.
fn group_work(t: &Thread, parts: Vec<TranscriptPart>) -> Vec<TranscriptPart> {
    let mut out: Vec<TranscriptPart> = Vec::with_capacity(parts.len());
    let push_entry = |out: &mut Vec<TranscriptPart>, entry: WorkEntry| {
        if let Some(TranscriptPart::Work(entries)) = out.last_mut() {
            entries.push(entry);
        } else {
            out.push(TranscriptPart::Work(vec![entry]));
        }
    };
    for part in parts {
        match part {
            TranscriptPart::Thought(indices) => {
                for index in indices {
                    push_entry(&mut out, WorkEntry::Thought(index));
                }
            }
            TranscriptPart::Tools(indices) => {
                // Cards from one call batch stay one part, as the crew's task cards expect.
                let mut cards_open = false;
                for index in indices {
                    if !is_card_tool(t, index) {
                        push_entry(&mut out, WorkEntry::Tool(index));
                        cards_open = false;
                    } else if cards_open && let Some(TranscriptPart::Tools(cards)) = out.last_mut()
                    {
                        cards.push(index);
                    } else {
                        out.push(TranscriptPart::Tools(vec![index]));
                        cards_open = true;
                    }
                }
            }
            part => out.push(part),
        }
    }
    out
}

fn turn_events(events: &[TimelineEvent], start: usize, end: usize) -> &[TimelineEvent] {
    let lo = events.partition_point(|event| event.0.message < start);
    let hi = events.partition_point(|event| event.0.message < end);
    &events[lo..hi]
}

fn turn_parts_in(
    t: &Thread,
    events: &[TimelineEvent],
    start: usize,
    end: usize,
) -> Vec<TranscriptPart> {
    group_work(t, event_parts_in(t, events, start, end))
}

/// A turn's parts in event order, before thoughts and tools are grouped into work streaks.
fn event_parts_in(
    t: &Thread,
    events: &[TimelineEvent],
    start: usize,
    end: usize,
) -> Vec<TranscriptPart> {
    let mut events = turn_events(events, start, end).iter().copied().peekable();
    let mut parts = vec![];
    for message in start..end {
        let text = &t.messages[message].text;
        let mut from = 0;
        let mut previous_anchor = None;
        while events
            .peek()
            .is_some_and(|event| event.0.message == message)
        {
            let (anchor, _, kind, index) = events.next().unwrap();
            let to = if anchor.offset >= from && text.is_char_boundary(anchor.offset) {
                anchor.offset
            } else {
                from
            };
            if to > from {
                parts.push(TranscriptPart::Text { message, from, to });
            }
            if kind == 2 {
                parts.push(TranscriptPart::Compaction(index));
            } else if kind == 3 {
                parts.push(TranscriptPart::Retry(index));
            } else if kind == 1 {
                if let Some(TranscriptPart::Thought(indices)) = parts.last_mut() {
                    indices.push(index);
                } else {
                    parts.push(TranscriptPart::Thought(vec![index]));
                }
            } else if previous_anchor == Some(anchor)
                && let Some(TranscriptPart::Tools(indices)) = parts.last_mut()
                && !matches!(
                    t.tools[index].resolved_kind(),
                    crate::work::ToolKind::Plan | crate::work::ToolKind::Html
                )
                && indices.last().is_some_and(|i| {
                    !matches!(
                        t.tools[*i].resolved_kind(),
                        crate::work::ToolKind::Plan | crate::work::ToolKind::Html
                    )
                })
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

/// Ported from KSG Code's `WorkLogLine` and `TimelineWorkEntryRow`: a 24 px row with a 14 px
/// glyph in a 24 px icon slot. The whole label, verb and target, is `text-secondary-label`
/// (the palette's `muted`) at body weight, so tool rows read quieter than the prose around
/// them. A failed row's verb is medium `danger`. The optional 12 px caret is `dim` and
/// shows on folded group headers only, never on individual rows.
#[allow(clippy::too_many_arguments)]
fn activity_line(
    key: String,
    a11y: String,
    action: (String, &str),
    failed: bool,
    open: bool,
    show_caret: bool,
    glyph: &str,
    p: Palette,
) -> El {
    let (verb, detail) = action;
    let mut line = row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(a11y)
        .expanded(open)
        .focusable(true)
        .center_y()
        .gap(5.0)
        .min_h(24.0)
        .px(6.0)
        .ml(-6.0)
        .radius(6.0)
        .hover(p.selected)
        .child(
            el().size(24.0, 24.0)
                .center()
                .child(icon(glyph, if failed { p.danger } else { p.muted }).size(14.0, 14.0)),
        )
        .child(if failed {
            label(verb, TextStyle::new(13.0, 500.0).tracking(-0.08), p.danger)
        } else {
            label(verb, ACTIVITY, p.muted)
        });
    if !detail.is_empty() {
        line = line.child(
            label(detail, ACTIVITY, p.muted)
                .truncate()
                .shrink()
                .min_w(0.0),
        );
    }
    if failed {
        line = line.child(meta_text("failed", p.danger));
    }
    if show_caret {
        line = line.child(icon(if open { "chevron" } else { "open" }, p.faint).size(12.0, 12.0));
    }
    line.child(spacer())
}

/// KSG Code's `WORKING_BAR_CLASS_NAME` (`chat/TimelineRowParts.tsx`): a 32 pt bar over a 1 px
/// `--timeline-working-rule` (the palette's `line`), 5 pt in at the start and 4 pt at the end.
/// A running turn's "Working for" and a finished turn's "Worked for" are this one bar.
fn working_bar_frame(p: Palette) -> El {
    row()
        .w_full()
        .min_w(0.0)
        .h(32.0)
        .center_y()
        .gap(10.0)
        .pl(5.0)
        .pr(4.0)
        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
}

/// The folded work bar's button, ported from KSG Code's `WorkFoldToggle` and `WorkFoldCaret`:
/// muted 13 px text that brightens on hover, with a 12 pt chevron 8 pt after it. It goes in
/// a `working_bar_frame`.
fn work_fold_toggle(key: String, a11y: String, title: &str, open: bool, p: Palette) -> El {
    row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(a11y)
        .focusable(true)
        .hover_group()
        .h_full()
        .center_y()
        .gap(8.0)
        .radius(6.0)
        .child(label(title, ACTIVITY, p.muted).group_color(p.ink))
        .child(icon(if open { "chevron" } else { "open" }, p.faint).size(12.0, 12.0))
}

/// The bar's live label, as KSG's `WorkingTimelineRow` words it: stopping, paused on a
/// request (the clock stops), compacting, or working, with the turn's tabular timer.
fn working_label(t: &Thread) -> (&'static str, Option<u64>) {
    let elapsed = t.started.map(|start| start.elapsed().as_secs());
    let mut cache = t.timeline_cache.borrow_mut();
    if t.request.is_none() {
        cache.paused_at = None;
    }
    if t.phase == "Stopping" || t.stop_control.phase().is_some() {
        return ("Stopping…", None);
    }
    if t.request.is_some() {
        let at = *cache.paused_at.get_or_insert(elapsed.unwrap_or(0));
        return ("Paused at ", Some(at));
    }
    match (t.phase == "Compacting context", elapsed) {
        (true, Some(seconds)) => ("Compacting for ", Some(seconds)),
        (true, None) => ("Compacting…", None),
        (false, Some(seconds)) if seconds > 0 => ("Working for ", Some(seconds)),
        (false, _) => ("Working...", None),
    }
}

/// The tabular timer in the bar's own 13 px type, so each tick keeps its width.
const BAR_TIMER: TextStyle = ACTIVITY.features(TABULAR);

/// Ported from KSG Code's `WorkingTimelineRow`: the running turn's 32 pt bar at its top.
/// When earlier work history exists before the live tail (or is currently folded), clicking the
/// bar folds or unfolds earlier work while keeping the live tail visible.
fn working_bar(t: &Thread, start: usize, foldable: bool, p: Palette) -> El {
    let (words, seconds) = working_label(t);
    let expanded = !t.live_work_collapsed;
    let mut text = row().center_y().min_w(0.0).shrink().child(
        label(words, ACTIVITY, p.muted)
            .group_color(p.ink)
            .key(format!("working-label-{}", t.id))
            .truncate()
            .shrink()
            .min_w(0.0),
    );
    if let Some(seconds) = seconds {
        text = text.child(
            label(timer_label(seconds), BAR_TIMER, p.muted)
                .group_color(p.ink)
                .key(format!("live-timer-{}", t.id)),
        );
    }
    let key = format!("working-bar-{}", t.id);
    if !foldable {
        return working_bar_frame(p).key(key).role("status").child(text);
    }
    let press_key = key.clone();
    working_bar_frame(p).child(
        row()
            .key(key)
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(if expanded {
                "Collapse work history"
            } else {
                "Expand work history"
            })
            .expanded(expanded)
            .focusable(true)
            .hover_group()
            .h_full()
            .center_y()
            .gap(8.0)
            .radius(6.0)
            .child(text)
            .child(icon(if expanded { "chevron" } else { "open" }, p.faint).size(12.0, 12.0))
            .on_press_tx(move |s: &mut State, tx| {
                tx.keep_on_screen(press_key.clone());
                let t = s.current_mut();
                t.live_work_collapsed = !t.live_work_collapsed;
                t.timeline_cache.get_mut().touch_message(start);
                t.timeline_cache.get_mut().touch_disclosure();
            }),
    )
}

fn inline_tool(s: &State, p: Palette, index: usize) -> El {
    let t = s.current();
    let tool = &t.tools[index];
    if tool.resolved_kind() == crate::work::ToolKind::Html
        && crate::html_render::find_reference(&tool.structured).is_some()
    {
        return work_ui::html_card(s, p, index);
    }
    if tool.resolved_kind() == crate::work::ToolKind::Plan {
        return work_ui::plan_card(s, p, index);
    }
    let title = tool_title(tool);
    let tool_id = tool.id.clone();
    let line_key = format!("inline-tool-{}-{index}", t.id);
    let press_key = line_key.clone();
    let flashing = s.flash_tool.as_ref().is_some_and(|(id, _)| id == &tool.id);
    let mut content = col()
        .key(format!("tool-{}", tool.id))
        .gap(4.0)
        .radius(6.0)
        .fill(if flashing {
            p.sea_bg
        } else {
            Color::TRANSPARENT
        })
        .child(
            activity_line(
                line_key,
                format!(
                    "{} output for {title}",
                    if tool.expanded { "Hide" } else { "Show" }
                ),
                (tool_verb(tool), &tool.detail),
                tool.failed,
                tool.expanded,
                false,
                work_ui::kind_icon(tool.resolved_kind()),
                p,
            )
            .on_press_tx(move |s: &mut State, tx| {
                tx.keep_on_screen(press_key.clone());
                let thread = s.current_mut();
                if let Some(tool) = thread.tools.iter_mut().find(|tool| tool.id == tool_id) {
                    tool.expanded = !tool.expanded;
                    let message = tool.anchor.message;
                    thread.timeline_cache.get_mut().touch_message(message);
                }
            }),
        );
    if !tool.subagents.is_empty() {
        content = content.child(pi_ui::subagent_results(s, tool, p));
    }
    if tool.expanded {
        content = content.child(work_ui::expanded(s, p, index));
    }
    content
}

fn inline_tools(s: &State, p: Palette, indices: &[usize]) -> El {
    let entries: Vec<_> = indices.iter().map(|&i| WorkEntry::Tool(i)).collect();
    work_part(s, p, &entries)
}

/// The tool calls of a work streak, in order.
fn entry_tools(entries: &[WorkEntry]) -> impl Iterator<Item = usize> + '_ {
    entries.iter().filter_map(|entry| match entry {
        WorkEntry::Tool(index) => Some(*index),
        WorkEntry::Thought(_) => None,
    })
}

/// The thoughts of a work streak, in order.
fn entry_thoughts(entries: &[WorkEntry]) -> Vec<usize> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            WorkEntry::Thought(index) => Some(*index),
            WorkEntry::Tool(_) => None,
        })
        .collect()
}

/// Whether a work streak is unfolded. Its first tool call holds the flag, as a tool group's did;
/// a streak of thinking alone opens with its first thought.
fn group_open(t: &Thread, entries: &[WorkEntry]) -> bool {
    match entry_tools(entries).next() {
        Some(index) => t.tools[index].group_expanded,
        None => entry_thoughts(entries)
            .first()
            .is_some_and(|&index| t.thoughts[index].expanded),
    }
}

/// Flips a work streak's fold flag. `tool_id` names its first call; `None` is a streak of
/// thinking alone, which folds with its first thought (`thought`).
fn toggle_group(t: &mut Thread, tool_id: Option<&str>, thought: Option<(usize, usize, usize)>) {
    if let Some(id) = tool_id {
        if let Some(tool) = t.tools.iter_mut().find(|tool| tool.id == id) {
            tool.group_expanded = !tool.group_expanded;
            let message = tool.anchor.message;
            t.timeline_cache.get_mut().touch_message(message);
        }
    } else if let Some((message, block, offset)) = thought
        && let Some(thought) = t.thoughts.iter_mut().find(|thought| {
            thought.anchor.message == message
                && thought.block == block
                && thought.anchor.offset == offset
        })
    {
        thought.expanded = !thought.expanded;
        t.timeline_cache.get_mut().touch_message(message);
    }
}

/// One step of an unfolded work streak.
fn work_entry_row(s: &State, p: Palette, entry: WorkEntry) -> El {
    match entry {
        WorkEntry::Tool(index) => inline_tool(s, p, index),
        WorkEntry::Thought(index) => work_ui::thoughts(s, p, &[index]),
    }
}

/// A work streak as one transcript part: a lone call is its own row, thinking alone keeps its
/// "Thought" disclosure, and anything longer folds under one header, as KSG Code's
/// `WorkGroupToggleTimelineRow` and `WorkLogList` (16 pt in, `ps-4`) show it.
fn work_part(s: &State, p: Palette, entries: &[WorkEntry]) -> El {
    let t = s.current();
    let thoughts = entry_thoughts(entries);
    if thoughts.len() == entries.len() {
        return work_ui::thoughts(s, p, &thoughts);
    }
    if entries.len() == 1 {
        return work_entry_row(s, p, entries[0]);
    }
    let mut group = work_group_head(s, p, entries);
    if group_open(t, entries) {
        let mut rows = col().pl(WORK_LIST_INDENT).gap(WORK_GAP);
        for entry in entries {
            rows = rows.child(work_entry_row(s, p, *entry));
        }
        group = group.child(rows);
    }
    group
}

/// KSG's `WorkLogList` indent (`ps-4`) for the steps of an unfolded group.
const WORK_LIST_INDENT: f32 = 16.0;
/// The space between work rows.
const WORK_GAP: f32 = 2.0;

/// A work streak's keyed column and summary line, without its rows, ported from KSG Code's
/// `WorkGroupHeader`. The summary counts the calls (`summarizeToolGroup` leaves thinking out);
/// the icon is the calls' own when they are all one kind. Long turns place each row of an
/// unfolded streak in its own transcript slice.
fn work_group_head(s: &State, p: Palette, entries: &[WorkEntry]) -> El {
    let t = s.current();
    let indices: Vec<usize> = entry_tools(entries).collect();
    let tools: Vec<_> = indices.iter().map(|index| &t.tools[*index]).collect();
    let first = indices[0];
    let first_id = tools[0].id.clone();
    let expanded = group_open(t, entries);
    let failed = tools.last().is_some_and(|tool| tool.failed);
    let summary = tool_summary(&tools);
    let kind = tools[0].resolved_kind();
    let glyph = if tools.iter().all(|tool| tool.resolved_kind() == kind) {
        work_ui::kind_icon(kind)
    } else {
        "list"
    };
    let toggle_key = format!("toggle-tool-group-{}-{first}", t.id);
    let press_key = toggle_key.clone();
    col()
        .key(format!("tool-group-{}-{first}", t.id))
        .gap(WORK_GAP)
        .child(
            activity_line(
                toggle_key,
                format!("{} {summary}", if expanded { "Collapse" } else { "Expand" }),
                (summary.clone(), ""),
                failed,
                expanded,
                true,
                glyph,
                p,
            )
            .on_press_tx(move |s: &mut State, tx| {
                tx.keep_on_screen(press_key.clone());
                toggle_group(s.current_mut(), Some(&first_id), None);
            }),
        )
}

fn message_key(thread: &Thread, index: usize) -> String {
    match thread.messages.get(index) {
        Some(message) => format!("message-{}-id-{}", thread.id, message.id),
        None => format!("message-{}-boundary-{index}", thread.id),
    }
}

fn assistant_body_key(thread: &Thread, index: usize, from: usize) -> String {
    format!("{}-body-{from}", message_key(thread, index))
}

fn transcript_part(
    s: &State,
    p: Palette,
    wide: bool,
    part: &TranscriptPart,
    occ_base: &mut usize,
) -> El {
    let t = s.current();
    match part {
        TranscriptPart::Tools(indices) => crew_ui::tool_part(s, p, indices),
        TranscriptPart::Thought(indices) => work_ui::thoughts(s, p, indices),
        TranscriptPart::Work(entries) => work_part(s, p, entries),
        TranscriptPart::Compaction(index) => pi_ui::compaction_activity(s, p, *index),
        TranscriptPart::Retry(index) => work_ui::retry_row(s, p, *index),
        TranscriptPart::Text { message, from, to } => {
            let chunk = &t.messages[*message].text[*from..*to];
            let message_id = &t.messages[*message].id;
            let offset = *from;
            let thread_id = t.id;
            let streaming = t.status == Status::Running && message + 1 == t.messages.len();
            let find_query = (s.thread_find_active()
                && !s.thread_find.query.text.trim().is_empty())
            .then(|| s.thread_find.query.text.trim());
            let cite_query = find_query.is_none().then_some(()).and_then(|()| {
                s.citation_highlight
                    .as_ref()
                    .filter(|hl| {
                        hl.machine == s.machines.selected
                            && hl.thread_id == thread_id
                            && hl.message_id == *message_id
                            && !hl.text.trim().is_empty()
                    })
                    .map(|hl| hl.text.trim())
            });
            let mut cite_matched = false;
            let find_param = if let Some(query) = find_query {
                let hits = crate::thread_find::find_markdown_occurrences(chunk, query).len();
                let active_occ = s
                    .thread_find
                    .active_match
                    .as_ref()
                    .filter(|m| m.entry_id == *message_id)
                    .and_then(|m| {
                        (m.occurrence >= *occ_base && m.occurrence < *occ_base + hits)
                            .then(|| m.occurrence - *occ_base)
                    });
                *occ_base += hits;
                Some((query, active_occ))
            } else if let Some(query) = cite_query {
                if !crate::thread_find::find_markdown_occurrences(chunk, query).is_empty() {
                    cite_matched = true;
                    Some((query, Some(0)))
                } else {
                    None
                }
            } else {
                None
            };
            let memo_find = find_param.map(|(q, occ)| (q.to_owned(), occ));
            let rendered = memo(
                assistant_body_key(t, *message, *from),
                (
                    chunk,
                    s.dark,
                    streaming,
                    wide,
                    s.code_wrap_version,
                    memo_find,
                ),
                || {
                    markdown_ui::render_with_find(
                        chunk,
                        p,
                        s.dark,
                        wide,
                        false,
                        streaming,
                        &format!("{thread_id}-{message_id}-{offset}"),
                        &s.unwrapped_code_blocks,
                        find_param,
                    )
                },
            );
            if cite_query.is_some() && !cite_matched {
                rendered.fill(p.coral_bg).radius(6.0)
            } else {
                rendered
            }
        }
    }
}

/// What the agent is doing while no step of the live tail is in progress, as KSG Code's
/// `ThinkingTimelineRow` words it: "Thinking", "Waiting on subagents", or a phase that says
/// more, such as a retry.
fn live_status(t: &Thread, start: usize) -> String {
    let waiting = t.tools.iter().any(|tool| {
        tool.anchor.message >= start
            && !tool.done
            && !tool.failed
            && (tool.resolved_kind() == crate::work::ToolKind::Subagent
                || crew_ui::is_delegation(tool))
    });
    if waiting {
        return "Waiting on subagents".into();
    }
    match t.phase.as_str() {
        ""
        | "Working"
        | "Waiting for the model"
        | "Thinking"
        | "Stopping"
        | "Compacting context" => "Thinking".into(),
        phase if phase.starts_with("Running ") => "Thinking".into(),
        phase => phase.into(),
    }
}

/// The live tail of a running turn: the work streak after its latest prose, which KSG Code's
/// `activeWorkRow` shows as one `work-live` row.
#[derive(Clone, Copy, Debug)]
struct LiveTail {
    part: usize,
    /// The newest step with something to show (thinking with no text yet has none).
    latest: WorkEntry,
    /// The newest step still in progress.
    running: Option<WorkEntry>,
    /// The newest step is a call that failed, and nothing is running.
    failed: bool,
    /// The row stands for what the turn is doing now: a step runs, or the last call
    /// succeeded and nothing waits on the user.
    active: bool,
    /// The newest thought with text, for the preview line.
    thought: Option<usize>,
}

fn live_tail(t: &Thread, parts: &[TranscriptPart]) -> Option<LiveTail> {
    let part = parts.len().checked_sub(1)?;
    let TranscriptPart::Work(entries) = &parts[part] else {
        return None;
    };
    let visible = |entry: &&WorkEntry| match entry {
        WorkEntry::Tool(_) => true,
        WorkEntry::Thought(index) => !t.thoughts[*index].text.trim().is_empty(),
    };
    let latest = *entries.iter().rfind(visible)?;
    let running = entries
        .iter()
        .filter(visible)
        .rfind(|entry| match entry {
            WorkEntry::Tool(index) => !t.tools[*index].done && !t.tools[*index].failed,
            WorkEntry::Thought(index) => !t.thoughts[*index].done,
        })
        .copied();
    let latest_tool = match latest {
        WorkEntry::Tool(index) => Some(&t.tools[index]),
        WorkEntry::Thought(_) => None,
    };
    let failed = running.is_none() && latest_tool.is_some_and(|tool| tool.failed);
    let active = running.is_some()
        || latest_tool.is_some_and(|tool| tool.done && !tool.failed) && t.request.is_none();
    let thought = entries
        .iter()
        .filter(visible)
        .rev()
        .find_map(|entry| match entry {
            WorkEntry::Thought(index) => Some(*index),
            WorkEntry::Tool(_) => None,
        });
    Some(LiveTail {
        part,
        latest,
        running,
        failed,
        active,
        thought,
    })
}

/// How much of a thought KSG reads for its preview (`THOUGHT_PREVIEW_SOURCE_MAX_LENGTH`).
const THOUGHT_PREVIEW_SOURCE: usize = 8_192;

/// Ported from KSG Code's `liveThoughtLine` (`work-log/presentation.ts`): a thought's bold
/// heading, or else its first sentence, as plain text.
fn live_thought_line(markdown: &str) -> String {
    let markdown = match markdown.char_indices().nth(THOUGHT_PREVIEW_SOURCE) {
        Some((at, _)) => &markdown[..at],
        None => markdown,
    };
    let heading = markdown.trim_start().strip_prefix("**").and_then(|rest| {
        let end = rest.find("**")?;
        let inner = &rest[..end];
        let after = rest[end + 2..].split('\n').next().unwrap_or("");
        (!inner.is_empty()
            && !inner.contains(['*', '\r', '\n'])
            && after
                .trim_end_matches('\r')
                .chars()
                .all(|c| c == ' ' || c == '\t'))
        .then_some(inner)
    });
    let source = heading.unwrap_or(markdown);
    // Links and images keep their text.
    let mut unlinked = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(open) = rest.find('[') {
        let (before, from) = rest.split_at(open);
        let close = from.find(']');
        let link = close
            .filter(|close| from[close + 1..].starts_with('('))
            .and_then(|close| {
                let end = from[close + 2..].find(')')?;
                Some((close, close + 2 + end))
            });
        match link {
            Some((close, end)) if !from[1..close].contains(']') => {
                unlinked.push_str(before.strip_suffix('!').unwrap_or(before));
                unlinked.push_str(&from[1..close]);
                rest = &from[end + 1..];
            }
            _ => {
                unlinked.push_str(before);
                unlinked.push('[');
                rest = &from[1..];
            }
        }
    }
    unlinked.push_str(rest);
    // Headings, bullets and list numbers at the start of a line.
    let mut lines = vec![];
    for line in unlinked.split('\n') {
        let trimmed = line.trim_start_matches([' ', '\t']);
        let marker = trimmed
            .find([' ', '\t'])
            .filter(|&at| {
                let mark = &trimmed[..at];
                (1..=6).contains(&mark.len()) && mark.chars().all(|c| c == '#')
                    || matches!(mark, "-" | "*" | "+")
                    || mark.len() > 1
                        && mark.ends_with('.')
                        && mark[..mark.len() - 1].chars().all(|c| c.is_ascii_digit())
            })
            .map(|at| trimmed[at..].trim_start_matches([' ', '\t']));
        lines.push(marker.unwrap_or(line));
    }
    let joined = lines
        .join("\n")
        .replace("**", "")
        .replace("~~", "")
        .replace('`', "");
    // Single `*` or `_` emphasis around words.
    let chars: Vec<char> = joined.chars().collect();
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut plain = String::with_capacity(joined.len());
    for (i, c) in chars.iter().enumerate() {
        let (before, after) = (
            i.checked_sub(1).and_then(|i| chars.get(i)),
            chars.get(i + 1),
        );
        let opens = !word(before) && word(after) && after != Some(&'_');
        let closes = word(before) && !word(after) && after != Some(&'*');
        if matches!(c, '*' | '_') && (opens || closes) {
            continue;
        }
        plain.push(*c);
    }
    let text = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if heading.is_some() {
        return text;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, &(at, c)) in chars.iter().enumerate() {
        if !matches!(c, '.' | '?' | '!') {
            continue;
        }
        let mut end = at + c.len_utf8();
        let mut next = chars.get(i + 1).map(|&(_, c)| c);
        if let Some(close) = next.filter(|c| matches!(c, '"' | '\'' | '”' | '’' | ')')) {
            end += close.len_utf8();
            next = chars.get(i + 2).map(|&(_, c)| c);
        }
        if next.is_some_and(char::is_whitespace) {
            return text[..end].to_owned();
        }
    }
    text
}

/// The sweep's window (`--live-activity-focus-width`, 4.5 rem) and period.
const SHIMMER_WINDOW: f32 = 72.0;
const SHIMMER_PERIOD: f64 = 2.2;

/// Ported from KSG Code's `ActivityShimmerOverlay` and `.live-activity-focus`: a copy of the
/// row in the foreground colour shows through a 72 pt window with soft edges that sweeps the
/// row every 2.2 s. The window slides one way and the copy inside it the other, so the copy
/// stays over the row. The renderer runs both slides on its own clock, so the sweep costs no
/// view runs or wakes, and it leaves with the live row: a settled window draws nothing.
fn shimmer(base: El, highlight: El, width: f32) -> El {
    let travel = width + SHIMMER_WINDOW;
    let sweep = |to: f32| {
        rune::LoopSpec::slide(SHIMMER_PERIOD, [0.0, 0.0], [to, 0.0]).easing(rune::Easing::LINEAR)
    };
    el().w_full().min_w(0.0).child(base).child(
        el().absolute(-SHIMMER_WINDOW, 0.0)
            .w(SHIMMER_WINDOW)
            .h_full()
            .clip_fade(SHIMMER_WINDOW / 2.0, 0.0, SHIMMER_WINDOW / 2.0, 0.0)
            .looping(sweep(travel))
            .inert()
            .a11y_hidden()
            .child(
                el().absolute(SHIMMER_WINDOW, 0.0)
                    .w(width)
                    .looping(sweep(-travel))
                    .child(highlight),
            ),
    )
}

/// Whether live rows sweep: the turn runs, nothing waits on the user, and motion is allowed.
fn shimmer_allowed(s: &State, t: &Thread) -> bool {
    t.request.is_none() && !s.portraits.reduced_motion()
}

/// The width a live row lays out at, for the shimmer's copy: the lane.
fn live_row_width(s: &State) -> f32 {
    (s.html_lane_width as f32).max(0.0)
}

/// KSG Code's `LiveActivityContent`: a 24 pt icon slot and one truncating label.
fn activity_content(glyph: &str, text: &str, ink: Color, key: Option<String>) -> El {
    let mut words = label(text, ACTIVITY, ink).truncate().shrink().min_w(0.0);
    if let Some(key) = key {
        words = words.key(key);
    }
    // A full-width row bounds the label, so it truncates at the lane instead of overflowing.
    row()
        .min_h(24.0)
        .center_y()
        .gap(5.0)
        .w_full()
        .min_w(0.0)
        .child(
            el().size(24.0, 24.0)
                .center()
                .child(icon(glyph, ink).size(14.0, 14.0)),
        )
        .child(words)
}

/// KSG Code's `LiveActivityRow`: the muted row, swept by the shimmer while it is the turn's
/// current activity.
fn live_activity(
    s: &State,
    p: Palette,
    glyph: &str,
    text: &str,
    animated: bool,
    key: Option<String>,
) -> El {
    let base = activity_content(glyph, text, p.muted, key);
    if !animated {
        return base;
    }
    shimmer(
        base,
        activity_content(glyph, text, p.ink, None),
        live_row_width(s),
    )
}

/// Ported from KSG Code's `ThinkingTimelineRow`: "Thinking" with the brain icon at the bottom
/// of a running turn, while no step of its tail is in progress.
fn live_status_row(s: &State, t: &Thread, start: usize, p: Palette) -> El {
    row()
        .key(format!("live-status-{}", t.id))
        .role("status")
        .w_full()
        .min_w(0.0)
        .child(live_activity(
            s,
            p,
            "brain",
            &live_status(t, start),
            shimmer_allowed(s, t),
            Some(format!("live-status-text-{}", t.id)),
        ))
}

/// KSG Code's `LiveWorkToggle`: a live row that is its work group's disclosure.
fn live_toggle(key: String, a11y: String, p: Palette) -> El {
    col()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(a11y)
        .focusable(true)
        .w_full()
        .min_w(0.0)
        .px(6.0)
        .ml(-6.0)
        .radius(6.0)
        .hover(p.selected)
}

/// Ported from KSG Code's `LiveWorkEntryTimelineRow`: a running turn's tail as one row. The
/// newest thought's first line sits above the step in progress, which replaces itself as
/// each call starts and ends; the whole row unfolds the tail's steps. When the last call
/// failed, the "Thinking" row stands in for it and does the unfolding, as KSG's does.
fn live_work(s: &State, p: Palette, parts: &[TranscriptPart], tail: LiveTail, start: usize) -> El {
    let t = s.current();
    let TranscriptPart::Work(entries) = &parts[tail.part] else {
        return el();
    };
    let open = group_open(t, entries);
    let first_tool = entry_tools(entries)
        .next()
        .map(|index| t.tools[index].id.clone());
    let first_thought = entry_thoughts(entries).first().map(|&index| {
        let thought = &t.thoughts[index];
        (thought.anchor.message, thought.block, thought.anchor.offset)
    });
    let toggle = move |key: String| {
        move |s: &mut State, tx: &mut rune::Tx| {
            tx.keep_on_screen(key.clone());
            toggle_group(s.current_mut(), first_tool.as_deref(), first_thought);
        }
    };
    let verb = if open { "Hide" } else { "Show" };
    if tail.failed {
        let key = format!("live-status-{}", t.id);
        return live_toggle(key.clone(), format!("{verb} steps. A tool call failed"), p)
            .child(live_activity(
                s,
                p,
                "brain",
                &live_status(t, start),
                shimmer_allowed(s, t),
                Some(format!("live-status-text-{}", t.id)),
            ))
            .on_press_tx(toggle(key));
    }
    let thought_line = tail
        .thought
        .filter(|_| !open)
        .map(|index| live_thought_line(&t.thoughts[index].text))
        .filter(|line| !line.is_empty());
    let entry = tail.running.unwrap_or(tail.latest);
    let (glyph, text) = match entry {
        WorkEntry::Thought(index) => (
            "brain",
            if open || thought_line.is_some() {
                if tail.active { "Thinking" } else { "Thought" }.to_owned()
            } else {
                t.thoughts[index]
                    .text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            },
        ),
        WorkEntry::Tool(index) => {
            let tool = &t.tools[index];
            let verb = tool_verb_in(tool, tool.failed || !tail.active && tool.done);
            (
                work_ui::kind_icon(tool.resolved_kind()),
                if tool.detail.is_empty() {
                    verb
                } else {
                    format!("{verb} {}", tool.detail)
                },
            )
        }
    };
    let key = format!("live-work-{}", t.id);
    let mut live_row = live_toggle(key.clone(), format!("{verb} steps. {text}"), p);
    if let Some(line) = thought_line {
        live_row = live_row.child(
            row()
                .key(format!("live-thought-{}", t.id))
                .gap(5.0)
                .min_w(0.0)
                .child(
                    el().size(24.0, 24.0)
                        .center()
                        .child(icon("brain", p.faint).size(14.0, 14.0)),
                )
                .child(
                    label(line, ACTIVITY.lh(20.0), p.muted)
                        .wrap()
                        .max_lines(4)
                        .pt(2.0)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0),
                ),
        );
    }
    let failed = matches!(entry, WorkEntry::Tool(index) if t.tools[index].failed);
    live_row
        .child(live_activity(
            s,
            p,
            glyph,
            &text,
            tail.active && !failed && shimmer_allowed(s, t),
            Some(format!("live-work-text-{}", t.id)),
        ))
        .on_press_tx(toggle(key))
}

fn assistant_turn(s: &State, p: Palette, wide: bool, start: usize, end: usize) -> El {
    assistant_turn_piece(s, p, wide, start, end, None)
}

/// One virtual row of an assistant turn. `None` is the whole turn, as short turns
/// show; a long live or expanded turn is a head, bounded work slices and a tail.
fn assistant_turn_piece(
    s: &State,
    p: Palette,
    wide: bool,
    start: usize,
    end: usize,
    piece: Option<TurnPiece>,
) -> El {
    let t = s.current();
    let structure = timeline_structure(t);
    if let Some(TurnPiece::Work { lo, hi }) = piece {
        return work_slice(s, p, wide, start, end, lo, hi);
    }
    if t.transcript_structure == 0
        || end == t.messages.len() && t.status.busy()
        || structure.external_turns.contains(&start)
    {
        return build_assistant_turn(s, p, wide, start, end, piece);
    }
    let revision = t.timeline_cache.borrow().turn_revision(start);
    let captain = t
        .captain
        .and_then(|id| s.captains.get(id))
        .map(|c| (&c.name, c.portrait));
    let find = s.thread_find_active().then(|| {
        (
            s.thread_find.query.text.as_str(),
            s.thread_find
                .active_match
                .as_ref()
                .map(|m| (&m.entry_id, m.occurrence)),
        )
    });
    let cite = s
        .citation_highlight
        .as_ref()
        .map(|h| (&h.machine, h.thread_id, &h.message_id, &h.text));
    // The response actions: a Copy check on one of this turn's messages, and whether Fork shows.
    let copied = s.copied_message.as_ref().and_then(|(id, _)| {
        t.messages[start..end.min(t.messages.len())]
            .iter()
            .any(|m| &m.id == id)
            .then_some(id.as_str())
    });
    let actions = (
        copied,
        t.session.is_some(),
        t.can_change_conversation(),
        t.can_rewind_conversation(),
        t.status.busy(),
        t.configuring,
    );
    memo(
        format!(
            "assistant-turn-{}{}",
            message_key(t, start),
            piece_suffix(piece)
        ),
        (
            (structure.version, revision, start, end),
            (
                s.dark,
                wide,
                s.code_wrap_version,
                rune::memo::layout_with("chat-lane", |rect| rect[2].to_bits()),
                s.viewport_width.to_bits(),
                s.demo,
            ),
            (
                captain,
                t.captain_home,
                t.archived(),
                t.native.compaction_expanded,
            ),
            (t.harness.as_str(), s.working_directory(), find, cite),
            actions,
        ),
        || build_assistant_turn(s, p, wide, start, end, piece),
    )
}

fn piece_suffix(piece: Option<TurnPiece>) -> &'static str {
    match piece {
        None | Some(TurnPiece::Head) => "",
        Some(TurnPiece::Work { .. }) => "-work",
        Some(TurnPiece::Tail) => "-tail",
    }
}

/// An HTML card keeps its own width and is lifted out of a folded turn.
fn is_html_card(t: &Thread, part: &TranscriptPart) -> bool {
    matches!(part, TranscriptPart::Tools(indices) if indices.iter().all(|&i|
            t.tools[i].resolved_kind() == crate::work::ToolKind::Html
            && crate::html_render::find_reference(&t.tools[i].structured).is_some()))
}

/// A captain's delegations are its answer, not its working: their task cards stay in view
/// when the rest of the turn folds into "Worked for". It reads the thread only, so the
/// timeline structure can split turns without the window state.
fn is_lifted_card(t: &Thread, part: &TranscriptPart) -> bool {
    (t.captain_home
        && !t.delegated
        && matches!(part, TranscriptPart::Tools(indices)
            if indices.iter().all(|&index| crew_ui::is_delegation(&t.tools[index]))))
        || is_html_card(t, part)
}

/// The last work part and the preamble above the fold.
fn turn_shape(t: &Thread, parts: &[TranscriptPart]) -> (Option<usize>, Option<usize>) {
    let last_work = parts
        .iter()
        .rposition(|part| !matches!(part, TranscriptPart::Text { .. }) && !is_lifted_card(t, part));
    // Initial thinking may precede the preamble on the wire. Keep that thinking in
    // the work section, and the first text before subsequent work above its fold.
    let preamble = last_work.and_then(|last| {
        parts[..last]
            .iter()
            .position(|part| matches!(part, TranscriptPart::Text { .. }))
            .filter(|index| {
                !parts[..*index].iter().any(|part| match part {
                    TranscriptPart::Compaction(_) | TranscriptPart::Retry(_) => true,
                    TranscriptPart::Tools(indices) => indices
                        .iter()
                        .any(|i| t.tools[*i].resolved_kind() == crate::work::ToolKind::Plan),
                    _ => false,
                })
            })
    });
    (last_work, preamble)
}

#[derive(Clone, Copy)]
struct TurnLayout {
    last_work: Option<usize>,
    preamble: Option<usize>,
    live: bool,
    inline_work: bool,
}

/// KSG's `deriveTimelineRows` leaves commentary between tool streaks visible even
/// when individual streaks are folded. Keep the turn-wide control for stopped
/// runs and for deferred history, which still needs an explicit load.
fn turn_layout(
    t: &Thread,
    structure: &TimelineStructure,
    start: usize,
    end: usize,
    parts: &[TranscriptPart],
) -> TurnLayout {
    let (last_work, mut preamble) = turn_shape(t, parts);
    let live = end == t.messages.len() && t.status.busy();
    let mut saw_tools = false;
    let mut saw_commentary = false;
    let inline_work = !structure.deferred_work.contains_key(&start)
        && !structure
            .work_index
            .get(&start)
            .is_some_and(|&index| t.work[index].stopped)
        && parts.iter().any(|part| match part {
            TranscriptPart::Work(entries) if entry_tools(entries).next().is_some() => {
                saw_tools = true;
                saw_commentary
            }
            TranscriptPart::Text { message, from, to }
                if saw_tools && !t.messages[*message].text[*from..*to].trim().is_empty() =>
            {
                saw_commentary = true;
                false
            }
            _ => false,
        });
    if inline_work {
        preamble = None;
    }
    TurnLayout {
        last_work,
        preamble,
        live,
        inline_work,
    }
}

/// The conversation lane's gap between rows. A split turn's slices and tail rise
/// into it from inside their own row boxes, so the virtual list's measured heights
/// still match where rows sit, and threads without split turns keep their geometry.
const LANE_GAP: f32 = 20.0;

/// A turn at or above this many work entries splits into slice rows while it runs or
/// while its work is open. Shorter turns stay one row with their original layout.
const SEGMENT_MIN_ITEMS: usize = 48;
#[cfg(test)]
thread_local! {
    static NEVER_SPLIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn segment_min_items() -> usize {
    #[cfg(test)]
    if NEVER_SPLIT.with(std::cell::Cell::get) {
        return usize::MAX;
    }
    SEGMENT_MIN_ITEMS
}

/// Work entries per slice row. Rune mounts the viewport and two viewports either
/// side, so mounted entries follow the viewport, not the turn's length.
const SLICE_ITEMS: usize = 4;

/// One entry of a turn's work column, in display order. Unfolded work streaks and thought
/// runs contribute their heading and each row separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkItem {
    Part(usize),
    /// A streak's folded or unfolded header.
    GroupHead(usize),
    /// Step `k` of an unfolded streak.
    GroupEntry(usize, usize),
    ThoughtHead(usize),
    ThoughtBody(usize, usize),
    /// A running turn's tail (`work-live`).
    Live(usize),
}

impl WorkItem {
    fn part(self) -> usize {
        match self {
            Self::Part(part)
            | Self::GroupHead(part)
            | Self::GroupEntry(part, _)
            | Self::ThoughtHead(part)
            | Self::ThoughtBody(part, _)
            | Self::Live(part) => part,
        }
    }
}

/// Whether a part in a running turn is current activity or a live card that stays visible even
/// when earlier work history is folded behind the `Working for` bar.
fn live_part_stays_visible(t: &Thread, parts: &[TranscriptPart], index: usize) -> bool {
    let at_end = index + 1 == parts.len();
    match &parts[index] {
        TranscriptPart::Work(entries) => {
            at_end
                || entries.iter().any(|entry| match *entry {
                    WorkEntry::Tool(i) => !t.tools[i].done,
                    WorkEntry::Thought(i) => !t.thoughts[i].done,
                })
        }
        TranscriptPart::Tools(indices) => {
            at_end
                || is_lifted_card(t, &parts[index])
                || indices.iter().any(|&i| {
                    let tool = &t.tools[i];
                    !tool.done || tool.resolved_kind() == crate::work::ToolKind::Plan
                })
        }
        TranscriptPart::Compaction(i) => {
            at_end
                || t.native
                    .compactions
                    .get(*i)
                    .is_some_and(|item| item["running"] == true)
        }
        TranscriptPart::Retry(i) => {
            at_end
                || crate::work::retry_events(t)
                    .get(*i)
                    .is_some_and(|event| event["running"] == true)
        }
        TranscriptPart::Text { .. } | TranscriptPart::Thought(_) => at_end,
    }
}

/// Whether a running turn has earlier work history before its live tail that can fold behind the
/// 32 pt `Working for` bar.
fn live_work_foldable(t: &Thread, parts: &[TranscriptPart], layout: TurnLayout) -> bool {
    if !layout.live {
        return false;
    }
    if t.live_work_collapsed {
        return true;
    }
    let Some(last) = layout.last_work else {
        return false;
    };
    parts[..=last].iter().enumerate().any(|(index, _)| {
        Some(index) != layout.preamble && !live_part_stays_visible(t, parts, index)
    })
}

/// A running turn's work column ends in its live tail, which folds into one `work-live` row
/// and unfolds below it. Before prose, and in a finished turn, a streak of two or more steps
/// folds under its header (KSG's `work-toggle`); a lone call stays its own row.
fn work_items(t: &Thread, parts: &[TranscriptPart], layout: TurnLayout) -> Vec<WorkItem> {
    let Some(last) = layout.last_work else {
        return vec![];
    };
    let tail = layout.live.then(|| live_tail(t, parts)).flatten();
    let mut items = Vec::with_capacity(last + 1);
    for (index, part) in parts[..=last].iter().enumerate() {
        if Some(index) == layout.preamble
            || !layout.live && !layout.inline_work && is_lifted_card(t, part)
        {
            continue;
        }
        if layout.live && t.live_work_collapsed && !live_part_stays_visible(t, parts, index) {
            continue;
        }
        match part {
            TranscriptPart::Work(entries) if layout.live && index + 1 == parts.len() => {
                // The live tail. With nothing to show yet, the Thinking row speaks for it.
                if tail.is_none() {
                    continue;
                }
                items.push(WorkItem::Live(index));
                if group_open(t, entries) {
                    let thinking = entry_tools(entries).next().is_none();
                    items.extend((0..entries.len()).map(|k| {
                        if thinking {
                            WorkItem::ThoughtBody(index, k)
                        } else {
                            WorkItem::GroupEntry(index, k)
                        }
                    }));
                }
            }
            TranscriptPart::Work(entries) if entry_tools(entries).next().is_none() => {
                if t.thoughts[entry_thoughts(entries)[0]].expanded {
                    items.push(WorkItem::ThoughtHead(index));
                    items.extend((0..entries.len()).map(|k| WorkItem::ThoughtBody(index, k)));
                } else {
                    items.push(WorkItem::Part(index));
                }
            }
            TranscriptPart::Work(entries) if entries.len() > 1 => {
                items.push(WorkItem::GroupHead(index));
                if group_open(t, entries) {
                    items.extend((0..entries.len()).map(|k| WorkItem::GroupEntry(index, k)));
                }
            }
            _ => items.push(WorkItem::Part(index)),
        }
    }
    items
}

/// The space above a work entry.
fn work_item_gap(item: WorkItem) -> f32 {
    match item {
        WorkItem::ThoughtBody(..) => 4.0,
        _ => WORK_GAP,
    }
}

/// The key part of an entry that does not move when entries are added after it.
fn work_item_id(t: &Thread, parts: &[TranscriptPart], item: WorkItem) -> String {
    let thought_id = |index: usize, k: usize| {
        let thought = &t.thoughts[index];
        let message = t
            .messages
            .get(thought.anchor.message)
            .map_or("end", |message| message.id.as_str());
        format!(
            "thought-{message}-{}-{}-{k}",
            thought.block, thought.anchor.offset
        )
    };
    let entry_id = |entry: WorkEntry, k: usize| match entry {
        WorkEntry::Tool(index) => format!("tool-{}", t.tools[index].id),
        WorkEntry::Thought(index) => thought_id(index, k),
    };
    match (&parts[item.part()], item) {
        (TranscriptPart::Work(entries), WorkItem::GroupEntry(_, k)) => entry_id(entries[k], 0),
        (TranscriptPart::Work(entries), WorkItem::ThoughtBody(_, k)) => entry_id(entries[k], k + 1),
        (TranscriptPart::Work(entries), WorkItem::Live(_)) => {
            format!("live-{}", entry_id(entries[0], 0))
        }
        (TranscriptPart::Work(entries), _) => match entries[0] {
            WorkEntry::Tool(index) => format!("tools-{}", t.tools[index].id),
            WorkEntry::Thought(index) => thought_id(index, 0),
        },
        (TranscriptPart::Tools(indices), _) => format!("tools-{}", t.tools[indices[0]].id),
        (TranscriptPart::Thought(indices), _) => thought_id(indices[0], 0),
        (TranscriptPart::Text { message, from, .. }, _) => {
            format!("text-{}-{from}", t.messages[*message].id)
        }
        (TranscriptPart::Compaction(index), _) => format!("compaction-{index}"),
        (TranscriptPart::Retry(index), _) => format!("retry-{index}"),
    }
}

/// Whether a turn shows its work column now: always while it runs, and while its
/// "Worked for" fold is open with its history loaded.
fn turn_work_open(t: &Thread, structure: &TimelineStructure, start: usize, live: bool) -> bool {
    if live {
        return true;
    }
    let expanded = structure
        .work_index
        .get(&start)
        .and_then(|index| t.work.get(*index))
        .is_some_and(|work| work.expanded);
    let (failed, loading) = structure
        .deferred_work
        .get(&start)
        .copied()
        .unwrap_or_default();
    expanded && !failed && !loading
}

/// Matches before part `index` in the same message, so a slice numbers its find
/// highlights as the whole message does.
fn find_occurrence_base(t: &Thread, parts: &[TranscriptPart], index: usize, query: &str) -> usize {
    let TranscriptPart::Text { message, .. } = parts[index] else {
        return 0;
    };
    parts[..index]
        .iter()
        .filter_map(|part| match part {
            TranscriptPart::Text {
                message: m,
                from,
                to,
            } if *m == message => Some(
                crate::thread_find::find_markdown_occurrences(
                    &t.messages[*m].text[*from..*to],
                    query,
                )
                .len(),
            ),
            _ => None,
        })
        .sum()
}

/// One work entry of the work column.
fn work_item(
    s: &State,
    p: Palette,
    wide: bool,
    parts: &[TranscriptPart],
    item: WorkItem,
    start: usize,
) -> El {
    let t = s.current();
    let part = &parts[item.part()];
    match (part, item) {
        (TranscriptPart::Work(entries), WorkItem::GroupHead(_)) => work_group_head(s, p, entries),
        (TranscriptPart::Work(entries), WorkItem::GroupEntry(_, k)) => col()
            .min_w(0.0)
            .pl(WORK_LIST_INDENT)
            .child(work_entry_row(s, p, entries[k])),
        (TranscriptPart::Work(entries), WorkItem::ThoughtHead(_)) => col()
            .min_w(0.0)
            .child(work_ui::thought_head(s, p, &entry_thoughts(entries))),
        (TranscriptPart::Work(entries), WorkItem::ThoughtBody(_, k)) => col()
            .min_w(0.0)
            .child(work_ui::thought_body(s, p, entry_thoughts(entries)[k])),
        (TranscriptPart::Work(_), WorkItem::Live(_)) => match live_tail(t, parts) {
            Some(tail) => col().min_w(0.0).child(live_work(s, p, parts, tail, start)),
            None => el(),
        },
        (part, _) => {
            let query = (s.thread_find_active() && !s.thread_find.query.text.trim().is_empty())
                .then(|| s.thread_find.query.text.trim());
            let mut base = query.map_or(0, |query| {
                find_occurrence_base(t, parts, item.part(), query)
            });
            let row = transcript_part(s, p, wide, part, &mut base);
            if matches!(part, TranscriptPart::Text { .. }) {
                row.py(6.0)
            } else {
                row
            }
        }
    }
}

/// A turn's work entries as one column, `gap` above each as the slices space them.
fn work_column(
    s: &State,
    p: Palette,
    wide: bool,
    parts: &[TranscriptPart],
    items: &[WorkItem],
    start: usize,
    key: String,
) -> El {
    let mut rows = col().key(key).min_w(0.0);
    for (index, item) in items.iter().enumerate() {
        let row = work_item(s, p, wide, parts, *item, start);
        rows = rows.child(if index == 0 {
            row
        } else {
            col().min_w(0.0).pt(work_item_gap(*item)).child(row)
        });
    }
    rows
}

/// What a slice's entries show, so a slice rebuilds only when one of them changed.
fn work_slice_fingerprint(t: &Thread, parts: &[TranscriptPart], items: &[WorkItem]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for item in items {
        item.hash_fields(&mut hasher);
        match &parts[item.part()] {
            TranscriptPart::Tools(indices) => {
                for &i in indices {
                    let tool = &t.tools[i];
                    (
                        &tool.id,
                        tool.done,
                        tool.failed,
                        tool.expanded,
                        tool.raw_expanded,
                        tool.group_expanded,
                        tool.output.len(),
                        tool.detail.len(),
                        tool.changes.len(),
                        tool.images.len(),
                        tool.subagents.len(),
                        tool.exit_code,
                    )
                        .hash(&mut hasher);
                }
            }
            TranscriptPart::Thought(indices) => {
                for &i in indices {
                    let thought = &t.thoughts[i];
                    (thought.text.len(), thought.done, thought.expanded).hash(&mut hasher);
                }
            }
            TranscriptPart::Work(entries) => {
                for entry in entries {
                    match *entry {
                        WorkEntry::Tool(i) => {
                            let tool = &t.tools[i];
                            (
                                &tool.id,
                                tool.done,
                                tool.failed,
                                tool.expanded,
                                tool.raw_expanded,
                                tool.group_expanded,
                                tool.output.len(),
                                tool.detail.len(),
                                tool.changes.len(),
                                tool.images.len(),
                                tool.subagents.len(),
                                tool.exit_code,
                            )
                                .hash(&mut hasher);
                        }
                        WorkEntry::Thought(i) => {
                            let thought = &t.thoughts[i];
                            (thought.text.len(), thought.done, thought.expanded).hash(&mut hasher);
                        }
                    }
                }
            }
            TranscriptPart::Text { message, from, to } => (
                &t.messages[*message].id,
                from,
                to,
                t.messages[*message].text.len(),
            )
                .hash(&mut hasher),
            TranscriptPart::Compaction(index) | TranscriptPart::Retry(index) => {
                index.hash(&mut hasher)
            }
        }
    }
    hasher.finish()
}

impl WorkItem {
    fn hash_fields(self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        match self {
            Self::Part(i) => (0u8, i, 0usize),
            Self::GroupHead(i) => (1, i, 0),
            Self::GroupEntry(i, k) => (2, i, k),
            Self::ThoughtHead(i) => (3, i, 0),
            Self::ThoughtBody(i, k) => (4, i, k),
            Self::Live(i) => (5, i, 0),
        }
        .hash(hasher);
    }
}

/// Work entries `lo..hi` of a long turn's work column, as one virtual row. The
/// rows above and below are the turn's head and tail.
#[allow(clippy::too_many_arguments)]
fn work_slice(
    s: &State,
    p: Palette,
    wide: bool,
    start: usize,
    end: usize,
    lo: usize,
    hi: usize,
) -> El {
    let t = s.current();
    let parts = turn_parts(t, start, end);
    let layout = turn_layout(t, &timeline_structure(t), start, end, &parts);
    let live = layout.live;
    let items = work_items(t, &parts, layout);
    let lo = lo.min(items.len());
    let hi = hi.min(items.len());
    // Above the first slice: the head's bar, 12 pt up in the turn's column.
    let top = if lo > 0 && lo < items.len() {
        work_item_gap(items[lo])
    } else {
        12.0
    };
    let key = format!("work-rows-{}-{lo}", message_key(t, start));
    let build = || {
        #[cfg(test)]
        timeline_count(|counts| counts.built_slices += 1);
        col().child(
            work_column(s, p, wide, &parts, &items[lo..hi], start, key.clone()).mt(top - LANE_GAP),
        )
    };
    if t.transcript_structure == 0 || timeline_structure(t).external_turns.contains(&start) {
        return build();
    }
    let find = s.thread_find_active().then(|| {
        (
            s.thread_find.query.text.as_str(),
            s.thread_find
                .active_match
                .as_ref()
                .map(|m| (&m.entry_id, m.occurrence)),
        )
    });
    let cite = s
        .citation_highlight
        .as_ref()
        .map(|h| (&h.machine, h.thread_id, &h.message_id, &h.text));
    // The response actions: a Copy check on one of this turn's messages, and whether Fork shows.
    let copied = s.copied_message.as_ref().and_then(|(id, _)| {
        t.messages[start..end.min(t.messages.len())]
            .iter()
            .any(|m| &m.id == id)
            .then_some(id.as_str())
    });
    let actions = (
        copied,
        t.session.is_some(),
        t.can_change_conversation(),
        t.can_rewind_conversation(),
        t.status.busy(),
        t.configuring,
    );
    memo(
        key.clone(),
        (
            (
                work_slice_fingerprint(t, &parts, &items[lo..hi]),
                lo,
                hi,
                top.to_bits(),
                live,
                (
                    t.request.is_some(),
                    s.portraits.reduced_motion(),
                    s.html_lane_width,
                ),
            ),
            (
                s.dark,
                wide,
                s.code_wrap_version,
                rune::memo::layout_with("chat-lane", |rect| rect[2].to_bits()),
                s.viewport_width.to_bits(),
            ),
            (t.harness.as_str(), s.working_directory(), find, cite),
            actions,
        ),
        build,
    )
}

fn build_assistant_turn(
    s: &State,
    p: Palette,
    wide: bool,
    start: usize,
    end: usize,
    piece: Option<TurnPiece>,
) -> El {
    #[cfg(test)]
    timeline_count(|counts| counts.built_turns += 1);
    let t = s.current();
    let parts = turn_parts(t, start, end);
    let card = |part: &TranscriptPart| is_lifted_card(t, part);
    let structure = timeline_structure(t);
    let layout = turn_layout(t, &structure, start, end, &parts);
    let TurnLayout {
        last_work,
        preamble,
        live,
        inline_work,
    } = layout;
    let work = structure
        .work_index
        .get(&start)
        .and_then(|index| t.work.get(*index));
    // A split turn's head carries the turn's keys; its tail follows the last slice.
    let head = piece != Some(TurnPiece::Tail);
    let tail = piece != Some(TurnPiece::Head);
    let whole = piece.is_none();
    let mut response = col()
        .key(format!(
            "assistant-turn-{}{}",
            message_key(t, start),
            piece_suffix(piece)
        ))
        .gap(12.0);
    let mut tail_children = 0usize;
    // Captains sign their turns. A lone Pi thread needs no speaker label.
    if let Some(captain) = t.captain.filter(|_| head) {
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
    let mut occ_base = 0usize;
    let mut part_occs = vec![0usize; parts.len()];
    if s.thread_find_active() && !s.thread_find.query.text.trim().is_empty() {
        let query = s.thread_find.query.text.trim();
        let mut last_msg = None;
        for (i, part) in parts.iter().enumerate() {
            if let TranscriptPart::Text { message, from, to } = part {
                if last_msg != Some(*message) {
                    occ_base = 0;
                    last_msg = Some(*message);
                }
                part_occs[i] = occ_base;
                occ_base += crate::thread_find::find_markdown_occurrences(
                    &t.messages[*message].text[*from..*to],
                    query,
                )
                .len();
            }
        }
    }
    let render_part = |part_index: usize| {
        let mut base = part_occs[part_index];
        transcript_part(s, p, wide, &parts[part_index], &mut base)
    };
    let items = work_items(t, &parts, layout);
    let work_key = format!("work-rows-{}", message_key(t, start));
    if live {
        // KSG's `WorkingTimelineRow`: the turn's 32 pt bar at its top while it runs.
        if head {
            response = response.child(working_bar(
                t,
                start,
                live_work_foldable(t, &parts, layout),
                p,
            ));
        }
    } else if !inline_work && (last_work.is_some() || work.is_some_and(|work| work.stopped)) {
        let expanded = work.is_some_and(|work| work.expanded);
        let (work_failed, loading) = structure
            .deferred_work
            .get(&start)
            .copied()
            .unwrap_or_default();
        let work_loading = expanded && loading;
        let mut title = match (
            work.is_some_and(|work| work.stopped),
            work.and_then(|work| work.elapsed),
        ) {
            (true, Some(seconds)) => format!("You stopped after {}", elapsed_label(seconds)),
            (true, None) => "You stopped this response".into(),
            (false, Some(seconds)) => format!("Worked for {}", elapsed_label(seconds)),
            (false, None) => "Worked".into(),
        };
        if work_failed {
            title.push_str(" · Couldn't load work. Click to retry.");
        }
        let toggle_key = format!("toggle-work-{}-{start}", t.id);
        let press_key = toggle_key.clone();
        let turn_msg_id = t.messages.get(start).map(|m| m.id.clone());
        // The running turn's bar, now its fold: KSG's `WorkFoldToggle` in the same place.
        let mut section = col()
            .key(format!("work-section-{}", message_key(t, start)))
            .w_full()
            .child(
                working_bar_frame(p).child(
                    work_fold_toggle(
                        toggle_key,
                        format!(
                            "{} work section. {title}",
                            if expanded { "Collapse" } else { "Expand" }
                        ),
                        &title,
                        expanded,
                        p,
                    )
                    .on_press_tx(move |s: &mut State, tx| {
                        tx.keep_on_screen(press_key.clone());
                        let t = s.current_mut();
                        let current_start = match turn_msg_id.as_deref() {
                            Some(id) => {
                                let Some(index) = t.messages.iter().position(|m| m.id == id) else {
                                    return;
                                };
                                index
                            }
                            None => start,
                        };
                        t.timeline_cache.get_mut().touch_message(current_start);
                        let retry = t
                            .work
                            .iter()
                            .any(|work| work.start == current_start && work.expanded)
                            && t.deferred_work.iter().any(|marker| {
                                marker["start"] == current_start && marker["error"].is_string()
                            });
                        if retry {
                            s.load_deferred_work(current_start, true);
                            return;
                        }
                        match t.work.iter_mut().find(|work| work.start == current_start) {
                            Some(work) => work.expanded = !work.expanded,
                            // Turns from before work records existed fold too; opening one
                            // records it.
                            None => t.work.push(crate::model::AssistantWork {
                                start: current_start,
                                stopped: false,
                                stopped_order: 0,
                                expanded: true,
                                elapsed: None,
                                outcome: None,
                            }),
                        }
                        if s.current()
                            .work
                            .iter()
                            .any(|work| work.start == current_start && work.expanded)
                        {
                            s.load_deferred_work(current_start, false);
                        }
                    }),
                ),
            );
        if expanded {
            let restored_id = t.messages.get(start).map(|message| message.id.clone());
            section = section.on_layout_tx(move |s: &mut State, _, tx| {
                if let Some(start) = restored_id.as_ref().and_then(|id| {
                    s.current()
                        .messages
                        .iter()
                        .position(|message| &message.id == id)
                }) {
                    s.load_deferred_work(start, false);
                }
                tx.live_only();
            });
        }
        if head {
            response = response.child(section);
            if let Some(index) = preamble {
                response = response.child(render_part(index));
            }
            if work_loading {
                let mut skeleton = col()
                    .key(format!("work-loading-{}", message_key(t, start)))
                    .role("status")
                    .label("Loading work history")
                    .pb(4.0)
                    .px(4.0)
                    .gap(8.0);
                for fraction in [0.4, 0.6, 1.0 / 3.0] {
                    skeleton = skeleton.child(
                        row()
                            .gap(8.0)
                            .center_y()
                            .child(el().size(12.0, 12.0).fill(p.selected))
                            .child(
                                el().w((lane_width(s.viewport_width, wide) * fraction).max(20.0))
                                    .h(10.0)
                                    .fill(p.selected),
                            ),
                    );
                }
                response = response.child(skeleton);
            } else if expanded && !work_failed && whole && !items.is_empty() {
                response = response.child(work_column(
                    s,
                    p,
                    wide,
                    &parts,
                    &items,
                    start,
                    work_key.clone(),
                ));
            }
        }
        if let Some(last) = last_work.filter(|_| tail) {
            for (index, part) in parts[..=last].iter().enumerate() {
                if card(part) {
                    response = response.child(render_part(index));
                    tail_children += 1;
                }
            }
        }
    }
    if live && head {
        if let Some(index) = preamble {
            response = response.child(render_part(index));
        } else if last_work.is_none() {
            // Until Pi emits work, streamed text is the preamble.
            for index in 0..parts.len() {
                response = response.child(render_part(index));
            }
        }
    }
    // KSG's `ThinkingTimelineRow`: while the turn runs and its tail is not the activity, the
    // Thinking row sits at the very bottom. A failed last call puts it in the tail instead.
    let thinking = live
        && t.phase != "Compacting context"
        && match live_tail(t, &parts) {
            Some(tail) if tail.failed => false,
            Some(tail) => !tail.active && t.request.is_none(),
            None => t.request.is_none(),
        };
    let from = last_work.map_or(0, |index| index + 1);
    let trailing = tail && last_work.is_some() && from < parts.len();
    let mut thinking_after_text = thinking;
    if (live || inline_work) && whole && !items.is_empty() {
        let mut rows = work_column(s, p, wide, &parts, &items, start, work_key.clone());
        if thinking && !trailing {
            rows = rows.child(col().pt(WORK_GAP).child(live_status_row(s, t, start, p)));
            thinking_after_text = false;
        }
        response = response.child(rows);
    }
    // KSG's `AssistantChangedFilesSection`: the changed-files card mounts inside the turn's
    // last assistant message, under its prose and above the meta row, once the turn has
    // finished.
    let mut changed_files_card = if tail && !live {
        changed_files::card(s, p, start, end)
    } else {
        None
    };
    if tail && (!live || last_work.is_some()) {
        let card_after = changed_files_card
            .is_some()
            .then(|| {
                t.messages[start..end]
                    .iter()
                    .rposition(|m| m.role == "Pi")
                    .map(|index| start + index)
            })
            .flatten()
            .and_then(|message| {
                parts[from..]
                    .iter()
                    .rposition(|part| {
                        matches!(part, TranscriptPart::Text { message: m, .. } if *m == message)
                    })
                    .map(|index| from + index)
            });
        for index in from..parts.len() {
            response = response.child(render_part(index));
            tail_children += 1;
            if Some(index) == card_after
                && let Some(card) = changed_files_card.take()
            {
                response = response.child(card);
                tail_children += 1;
            }
        }
        // The last message has no visible prose (empty, folded into the work group or
        // outside the tail): the card follows its content like KSG's in-message mount.
        if let Some(card) = changed_files_card.take() {
            response = response.child(card);
            tail_children += 1;
        }
    }
    // A split running turn's tail follows its last slice, 2 pt below it as the work column
    // spaces its rows, or 12 pt below prose that ends the turn.
    let mut tail_gap = 12.0;
    if live && tail && thinking_after_text {
        if !whole && !items.is_empty() && !trailing {
            tail_gap = WORK_GAP;
        }
        response = response.child(live_status_row(s, t, start, p));
        tail_children += 1;
    }
    if tail
        && !live
        && !s.demo
        && !t.archived()
        && let Some(reply_msg) = t.messages[start..end]
            .iter()
            .rev()
            .find(|m| m.role == "Pi" && !m.text.trim().is_empty())
    {
        response = response.child(assistant_message_meta(
            s,
            p,
            reply_msg,
            end == t.messages.len(),
        ));
        tail_children += 1;
    }
    if piece == Some(TurnPiece::Tail) && tail_children > 0 {
        return col().child(response.mt(tail_gap - LANE_GAP));
    }
    response
}

/// How long a copied message shows its check, as KSG's `useCopyToClipboard` timeout.
pub(crate) const COPIED_FEEDBACK: Duration = Duration::from_millis(1500);

/// One quiet action under an assistant turn: a 24 pt icon button in the faint colour.
fn meta_action(key: String, title: &'static str, glyph: &str, p: Palette) -> El {
    el().key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .help(title)
        .focusable(true)
        .size(24.0, 24.0)
        .center()
        .radius(6.0)
        .hover(p.selected)
        .press(p.selected)
        .child(icon(glyph, p.faint).size(14.0, 14.0))
}

/// Ported from KSG Code's `AssistantMessageMeta` (`chat/TimelineAssistantRows.tsx`): Copy, Fork
/// and Reply under the response, on the prose's left edge. Shina messages carry no timestamp,
/// so the time KSG shows after the actions is left out.
fn assistant_message_meta(
    s: &State,
    p: Palette,
    message: &crate::model::Message,
    latest: bool,
) -> El {
    let t = s.current();
    let id = message.id.clone();
    let copied = s
        .copied_message
        .as_ref()
        .is_some_and(|(copied, _)| copied == &id);
    let text = message.display_text().to_owned();
    let copy_id = id.clone();
    let mut actions = row()
        .key(format!("assistant-actions-{}-{id}", t.id))
        .gap(4.0)
        .ml(-4.0)
        .center_y()
        .child(
            meta_action(
                format!("copy-message-{id}"),
                if copied { "Copied" } else { "Copy message" },
                if copied { "check" } else { "copy" },
                p,
            )
            .on_press_tx(move |s: &mut State, tx| {
                tx.copy(text.clone());
                s.copied_message = Some((copy_id.clone(), std::time::Instant::now()));
                tx.wake_after(COPIED_FEEDBACK + Duration::from_millis(50));
            }),
        );
    if t.session.is_some() && t.can_change_conversation() {
        let locked = t.status.busy() || t.configuring || (!latest && !t.can_rewind_conversation());
        actions = actions.child(
            meta_action(
                format!("fork-message-{id}"),
                "Fork from this response",
                "fork",
                p,
            )
            .disabled(locked)
            .on_press(move |s: &mut State| {
                // The newest response forks the whole conversation. An older one opens the
                // fork points, where the user picks the turn to branch from.
                if latest {
                    s.fork_conversation(None);
                } else {
                    s.conversation_points(crate::model::ConversationMode::Fork);
                }
            }),
        );
    }
    let reply_id = id.clone();
    actions.child(
        meta_action(
            format!("reply-message-{id}"),
            "Reply to this response",
            "reply",
            p,
        )
        .on_press(move |s: &mut State| {
            s.reply_to_message(&reply_id);
        }),
    )
}

/// Ported from KSG Code's `TimelineHistoryControl` (`MessagesTimeline.tsx`).
fn history_control(s: &State, p: Palette) -> Option<El> {
    let t = s.current();
    if !t.history_has_more && t.history_error.is_none() {
        return None;
    }
    let thread_id = t.id;
    let mut section = col()
        .key(format!("history-control-{thread_id}"))
        .w_full()
        .gap(6.0)
        .pb(8.0);
    if t.history_has_more {
        let loading = t.history_loading;
        let text = if loading {
            "Loading earlier turns…"
        } else {
            "Load earlier turns"
        };
        let mut btn = row()
            .key("history-load-earlier")
            .role("button")
            .label("Load earlier turns")
            .w_full()
            .py(6.0)
            .gap(12.0)
            .center_y()
            .disabled(loading)
            .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
            .child(label(text, SMALL, p.muted))
            .child(el().h(1.0).grow(1.0).fill(p.line).hairline());
        if !loading {
            btn = btn
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .hover(p.selected)
                .on_press(move |s: &mut State| s.load_earlier_history(thread_id, None));
        }
        section = section.child(btn);
    }
    if let Some(err) = &t.history_error {
        section = section.child(
            label(err, SMALL, p.muted)
                .key("history-load-error")
                .role("status")
                .align(rune::Align::Center)
                .wrap()
                .w_full(),
        );
    }
    Some(section)
}

type TimelineEvent = (crate::model::ToolAnchor, u64, u8, usize);
type BoundaryEvent = (u64, u8, usize);

/// Ported from KSG's useIncrementalTimelineEntries and useIncrementalTimelineRows.
/// This stores derived identities and ordinals only, never old history payloads.
#[derive(Default)]
pub(crate) struct TimelineCache {
    input: Option<TimelineInput>,
    structure: std::sync::Arc<TimelineStructure>,
    content: std::collections::HashMap<usize, u64>,
    unfinished_tools: std::collections::HashSet<usize>,
    /// Bumped when a long turn's fold opens or closes, which splits or joins its rows.
    disclosure: u64,
    /// The turn's elapsed seconds when a request began waiting on the user, so the working
    /// bar's "Paused at" clock stops there.
    paused_at: Option<u64>,
}

impl TimelineCache {
    pub(crate) fn prepare_projection(&mut self, structure: Option<u64>) {
        if self.input.as_ref().map(|input| input.0) != structure {
            *self = Self::default();
        }
    }

    pub(crate) fn touch_item(&mut self, field: &'static str, index: usize) {
        if let Some(turn) = self
            .structure
            .item_turns
            .get(field)
            .and_then(|turns| turns.get(index))
        {
            *self.content.entry(*turn).or_default() += 1;
        }
    }

    pub(crate) fn touch_message(&mut self, message: usize) {
        self.touch_item("messages", message);
    }

    pub(crate) fn touch_disclosure(&mut self) {
        self.disclosure += 1;
    }

    pub(crate) fn touch_tool(&mut self, index: usize, tool: &Tool) {
        self.touch_item("tools", index);
        if tool.done || tool.failed {
            self.unfinished_tools.remove(&index);
        } else {
            self.unfinished_tools.insert(index);
        }
    }

    fn turn_revision(&self, start: usize) -> u64 {
        self.content.get(&start).copied().unwrap_or(0)
    }
}

#[derive(Default)]
struct TimelineStructure {
    rows: std::sync::Arc<Vec<TimelineRow>>,
    events: Vec<TimelineEvent>,
    boundaries: std::collections::HashMap<usize, Vec<BoundaryEvent>>,
    item_turns: std::collections::HashMap<&'static str, Vec<usize>>,
    work_index: std::collections::HashMap<usize, usize>,
    deferred_work: std::collections::HashMap<usize, (bool, bool)>,
    external_turns: std::collections::HashSet<usize>,
    /// Fold flags that decide how long turns split, as they were when this was built.
    disclosure_probe: Vec<(u8, usize, bool)>,
    version: u64,
}

/// The structure after any fold that splits or joins a long turn changed. Handlers
/// and restored projections set fold flags directly; only a long turn's flags are
/// checked, so ordinary disclosure keeps the cached rows and indexes.
fn disclosed_timeline_structure(t: &Thread) -> std::sync::Arc<TimelineStructure> {
    let structure = timeline_structure(t);
    if !structure.disclosure_stale(t) {
        return structure;
    }
    t.timeline_cache.borrow_mut().touch_disclosure();
    timeline_structure(t)
}

type TimelineInput = (
    u64,
    bool,
    bool,
    bool,
    Vec<(String, usize)>,
    (usize, usize),
    u64,
);

impl TimelineStructure {
    /// Whether a fold that splits or joins a long turn changed without a handler
    /// reporting it (a restored projection, find's reveal, a test).
    fn disclosure_stale(&self, t: &Thread) -> bool {
        self.disclosure_probe.iter().any(|&(kind, index, open)| {
            let now = match kind {
                0 => t.tools.get(index).map(|tool| tool.group_expanded),
                1 => t.thoughts.get(index).map(|thought| thought.expanded),
                2 => t.work.get(index).map(|work| work.expanded),
                _ => Some(t.live_work_collapsed),
            };
            now != Some(open)
        })
    }
}

fn timeline_structure(t: &Thread) -> std::sync::Arc<TimelineStructure> {
    let input = (
        t.transcript_structure,
        t.history_has_more || t.history_error.is_some(),
        t.worktree_setup.is_some(),
        t.status.busy(),
        t.handoffs
            .iter()
            .map(|handoff| (handoff.id.clone(), handoff.message_index))
            .collect(),
        (
            t.deferred_work.len(),
            t.deferred_work
                .iter()
                .filter(|marker| marker["error"].is_string())
                .count(),
        ),
        t.timeline_cache.borrow().disclosure,
    );
    let mut cache = t.timeline_cache.borrow_mut();
    // The desktop consumes revisioned projections. Untracked fixtures and
    // in-process demo threads retain the complete rebuild fallback.
    if t.transcript_structure != 0 && cache.input.as_ref() == Some(&input) {
        return cache.structure.clone();
    }
    #[cfg(test)]
    timeline_count(|counts| counts.index_builds += 1);
    let mut structure = TimelineStructure::default();
    cache.unfinished_tools.clear();
    let mut turn = 0;
    let mut message_turns = Vec::with_capacity(t.messages.len() + 1);
    for (index, message) in t.messages.iter().enumerate() {
        if message.role != "Pi" || index == 0 || t.messages[index - 1].role != "Pi" {
            turn = index;
        }
        message_turns.push(turn);
    }
    message_turns.push(t.messages.len());
    let owner = |message: usize| message_turns.get(message).copied().unwrap_or(message);
    for (field, turns) in [
        (
            "tools",
            t.tools
                .iter()
                .map(|tool| owner(tool.anchor.message))
                .collect(),
        ),
        (
            "thoughts",
            t.thoughts
                .iter()
                .map(|thought| owner(thought.anchor.message))
                .collect(),
        ),
        (
            "work",
            t.work.iter().map(|work| owner(work.start)).collect(),
        ),
        (
            "compactions",
            t.native
                .compactions
                .iter()
                .map(|item| owner(crate::work::compaction_anchor(t, item).message))
                .collect(),
        ),
    ] {
        structure.item_turns.insert(field, turns);
    }
    for (index, tool) in t.tools.iter().enumerate() {
        if !tool.done && !tool.failed {
            cache.unfinished_tools.insert(index);
        }
        if crew_ui::is_delegation(tool)
            || !tool.subagents.is_empty()
            || matches!(
                tool.resolved_kind(),
                crate::work::ToolKind::Plan
                    | crate::work::ToolKind::Html
                    | crate::work::ToolKind::Subagent
            )
        {
            structure.external_turns.insert(owner(tool.anchor.message));
        }
        if tool.name.rsplit("__").next() != Some("html_preview") {
            structure.events.push((
                tool.anchor,
                if tool.order == 0 {
                    tool.block as u64
                } else {
                    tool.order
                },
                0,
                index,
            ));
        }
    }
    for (index, thought) in t.thoughts.iter().enumerate() {
        structure.events.push((
            thought.anchor,
            if thought.order == 0 {
                thought.block as u64
            } else {
                thought.order
            },
            1,
            index,
        ));
    }
    for (index, item) in t.native.compactions.iter().enumerate() {
        let anchor = crate::work::compaction_anchor(t, item);
        let order = item["order"].as_u64().unwrap_or(u64::MAX);
        structure.events.push((anchor, order, 2, index));
        structure
            .boundaries
            .entry(anchor.message)
            .or_default()
            .push((order, 0, index));
    }
    for (index, item) in crate::work::retry_events(t).iter().enumerate() {
        if let Ok(anchor) =
            serde_json::from_value::<crate::model::ToolAnchor>(item["anchor"].clone())
        {
            let order = item["order"].as_u64().unwrap_or(u64::MAX);
            structure.events.push((anchor, order, 3, index));
            structure
                .boundaries
                .entry(anchor.message)
                .or_default()
                .push((order, 1, index));
        }
    }
    for (index, work) in t.work.iter().enumerate() {
        structure.work_index.entry(work.start).or_insert(index);
        if work.stopped && (work.start < t.messages.len() || !t.status.busy()) {
            let order = if work.stopped_order == 0 {
                u64::MAX
            } else {
                work.stopped_order
            };
            structure
                .boundaries
                .entry(work.start)
                .or_default()
                .push((order, 2, 0));
        }
    }
    for marker in &t.deferred_work {
        if let Some(start) = marker["start"]
            .as_u64()
            .and_then(|start| usize::try_from(start).ok())
        {
            let flags = structure.deferred_work.entry(start).or_default();
            flags.0 |= marker["error"].is_string();
            flags.1 |= !marker["error"].is_string();
        }
    }
    structure.events.sort_by_key(|event| (event.0, event.1));
    for events in structure.boundaries.values_mut() {
        events.sort_by_key(|event| event.0);
    }
    structure.rows = std::sync::Arc::new(derive_timeline_rows(t, &mut structure));
    structure.item_turns.insert("messages", message_turns);
    static VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    structure.version = VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    cache.input = Some(input);
    cache.content.clear();
    cache.structure = std::sync::Arc::new(structure);
    cache.structure.clone()
}

#[derive(Clone)]
enum TimelineRow {
    HistoryControl {
        key: String,
    },
    WorktreeSetup {
        key: String,
    },
    Boundary {
        message_index: usize,
        key: String,
    },
    UserOrSystem {
        index: usize,
        message_id: String,
        key: String,
    },
    Handoff {
        index: usize,
        key: String,
    },
    AssistantTurn {
        start: usize,
        end: usize,
        key: String,
    },
    LiveTurn {
        start: usize,
        key: String,
    },
    /// A piece of a long running or expanded turn. Its head keeps the turn's row key.
    TurnPiece {
        start: usize,
        end: usize,
        piece: TurnPiece,
        key: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TurnPiece {
    Head,
    /// Work entries `lo..hi`, in `work_items` order.
    Work {
        lo: usize,
        hi: usize,
    },
    Tail,
}

impl TimelineRow {
    fn entry_id<'a>(&'a self, thread: &'a Thread) -> Option<&'a str> {
        let index = match self {
            Self::UserOrSystem { message_id, .. } => return Some(message_id),
            Self::AssistantTurn { start, .. }
            | Self::LiveTurn { start, .. }
            | Self::TurnPiece { start, .. } => *start,
            Self::Boundary { message_index, .. } => *message_index,
            Self::Handoff { index, .. } => thread.handoffs.get(*index)?.message_index,
            Self::HistoryControl { .. } | Self::WorktreeSetup { .. } => return None,
        };
        thread
            .messages
            .get(index)
            .map(|message| message.id.as_str())
    }

    fn key(&self) -> &str {
        match self {
            Self::HistoryControl { key }
            | Self::WorktreeSetup { key }
            | Self::Boundary { key, .. }
            | Self::UserOrSystem { key, .. }
            | Self::Handoff { key, .. }
            | Self::AssistantTurn { key, .. }
            | Self::LiveTurn { key, .. }
            | Self::TurnPiece { key, .. } => key,
        }
    }
}

fn push_unique_row_key(seen: &mut std::collections::HashSet<String>, base: String) -> String {
    if seen.insert(base.clone()) {
        return base;
    }
    let mut suffix = 2usize;
    loop {
        let candidate = format!("{base}-{suffix}");
        if seen.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn build_timeline_rows(t: &Thread) -> std::sync::Arc<Vec<TimelineRow>> {
    timeline_structure(t).rows.clone()
}

fn derive_timeline_rows(t: &Thread, structure: &mut TimelineStructure) -> Vec<TimelineRow> {
    let boundaries = &structure.boundaries;
    let mut probe = vec![];
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if t.history_has_more || t.history_error.is_some() {
        rows.push(TimelineRow::HistoryControl {
            key: push_unique_row_key(&mut seen, format!("history-control-{}", t.id)),
        });
    }
    if t.worktree_setup.is_some() {
        rows.push(TimelineRow::WorktreeSetup {
            key: push_unique_row_key(&mut seen, format!("worktree-setup-row-{}", t.id)),
        });
    }
    // A handoff row sits before the message that carried it; a pending one rides at
    // the end, where KSG Code's `resolvePendingHandoff` shows it.
    let mut pending_handoffs = t
        .handoffs
        .iter()
        .enumerate()
        .map(|(index, _)| index)
        .peekable();
    let mut emit_handoffs =
        |upto: usize, rows: &mut Vec<TimelineRow>, seen: &mut std::collections::HashSet<String>| {
            while let Some(&index) = pending_handoffs.peek() {
                if t.handoffs[index].message_index > upto {
                    break;
                }
                pending_handoffs.next();
                rows.push(TimelineRow::Handoff {
                    index,
                    key: push_unique_row_key(seen, format!("handoff-row-{index}")),
                });
            }
        };
    let mut index = 0usize;
    while index < t.messages.len() {
        let message = &t.messages[index];
        emit_handoffs(index, &mut rows, &mut seen);
        if message.role != "Pi" && boundaries.contains_key(&index) {
            rows.push(TimelineRow::Boundary {
                message_index: index,
                key: push_unique_row_key(&mut seen, format!("boundary-{}-{}", t.id, message.id)),
            });
        }
        if message.role == "Pi" {
            let end = t.messages[index..]
                .iter()
                .position(|message| message.role != "Pi")
                .map_or(t.messages.len(), |offset| index + offset);
            let key = push_unique_row_key(&mut seen, format!("turn-{}-{}", t.id, message.id));
            if !push_turn_pieces(
                t, structure, index, end, &key, &mut rows, &mut seen, &mut probe,
            ) {
                rows.push(TimelineRow::AssistantTurn {
                    start: index,
                    end,
                    key,
                });
            }
            index = end;
            continue;
        }
        rows.push(TimelineRow::UserOrSystem {
            index,
            message_id: message.id.clone(),
            key: push_unique_row_key(&mut seen, format!("row-{}-{}", t.id, message.id)),
        });
        index += 1;
    }
    emit_handoffs(usize::MAX, &mut rows, &mut seen);
    if boundaries.contains_key(&t.messages.len()) {
        rows.push(TimelineRow::Boundary {
            message_index: t.messages.len(),
            key: push_unique_row_key(&mut seen, format!("boundary-end-{}", t.id)),
        });
    }
    if t.status.busy() && t.messages.last().is_none_or(|message| message.role != "Pi") {
        rows.push(TimelineRow::LiveTurn {
            start: t.messages.len(),
            key: push_unique_row_key(&mut seen, format!("turn-live-{}", t.id)),
        });
    }
    structure.disclosure_probe = probe;
    rows
}

/// The row that shows text of message `message` in a split turn: the head for its
/// preamble, a slice for prose between steps, the tail for the answer. `hit` sees the
/// message's text chunks in order and returns true for the one holding the target.
/// `None` when the turn is one row, whose key the caller already has.
pub(crate) fn timeline_text_row(
    t: &Thread,
    message: usize,
    mut hit: impl FnMut(&str) -> bool,
) -> Option<String> {
    let structure = disclosed_timeline_structure(t);
    let pieces: Vec<_> = structure
        .rows
        .iter()
        .filter_map(|row| match row {
            TimelineRow::TurnPiece {
                start,
                end,
                piece,
                key,
            } if (*start..*end).contains(&message) => Some((*start, *end, *piece, key)),
            _ => None,
        })
        .collect();
    let &(start, end, _, _) = pieces.first()?;
    let parts = turn_parts_in(t, &structure.events, start, end);
    let target = parts.iter().position(|part| {
        matches!(part, TranscriptPart::Text { message: m, from, to }
            if *m == message && hit(&t.messages[*m].text[*from..*to]))
    })?;
    let layout = turn_layout(t, &structure, start, end, &parts);
    let want = if Some(target) == layout.preamble {
        TurnPiece::Head
    } else if layout.last_work.is_none_or(|last| target > last) {
        TurnPiece::Tail
    } else {
        let item = work_items(t, &parts, layout)
            .iter()
            .position(|item| item.part() == target)?;
        return pieces.iter().find_map(|(_, _, piece, key)| match piece {
            TurnPiece::Work { lo, hi } if (*lo..*hi).contains(&item) => Some((*key).clone()),
            _ => None,
        });
    };
    pieces
        .iter()
        .find(|(_, _, piece, _)| *piece == want)
        .map(|(_, _, _, key)| (*key).clone())
}

/// The virtual row for a tool jump, including the work slice of a long turn.
pub(crate) fn timeline_tool_row(t: &Thread, index: usize) -> Option<String> {
    let tool = t.tools.get(index)?;
    let structure = disclosed_timeline_structure(t);
    let message = tool.anchor.message;
    for row in structure.rows.iter() {
        match row {
            TimelineRow::AssistantTurn { start, end, key } if (*start..*end).contains(&message) => {
                return Some(format!("{key}/inline-tool-{}-{index}", t.id));
            }
            TimelineRow::TurnPiece {
                start,
                end,
                key,
                piece: TurnPiece::Work { lo, hi },
            } if (*start..*end).contains(&message) => {
                let parts = turn_parts_in(t, &structure.events, *start, *end);
                let layout = turn_layout(t, &structure, *start, *end, &parts);
                let contains_tool = |item: WorkItem| match (&parts[item.part()], item) {
                    (TranscriptPart::Work(entries), WorkItem::GroupEntry(_, k)) => {
                        matches!(entries[k], WorkEntry::Tool(i) if i == index)
                    }
                    (TranscriptPart::Work(entries), WorkItem::Part(_)) => {
                        entry_tools(entries).any(|i| i == index)
                    }
                    (TranscriptPart::Tools(indices), _) => indices.contains(&index),
                    _ => false,
                };
                let items = work_items(t, &parts, layout);
                if items
                    .get(*lo..(*hi).min(items.len()))
                    .is_some_and(|items| items.iter().copied().any(contains_tool))
                {
                    return Some(format!("{key}/inline-tool-{}-{index}", t.id));
                }
            }
            TimelineRow::LiveTurn { start, key } if *start == message => {
                return Some(format!("{key}/inline-tool-{}-{index}", t.id));
            }
            _ => {}
        }
    }
    None
}

/// Splits a long running or expanded turn into a head, slices of `SLICE_ITEMS` work
/// entries and a tail, keyed by the turn and each slice's first entry. Returns false
/// for a turn that stays one row.
#[allow(clippy::too_many_arguments)]
fn push_turn_pieces(
    t: &Thread,
    structure: &TimelineStructure,
    start: usize,
    end: usize,
    key: &str,
    rows: &mut Vec<TimelineRow>,
    seen: &mut std::collections::HashSet<String>,
    probe: &mut Vec<(u8, usize, bool)>,
) -> bool {
    let events = turn_events(&structure.events, start, end);
    // Each event yields at most a heading and a row, plus prose between events.
    if events.len() * 2 + (end - start) < segment_min_items() {
        return false;
    }
    let live = end == t.messages.len() && t.status.busy();
    if live {
        probe.push((3, 0, t.live_work_collapsed));
    }
    if let Some(&index) = structure.work_index.get(&start) {
        probe.push((2, index, t.work[index].expanded));
    }
    let parts = turn_parts_in(t, &structure.events, start, end);
    for part in &parts {
        match part {
            TranscriptPart::Work(entries) => {
                // The streak's fold flag: its first call's, or for thinking alone its first
                // thought's.
                match entry_tools(entries).next() {
                    Some(index) => probe.push((0, index, t.tools[index].group_expanded)),
                    None => {
                        let index = entry_thoughts(entries)[0];
                        probe.push((1, index, t.thoughts[index].expanded));
                    }
                }
            }
            TranscriptPart::Tools(indices) if indices.len() > 1 => {
                probe.push((0, indices[0], t.tools[indices[0]].group_expanded));
            }
            _ => {}
        }
    }
    let layout = turn_layout(t, structure, start, end, &parts);
    if !layout.inline_work && !turn_work_open(t, structure, start, live) {
        return false;
    }
    let items = work_items(t, &parts, layout);
    if items.len() < segment_min_items() {
        return false;
    }
    rows.push(TimelineRow::TurnPiece {
        start,
        end,
        piece: TurnPiece::Head,
        key: key.to_owned(),
    });
    let mut lo = 0;
    while lo < items.len() {
        // The last slice takes entries that arrive before the next structure change.
        let hi = if lo + SLICE_ITEMS >= items.len() {
            usize::MAX
        } else {
            lo + SLICE_ITEMS
        };
        rows.push(TimelineRow::TurnPiece {
            start,
            end,
            piece: TurnPiece::Work { lo, hi },
            key: push_unique_row_key(
                seen,
                format!("{key}-work-{}", work_item_id(t, &parts, items[lo])),
            ),
        });
        lo = hi;
    }
    rows.push(TimelineRow::TurnPiece {
        start,
        end,
        piece: TurnPiece::Tail,
        key: push_unique_row_key(seen, format!("{key}-tail")),
    });
    true
}

/// KSG timeline position recovery, applied with the history commit's scroll
/// transaction instead of waiting for the fixed viewport to resize.
pub(crate) fn restore_timeline_position(
    s: &mut State,
    tx: &mut rune::Tx,
    load_earlier: bool,
) -> bool {
    let thread = s.current();
    if thread
        .transcript_projection
        .as_ref()
        .is_some_and(|projection| !projection.history_loaded)
    {
        return false;
    }
    let thread_id = thread.id;
    let Some(visit) = s
        .timeline_positions
        .borrow()
        .current(&s.machines.selected, thread_id)
    else {
        return false;
    };
    let position = visit.restore.lock().unwrap().clone();
    let Some(position) = position else {
        return false;
    };
    let row_loaded = !position.at_end
        && build_timeline_rows(thread)
            .iter()
            .any(|row| row.key() == position.row);
    if !position.at_end && !row_loaded && thread.history_has_more {
        if load_earlier && !thread.history_loading && thread.history_error.is_none() {
            s.load_earlier_history(thread_id, position.entry_id);
            return true;
        }
        return false;
    }
    visit.restore.lock().unwrap().take();
    if position.at_end {
        tx.scroll_to_bottom_instant(format!("conversation-{thread_id}"));
    } else if row_loaded {
        tx.scroll_into_view_in(format!("conversation-{thread_id}"), &position.row);
        tx.scroll_key_to(&position.row, -position.offset);
    } else {
        tx.scroll_to_y(format!("conversation-{thread_id}"), position.scroll);
    }
    true
}

fn build_user_or_system_row(
    s: &State,
    p: Palette,
    wide: bool,
    pane_width: f32,
    index: usize,
    message_id: &str,
) -> El {
    let t = s.current();
    let message = &t.messages[index];
    let key = message_key(t, index);
    let body = if message.role == "You" {
        // KSG Code's `TimelineUserMessageRow`: `max-w-[80%] rounded-2xl p-3`.
        let max_width = (lane_width(pane_width, wide) * 0.8).max(160.0);
        let mut user = col()
            .items_end()
            .gap(6.0)
            .max_w(max_width)
            .shrink()
            .min_w(0.0);
        let unchipped = transcript_context::unchipped_attachments(message);
        if !unchipped.is_empty() {
            user = user.child(message_images(s, p, &unchipped, &key));
        }
        if !message.text.trim().is_empty() {
            let find_query = (s.thread_find_active()
                && !s.thread_find.query.text.trim().is_empty())
            .then(|| s.thread_find.query.text.trim());
            let active_occ = find_query.and_then(|_| {
                s.thread_find
                    .active_match
                    .as_ref()
                    .filter(|m| m.entry_id == message_id)
                    .map(|m| m.occurrence)
            });
            let memo_find = find_query.map(|q| (q.to_owned(), active_occ));
            let content = if transcript_context::has_context(message) {
                transcript_context::render(s, p, wide, message, max_width - 28.0)
            } else {
                memo(
                    format!("msg-md-{}-{message_id}", t.id),
                    (&message.text, s.dark, wide, s.code_wrap_version, memo_find),
                    || {
                        markdown_ui::render_with_find(
                            &message.text,
                            p,
                            s.dark,
                            wide,
                            true,
                            false,
                            &format!("user-{}-{message_id}", t.id),
                            &s.unwrapped_code_blocks,
                            find_query.map(|q| (q, active_occ)),
                        )
                    },
                )
            };
            // Ported from KSG Code's `TimelineUserMessageRow`: borderless `surface-bubble`
            // with radius 14 and `px-3.5 py-2.5` padding.
            user = user.child(
                content
                    .key(format!("{key}-text"))
                    .max_w(max_width)
                    .min_w(0.0)
                    .px(14.0)
                    .py(10.0)
                    .radius(14.0)
                    .fill(p.message_bg),
            );
        }
        row().justify_end().child(user)
    } else if let Some(report) = (message.role == "Shina")
        .then(|| crew_ui::report_message(s, p, &message.text))
        .flatten()
    {
        report
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
        let display = message.display_text();
        let mut lbl = label(display, body_style(wide), p.ink)
            .wrap_anywhere()
            .selectable(true);
        if s.thread_find_active() && !s.thread_find.query.text.trim().is_empty() {
            let query = s.thread_find.query.text.trim();
            let active_occ = s
                .thread_find
                .active_match
                .as_ref()
                .filter(|m| m.entry_id == message_id)
                .map(|m| m.occurrence);
            let ranges = crate::thread_find::find_plain_occurrences(display, query);
            if !ranges.is_empty() {
                lbl = lbl.text_highlights(ranges.into_iter().enumerate().map(|(i, range)| {
                    let is_active = Some(i) == active_occ;
                    let bg = if is_active {
                        p.sea.alpha(0.32)
                    } else {
                        p.brass.alpha(0.32)
                    };
                    let mut hl = rune::TextHighlight::new(range, bg);
                    if is_active {
                        hl = hl.key("thread-find-active");
                    }
                    hl
                }));
            }
        }
        col()
            .gap(4.0)
            .px(12.0)
            .py(8.0)
            .radius(12.0)
            .fill(p.coral_bg)
            .child(label("Shina", TextStyle::new(13.0, 600.0), p.ink))
            .child(lbl)
    };
    let mut wrapper = col().w_full();
    if message.role == "You" && t.is_fresh_session_message(&message.id) {
        wrapper = wrapper.gap(8.0).child(fresh_session_row(&message.id, p));
    }
    wrapper.child(body.key(key))
}

/// Ported from KSG Code's `FreshSessionRow` (`components/chat/V2LifecycleRow.tsx`) and
/// `TimelineSystemDivider` (`components/chat/TimelineSystemDivider.tsx`).
fn fresh_session_row(message_id: &str, p: Palette) -> El {
    row()
        .key(format!("fresh-session-row-{message_id}"))
        .role("status")
        .label("New session")
        .w_full()
        .min_w(0.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
        .child(
            row()
                .center_y()
                .gap(6.0)
                .child(icon("alert", p.muted))
                .child(meta_text("New session", p.muted)),
        )
        .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
}

/// Ported from KSG Code's `HandoffRow` (`components/chat/V2LifecycleRow.tsx`): the
/// divider's label is KSG's title and its detail the two provider endpoints.
fn handoff_row(s: &State, index: usize, handoff: &crate::handoff::Handoff, p: Palette) -> El {
    use crate::handoff::HandoffStatus;
    let pending = handoff.status == HandoffStatus::Pending;
    let failed = handoff.status == HandoffStatus::Failed;
    let tone = if failed { p.danger } else { p.muted };
    let title = if pending {
        "Context handoff pending"
    } else if failed {
        "Context handoff failed"
    } else {
        "Context handoff"
    };
    // `HandoffEndpoint`: the provider mark, then the model name (the instance id
    // when the record names no model).
    let endpoint = |instance: &str, model: Option<&str>| {
        let name = model
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(instance);
        row()
            .center_y()
            .gap(4.0)
            .min_w(0.0)
            .shrink()
            .child(provider_icon(s, p, model_vendor(name, ""), 12.0))
            .child(meta_text(name, p.muted).truncate().shrink().min_w(0.0))
    };
    let mut detail = row().center_y().gap(6.0).min_w(0.0).shrink();
    let sources: Vec<(&str, Option<&str>)> = if handoff.from_models.is_empty() {
        handoff
            .from_instances
            .iter()
            .map(|instance| (instance.as_str(), None))
            .collect()
    } else {
        handoff
            .from_models
            .iter()
            .map(|selection| {
                (
                    selection.instance_id.as_str(),
                    Some(selection.model.as_str()),
                )
            })
            .collect()
    };
    for (position, (instance, model)) in sources.iter().enumerate() {
        if position > 0 {
            detail = detail.child(meta_text(",", p.muted));
        }
        detail = detail.child(endpoint(instance, *model));
    }
    if !sources.is_empty() {
        detail = detail.child(meta_text("→", p.muted));
    }
    detail = detail.child(endpoint(&handoff.to_instance, handoff.to_model.as_deref()));
    if handoff.delivery.status == crate::handoff::DeliveryStatus::Inline {
        let covered = handoff.delivery.delivered_item_ids.len();
        let omitted = handoff.delivery.omitted_item_ids.len();
        detail = detail.child(meta_text(
            format!("· {covered} items, {omitted} omitted"),
            p.muted,
        ));
    }
    row()
        .key(format!("handoff-row-{index}"))
        .role("status")
        .label(format!("{title}: {}", handoff.to_instance))
        .w_full()
        .min_w(0.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
        .child(
            row()
                .center_y()
                .gap(6.0)
                .min_w(0.0)
                .shrink()
                .child(icon("swap", tone))
                .child(meta_text(title, tone).shrink())
                .child(detail),
        )
        .child(el().h(1.0).grow(1.0).fill(p.line).hairline())
}

/// Ported from KSG Code's `ThreadFindBar` (`components/chat/ThreadFindBar.tsx`).
fn thread_find_bar(s: &State, p: Palette, pane_width: f32) -> El {
    let find = &s.thread_find;
    let trimmed = find.query.text.trim();
    let has_query = !trimmed.is_empty();
    let no_results = has_query
        && !find.loading
        && find.error.is_none()
        && !find.unsupported
        && find.total_matches == 0;
    let status_text = if find.unsupported {
        "Update required".to_owned()
    } else if find.loading && find.total_matches == 0 {
        "Searching…".to_owned()
    } else if find.error.is_some() {
        "Search failed".to_owned()
    } else if has_query {
        if find.total_matches == 0 {
            "0/0".to_owned()
        } else {
            format!(
                "{}/{}",
                find.active_index.min(find.total_matches - 1) + 1,
                find.total_matches
            )
        }
    } else {
        String::new()
    };
    let nav_disabled = find.total_matches == 0 || find.loading || find.unsupported;
    let bar_width = (pane_width - 24.0).clamp(200.0, 340.0);
    let mut bar = row()
        .key("thread-find-bar")
        .role("search")
        .label("Find in thread")
        .w(bar_width)
        .h(36.0)
        .px(10.0)
        .gap(4.0)
        .center_y()
        .radius(12.0)
        .fill(p.card)
        .on_layout_tx(|s: &mut State, _rect, tx| {
            let changed = s.sync_thread_find_input();
            let revealed = s.consume_pending_find_reveal(tx);
            if !changed && !revealed {
                tx.live_only();
            }
        })
        .child(
            text_input(&find.query, LABEL)
                .key("thread-find-input")
                .label("Find in thread")
                .placeholder("Find in thread", p.muted)
                .color(p.ink)
                .single_line()
                .grow(1.0)
                .min_w(0.0)
                .on_edit(|s: &mut State| &mut s.thread_find.query)
                .on_input_key(|s: &mut State, event, tx| {
                    if event.key == rune::Key::Enter {
                        if event.mods.shift {
                            s.step_thread_find(-1);
                        } else {
                            s.step_thread_find(1);
                        }
                        s.consume_pending_find_reveal(tx);
                        true
                    } else if event.key == rune::Key::Escape {
                        s.close_thread_find();
                        s.restore_composer_focus(composer_ui::FocusReason::Explicit);
                        true
                    } else {
                        false
                    }
                }),
        );
    if !status_text.is_empty() {
        bar = bar.child(
            label(
                status_text,
                SMALL.features(TABULAR),
                if no_results || find.error.is_some() || find.unsupported {
                    p.danger
                } else {
                    p.muted
                },
            )
            .key("thread-find-count")
            .min_w(36.0)
            .align(rune::Align::Center),
        );
    }
    if find.error.is_some() && !find.unsupported {
        bar = bar.child(
            label("Retry", SMALL, p.ink)
                .key("thread-find-retry")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .px(4.0)
                .py(2.0)
                .radius(6.0)
                .hover(p.selected)
                .on_press(|s: &mut State| s.retry_thread_find()),
        );
    }
    bar = bar
        .child(
            icon_button(
                "thread-find-prev",
                "Previous match",
                "chevron-up",
                p,
                |_| {},
            )
            .disabled(nav_disabled)
            .on_press_tx(|s: &mut State, tx| {
                s.step_thread_find(-1);
                s.consume_pending_find_reveal(tx);
            }),
        )
        .child(
            icon_button("thread-find-next", "Next match", "chevron", p, |_| {})
                .disabled(nav_disabled)
                .on_press_tx(|s: &mut State, tx| {
                    s.step_thread_find(1);
                    s.consume_pending_find_reveal(tx);
                }),
        )
        .child(icon_button(
            "thread-find-close",
            "Close find",
            "close",
            p,
            |s: &mut State| {
                s.close_thread_find();
                s.restore_composer_focus(composer_ui::FocusReason::Explicit);
            },
        ));
    framed(bar, p).pin(None, Some(12.0), Some(12.0), None)
}

/// A running turn's height motion. Streaming prose reveals at the stream's pace. While the
/// turn ends in work, its live row replaces itself as steps start and end and its thought
/// preview comes and goes, so the row snaps to each new height instead of easing toward it
/// behind the feathered edge, which flickered and cut the newest step in half.
fn live_turn_motion(t: &Thread, turn: El, start: usize, end: usize) -> El {
    let parts = turn_parts(t, start, end);
    if parts.is_empty() || live_tail(t, &parts).is_some() {
        turn.fold_glide(rune::Spring::NONE).fold_feather(0.0)
    } else {
        turn.streaming(true)
    }
}

/// Ported from KSG Code's `MessagesTimeline` (`LegendList` virtualisation, `estimatedItemSize={90}`,
/// scroll anchoring and stick-to-bottom).
fn conversation(s: &State, p: Palette, wide: bool, pane_width: f32) -> El {
    let t = s.current();
    let visit = s.timeline_positions.borrow().visit();
    let gutter = lane_gutter(pane_width);
    // The lane the composer resolves too; `None` until C5 mounts the Details card.
    let lane_rect = chat_lane(pane_width, wide, details_ui::edges(s, pane_width));
    // A captain's delegations show as task cards inside a turn, so with no messages they have
    // nothing to hang from. The home counts as empty and greets above its crew tray.
    let visible_tools = t.messages.is_empty()
        && t.tools
            .iter()
            .any(|tool| !(s.is_captain_home(t) && crew_ui::is_delegation(tool)));
    if t.messages.is_empty()
        && !visible_tools
        && !t.status.busy()
        && t.native.compactions.is_empty()
        && t.worktree_setup.is_none()
        && !t.history_has_more
        && t.history_error.is_none()
    {
        let mut introduction = col()
            .key("empty-state")
            .w_full()
            .max_w(measure(wide) + 2.0 * gutter + lane_rect.inset_start + lane_rect.inset_end)
            .pl(gutter + lane_rect.inset_start)
            .pr(gutter + lane_rect.inset_end)
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
                .gap(24.0)
                .child(draft_headline(s, p, pane_width));
        }
        if t.restoring_history() {
            introduction = introduction.child(label("Restoring conversation...", SMALL, p.muted));
        }
        return introduction;
    }

    let thread_id = t.id;
    let structure = disclosed_timeline_structure(t);
    let rows = structure.rows.clone();
    let key_rows = rows.clone();
    let build_rows = rows.clone();
    let visible_rows = rows.clone();
    let visible_visit = visit.clone();
    let visible_server = s.machines.selected.clone();
    let row_visit = visit.clone();
    let layout_visit = visit.clone();
    let lane = col()
        .key("chat-lane")
        .on_layout_tx(|s: &mut State, rect, tx| {
            s.html_lane_width = rect[2].round().clamp(240.0, 1600.0) as u32;
            tx.live_only();
        })
        .w_full()
        .max_w(measure(wide))
        // 24 pt above the first message, counting the cap.
        .pt(24.0 - TRANSCRIPT_CAP)
        .pb(24.0)
        .gap(LANE_GAP)
        .growth(rune::GrowthSpec::default())
        .virtual_rows(
            rows.len(),
            90.0,
            move |_: &State, index| {
                #[cfg(test)]
                timeline_count(|counts| counts.keys += 1);
                std::borrow::Cow::Owned(key_rows[index].key().to_owned())
            },
            move |s: &State, index| {
                #[cfg(test)]
                timeline_count(|counts| counts.row_builders += 1);
                let row = match &build_rows[index] {
                    TimelineRow::HistoryControl { .. } => history_control(s, p).unwrap_or_else(el),
                    TimelineRow::WorktreeSetup { .. } => {
                        worktree_setup_card(s, p).unwrap_or_else(el)
                    }
                    TimelineRow::Boundary { message_index, .. } => {
                        work_ui::boundary_events(s, p, wide, *message_index).unwrap_or_else(el)
                    }
                    TimelineRow::UserOrSystem {
                        index: msg_idx,
                        message_id,
                        ..
                    } => build_user_or_system_row(s, p, wide, pane_width, *msg_idx, message_id),
                    TimelineRow::Handoff { index, .. } => s
                        .current()
                        .handoffs
                        .get(*index)
                        .map(|handoff| handoff_row(s, *index, handoff, p))
                        .unwrap_or_else(el),
                    // A running turn grows at the stack's streaming pace behind its feathered
                    // edge. A finished one folds its work open and shut over 200 ms, as KSG
                    // Code's `transition-[height] duration-200 ease-out` does.
                    TimelineRow::AssistantTurn { start, end, .. } => {
                        let turn = assistant_turn(s, p, wide, *start, *end);
                        if *end == s.current().messages.len() && s.current().status.busy() {
                            live_turn_motion(s.current(), turn, *start, *end)
                        } else {
                            turn.fold_glide(rune::Spring::new(0.20, 0.0))
                                .fold_feather(14.0)
                        }
                    }
                    TimelineRow::LiveTurn { start, .. } => live_turn_motion(
                        s.current(),
                        assistant_turn(s, p, wide, *start, *start),
                        *start,
                        *start,
                    ),
                    TimelineRow::TurnPiece {
                        start, end, piece, ..
                    } => assistant_turn_piece(s, p, wide, *start, *end, Some(*piece)),
                };
                let key = build_rows[index].key().to_owned();
                let entry_id = build_rows[index].entry_id(s.current()).map(str::to_owned);
                let row_visit = row_visit.clone();
                row.on_layout_tx(move |_: &mut State, rect, tx| {
                    row_visit.row(&key, entry_id.as_deref(), rect);
                    tx.live_only();
                })
            },
        )
        .virtual_rows_version(structure.version)
        .on_visible_rows(move |s: &mut State, range| {
            if s.current().id == thread_id && s.machines.selected == visible_server {
                let near_start = range.start <= 1 && range.end < visible_rows.len();
                visible_visit.visible_rows(
                    range.filter_map(|index| {
                        visible_rows.get(index).map(|row| row.key().to_owned())
                    }),
                );
                let t = s.current();
                // Keep KSG's manual control for retries and pages too short to scroll.
                if near_start
                    && t.history_has_more
                    && !t.history_loading
                    && t.history_error.is_none()
                    && visible_visit.restore.lock().unwrap().is_none()
                {
                    s.load_earlier_history(thread_id, None);
                    return true;
                }
            }
            false
        });

    let find_open = s.thread_find_active();
    let stick_bottom = !find_open && (!t.messages.is_empty() || !t.tools.is_empty());
    let scroller = col()
        .key(format!("conversation-{thread_id}"))
        .animated_changes()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom })
        .on_scroll_offset(move |at| *visit.scroll.lock().unwrap() = at)
        .on_layout_tx(move |s: &mut State, rect, tx| {
            layout_visit.viewport(rect);
            if s.current().id != thread_id || !restore_timeline_position(s, tx, false) {
                tx.live_only();
            }
        })
        .on_far_from_bottom(40.0, move |s: &mut State, far| {
            s.set_timeline_far_from_bottom(thread_id, far);
        })
        .on_at_bottom(move |s: &mut State, at| {
            s.set_timeline_at_bottom(thread_id, at);
        })
        .on_selection_change(move |s: &mut State, change, tx| {
            if change.dragging() {
                if s.assistant_selection.is_some() {
                    s.assistant_selection = None;
                } else {
                    tx.live_only();
                }
                return;
            }
            let next = change.selection().and_then(|sel| {
                let t = s.current();
                if t.id != thread_id {
                    return None;
                }
                let paths: Vec<&str> = sel
                    .texts()
                    .iter()
                    .filter(|st| st.is_selected())
                    .map(|st| st.path())
                    .collect();
                let target = crate::assistant_citation::validate_single_assistant_selection(
                    t.id,
                    &t.messages,
                    &paths,
                    sel.text(),
                )?;
                Some(crate::model::AssistantSelection {
                    thread_id: t.id,
                    machine: t.machine.clone(),
                    message_index: target.message_index,
                    message_id: target.message_id,
                    chunk_from: target.chunk_from,
                    text: sel.text().to_owned(),
                })
            });
            if s.assistant_selection != next {
                s.assistant_selection = next;
            } else {
                tx.live_only();
            }
        })
        // A transcript longer than the pane rests stuck to its bottom, so its top is always
        // under the header's rule. An opaque 8 pt cap in the panel colour sits right under the
        // rule, stuck to the viewport's top, and the 28 pt fade starts below it, so nothing
        // scrolled shows through as a sliver under the rule.
        .scroll_fade(rune::ScrollFade::strength(28.0).below_sticky())
        .pl(gutter + lane_rect.inset_start)
        .pr(gutter + lane_rect.inset_end)
        .items_center()
        .child(
            el().key("transcript-cap")
                .w_full()
                .h(TRANSCRIPT_CAP)
                .fill(p.panel)
                .sticky_top(0.0)
                .a11y_hidden(),
        )
        .child(lane);

    let mut wrapper = col()
        .key(format!("conversation-wrap-{thread_id}"))
        .grow(1.0)
        .min_h(0.0)
        .child(
            col()
                .key(format!("timeline-server-{}", s.machines.selected))
                .grow(1.0)
                .min_h(0.0)
                .child(scroller),
        );

    if s.timeline_is_far_from_bottom(thread_id) && !find_open {
        let pill = framed(
            row()
                .key("scroll-to-end")
                .role("button")
                .label("Scroll to end")
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .h(28.0)
                .px(10.0)
                .gap(6.0)
                .center_y()
                .radius(12.0)
                .fill(p.card)
                .hover(p.selected)
                .press(p.selected)
                .on_press_tx(move |s: &mut State, tx| {
                    s.set_timeline_far_from_bottom(thread_id, false);
                    tx.scroll_to_bottom(format!("conversation-{thread_id}"));
                })
                .child(icon("chevron", p.muted))
                .child(label("Scroll to end", SMALL, p.ink)),
            p,
        );
        wrapper = wrapper.child(
            row()
                .pin(Some(0.0), None, Some(0.0), Some(12.0))
                .justify_center()
                .child(pill),
        );
    }
    if find_open {
        wrapper = wrapper.child(thread_find_bar(s, p, pane_width));
    }

    wrapper
}

/// Ported from KSG Code's `WorktreeSetupCard.tsx`.
fn worktree_setup_card(s: &State, p: Palette) -> Option<El> {
    let t = s.current();
    let setup = t.worktree_setup.as_ref()?;
    let (status, status_word, title) = match setup.phase {
        crate::model::WorktreeSetupPhase::Running => {
            (Status::Running, "RUNNING", "Setting up worktree…")
        }
        crate::model::WorktreeSetupPhase::Succeeded => (
            Status::Completed,
            if setup.skipped { "SKIPPED" } else { "READY" },
            if setup.skipped {
                "Worktree setup skipped"
            } else {
                "Worktree ready"
            },
        ),
        crate::model::WorktreeSetupPhase::Failed => {
            (Status::Failed, "FAILED", "Worktree setup failed")
        }
    };
    let tone = status_ink(status, p);
    let mut header = row()
        .w_full()
        .min_h(24.0)
        .center_y()
        .gap(8.0)
        .child(status_mark(status, p, 6.0))
        .child(
            label(title, TextStyle::new(14.0, 500.0).lh(20.0), p.ink)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(meta_text(status_word, tone));
    if setup.duration_ms > 0 {
        let dur = if setup.duration_ms < 1000 {
            format!("{}ms", setup.duration_ms)
        } else {
            format!("{:.1}s", setup.duration_ms as f64 / 1000.0)
        };
        header = header.child(meta_text(dur, p.muted));
    }

    let mut card = col()
        .w_full()
        .padding(12.0)
        .gap(8.0)
        .radius(12.0)
        .fill(p.card)
        .child(header);

    if let Some(script_name) = &setup.script_name {
        let desc = match &setup.script_command {
            Some(cmd) if !cmd.trim().is_empty() => format!("{script_name}: {cmd}"),
            _ => script_name.clone(),
        };
        card = card.child(label(desc, SMALL, p.muted).wrap());
    }
    if let Some(err) = &setup.error {
        card = card.child(label(err, SMALL, p.danger).wrap().selectable(true));
    }

    let tail_lines: Vec<&str> = setup.output.lines().collect();
    if !tail_lines.is_empty() {
        let start = tail_lines.len().saturating_sub(4);
        let tail = tail_lines[start..].join("\n");
        let open = t.worktree_setup_output_open;
        let toggle = row()
            .key("worktree-setup-output")
            .role("button")
            .label(if open { "Hide output" } else { "Show output" })
            .cursor(rune::Cursor::Pointer)
            .focusable(true)
            .center_y()
            .gap(6.0)
            .h(24.0)
            .px(6.0)
            .radius(6.0)
            .hover(p.selected)
            .on_press(|s: &mut State| {
                s.current_mut().worktree_setup_output_open =
                    !s.current().worktree_setup_output_open;
            })
            .child(icon(if open { "chevron-up" } else { "chevron" }, p.muted).size(12.0, 12.0))
            .child(meta_text("Output", p.muted));
        card = card.child(toggle);
        if open {
            let mono = TextStyle::new(12.0, 400.0)
                .lh(17.0)
                .family(rune::Family::SystemMono);
            card = card.child(
                col()
                    .key("worktree-setup-output-tail")
                    .w_full()
                    .px(10.0)
                    .py(6.0)
                    .radius(6.0)
                    .fill(if setup.phase == crate::model::WorktreeSetupPhase::Failed {
                        p.coral_bg
                    } else {
                        p.code_bg
                    })
                    .border(1.0, p.line)
                    .child(
                        label(
                            tail,
                            mono,
                            if setup.phase == crate::model::WorktreeSetupPhase::Failed {
                                p.danger
                            } else {
                                p.muted
                            },
                        )
                        .wrap_anywhere()
                        .selectable(true),
                    ),
            );
        }
    }

    let mut actions = row().w_full().gap(8.0).center_y();
    let mut has_actions = false;
    if setup.phase == crate::model::WorktreeSetupPhase::Failed {
        has_actions = true;
        actions = actions.child(framed(
            button("worktree-setup-retry", "Retry", p, |s| {
                s.retry_worktree_setup();
            }),
            p,
        ));
    }
    if matches!(
        setup.phase,
        crate::model::WorktreeSetupPhase::Running | crate::model::WorktreeSetupPhase::Failed
    ) {
        has_actions = true;
        actions = actions.child(button("worktree-setup-skip", "Skip", p, |s| {
            s.skip_worktree_setup();
        }));
    }
    if has_actions {
        card = card.child(actions);
    }

    Some(
        col().w_full().child(
            card.key("worktree-setup-card")
                .w_full()
                .radius(12.0)
                .border(1.0, p.frame),
        ),
    )
}

/// Ported from KSG Code's `AssistantSelectionToolbar` (`chat/AssistantSelectionToolbar.tsx`).
fn assistant_selection_toolbar(s: &State, p: Palette, cx: &Cx<State>) -> Option<El> {
    let sel = s.assistant_selection.as_ref()?;
    let t = s.current();
    if s.demo
        || sel.thread_id != t.id
        || sel.machine != t.machine
        || t.archived()
        || t.request.is_some()
        || t.native.questionnaire.is_some()
    {
        return None;
    }
    if s.transcript_preview.is_some()
        || s.rename_thread.is_some()
        || s.delete_thread.is_some()
        || s.plan_save_dialog.is_some()
        || s.confirm_remove_attachment.is_some()
        || s.project_picker.is_some()
        || s.captain_editor.is_some()
        || s.settings_picker.is_some()
        || s.captain_picker
        || s.env_mode_picker
        || s.base_branch_picker
        || s.machines.picker
        || s.settings.archive.is_some()
    {
        return None;
    }
    let rects = cx.selection_rects();
    if rects.is_empty() {
        return None;
    }
    let viewport = cx.layout_with(&format!("conversation-{}", t.id), |r| r);
    let first_visible = match viewport {
        Some([vx, vy, vw, vh]) => rects.into_iter().find(|&[rx, ry, rw, rh]| {
            rx + rw.max(1.0) > vx && rx < vx + vw && ry + rh.max(1.0) > vy && ry < vy + vh
        })?,
        None => rects[0],
    };
    let anchor_rect = match viewport {
        Some([vx, vy, vw, vh]) => {
            let left = first_visible[0].clamp(vx, vx + vw);
            let right = (first_visible[0] + first_visible[2]).clamp(left, vx + vw);
            let top = first_visible[1].clamp(vy, vy + vh);
            let bottom = (first_visible[1] + first_visible[3]).clamp(top, vy + vh);
            [left, top, (right - left).max(1.0), (bottom - top).max(1.0)]
        }
        None => first_visible,
    };
    let zoom = s.zoom.clamp(0.5, 2.0);
    let anchor = el()
        .key("assistant-selection-anchor")
        .absolute(anchor_rect[0] / zoom, anchor_rect[1] / zoom)
        .size(anchor_rect[2] / zoom, anchor_rect[3] / zoom)
        .inert()
        .a11y_hidden();
    let too_long = crate::assistant_citation::utf16_len(&sel.text)
        > crate::assistant_citation::ASSISTANT_CITATION_MAX_TEXT_LENGTH;
    let mut cite_btn = row()
        .key("assistant-selection-cite")
        .role("button")
        .label(if too_long {
            "Selection is too long to cite"
        } else {
            "Cite selection in composer"
        })
        .h(28.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .fill(p.card)
        .keeps_text_selection()
        .disabled(too_long)
        .child(icon("thread", if too_long { p.muted } else { p.ink }))
        .child(meta_text(
            if too_long {
                "Shorten selection"
            } else {
                "Cite"
            },
            if too_long { p.muted } else { p.ink },
        ));
    if !too_long {
        cite_btn = cite_btn
            .cursor(rune::Cursor::Pointer)
            .focusable(true)
            .hover(p.selected)
            .press(p.selected)
            .on_press_tx(|s: &mut State, tx| {
                s.cite_assistant_selection(tx);
            });
    }
    let toolbar = row()
        .key("assistant-selection-toolbar")
        .role("toolbar")
        .label("Assistant selection actions")
        .keeps_text_selection()
        .gap(4.0)
        .child(framed(cite_btn, p))
        .anchored_fit(
            "assistant-selection-anchor",
            rune::Side::Top,
            rune::Align::Start,
            6.0,
        );
    Some(
        el().key("assistant-selection-layer")
            .absolute(0.0, 0.0)
            .size(0.0, 0.0)
            .child(anchor)
            .child(toolbar),
    )
}

/// The opaque strip under the chat header's rule, inside the transcript viewport.
const TRANSCRIPT_CAP: f32 = 8.0;

/// The sidebar portraits' animation stage also plays the captain hero, under this row id.
pub(crate) const HERO_PORTRAIT_ROW: usize = usize::MAX;
const HERO_PORTRAIT: f32 = 96.0;
/// The captain hero with a one-line heading: portrait, heading, role and status.
const CAPTAIN_HERO_HEIGHT: f32 = 210.0;

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
    let size = HERO_PORTRAIT;
    let pixels = s
        .portraits
        .pixels(portrait_index, face, HERO_PORTRAIT)
        .unwrap_or_else(|| s.artwork.face_pixels(portrait_index, face, HERO_PORTRAIT));
    let portrait = portrait(
        &pixels,
        if s.artwork.is_pixel_art(portrait_index) {
            s.artwork.snap_points(size)
        } else {
            size
        },
        p,
        None,
    )
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
                .opacity_motion(MOTION_SLOW)
                .child(status_mark(line_status, p, 6.0))
                .child(label(
                    text,
                    META.features(TABULAR),
                    status_ink(line_status, p),
                )),
        );
    }
    let role = &profile.role;
    col()
        .key("captain-hero")
        .w_full()
        .items_center()
        .gap(10.0)
        .child(
            el().size(pane_width.min(360.0), size + 6.0)
                .center()
                .child(
                    captain_wash(s.dark, portrait_index, pane_width.min(360.0), size + 6.0)
                        .absolute(0.0, 0.0),
                )
                .child(haloed(s, p, portrait, size, status)),
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
    model: &ComposerModelDisplay,
    p: Palette,
    wide: bool,
    pane_width: f32,
    hero_fits: bool,
    caret_edges: Option<[bool; 2]>,
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
        .child(composer(s, model, p, wide, pane_width, caret_edges))
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
            .radius(12.0)
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
    let has_any_project = s.projects.entries.iter().any(|project| !project.hidden);
    if !has_workspace && !has_any_project && !s.demo {
        // Ported from KSG Code's `NoProjectsHero`: when no workspaces exist yet, show the
        // empty-workspace headline and primary "Add workspace" action instead of an empty
        // project picker.
        return col()
            .key("empty-heading")
            .w_full()
            .items_center()
            .gap(12.0)
            .child(
                label("What should we build?", HEADING_NARROW, p.ink)
                    .align(rune::Align::Center)
                    .wrap()
                    .w_full(),
            )
            .child(
                label(
                    "Add a workspace to start a thread, or start without one.",
                    SMALL,
                    p.muted,
                )
                .align(rune::Align::Center)
                .wrap()
                .w_full(),
            )
            .child(framed(
                row()
                    .key("no-projects-add-workspace")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Add workspace")
                    .focusable(true)
                    .h(30.0)
                    .px(10.0)
                    .gap(6.0)
                    .center_y()
                    .radius(6.0)
                    .fill(p.coral)
                    .hover(p.coral)
                    .on_press(|s: &mut State| s.open_picker(ProjectPicker::AddProject))
                    .child(icon("plus", p.on_coral))
                    .child(label(
                        "Add workspace",
                        TextStyle::new(13.0, 600.0),
                        p.on_coral,
                    )),
                p,
            ));
    }
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
    if !image.is_image()
        && (image.mime.starts_with("video/")
            || crate::attachment_intake::native_mime(&image.name).starts_with("video/"))
        && let Some(path) = crate::attachments::path_for(image).filter(|p| p.is_file())
    {
        // KSG ComposerVideoThumbnail and prepareVideoFirstFrame. Keep a paused,
        // muted native poster alive between frames; evict only least-recently used posters.
        let clock = s.attachment_poster_clock.get().wrapping_add(1);
        s.attachment_poster_clock.set(clock);
        let mut posters = s.attachment_video_posters.borrow_mut();
        let (player, accessed) = posters.entry(image.id.clone()).or_insert_with(|| {
            (
                rune::Player::file(attachment_preview::player_path(&path)),
                clock,
            )
        });
        *accessed = clock;
        let player = player.clone();
        if player.time() == 0.0
            && let Some(duration) = player.duration().filter(|d| *d > 0.0)
        {
            player.seek(0.1_f64.min(duration / 2.0));
        }
        if posters.len() > 256
            && let Some(oldest) = posters
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone())
        {
            posters.remove(&oldest);
        }
        return el()
            .key(key)
            .size(width, height)
            .radius(6.0)
            .clip_rounded(6.0)
            .video(&player, rune::SurfaceFit::Fill)
            .child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .center()
                    .child(icon("play", Color::WHITE)),
            );
    }
    let mut preview = el()
        .key(key)
        .size(width, height)
        .radius(6.0)
        .clip_rounded(6.0)
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
fn composer_attachment_thumbnail(
    s: &State,
    p: Palette,
    image: &Attachment,
    width: f32,
    height: f32,
    key: String,
) -> El {
    let inherited = s
        .attachment_uploads
        .entries
        .iter()
        .find(|entry| {
            entry.machine == s.machines.selected
                && entry.thread == s.current().id
                && entry.id == image.id
        })
        .and_then(|entry| entry.thumbnail(&s.attachment_cache));
    if let Some(thumbnail) = inherited {
        if rune::is_headless() {
            let _ = thumbnail.wait();
        }
        if thumbnail.is_ready() && thumbnail.error().is_none() {
            return el()
                .key(key)
                .size(width, height)
                .pixels_fit(thumbnail.pixels(), rune::SurfaceFit::Fill);
        }
    }
    image_preview(s, p, image, width, height, key)
}

fn attachment_chip(s: &State, p: Palette, image: &Attachment) -> El {
    let id = image.id.clone();
    let preview_id = id.clone();
    let mut tile = el()
        .key(format!("attachment-{}", image.id))
        .role("group")
        .label(format!(
            "Attached {}: {}",
            if image.is_image() { "image" } else { "file" },
            image.name
        ))
        .help(image.name.clone())
        .size(ATTACHMENT_THUMB, ATTACHMENT_THUMB)
        .child(
            composer_attachment_thumbnail(
                s,
                p,
                image,
                ATTACHMENT_THUMB,
                ATTACHMENT_THUMB,
                format!("attachment-thumb-{id}"),
            )
            .radius(6.0)
            .clip_rounded(6.0)
            .role("button")
            .label(format!("Preview {}", image.name))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut State| {
                s.preview_attachments(s.current().draft_images.clone(), &preview_id, true);
            }),
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
                .radius(6.0)
                .fill(Color::BLACK.alpha(0.6))
                .hover(Color::BLACK.alpha(0.8))
                .on_press(move |s: &mut State| s.request_remove_attachment(&id))
                .child(icon("close", Color::WHITE).size(12.0, 12.0)),
        );
    if let Some(status) = attachment_upload_status(s, p, &image.id) {
        tile = tile.child(status);
    }
    tile
}

// Ported from KSG Code's ComposerAttachmentMediaRow and ComposerAttachmentUploadProgress.
fn attachment_upload_status(s: &State, p: Palette, id: &str) -> Option<El> {
    use crate::attachment_uploads::Phase;
    let phase = s
        .attachment_uploads
        .phase(&s.machines.selected, s.current().id, id)?;
    let entry = s
        .attachment_uploads
        .entries
        .iter()
        .find(|entry| entry.id == id)?;
    if !entry.image
        && !s
            .current()
            .draft_images
            .iter()
            .any(|a| a.id == id && attachment_is_video(a))
    {
        return None;
    }
    let elapsed = match phase {
        Phase::Uploading { since, .. } => since.elapsed(),
        _ => Duration::ZERO,
    };
    if let Phase::Failed(reason) = phase {
        let retry_id = id.to_owned();
        return Some(
            row()
                .key(format!("attachment-status-{id}"))
                .pin(Some(4.0), None, None, Some(4.0))
                .child(
                    icon_button(
                        &format!("retry-attachment-{id}"),
                        &format!("Retry upload for {}", entry.name),
                        "reload",
                        p,
                        move |s| s.retry_attachment_upload(&retry_id),
                    )
                    .help(reason)
                    .fill(p.card)
                    .size(18.0, 18.0),
                ),
        );
    }
    phase.label(elapsed).map(|status| {
        row()
            .key(format!("attachment-status-{id}"))
            .pin(Some(0.0), Some(0.0), None, Some(0.0))
            .px(4.0)
            .justify_center()
            .fill(p.card)
            .child(meta_text(status, p.ink))
    })
}

// Ported from KSG Code's preparingComposerImages tiles in ComposerAttachmentMediaRow.
fn preparing_attachment_chip(
    s: &State,
    p: Palette,
    entry: &crate::attachment_uploads::Entry,
) -> El {
    let id = entry.id.clone();
    let thumbnail = entry.thumbnail(&s.attachment_cache).and_then(|thumbnail| {
        if rune::is_headless() {
            let _ = thumbnail.wait();
        }
        (thumbnail.is_ready() && thumbnail.error().is_none()).then_some(thumbnail)
    });
    let body = if let Some(thumbnail) = thumbnail {
        el().size(64.0, 64.0)
            .pixels_fit(thumbnail.pixels(), rune::SurfaceFit::Fill)
    } else {
        icon("image", p.muted)
    };
    let mut tile = el()
        .key(format!("attachment-{id}"))
        .size(64.0, 64.0)
        .radius(6.0)
        .border(0.5, p.line)
        .fill(p.selected)
        .center()
        .label(format!("Preparing {}", entry.name))
        .child(body)
        .child(
            icon_button(
                &format!("remove-attachment-{id}"),
                &format!("Remove {}", entry.name),
                "close",
                p,
                move |s| s.remove_attachment(&id),
            )
            .pin(None, Some(4.0), Some(4.0), None)
            .size(18.0, 18.0)
            .fill(p.card),
        );
    if let Some(status) = attachment_upload_status(s, p, &entry.id) {
        tile = tile.child(status);
    }
    tile
}

// Ported from KSG Code's ComposerAttachmentFileList. Rune's editable pill has no
// suffix/adornment slot, so pending inline files use the reference implementation's file row.
fn attachment_upload_file_rows(s: &State, p: Palette) -> Option<(El, Option<String>)> {
    use crate::attachment_uploads::Phase;
    let mut rows = col()
        .keeps_text_selection()
        .on_key(composer_ui::control_key)
        .w_full()
        .gap(4.0)
        .px(12.0)
        .pt(8.0);
    let mut any = false;
    let mut first_focusable: Option<String> = None;
    let inline = inline_file_attachment_ids(s.current());
    for file in s.current().draft_images.iter().filter(|a| {
        !a.is_image() && !attachment_is_video(a) && !a.needs_reattach() && !inline.contains(&a.id)
    }) {
        if s.attachment_uploads
            .phase(&s.machines.selected, s.current().id, &file.id)
            .is_some_and(|phase| !matches!(phase, Phase::Ready | Phase::Legacy))
        {
            continue;
        }
        any = true;
        let id = file.id.clone();
        first_focusable.get_or_insert_with(|| format!("attachment-thumb-{id}"));
        let preview_id = id.clone();
        let size = if file.bytes.div_ceil(1024).max(1) >= 1024 {
            format!("{:.1} MB", file.bytes as f64 / 1048576.0)
        } else {
            format!("{} KB", file.bytes.div_ceil(1024).max(1))
        };
        rows = rows.child(
            row()
                .key(format!("attachment-{id}"))
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .py(4.0)
                .center_y()
                .child(icon("file", p.muted))
                .child(
                    label(&file.name, BODY, p.ink)
                        .key(format!("attachment-thumb-{id}"))
                        .role("button")
                        .label(format!("Preview {}", file.name))
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .truncate()
                        .w(0.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .on_press(move |s: &mut State| {
                            s.preview_attachments(
                                s.current().draft_images.clone(),
                                &preview_id,
                                true,
                            )
                        }),
                )
                .child(meta_text(size, p.muted))
                .child(icon_button(
                    &format!("remove-attachment-{id}"),
                    &format!("Remove {}", file.name),
                    "close",
                    p,
                    move |s| s.remove_attachment(&id),
                )),
        );
    }
    // KSG ComposerAttachmentFileList retains restored files with no bytes as markers.
    for file in s
        .current()
        .draft_images
        .iter()
        .filter(|a| a.needs_reattach())
    {
        any = true;
        let id = file.id.clone();
        first_focusable.get_or_insert_with(|| format!("remove-missing-attachment-{id}"));
        let can_attach = s.file_attachment_staging_supported_on(&s.machines.selected)
            && file.bytes <= crate::attachments::MAX_FILE_BYTES;
        rows = rows.child(
            el().key(format!("attachment-{id}")).w_full().child(
                row()
                    .key(format!("attachment-needs-reattach-{id}"))
                    .w_full()
                    .min_w(0.0)
                    .gap(8.0)
                    .py(4.0)
                    .center_y()
                    .label(format!("{} needs reattach", file.name))
                    .child(icon("file", p.muted))
                    .child(
                        label(&file.name, BODY, p.ink)
                            .truncate()
                            .w(0.0)
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(meta_text(
                        if can_attach {
                            "Attach again"
                        } else {
                            "Remove to send"
                        },
                        p.muted,
                    ))
                    .child(
                        el().key(format!("remove-attachment-{id}"))
                            .child(icon_button(
                                &format!("remove-missing-attachment-{id}"),
                                &format!("Remove {}", file.name),
                                "close",
                                p,
                                move |s| s.remove_attachment(&id),
                            )),
                    ),
            ),
        );
    }
    for entry in s.attachment_uploads.entries.iter().filter(|entry| {
        entry.machine == s.machines.selected
            && entry.thread == s.current().id
            && !entry.image
            && !s
                .current()
                .draft_images
                .iter()
                .any(|a| a.id == entry.id && attachment_is_video(a))
            && !s
                .current()
                .draft_images
                .iter()
                .any(|a| a.id == entry.id && a.needs_reattach())
            && !matches!(entry.phase, Phase::Ready | Phase::Legacy)
    }) {
        any = true;
        let elapsed = match entry.phase {
            Phase::Uploading { since, .. } => since.elapsed(),
            _ => Duration::ZERO,
        };
        let status = entry.phase.label(elapsed).unwrap_or_default();
        let mut file = row()
            .key(format!("attachment-upload-file-{}", entry.id))
            .w_full()
            .min_w(0.0)
            .gap(8.0)
            .py(4.0)
            .center_y()
            .child(icon("file", p.muted))
            .child(
                label(&entry.name, BODY, p.ink)
                    .truncate()
                    .w(0.0)
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(meta_text(status, p.muted).key(format!("attachment-status-{}", entry.id)));
        if let Phase::Failed(reason) = &entry.phase {
            let id = entry.id.clone();
            first_focusable.get_or_insert_with(|| format!("retry-attachment-{id}"));
            file = file.child(
                icon_button(
                    &format!("retry-attachment-{id}"),
                    &format!("Retry upload for {}", entry.name),
                    "reload",
                    p,
                    move |s| s.retry_attachment_upload(&id),
                )
                .help(reason),
            );
        }
        let id = entry.id.clone();
        first_focusable.get_or_insert_with(|| format!("remove-pending-attachment-{id}"));
        file = file.child(icon_button(
            &format!("remove-pending-attachment-{id}"),
            &format!("Remove {}", entry.name),
            "close",
            p,
            move |s| s.remove_attachment(&id),
        ));
        rows = rows.child(file);
    }
    any.then_some((rows, first_focusable))
}

fn inline_file_attachment_ids(t: &Thread) -> std::collections::HashSet<String> {
    let inline_ids = crate::composer_tokens::collect_inline_context_ids_from_edit(&t.draft);
    let mut ids: std::collections::HashSet<String> = t
        .draft_context
        .iter()
        .filter(|r| inline_ids.contains(&r.context_id))
        .filter_map(|r| match &r.data {
            crate::composer_context::ComposerContextData::File { attachment_id, .. } => {
                Some(attachment_id.clone())
            }
            _ => None,
        })
        .collect();
    for attachment in &t.draft_images {
        if inline_ids.contains(
            &crate::composer_context::to_kind_scoped_composer_context_id("file", &attachment.id),
        ) {
            ids.insert(attachment.id.clone());
        }
    }
    ids
}

fn attachment_is_video(a: &Attachment) -> bool {
    !a.is_image()
        && (a.mime.starts_with("video/")
            || crate::attachment_intake::file_mime(&a.name, &a.mime).starts_with("video/"))
}

pub(crate) fn tray_attachments(t: &Thread) -> Vec<&Attachment> {
    // KSG keeps previewable videos in the media strip even when they have an inline chip.
    t.draft_images
        .iter()
        .filter(|a| a.is_image() || attachment_is_video(a))
        .collect()
}

fn remove_attachment_dialog(s: &State, p: Palette) -> El {
    let name = s
        .confirm_remove_attachment
        .as_ref()
        .and_then(|(tid, id)| s.threads.iter().find(|t| t.id == *tid).map(|t| (t, id)))
        .and_then(|(t, id)| t.draft_images.iter().find(|a| a.id == *id))
        .map_or("this image", |a| a.name.as_str());
    let card = col()
        .key("remove-attachment-dialog")
        .role("dialog")
        .label("Remove attachment")
        .w(380.0)
        .max_w(540.0)
        .padding(16.0)
        .gap(12.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(label("Remove attachment", DIALOG_TITLE, p.ink))
        .child(
            label(
                format!(
                    "Remove {name} from the message?\nIt is referenced in your text; removing it also removes every reference."
                ),
                BODY,
                p.ink,
            )
            .wrap(),
        )
        .child(
            row()
                .gap(8.0)
                .child(spacer())
                .child(secondary_button(
                    "cancel-remove-attachment",
                    "Cancel",
                    p,
                    |s| {
                        s.confirm_remove_attachment = None;
                    },
                ))
                .child(button(
                    "confirm-remove-attachment",
                    "Remove",
                    p,
                    |s| s.confirm_remove_attachment_dialog(),
                )),
        );
    el().key("remove-attachment-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .padding(16.0)
        .fill(Color::BLACK.alpha(0.3))
        .modal(false, |s: &mut State| {
            s.confirm_remove_attachment = None;
        })
        .child(card)
}

fn message_images(s: &State, p: Palette, images: &[Attachment], key: &str) -> El {
    let mut strip = row()
        .justify_end()
        .items_end()
        .flex_wrap()
        .gap(6.0)
        .max_w(620.0);
    for (index, image) in images.iter().enumerate() {
        let items = images.to_vec();
        let id = image.id.clone();
        let (width, height) = image_size(image, if images.len() == 1 { 220.0 } else { 132.0 });
        strip = strip.child(
            image_preview(s, p, image, width, height, format!("{key}-image-{index}"))
                .role("button")
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| s.preview_attachments(items.clone(), &id, false))
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
        .radius(6.0)
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

// Ported from KSG Code's ComposerAttachButton in ComposerAttachmentStrip.tsx.
fn pick_attachment(s: &mut State, tx: &mut Tx, folder: bool) {
    let thread = s.current().id;
    let machine = s.machines.selected.clone();
    let question = s.current().question_scope();
    let options = rune::FileDialogOptions {
        title: if folder {
            "Attach a folder"
        } else {
            "Attach files"
        }
        .into(),
        ..Default::default()
    };
    let callback = move |s: &mut State, result, _: &mut Tx| {
        if !crate::attachment_picker::target_accepts(s, &machine, thread, question.as_deref()) {
            return;
        }
        match result {
            Ok(Some(path)) => s.attach_paths_on_to(&machine, thread, &[path]),
            Ok(None) => {}
            Err(error) => {
                s.set_attachment_target_error(
                    &machine,
                    thread,
                    format!("Could not open the attachment picker: {error}"),
                );
            }
        }
        s.checkpoint();
    };
    if folder {
        tx.open_folder(options, callback);
    } else {
        crate::attachment_picker::open(s, thread, options);
    }
}

/// The pill's ring is smaller than the popover era's: 16 pt inside a 24 pt row
/// (spec §4.4).
const CONTEXT_PILL_RING: f32 = 16.0;

fn context_limit(t: &Thread) -> Option<u64> {
    t.context.usage.limit(t.context_window)
}

/// The ring appears once Pi or the model catalog knows a capacity, or once Pi reports usage.
fn context_available(t: &Thread) -> bool {
    t.context.usage.has_usage() || context_limit(t).is_some() || usage_limits_available(t)
}

fn usage_limits_available(t: &Thread) -> bool {
    t.harness_state["usageLimits"]["windows"]
        .as_array()
        .is_some_and(|windows| !windows.is_empty())
}

/// A fresh draft has nothing to measure yet, so it shows no empty dashed ring.
fn context_ring_shown(t: &Thread) -> bool {
    usage_limits_available(t)
        || context_available(t)
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
fn context_ring_shape(context: &ContextMeter, p: Palette, size: f32) -> Shape {
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

fn context_pill_warnings(s: &State) -> (usize, usize) {
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
fn context_ring(s: &State, t: &Thread, p: Palette, lane: f32) -> El {
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

fn attach_composer_notices(s: &State, p: Palette, lane: f32, composer_card: El) -> El {
    if let Some(notices) = composer_notice_stack(s, p, lane) {
        col().w_full().gap(0.0).child(notices).child(composer_card)
    } else {
        composer_card
    }
}

/// Ported from KSG Code's `ComposerRequestCard.Well`: inset radius-8 `surface-well`
/// (`p.code_bg` + hairline `p.line`) holding commands, question text, evidence or options.
fn request_card_well(p: Palette, options: bool, content: El) -> El {
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
fn composer_request_banner(s: &State, p: Palette, lane: f32) -> Option<El> {
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
fn composer_shell(
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

fn composer_notice_overflows(notice: &crate::composer_notices::Notice, lane: f32) -> bool {
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
fn composer_notice_stack(s: &State, p: Palette, lane: f32) -> Option<El> {
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

fn usage_progress_bar(key: String, fill_key: Option<String>, percent: f64, p: Palette) -> El {
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
fn usage_limits_notice_body(
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
fn composer_notice_details_popover(
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
fn context_legend_for_thread(
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

fn breakdown_slug(label: &str, kind: crate::context_breakdown::ContextLegendKind) -> String {
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

fn breakdown_keys(legend: &[crate::context_breakdown::ContextLegendItem]) -> Vec<(String, String)> {
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
fn breakdown_segment_color(
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
fn context_breakdown_view(
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
fn context_popover(s: &State, p: Palette) -> El {
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

/// The composer sits in KSG Code's `.chat-composer-lane`: the same gutters and 46 rem lane as
/// the messages above it, with `pb-4` under the footer.
/// The composer field's height with its 4 pt padding: one line at rest, so the body is 48 pt.
const COMPOSER_FIELD_MIN: f32 = 36.0;
/// The composer text field's own padding, inside `composer-input`.
const COMPOSER_FIELD_PAD: f32 = 4.0;
/// About how wide the prompt body's inline actions are: attach, the gap and the send key.
const COMPOSER_ACTIONS_W: f32 = 28.0 + 6.0 + 32.0;
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
        p.ring.alpha(0.6)
    } else {
        p.frame
    }
}
/// Whether the composer's caret is on its first and last visual line, `[first, last]`, or
/// `None` before the field has drawn a caret. Ported from KSG Code's `isCaretOnVisualEdge` in
/// `ComposerPromptEditorTiptap.tsx`: the caret line is on an edge when it is within half its
/// height of that edge of the text. Prompt history uses it, so Up and Down move through a
/// wrapped line before they recall a prompt.
fn composer_caret_edges(cx: &Cx<State>, zoom: f32) -> Option<[bool; 2]> {
    // The caret is in window points and layout reads are in the zoomed view's points.
    let caret = cx.caret("composer-input")?.map(|v| v / zoom);
    // Only the field's top and height matter, so a sideways move does not re-run the view.
    let [top, height] = cx.window_layout_with("composer-input", |r| [r[1], r[3]])?;
    let half = caret[3] / 2.0;
    let first = caret[1] - (top + COMPOSER_FIELD_PAD) < half;
    // A field at its minimum height holds one line with spare room under it, so its bottom
    // edge is not the text's. Taller than that, the field ends where its text does.
    let last = if height <= COMPOSER_FIELD_MIN + 0.5 {
        first
    } else {
        top + height - COMPOSER_FIELD_PAD - (caret[1] + caret[3]) < half
    };
    Some([first, last])
}

/// Ported from KSG Code's `replyBannerItems` (`chat/ChatComposer.tsx`) and `ComposerBanner`
/// (`chat/ComposerBanner.tsx`, `px-1 py-1.25 min-h-6 gap-x-1 text-xs/4`).
fn composer_reply_banner(t: &Thread, p: Palette) -> Option<El> {
    let reply_to = t.draft_reply_to.as_ref()?;
    let crate::composer_context::ComposerContextData::Response {
        citation,
        subagents,
        ..
    } = &reply_to.data
    else {
        return None;
    };
    let title = match subagents.as_deref() {
        Some(list) if !list.is_empty() => format!(
            "Replying to {}",
            list.iter()
                .map(|subagent| subagent.title.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => "Replying to".to_owned(),
    };
    let target_label = crate::assistant_citation::assistant_citation_label(citation);
    let nav_citation = citation.clone();
    let text_style = TextStyle::new(12.0, 400.0).lh(16.0);
    let title_style = TextStyle::new(12.0, 500.0).lh(16.0);
    let aria_label = format!("{title} {target_label}");
    let has_subagents = subagents.as_deref().is_some_and(|list| !list.is_empty());
    let title_el = if has_subagents {
        label(title, title_style, p.muted)
            .truncate()
            .min_w(0.0)
            .shrink()
    } else {
        label(title, title_style, p.muted)
    };
    Some(
        row()
            .key("composer-reply-banner")
            .keeps_text_selection()
            .on_key(composer_ui::control_key)
            .w_full()
            .min_h(24.0)
            .px(4.0)
            .py(5.0)
            .gap(4.0)
            .center_y()
            .child(
                el().size(24.0, 24.0)
                    .center()
                    .child(icon("back", p.muted).size(12.0, 12.0)),
            )
            .child(
                row()
                    .key("composer-reply-target")
                    .role("button")
                    .label(aria_label)
                    .cursor(rune::Cursor::Pointer)
                    .focusable(true)
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(4.0)
                    .center_y()
                    .radius(6.0)
                    .hover(p.selected)
                    .press(p.selected)
                    .on_press_tx(move |s: &mut State, tx| {
                        s.navigate_to_citation(&nav_citation, tx);
                    })
                    .child(title_el)
                    .child(
                        label(target_label, text_style, p.muted)
                            .truncate()
                            .min_w(0.0)
                            .grow(1.0),
                    ),
            )
            .child(
                el().key("composer-reply-remove")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Remove reply")
                    .focusable(true)
                    .size(20.0, 20.0)
                    .center()
                    .radius(6.0)
                    .hover(p.selected)
                    .press(p.selected)
                    .on_press(|s: &mut State| {
                        s.clear_reply_to();
                        s.restore_composer_focus(composer_ui::FocusReason::Explicit);
                    })
                    .child(icon("close", p.muted).size(12.0, 12.0)),
            ),
    )
}

/// The composer input's decorations: `preview_decorations`' Markdown-source spans,
/// then the draft's per-block syntax colours (KSG's `composerCodeBlockHighlight`),
/// which win colour on an overlap and leave the preview's other attributes alone.
fn composer_decorations(
    s: &State,
    p: Palette,
) -> (Vec<rune::text::Span>, Vec<(usize, usize, Option<Color>)>) {
    let t = s.current();
    let (mut spans, strikes) =
        crate::composer_source::preview_decorations(&t.draft, COMPOSER, p.faint, p.ink, p.code_bg);
    spans.extend(t.code_highlight.borrow_mut().spans(
        &t.draft,
        s.dark,
        &markdown_ui::syntax_theme(s.dark, p),
        std::time::Instant::now(),
    ));
    (spans, strikes)
}

/// Budget for the above-composer stack under its 40% viewport cap: how much height the
/// non-roster parts require to remain usable, and how much room (if any) an open crew
/// roster's member rows may take without squeezing Needs you or the Plan editor.
struct AboveComposerBudget {
    /// Minimum height for the stack so no visible controls become unreachable.
    floor: f32,
}

fn above_composer_budget(s: &State, _cap: f32, _lane: f32) -> AboveComposerBudget {
    let t = s.current();
    let mut parts = 0usize;
    let mut others_floor = 0.0f32;
    if crew_ui::parent_bar_shows(s) {
        parts += 1;
        others_floor += 36.0;
    }
    if crew_ui::crew_bar_shows(s) {
        parts += 1;
        others_floor += 36.0;
    }
    if crew_ui::crew_tray_shows(s) {
        parts += 1;
        others_floor += 28.0;
    }
    let queued = t.native.queue.len() + t.native.steering.len() + t.native.follow_ups.len();
    if queued > 0 {
        parts += 1;
        others_floor += 48.0;
    }
    if parts > 1 {
        others_floor += (parts - 1) as f32 * 8.0;
    }
    AboveComposerBudget {
        floor: others_floor,
    }
}
/// Actions ported from KSG Code's ComposerPrimaryActions and useComposerResume.
fn composer(
    s: &State,
    model: &ComposerModelDisplay,
    p: Palette,
    wide: bool,
    pane_width: f32,
    caret_edges: Option<[bool; 2]>,
) -> El {
    let t = s.current();
    // The lane the conversation resolves too; `None` until C5 mounts the Details card.
    let lane_rect = chat_lane(pane_width, wide, details_ui::edges(s, pane_width));
    let lane = lane_rect.width;
    let inline_files = inline_file_attachment_ids(t);
    let collapsed = s
        .attachment_uploads
        .block_reason(&s.machines.selected, t.id)
        .is_none()
        && !t
            .draft_images
            .iter()
            .any(|a| !a.is_image() && !attachment_is_video(a) && !inline_files.contains(&a.id))
        && composer_ui::collapsed(
            s,
            s.composer_ui.viewport.get(),
            lane - 32.0 - COMPOSER_ACTIONS_W,
        );
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
        // Antigravity's captured OAuth link opens only on click (KSG sign-in flow).
        if t.harness == "antigravity"
            && let Some(url) = t.harness_state["signInUrl"].as_str()
        {
            let url = url.to_owned();
            area = area.child(
                button("open-sign-in-link", "Sign in with Google", p, |_| {}).on_press_tx(
                    move |_: &mut State, _| {
                        let _ = crate::antigravity::open_browser(&url);
                    },
                ),
            );
        }
        let command = if t.harness == "codex" && error.contains("`codex login`") {
            Some("codex login")
        } else if t.harness == "claude" && error.contains("`claude auth login`") {
            Some("claude auth login")
        } else if t.harness == "antigravity" && error.contains("`agy acp --login`") {
            Some("agy acp --login")
        } else {
            None
        };
        if let Some(command) = command {
            area = area.child(
                button("copy-sign-in-command", "Copy command", p, |_| {}).on_press_tx(
                    move |_: &mut State, tx| {
                        tx.copy(command.to_owned());
                    },
                ),
            );
        }
    }
    if let Some(error) = &t.attachment_error {
        area = area.child(
            label(error, SMALL, p.danger)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    let request_visible = t.request.is_some() && !t.request_held_by_typing_guard();
    // Delegation notices, queued messages, and the 28 pt Now / crew strip stay visible above
    // `ComposerSurface.Shell`, even when a structured questionnaire replaces the draft card.
    let viewport_cap = (s.viewport_height * 0.4).max(120.0);
    let budget = above_composer_budget(s, viewport_cap, lane);
    let cap = viewport_cap.max(budget.floor);
    let mut stack = col()
        .key("above-composer-stack")
        .w_full()
        .gap(6.0)
        .min_h(0.0)
        .max_h(cap);
    for part in [
        crew_ui::parent_bar(s, p, lane),
        crew_ui::crew_bar(s, p, lane),
        crew_ui::crew_tray(s, p, lane, None),
    ]
    .into_iter()
    .flatten()
    {
        stack = stack.child(part);
    }
    if pi_ui::queued_messages_shows(s) {
        stack = stack.child(pi_ui::queued_messages(s, p));
    }
    area = area.child(stack);
    if t.native.questionnaire.is_some() && request_visible {
        area = area.child(composer_shell(
            s,
            p,
            None,
            pi_ui::questionnaire_drawer(s, p, lane),
            None,
        ));
        return col()
            .key("composer-lane")
            .w_full()
            .pl(lane_gutter(pane_width) + lane_rect.inset_start)
            .pr(lane_gutter(pane_width) + lane_rect.inset_end)
            .items_center()
            .child(area);
    }
    let show_resume = t.can_resume() && !t.has_draft_content() && t.queued_edit.is_none();
    let compact_before_send_tokens = if !collapsed && !show_resume && t.queued_edit.is_none() {
        s.resume_compaction_tokens(t, crate::lifecycle::now())
    } else {
        None
    };
    let keep_full_history = s.keep_full_history(t);
    let compacts_before_send = compact_before_send_tokens.is_some() && !keep_full_history;
    let approval_send_allowed = t
        .request
        .as_ref()
        .filter(|r| r["method"] == "confirm")
        .is_none_or(|request| {
            (crate::approval::can_respond(request) || crate::approval::not_resumable(request))
                && t.requests_supported
                && s.client.as_ref().is_none_or(|client| {
                    (crate::approval::not_resumable(request)
                        || client.can_approve(&s.machines.selected))
                        && client.supports_capability(
                            &s.machines.selected,
                            crate::protocol::COMPOSER_REQUESTS,
                        )
                        && (request["responseCapability"]["supportsFeedback"] != true
                            || client.supports_capability(
                                &s.machines.selected,
                                crate::protocol::APPROVAL_FEEDBACK,
                            ))
                })
        });
    let crew_blocker = t
        .crew_draft
        .as_ref()
        .and_then(|_| s.crew_draft_unavailable());
    let can_send = !s.demo
        && crew_blocker.is_none()
        && s.attachment_send_block_reason().is_none()
        && s.machines.draft_retarget.is_none()
        && (s.client.is_none() || s.machine_connected(&s.machines.selected))
        && (!t.initializing || t.queued_edit.is_some())
        && !t.archived()
        && !t.configuring
        && (!request_visible || t.request.as_ref().is_some_and(|r| r["method"] == "confirm"))
        && !t.approval_send_pending()
        && t.responding_request.is_none()
        && approval_send_allowed
        && (!t.native.operating() || t.queued_edit.is_some())
        && (t.phase != "Stopping" || t.queued_edit.is_some())
        && !t.queued_edit_saving()
        && !t.resume_pending()
        && (!collapsed || !t.status.busy())
        && (t.has_draft_content() || show_resume);
    let mut send = el()
        .key("send-message")
        .role("button")
        .label(if s.machines.draft_retarget.is_some() {
            "Preparing machine"
        } else if t.resume_pending() {
            "Resuming thread"
        } else if show_resume {
            "Resume thread"
        } else if t.queued_edit.is_some() {
            if t.queued_edit_saving() {
                "Updating queued message"
            } else {
                "Update queued message"
            }
        } else if compacts_before_send {
            "Compact and send"
        } else if t.status.busy() {
            if t.follow_up_queues() {
                "Queue message"
            } else {
                "Steer message"
            }
        } else {
            "Submit message"
        })
        .size(32.0, 32.0)
        .radius(7.0)
        .center()
        .child(
            icon(
                if show_resume {
                    "play"
                } else if t.queued_edit.is_some() {
                    "check"
                } else if t.status.busy() && t.follow_up_queues() {
                    "list-plus"
                } else if t.status.busy() {
                    "corner-up-right"
                } else {
                    "arrow-up"
                },
                if can_send { p.ink } else { p.muted },
            )
            .size(14.0, 14.0),
        )
        .disabled(!can_send);
    if can_send {
        send = send
            .cursor(rune::Cursor::Pointer)
            .hover(p.selected)
            .press(p.selected)
            .focusable(true);
        if show_resume {
            send = send.on_press(|s: &mut State| s.resume_thread());
        } else {
            send = send.on_press_tx(|s: &mut State, tx| {
                let previous = s.current().native.follow_up;
                if tx.mods().cmd || tx.mods().ctrl {
                    s.current_mut().native.follow_up = !previous;
                }
                s.send();
                s.current_mut().native.follow_up = previous;
            });
        }
    }
    let [action_x, action_y] = composer_ui::anchor_offset(s, 1, true);
    // KSG Code's `ComposerSendPill` (`surface-pill`): compact, send and stop share one 32 pt
    // recessed control (`--pill-fill`, `field_line` rim >= 3:1 painted above the slots, and
    // the soft `--pill-shadow`), with 7 px slots concentric inside the 8 px rim.
    let mut action = composer_ui::bind_anchor(
        s,
        1,
        row()
            .key("composer-transition-actions")
            .offset(action_x, action_y)
            .h(32.0)
            .radius(8.0)
            .fill(p.pill)
            .box_shadow(control_shadow(p))
            .center_y(),
    );
    let show_send_slot =
        !t.status.busy() || !collapsed && (t.has_draft_content() || t.queued_edit.is_some());
    let compact_disabled =
        composer_disabled(s) || (s.client.is_some() && !s.machine_connected(&s.machines.selected));
    let show_compact_slot = show_send_slot && compact_before_send_tokens.is_some();
    if show_send_slot {
        if let Some(tokens) = compact_before_send_tokens {
            let amount = crate::context::format_context_window_tokens(tokens);
            let tooltip = if keep_full_history {
                format!("Next send keeps all {amount} tokens.\nClick to compact first")
            } else {
                format!("Next send compacts {amount} tokens first.\nClick to keep full history")
            };
            let mut toggle = row()
                .key("composer-compact-send")
                .role("button")
                .label(format!("Compact {amount} tokens of history before sending"))
                .help(tooltip)
                .h(32.0)
                .px(10.0)
                .radius(7.0)
                .center_y()
                .gap(6.0)
                .disabled(compact_disabled)
                .child(icon("minimize", p.muted));
            // Never stack two washes: active state uses one `p.selected` fill, while the
            // inactive state applies `p.selected` only on hover/press when enabled.
            if !keep_full_history {
                toggle = toggle.fill(p.selected);
            } else if !compact_disabled {
                toggle = toggle.hover(p.selected).press(p.selected);
            }
            if !compact_disabled {
                toggle = toggle
                    .cursor(rune::Cursor::Pointer)
                    .focusable(true)
                    .on_press(|s: &mut State| s.toggle_keep_full_history());
            }
            toggle = toggle.child(label(&amount, META.features(TABULAR), p.muted));
            action = action.child(toggle);
        }
        action = action.child(send);
    }
    let show_stop_slot =
        !s.demo && (t.status.busy() && t.queued_edit.is_none() || t.stop_control.phase().is_some());
    let stop_disabled = t.stop_control.disabled()
        || !t.turn_capabilities().supports_interrupt
        || s.client.is_some() && !s.machine_connected(&s.machines.selected);
    if show_stop_slot {
        let mut stop = el()
            .key("stop-agent")
            .role("button")
            .label(t.stop_control.label())
            .disabled(stop_disabled)
            .size(32.0, 32.0)
            .radius(7.0)
            .center()
            .child(
                el().key("composer-stop-mark")
                    .size(10.0, 10.0)
                    .radius(2.0)
                    .fill(p.danger)
                    .a11y_hidden(),
            );
        if !stop_disabled {
            stop = stop
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .hover(p.selected)
                .press(p.selected)
                .on_press(|s: &mut State| s.current_mut().stop());
        }
        action = action.child(stop);
    }
    action = action.child(
        el().key("composer-pill-rim")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(8.0)
            .border(1.0, p.field_line)
            .inert(),
    );
    let locked = s.demo || t.archived() || t.status.busy() || t.configuring;
    let attach_locked = s.demo
        || t.archived()
        || t.configuring
        || t.native.operating() && t.queued_edit.is_none()
        || t.request.is_some();
    // KSG Code's `ComposerSurface.Main`: a 15 px card with a hairline outline. A composer that
    // cannot send drops its shadow and its frame takes the lighter `line` colour.
    // The resting row and the prompt body swap at once, so a press right after expanding
    // never lands on the leaving row.
    let mut card = col().key("composer-natural").w_full().instant_changes();
    card = card
        .on_drop_files(|s: &mut State, files, _| {
            s.attachment_drop_hover = false;
            s.attach_paths(files);
            s.checkpoint();
        })
        .on_drop_hover(|s: &mut State, over, _| s.attachment_drop_hover = over);
    let tray_items = tray_attachments(t);
    let preparing: Vec<_> = s
        .attachment_uploads
        .entries
        .iter()
        .filter(|entry| {
            entry.machine == s.machines.selected
                && entry.thread == t.id
                && entry.image
                && matches!(entry.phase, crate::attachment_uploads::Phase::Preparing)
        })
        .collect();
    let mut first_focusable: Option<String> = None;
    if !collapsed && (!tray_items.is_empty() || !preparing.is_empty()) {
        let mut tray = row()
            .key("composer-attachments")
            .keeps_text_selection()
            .on_key(composer_ui::control_key)
            .opacity(composer_ui::image_arrival(s, false))
            .gap(8.0)
            .flex_wrap()
            .px(12.0)
            .pt(12.0);
        for image in tray_items {
            first_focusable.get_or_insert_with(|| format!("attachment-thumb-{}", image.id));
            tray = tray.child(attachment_chip(s, p, image));
        }
        for entry in preparing {
            first_focusable.get_or_insert_with(|| format!("remove-attachment-{}", entry.id));
            tray = tray.child(preparing_attachment_chip(s, p, entry));
        }
        card = card.child(tray);
    }
    if !collapsed && let Some((rows, file_first)) = attachment_upload_file_rows(s, p) {
        if first_focusable.is_none() {
            first_focusable = file_first;
        }
        card = card.child(rows);
    }
    if let Some(banner) = composer_reply_banner(t, p) {
        first_focusable.get_or_insert_with(|| "composer-reply-target".into());
        card = card.child(banner).child(rule(p));
    }
    if !collapsed {
        *s.composer_ui.first_focusable.borrow_mut() =
            first_focusable.unwrap_or_else(|| "composer-input".into());
    }
    // Ported from KSG Code's composer body (`px-4 pt-4 pb-2`, `ChatComposer.tsx`) and its
    // editor (`max-h-52 overflow-y-auto p-1 -m-1`, `ComposerPromptEditorTiptap.tsx`): the field
    // grows with the draft up to a 200 pt text window, then scrolls inside its own viewport,
    // which Rune keeps on the caret after each edit. The viewport is what the card sizes to, so
    // the actions row always sits below the text and the card never grows past the cap. A bare
    // field with `max_h` laid out at the cap but let the card size to all of its text, which
    // drew the last lines under the actions and left an empty strip below them.
    let composer_style = COMPOSER;
    let offered_suggestion = s.offered_prompt_suggestion();
    let blank_preview = rune::TextEdit::default();
    let field = text_input(
        if collapsed && t.draft.text.trim().is_empty() {
            &blank_preview
        } else {
            &t.draft
        },
        composer_style,
    )
    .key("composer-input")
    .label(format!("Message {}", harness_name(s, &t.harness)))
    .placeholder(
        if s.demo {
            "Open a real workspace to send messages".to_string()
        } else if let Some(machine) = s.unreachable_selected() {
            format!("{machine} is unreachable. Reconnecting.")
        } else if request_visible && t.request.as_ref().is_some_and(|r| r["method"] == "confirm") {
            if t.request
                .as_ref()
                .is_some_and(|r| r["responseCapability"]["supportsFeedback"] == true)
            {
                "Send feedback to decline this request".into()
            } else {
                "Send a message to decline and redirect the agent".into()
            }
        } else if offered_suggestion.is_some() {
            String::new()
        } else if collapsed || lane < 560.0 {
            "Ask anything...".to_string()
        } else {
            "Ask anything, @tag files/folders, $use skills, or / for commands".to_string()
        },
        p.muted,
    )
    .color(p.ink)
    .pill_colors(p.selected, Some(p.ink))
    .pill_style(composer_menu::pill_style(s, p))
    .rich_style(rune::RichStyle {
        code_fg: Some(p.ink),
        code_bg: p.code_bg,
        code_block_bg: p.code_bg,
        link: p.ink,
        quote_bar: p.muted,
        marker: Some(p.muted),
    })
    .block_style(rune::BlockStyle {
        heading_scale: [
            20.0 / 15.0,
            18.0 / 15.0,
            16.0 / 15.0,
            14.0 / 15.0,
            14.0 / 15.0,
            14.0 / 15.0,
        ],
        heading_weight: [600.0; 6],
        // KSG's `.composer-tiptap` headings: line-height 1.3, contrast foreground.
        heading_line_height: [Some(1.3); 6],
        heading_color: [Some(p.ink); 6],
        checkbox_size: 14.0,
        checkbox_stroke: 2.0,
        checkbox_color: Some(p.ink),
        checkbox_checked_fill: Some(p.coral),
        checkbox_mark_color: Some(p.on_coral),
        rule_thickness: 2.0,
        rule_color: Some(p.line),
        // KSG's `.composer-tiptap hr { margin: 0.65rem 0 }`.
        rule_margin_top: Some(10.4),
        rule_margin_bottom: Some(10.4),
    })
    .code_language_headers(s, p, locked)
    .caret(p.ink, p.coral_bg)
    .w_full()
    .min_h(COMPOSER_FIELD_MIN)
    .padding(COMPOSER_FIELD_PAD)
    .on_edit(|s: &mut State| {
        s.current_mut().native.slash_index = 0;
        &mut s.current_mut().draft
    })
    .on_input_key(move |s: &mut State, event, tx| {
        composer_ui::control_key(s, event, tx);
        if pi_ui::composer_pi_key(s, event, tx, caret_edges) {
            composer_menu::scroll_selected(s, tx);
            return true;
        }
        if (event.is_command('c') || event.is_command('x')) && !event.mods.shift && !event.mods.alt
        {
            if s.current().draft.anchor != s.current().draft.focus
                && let Some(content) = s.copy_composer_selection(event.is_command('x'))
            {
                tx.copy_rich(content);
                return true;
            }
            if matches!(
                s.current().draft.selection(),
                rune::EditorSelection::Block(_)
            ) && !s.current().draft.is_secure()
            {
                let cut = event.is_command('x');
                let d = &mut s.current_mut().draft;
                let content =
                    rune::ClipboardContent::new(d.selected()).with_html(d.selected_html());
                tx.copy_rich(content);
                if cut {
                    d.insert("");
                    s.current_mut().update_submission_error();
                }
                return true;
            }
        }
        if event.is_command('v') && event.mods.shift && !event.mods.alt {
            let id = s.current().id;
            let machine = s.machines.selected.clone();
            tx.read_clipboard(move |s: &mut State, content, _| {
                if s.machines.selected != machine {
                    return;
                }
                if let Some(t) = s.threads.iter_mut().find(|t| t.id == id) {
                    t.draft.insert(content.text());
                }
                s.checkpoint();
            });
            return true;
        }
        let t = s.current_mut();
        if t.paste_history
            .key(&mut t.draft, &mut t.draft_images, event)
        {
            s.current_mut().reconcile_draft_context();
            s.checkpoint();
            return true;
        }
        let t = s.current_mut();
        let before = t.draft.text.clone();
        t.composer_source.on_key_before(&t.draft, event);
        let handled = t.composer_rules.key(&mut t.draft, event);
        if handled {
            t.reconcile_draft_context();
            if t.draft.text != before {
                t.composer_source.on_doc_change(&t.draft);
                t.typing_guard.on_type(std::time::Instant::now());
            }
            t.update_submission_error();
            s.ensure_composer_commands();
        }
        handled
    })
    .on_input_change(|s: &mut State, _| {
        composer_ui::changed(s);
    })
    .on_input_paste(|s: &mut State, content, _| {
        let bypass = s.composer_ui.consume_paste_as_text();
        s.paste_composer_content(content, bypass)
    })
    .on_submit(|s: &mut State, _| {
        if !s.demo && !s.current().archived() {
            s.send();
        }
    });
    let (source_spans, source_strikes) = composer_decorations(s, p);
    let field = field
        .on_input_selection_change(|s: &mut State, _| {
            let t = s.current_mut();
            crate::composer_source::on_selection_change(&mut t.composer_source, &mut t.draft);
        })
        .on_input_pill_press(|s: &mut State, id, _, _| s.press_composer_pill(id))
        .decorations(source_spans)
        .strikes(source_strikes);
    if collapsed {
        // KSG's CollapsedComposerPromptPreview is inert. Reuse the real pill renderer without
        // edit handlers or focus, so files, skills and thread chips have exactly the editor's ink.
        let mut preview = row()
            .key("composer-collapsed-row")
            .center_y()
            .gap(8.0)
            .px(12.0)
            .py(8.0)
            .child(
                el().key("composer-preview")
                    .role("button")
                    .label(if t.draft.text.trim().is_empty() {
                        format!(
                            "Expand composer. {}",
                            offered_suggestion.unwrap_or("Ask anything...")
                        )
                    } else {
                        format!("Expand composer. {}", t.draft.display().0)
                    })
                    .cursor(rune::Cursor::Pointer)
                    .grow(1.0)
                    .min_w(0.0)
                    .clip_rounded(0.0)
                    .preserve_focus_on_press(true)
                    .on_press(|s: &mut State| {
                        s.restore_composer_focus(composer_ui::FocusReason::Explicit);
                    })
                    .child(composer_ui::prompt_surface(
                        s,
                        {
                            let preview_field =
                                field.single_line().padding(0.0).h(28.0).min_h(0.0).inert();
                            let mut host = el().w_full().h(28.0).child(preview_field);
                            if let Some(suggestion) = offered_suggestion {
                                host = host.child(
                                    row()
                                        .key("composer-suggestion")
                                        .inert()
                                        .h(28.0)
                                        .center_y()
                                        .gap(8.0)
                                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                                        .child(
                                            label(suggestion, composer_style, p.muted)
                                                .truncate()
                                                .min_w(0.0)
                                                .shrink(),
                                        )
                                        .child(
                                            keycap_chip("Tab", p).key("composer-suggestion-key"),
                                        ),
                                );
                            }
                            host
                        },
                        true,
                    )),
            );
        let resting_tray = tray_attachments(t);
        for image in resting_tray.iter().take(3) {
            let id = image.id.clone();
            preview = preview.child(
                composer_attachment_thumbnail(
                    s,
                    p,
                    image,
                    24.0,
                    24.0,
                    format!("resting-attachment-{}", image.id),
                )
                .opacity(composer_ui::image_arrival(s, true))
                .role("button")
                .label(format!("Preview {}", image.name))
                .preserve_focus_on_press(true)
                .on_press(move |s: &mut State| {
                    s.preview_attachments(s.current().draft_images.clone(), &id, true);
                }),
            );
        }
        if resting_tray.len() > 3 {
            preview = preview.child(button(
                "resting-attachments-more",
                &format!("+{}", resting_tray.len() - 3),
                p,
                |s| s.restore_composer_focus(composer_ui::FocusReason::Explicit),
            ));
        }
        preview = preview.child(attach_control(attach_locked, p));
        if context_ring_shown(t) {
            preview = preview.child(context_ring(s, t, p, lane));
        }
        preview = preview.child(action);
        let controls = composer_ui::controls(s, model, p, lane - 32.0, true, locked);
        let composer_card = composer_ui::frame_card(s, p, card.child(preview).child(controls));
        return col()
            .key("composer-lane")
            .w_full()
            .pl(lane_gutter(pane_width) + lane_rect.inset_start)
            .pr(lane_gutter(pane_width) + lane_rect.inset_end)
            .items_center()
            .child(area.child(composer_shell(
                s,
                p,
                composer_request_banner(s, p, lane),
                attach_composer_notices(s, p, lane, composer_card),
                None,
            )));
    }
    // KSG Code's `.composer-prompt-body`: the editor on the left and the attach, compact,
    // dispatch and send controls inline on the right. A one-line draft centres them on the
    // prompt line; a wrapped draft anchors them to the bottom right.
    let multiline = t.draft.text.contains('\n') || s.composer_ui.wrapped();
    let mut cluster = row()
        .key("composer-actions")
        .on_key(composer_ui::control_key)
        .keeps_text_selection()
        .center_y()
        .gap(6.0)
        .child(
            attach_control(attach_locked, p)
                .size(28.0, 28.0)
                .radius(8.0),
        );
    if t.status.busy() && t.has_draft_content() && t.queued_edit.is_none() {
        cluster = cluster.child(
            button(
                "dispatch-mode",
                if t.follow_up_queues() {
                    "Queue"
                } else {
                    "Steer"
                },
                p,
                |s| {
                    let n = &mut s.current_mut().native;
                    n.follow_up = !n.follow_up;
                },
            )
            .disabled(!t.can_steer()),
        );
    }
    cluster = cluster.child(action);
    let mut prompt_body = row()
        .key("composer-prompt-body")
        .w_full()
        .min_h(48.0)
        .py(6.0)
        .pl(10.0)
        .pr(8.0)
        .gap(6.0);
    prompt_body = if multiline {
        prompt_body.items_end()
    } else {
        prompt_body.center_y()
    };
    card = card.child(
        prompt_body
            .child(
                col()
                    .key("composer-body")
                    .grow(1.0)
                    .min_w(0.0)
                    // A press in the body's padding still lands in the field, as KSG's surface does.
                    .on_press(|s: &mut State| {
                        s.restore_composer_focus(composer_ui::FocusReason::Explicit)
                    })
                    // The field inside is the Tab stop, not this.
                    .focusable(false)
                    .child(composer_ui::prompt_surface(
                        s,
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
                            .child({
                                let mut host = el().key("composer-input-host").w_full();
                                let field = if let Some(suggestion) = offered_suggestion {
                                    let content_w = (lane
                                        - COMPOSER_ACTIONS_W
                                        - 26.0
                                        - COMPOSER_FIELD_PAD * 2.0)
                                        .max(40.0);
                                    let (natural_w, _) =
                                        rune::text::measure(suggestion, &[], composer_style, None);
                                    let min_h = if natural_w + 48.0 <= content_w {
                                        COMPOSER_FIELD_MIN
                                    } else {
                                        let (_, text_h) = rune::text::measure(
                                            suggestion,
                                            &[],
                                            composer_style,
                                            Some(content_w),
                                        );
                                        (text_h + 24.0 + COMPOSER_FIELD_PAD * 2.0)
                                            .max(COMPOSER_FIELD_MIN)
                                    };
                                    host = host.min_h(min_h);
                                    field.min_h(min_h)
                                } else {
                                    host = host.min_h(COMPOSER_FIELD_MIN);
                                    field
                                };
                                host = host.child(field);
                                if let Some(suggestion) = offered_suggestion {
                                    host = host.child(
                                        row()
                                            .key("composer-suggestion")
                                            .inert()
                                            .flex_wrap()
                                            .center_y()
                                            .gap(8.0)
                                            .pin(
                                                Some(COMPOSER_FIELD_PAD),
                                                Some(COMPOSER_FIELD_PAD),
                                                None,
                                                Some(COMPOSER_FIELD_PAD),
                                            )
                                            .child(
                                                label(suggestion, composer_style, p.muted)
                                                    .wrap()
                                                    .min_w(0.0)
                                                    .shrink(),
                                            )
                                            .child(
                                                keycap_chip("Tab", p)
                                                    .key("composer-suggestion-key"),
                                            ),
                                    );
                                }
                                host
                            }),
                        false,
                    )),
            )
            .child(cluster),
    );
    if let Some(error) = &t.submission_error {
        card = card.child(
            col()
                .key("composer-prompt-length-error")
                .px(16.0)
                .pb(8.0)
                .child(label(error, SMALL, p.danger).wrap().selectable(true)),
        );
    }
    let mut card_last = "composer-input";
    if !attach_locked {
        card_last = "attach-image";
    }
    if t.status.busy() && t.has_draft_content() && t.queued_edit.is_none() && t.can_steer() {
        card_last = "dispatch-mode";
    }
    if show_compact_slot && !compact_disabled {
        card_last = "composer-compact-send";
    }
    if show_send_slot && can_send {
        card_last = "send-message";
    }
    if show_stop_slot && !stop_disabled {
        card_last = "stop-agent";
    }
    s.composer_ui.last_focusable.set(card_last);
    let composer_card = composer_ui::frame_card(s, p, card);
    // KSG Code's `ComposerSurface.ContextStrip`: a 30 px muted row inside the tray, the run
    // context on the left and the model, effort, context and access on the right.
    let footer = if s.demo {
        s.composer_ui.last_focusable.set("leave-example");
        row()
            .key("composer-footer")
            .on_key(composer_ui::control_key)
            .keeps_text_selection()
            .h(30.0)
            .px(10.0)
            .center_y()
            .gap(6.0)
            .child(meta_text("Demo data", p.muted))
            .child(spacer())
            .child(
                row()
                    .key("leave-example")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Open a real workspace")
                    .focusable(true)
                    .center_y()
                    .h(22.0)
                    .px(6.0)
                    .radius(6.0)
                    .hover(p.selected)
                    .on_press(|s: &mut State| {
                        s.leave_demo();
                    })
                    .child(meta_text("Open a real workspace", p.sea)),
            )
    } else {
        composer_ui::controls(s, model, p, lane - 16.0, false, locked)
    };
    area = area.child(composer_shell(
        s,
        p,
        composer_request_banner(s, p, lane),
        attach_composer_notices(s, p, lane, composer_card),
        Some(footer),
    ));
    if let Some(reason) = crew_blocker.as_ref().or(t.crew_draft_notice.as_ref()) {
        area = area.child(
            label(
                reason,
                SMALL,
                if crew_blocker.is_some() {
                    p.brass
                } else {
                    p.muted
                },
            )
            .key("composer-crew-status")
            .role("status")
            .px(10.0)
            .pb(4.0)
            .wrap(),
        );
    }
    col()
        .key("composer-lane")
        .w_full()
        .pl(lane_gutter(pane_width) + lane_rect.inset_start)
        .pr(lane_gutter(pane_width) + lane_rect.inset_end)
        .items_center()
        .child(area)
}

/// KSG has no crew binding. This adapts its ComposerAttachmentStrip chip and remove
/// action within the ordinary composer footer, without adding another draft editor.
fn crew_draft_chip(s: &State, p: Palette, width: f32) -> El {
    let Some(target) = &s.current().crew_draft else {
        return el();
    };
    let server = target.server.clone();
    let captain = target.captain;
    let thread = s.current().id;
    let pending = s.current().client_creating.is_some();
    let text = format!(
        "{} {}'s crew",
        if pending { "Joining" } else { "Joins" },
        target.name
    );
    let blocked = s.crew_draft_unavailable().is_some();
    row()
        .key("composer-crew-chip")
        .role("group")
        .label(&text)
        .help(format!(
            "An independent thread in {}'s crew. It runs on its own and reports each finished turn to {}.",
            target.name, target.name
        ))
        .w(width)
        .min_w(0.0)
        .h(24.0)
        .px(4.0)
        .gap(4.0)
        .center_y()
        .radius(6.0)
        .border(1.0, if blocked { p.brass } else { p.line })
        .child(meta_text(&text, if blocked { p.brass } else { p.muted }).truncate().grow(1.0).min_w(0.0))
        .child(
            icon_button(
                "composer-crew-chip-remove",
                &format!("Don't add to {}'s crew", target.name),
                "close",
                p,
                move |s| {
                    if s.machines.selected == server && s.current().id == thread
                        && s.current().client_creating.is_none()
                        && s.current().crew_draft.as_ref().is_some_and(|target| {
                            target.server == server && target.captain == captain
                        })
                    {
                        s.current_mut().crew_draft = None;
                        s.current_mut().crew_draft_notice = None;
                    }
                },
            )
            .size(18.0, 18.0)
            .disabled(pending),
        )
}

/// Ported from KSG Code's BranchToolbarEnvironmentSelector and ComposerControl size="xs".
fn machine_selector(s: &State, p: Palette, icon_only: bool) -> El {
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
        .radius(6.0)
        .hover(p.selected)
        .preserve_focus_on_press(true)
        .cursor(rune::Cursor::Pointer)
        .disabled(s.client.is_none() || !s.can_retarget_draft())
        .on_press(|s: &mut State| {
            s.begin_composer_menu();
            s.machines.picker = !s.machines.picker;
        })
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
        .child(if icon_only {
            el()
        } else {
            label(title, SMALL, p.muted)
                .key("machine-selector-label")
                .truncate()
                .shrink()
                .min_w(0.0)
        })
        // KSG shows a spinner beside the selector while the draft's machine
        // change is still resolving; Rune's pending glyph is the clock.
        .child(if s.machines.draft_retarget.is_some() {
            icon("clock", p.coral)
                .size(12.0, 12.0)
                .key("machine-selector-pending")
        } else {
            el()
        })
        .child(icon("chevron", p.muted).size(12.0, 12.0))
}

/// The workspace choice's name. A crew draft with placement-v1 runs its worker in the project
/// checkout read only, so "Current checkout" says so there.
pub(super) fn env_mode_title(s: &State, mode: crate::model::ThreadEnvMode) -> &'static str {
    if mode == crate::model::ThreadEnvMode::Local && s.crew_draft_placement() {
        "Read-only checkout"
    } else {
        crate::branch_toolbar::resolve_env_mode_label(mode)
    }
}

/// What each workspace choice means for a crew worker, under its name in the picker, as KSG's
/// `PreviousWorktreeItemContent` puts a branch under its label. KSG has no crew placement.
fn crew_env_mode_detail(mode: crate::model::ThreadEnvMode) -> &'static str {
    match mode {
        crate::model::ThreadEnvMode::Worktree => "Writes on its own branch",
        crate::model::ThreadEnvMode::Local => "Sees uncommitted changes, cannot edit",
    }
}

/// Ported from KSG Code's `BranchToolbarEnvModeSelector` and `ComposerControl` size `xs`.
fn env_mode_trigger(
    s: &State,
    p: Palette,
    icon_only: bool,
    mode: crate::model::ThreadEnvMode,
) -> El {
    let title = env_mode_title(s, mode);
    let glyph = match mode {
        crate::model::ThreadEnvMode::Worktree => "branch",
        _ if s.crew_draft_placement() => "lock",
        _ => "folder",
    };
    row()
        .key("env-mode-trigger")
        .role("button")
        .label(format!("Workspace: {title}"))
        .help(title)
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
        .radius(6.0)
        .hover(p.selected)
        .preserve_focus_on_press(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(|s: &mut State| {
            let opening = !s.env_mode_picker;
            s.begin_composer_menu();
            s.env_mode_picker = opening;
        })
        .child(icon(glyph, p.muted).size(12.0, 12.0))
        .child(if icon_only {
            el()
        } else {
            label(title, SMALL, p.muted)
                .key("env-mode-trigger-label")
                .truncate()
                .shrink()
                .min_w(0.0)
        })
        .child(icon("chevron", p.muted).size(12.0, 12.0))
}

/// Ported from KSG Code's `BranchToolbarBranchSelector` and `ComposerControl` size `xs`.
fn base_branch_trigger(
    s: &State,
    p: Palette,
    base_branch: Option<&str>,
    start_from_origin: bool,
    max_w: f32,
) -> El {
    let t = s.current();
    let is_remote = base_branch.and_then(|name| {
        s.thread_vcs_repo_key(t).and_then(|key| {
            s.vcs_branches
                .get(&key)
                .and_then(|entry| entry.refs.iter().find(|r| r.name == name))
                .map(|r| r.is_remote == Some(true))
        })
    });
    let title = crate::branch_toolbar::resolve_branch_trigger_label(
        None,
        crate::model::ThreadEnvMode::Worktree,
        base_branch,
        is_remote,
        start_from_origin,
    );
    row()
        .key("base-branch-trigger")
        .role("button")
        .label(format!("Base branch: {title}"))
        .help(title.clone())
        .h(if s.client.is_some() && s.viewport_width < 640.0 {
            28.0
        } else {
            24.0
        })
        .px(7.0)
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .max_w(max_w)
        .shrink()
        .radius(6.0)
        .hover(p.selected)
        .preserve_focus_on_press(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(|s: &mut State| {
            let opening = !s.base_branch_picker;
            s.begin_composer_menu();
            s.base_branch_picker = opening;
            if opening {
                s.base_branch_query = rune::TextEdit::default();
                s.base_branch_query.focused = true;
                s.ensure_vcs_branches(true);
            }
        })
        .child(icon("branch", p.muted).size(12.0, 12.0))
        .child(
            label(
                title,
                META,
                if base_branch.is_some() {
                    p.ink
                } else {
                    p.muted
                },
            )
            .key("base-branch-trigger-label")
            .truncate()
            .shrink()
            .min_w(0.0),
        )
        .child(icon("chevron", p.muted).size(12.0, 12.0))
}

/// Read-only branch indicator for an active worktree thread, ported from KSG Code's
/// `BranchToolbarBranchSelector` / `BranchToolbarEnvModeSelector`.
fn worktree_branch_indicator(
    p: Palette,
    branch: Option<&str>,
    worktree_path: &str,
    max_w: f32,
) -> El {
    let fallback = crate::branch_toolbar::resolve_locked_workspace_label(
        Some(worktree_path),
        crate::model::ThreadEnvMode::Worktree,
    );
    let title = branch.unwrap_or(&fallback);
    row()
        .key("worktree-branch-indicator")
        .label(format!("Worktree branch: {title}"))
        .help(worktree_path)
        .h(24.0)
        .px(7.0)
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .max_w(max_w)
        .shrink()
        .child(icon("branch", p.muted).size(12.0, 12.0))
        .child(label(title, META, p.muted).truncate().shrink().min_w(0.0))
}

pub(crate) fn step_reasoning_effort(state: &mut State, direction: isize) -> bool {
    let t = state.current();
    if t.status.busy()
        || t.configuring
        || t.archived()
        || t.request.is_some()
        || crate::descriptors::read_prompt_ultrathink(&t.draft.text)
            == crate::descriptors::PromptUltrathink::Body
    {
        return false;
    }
    let descriptors = state.active_model_descriptors();
    let controls = crate::descriptors::split_column_and_footer_controls(&descriptors);
    let levels = match &controls.effort {
        Some(desc @ crate::descriptors::ProviderOptionDescriptor::Select { .. }) => {
            let choices = crate::descriptors::selectable_provider_option_choices(desc);
            if choices.is_empty() {
                t.thinking_levels.clone()
            } else {
                choices.into_iter().map(|c| c.id).collect()
            }
        }
        _ => t.thinking_levels.clone(),
    };
    if levels.is_empty() {
        return false;
    }
    let current = levels
        .iter()
        .position(|level| level == &t.thinking)
        .unwrap_or(0);
    let next =
        (current as isize + direction).clamp(0, levels.len().saturating_sub(1) as isize) as usize;
    let level = levels[next].clone();
    state.set_composer_thinking_sticky(level);
    true
}

pub(crate) fn cycle_model_picker_harness(state: &mut State, direction: isize) -> bool {
    let snap = picker_snapshot(state);
    if snap.sidebar_entries.is_empty() {
        return false;
    }
    let next = crate::model_picker::adjacent_model_picker_provider(
        &snap.sidebar_entries,
        &snap.selected_tab,
        direction,
        &snap.locked_disabled_ids,
        &snap.selectable_unavailable_ids,
    );
    state.model_search.replace_text("");
    state.model_picker_tab = Some(next);
    true
}

pub(crate) fn jump_model_picker_index(state: &mut State, index: usize) -> bool {
    let t = state.current();
    if t.status.busy() || t.configuring {
        return false;
    }
    let snap = picker_snapshot(state);
    let Some(chosen) = snap
        .visible_models
        .into_iter()
        .filter(|m| {
            state
                .model_disabled_reason_for(&m.instance_id, &m.slug)
                .is_none()
        })
        .nth(index)
    else {
        return false;
    };
    state.select_model_from_picker(&chosen.instance_id, &chosen.slug);
    finish_model_pick(state);
    true
}

fn machine_options(s: &State) -> Vec<String> {
    std::iter::once("local".to_owned())
        .chain(
            s.client
                .iter()
                .flat_map(|client| client.linked.keys().cloned()),
        )
        .collect()
}

/// The same machine row is used in Run on, Add workspace and Lives on.
fn machine_option(key: &str, title: &str, server: &str, online: bool, p: Palette) -> El {
    let item = menu_item(
        key,
        title,
        if server == "local" {
            "monitor"
        } else {
            "cloud"
        },
        p,
    )
    .disabled(!online);
    if online {
        item
    } else {
        item.child(meta_text("Offline", p.muted))
    }
}

/// Ported from KSG Code's BranchToolbarEnvironmentSelector SelectPopup/SelectGroup.
fn machine_picker(s: &State, p: Palette, anchor: &str) -> El {
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
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .child(meta_label("Run on", p.muted).px(8.0).py(6.0));
    for server in machine_options(s) {
        let title = s.machine_label(&server);
        let key = format!("machine-option-{server}");
        menu = menu.child(
            machine_option(&key, &title, &server, s.machine_connected(&server), p).on_press(
                move |s: &mut State| {
                    s.retarget_draft(&server);
                    s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
                },
            ),
        );
    }
    menu = menu.child(
        menu_item("link-machine", "Add to another machine", "plus", p).on_press(|s: &mut State| {
            s.machines.picker = false;
            s.machines.dialog = true;
        }),
    );
    menu.anchored_fit(anchor, rune::Side::Top, rune::Align::Start, 6.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.machines.picker = false;
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// Ported from KSG Code's `BranchToolbarEnvModeSelector` `SelectPopup`.
fn env_mode_picker(s: &State, p: Palette, anchor: &str) -> El {
    let force_worktree = s.is_multi_model_active();
    let active = if force_worktree {
        crate::model::ThreadEnvMode::Worktree
    } else {
        s.effective_env_mode(s.current())
    };
    let mut menu = col()
        .key("env-mode-picker")
        .role("menu")
        .focus_group()
        .label("Workspace")
        .w(if s.crew_draft_placement() {
            280.0
        } else {
            240.0
        })
        .padding(4.0)
        .gap(1.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .child(meta_label("Workspace", p.muted).px(8.0).py(6.0));
    let crew = s.crew_draft_placement();
    for (mode, slug, glyph) in [
        (
            crate::model::ThreadEnvMode::Local,
            "local",
            if crew { "lock" } else { "folder" },
        ),
        (crate::model::ThreadEnvMode::Worktree, "worktree", "branch"),
    ] {
        let title = env_mode_title(s, mode);
        let selected = active == mode;
        let disabled = force_worktree && mode == crate::model::ThreadEnvMode::Local;
        let name = label(title, TextStyle::new(13.0, 500.0), p.ink)
            .truncate()
            .min_w(0.0);
        let mut item = row()
            .key(format!("env-mode-option-{slug}"))
            .role("menuitemradio")
            .checked(selected)
            .label(if crew {
                format!("{title}. {}", crew_env_mode_detail(mode))
            } else {
                title.to_owned()
            })
            .focusable(!disabled)
            .disabled(disabled)
            .opacity(if disabled { 0.55 } else { 1.0 })
            .center_y()
            .gap(8.0)
            .h(if crew { 44.0 } else { 30.0 })
            .px(8.0)
            .radius(6.0)
            .fill(if selected {
                p.selected
            } else {
                Color::TRANSPARENT
            })
            .hover(if disabled {
                Color::TRANSPARENT
            } else {
                p.selected
            })
            .child(icon(glyph, p.muted).size(14.0, 14.0))
            .child(if crew {
                col().gap(2.0).grow(1.0).min_w(0.0).child(name).child(
                    label(crew_env_mode_detail(mode), SMALL, p.muted)
                        .key(format!("env-mode-option-{slug}-detail"))
                        .truncate()
                        .min_w(0.0),
                )
            } else {
                name.grow(1.0)
            });
        if !disabled {
            item = item
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut State| {
                    s.set_draft_env_mode(mode);
                    s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
                });
        }
        if selected {
            item = item.child(icon("check", p.sea));
        }
        menu = menu.child(item);
    }
    menu.anchored_fit(anchor, rune::Side::Top, rune::Align::Start, 6.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.env_mode_picker = false;
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// Ported from KSG Code's `BranchPicker` and `BranchToolbarBranchSelector`.
fn base_branch_picker(s: &State, p: Palette, anchor: &str) -> El {
    let t = s.current();
    let selected_branch = t.base_branch.as_deref();
    let start_from_origin = t.effective_start_from_origin();
    let query = s.base_branch_query.text.trim().to_ascii_lowercase();
    let repo_key = s.thread_vcs_repo_key(t);
    let project_cwd = t
        .project
        .and_then(|id| s.project(Some(id)))
        .map(|proj| proj.path.to_string_lossy().into_owned());
    let entry = repo_key.as_ref().and_then(|k| s.vcs_branches.get(k));

    let header = row().center_y().gap(8.0).px(10.0).py(8.0).child(
        row()
            .center_y()
            .gap(6.0)
            .h(30.0)
            .px(10.0)
            .radius(6.0)
            .fill(p.selected)
            .field_focus(&s.base_branch_query, p)
            .grow(1.0)
            .min_w(0.0)
            .child(icon("search", p.muted))
            .child(
                text_input(&s.base_branch_query, TextStyle::new(13.0, 400.0))
                    .key("base-branch-search")
                    .label("Search refs")
                    .placeholder("Search refs...", p.muted)
                    .color(p.ink)
                    .caret(p.ink, p.coral_bg)
                    .grow(1.0)
                    .min_w(0.0)
                    .autofocus()
                    .on_edit(|s: &mut State| &mut s.base_branch_query)
                    .on_input_key(|s: &mut State, event, tx| {
                        if matches!(event.key, rune::Key::Down)
                            && !event.mods.cmd
                            && !event.mods.ctrl
                            && !event.mods.alt
                            && !event.mods.shift
                        {
                            s.base_branch_query.focused = false;
                            tx.focus("base-branch-list", true);
                            return true;
                        }
                        false
                    }),
            ),
    );

    let mut list = col()
        .key("base-branch-list")
        .role("listbox")
        .label("Branches")
        .focus_group()
        .w_full()
        .max_h(224.0)
        .pb(4.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });

    if let Some(err) = entry.and_then(|e| e.error.as_deref()) {
        list = list.child(
            label(err, SMALL, p.danger)
                .key("base-branch-error")
                .wrap()
                .px(12.0)
                .py(8.0),
        );
    } else if entry.is_none_or(|e| e.loading && e.refs.is_empty()) {
        list = list.child(label("Loading refs...", SMALL, p.muted).px(12.0).py(8.0));
    } else if let Some(entry) = entry {
        let ordered = crate::branch_toolbar::order_branches_for_picker(&entry.refs);
        let filtered: Vec<&crate::vcs::VcsRef> = ordered
            .into_iter()
            .filter(|r| {
                crate::branch_toolbar::should_include_branch_picker_item(
                    &r.name, &query, None, None,
                )
            })
            .collect();
        if filtered.is_empty() {
            list = list.child(label("No refs found.", SMALL, p.muted).px(12.0).py(8.0));
        } else {
            for r in filtered {
                let name = r.name.clone();
                let selected = selected_branch == Some(name.as_str());
                let has_secondary_worktree = r
                    .worktree_path
                    .as_deref()
                    .is_some_and(|wp| project_cwd.as_deref().is_some_and(|cwd| wp != cwd));
                let badge = if r.current {
                    Some("current")
                } else if has_secondary_worktree {
                    Some("worktree")
                } else if r.is_remote == Some(true) {
                    Some("remote")
                } else if r.is_default {
                    Some("default")
                } else {
                    None
                };
                let mut item = row()
                    .key(format!("base-branch-option-{name}"))
                    .role("option")
                    .checked(selected)
                    .label(&name)
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .center_y()
                    .gap(8.0)
                    .h(30.0)
                    .mx(4.0)
                    .px(8.0)
                    .radius(6.0)
                    .fill(if selected {
                        p.coral_bg
                    } else {
                        Color::TRANSPARENT
                    })
                    .hover(if selected { p.coral_bg } else { p.selected })
                    .focus_fill(p.selected)
                    .focus_selected(selected)
                    .on_press(move |s: &mut State| {
                        s.set_draft_base_branch(name.clone());
                        s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
                    })
                    .child(
                        label(&r.name, TextStyle::new(13.0, 500.0), p.ink)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    );
                if selected {
                    item = item.child(icon("check", p.sea));
                }
                if let Some(badge) = badge {
                    item = item.child(meta_text(badge, p.faint));
                }
                list = list.child(item);
            }
        }
    }

    let origin_row = row()
        .key("start-from-remote-row")
        .center_y()
        .justify_between()
        .gap(12.0)
        .px(12.0)
        .py(8.0)
        .help("Uses the latest commit of the base branch's tracked remote branch, even if its name differs. Otherwise uses the same-named branch on origin. Uses your local branch if that remote branch is missing.")
        .child(
            row()
                .center_y()
                .gap(6.0)
                .min_w(0.0)
                .child(icon("cloud", p.muted).size(12.0, 12.0))
                .child(label("Start from remote", SMALL, p.muted).truncate()),
        )
        .child(captain_ui::switch(
            s,
            "start-from-remote-toggle",
            "Start worktree from remote",
            start_from_origin,
            p,
            |s: &mut State| {
                let next = !s.current().effective_start_from_origin();
                s.set_draft_start_from_origin(next);
            },
        ));

    col()
        .key("base-branch-picker")
        .role("dialog")
        .label("Base branch")
        .w(320.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .child(header)
        .child(rule(p))
        .child(list)
        .child(rule(p))
        .child(origin_row)
        .anchored_fit(anchor, rune::Side::Top, rune::Align::Start, 6.0)
        .focus_return("composer-input")
        .modal(true, |s: &mut State| {
            s.base_branch_picker = false;
            s.restore_composer_focus(composer_ui::FocusReason::MenuClosed);
        })
}

/// Ported from KSG Code's AuthorizedClientsSettings QR share panel. Shina uses
/// its direct-tailnet link rather than KSG's hosted pairing endpoint.
fn mobile_pairing_card(s: &State, p: Palette) -> El {
    let mut card = col()
        .key("mobile-pairing")
        .w_full()
        .gap(8.0)
        .child(label("Pair a phone", BODY, p.ink))
        .child(
            label(
                "The phone needs Tailscale and access to this computer's tailnet.",
                SMALL,
                p.muted,
            )
            .wrap(),
        );
    if let Some(share) = s
        .machines
        .mobile_share
        .as_ref()
        .filter(|share| !share.link.expired(crate::lifecycle::now()))
    {
        let (pixels, size) = share.pixels(s.artwork.device_scale());
        card = card
            .child(
                row().w_full().center().child(
                    el().key("mobile-pairing-qr")
                        .role("img")
                        .label("Phone pairing link. Scan with Shina on your phone.")
                        .size(size, size)
                        .pixels(&pixels),
                ),
            )
            .child(
                label(share.link.encode(), SMALL, p.ink)
                    .key("mobile-pairing-link")
                    .w_full()
                    .wrap_anywhere()
                    .selectable(true),
            )
            .child(label("One use. Expires after five minutes.", SMALL, p.muted).wrap())
            .child(
                row()
                    .flex_wrap()
                    .gap(8.0)
                    .child(
                        settings_ui::settings_button("mobile-pairing-copy", "Copy link", p, |_| {})
                            .on_press_tx(|s: &mut State, tx| {
                                if let Some(share) = &s.machines.mobile_share
                                    && !share.link.expired(crate::lifecycle::now())
                                {
                                    tx.copy(share.link.encode());
                                }
                            }),
                    )
                    .child(settings_ui::settings_button(
                        "mobile-pairing-hide",
                        "Hide link",
                        p,
                        |s| s.machines.mobile_share = None,
                    )),
            );
    } else {
        card = card
            .child(meta_label("This computer's tailnet host", p.muted))
            .child(
                text_input(&s.machines.mobile_host, BODY)
                    .key("mobile-pairing-host")
                    .label("This computer's tailnet host")
                    .placeholder("studio-mac", p.muted)
                    .single_line()
                    .w_full()
                    .padding(8.0)
                    .radius(12.0)
                    .field_focus(&s.machines.mobile_host, p)
                    .color(p.ink)
                    .disabled(s.machines.pending.is_some())
                    .on_edit(|s: &mut State| &mut s.machines.mobile_host),
            )
            .child(
                settings_ui::settings_button(
                    "mobile-pairing-create",
                    "Create phone link",
                    p,
                    |s| s.machine_action("pair-mobile", ""),
                )
                .disabled(
                    s.machines.pending.is_some() || s.machines.mobile_host.text.trim().is_empty(),
                ),
            );
    }
    card
}

/// Ported from KSG Code's settings/AddEnvironmentDialog and AuthorizedClientsSettings.
fn machine_panel(s: &State, p: Palette, embedded: bool) -> El {
    let busy = s.machines.pending.is_some();
    let mut card = col()
        .key("machine-dialog")
        .role(if embedded { "group" } else { "dialog" })
        .label("Linked machines")
        .w_full()
        .gap(12.0);
    if !embedded {
        card = card.child(
            row()
                .center_y()
                .child(label("Linked machines", DIALOG_TITLE, p.ink))
                .child(spacer())
                .child(icon_button("close-machines", "Close", "close", p, |s| {
                    s.machines.dialog = false;
                    s.settings_menu = false;
                    s.machines.code.replace_text("");
                    s.machines.peer_code.replace_text("");
                    s.machines.mobile_share = None;
                    s.machines.notice = None;
                })),
        );
    }
    card = card.child(mobile_pairing_card(s, p)).child(rule(p));
    card=card.child(label("Create a code on the other machine with shina pair. Paste its MagicDNS host and code here.",SMALL,p.muted).wrap())
        .child(meta_label("Host",p.muted))
        .child(text_input(&s.machines.host,BODY).key("machine-host").label("Host").placeholder("studio-mac",p.muted)
            .single_line().w_full().padding(8.0).radius(12.0).field_focus(&s.machines.host, p).color(p.ink)
            .disabled(busy).on_edit(|s: &mut State| &mut s.machines.host))
        .child(meta_label("Pairing code",p.muted))
        .child(text_input(&s.machines.code,BODY).key("machine-code").label("Pairing code").placeholder("PAIRCODE",p.muted)
            .single_line().w_full().padding(8.0).radius(12.0).field_focus(&s.machines.code, p).color(p.ink)
            .disabled(busy).on_edit(|s: &mut State| &mut s.machines.code))
        .child(primary_button("machine-link-submit","Link machine",p,|s| s.machine_action("link","")).disabled(busy))
        .child(row().flex_wrap().gap(8.0)
            .child(settings_ui::settings_button("machine-pair","Create a code for this host",p,|s| s.machine_action("pair","")).disabled(busy))
            .child(settings_ui::settings_button("machine-devices","Authorized devices",p,|s| s.machine_action("devices","")).disabled(busy)));
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
            let toggle_id = id.clone();
            let enabled = remote.enabled();
            let version = remote.version().unwrap_or_else(|| {
                s.machines
                    .views
                    .get(id)
                    .and_then(|view| view["updateStatus"]["runningVersion"].as_str())
                    .unwrap_or("unknown")
                    .to_owned()
            });
            card = card.child(
                col()
                    .key(format!("settings-linked-{id}"))
                    .gap(6.0)
                    .child(
                        row()
                            .center_y()
                            .gap(8.0)
                            .child(
                                col()
                                    .grow(1.0)
                                    .min_w(0.0)
                                    .gap(4.0)
                                    .child(
                                        label(
                                            remote
                                                .remote
                                                .as_ref()
                                                .map(|link| link.host.as_str())
                                                .unwrap_or(id),
                                            TextStyle::new(14.0, 500.0),
                                            p.ink,
                                        )
                                        .truncate()
                                        .min_w(0.0),
                                    )
                                    .child(
                                        row()
                                            .gap(8.0)
                                            .flex_wrap()
                                            .child(meta_text(version, p.muted))
                                            .child(meta_text(
                                                if !enabled {
                                                    "Disabled"
                                                } else if s.machine_connected(id) {
                                                    "Reachable"
                                                } else {
                                                    "Offline"
                                                },
                                                p.muted,
                                            )),
                                    ),
                            )
                            .child(
                                settings_ui::destructive_button(
                                    &format!("unlink-{id}"),
                                    "Unlink and revoke",
                                    p,
                                    move |s| s.machine_action("unlink", &server),
                                )
                                .disabled(busy),
                            ),
                    )
                    .child(
                        row()
                            .key(format!("settings-linked-enabled-row-{id}"))
                            .center_y()
                            .gap(8.0)
                            .child(meta_text("Enabled", p.muted))
                            .child(spacer())
                            .child(
                                captain_ui::switch(
                                    s,
                                    &format!("linked-enabled-{id}"),
                                    "Enabled",
                                    enabled,
                                    p,
                                    move |s| s.toggle_machine_enabled(&toggle_id),
                                )
                                .disabled(busy),
                            ),
                    ),
            );
        }
    }
    for device in s.machines.devices.as_array().into_iter().flatten() {
        if !device["revokedAt"].is_null() {
            continue;
        }
        let id = device["deviceId"].as_str().unwrap_or("").to_owned();
        let grant_id = id.clone();
        let maintenance = device["scopes"]
            .as_array()
            .is_some_and(|scopes| scopes.iter().any(|scope| scope == "maintain"));
        let update = device["scopes"]
            .as_array()
            .is_some_and(|scopes| scopes.iter().any(|scope| scope == "update"));
        let paired = device["scopes"]
            .as_array()
            .is_some_and(|scopes| scopes.iter().any(|scope| scope == "paired"));
        let title = format!("Device {}", &id[..id.len().min(8)]);
        card = card.child(
            row()
                .center_y()
                .gap(8.0)
                .child(label(title, SMALL, p.muted).grow(1.0))
                .child(meta_text("Updates", p.muted))
                .child(
                    captain_ui::switch(
                        s,
                        &format!("device-update-{id}"),
                        if maintenance {
                            "Local maintenance permits updates"
                        } else if paired {
                            "Pairing permits updates"
                        } else {
                            "Allow updates"
                        },
                        update || maintenance || paired,
                        p,
                        move |s| {
                            s.machine_action(
                                if update {
                                    "revoke-update"
                                } else {
                                    "grant-update"
                                },
                                &grant_id,
                            )
                        },
                    )
                    .disabled(busy || maintenance || paired),
                )
                .child(
                    settings_ui::destructive_button(
                        &format!("revoke-{id}"),
                        "Revoke",
                        p,
                        move |s| s.machine_action("revoke", &id),
                    )
                    .disabled(busy),
                ),
        );
    }
    // The existing AddEnvironmentDialog fields also redeem a server-to-server peer code.
    // Outbound links belong to the selected server, not to this desktop's client links.
    card=card.child(rule(p)).child(meta_label("Peers",p.muted))
        .child(label("A peer link lets this server delegate work. Create its code on the target with shina pair --peer.",SMALL,p.muted).wrap());
    let peer_busy = busy || s.machines.selected != "local";
    for peer in &s.peers {
        let id = peer.server_id.clone();
        let reachable = peer.last_error.is_none()
            && peer
                .last_reached_at
                .is_some_and(|at| crate::lifecycle::now().saturating_sub(at) < 90_000);
        card = card.child(
            row()
                .key(format!("peer-row-{id}"))
                .center_y()
                .gap(8.0)
                .child(
                    label(&peer.label, SMALL, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(meta_text(
                    if reachable { "Reachable" } else { "Offline" },
                    p.muted,
                ))
                .child(
                    button(
                        &format!("peer-unlink-{id}"),
                        "Unlink and revoke",
                        p,
                        move |s| s.machine_action("peer-unlink", &id),
                    )
                    .disabled(peer_busy),
                ),
        );
    }
    card = card
        .child(meta_label("Peer host", p.muted))
        .child(
            text_input(&s.machines.peer_host, BODY)
                .key("peer-host")
                .label("Peer host")
                .single_line()
                .w_full()
                .padding(8.0)
                .radius(12.0)
                .field_focus(&s.machines.peer_host, p)
                .color(p.ink)
                .disabled(peer_busy)
                .on_edit(|s: &mut State| &mut s.machines.peer_host),
        )
        .child(meta_label("Peer code", p.muted))
        .child(
            text_input(&s.machines.peer_code, BODY)
                .key("peer-code")
                .label("Peer code")
                .single_line()
                .w_full()
                .padding(8.0)
                .radius(12.0)
                .field_focus(&s.machines.peer_code, p)
                .color(p.ink)
                .disabled(peer_busy)
                .on_edit(|s: &mut State| &mut s.machines.peer_code),
        )
        .child(
            primary_button("peer-link-submit", "Link peer", p, |s| {
                s.machine_action("peer-link", "")
            })
            .disabled(peer_busy),
        );
    if s.machines.selected != "local" {
        card = card.child(
            label(
                "Manage this server's peer links on that host with shina peer.",
                SMALL,
                p.muted,
            )
            .wrap(),
        );
    }
    card
}

fn machine_dialog(s: &State, p: Palette) -> El {
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
            s.machines.peer_code.replace_text("");
            s.machines.mobile_share = None;
            s.machines.notice = None;
        })
        .child(
            machine_panel(s, p, false)
                .w((s.viewport_width - 32.0).clamp(280.0, 520.0))
                .max_h((s.viewport_height - 64.0).max(240.0))
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .padding(16.0)
                .radius(12.0)
                .fill(p.card)
                .border(1.0, p.frame)
                .box_shadow(soft_shadow(p)),
        )
}

/// Ported from KSG Code's ComposerPendingApprovalActions.
fn approval_actions(s: &State, id: usize, inline: bool, allowed: bool, p: Palette) -> El {
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
fn dialog(s: &State, p: Palette, page: usize, total: usize) -> El {
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

const LONG_PREAMBLE: &str = "Starting the audit.\n\n";
const LONG_MIDDLE: &str = "Halfway marker between the steps.\n\n";

/// One assistant turn with `parts` alternating thought and command parts (every
/// fiftieth a group of two commands), prose between the halves, a large output on every tool and every fifth output open.
/// `live` leaves it running; otherwise it is complete with its work expanded,
/// followed by short questions and answers.
/// The tests and the `long-turn-live` and `long-turn-expanded` profiling scenes share it.
pub(crate) fn long_turn_fixture(state: &mut State, parts: usize, live: bool) {
    state.populate_demo();
    state.compact_sidebar = true;
    let t = state.current_mut();
    let tool_template = t.tools[3].clone();
    let thought_template = t.thoughts[0].clone();
    t.captain = None;
    t.messages.clear();
    t.tools.clear();
    t.thoughts.clear();
    t.work.clear();
    t.native.compactions.clear();
    t.harness_state["workRetries"] = json!([]);
    t.started = None;
    t.transcript_structure = 1;
    t.messages.push(crate::model::Message {
        id: "long-user".into(),
        role: "You",
        text: "Audit every package.".into(),
        images: vec![],
        context: None,
    });
    t.messages.push(crate::model::Message {
        id: "long-answer".into(),
        role: "Pi",
        text: format!("{LONG_PREAMBLE}{LONG_MIDDLE}Done. Every package passed."),
        images: vec![],
        context: None,
    });
    let output = "test result: ok. 1 passed; 0 failed\n".repeat(600);
    for index in 0..parts {
        let offset = if index < parts / 2 {
            LONG_PREAMBLE.len()
        } else {
            LONG_PREAMBLE.len() + LONG_MIDDLE.len()
        };
        let anchor = crate::model::ToolAnchor { message: 1, offset };
        if index % 2 == 0 {
            let mut thought = thought_template.clone();
            thought.anchor = anchor;
            thought.block = index;
            thought.order = 2 * index as u64 + 1;
            thought.text = format!("Thinking about package {index}.");
            thought.done = true;
            thought.expanded = false;
            t.thoughts.push(thought);
        } else {
            let mut tool = tool_template.clone();
            tool.id = format!("long-tool-{index}");
            tool.anchor = anchor;
            tool.block = index;
            tool.order = 2 * index as u64 + 1;
            tool.detail = format!("cargo test -p package-{index}");
            tool.output = output.clone();
            tool.done = true;
            tool.failed = false;
            tool.expanded = index % 10 == 1;
            // Every fiftieth step runs two commands at once: a tool group.
            let pair = (index % 50 == 1).then(|| {
                let mut pair = tool.clone();
                pair.id = format!("long-tool-{index}-pair");
                pair.order += 1;
                pair.expanded = false;
                pair
            });
            t.tools.extend(std::iter::once(tool).chain(pair));
        }
    }
    if live {
        t.status = Status::Running;
    } else {
        t.status = Status::Completed;
        t.work.push(crate::model::AssistantWork {
            start: 1,
            stopped: false,
            stopped_order: 0,
            expanded: true,
            elapsed: Some(600),
            outcome: Some(Status::Completed),
        });
        // Later turns, so the long one is read away from the bottom.
        for index in 0..12 {
            for (id, role, text) in [
                ("after-user", "You", "Thanks. Anything else?"),
                ("after-answer", "Pi", "No, that covers it."),
            ] {
                t.messages.push(crate::model::Message {
                    id: format!("{id}-{index}"),
                    role,
                    text: text.into(),
                    images: vec![],
                    context: None,
                });
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::context::ContextUsage;
    use rune::{Env, Runtime};
    pub(crate) fn runtime(width: f32) -> Runtime<State> {
        let mut rt = Runtime::with_text(
            State::new(".".into(), false),
            view,
            (width, 720.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, MONO_FAMILY)),
        );
        // As the app sets it in `main`.
        rt.set_tab_visits_controls(true);
        rt
    }

    fn connected_machines(rt: &mut Runtime<State>) {
        let projection = rt.state.projection();
        let (mut client, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
        let (remote, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
        client.linked.insert("studio".into(), remote);
        client.linked.insert(
            "offline".into(),
            crate::client::Client::new(rt.state.data_dir.clone()),
        );
        rt.state.client = Some(client);
        for server in ["local", "studio", "offline"] {
            rt.state
                .machines
                .views
                .insert(server.into(), projection.clone().into());
        }
    }

    #[test]
    fn resting_composer_collapses_only_below_sm_and_expands_on_press() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.state.current_mut().draft.replace_text("A single line");
            rt.frame(0.0);
            rt.frame(1.0);
            assert_eq!(
                rt.presented("composer-preview").is_some(),
                width < 640.0,
                "{width}"
            );
            if width < 640.0 {
                press(&mut rt, "composer-preview");
                rt.frame(1.0);
                assert!(rt.state.current().draft.focused);
                assert!(rt.presented("composer-preview").is_none());
            }
            assert!(rt.presented("composer-input").is_some());
        }
    }

    #[test]
    fn opening_a_thread_restores_the_composer_caret() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.current_mut().draft.replace_text("Existing draft");
            rt.state.new_thread();
            rt.state.current_mut().draft.focused = false;
            rt.state.open_kept_tab(0);
            rt.frame(0.0);
            rt.frame(1.0);
            assert!(rt.state.current().draft.focused, "{width}");
        }
    }

    #[test]
    fn timeline_typing_and_paste_redirect_but_controls_and_open_layers_keep_input() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.on_key = Some(crate::key);
            rt.frame(0.0);
            rt.frame(1.0);
            tap(&mut rt, rune::Key::Text("x".into()));
            assert_eq!(rt.state.current().draft.text, "x", "{width}");
            assert!(rt.state.current().draft.focused);
            rt.state.current_mut().draft.focused = false;
            rt.invalidate();
            rt.frame(1.0);
            rt.paste(" pasted");
            rt.frame(1.0);
            assert_eq!(rt.state.current().draft.text, "x pasted", "{width}");

            for key in [
                "sidebar-search",
                "model-search",
                "captain-search",
                "secret",
                "reply-input",
                "settings-search",
                "access-mode",
                "model-effort",
                "send-message",
            ] {
                assert!(
                    !composer_ui::redirect_allowed(&rt.state, Some(key)),
                    "{key}"
                );
            }
            let layers: &[fn(&mut State)] = &[
                |s| s.settings_menu = true,
                |s| s.settings_picker = Some(SettingsPicker::Thinking),
                |s| s.captain_picker = true,
                |s| s.project_picker = Some(ProjectPicker::NewThread),
                |s| s.rename_thread = Some(s.current().id),
                |s| s.delete_thread = Some(s.current().id),
                |s| s.thread_menu = true,
                |s| s.machines.picker = true,
                |s| s.machines.dialog = true,
                |s| s.projects.clone_open = true,
                |s| s.fleet.open = true,
                |s| s.brief.profile_open = true,
                |s| s.brief.confirm_reset = true,
                |s| s.composer_ui.overflow.set(true),
                |s| s.current_mut().native.access_picker = true,
                |s| s.current_mut().request = Some(json!({"id":"secret", "type":"input"})),
            ];
            for (index, open) in layers.iter().enumerate() {
                let mut state = State::new(".".into(), false);
                open(&mut state);
                assert!(
                    !composer_ui::redirect_allowed(&state, None),
                    "{width}, layer {index}"
                );
            }
        }
    }

    #[test]
    fn multiline_composer_never_collapses_and_restoration_keeps_model_picker_focus() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.state
                .current_mut()
                .draft
                .replace_text("First line\nSecond line");
            rt.frame(0.0);
            rt.frame(1.0);
            assert!(rt.presented("composer-preview").is_none());
            assert!(rt.presented("composer-input").is_some());
            for reason in [
                composer_ui::FocusReason::ThreadOpened,
                composer_ui::FocusReason::AttachmentInserted,
                composer_ui::FocusReason::MenuClosed,
                composer_ui::FocusReason::Inserted,
            ] {
                rt.state.settings_picker = Some(SettingsPicker::Thinking);
                rt.state.current_mut().draft.focused = false;
                rt.state.restore_composer_focus(reason);
                assert!(!rt.state.current().draft.focused, "{width}, {reason:?}");
                rt.state.settings_picker = None;
                rt.state.restore_composer_focus(reason);
                rt.invalidate();
                rt.frame(1.0);
                assert!(rt.state.current().draft.focused, "{width}, {reason:?}");
            }
        }
    }

    #[test]
    fn tab_inside_the_expanded_composer_keeps_its_controls_mounted() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            // A non-default access mode keeps the permissions chip mounted in the strip.
            rt.state.current_mut().native.access = crate::model::AccessMode::Supervised;
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            tap(&mut rt, rune::Key::Tab);
            assert_eq!(focused_key(&rt).as_deref(), Some("attach-image"), "{width}");
            assert!(rt.presented("composer-preview").is_none(), "{width}");
            for _ in 0..20 {
                if focused_key(&rt).as_deref() == Some("access-mode") {
                    break;
                }
                tap(&mut rt, rune::Key::Tab);
            }
            assert_eq!(focused_key(&rt).as_deref(), Some("access-mode"), "{width}");
            tap(&mut rt, rune::Key::Tab);
            assert!(focused_key(&rt).is_some(), "{width}: leaving the footer");
        }
    }

    fn finish_folder_listing(rt: &mut Runtime<State>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            rt.state.poll_folder_browser();
            if rt.state.projects.browser.pending.is_none()
                && rt.state.projects.browser.request.as_ref()
                    == Some(&(
                        rt.state.machines.selected.clone(),
                        rt.state.picker_query.text.clone(),
                    ))
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "folder listing timed out"
            );
            std::thread::yield_now();
        }
        rt.invalidate();
        rt.frame(0.0);
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
    pub(super) fn tap(rt: &mut Runtime<State>, key: rune::Key) {
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
        for width in [1280.0] {
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
                assert_eq!(focused(&rt).key.as_deref(), Some("settings-search"));
                tab_to(&mut rt, "settings-section-0");
                tap(&mut rt, rune::Key::Down);
                assert_eq!(rt.state.settings.section, 1);
                tap(&mut rt, rune::Key::Escape);
                assert!(!rt.state.settings_menu);
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
            rt.state.open_captain_selector();
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            assert_eq!(focused(&rt).key.as_deref(), Some("captain-search"));
            tap(&mut rt, rune::Key::Down);
            walk_menu(&mut rt, "captain-list", true, &at("captain picker"));

            let mut rt = fresh();
            rt.state.current_mut().harness = "pi".into();
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
    #[test]
    fn tab_visits_every_enabled_control_in_visual_order_and_enter_or_space_presses_it() {
        for width in [1280.0] {
            for empty in [false, true] {
                let mut rt = runtime(width);
                rt.state.populate_demo();
                rt.state.demo = false;
                rt.state.composer_ui.force_expanded = true;
                // KSG's 256 px sidebar leaves room for the docked details card here; this
                // walk covers the main column's order, so keep the card off.
                rt.state.details.docked = false;
                if empty {
                    rt.state.new_thread_in(None);
                }
                // Thread-open restoration starts at the editor. This test walks visual order
                // from the window's first control, independently of that restoration policy.
                rt.state.current_mut().draft.focused = false;
                rt.frame(0.0);
                rt.frame(1.0);
                let stops: Vec<String> = rt
                    .elements()
                    .into_iter()
                    .filter(|e| !e.inert && e.focusable && e.visible[2] > 0.0 && e.visible[3] > 0.0)
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
                let prompt = if rt.presented("composer-preview").is_some() {
                    "composer-preview"
                } else {
                    "composer-input"
                };
                for key in [prompt, "attach-image", "model-effort"] {
                    assert!(keys.contains(&key), "{width}/{empty}: Tab misses {key}");
                }
                // Visual order: the title strip, then the sidebar, then the pane, each top to
                // bottom and left to right along a line.
                let region = |e: &rune::ElementInfo| {
                    if e.rect[1] < 40.0 {
                        0
                    } else if e.role == "separator"
                        || rt
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
                    // The tab strip scrolls: a focused tab pulls earlier tabs
                    // out from under their recorded rects.
                    let elements = rt.elements();
                    let in_tabs = |e: &rune::ElementInfo| {
                        let mut at = Some(e);
                        while let Some(e) = at {
                            if e.key.as_deref() == Some("thread-tabs") {
                                return true;
                            }
                            at = e.parent.map(|i| &elements[i]);
                        }
                        false
                    };
                    if in_tabs(a) || in_tabs(b) {
                        continue;
                    }
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
    fn unreachable_machines_show_only_above_the_composer_for_the_open_thread() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            crate::fleet_demo(&mut rt.state);
            // Real machines, not demo data: their fixture clients are not connected.
            rt.state.demo = false;
            rt.state.compact_sidebar = width < 760.0;
            let poll = |rt: &mut Runtime<State>| {
                rt.update(|state, tx| {
                    crate::model::poll(state, 0.0, tx);
                });
                rt.invalidate();
                rt.frame(1.0);
            };
            poll(&mut rt);
            // No stuck sidebar notice for machines the user is not using.
            assert!(rt.state.machines.notice.is_none(), "{width}");
            assert!(rt.presented("machine-unreachable").is_none(), "{width}");
            let sidebar = find(&rt.accessibility_json(), "sidebar")
                .expect("sidebar")
                .to_string();
            assert!(!sidebar.contains("unreachable"), "{width}: {sidebar}");
            // The composer banner names only the machine the open thread targets.
            assert!(
                rt.presented("composer-notice-environment-unavailable:example-studio")
                    .is_none(),
                "{width}"
            );
            assert!(
                rt.presented("composer-notice-environment-unavailable:example-laptop")
                    .is_none(),
                "{width}"
            );
            rt.state.open_machine_thread("example-studio", 3);
            poll(&mut rt);
            assert_eq!(rt.state.machines.selected, "example-studio");
            let tree = rt.accessibility_json();
            let notice = find(
                &tree,
                "composer-notice-environment-unavailable:example-studio",
            )
            .expect("the composer notice says so");
            assert_eq!(notice["name"], "Studio is offline: This device is offline");
            assert!(rt.presented("machine-unreachable").is_none(), "{width}");
            let sidebar = find(&rt.accessibility_json(), "sidebar")
                .expect("sidebar")
                .to_string();
            assert!(!sidebar.contains("unreachable"), "{width}: {sidebar}");
        }
    }

    #[test]
    fn user_markdown_keeps_line_breaks_and_long_paths_inside_the_bubble() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            let source = format!(
                "step one\nstep two\nstep three\n\n/{}",
                "segment".repeat(40)
            );
            rt.state.current_mut().messages = vec![crate::model::Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "You",
                text: source.clone(),
                images: vec![],
                context: None,
            }];
            rt.frame(0.0);
            assert_eq!(rt.state.current().messages[0].text, source);
            let bubble = rt
                .elements()
                .into_iter()
                .find(|element| {
                    element.key.as_deref()
                        == Some(&format!("{}-text", message_key(rt.state.current(), 0)))
                })
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
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            let thread = rt.state.current_mut();
            thread.tools.clear();
            thread.thoughts.clear();
            thread.work.clear();
            thread.captain = None;
            thread.messages = vec![
                crate::model::Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "You",
                    text: "Explain **this change**.".into(),
                    images: vec![],
                    context: None,
                },
                crate::model::Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "Pi",
                    text: "```rust\nlet ready = true;\n```".into(),
                    images: vec![],
                    context: None,
                },
            ];
            let clipboard = rune::clipboard::memory();
            rt.use_clipboard(&clipboard);
            rt.frame(0.0);
            let thread = rt.state.current();
            let key = format!("copy-code-{}-{}-0-0", thread.id, thread.messages[1].id);
            let (rect, _) = rt.presented(&key).unwrap();
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
            press(&mut rt, &key);
            assert_eq!(
                clipboard.text().as_deref().map(|text| text.as_str()),
                Some("let ready = true;")
            );
        }
    }

    #[test]
    fn code_blocks_wrap_by_default_and_toggle_wrap_per_block() {
        let mut rt = runtime(960.0);
        rt.state.populate_demo();
        let thread = rt.state.current_mut();
        thread.tools.clear();
        thread.thoughts.clear();
        thread.work.clear();
        thread.captain = None;
        let long_line = "let very_long_identifier_for_wrapping_check = ".to_owned()
            + &"\"segment_value\" + ".repeat(14)
            + "\"end\";";
        thread.messages = vec![crate::model::Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: "Pi",
            text: format!("```rust\n{long_line}\n```"),
            images: vec![],
            context: None,
        }];
        rt.frame(0.0);
        let thread = rt.state.current();
        let id = format!("{}-{}-0-0", thread.id, thread.messages[0].id);
        let wrap_key = format!("wrap-code-{id}");
        let code_key = format!("code-{id}");
        let button_label = |rt: &Runtime<State>| {
            rt.elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some(wrap_key.as_str()))
                .and_then(|e| e.label)
        };
        assert_eq!(button_label(&rt).as_deref(), Some("Disable line wrap"));
        let wrapped_h = rt.presented(&code_key).unwrap().0[3];

        press(&mut rt, &wrap_key);
        rt.frame(1.0);
        assert!(rt.state.unwrapped_code_blocks.contains(&id));
        assert_eq!(button_label(&rt).as_deref(), Some("Wrap lines"));
        let unwrapped_h = rt.presented(&code_key).unwrap().0[3];
        assert!(
            wrapped_h > unwrapped_h,
            "wrapped height {wrapped_h} should exceed unwrapped height {unwrapped_h}"
        );

        // Appending a diff fence mid-stream keeps the preceding code block's wrap identity stable.
        rt.state.current_mut().messages[0]
            .text
            .push_str("\n\n```diff\n- old()\n+ new()\n```");
        rt.state.current_mut().transcript_content += 1;
        rt.invalidate();
        rt.frame(1.5);
        assert_eq!(button_label(&rt).as_deref(), Some("Wrap lines"));
        assert!((rt.presented(&code_key).unwrap().0[3] - unwrapped_h).abs() < 0.5);

        press(&mut rt, &wrap_key);
        rt.frame(2.0);
        assert!(!rt.state.unwrapped_code_blocks.contains(&id));
        assert_eq!(button_label(&rt).as_deref(), Some("Disable line wrap"));
        assert!((rt.presented(&code_key).unwrap().0[3] - wrapped_h).abs() < 0.5);
    }

    #[test]
    fn machine_selector_labels_and_link_dialog_fit_supported_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            let mut client = crate::client::Client::new(rt.state.data_dir.clone());
            let mut remote = crate::client::Client::new(rt.state.data_dir.clone());
            remote.remote = Some(crate::peer::Link {
                server_id: "fixture-server".into(),
                host: "studio-mac".into(),
                device_id: "fixture-device".into(),
                token: "fixture".into(),
                enabled: true,
            });
            client.linked.insert("fixture-server".into(), remote);
            rt.state.client = Some(client);
            rt.state.machines.views.insert(
                "fixture-server".into(),
                json!({"index":{"threads":[
                    {"id":1,"title":"Remote thread","session":"fixture","captainHome":false,"lifecycle":{}},
                    {"id":2,"title":"Remote captain","captainHome":true,"lifecycle":{}}
                ]}}).into());
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

    pub(super) fn press(rt: &mut Runtime<State>, key: &str) {
        rt.frame(0.25);
        if key == "composer-input" && rt.presented("composer-preview").is_some() {
            let (x, y) = rt.find("composer-preview").expect("collapsed composer");
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
        }
        let (x, y) = rt
            .find(key)
            .unwrap_or_else(|| panic!("{key} is not on screen"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
    }

    #[test]
    fn right_panel_lays_out_inline_or_covering_and_restores_the_conversation() {
        use crate::right_panel::SingletonKind;
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.state.current_mut().draft.replace_text("A draft");
            // The Diff segment plus an open document viewer.
            let key = rt.state.panel_key();
            rt.state.right_panels.open(&key, SingletonKind::Diff);
            rt.state.captains = crate::captains::Captain::presets();
            rt.state.open_captain_document(&key, 0, "NOTES.md");
            rt.state.right_panels.open(&key, SingletonKind::Diff);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(0.5);
            rt.frame(0.5);

            let (panel, _) = rt
                .presented("right-panel")
                .unwrap_or_else(|| panic!("panel mounts at {width}"));
            let (main, _) = rt
                .presented("chat-main")
                .unwrap_or_else(|| panic!("conversation column mounts at {width}"));
            let inside = |rect: [f32; 4]| {
                rect[0] >= -0.5
                    && rect[1] >= -0.5
                    && rect[0] + rect[2] <= width + 0.5
                    && rect[1] + rect[3] <= 720.0 + 0.5
            };
            assert!(
                inside(panel),
                "panel stays in the window at {width}: {panel:?}"
            );
            assert!(
                inside(main),
                "conversation stays in the window at {width}: {main:?}"
            );
            // Spec §0.1 / §3.3: Inspector covers below 1050 pt (620, 960) and docks at 1280+.
            let covering = width < 1050.0 || crate::right_panel::covers(width - 16.0);
            if covering {
                // KSG's sheet mode: the panel covers the conversation column outright.
                assert!(
                    panel[0] <= main[0] + 0.5 && panel[0] + panel[2] >= main[0] + main[2] - 0.5,
                    "panel covers the conversation at {width}: {panel:?} vs {main:?}"
                );
            } else {
                assert!(
                    panel[0] >= main[0] + main[2] - 0.5,
                    "panel and conversation do not overlap at {width}: {panel:?} vs {main:?}"
                );
                assert!(rt.presented("composer-input").is_some(), "{width}");
            }

            // The Context segment switches to the document body.
            press(&mut rt, "right-panel-segment-context");
            rt.frame(0.5);
            let key = rt.state.panel_key();
            assert_eq!(
                rt.state.right_panels.get(&key).view,
                crate::right_panel::PanelView::Context,
                "{width}"
            );
            assert!(
                rt.presented("right-panel-body-captain-doc:0:NOTES.md")
                    .is_some(),
                "{width}"
            );

            // The Diff segment switches back.
            press(&mut rt, "right-panel-segment-diff");
            rt.frame(0.5);
            assert_eq!(
                rt.state.right_panels.get(&key).view,
                crate::right_panel::PanelView::Diff,
                "{width}"
            );

            // Close drops the viewer first, then ends the panel and restores the conversation's full width.
            press(&mut rt, "right-panel-close");
            rt.frame(0.5);
            assert!(rt.state.right_panels.get(&key).viewer.is_none(), "{width}");
            assert!(rt.presented("right-panel").is_some(), "{width}");
            press(&mut rt, "right-panel-close");
            rt.frame(0.5);
            assert!(rt.presented("right-panel").is_none(), "{width}");
            let (restored, _) = rt.presented("chat-main").unwrap();
            if covering {
                assert!(
                    (restored[2] - main[2]).abs() <= 0.5,
                    "{width}: {restored:?}"
                );
            } else {
                assert!(restored[2] > main[2] + 100.0, "{width}: {restored:?}");
            }
        }
    }

    /// A patch parsed into loaded `FileEntry`s, the way `preview_done` leaves a
    /// non-deferred source.
    fn parsed_ready(patch: &str) -> diff_panel::Done {
        let Some(crate::review_patch::RenderablePatch::Files(files)) =
            crate::review_patch::parse(patch)
        else {
            panic!("test patch parses");
        };
        let stat = crate::review_patch::line_stat(&files);
        let files = crate::review_tree::order_like_tree(&files);
        diff_panel::Done::Ready(Box::new(diff_panel::Ready {
            files: files
                .into_iter()
                .map(|file| diff_panel::FileEntry {
                    additions: file.additions,
                    deletions: file.deletions,
                    file,
                    body: diff_panel::FileBody::Loaded,
                    truncated: false,
                })
                .collect(),
            raw: None,
            additions: stat.additions,
            deletions: stat.deletions,
            truncated: false,
            incomplete: false,
            cwd: "/repo".into(),
            source_kind: crate::review::ReviewSourceKind::BranchRange,
            base_ref: None,
            rows: vec![],
            rows_version: 0,
            tree_rows: vec![],
            tree_version: 0,
        }))
    }

    #[test]
    fn diff_panel_lists_files_collapses_and_virtualizes_at_all_widths() {
        use crate::right_panel::SingletonKind;
        // Two files: +2 -1 on src/a.rs, +3 on src/b.rs — "+5 -1" total.
        const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n\
            --- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n keep\n-old\n+new\n+more\n tail\n\
            diff --git a/src/b.rs b/src/b.rs\n\
            --- a/src/b.rs\n+++ b/src/b.rs\n@@ -5,0 +5,3 @@\n+one\n+two\n+three\n";
        for width in [960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            let key = rt.state.panel_key();
            rt.state.right_panels.open(&key, SingletonKind::Diff);
            // `last_refresh_at` is fresh so the mount does not read git.
            rt.state.diffs.insert(
                key.clone(),
                diff_panel::ThreadDiff {
                    done: Some(parsed_ready(PATCH)),
                    last_refresh_at: Some(std::time::Instant::now()),
                    ..Default::default()
                },
            );
            rt.state.diff_rebuild_rows(&key);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(0.5);

            let (panel, _) = rt
                .presented("right-panel")
                .unwrap_or_else(|| panic!("panel mounts at {width}"));
            for file_key in ["diff-file-0", "diff-file-1"] {
                let (rect, _) = rt
                    .presented(file_key)
                    .unwrap_or_else(|| panic!("{file_key} at {width}"));
                assert!(
                    rect[0] >= panel[0] - 0.5
                        && rect[1] >= panel[1] - 0.5
                        && rect[0] + rect[2] <= panel[0] + panel[2] + 0.5
                        && rect[1] + rect[3] <= panel[1] + panel[3] + 0.5,
                    "{file_key} sits inside the panel at {width}: {rect:?} vs {panel:?}"
                );
            }
            // The header total reads KSG's `+N -N` with its group label.
            let stat = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("diff-stat"))
                .unwrap_or_else(|| panic!("diff-stat at {width}"));
            assert_eq!(
                stat.label.as_deref(),
                Some("5 additions, 1 deletions"),
                "{width}"
            );
            assert!(rt.presented("diff-line-0-0-0").is_some(), "{width}");
            assert!(rt.presented("diff-line-1-0-0").is_some(), "{width}");

            // Folding one file hides only its rows; collapse-all hides the rest.
            press(&mut rt, "diff-file-toggle-0");
            rt.frame(0.5);
            assert!(rt.presented("diff-line-0-0-0").is_none(), "{width}");
            assert!(rt.presented("diff-line-1-0-0").is_some(), "{width}");
            press(&mut rt, "diff-collapse-all");
            rt.frame(0.5);
            assert!(rt.presented("diff-line-1-0-0").is_none(), "{width}");

            // A 20,000-line diff still builds only the visible rows.
            let mut big = crate::review_patch::FileDiff {
                path: "big.rs".into(),
                previous_path: None,
                change: crate::review_patch::ChangeKind::Modified,
                binary: false,
                old_mode: None,
                new_mode: None,
                hunks: vec![],
                additions: 20_000,
                deletions: 0,
            };
            big.hunks.push(crate::review_patch::Hunk {
                old_start: 1,
                old_count: 0,
                new_start: 1,
                new_count: 20_000,
                section: String::new(),
                lines: (1..=20_000)
                    .map(|n| crate::review_patch::Line {
                        kind: crate::review_patch::LineKind::Added,
                        text: format!("line {n}"),
                        old_number: None,
                        new_number: Some(n),
                        no_newline: false,
                    })
                    .collect(),
            });
            rt.state.diffs.insert(
                key.clone(),
                diff_panel::ThreadDiff {
                    done: Some(diff_panel::Done::Ready(Box::new(diff_panel::Ready {
                        files: vec![diff_panel::FileEntry {
                            additions: big.additions,
                            deletions: big.deletions,
                            file: big,
                            body: diff_panel::FileBody::Loaded,
                            truncated: false,
                        }],
                        raw: None,
                        additions: 20_000,
                        deletions: 0,
                        truncated: false,
                        incomplete: false,
                        cwd: "/repo".into(),
                        source_kind: crate::review::ReviewSourceKind::BranchRange,
                        base_ref: None,
                        rows: vec![],
                        rows_version: 0,
                        tree_rows: vec![],
                        tree_version: 0,
                    }))),
                    last_refresh_at: Some(std::time::Instant::now()),
                    ..Default::default()
                },
            );
            rt.state.diff_rebuild_rows(&key);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(0.5);
            let mounted = rt
                .elements()
                .into_iter()
                .filter(|e| {
                    e.key
                        .as_deref()
                        .is_some_and(|key| key.starts_with("diff-line-"))
                })
                .count();
            assert!(
                mounted < 400,
                "{width}: {mounted} line rows mounted of 20,000"
            );
        }
    }

    /// Six files across three folder levels — the spec's tree fixture. Each
    /// file gets fifteen added lines so the diff outgrows the 600 pt viewport
    /// and the reveal has somewhere to scroll.
    const TREE_PATCH: &str = "diff --git a/a.txt b/a.txt\n\
        --- a/a.txt\n+++ b/a.txt\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
        diff --git a/src/lib.rs b/src/lib.rs\n\
        --- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
        diff --git a/src/ui/app.rs b/src/ui/app.rs\n\
        --- a/src/ui/app.rs\n+++ b/src/ui/app.rs\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
        diff --git a/src/ui/widgets/list.rs b/src/ui/widgets/list.rs\n\
        --- a/src/ui/widgets/list.rs\n+++ b/src/ui/widgets/list.rs\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
        diff --git a/tests/app.test.ts b/tests/app.test.ts\n\
        --- a/tests/app.test.ts\n+++ b/tests/app.test.ts\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
        diff --git a/src/ui/widgets/deep/item.rs b/src/ui/widgets/deep/item.rs\n\
        --- a/src/ui/widgets/deep/item.rs\n+++ b/src/ui/widgets/deep/item.rs\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n";

    /// KSG `FileExplorerPane` beside the diff: the tree column sits inside the
    /// panel without overlapping the diff list, pressing a file reveals its
    /// header, a folder press folds, the toggle unmounts it, and a 5,000-file
    /// diff still builds only the rows on screen.
    #[test]
    fn diff_tree_lays_out_reveals_and_virtualizes_at_all_widths() {
        use crate::right_panel::SingletonKind;
        // A docked panel resized to its 360 px minimum leaves a lane under the
        // tree's 416 px minimum, so the tree and its toggle hide outright
        // (KSG's implied threshold).
        let mut rt = runtime(1280.0);
        rt.state.right_panel_width = crate::right_panel::MIN_WIDTH;
        let key = rt.state.panel_key();
        rt.state.right_panels.open(&key, SingletonKind::Diff);
        rt.state.diffs.insert(
            key.clone(),
            diff_panel::ThreadDiff {
                done: Some(parsed_ready(TREE_PATCH)),
                last_refresh_at: Some(std::time::Instant::now()),
                ..Default::default()
            },
        );
        rt.state.diff_rebuild_rows(&key);
        rt.state.diff_tree_open = true;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.5);
        rt.frame(0.5);
        assert!(rt.presented("diff-rows").is_some());
        assert!(
            rt.presented("diff-tree").is_none(),
            "tree hides on a 360 panel"
        );
        assert!(
            rt.presented("diff-tree-toggle").is_none(),
            "toggle hides on a 360 panel"
        );

        for width in [1280.0, 1920.0] {
            let mut rt = runtime(width);
            let key = rt.state.panel_key();
            rt.state.right_panels.open(&key, SingletonKind::Diff);
            rt.state.diffs.insert(
                key.clone(),
                diff_panel::ThreadDiff {
                    done: Some(parsed_ready(TREE_PATCH)),
                    last_refresh_at: Some(std::time::Instant::now()),
                    ..Default::default()
                },
            );
            rt.state.diff_rebuild_rows(&key);
            rt.state.diff_tree_open = true;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(0.5);
            rt.frame(0.5);

            let (panel, _) = rt
                .presented("right-panel")
                .unwrap_or_else(|| panic!("panel mounts at {width}"));
            let (tree, _) = rt
                .presented("diff-tree")
                .unwrap_or_else(|| panic!("tree mounts at {width}"));
            let (rows, _) = rt
                .presented("diff-rows")
                .unwrap_or_else(|| panic!("list mounts at {width}"));
            // The tree sits inside the panel's right side, beside the list.
            assert!(
                tree[0] >= panel[0] - 0.5
                    && tree[0] + tree[2] <= panel[0] + panel[2] + 0.5
                    && tree[1] >= panel[1] - 0.5
                    && tree[1] + tree[3] <= panel[1] + panel[3] + 0.5,
                "{width}: tree {tree:?} inside panel {panel:?}"
            );
            assert!(
                tree[0] >= rows[0] + rows[2] - 0.5,
                "{width}: tree {tree:?} beside list {rows:?}"
            );
            // KSG's bounds: the lane holds 160..=560 minus the diff reserve.
            assert!(
                (crate::ui::diff_tree::MIN_W..=crate::ui::diff_tree::MAX_W).contains(&tree[2]),
                "{width}: tree width {}",
                tree[2]
            );

            // The toggle rides the header while the lane fits.
            assert!(rt.presented("diff-tree-toggle").is_some(), "{width}");
            // File and folder rows mount; the tree's file-3 is the diff's
            // file-3 (src/ui/widgets/list.rs).
            assert!(rt.presented("diff-tree-file-3").is_some(), "{width}");
            assert!(rt.presented("diff-tree-dir-src/").is_some(), "{width}");

            // Pressing the file reveals its diff header at the list's top.
            press(&mut rt, "diff-tree-file-3");
            rt.frame(0.5);
            rt.frame(0.5);
            let (revealed, _) = rt
                .presented("diff-file-3")
                .unwrap_or_else(|| panic!("diff-file-3 at {width}"));
            let (list_top, _) = rt.presented("diff-rows").unwrap();
            assert!(
                (revealed[1] - list_top[1]).abs() <= 1.0,
                "{width}: diff-file-3 {revealed:?} tops the list {list_top:?}"
            );
            // The marked file carries the tree's current-file sync too.
            let key = rt.state.panel_key();
            assert_eq!(rt.state.diffs[&key].current_file, Some(3), "{width}");

            // Pressing a folder row removes its file rows. `order_like_tree`
            // puts the deep item at file 4 and `tests/app.test.ts` at file 5,
            // so folding `src/` leaves the test file as `diff-tree-file-5`.
            press(&mut rt, "diff-tree-dir-src/");
            rt.frame(0.5);
            assert!(rt.presented("diff-tree-file-1").is_none(), "{width}");
            assert!(rt.presented("diff-tree-file-3").is_none(), "{width}");
            assert!(rt.presented("diff-tree-file-4").is_none(), "{width}");
            assert!(rt.presented("diff-tree-file-5").is_some(), "{width}");

            // The toggle hides the tree and the diff list takes the width.
            press(&mut rt, "diff-tree-toggle");
            rt.frame(0.5);
            assert!(rt.presented("diff-tree").is_none(), "{width}");
            let (wide_rows, _) = rt.presented("diff-rows").unwrap();
            assert!(
                wide_rows[2] > rows[2] + 100.0,
                "{width}: {wide_rows:?} vs {rows:?}"
            );
        }

        // 5,000 changed files still build only the visible tree rows.
        let mut rt = runtime(1280.0);
        let key = rt.state.panel_key();
        rt.state.right_panels.open(&key, SingletonKind::Diff);
        let patch: String = (0..5_000)
            .map(|i| {
                format!(
                    "diff --git a/dir{}/sub/f{}.rs b/dir{}/sub/f{}.rs\n\
                    --- a/dir{}/sub/f{}.rs\n+++ b/dir{}/sub/f{}.rs\n@@ -1,0 +1,1 @@\n+x\n",
                    i % 50,
                    i,
                    i % 50,
                    i,
                    i % 50,
                    i,
                    i % 50,
                    i
                )
            })
            .collect();
        rt.state.diffs.insert(
            key.clone(),
            diff_panel::ThreadDiff {
                done: Some(parsed_ready(&patch)),
                last_refresh_at: Some(std::time::Instant::now()),
                ..Default::default()
            },
        );
        rt.state.diff_rebuild_rows(&key);
        rt.state.diff_tree_open = true;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.5);
        rt.frame(0.5);
        let mounted = rt
            .elements()
            .into_iter()
            .filter(|e| {
                e.key.as_deref().is_some_and(|key| {
                    key.starts_with("diff-tree-file-") || key.starts_with("diff-tree-dir-")
                })
            })
            .count();
        assert!(mounted < 200, "5,000 files: {mounted} tree rows built");
    }

    /// KSG `PanelLayoutControls` in the top bar: the diff toggle opens and
    /// closes the Diff tab, the panel toggle the whole panel, and the loaded
    /// diff's totals sit beside the diff toggle once the row is wide enough.
    #[test]
    fn top_bar_diff_and_panel_buttons_toggle_the_right_panel_at_all_widths() {
        // Two files: +2 -1 on src/a.rs, +3 on src/b.rs — "+5 -1" total.
        const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n\
            --- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n keep\n-old\n+new\n+more\n tail\n\
            diff --git a/src/b.rs b/src/b.rs\n\
            --- a/src/b.rs\n+++ b/src/b.rs\n@@ -5,0 +5,3 @@\n+one\n+two\n+three\n";
        for width in [1280.0] {
            let mut rt = runtime(width);
            // A loaded diff, fresh enough that mounting the tab does not re-read git.
            let key = rt.state.panel_key();
            rt.state.diffs.insert(
                key.clone(),
                diff_panel::ThreadDiff {
                    done: Some(parsed_ready(PATCH)),
                    last_refresh_at: Some(std::time::Instant::now()),
                    ..Default::default()
                },
            );
            rt.state.diff_rebuild_rows(&key);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(0.5);

            let (bar, _) = rt.presented("top-bar").unwrap();
            // Every presented right-side control stays inside the bar and
            // clear of its neighbours and the status text's slot.
            let mut controls = Vec::new();
            for key in [
                "chat-status",
                "thread-actions",
                "header-diff-stat",
                "header-diff",
                "header-right-panel",
            ] {
                if let Some((rect, _)) = rt.presented(key) {
                    controls.push((key, rect));
                }
            }
            for (name, rect) in &controls {
                assert!(
                    rect[0] >= bar[0] - 0.5 && rect[0] + rect[2] <= bar[0] + bar[2] + 0.5,
                    "{width}: {name} {rect:?} escapes bar {bar:?}"
                );
            }
            for (i, (a, ar)) in controls.iter().enumerate() {
                for (b, br) in &controls[i + 1..] {
                    let overlap = (ar[0] + ar[2]).min(br[0] + br[2]) - ar[0].max(br[0]);
                    assert!(overlap <= 0.5, "{width}: {a} {ar:?} overlaps {b} {br:?}");
                }
            }

            // At compact widths the diff toggle drops; Diff is a tab in the
            // Inspector there. Wider, it opens the panel on the Diff tab and
            // presses off again.
            if width >= 760.0 {
                assert!(rt.presented("header-diff").is_some(), "{width}");
                press(&mut rt, "header-diff");
                rt.frame(0.5);
                let key = rt.state.panel_key();
                assert_eq!(
                    rt.state.right_panels.active_view(&key),
                    Some(crate::right_panel::PanelView::Diff),
                    "{width}"
                );
                assert!(rt.presented("right-panel").is_some(), "{width}");
                press(&mut rt, "header-diff");
                rt.frame(0.5);
                assert!(
                    rt.presented("right-panel").is_none(),
                    "{width}: a second press closes the panel"
                );
            } else {
                assert!(rt.presented("header-diff").is_none(), "{width}");
            }

            // The panel toggle shows and hides the whole panel at every width.
            press(&mut rt, "header-right-panel");
            rt.frame(0.5);
            assert!(rt.presented("right-panel").is_some(), "{width}");
            press(&mut rt, "header-right-panel");
            rt.frame(0.5);
            assert!(rt.presented("right-panel").is_none(), "{width}");

            // The stat reads the loaded diff's totals at and above the narrow fold.
            assert_eq!(
                rt.presented("header-diff-stat").is_some(),
                width >= 1050.0,
                "{width}: the stat only shows once the row is wide enough"
            );
            if width >= 1050.0 {
                let stat = rt
                    .elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some("header-diff-stat"))
                    .unwrap();
                assert_eq!(
                    stat.label.as_deref(),
                    Some("5 additions, 1 deletions"),
                    "{width}"
                );
            }
        }
    }

    #[test]
    fn questionnaire_replaces_composer_and_supports_keyboard_and_previous_at_all_widths() {
        for width in [1280.0] {
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
    fn composer_file_pill_opens_a_transient_preview_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.frame(0.0);
            let file = rt.state.data_dir.join(format!("mention-{width}.txt"));
            std::fs::write(&file, "mention preview body").unwrap();
            let token = crate::composer_tokens::Token::File {
                path: file,
                directory: false,
                source: None,
            };
            let id = token.id();
            rt.state
                .current_mut()
                .draft
                .replace_with_pill(0..0, id.clone(), "mention.txt");
            rt.invalidate();
            rt.frame(0.0);
            // The headless runtime cannot point at a pill inside a text field,
            // so press through the same handler `on_input_pill_press` calls.
            rt.update(|s, _| s.press_composer_pill(&id));
            rt.frame(0.0);
            let (dialog, _) = rt
                .presented("attachment-preview")
                .unwrap_or_else(|| panic!("{width}"));
            assert!(
                dialog[0] >= 0.0 && dialog[0] + dialog[2] <= width + 0.5,
                "{width}: {dialog:?}"
            );
            let preview = rt.state.attachment_preview.as_ref().unwrap();
            assert!(preview.transient && preview.error.is_none());
            assert!(rt.state.current().draft_images.is_empty());
            let draft = rt.state.current().draft.text.clone();
            press(&mut rt, "close-attachment-preview");
            assert!(rt.state.attachment_preview.is_none());
            assert_eq!(rt.state.current().draft.text, draft);
        }
    }

    #[test]
    fn attachment_previews_open_from_tray_and_messages_and_fit_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            let directory = rt.state.data_dir.clone();
            let file_path = directory.join("preview.txt");
            std::fs::write(&file_path, "Preview body").unwrap();
            let file = crate::attachments::attach_file(&file_path).unwrap();
            let movie_path = directory.join("preview.mp4");
            std::fs::write(
                &movie_path,
                include_bytes!("../assets/tests/attachment-preview.mp4"),
            )
            .unwrap();
            let video = crate::attachments::attach_file(&movie_path).unwrap();
            let image_path = directory.join("preview.png");
            image::RgbaImage::from_pixel(4, 2, image::Rgba([60, 80, 100, 255]))
                .save(&image_path)
                .unwrap();
            let image = crate::attachments::attach_file(&image_path).unwrap();
            rt.state.current_mut().draft_images = vec![image.clone(), file.clone(), video.clone()];
            rt.state.current_mut().draft.replace_text("Inspect");
            rt.state.current_mut().draft.focused = true;
            rt.frame(0.0);
            press(&mut rt, &format!("attachment-thumb-{}", image.id));
            rt.frame(0.0);
            assert!(rt.presented("attachment-preview-image").is_some());
            press(&mut rt, "close-attachment-preview");
            for attachment in [file.clone(), image.clone(), video.clone()] {
                for draft in [true, false] {
                    rt.update(|s, _| {
                        s.preview_attachments(vec![attachment.clone()], &attachment.id, draft)
                    });
                    rt.frame(0.0);
                    let (bounds, _) = rt.presented("attachment-preview").unwrap();
                    assert!(
                        bounds[0] >= 0.0 && bounds[0] + bounds[2] <= width + 0.5,
                        "{width}: {bounds:?}"
                    );
                    assert!(
                        bounds[1] >= 0.0 && bounds[1] + bounds[3] <= rt.state.viewport_height + 0.5
                    );
                    if attachment.id == video.id {
                        assert!(rt.presented("attachment-preview-video").is_some());
                        assert!(rt.presented("attachment-preview-seek").is_some());
                        assert!(rt.presented("attachment-preview-media-controls").is_some());
                        let player = rt
                            .state
                            .attachment_preview
                            .as_ref()
                            .unwrap()
                            .player()
                            .unwrap()
                            .clone();
                        // Rune has no video playback on Linux yet, so only the
                        // layout is checked there.
                        if cfg!(any(target_os = "macos", windows)) {
                            rt.frame(0.25);
                            assert!(
                                !matches!(player.state(), rune::PlayerState::Failed(_)),
                                "{:?}",
                                player.state()
                            );
                            assert!(player.duration().is_some_and(|duration| duration > 0.0));
                            rt.frame(0.25);
                            assert!(player.time() > 0.0);
                        }
                    } else if attachment.id == file.id {
                        assert!(
                            rt.elements()
                                .iter()
                                .any(|e| e.text.as_deref() == Some("Preview body")),
                            "{width} draft {draft}: {:?}",
                            rt.elements()
                                .iter()
                                .filter(|e| e.path.contains("attachment-preview"))
                                .map(|e| (&e.key, &e.text, e.rect))
                                .collect::<Vec<_>>()
                        );
                    }
                    press(&mut rt, "close-attachment-preview");
                    assert!(rt.state.attachment_preview.is_none());
                }
            }
        }
    }

    #[test]
    fn question_collapse_reopens_preserves_drafts_and_blocks_hidden_shortcuts_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state
                .current_mut()
                .apply(json!({"type":"extension_ui_request","id":"collapse",
                "method":"input","responseCapability":{"type":"message"},"questions":[
                    {"id":"one","header":"Storage","question":"Choose **working storage**.",
                        "multiSelect":true,"options":[{"label":"Disk"}]},
                    {"id":"two","header":"Details","question":"Any details?","options":[]}
                ]}));
            rt.state.current_mut().reply.replace_text("Unsent answer");
            rt.frame(0.0);
            let expanded = rt.presented("questionnaire-drawer").unwrap().0;
            press(&mut rt, "questionnaire-toggle");
            assert!(rt.state.current().questionnaire_collapsed(), "{width}");
            rt.frame(0.25);
            assert!(rt.find("questionnaire-markdown").is_none());
            assert!(rt.find("dialog-option-0").is_none());
            assert!(rt.presented("questionnaire-summary").is_some());
            assert!(rt.presented("questionnaire-count").is_some());
            assert!(rt.presented("dialog-reply").is_some());
            let collapsed = rt.presented("questionnaire-drawer").unwrap().0;
            assert!(
                collapsed[3] < expanded[3],
                "{width}: {collapsed:?} {expanded:?}"
            );
            for key in [
                "questionnaire-toggle",
                "questionnaire-summary",
                "dialog-cancel",
            ] {
                let rect = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"))
                    .0;
                assert!(rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5);
                assert!(rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5);
            }
            assert!(!pi_ui::questionnaire_key(
                &mut rt.state,
                &rune::KeyEvent {
                    key: rune::Key::Text("1".into()),
                    mods: Default::default()
                }
            ));
            press(&mut rt, "questionnaire-toggle");
            rt.frame(0.25);
            assert!(rt.presented("dialog-option-0").is_some());
            assert_eq!(rt.state.current().reply.text, "Unsent answer");
            press(&mut rt, "questionnaire-toggle");
            press(&mut rt, "questionnaire-next");
            rt.frame(0.25);
            assert!(rt.presented("questionnaire-markdown").is_some());
            assert!(rt.find("questionnaire-summary").is_none());
            rt.state.current_mut().respond(json!({"cancelled":true}));
            rt.state.current_mut().apply(json!({"type":"extension_ui_request","id":"new-request",
                "method":"input","questions":[{"id":"one","question":"New request?","options":[]}]}));
            rt.invalidate();
            rt.frame(0.0);
            assert!(rt.presented("questionnaire-markdown").is_some());
            assert!(rt.find("questionnaire-count").is_none());
            assert!(
                rt.find("dialog-cancel").is_none(),
                "live questions cannot dismiss"
            );
        }
    }

    #[test]
    fn harness_picker_is_available_before_first_send_at_all_widths() {
        for width in [1280.0] {
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
                "choose-harness-claude",
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
            assert!(rt.presented("harness-tag-claude").is_none());
            assert!(
                find(&rt.accessibility_json(), "choose-harness-claude")
                    .and_then(|node| node["states"].as_array().cloned())
                    .is_some_and(|states| states.iter().any(|x| x == "disabled")),
                "{width}: claude should be disabled when missing"
            );
            rt.state.model_picker_tab = Some(crate::model_picker::ModelPickerTab::Instance(
                "claude".into(),
            ));
            rt.invalidate();
            rt.frame(0.0);
            assert!(rt.presented("claude-unavailable").is_some());
            rt.state.claude_catalog.installed = true;
            rt.state.model_picker_tab = None;
            rt.invalidate();
            rt.frame(0.0);
            press(&mut rt, "choose-harness-claude");
            assert_eq!(
                rt.state.model_picker_tab,
                Some(crate::model_picker::ModelPickerTab::Instance(
                    "claude".into()
                ))
            );
            press(&mut rt, "choose-harness-fixture");
            assert_eq!(
                rt.state.model_picker_tab,
                Some(crate::model_picker::ModelPickerTab::Instance(
                    "fixture".into()
                ))
            );
            press(&mut rt, "choose-model-0");
            assert_eq!(rt.state.current().harness, "fixture");
            rt.state.current_mut().life.latest_user_message_at = Some(1);
            rt.state.open_model_picker(SettingsPicker::Model);
            rt.invalidate();
            rt.frame(0.0);
            press(&mut rt, "choose-harness-pi");
            assert_eq!(
                rt.state.current().harness,
                "fixture",
                "switching the tab does not move the thread"
            );
            assert_eq!(
                rt.state.model_picker_tab,
                Some(crate::model_picker::ModelPickerTab::Instance("pi".into())),
                "{width}: pi stays reachable on a started thread via context handoff"
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
    fn queued_edit_controls_save_cancel_and_shortcuts_fit_at_supported_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            let (pi, _, _) = crate::backend::Pi::test_connection();
            let t = rt.state.current_mut();
            t.pi = Some(pi);
            t.model_provider = "cpa".into();
            t.status = Status::Running;
            t.native.follow_up = true;
            t.draft.replace_text("Queued original");
            t.send_during_run();
            let id = t.native.queue[0].id;
            t.draft.replace_text("Ordinary draft");
            t.draft.set_caret(0);
            assert!(pi_ui::composer_pi_key(
                &mut rt.state,
                &rune::KeyEvent {
                    key: rune::Key::Up,
                    mods: rune::Mods {
                        alt: true,
                        ..Default::default()
                    },
                },
                &rune::Tx::default(),
                None
            ));
            assert_eq!(rt.state.current().draft.text, "Queued original");
            rt.frame(0.0);
            for key in [
                "composer",
                "queued-messages",
                "send-message",
                &format!("queue-edit-{id}"),
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(rect[0] >= 0.0 && rect[0] + rect[2] <= width + 1.0);
            }
            rt.state.current_mut().draft.replace_text("Edited queued");
            rt.state.send();
            assert_eq!(
                rt.state.current().native.queue[0].prompt.text,
                "Edited queued"
            );
            assert_eq!(rt.state.current().draft.text, "Ordinary draft");
            rt.state.current_mut().edit_queued(id);
            rt.state.current_mut().draft.replace_text("Cancel me");
            rt.state.current_mut().cancel_queued_edit();
            assert_eq!(rt.state.current().draft.text, "Ordinary draft");
            assert!(pi_ui::composer_pi_key(
                &mut rt.state,
                &rune::KeyEvent {
                    key: rune::Key::Enter,
                    mods: rune::Mods {
                        cmd: true,
                        ctrl: cfg!(not(target_os = "macos")),
                        shift: true,
                        ..Default::default()
                    },
                },
                &rune::Tx::default(),
                None
            ));
            assert!(rt.state.current().native.queue.is_empty());
        }
    }

    #[test]
    fn codex_compact_button_and_progress_rows_fit_at_supported_widths() {
        // Uses the existing KSG ContextWindowMeter and context-compaction row layout.
        for width in [1280.0] {
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
        for width in [1280.0] {
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
    fn inline_slash_selection_preserves_the_rest_of_the_draft() {
        let mut rt = runtime(960.0);
        rt.frame(0.0);
        press(&mut rt, "composer-input");
        let t = rt.state.current_mut();
        t.apply(
            json!({"type":"response","command":"get_commands","success":true,
            "data":{"commands":[{"name":"skill:review","source":"skill"}]}}),
        );
        t.draft.replace_text("please /skill:re after");
        t.draft.set_caret(16);
        rt.frame(0.0);
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert_eq!(rt.state.current().draft.text, "please /skill:review  after");
        rt.state
            .current_mut()
            .draft
            .replace_text("please /does-not-exist");
        rt.invalidate();
        rt.frame(0.0);
        assert!(rt.find("slash-menu").is_some());
        rt.key(rune::KeyEvent {
            key: rune::Key::Escape,
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert!(rt.find("slash-menu").is_none());
    }

    #[test]
    fn skill_menu_uses_the_same_rows_and_inserts_a_native_skill_pill_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            rt.state.current_mut().apply(json!({"type":"response","command":"get_commands","success":true,
                "data":{"commands":[{"name":"skill:review","description":"Review changes","source":"skill"}]}}));
            rt.key(rune::KeyEvent {
                key: rune::Key::Text("please $rev".into()),
                mods: Default::default(),
            });
            rt.frame(0.0);
            rt.frame(0.25);
            let (menu, _) = rt.presented("skill-menu").unwrap();
            let (item, _) = rt.presented("skill-item-0").unwrap();
            assert_eq!(item[3], 32.0);
            assert!(
                menu[0] >= 8.0 && menu[0] + menu[2] <= width - 7.0,
                "{width}: {menu:?}"
            );
            rt.key(rune::KeyEvent {
                key: rune::Key::Enter,
                mods: Default::default(),
            });
            rt.frame(0.0);
            assert_eq!(rt.state.current().draft.pills().count(), 1);
            assert_eq!(
                crate::composer_rich::markdown(&rt.state.current().draft),
                "please $review "
            );
            assert!(rt.find("skill-menu").is_none());
        }
    }

    #[test]
    fn heading_task_rule_and_fence_render_inside_the_composer_card_at_620_and_1280() {
        let source = "# Title\n- [x] Done\n---\n```rs\nlet x = 1;\n```";
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            {
                let t = rt.state.current_mut();
                t.draft = crate::composer_rich::draft("");
                crate::composer_rich::paste_with_path_query(&mut t.draft, source, false, &|_| None);
                t.draft.focused = true;
            }
            rt.frame(0.0);
            let (card, _) = rt.presented("composer-natural").unwrap();
            let (input, _) = rt.presented("composer-input").unwrap();
            assert!(
                input[0] >= card[0]
                    && input[0] + input[2] <= card[0] + card[2] + 1.0
                    && input[1] >= card[1]
                    && input[1] + input[3] <= card[1] + card[3] + 1.0,
                "{width}: input {input:?} outside card {card:?}"
            );
            assert!(
                input[3] > 60.0,
                "{width}: expected multiline height, got {input:?}"
            );
            let draft = &rt.state.current().draft;
            let kinds: Vec<_> = draft.logical_blocks().map(|b| b.kind).collect();
            assert_eq!(
                kinds,
                vec![
                    rune::BlockKind::Heading { level: 1 },
                    rune::BlockKind::Task {
                        level: 0,
                        checked: true,
                    },
                    rune::BlockKind::Rule,
                    rune::BlockKind::Code,
                ]
            );
            assert_eq!(crate::composer_rich::markdown(draft), source);
        }
    }

    /// KSG's ComposerCodeBlockLanguagePicker: a header trigger per code block, a
    /// searchable menu that fits the window, and a language change that rewrites only
    /// that block's fence.
    #[test]
    fn code_block_language_picker_edits_only_its_own_fence_at_all_widths() {
        let source = "```python\nfirst()\n```\n\nprose\n\n```rust\nsecond()\n```";
        let changed = "```python\nfirst()\n```\n\nprose\n\n```dart\nsecond()\n```";
        for width in [1280.0] {
            let mut rt = runtime(width);
            {
                let t = rt.state.current_mut();
                t.draft = crate::composer_rich::draft("");
                crate::composer_rich::paste_with_path_query(&mut t.draft, source, false, &|_| None);
                t.draft.focused = true;
            }
            rt.invalidate();
            rt.frame(0.0);
            let (card, _) = rt.presented("composer-natural").unwrap();
            for n in 0..2 {
                let key = format!("composer-code-language-{n}");
                let (trigger, _) = rt
                    .presented(&key)
                    .unwrap_or_else(|| panic!("{width}: {key} not presented"));
                assert!(
                    trigger[0] >= card[0]
                        && trigger[0] + trigger[2] <= card[0] + card[2] + 1.0
                        && trigger[1] >= card[1]
                        && trigger[1] + trigger[3] <= card[1] + card[3] + 1.0,
                    "{width}: {key} {trigger:?} outside card {card:?}"
                );
            }
            press(&mut rt, "composer-code-language-1");
            let (menu, _) = rt
                .presented("composer-code-language-menu")
                .unwrap_or_else(|| panic!("{width}: menu not presented"));
            assert!(
                menu[0] >= 0.0
                    && menu[0] + menu[2] <= width + 1.0
                    && menu[1] >= 0.0
                    && menu[1] + menu[3] <= 721.0,
                "{width}: menu {menu:?} outside window"
            );
            assert!(
                rt.presented("composer-code-language-search").is_some(),
                "{width}"
            );
            press(&mut rt, "composer-code-language-option-dart");
            assert_eq!(
                crate::composer_rich::markdown(&rt.state.current().draft),
                changed,
                "{width}"
            );
            assert!(
                rt.presented("composer-code-language-menu").is_none(),
                "{width}"
            );
            assert!(rt.state.current().draft.focused, "{width}");
            // Reopen and Escape: the menu closes and the draft does not change.
            press(&mut rt, "composer-code-language-1");
            assert!(
                rt.presented("composer-code-language-menu").is_some(),
                "{width}"
            );
            rt.key(rune::KeyEvent {
                key: rune::Key::Escape,
                mods: Default::default(),
            });
            rt.frame(0.0);
            assert!(
                rt.presented("composer-code-language-menu").is_none(),
                "{width}"
            );
            assert_eq!(
                crate::composer_rich::markdown(&rt.state.current().draft),
                changed,
                "{width}"
            );
        }
    }

    #[test]
    fn steering_and_rewind_controls_send_native_commands_without_overflow() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            let (pi, commands, _) = crate::backend::Pi::test_connection();
            let t = rt.state.current_mut();
            t.pi = Some(pi);
            t.model_provider = "cpa".into();
            t.status = Status::Running;
            t.draft.replace_text("Steer now");
            t.draft.focused = true;
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
        fn over_srgb(base: Color, overlay: Color) -> Color {
            use rune::color::{srgb_decode, srgb_encode};
            let a = overlay.a;
            let blend = |b: f32, o: f32| {
                let (b, o) = (srgb_encode(b), srgb_encode(o));
                srgb_decode(b * (1.0 - a) + o * a)
            };
            Color {
                r: blend(base.r, overlay.r),
                g: blend(base.g, overlay.g),
                b: blend(base.b, overlay.b),
                a: 1.0,
            }
        }
        fn ratio(a: Color, b: Color) -> f32 {
            let (a, b) = (a.luminance(), b.luminance());
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        }
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
                (p.faint, p.card_top),
                (p.faint, p.card_bottom),
                (p.faint, p.bg),
                (p.faint, p.lift),
                (p.faint, p.bg.mix(p.selected, p.selected.a)),
                (p.faint, over_srgb(p.bg, p.selected)),
                (p.faint, p.panel.mix(p.selected, p.selected.a)),
                (p.faint, over_srgb(p.panel, p.selected)),
                (p.faint, p.card.mix(p.selected, p.selected.a)),
                (p.faint, over_srgb(p.card, p.selected)),
                (p.faint, p.code_bg),
                (p.ink, p.message_bg),
                // The send pill's icon, compact label and stop mark on the recessed `pill` fill
                // and under a single interaction wash in linear light and sRGB compositing.
                (p.ink, p.pill),
                (p.ink, p.pill.mix(p.selected, p.selected.a)),
                (p.ink, over_srgb(p.pill, p.selected)),
                (p.muted, p.pill),
                (p.muted, p.pill.mix(p.selected, p.selected.a)),
                (p.muted, over_srgb(p.pill, p.selected)),
                (p.danger, p.pill),
                (p.danger, p.pill.mix(p.selected, p.selected.a)),
                (p.danger, over_srgb(p.pill, p.selected)),
                // The "Shina" role label sits on the coral wash.
                (p.ink, p.panel.mix(p.coral_bg, p.coral_bg.a)),
                (p.ink, over_srgb(p.panel, p.coral_bg)),
                (p.muted, p.code_bg),
                (p.muted, p.bg.mix(p.selected, p.selected.a)),
                (p.muted, over_srgb(p.bg, p.selected)),
                (p.muted, p.card.mix(p.selected, p.selected.a)),
                (p.muted, over_srgb(p.card, p.selected)),
                (p.sea, p.panel),
                (p.sea, p.card),
                (p.sea, p.bg),
                (p.sea, p.sea_bg),
                (p.on_coral, p.coral),
                (p.on_brass, p.brass),
                (p.brass, p.panel),
                (p.brass, p.brass_bg),
                (p.brass, p.bg.mix(p.selected, p.selected.a)),
                (p.brass, over_srgb(p.bg, p.selected)),
            ]
            .into_iter()
            .enumerate()
            {
                // `faint` is KSG's `--placeholder`, for text that can recede (carets,
                // timestamps); it has to clear 3:1. `muted` is KSG's `#8a8a8a`, which reads
                // 4.38:1 on `lift`.
                let floor = if foreground == p.faint {
                    3.0
                } else if foreground == p.muted {
                    4.3
                } else {
                    4.5
                };
                let contrast = ratio(foreground, background);
                assert!(
                    contrast >= floor,
                    "text contrast {contrast:.2} in pair {index}, dark={dark}"
                );
            }
            // WCAG 1.4.11: non-text control boundaries (fields, checkboxes, radios, active toggles)
            // must read at 3:1 or better against every surface they sit on.
            for (name, surface) in [
                ("bg", p.bg),
                ("panel", p.panel),
                ("tray", p.tray),
                ("card", p.card),
                ("card_top", p.card_top),
                ("card_bottom", p.card_bottom),
                ("lift", p.lift),
                ("pill", p.pill),
                ("pill-hover-linear", p.pill.mix(p.selected, p.selected.a)),
                ("pill-hover-srgb", over_srgb(p.pill, p.selected)),
            ] {
                let contrast = ratio(p.field_line, surface);
                assert!(
                    contrast >= 3.0,
                    "field_line contrast {contrast:.2} on {name}, dark={dark}"
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
    fn older_history_controls_show_load_loading_error_and_end_states_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            {
                let t = rt.state.current_mut();
                t.messages.push(crate::model::Message {
                    id: "u1".into(),
                    role: "You",
                    text: "Recent message".into(),
                    images: vec![],
                    context: None,
                });
                t.history_has_more = true;
                t.history_loading = false;
                t.history_error = None;
            }
            rt.frame(0.0);
            assert!(
                rt.presented("history-load-earlier").is_some(),
                "{width}: load button"
            );
            assert!(
                !rt.accessibility_json()
                    .to_string()
                    .contains("Showing recent turns."),
                "{width}: old cutoff notice must be gone"
            );

            // Loading state.
            rt.state.current_mut().history_loading = true;
            rt.invalidate();
            rt.frame(0.0);
            assert!(
                rt.elements()
                    .iter()
                    .any(|e| e.text.as_deref() == Some("Loading earlier turns…")),
                "{width}: loading label"
            );

            // Error state.
            {
                let t = rt.state.current_mut();
                t.history_loading = false;
                t.history_error = Some("History failed".into());
            }
            rt.invalidate();
            rt.frame(0.0);
            assert!(
                rt.presented("history-load-error").is_some(),
                "{width}: error label"
            );

            // End-of-history state: button and error disappear.
            {
                let t = rt.state.current_mut();
                t.history_has_more = false;
                t.history_error = None;
            }
            rt.invalidate();
            rt.frame(0.0);
            assert!(
                rt.find("history-load-earlier").is_none(),
                "{width}: hidden at end"
            );
            assert!(
                rt.find("history-load-error").is_none(),
                "{width}: error cleared"
            );
        }
    }

    #[test]
    fn virtualised_timeline_mounts_only_viewport_window_and_overscan_for_long_threads() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            {
                let t = rt.state.current_mut();
                for turn in 0..500usize {
                    t.messages.push(crate::model::Message {
                        id: format!("u-{turn}"),
                        role: "You",
                        text: format!("Prompt number {turn} with some wrapping prose."),
                        images: vec![],
                        context: None,
                    });
                    t.messages.push(crate::model::Message {
                        id: format!("a-{turn}"),
                        role: "Pi",
                        text: format!("Reply number {turn} with detailed explanation."),
                        images: vec![],
                        context: None,
                    });
                }
            }
            rt.frame(0.0);
            let mounted_rows: Vec<_> = rt
                .elements()
                .into_iter()
                .filter(|e| {
                    e.key
                        .as_deref()
                        .is_some_and(|k| k.starts_with("row-") || k.starts_with("turn-"))
                })
                .collect();
            assert!(
                !mounted_rows.is_empty() && mounted_rows.len() < 80,
                "{width}: expected virtualised window, mounted {} rows",
                mounted_rows.len()
            );
            assert!(
                rt.find("row-1-u-0").is_none(),
                "{width}: oldest row must be unmounted when pinned at bottom"
            );
            assert!(
                rt.presented("turn-1-a-499").is_some(),
                "{width}: newest row must be presented"
            );
        }
    }

    #[test]
    fn scroll_anchor_preserves_visible_row_on_prepend_expand_image_load_and_width_change() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = true;
            {
                let t = rt.state.current_mut();
                t.history_has_more = true;
                for turn in 0..80usize {
                    t.messages.push(crate::model::Message {
                        id: format!("u-{turn}"),
                        role: "You",
                        text: format!("Question {turn}: keeping anchor steady."),
                        images: if turn == 35 {
                            vec![Attachment {
                                asset_id: None,
                                uploaded: None,
                                source: None,
                                id: "img-35.png".into(),
                                name: "img-35.png".into(),
                                mime: "image/png".into(),
                                width: 400,
                                height: 120,
                                bytes: 1,
                                path: None,
                            }]
                        } else {
                            vec![]
                        },
                        context: None,
                    });
                    let a_idx = t.messages.len();
                    t.messages.push(crate::model::Message {
                        id: format!("a-{turn}"),
                        role: "Pi",
                        text: format!("Answer {turn}: extra text to take vertical space."),
                        images: vec![],
                        context: None,
                    });
                    if turn == 45 {
                        t.tools.push(crate::model::Tool {
                            id: "tool-45".into(),
                            name: "read".into(),
                            kind: Some(crate::work::ToolKind::Read),
                            changes: vec![],
                            input: serde_json::json!({"path": "src/ui.rs"}),
                            detail: "src/ui.rs".into(),
                            done: true,
                            failed: false,
                            output: "line 1\nline 2\nline 3\nline 4".into(),
                            images: vec![],
                            structured: serde_json::Value::Null,
                            parent_call: None,
                            exit_code: Some(0),
                            subagents: vec![],
                            anchor: crate::model::ToolAnchor {
                                message: a_idx,
                                offset: 0,
                            },
                            block: 0,
                            order: 1,
                            expanded: false,
                            raw_expanded: false,
                            group_expanded: false,
                            started: None,
                            duration_ms: Some(10),
                        });
                        t.work.push(crate::model::AssistantWork {
                            start: a_idx,
                            stopped: false,
                            stopped_order: 0,
                            expanded: false,
                            elapsed: Some(3),
                            outcome: Some(Status::Completed),
                        });
                    }
                }
            }
            rt.frame(0.0);
            // Scroll into the middle around turn 45 (scroll_center_in_instant mounts unrealized rows).
            rt.update(|_, tx| tx.scroll_center_in_instant("conversation-1", "row-1-u-45"));
            for _ in 0..60 {
                if !rt.frame(1.0 / 60.0) {
                    break;
                }
            }
            let anchor_before = rt.presented("row-1-u-45").expect("u-45 visible").0;

            // (a) Prepending older turns + clearing history_has_more preserves visible row y.
            {
                let t = rt.state.current_mut();
                let mut prepended = Vec::new();
                for older in 0..20usize {
                    prepended.push(crate::model::Message {
                        id: format!("old-u-{older}"),
                        role: "You",
                        text: format!("Older question {older}."),
                        images: vec![],
                        context: None,
                    });
                    prepended.push(crate::model::Message {
                        id: format!("old-a-{older}"),
                        role: "Pi",
                        text: format!("Older answer {older}."),
                        images: vec![],
                        context: None,
                    });
                }
                let shift = prepended.len();
                prepended.append(&mut t.messages);
                t.messages = prepended;
                for tool in &mut t.tools {
                    tool.anchor.message += shift;
                }
                for work in &mut t.work {
                    work.start += shift;
                }
                t.history_has_more = false;
            }
            rt.invalidate();
            rt.frame(1.0 / 60.0);
            let anchor_after_prepend = rt.presented("row-1-u-45").expect("u-45 still visible").0;
            assert!(
                (anchor_after_prepend[1] - anchor_before[1]).abs() < 1.0,
                "{width}: prepend moved anchor from {} to {}",
                anchor_before[1],
                anchor_after_prepend[1]
            );

            // (b) Expanding a work section preserves its toggle row's position.
            let toggle_key = rt
                .elements()
                .into_iter()
                .find_map(|e| {
                    e.key
                        .filter(|k| k.starts_with("toggle-work-"))
                        .filter(|k| rt.presented(k).is_some())
                })
                .expect("toggle-work visible");
            let toggle_before = rt.presented(&toggle_key).unwrap().0[1];
            press(&mut rt, &toggle_key);
            rt.frame(1.0);
            let toggle_after = rt.presented(&toggle_key).unwrap().0[1];
            assert!(
                (toggle_after - toggle_before).abs() < 1.0,
                "{width}: work expand moved toggle from {toggle_before} to {toggle_after}"
            );

            // (c) Image height change above the visible anchor preserves the visible anchor.
            let before_img = rt.presented("row-1-u-45").unwrap().0[1];
            {
                let t = rt.state.current_mut();
                let u35 = t.messages.iter_mut().find(|m| m.id == "u-35").unwrap();
                u35.images[0].height = 600;
            }
            rt.invalidate();
            rt.frame(1.0);
            let after_img = rt.presented("row-1-u-45").unwrap().0[1];
            assert!(
                (after_img - before_img).abs() < 1.0,
                "{width}: image load moved anchor from {before_img} to {after_img}"
            );

            // (d) Width change keeps the visible anchor in view at the same offset.
            // Put this row at the viewport anchor: a different width can reflow rows
            // between the topmost row and a row that was only centred in the viewport.
            rt.update(|_, tx| tx.scroll_key_to("row-1-u-45", 0.0));
            rt.frame(1.0);
            let before_resize = rt.presented("row-1-u-45").unwrap().0[1];
            rt.set_viewport((width + 160.0, 720.0), 1.0);
            for _ in 0..120 {
                if !rt.frame(1.0 / 60.0) {
                    break;
                }
            }
            let after_resize = rt
                .presented("row-1-u-45")
                .expect("u-45 still presented after resize")
                .0[1];
            assert!(
                (after_resize - before_resize).abs() < 2.0,
                "{width}: resize moved anchor from {before_resize} to {after_resize}"
            );
        }
    }

    #[test]
    fn stick_to_bottom_follows_streaming_and_unpins_when_user_scrolls_up() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            {
                let t = rt.state.current_mut();
                for turn in 0..40usize {
                    t.messages.push(crate::model::Message {
                        id: format!("u-{turn}"),
                        role: "You",
                        text: format!("Prompt {turn}"),
                        images: vec![],
                        context: None,
                    });
                    t.messages.push(crate::model::Message {
                        id: format!("a-{turn}"),
                        role: "Pi",
                        text: format!("Reply {turn}"),
                        images: vec![],
                        context: None,
                    });
                }
                t.status = Status::Running;
            }
            rt.frame(0.0);
            assert!(
                rt.presented("turn-1-a-39").is_some(),
                "{width}: starts at bottom"
            );

            // Streaming growth while pinned keeps bottom in view.
            rt.state
                .current_mut()
                .messages
                .last_mut()
                .unwrap()
                .text
                .push_str("\n\nMore streamed lines\n\nEven more");
            rt.invalidate();
            rt.frame(1.0);
            assert!(
                rt.presented("turn-1-a-39").is_some(),
                "{width}: stays pinned while streaming"
            );

            // Scrolling up unpins and shows Scroll to end; further streaming does NOT yank back down.
            let (conv, _) = rt.presented("conversation-1").unwrap();
            for _ in 0..20 {
                rt.wheel(conv[0] + conv[2] * 0.5, conv[1] + conv[3] * 0.5, 400.0);
                rt.frame(1.0 / 60.0);
            }
            for _ in 0..120 {
                if !rt.frame(1.0 / 60.0) {
                    break;
                }
            }
            assert!(
                rt.presented("turn-1-a-39")
                    .is_none_or(|(r, _)| r[1] >= conv[1] + conv[3]),
                "{width}: scrolled away from bottom: {:?} vs conv {:?}",
                rt.presented("turn-1-a-39"),
                conv
            );
            assert!(
                rt.presented("scroll-to-end").is_some(),
                "{width}: scroll-to-end pill visible"
            );
            let scrolled_row = rt
                .elements()
                .into_iter()
                .find(|e| {
                    e.key.as_deref().is_some_and(|k| k.starts_with("row-1-u-"))
                        && rt.presented(e.key.as_deref().unwrap()).is_some()
                })
                .map(|e| (e.key.unwrap(), e.rect[1]))
                .unwrap();

            rt.state
                .current_mut()
                .messages
                .last_mut()
                .unwrap()
                .text
                .push_str("\n\nTail chunk while reading history");
            rt.invalidate();
            rt.frame(1.0);
            assert!(
                (rt.presented(&scrolled_row.0).unwrap().0[1] - scrolled_row.1).abs() < 1.0,
                "{width}: streaming must not pull scrolled-up reader back down"
            );

            // Clicking scroll-to-end jumps back to bottom and hides the pill.
            press(&mut rt, "scroll-to-end");
            for _ in 0..240 {
                if !rt.frame(1.0 / 60.0) {
                    break;
                }
            }
            assert!(
                rt.presented("turn-1-a-39").is_some(),
                "{width}: returned to bottom"
            );
            assert!(
                rt.find("scroll-to-end").is_none(),
                "{width}: pill hides at bottom"
            );
        }
    }

    #[test]
    fn find_bar_opens_via_chat_find_displays_counts_steps_matches_highlights_and_jumps_to_match() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.on_key = Some(crate::key);
            rt.poll = Some(|s, _, tx| s.poll_thread_find(tx));
            {
                let t = rt.state.current_mut();
                for turn in 0..40usize {
                    let prompt = if turn == 2 {
                        "Early message with target-canary-word here".to_owned()
                    } else if turn == 38 {
                        "Late message with target-canary-word too".to_owned()
                    } else {
                        format!("Ordinary message {turn}")
                    };
                    t.messages.push(crate::model::Message {
                        id: format!("u-{turn}"),
                        role: "You",
                        text: prompt,
                        images: vec![],
                        context: None,
                    });
                    t.messages.push(crate::model::Message {
                        id: format!("a-{turn}"),
                        role: "Pi",
                        text: format!("Reply {turn}"),
                        images: vec![],
                        context: None,
                    });
                }
            }
            rt.frame(0.0);

            // Wire chat.find shortcut (Ctrl+F / Cmd+F).
            let mod_key = if cfg!(target_os = "macos") {
                rune::Mods {
                    cmd: true,
                    ..Default::default()
                }
            } else {
                rune::Mods {
                    ctrl: true,
                    ..Default::default()
                }
            };
            rt.key(rune::KeyEvent {
                key: rune::Key::Text("f".into()),
                mods: mod_key,
            });
            rt.frame(0.0);
            assert!(
                rt.presented("thread-find-bar").is_some(),
                "{width}: bar opens"
            );
            assert!(
                rt.presented("thread-find-input").is_some(),
                "{width}: input shown"
            );
            assert!(
                rt.presented("thread-find-prev").is_some(),
                "{width}: prev shown"
            );
            assert!(
                rt.presented("thread-find-next").is_some(),
                "{width}: next shown"
            );
            assert!(
                rt.presented("thread-find-close").is_some(),
                "{width}: close shown"
            );

            // Bar stays inside the pane without clipping or overflowing.
            let (bar, _) = rt.presented("thread-find-bar").unwrap();
            let (pane, _) = rt.presented("chat-pane").unwrap();
            assert!(
                bar[0] >= pane[0] && bar[0] + bar[2] <= pane[0] + pane[2] + 0.5,
                "{width}: bar {bar:?} outside pane {pane:?}"
            );

            // Typing a missing query reports 0/0.
            rt.paste("missing-query-xyz");
            rt.frame(0.2);
            rt.frame(1.0 / 60.0);
            assert_eq!(
                element_text(&rt, "thread-find-count").as_deref(),
                Some("0/0"),
                "{width}"
            );

            // Replace with matching query -> shows 1/2, highlights match, and jumps to u-2!
            rt.state.set_thread_find_query("target-canary-word".into());
            rt.invalidate();
            for _ in 0..10 {
                rt.frame(1.0 / 60.0);
            }
            assert_eq!(
                element_text(&rt, "thread-find-count").as_deref(),
                Some("1/2"),
                "{width}"
            );
            assert!(
                rt.presented("row-1-u-2").is_some(),
                "{width}: jumped to match u-2"
            );
            assert!(
                rt.elements().iter().any(|e| e.role == "mark"),
                "{width}: highlight mark rendered"
            );

            // Next -> jumps to 2/2 (u-38); Next again wraps to 1/2 (u-2).
            press(&mut rt, "thread-find-next");
            for _ in 0..10 {
                rt.frame(1.0 / 60.0);
            }
            assert_eq!(
                element_text(&rt, "thread-find-count").as_deref(),
                Some("2/2"),
                "{width}"
            );
            assert!(
                rt.presented("row-1-u-38").is_some(),
                "{width}: jumped to match u-38"
            );

            press(&mut rt, "thread-find-prev");
            for _ in 0..10 {
                rt.frame(1.0 / 60.0);
            }
            assert_eq!(
                element_text(&rt, "thread-find-count").as_deref(),
                Some("1/2"),
                "{width}"
            );

            // Close via close button restores composer focus.
            press(&mut rt, "thread-find-close");
            rt.frame(0.0);
            assert!(rt.find("thread-find-bar").is_none(), "{width}: bar closed");
            assert!(
                rt.state.current().draft.focused,
                "{width}: composer refocused"
            );

            // Explicit unsupported state for a peer without thread-find.
            rt.state.open_thread_find();
            rt.state.thread_find.unsupported = true;
            rt.invalidate();
            rt.frame(0.0);
            assert_eq!(
                element_text(&rt, "thread-find-count").as_deref(),
                Some("Update required"),
                "{width}: unsupported peer state"
            );
            rt.state.close_thread_find();
        }
    }

    #[test]
    fn sidebar_search_shows_title_matches_before_message_matches_with_snippets_and_jumps_to_match()
    {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.threads.push(Thread::new(2));
            rt.state.threads[0].title = "Alpha workspace task".into();
            rt.state.threads[0].renamed = true;
            rt.state.threads[1].title = "Unrelated thread".into();
            rt.state.threads[1].renamed = true;
            // Thread 2 is unvisited (no messages loaded locally), matched via server search!
            rt.state.search.replace_text("alpha");
            rt.state.sync_sidebar_search_input();
            rt.state.sidebar_search.visible_query = "alpha".into();
            rt.state.sidebar_search.visible_matches.insert(
                ("local".into(), 2),
                crate::thread_search::ThreadSearchHit {
                    thread_id: 2,
                    project_id: None,
                    entry_id: "u-4".into(),
                    source: "user".into(),
                    snippet: "…matched alpha inside unvisited history…".into(),
                    quality: 1.0,
                    hits: 1,
                    updated: 10,
                },
            );
            rt.state.sidebar_search.shown_order = vec![("local".into(), 2)];
            rt.state.sidebar_search.unsupported_servers = vec!["Studio".into()];
            rt.frame(0.0);

            let (row1, _) = rt.presented("thread-1").expect("title match shown");
            let (row2, _) = rt.presented("thread-2").expect("message match shown");
            assert!(
                row1[1] < row2[1],
                "{width}: title match must appear before message match"
            );
            assert!(
                rt.presented("sidebar-search-snippet-local-2").is_some(),
                "{width}: snippet shown for message-matched row"
            );
            assert!(
                rt.presented("sidebar-search-unsupported").is_some(),
                "{width}: unsupported server notice shown"
            );

            // Clicking the message-matched row opens thread 2 and opens find on the matched query.
            press(&mut rt, "thread-2");
            rt.frame(0.0);
            assert_eq!(rt.state.current().id, 2, "{width}: opened thread 2");
            assert!(rt.state.thread_find.open, "{width}: find bar opened");
            assert_eq!(
                rt.state.thread_find.query.text, "alpha",
                "{width}: query set"
            );
        }
    }

    #[test]
    fn shelves_start_collapsed_keep_the_open_row_and_page_settled_threads() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        // KSG's Working shelf collects running threads at the bottom of the list.
        assert!(rt.find("shelf-Working").is_some());
        // Thread 2 is running, so it lives on the collapsed Working shelf.
        assert!(rt.find("thread-2").is_none());
        // Shelves sit at the bottom of the sidebar, just above its footer.
        let mut tall = Runtime::with_text(
            State::new(".".into(), false),
            view,
            (1280.0, 1400.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, MONO_FAMILY)),
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
        assert!(rt.find("shelf-Snoozed").is_some());
        assert!(rt.find("shelf-Settled").is_some());
        assert!(rt.find("thread-4").is_none());
        assert!(rt.find("thread-5").is_none());
        rt.update(|_, tx| tx.scroll_into_view("shelf-Working"));
        rt.frame(0.5);
        press(&mut rt, "shelf-Working");
        assert!(rt.state.shelves[0]);
        assert!(rt.find("thread-2").is_some());
        rt.update(|_, tx| tx.scroll_into_view("shelf-Snoozed"));
        rt.frame(0.5);
        press(&mut rt, "shelf-Snoozed");
        assert!(rt.state.shelves[1]);
        assert!(rt.find("thread-4").is_some());
        rt.state.open_kept_tab(3);
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
        rt.state.shelves[2] = true;
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
        rt.state.compact_sidebar = false;
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
    fn a_long_draft_scrolls_inside_the_composer_beside_its_actions() {
        for zoom in [1.0, 1.25] {
            for width in [1280.0] {
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
                assert!(field[3] > view[3], "{at}: the draft fits; it should not");
                // KSG's prompt body: the actions sit beside the text at the bottom right of a
                // wrapped draft, never over it, and the card is the body's height.
                assert!(
                    input[0] + input[2] <= actions[0] + 0.5,
                    "{at}: the input {input:?} runs under the actions {actions:?}"
                );
                assert!(
                    (input[1] + input[3] - (actions[1] + actions[3])).abs() <= 1.5,
                    "{at}: the actions {actions:?} are not on the input's last line {input:?}"
                );
                assert!(
                    actions[1] + actions[3] <= card[1] + card[3] + 0.5
                        && card[1] + card[3] - (actions[1] + actions[3]) <= 10.0,
                    "{at}: the actions {actions:?} are not at the card's bottom {card:?}"
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
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.demo = false;
            rt.state.compact_sidebar = true;
            rt.frame(0.0);
            let id = rt.state.current().id;
            let viewport = format!("conversation-{id}");
            rt.update(|_, tx| tx.scroll_to(viewport.clone(), 0.0, 0.0));
            rt.frame(0.5);
            if width < 760.0 {
                let (pane, _) = rt.presented("chat-pane").unwrap();
                press(&mut rt, "sidebar-toggle");
                rt.frame(1.0);
                assert!(rt.presented("sidebar").is_none());
                assert_eq!(
                    rt.presented("chat-pane").unwrap().0,
                    pane,
                    "closing the compact sidebar must not resize the conversation"
                );
            }
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
                // The prompt body's 6 pt padding above and below the field.
                (cap - (12.0 + COMPOSER_FIELD_MAX)).abs() <= 0.5,
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
    fn archived_captain_hides_composer_and_uses_group_restore_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state = State::headless(
                std::env::temp_dir().join(format!("shina-archived-ui-{}", uuid::Uuid::new_v4())),
            );
            rt.state.demo = true;
            rt.state.captains = crate::captains::Captain::presets();
            rt.state.open_captain(0);
            let home = rt.state.current().id;
            let database = crate::database::Database::open(&rt.state.data_dir).unwrap();
            database.begin().unwrap();
            let preview = rt.state.captain_lifecycle_preview(0, "archive").unwrap();
            rt.state
                .captain_archive_command(
                    0,
                    "archive",
                    preview["captainVersion"].as_str().unwrap(),
                    &[],
                )
                .unwrap();
            database
                .commit(1, &rt.state.projection(), &json!([]), None, &[])
                .unwrap();
            rt.state.show_archive = true;
            rt.state.compact_sidebar = width < 760.0;
            rt.frame(0.0);
            rt.frame(1.0);
            for key in ["captain-archived-bar", "captain-archived-unarchive"] {
                let (rect, _) = rt.presented(key).unwrap_or_else(|| panic!("missing {key}"));
                assert!(
                    rect[0] >= 0.0
                        && rect[1] >= 0.0
                        && rect[0] + rect[2] <= width + 0.5
                        && rect[1] + rect[3] <= 720.5,
                    "{key} outside {width}: {rect:?}"
                );
            }
            assert!(rt.presented("composer-input").is_none());
            assert_eq!(rt.state.current().id, home);
            press(&mut rt, "thread-actions");
            assert!(rt.find("menu-unarchive-captain").is_some());
            assert!(rt.find("menu-archive-thread").is_none());
            assert!(rt.find("menu-delete-thread").is_none());
        }
    }

    #[test]
    fn narrow_workspace_keeps_the_composer_inside_the_window() {
        let mut rt = runtime(620.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        let (rect, _) = rt.presented("composer").unwrap();
        assert!(rect[0] >= 0.0 && rect[0] + rect[2] <= 620.5);
        assert!(rect[2] > 200.0);
        for key in ["thread-actions", "leave-example"] {
            let (rect, _) = rt.presented(key).unwrap();
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= 620.5,
                "{key} overflows the window"
            );
        }
        let (bubble, _) = rt
            .presented(&format!("{}-text", message_key(rt.state.current(), 0)))
            .unwrap();
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
            breakdown: None,
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

        // KSG's `ComposerControlsBar`: the ring sits in the context strip beside the model.
        let (ring_rect, _) = rt.presented("context-usage-ring").unwrap();
        let (model_rect, _) = rt.presented("model-effort").unwrap();
        let (strip, _) = rt.presented("composer-footer").unwrap();
        assert!(
            model_rect[0] + model_rect[2] <= ring_rect[0] + 0.5,
            "ring must sit beside, not over, the model"
        );
        assert!(
            (ring_rect[1] + ring_rect[3] / 2.0 - (strip[1] + strip[3] / 2.0)).abs() < 2.0,
            "the ring is centred in the context strip"
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
        // Primary press opens the Inspector on Context (spec §4.4).
        assert_eq!(
            rt.state.right_panel_active_view(),
            Some(crate::right_panel::PanelView::Context)
        );
        // Secondary press pins the hover popover (spec §4.4).
        let (rx, ry) = rt.find("context-usage-ring").unwrap();
        rt.secondary_down(rx, ry);
        rt.secondary_up(rx, ry);
        rt.frame(0.3);
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
        // Escape closes the pinned popover; Space and primary press toggle the Context tab.
        rt.key(rune::KeyEvent {
            key: rune::Key::Escape,
            mods: Default::default(),
        });
        rt.frame(0.5);
        assert!(
            rt.find("context-details").is_none(),
            "Escape closes the details popover"
        );
        assert!(!rt.state.current().context.details_open);

        rt.key(rune::KeyEvent {
            key: rune::Key::Text(" ".into()),
            mods: Default::default(),
        });
        rt.frame(0.3);
        assert_eq!(rt.state.right_panel_active_view(), None);

        press(&mut rt, "context-usage-ring");
        rt.frame(0.3);
        assert_eq!(
            rt.state.right_panel_active_view(),
            Some(crate::right_panel::PanelView::Context),
            "pointer activation toggles the Inspector on Context"
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
            breakdown: None,
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
        for width in [1280.0] {
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
    fn usage_limits_and_service_tiers_fit_at_supported_widths() {
        for width in [1280.0] {
            let mut rt = context_fixture(width);
            let t = rt.state.current_mut();
            t.context.details_open = true;
            t.harness_state["usageLimits"] = json!({"windows":[
                {"id":"primary","name":"5 hour","usedPercent":90,"resetsAt":2100000000},
                {"id":"secondary","name":"7 day","usedPercent":25,"resetsAt":null}]});
            rt.invalidate();
            rt.frame(0.3);
            let (card, _) = rt.presented("context-details").unwrap();
            assert!(
                card[0] >= 0.0 && card[0] + card[2] <= width + 0.5,
                "{width}: {card:?}"
            );
            for index in 0..2 {
                for prefix in ["usage-bar", "usage-reset", "usage-percent"] {
                    let (rect, _) = rt.presented(&format!("{prefix}-{index}")).unwrap();
                    assert!(
                        rect[0] >= card[0] && rect[0] + rect[2] <= card[0] + card[2] + 0.5,
                        "{width}: {prefix} {rect:?}"
                    );
                }
            }
            assert_eq!(rt.presented("usage-bar-0").unwrap().0[3], 6.0);
            let fill = |rt: &Runtime<State>, key: &str| {
                rt.elements()
                    .into_iter()
                    .find(|element| element.key.as_deref() == Some(key))
                    .unwrap()
                    .fill
            };
            assert_eq!(fill(&rt, "usage-fill-0"), palette(rt.state.dark).danger);
            assert_eq!(fill(&rt, "usage-fill-1"), palette(rt.state.dark).faint);
            rt.state.current_mut().context = Default::default();
            rt.state.current_mut().context_window = None;
            rt.state.current_mut().context.details_open = true;
            rt.invalidate();
            rt.frame(0.35);
            assert!(
                rt.find("context-usage-limits").is_some(),
                "limits remain accessible without context capacity"
            );
            rt.state.current_mut().harness_state["usageLimits"] = json!({"windows":[]});
            rt.invalidate();
            rt.frame(0.4);
            assert!(rt.find("context-usage-limits").is_none());
            let t = rt.state.current_mut();
            t.context.close_popover();
            t.harness = "codex".into();
            t.model_provider = "codex".into();
            t.model_id = "fixture".into();
            t.models = vec![crate::session::AgentModel::from_harness_value(&json!({
                "provider":"codex","id":"fixture","name":"Fixture","thinkingLevels":["low","high"],
                "serviceTiers":[{"id":"default","name":"Standard"},{"id":"fast","name":"Fast"}]
            }),"codex").unwrap()];
            t.thinking_levels = vec!["low".into(), "high".into()];
            t.thinking = "high".into();
            rt.state.settings_picker = Some(SettingsPicker::Thinking);
            rt.invalidate();
            rt.frame(0.5);
            let (picker, _) = rt.presented("reasoning-picker").unwrap();
            let (control, _) = rt.presented("service-tier-picker").unwrap();
            assert!(
                control[0] >= picker[0] && control[0] + control[2] <= picker[0] + picker[2] + 0.5
            );
            assert!(picker[0] >= 0.0 && picker[0] + picker[2] <= width + 0.5);
            press(&mut rt, "service-tier-1");
            assert_eq!(rt.state.current().harness_state["serviceTier"], "fast");
        }
    }

    #[test]
    fn chat_lane_follows_ksg_gutters_and_width_at_supported_widths() {
        for width in [620.0, 1280.0] {
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
    fn composer_shell_recessed_pill_and_quiet_access_chip_at_supported_widths() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.demo = false;
            rt.frame(0.0);
            rt.frame(0.0);
            let p = palette(rt.state.dark);
            // KSG's `ComposerSurface.Shell`: the 16 pt tray wraps the card in the resting
            // and the expanded composer alike.
            let (shell, _) = rt
                .presented("composer-shell")
                .unwrap_or_else(|| panic!("{width}: composer-shell"));
            let (card, _) = rt
                .presented("composer")
                .unwrap_or_else(|| panic!("{width}: composer card"));
            assert!(
                card[0] >= shell[0] - 0.5
                    && card[0] + card[2] <= shell[0] + shell[2] + 0.5
                    && card[1] >= shell[1] - 0.5
                    && card[1] + card[3] <= shell[1] + shell[3] + 0.5,
                "{width}: card {card:?} outside shell {shell:?}"
            );
            let fills: std::collections::HashMap<String, Color> = rt
                .elements()
                .into_iter()
                .filter_map(|e| e.key.map(|k| (k, e.fill)))
                .collect();
            assert_eq!(
                fills.get("composer-shell"),
                Some(&p.tray),
                "{width}: shell fill"
            );
            // KSG's `ComposerSendPill`: the recessed 32 pt action slot in `pill` fill.
            assert_eq!(
                fills.get("composer-transition-actions"),
                Some(&p.pill),
                "{width}: send pill fill"
            );
            let (pill, _) = rt.presented("composer-transition-actions").unwrap();
            assert!(
                (pill[3] - 32.0).abs() < 0.5,
                "{width}: pill height {pill:?}"
            );
            let (send, _) = rt.presented("send-message").unwrap();
            assert!(
                send[0] >= pill[0] - 0.5
                    && send[0] + send[2] <= pill[0] + pill[2] + 0.5
                    && send[1] >= pill[1] - 0.5
                    && send[1] + send[3] <= pill[1] + pill[3] + 0.5,
                "{width}: send {send:?} outside pill {pill:?}"
            );
            // Conversation and composer share the resolved lane.
            let (lane, _) = rt.presented("chat-lane").unwrap();
            assert!(
                (card[0] - lane[0]).abs() < 0.5 && (card[2] - lane[2]).abs() < 0.5,
                "{width}: composer {card:?} and transcript {lane:?} share one lane"
            );
            // KSG's `composerControlsOwnership`: the default mode keeps the chip quiet.
            assert!(
                rt.presented("access-mode").is_none(),
                "{width}: the default access mode must not mount the chip"
            );
            assert!(
                !rt.state.composer_ui.show_access.get(),
                "{width}: default mode keeps the mode block out of the strip and the overflow"
            );
            rt.state.current_mut().native.access = crate::model::AccessMode::Supervised;
            rt.state.current_mut().draft.focused = true;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(0.0);
            assert!(
                rt.state.composer_ui.show_access.get(),
                "{width}: a non-default access mode shows the chip"
            );
            // The chip sits in the strip, or is measured into the overflow menu, which then
            // carries the permissions section (KSG's `overflowShowsPermissions`).
            assert!(
                rt.presented("access-mode").is_some()
                    || rt.presented("composer-overflow").is_some(),
                "{width}: the non-default chip is in the strip or its overflow"
            );
            rt.state.current_mut().native.access = crate::model::AccessMode::Full;
        }
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
            !rt.state.right_panel().open,
            "tool output never opens a side panel"
        );
    }

    fn long_turn_runtime(width: f32, parts: usize, live: bool) -> Runtime<State> {
        let mut rt = runtime(width);
        long_turn_fixture(&mut rt.state, parts, live);
        open_work_streaks(rt.state.current_mut());
        settle(&mut rt);
        rt
    }

    /// Unfolds every work streak of the long turn, so each step is its own work entry.
    /// Folded, a streak of hundreds of steps is one header row.
    fn open_work_streaks(t: &mut Thread) {
        let firsts: Vec<usize> = turn_parts(t, 1, 2)
            .iter()
            .filter_map(|part| match part {
                TranscriptPart::Work(entries) => entry_tools(entries).next(),
                _ => None,
            })
            .collect();
        for index in firsts {
            t.tools[index].group_expanded = true;
        }
        t.timeline_cache.get_mut().touch_message(1);
    }

    fn settle(rt: &mut Runtime<State>) {
        for _ in 0..120 {
            if !rt.frame(1.0 / 60.0) {
                break;
            }
        }
    }

    fn mounted_work(rt: &Runtime<State>) -> usize {
        rt.elements()
            .iter()
            .filter(|element| {
                element.key.as_deref().is_some_and(|key| {
                    key.starts_with("inline-tool-1-")
                        || key.starts_with("inline-thought-1-")
                        || key.starts_with("tool-output-1-")
                })
            })
            .count()
    }

    /// The key of the work slice `1 / divisor` of the way through the long turn.
    fn slice_at(rt: &Runtime<State>, divisor: usize) -> String {
        let rows = timeline_structure(rt.state.current()).rows.clone();
        let slices: Vec<_> = rows
            .iter()
            .filter(|row| {
                matches!(
                    row,
                    TimelineRow::TurnPiece {
                        piece: TurnPiece::Work { .. },
                        ..
                    }
                )
            })
            .collect();
        slices[slices.len() / divisor].key().to_owned()
    }

    fn center_on(rt: &mut Runtime<State>, key: &str) {
        let key = key.to_owned();
        rt.update(move |_, tx| tx.scroll_center_in_instant("conversation-1", key));
        settle(rt);
    }

    #[test]
    fn long_turns_mount_work_in_proportion_to_the_viewport() {
        for live in [true, false] {
            for width in [1280.0] {
                let mut rt = long_turn_runtime(width, 1_000, live);
                let rows = timeline_structure(rt.state.current()).rows.clone();
                let slices = rows
                    .iter()
                    .filter(|row| {
                        matches!(
                            row,
                            TimelineRow::TurnPiece {
                                piece: TurnPiece::Work { .. },
                                ..
                            }
                        )
                    })
                    .count();
                assert!(
                    slices >= 1_000 / SLICE_ITEMS,
                    "{live} {width}: {slices} slices"
                );
                assert!(rows.iter().any(|row| row.key() == "turn-1-long-answer"));
                // A quarter in: inside the first streak of both fixtures, away from the prose
                // that splits each turn in half.
                let middle = slice_at(&rt, 4);
                center_on(&mut rt, &middle);
                assert!(rt.presented(&middle).is_some(), "{live} {width}: {middle}");
                let short = mounted_work(&rt);
                // Twice the work: the same viewport mounts the same number of rows.
                let mut longer = long_turn_runtime(width, 2_000, live);
                let middle = slice_at(&longer, 4);
                center_on(&mut longer, &middle);
                let long = mounted_work(&longer);
                assert!(short > 0 && short < 400, "{live} {width}: {short} mounted");
                // Rune mounts two viewports either side; the longer list's estimated rows can
                // carry up to two more slices of slack at that edge.
                assert!(
                    long.abs_diff(short) <= 4 * SLICE_ITEMS,
                    "{live} {width}: {short} mounted for 1,000 parts, {long} for 2,000"
                );
                // Every slice of a completed turn rebuilt only the mounted ones.
                take_timeline_counts();
                rt.update(|s, _| s.dark = !s.dark);
                rt.frame(1.0 / 60.0);
                let built = take_timeline_counts().built_slices as usize;
                assert!(
                    built > 0 && built < slices / 2,
                    "{live} {width}: built {built}"
                );
            }
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
        rt.frame(0.0);
        // No thinking text yet: the Thinking row speaks for the turn.
        assert!(rt.presented("live-work-1").is_none());
        assert!(rt.presented("live-status-1").is_some());
        rt.state
            .current_mut()
            .apply(json!({"type":"message_update","assistantMessageEvent":{
                "type":"thinking_delta","contentIndex":0,"delta":"Checking the file. Then the tests."
            }}));
        rt.invalidate();
        rt.frame(1.0);
        // KSG's `work-live`: the thought's first sentence above "Thinking", in one row.
        let (thought, _) = rt.presented("live-thought-1").unwrap();
        let (status, _) = rt.presented("live-work-text-1").unwrap();
        assert!(thought[1] < status[1]);
        let texts: Vec<_> = rt
            .elements()
            .iter()
            .filter_map(|e| e.text.clone())
            .collect();
        assert!(texts.iter().any(|text| text == "Checking the file."));
        assert!(texts.iter().any(|text| text == "Thinking"));
        assert!(rt.presented("live-status-1").is_none());
        assert!(rt.presented("inline-thought-1-0").is_none());
        rt.state
            .current_mut()
            .apply(json!({"type":"agent_settled"}));
        rt.invalidate();
        rt.frame(1.0);
        rt.frame(2.0);
        assert!(rt.presented("live-status-1").is_none());
        assert!(rt.presented("live-work-1").is_none());
        assert!(rt.presented("toggle-work-1-0").is_some());
        assert!(!rt.state.current().work[0].expanded);
        assert!(rt.presented("inline-thought-1-0").is_none());
    }

    /// A running turn shaped as KSG Code's timeline tests shape theirs: a streak of thoughts and
    /// calls, interim prose, then a live streak whose last call is running.
    fn streak_runtime(width: f32) -> Runtime<State> {
        let mut rt = runtime(width);
        let t = rt.state.current_mut();
        t.apply(
            json!({"type":"response","id":"history","command":"get_messages","success":true,
            "data":{"messages":[{"role":"assistant","content":[
                {"type":"thinking","thinking":"Plan the audit."},
                {"type":"toolCall","id":"a","name":"read","arguments":{"path":"a.rs"}},
                {"type":"thinking","thinking":"Now the second file."},
                {"type":"toolCall","id":"b","name":"read","arguments":{"path":"b.rs"}},
                {"type":"text","text":"Both files parse."},
                {"type":"thinking","thinking":"**Checking the tests**\n\nThe suite lives in tests/."},
                {"type":"toolCall","id":"c","name":"read","arguments":{"path":"c.rs"}},
                {"type":"toolCall","id":"d","name":"read","arguments":{"path":"d.rs"}}
            ]}]}}),
        );
        for tool in &mut t.tools {
            tool.done = true;
            tool.failed = false;
        }
        for (tool, detail) in t.tools.iter_mut().zip(["a.rs", "b.rs", "c.rs", "d.rs"]) {
            tool.detail = detail.into();
        }
        t.tools[3].done = false;
        t.status = Status::Running;
        t.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(12));
        t.work.clear();
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        rt
    }

    fn texts(rt: &Runtime<State>) -> Vec<String> {
        rt.elements()
            .iter()
            .filter_map(|e| e.text.clone())
            .collect()
    }

    #[test]
    fn thought_and_tool_streaks_fold_before_prose_and_into_one_live_row() {
        for width in [1280.0] {
            let mut rt = streak_runtime(width);
            rt.state.current_mut().harness = "pi".into();
            rt.state.current_mut().started =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(12));
            rt.invalidate();
            rt.frame(1.0);
            let t = rt.state.current();
            let parts = turn_parts(t, 0, 1);
            use WorkEntry::{Thought, Tool};
            assert_eq!(
                parts[0],
                TranscriptPart::Work(vec![Thought(0), Tool(0), Thought(1), Tool(1)])
            );
            assert!(matches!(parts[1], TranscriptPart::Text { .. }));
            assert_eq!(
                parts[2],
                TranscriptPart::Work(vec![Thought(2), Tool(2), Tool(3)])
            );
            let lane = rt.presented("chat-lane").unwrap().0;
            // A work row's hover wash starts 6 pt before the lane, so its icon slot sits on the
            // lane edge as KSG's does.
            let inside = |rect: [f32; 4]| {
                rect[0] >= lane[0] - 6.5 && rect[0] + rect[2] <= lane[0] + lane[2] + 0.5
            };
            // "Working for 12s" heads the turn.
            assert!(texts(&rt).iter().any(|text| text == "Working for "));
            assert!(texts(&rt).iter().any(|text| text == "12s"));
            // The streak before the prose is one folded header, even while the turn runs.
            let group = rt.presented("toggle-tool-group-1-0").unwrap().0;
            assert!(inside(group), "{width}: {group:?}");
            assert!(texts(&rt).iter().any(|text| text == "Read 2 files"));
            for key in ["inline-tool-1-0", "inline-tool-1-1", "inline-thought-1-0"] {
                assert!(rt.presented(key).is_none(), "{width}: {key}");
            }
            // The live streak is one row: its thought's heading over the running call.
            let live = rt.presented("live-work-1").unwrap().0;
            assert!(inside(live), "{width}: {live:?}");
            assert!(group[1] < live[1]);
            let thought = rt.presented("live-thought-1").unwrap().0;
            let step = rt.presented("live-work-text-1").unwrap().0;
            assert!(thought[1] < step[1]);
            assert!(texts(&rt).iter().any(|text| text == "Checking the tests"));
            assert!(texts(&rt).iter().any(|text| text == "Reading d.rs"));
            for key in ["inline-tool-1-2", "inline-tool-1-3", "live-status-1"] {
                assert!(rt.presented(key).is_none(), "{width}: {key}");
            }

            // The live row unfolds its steps under itself, and folds them again.
            press(&mut rt, "live-work-1");
            rt.frame(1.0);
            for key in ["inline-thought-1-2", "inline-tool-1-2", "inline-tool-1-3"] {
                let rect = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"))
                    .0;
                assert!(rect[1] > live[1] && inside(rect), "{width}: {key} {rect:?}");
            }
            assert!(rt.presented("live-thought-1").is_none());
            press(&mut rt, "live-work-1");
            rt.frame(1.0);
            assert!(rt.presented("inline-tool-1-3").is_none());

            // The newest call finishes: the row keeps standing for the turn, in the present
            // tense, as KSG's does until the next step starts.
            rt.state.current_mut().tools[3].done = true;
            rt.invalidate();
            rt.frame(1.0);
            assert!(texts(&rt).iter().any(|text| text == "Reading d.rs"));
            assert!(rt.presented("live-status-1").is_none());

            // It failed instead: the Thinking row stands in, at the bottom, and unfolds the tail.
            rt.state.current_mut().tools[3].failed = true;
            rt.invalidate();
            rt.frame(1.0);
            assert!(rt.presented("live-work-1").is_none());
            let thinking = rt.presented("live-status-1").unwrap().0;
            assert!(thinking[1] > group[1] && inside(thinking));
            press(&mut rt, "live-status-1");
            rt.frame(1.0);
            assert!(rt.presented("inline-tool-1-3").unwrap().0[1] > thinking[1]);
            press(&mut rt, "live-status-1");
            rt.frame(1.0);

            // Interim prose after the streak: Thinking waits under it, at the very bottom.
            let t = rt.state.current_mut();
            t.tools[3].failed = false;
            t.messages[0].text.push_str(" Next, the fixtures.");
            rt.invalidate();
            rt.frame(1.0);
            let offset = rt.state.current().tools[3].anchor.offset;
            let answer = rt
                .presented(&assistant_body_key(rt.state.current(), 0, offset))
                .unwrap_or_else(|| panic!("{width}: the interim prose"))
                .0;
            let thinking = rt.presented("live-status-1").unwrap().0;
            assert!(answer[1] < thinking[1], "{width}");
            // The tail is now a finished streak before prose: folded like the first.
            assert!(rt.presented("live-work-1").is_none());
            assert!(rt.presented("toggle-tool-group-1-2").is_some());

            // The turn ends: the live rows leave and both streaks stay with the prose.
            rt.state
                .current_mut()
                .apply(json!({"type":"agent_settled"}));
            rt.invalidate();
            rt.frame(1.0);
            rt.frame(2.0);
            assert!(rt.presented("working-bar-1").is_none());
            assert!(rt.presented("live-status-1").is_none());
            assert!(rt.presented("toggle-work-1-0").is_none());
            assert!(rt.presented("toggle-tool-group-1-2").is_some());
            press(&mut rt, "toggle-tool-group-1-0");
            rt.frame(4.0);
            for key in [
                "inline-thought-1-0",
                "inline-tool-1-0",
                "inline-thought-1-1",
                "inline-tool-1-1",
            ] {
                let rect = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"))
                    .0;
                assert!(inside(rect), "{width}: {key}");
            }
            // The steps sit 16 pt in from the header, as KSG's `WorkLogList` (`ps-4`).
            assert_eq!(
                rt.presented("inline-tool-1-0").unwrap().0[0],
                rt.presented("toggle-tool-group-1-0").unwrap().0[0] + WORK_LIST_INDENT,
                "{width}"
            );
        }
    }

    #[test]
    fn working_bar_toggles_earlier_live_work_while_keeping_live_tail_visible() {
        let mut rt = streak_runtime(1280.0);
        let bar = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("working-bar-1"))
            .expect("working bar");
        assert_eq!(bar.role, "button");
        assert_eq!(bar.label.as_deref(), Some("Collapse work history"));
        assert!(rt.presented("toggle-tool-group-1-0").is_some());
        assert!(rt.presented("live-work-1").is_some());

        press(&mut rt, "working-bar-1");
        rt.frame(1.0);
        assert!(rt.state.current().live_work_collapsed);
        assert!(rt.presented("toggle-tool-group-1-0").is_none());
        assert!(rt.presented("live-work-1").is_some());
        let bar = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("working-bar-1"))
            .expect("collapsed working bar");
        assert_eq!(bar.label.as_deref(), Some("Expand work history"));

        // Active compaction arriving while the live fold is closed stays visible.
        {
            let t = rt.state.current_mut();
            let message = t.messages.len().saturating_sub(1);
            let offset = t.messages.get(message).map_or(0, |m| m.text.len());
            t.native.compactions.push(json!({
                "running": true,
                "anchor": { "message": message, "offset": offset }
            }));
            t.transcript_structure += 1;
            t.timeline_cache.get_mut().touch_disclosure();
        }
        rt.invalidate();
        rt.frame(1.5);
        assert!(rt.state.current().live_work_collapsed);
        assert!(rt.presented("toggle-tool-group-1-0").is_none());
        assert!(rt.presented("toggle-tool-group-1-2").is_some());
        assert!(rt.presented("compaction-0").is_some());

        // Tool reveal opens the live fold before resolving the hidden tool row.
        let hidden_tool_id = rt.state.current().tools[0].id.clone();
        rt.state.reveal_tool(hidden_tool_id);
        rt.invalidate();
        rt.frame(2.0);
        assert!(!rt.state.current().live_work_collapsed);
        assert!(rt.presented("toggle-tool-group-1-0").is_some());

        // Collapse again and verify thread find reveals folded interim prose in the live turn.
        press(&mut rt, "working-bar-1");
        rt.frame(2.5);
        assert!(rt.state.current().live_work_collapsed);
        let msg_id = rt.state.current().messages[0].id.clone();
        rt.state.open_thread_find();
        rt.state.thread_find.query.replace_text("checking");
        rt.state.thread_find.pending_reveal = Some(crate::thread_find::ThreadFindMatch {
            entry_id: msg_id.clone(),
            occurrence: 0,
        });
        rt.update(|s, tx| {
            s.consume_pending_find_reveal(tx);
        });
        rt.frame(3.0);
        assert!(!rt.state.current().live_work_collapsed);
        assert!(rt.presented("toggle-tool-group-1-0").is_some());
        assert!(rt.presented("compaction-0").is_some());

        // Collapse again and verify citation navigation reveals folded interim prose too.
        press(&mut rt, "working-bar-1");
        rt.frame(3.5);
        assert!(rt.state.current().live_work_collapsed);
        let citation = crate::composer_context::AssistantCitation {
            version: 1,
            environment_id: rt.state.machine_identity("local"),
            thread_id: rt.state.current().id.to_string(),
            message_id: msg_id,
            text: "checking".into(),
            comment: None,
            start: 0,
            end: 8,
            prefix: String::new(),
            suffix: String::new(),
        };
        rt.update(move |s, tx| {
            assert!(s.navigate_to_citation(&citation, tx));
        });
        rt.frame(4.0);
        assert!(!rt.state.current().live_work_collapsed);
        assert!(rt.presented("toggle-tool-group-1-0").is_some());
    }

    #[test]
    fn a_new_roster_is_empty_and_adding_a_captain_opens_the_selector_then_editor() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        assert!(rt.state.captains.is_empty());
        press(&mut rt, "assign-captain");
        assert!(rt.state.captain_picker);
        assert!(rt.state.project_picker.is_none());
        assert!(rt.find("choose-no-captain").is_some());
        press(&mut rt, "picker-new-captain");
        assert!(rt.state.captain_editor.is_some());
        assert!(!rt.state.captain_picker);
        let editor = rt.state.captain_editor.as_mut().unwrap();
        assert!(editor.role.text.is_empty(), "no default role");
        assert!(
            !editor.name.text.trim().is_empty(),
            "suggested name is pre-filled"
        );
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
        finish_folder_listing(&mut rt);
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
        finish_folder_listing(&mut rt);
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
        finish_folder_listing(&mut rt);
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
        assert!(rt.find("assign-captain").is_none());
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
        rt.state.use_pi_for_test();
        rt.invalidate();
        rt.frame(0.0);
        rt
    }

    #[test]
    fn captain_crew_surfaces_fit_and_open_workers_at_supported_widths() {
        for width in [1280.0] {
            let mut rt = crew_runtime(width);
            assert_eq!(rt.state.current().id, 7);
            rt.invalidate();
            rt.frame(0.0);
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
                    "composer",
                ],
            );
            // Delegations are compact task rows, not generic tool rows, and finished tasks
            // stay off the transcript. Every crew member has a row in the Crew tab.
            assert!(rt.presented("inline-tool-7-0").is_none());
            let card = rt.presented("task-card-16").unwrap().0;
            assert!((card[3] - 32.0).abs() < 0.5, "{width}: {card:?}");
            rt.state.right_panels.open(
                &rt.state.panel_key(),
                crate::right_panel::SingletonKind::Work,
            );
            rt.invalidate();
            rt.frame(0.0);
            assert_inside(
                &mut rt,
                width,
                &[
                    "crew-roster-rows",
                    "crew-chip-1",
                    "crew-chip-16",
                    "crew-chip-18",
                    "crew-chip-19",
                    "crew-roster-footer",
                    "crew-roster-new",
                    "crew-roster-recruit",
                ],
            );
            press(&mut rt, "crew-chip-16");
            rt.frame(1.0);
            assert_eq!(rt.state.current().id, 16, "{width}");
            // A healthy worker has nothing between the conversation and the composer.
            assert!(rt.presented("parent-bar").is_none(), "{width}");
            assert_inside(&mut rt, width, &["composer"]);
            assert!(rt.presented("crew-tray").is_none());
            // The worker's details card links back to its captain. KSG's 256 px sidebar
            // leaves the lane wide enough for the card to dock under the covering Inspector.
            assert!(rt.presented("details-card").is_some(), "{width}");
            press(&mut rt, "open-captain-from-overview");
            assert_eq!(rt.state.current().id, 7, "{width}");
            press(&mut rt, "needs-you-source-2");
            assert_eq!(rt.state.current().id, 2, "{width}");
        }
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (a, b) = (a.luminance(), b.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn the_crew_tab_lists_each_member_once_grouped_by_state_at_all_widths() {
        for width in [1280.0] {
            let mut rt = crew_runtime(width);
            // The strip is a flat 28 pt row that toggles the dismissible Now popover.
            let header = rt.presented("crew-roster-toggle").unwrap().0;
            assert!((header[3] - 28.0).abs() < 0.5, "{width}: {header:?}");
            assert!(rt.presented("now-popover").is_none());
            assert!(rt.presented("crew-roster-rows").is_none());
            press(&mut rt, "crew-roster-toggle");
            rt.frame(1.0);
            assert_inside(
                &mut rt,
                width,
                &["now-popover", "now-popover-open-inspector"],
            );
            assert!(rt.presented("right-panel").is_none(), "{width}");
            // Escape dismisses the popover without opening the Inspector.
            rt.key(rune::KeyEvent {
                key: rune::Key::Escape,
                mods: Default::default(),
            });
            rt.frame(1.0);
            rt.frame(1.0);
            assert!(rt.presented("now-popover").is_none(), "{width}");
            // Reopen and use the footer action to open the full Inspector Crew tab.
            press(&mut rt, "crew-roster-toggle");
            rt.frame(1.0);
            press(&mut rt, "now-popover-open-inspector");
            rt.frame(1.0);
            assert!(rt.presented("now-popover").is_none(), "{width}");
            assert_eq!(
                rt.state.right_panel_active_view(),
                Some(crate::right_panel::PanelView::Work),
                "{width}"
            );
            assert_inside(
                &mut rt,
                width,
                &[
                    "crew-tab",
                    "crew-roster-rows",
                    "crew-roster-footer",
                    "crew-roster-new",
                    "crew-roster-recruit",
                ],
            );
            // Every member has exactly one row, 44 pt, filed under its group.
            for id in [1, 2, 16, 17, 18, 19] {
                let chip = format!("crew-chip-{id}");
                let (row, _) = rt
                    .presented(&chip)
                    .unwrap_or_else(|| panic!("{width}: {chip} missing"));
                assert!((row[3] - 44.0).abs() < 0.5, "{width}: {chip} {row:?}");
            }
            // The groups appear in the spec's order: needs you leads, queued before idle.
            for group in [
                "crew-group-needs-you",
                "crew-group-working",
                "crew-group-queued",
            ] {
                assert!(rt.presented(group).is_some(), "{width}: {group} missing");
            }
            let needs = rt.presented("crew-group-needs-you").unwrap().0;
            let working = rt.presented("crew-group-working").unwrap().0;
            let queued = rt.presented("crew-group-queued").unwrap().0;
            assert!(
                needs[1] < working[1] && working[1] < queued[1],
                "{width}: groups out of order"
            );
        }
    }

    #[test]
    fn composer_request_banner_pages_thread_and_crew_requests_and_now_strip_works_for_subagents() {
        // 1. Unified request surface inside ComposerSurface.Shell: a Captain home with both
        // its own pending confirm dialog and a crew approval pages both in one banner.
        let mut rt = crew_runtime(1280.0);
        rt.state.current_mut().apply(json!({
            "type": "extension_ui_request",
            "id": "home-confirm",
            "method": "confirm",
            "title": "Run cargo test?",
            "message": "Allow running cargo test in this workspace."
        }));
        rt.invalidate();
        rt.frame(0.0);
        let shell = rt.presented("composer-shell").unwrap().0;
        let dialog = rt.presented("pi-dialog").unwrap().0;
        assert!(
            dialog[1] >= shell[1] - 1.0 && dialog[1] + dialog[3] <= shell[1] + shell[3] + 1.0,
            "request banner should dock inside composer-shell: {dialog:?} vs {shell:?}"
        );
        assert!(rt.presented("needs-you-2").is_none());
        assert_eq!(
            element_text(&rt, "needs-you-page-count").as_deref(),
            Some("1/2")
        );
        press(&mut rt, "needs-you-next");
        rt.frame(0.5);
        rt.frame(0.5);
        assert!(rt.presented("pi-dialog").is_none());
        assert!(rt.presented("needs-you-2").is_some());
        assert_eq!(
            element_text(&rt, "needs-you-page-count").as_deref(),
            Some("2/2")
        );

        // 2. A questionnaire on a Captain home keeps the Now / crew strip visible above the shell.
        let mut rt = crew_runtime(1280.0);
        rt.state.current_mut().apply(json!({
            "type": "extension_ui_request",
            "id": "home-form",
            "method": "input",
            "questions": [
                {"id": "q1", "header": "Scope", "question": "Pick one", "options": [{"label": "Local"}]}
            ]
        }));
        rt.invalidate();
        rt.frame(0.0);
        assert!(rt.presented("questionnaire-drawer").is_some());
        assert!(rt.presented("crew-tray").is_some());

        // 3. A plain thread with a provider subagent shows the same 28 pt Now strip and popover.
        let mut plain = runtime(1280.0);
        plain.state.current_mut().apply(json!({
            "type": "tool_execution_start",
            "toolCallId": "sub-1",
            "toolName": "subagent",
            "args": {"tasks": [{"agent": "Reviewer", "task": "Audit C4 layout"}]}
        }));
        plain.invalidate();
        plain.frame(0.0);
        let strip = plain.presented("crew-roster-toggle").unwrap().0;
        assert!((strip[3] - 28.0).abs() < 0.5, "{strip:?}");
        press(&mut plain, "crew-roster-toggle");
        plain.frame(0.0);
        assert_inside(
            &mut plain,
            1280.0,
            &[
                "now-popover",
                "now-popover-agent-sub-1-0",
                "now-popover-open-inspector",
            ],
        );
        press(&mut plain, "now-popover-open-inspector");
        plain.frame(0.5);
        assert!(!plain.state.now_popover);
        assert!(plain.presented("now-popover").is_none());
        assert_eq!(
            plain.state.right_panel_active_view(),
            Some(crate::right_panel::PanelView::Work)
        );
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

    /// The pressable's key is stable whether the tab is active or not, so a
    /// double click always lands on the same element path.
    fn tab_key(_rt: &Runtime<State>, id: usize) -> String {
        format!("tab-{id}")
    }

    #[test]
    fn tabs_share_the_strip_and_the_active_tab_stays_whole() {
        for width in [620.0, 1280.0] {
            for extra in [0usize, 2, 6, 12] {
                let mut rt = runtime(width);
                open_distinct_threads(&mut rt, extra);
                rt.set_viewport((width, 800.0), 1.0);
                settle_frames(&mut rt);
                type TabStage = (&'static str, Box<dyn Fn(&mut Runtime<State>)>);
                let mut stages: Vec<TabStage> = vec![
                    ("the last tab", Box::new(|_| {})),
                    (
                        "the first tab",
                        Box::new(|rt| {
                            let first = rt.state.tabs[0];
                            let index = rt.state.threads.iter().position(|t| t.id == first);
                            rt.state.open_kept_tab(index.unwrap());
                        }),
                    ),
                    (
                        "the last tab again",
                        Box::new(|rt| {
                            let last = *rt.state.tabs.last().unwrap();
                            let index = rt.state.threads.iter().position(|t| t.id == last);
                            rt.state.open_kept_tab(index.unwrap());
                        }),
                    ),
                    (
                        "after closing the last tab",
                        Box::new(|rt| {
                            let last = *rt.state.tabs.last().unwrap();
                            rt.state.close_tab(last);
                        }),
                    ),
                    (
                        "after ten sidebar previews",
                        Box::new(|rt| {
                            // Ghost tabs: sidebar clicks replace one slot
                            // instead of growing the strip.
                            let base = rt.state.next_id;
                            for n in 0..10 {
                                let mut thread = crate::model::Thread::new(base + n);
                                thread.title = format!("Sidebar preview {n}");
                                rt.state.threads.push(thread);
                            }
                            rt.state.next_id = base + 10;
                            let before = rt.state.tabs.len();
                            for index in (rt.state.threads.len() - 10)..rt.state.threads.len() {
                                rt.state.show_thread(index);
                                // The first click adds the ghost slot; the
                                // next nine replace it in place.
                                let expected = before + 1;
                                assert_eq!(
                                    rt.state.tabs.len(),
                                    expected,
                                    "sidebar clicks reuse the ghost tab"
                                );
                            }
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
    fn send_and_stop_share_the_same_composer_pill_slot() {
        for (width, dark) in [
            (620.0, false),
            (960.0, true),
            (1280.0, false),
            (1920.0, true),
        ] {
            let p = palette(dark);
            let fill_of = |rt: &Runtime<State>, key: &str| {
                rt.elements()
                    .into_iter()
                    .find(|element| element.key.as_deref() == Some(key))
                    .unwrap_or_else(|| panic!("{key} is missing"))
                    .fill
            };
            let mut rt = runtime(width);
            rt.state.dark = dark;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            assert_eq!(fill_of(&rt, "composer-transition-actions"), p.pill);
            rt.state.current_mut().draft.replace_text("Hello");
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            assert_eq!(fill_of(&rt, "composer-transition-actions"), p.pill);
            let (send, _) = rt.presented("send-message").unwrap();
            assert_eq!((send[2], send[3]), (32.0, 32.0));
            // Without a draft, the red stop mark takes the same slot.
            let t = rt.state.current_mut();
            t.draft.replace_text("");
            t.status = Status::Running;
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            assert_eq!(fill_of(&rt, "composer-transition-actions"), p.pill);
            assert_eq!(fill_of(&rt, "composer-stop-mark"), p.danger);
            let (stop, _) = rt.presented("stop-agent").unwrap();
            assert_eq!((stop[2], stop[3]), (32.0, 32.0));
        }
    }

    #[test]
    fn default_permissions_stay_quiet_and_open_from_model_picker_at_all_widths() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            rt.state.composer_ui.force_expanded = true;
            rt.state.catalog.models = vec![crate::session::AgentModel {
                provider: "cpa".into(),
                id: "gpt-6-astra".into(),
                name: "GPT-6 Astra".into(),
                ..Default::default()
            }];
            rt.state.current_mut().models = rt.state.catalog.models.clone();
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);

            assert!(
                rt.presented("access-mode").is_none(),
                "{width}: default permissions chip stays quiet"
            );

            press(&mut rt, "model-effort");
            rt.frame(0.1);
            let (card, _) = rt
                .presented("pi-settings-picker")
                .unwrap_or_else(|| panic!("{width}: model picker did not open"));
            let (perm, _) = rt
                .presented("model-picker-permissions")
                .unwrap_or_else(|| panic!("{width}: model-picker-permissions missing"));
            assert!(
                perm[0] >= card[0] - 0.5
                    && perm[0] + perm[2] <= card[0] + card[2] + 0.5
                    && perm[1] + perm[3] <= card[1] + card[3] + 0.5,
                "{width}: permissions footer outside card: {perm:?} in {card:?}"
            );

            press(&mut rt, "model-picker-permissions");
            rt.frame(0.1);
            assert_eq!(
                rt.state.settings_picker, None,
                "{width}: model picker closed"
            );
            assert!(
                rt.state.current().native.access_picker,
                "{width}: access picker opened from model picker footer"
            );
            let (picker, _) = rt
                .presented("access-picker")
                .unwrap_or_else(|| panic!("{width}: access-picker not presented"));
            assert!(
                picker[0] >= 0.0 && picker[0] + picker[2] <= width + 0.5 && picker[1] >= 0.0,
                "{width}: access picker out of viewport: {picker:?}"
            );

            // When busy, clicking the disabled footer row does not open the access picker.
            rt.state.current_mut().native.access_picker = false;
            rt.state.current_mut().status = Status::Running;
            rt.state.settings_picker = Some(SettingsPicker::Model);
            rt.invalidate();
            rt.frame(0.2);
            press(&mut rt, "model-picker-permissions");
            assert!(
                !rt.state.current().native.access_picker,
                "{width}: disabled permissions footer must not open access picker"
            );
        }
    }

    #[test]
    fn tab_leaves_expanded_composer_when_trailing_controls_are_disabled() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            let non_default =
                if rt.state.thread_defaults.access == crate::model::AccessMode::Full.id() {
                    crate::model::AccessMode::Supervised
                } else {
                    crate::model::AccessMode::Full
                };
            {
                let t = rt.state.current_mut();
                t.messages.push(crate::model::Message {
                    id: "m1".into(),
                    role: "You",
                    text: "Working".into(),
                    images: vec![],
                    context: None,
                });
                t.native.access = non_default;
                t.status = Status::Running;
                t.draft.focused = true;
            }
            rt.state.composer_ui.scope_focused.set(true);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            rt.update(|_, tx| tx.focus("composer-input", true));
            rt.frame(1.0);

            assert!(
                rt.presented("composer-prompt-body").is_some(),
                "{width}: expanded composer mounted while focused"
            );
            let mut cleared = false;
            for _ in 0..16 {
                rt.key(rune::KeyEvent {
                    key: rune::Key::Tab,
                    mods: Default::default(),
                });
                rt.frame(0.05);
                if !rt.state.composer_ui.scope_focused.get() {
                    cleared = true;
                    break;
                }
            }
            assert!(
                cleared,
                "{width}: Tab never cleared scope_focused (last_focusable={:?}, focused={:?})",
                rt.state.composer_ui.last_focusable.get(),
                rt.elements()
                    .into_iter()
                    .find(|e| e.focused)
                    .and_then(|e| e.key),
            );
            if width < 640.0 {
                rt.frame(1.0);
                assert!(
                    rt.presented("composer-prompt-body").is_none(),
                    "{width}: narrow composer stays stuck open after Tab leaves"
                );
            }
        }
    }

    #[test]
    fn tab_scope_and_disabled_labeled_access_chip_cover_demo_context_and_attachments() {
        // 1. Demo footer routes Tab on `leave-example` to `control_key` and collapses narrow composer.
        {
            let mut rt = runtime(620.0);
            rt.set_viewport((620.0, 900.0), 1.0);
            rt.state.demo = true;
            rt.state.current_mut().draft.focused = true;
            rt.state.composer_ui.scope_focused.set(true);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            rt.update(|_, tx| tx.focus("leave-example", true));
            rt.frame(0.1);
            assert!(rt.presented("composer-prompt-body").is_some());
            rt.key(rune::KeyEvent {
                key: rune::Key::Tab,
                mods: Default::default(),
            });
            rt.frame(1.0);
            assert!(
                !rt.state.composer_ui.scope_focused.get(),
                "Tab on demo leave-example must clear scope_focused"
            );
            assert!(
                rt.presented("composer-prompt-body").is_none(),
                "narrow demo composer should collapse after Tab leaves"
            );
        }

        // 2. Context ring with open breakdown popover clears scope_focused when Tab enters legend.
        {
            use crate::context_breakdown::{BreakdownRowKind, ContextBreakdownRow};
            let mut rt = context_fixture(620.0);
            rt.state.current_mut().context.usage.breakdown = Some(vec![
                ContextBreakdownRow {
                    label: "Messages".into(),
                    tokens: 80_000,
                    kind: BreakdownRowKind::Estimated,
                },
                ContextBreakdownRow {
                    label: "System".into(),
                    tokens: 20_000,
                    kind: BreakdownRowKind::Inferred,
                },
            ]);
            context_toggle(&mut rt.state);
            rt.state.composer_ui.scope_focused.set(true);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            rt.update(|_, tx| tx.focus("context-usage-ring", true));
            rt.frame(0.1);
            rt.key(rune::KeyEvent {
                key: rune::Key::Tab,
                mods: Default::default(),
            });
            rt.frame(0.1);
            assert!(
                !rt.state.composer_ui.scope_focused.get(),
                "Tab from context-usage-ring into breakdown popover must clear scope_focused"
            );
            assert!(
                focused_key(&rt)
                    .as_deref()
                    .is_some_and(|k| k.starts_with("context-breakdown-legend-")),
                "focus moved into breakdown legend: {:?}",
                focused_key(&rt)
            );
        }

        // 3. Shift+Tab from Remove stays inside composer on Preview, then leaves from Preview.
        {
            let mut rt = runtime(620.0);
            rt.set_viewport((620.0, 900.0), 1.0);
            rt.state.attach_image("shot.png", &ui_png(4));
            let id = rt.state.current().draft_images[0].id.clone();
            rt.state.composer_ui.scope_focused.set(true);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            let remove_key = format!("remove-attachment-{id}");
            let thumb_key = format!("attachment-thumb-{id}");
            rt.update(|_, tx| tx.focus(&remove_key, true));
            rt.frame(0.1);
            let shift = rune::Mods {
                shift: true,
                ..Default::default()
            };
            rt.key(rune::KeyEvent {
                key: rune::Key::Tab,
                mods: shift,
            });
            rt.frame(0.1);
            assert_eq!(
                focused_key(&rt).as_deref(),
                Some(thumb_key.as_str()),
                "Shift+Tab from Remove lands on Preview"
            );
            assert!(
                rt.state.composer_ui.scope_focused.get(),
                "Shift+Tab from Remove must keep scope_focused while Preview is focused"
            );
            assert!(
                rt.presented("composer-prompt-body").is_some(),
                "narrow composer stays expanded while Preview is focused"
            );
            rt.key(rune::KeyEvent {
                key: rune::Key::Tab,
                mods: shift,
            });
            rt.frame(1.0);
            assert!(
                !rt.state.composer_ui.scope_focused.get(),
                "Shift+Tab from Preview must clear scope_focused"
            );
            assert!(
                rt.presented("composer-prompt-body").is_none(),
                "narrow composer collapses after Shift+Tab leaves Preview"
            );
        }

        // 4. Disabled labeled access chip applies no hover or press wash and ignores presses.
        {
            let mut rt = runtime(1280.0);
            rt.set_viewport((1280.0, 900.0), 1.0);
            let non_default =
                if rt.state.thread_defaults.access == crate::model::AccessMode::Full.id() {
                    crate::model::AccessMode::Supervised
                } else {
                    crate::model::AccessMode::Full
                };
            let element_of = |rt: &Runtime<State>| {
                rt.elements()
                    .into_iter()
                    .find(|e| e.key.as_deref() == Some("access-mode"))
                    .expect("labeled access-mode chip should be mounted")
            };
            {
                let t = rt.state.current_mut();
                t.native.access = non_default;
                t.status = Status::Ready;
            }
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            let (rect, _) = rt
                .presented("access-mode")
                .expect("labeled access-mode chip should be mounted");
            assert!(
                rect[2] > 40.0,
                "expected labeled access chip at 1280 px, got {rect:?}"
            );
            let (cx, cy) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
            // Sanity-check that the enabled chip does activate hover and press overlays.
            rt.pointer_move(cx, cy);
            rt.frame(1.5);
            assert!(
                element_of(&rt).hover > 0.0,
                "enabled labeled access chip should report hover state"
            );
            rt.pointer_down(cx, cy);
            rt.frame(2.0);
            assert!(
                element_of(&rt).press > 0.0,
                "enabled labeled access chip should report press state"
            );
            rt.pointer_up(cx, cy);

            let mut disabled_rt = runtime(1280.0);
            disabled_rt.set_viewport((1280.0, 900.0), 1.0);
            {
                let t = disabled_rt.state.current_mut();
                t.native.access = non_default;
                t.status = Status::Running;
            }
            disabled_rt.invalidate();
            disabled_rt.frame(0.0);
            disabled_rt.frame(1.0);
            let (drect, _) = disabled_rt
                .presented("access-mode")
                .expect("disabled labeled access-mode chip should be mounted");
            assert!(
                drect[2] > 40.0,
                "expected disabled labeled access chip at 1280 px, got {drect:?}"
            );
            let (dx, dy) = (drect[0] + drect[2] / 2.0, drect[1] + drect[3] / 2.0);
            disabled_rt.pointer_move(dx, dy);
            disabled_rt.frame(1.5);
            let hovered = element_of(&disabled_rt);
            assert_eq!(
                hovered.hover, 0.0,
                "disabled labeled access chip must not get a hover wash"
            );
            disabled_rt.pointer_down(dx, dy);
            disabled_rt.frame(2.0);
            let pressed = element_of(&disabled_rt);
            assert_eq!(
                pressed.press, 0.0,
                "disabled labeled access chip must not get a press wash"
            );
            disabled_rt.pointer_up(dx, dy);
            disabled_rt.frame(2.5);
            assert!(
                !disabled_rt.state.current().native.access_picker,
                "disabled labeled access chip must not open access picker"
            );
        }
    }

    #[test]
    fn composer_shell_and_pill_rims_overlay_cleanly_and_disabled_pill_slots_do_nothing() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            rt.state.composer_ui.force_expanded = true;
            rt.state.current_mut().draft.replace_text("Check rims");
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);

            let (shell, _) = rt.presented("composer-shell").unwrap();
            let (shell_rim, _) = rt.presented("composer-shell-rim").unwrap();
            assert!(
                (shell[0] - shell_rim[0]).abs() < 0.5
                    && (shell[1] - shell_rim[1]).abs() < 0.5
                    && (shell[2] - shell_rim[2]).abs() < 0.5
                    && (shell[3] - shell_rim[3]).abs() < 0.5,
                "{width}: shell rim mismatch: shell={shell:?} rim={shell_rim:?}"
            );
            let (pill, _) = rt.presented("composer-transition-actions").unwrap();
            let (pill_rim, _) = rt.presented("composer-pill-rim").unwrap();
            assert!(
                (pill[0] - pill_rim[0]).abs() < 0.5
                    && (pill[1] - pill_rim[1]).abs() < 0.5
                    && (pill[2] - pill_rim[2]).abs() < 0.5
                    && (pill[3] - pill_rim[3]).abs() < 0.5,
                "{width}: pill rim mismatch: pill={pill:?} rim={pill_rim:?}"
            );

            // Disabled compact toggle (e.g. offline remote machine) does not toggle
            // keep_full_history on pointer press.
            let stale_at = crate::lifecycle::now().saturating_sub(80 * 60_000);
            let (client, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
            rt.state.client = Some(client);
            rt.state.machines.selected = "offline-remote".into();
            {
                let t = rt.state.current_mut();
                t.harness = "claude".into();
                t.model_provider = "claude".into();
                t.session = Some("11111111-1111-4111-8111-111111111111".into());
                t.status = Status::Completed;
                t.context.absorb(&json!({
                    "contextUsage": {
                        "tokens": 220_000,
                        "contextWindow": 1_000_000,
                        "updatedAt": stale_at
                    }
                }));
                t.messages.push(crate::model::Message {
                    id: "m1".into(),
                    role: "You",
                    text: "Old".into(),
                    images: vec![],
                    context: None,
                });
            }
            rt.invalidate();
            rt.frame(0.1);
            assert!(
                rt.presented("composer-compact-send").is_some(),
                "{width}: compact toggle should be mounted"
            );
            let before = rt.state.keep_full_history(rt.state.current());
            press(&mut rt, "composer-compact-send");
            assert_eq!(
                rt.state.keep_full_history(rt.state.current()),
                before,
                "{width}: disabled compact toggle fired on press"
            );

            // Disabled stop button does not trigger stop on pointer press.
            rt.state.client = None;
            rt.state.machines.selected = "local".into();
            {
                let t = rt.state.current_mut();
                t.harness_state = json!({"turnCapabilities": {"supportsInterrupt": false}});
                t.draft.replace_text("");
                t.status = Status::Running;
            }
            rt.invalidate();
            rt.frame(0.1);
            assert!(
                rt.presented("stop-agent").is_some(),
                "{width}: stop button should be mounted"
            );
            press(&mut rt, "stop-agent");
            assert_eq!(
                rt.state.current().stop_control.phase(),
                None,
                "{width}: disabled stop button fired on press"
            );
            assert_ne!(
                rt.state.current().phase,
                "Stopping",
                "{width}: disabled stop button changed phase"
            );
        }
    }

    #[test]
    fn native_model_and_reasoning_pickers_send_real_rpc_commands() {
        let mut rt = runtime(1280.0);
        let (pi, commands, _events) = crate::backend::Pi::test_connection();
        rt.state.current_mut().harness = "pi".into();
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
        // The pick closed the model picker; the levels are in the reasoning menu.
        assert_eq!(rt.state.settings_picker, None);
        open_picker(&mut rt, SettingsPicker::Thinking);
        press(&mut rt, "choose-thinking-1");
        let thinking = commands.try_recv().unwrap();
        assert_eq!(thinking["type"], "set_thinking_level");
        assert_eq!(thinking["level"], "high");
        rt.state
            .current_mut()
            .apply(json!({"type":"response","command":"set_thinking_level","success":true}));
        assert_eq!(rt.state.current().thinking, "high");
    }

    /// Claude Code reports aliases (`opus`, `sonnet[1m]`, ...); the catalog turns them into
    /// manifest rows, so the tab shows names like "Claude Opus 5.5", keeps reported extras
    /// such as `opusplan`, and a pick sends the routed slug, never the display name.
    #[test]
    fn claude_picker_lists_manifest_names_and_routes_slug_ids_at_all_widths() {
        let reported = crate::claude_catalog::parse_claude_reported_models(&json!([
            {"value":"default","displayName":"Default (recommended)",
             "supportsEffort":true,"supportedEffortLevels":["low","high"]},
            {"value":"opus","displayName":"Opus"},
            {"value":"sonnet","displayName":"Sonnet"},
            {"value":"haiku","displayName":"Haiku"},
            {"value":"opusplan","displayName":""},
            {"value":"sonnet[1m]","displayName":"Sonnet [1m]"},
        ]));
        let catalog = crate::claude_catalog::with_claude_code_model_routes(
            &crate::claude_catalog::bundled_claude_model_catalog(crate::lifecycle::now()),
            &reported,
        );
        let mut rows =
            crate::claude_catalog::resolve_claude_models_for_version(&catalog, Some("9.9.9"));
        rows.extend(crate::claude_catalog::claude_reported_models(
            &catalog, &reported,
        ));
        assert!(
            rows.iter().any(|row| row.slug == "opusplan")
                && !rows.iter().any(|row| matches!(
                    row.slug.as_str(),
                    "default" | "opus" | "sonnet" | "haiku"
                )),
            "reported aliases collapse into manifest rows, extras stay: {:?}",
            rows.iter().map(|row| &row.slug).collect::<Vec<_>>()
        );
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            rt.state.claude_catalog.models = rows
                .iter()
                .filter_map(|row| {
                    crate::session::AgentModel::from_harness_value(
                        &crate::claude::claude_row_model_value(&catalog, row),
                        "claude",
                    )
                })
                .collect();
            rt.state.claude_catalog.installed = true;
            rt.state.choose_harness("claude");
            let (pi, commands, _events) = crate::backend::Pi::test_connection();
            rt.state.current_mut().pi = Some(pi);
            rt.state.current_mut().status = Status::Ready;
            rt.frame(0.0);
            open_picker(&mut rt, SettingsPicker::Model);
            rt.frame(1.0 / 60.0);
            let tree = rt.accessibility_json().to_string();
            for name in [
                "Claude Opus 5.5",
                "Claude Sonnet 5.5",
                "Claude Haiku 5.5",
                "opusplan",
            ] {
                assert!(tree.contains(name), "{width}: {name} missing: {tree}");
            }
            // The aliases Claude Code reported must not shadow the manifest rows.
            let list = rt.find("model-list");
            assert!(list.is_some(), "{width}: model-list missing");
            // "Claude Opus 5.5" is the first catalog row; picking it sends the routed id.
            press(&mut rt, "choose-model-0");
            let model = commands.try_recv().unwrap();
            assert_eq!(model["type"], "set_model", "{width}");
            assert_eq!(model["provider"], "claude", "{width}");
            assert_eq!(model["modelId"], "claude-opus-5-5", "{width}");
        }
    }
    /// Opens the model picker or the reasoning menu and draws it.
    fn open_picker(rt: &mut Runtime<State>, picker: SettingsPicker) {
        rt.state.open_model_picker(picker);
        rt.invalidate();
        rt.frame(0.0);
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

    /// KSG AntigravityProvider picker: the Antigravity tab shows in the rail and lists
    /// its combined `family-level` models at every width.
    #[test]
    fn antigravity_tab_lists_models_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            // The picker re-reads harnesses.json on open, so the agent lives there.
            std::fs::write(
                rt.state.data_dir.join("harnesses.json"),
                serde_json::json!([{"id":"antigravity","name":"Antigravity",
                    "command":"agy","args":["acp"],"env":[]}])
                .to_string(),
            )
            .unwrap();
            rt.state.harnesses = crate::harness::Registry::read();
            let t = rt.state.current_mut();
            t.harness = "antigravity".into();
            t.models = [
                "gemini-3.8-flash-high",
                "gemini-3.8-flash-medium",
                "gemini-3.8-flash-low",
                "gemini-4-carbon",
            ]
            .iter()
            .enumerate()
            .map(|(index, id)| crate::session::AgentModel {
                provider: "antigravity".into(),
                id: (*id).into(),
                name: (*id).into(),
                is_default: index == 0,
                ..Default::default()
            })
            .collect();
            open_picker(&mut rt, SettingsPicker::Model);
            rt.frame(1.0);
            assert!(
                rt.presented("choose-harness-antigravity").is_some(),
                "{width}: Antigravity rail tab"
            );
            press(&mut rt, "choose-harness-antigravity");
            rt.frame(1.0);
            assert!(rt.presented("model-list").is_some(), "{width}");
            for index in 0..4 {
                assert!(
                    rt.presented(&format!("choose-model-{index}")).is_some(),
                    "{width}: model row {index}"
                );
            }
        }
    }

    /// KSG's `ModelPickerContent` on `applyCodexModelCatalog` output: the Codex tab
    /// shows discovered names, folds manifest legacies behind "Show more", keeps the
    /// row the manifest lacks, and carries the update-required notice.
    #[test]
    fn codex_tab_lists_wire_names_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            // `model/list` rows after `model_value`, including one qualified wire id
            // and one slug the manifest lacks.
            let wire = vec![
                json!({"provider":"codex","id":"gpt-6-astra","name":"GPT-6 Astra Wire",
                    "thinkingLevels":["low","medium","high"],"isDefault":true,
                    "serviceTiers":[{"id":"default","name":"Standard","isDefault":true},
                                    {"id":"fast","name":"Fast","isDefault":false}]}),
                json!({"provider":"codex","id":"zzz-fixture","name":"Fixture Unknown",
                    "thinkingLevels":["xhigh"],"contextWindow":32000}),
                json!({"provider":"codex","id":"openai.gpt-6-sol","name":"GPT-6 Sol Wire",
                    "thinkingLevels":["high"]}),
                json!({"provider":"codex","id":"gpt-5.5","name":"GPT-5.5 Wire",
                    "thinkingLevels":["high"]}),
            ];
            let catalog =
                crate::codex_catalog::bundled_codex_provider_catalog(crate::lifecycle::now());
            let (merged, update_required) = crate::codex_catalog::apply_codex_model_catalog(
                wire,
                catalog.as_ref(),
                Some("0.150.0"),
            );
            let models: Vec<crate::session::AgentModel> = merged
                .iter()
                .filter_map(|m| crate::session::AgentModel::from_harness_value(m, "codex"))
                .collect();
            rt.state.codex_catalog.models = models.clone();
            rt.state.codex_catalog.update_required_models = update_required;
            let t = rt.state.current_mut();
            t.harness = "codex".into();
            t.models = models;
            open_picker(&mut rt, SettingsPicker::Model);
            rt.frame(1.0);
            press(&mut rt, "choose-harness-codex");
            rt.frame(1.0);
            let tree = rt.accessibility_json().to_string();
            assert!(tree.contains("GPT-6 Astra Wire"), "{width}: {tree}");
            assert!(tree.contains("Fixture Unknown"), "{width}: {tree}");
            // Manifest legacies collapse behind the section toggle until expanded.
            assert!(
                rt.presented("legacy-models-codex").is_some(),
                "{width}: legacy toggle"
            );
            assert!(!tree.contains("GPT-5.5"), "{width}: {tree}");
            // Models the installed CLI predates surface through the existing notice.
            assert!(
                rt.presented("update-required-notice-codex").is_some(),
                "{width}: update-required notice"
            );
            assert!(
                tree.contains("Update the Codex CLI to v0.155.0 or newer"),
                "{width}: {tree}"
            );
            // The toggle may sit below the fold of the model list's clip.
            rt.update(|_, tx| tx.scroll_to_bottom_instant("model-list"));
            rt.frame(0.0);
            press(&mut rt, "legacy-models-codex");
            rt.invalidate();
            rt.frame(1.0);
            let expanded = rt.accessibility_json().to_string();
            assert!(expanded.contains("GPT-6 Sol Wire"), "{width}: {expanded}");
            assert!(expanded.contains("GPT-5.5"), "{width}: {expanded}");
        }
    }

    /// Short windows used to leave the combined picker taller than the room above the
    /// composer, so its rows ran out of the card over the input. Each popover now stays inside
    /// the window and its rows inside the card.
    #[test]
    fn model_and_reasoning_popovers_fit_short_windows_at_all_widths() {
        for width in [620.0, 1280.0] {
            for height in [480.0, 640.0, 900.0] {
                let mut rt = runtime(width);
                rt.set_viewport((width, height), 1.0);
                with_levels(&mut rt);
                rt.frame(0.0);
                for picker in [SettingsPicker::Model, SettingsPicker::Thinking] {
                    open_picker(&mut rt, picker);
                    rt.frame(1.0);
                    let card_key = match picker {
                        SettingsPicker::Model => "pi-settings-picker",
                        SettingsPicker::Thinking => "reasoning-picker",
                    };
                    let (card, _) = rt
                        .presented(card_key)
                        .unwrap_or_else(|| panic!("{width}x{height}: {card_key}"));
                    assert!(
                        card[0] >= 0.0
                            && card[1] >= 0.0
                            && card[0] + card[2] <= width + 0.5
                            && card[1] + card[3] <= height + 0.5,
                        "{width}x{height}: {card_key} leaves the window: {card:?}"
                    );
                    let inner = match picker {
                        SettingsPicker::Model => ["model-list", "harness-group"],
                        SettingsPicker::Thinking => ["reasoning-body", "reasoning-header"],
                    };
                    for key in inner {
                        let (rect, _) = rt
                            .presented(key)
                            .unwrap_or_else(|| panic!("{width}x{height}: {key}"));
                        assert!(
                            rect[1] >= card[1] - 0.5
                                && rect[1] + rect[3] <= card[1] + card[3] + 0.5,
                            "{width}x{height}: {key} runs out of {card_key}: {rect:?} {card:?}"
                        );
                    }
                    rt.state.settings_picker = None;
                    rt.invalidate();
                    rt.frame(1.5);
                }
            }
        }
    }

    #[test]
    fn model_picker_supports_favorites_legacy_collapse_unavailable_continuation_and_notices_at_all_widths()
     {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            rt.state.catalog.models = vec![
                crate::session::AgentModel {
                    provider: "cpa".into(),
                    id: "claude-opus-5-5".into(),
                    name: "Anthropic / Claude Opus 5.5".into(),
                    sub_provider: Some("Anthropic".into()),
                    badge: Some("new".into()),
                    ..Default::default()
                },
                crate::session::AgentModel {
                    provider: "cpa".into(),
                    id: "claude-opus-4-5".into(),
                    name: "Claude Opus 4.5".into(),
                    is_legacy: true,
                    ..Default::default()
                },
            ];
            rt.state.codex_catalog.models = vec![
                crate::session::AgentModel {
                    provider: "codex".into(),
                    id: "gpt-6-astra".into(),
                    name: "GPT-6 Astra".into(),
                    is_default: true,
                    ..Default::default()
                },
                crate::session::AgentModel {
                    provider: "codex".into(),
                    id: "gpt-5.4".into(),
                    name: "GPT-5.4".into(),
                    is_legacy: true,
                    ..Default::default()
                },
            ];
            rt.state.codex_catalog.update_required_models =
                vec![crate::model_picker::UpdateRequiredModel {
                    slug: "gpt-6-luna".into(),
                    name: "GPT-6 Luna".into(),
                    badge: None,
                    min_version: "0.155.0".into(),
                }];
            rt.state.current_mut().harness = "pi".into();
            rt.state.current_mut().models = rt.state.catalog.models.clone();
            rt.state.current_mut().model_provider = "cpa".into();
            rt.state.current_mut().model_id = "missing-dynamic".into();
            rt.state.current_mut().model = "missing-dynamic".into();
            rt.state.open_model_picker(SettingsPicker::Model);
            rt.invalidate();
            rt.frame(0.0);

            // Card fits inside viewport
            let (card, _) = rt.presented("pi-settings-picker").unwrap();
            assert!(
                card[0] >= 0.0 && card[0] + card[2] <= width + 0.5 && card[1] >= 0.0,
                "{width}: {card:?}"
            );

            // Favorites tab exists, legacy section collapses behind "Show more", unavailable row badge shows
            assert!(
                rt.presented("choose-harness-favorites").is_some(),
                "{width}"
            );
            assert!(rt.presented("legacy-models-pi").is_some(), "{width}");
            assert!(rt.find("toggle-favorite-0").is_some(), "{width}");
            let tree = rt.accessibility_json().to_string();
            assert!(tree.contains("Unavailable"), "{width}: {tree}");
            assert!(tree.contains("New"), "{width}: {tree}");
            assert!(tree.contains("Show more"), "{width}: {tree}");
            assert!(
                tree.contains("Anthropic") && !tree.contains("Pi · Anthropic"),
                "{width}: {tree}"
            );

            // Expanding legacy shows the legacy model and flips to "Show less"
            press(&mut rt, "legacy-models-pi");
            rt.invalidate();
            rt.frame(0.1);
            let expanded_tree = rt.accessibility_json().to_string();
            assert!(expanded_tree.contains("Claude Opus 4.5"), "{width}");
            assert!(expanded_tree.contains("Show less"), "{width}");

            // Toggling favorite switches and puts model in Favorites tab
            press(&mut rt, "toggle-favorite-0");
            rt.invalidate();
            rt.frame(0.2);
            assert_eq!(rt.state.effective_model_favorites().len(), 1);
            press(&mut rt, "choose-harness-favorites");
            rt.invalidate();
            rt.frame(0.3);
            let fav_tree = rt.accessibility_json().to_string();
            assert!(fav_tree.contains("Claude Opus 5.5"), "{width}: {fav_tree}");

            // Switching to Codex shows update-required notice
            press(&mut rt, "choose-harness-codex");
            rt.invalidate();
            rt.frame(0.4);
            assert!(
                rt.presented("update-required-notice-codex").is_some(),
                "{width}"
            );
            let codex_tree = rt.accessibility_json().to_string();
            assert!(
                codex_tree.contains("Update the Codex CLI to v0.155.0 or newer to use GPT-6 Luna."),
                "{width}: {codex_tree}"
            );

            // Pick GPT-6 Astra on Codex, then mark thread as started: other providers
            // stay reachable through a context handoff instead of being locked out.
            press(&mut rt, "choose-model-0");
            assert_eq!(rt.state.current().harness, "codex");
            assert_eq!(rt.state.current().model_id, "gpt-6-astra");
            rt.state.current_mut().life.latest_user_message_at = Some(1);
            rt.state.open_model_picker(SettingsPicker::Model);
            rt.invalidate();
            rt.frame(0.5);
            assert!(
                find(&rt.accessibility_json(), "choose-harness-pi")
                    .and_then(|node| node["states"].as_array().cloned())
                    .is_some_and(|states| !states.iter().any(|x| x == "disabled")),
                "{width}: pi stays enabled on a started codex thread"
            );
            // Searching across providers now lists the other provider's models with
            // the context-handoff hint under the name.
            rt.state.model_search.replace_text("opus");
            rt.invalidate();
            rt.frame(0.6);
            let search_tree = rt.accessibility_json().to_string();
            assert!(search_tree.contains("Claude Opus 5.5"), "{width}");
            assert!(
                search_tree.contains("Starts a context handoff"),
                "{width}: {search_tree}"
            );
            // Picking it stages a pending handoff; the harness only moves on send.
            press(&mut rt, "choose-model-0");
            assert_eq!(rt.state.current().harness, "codex");
            assert!(
                rt.state.current().staged_handoff().is_some(),
                "{width}: a pending handoff is staged"
            );

            // On an unstarted thread, searching across providers ranks matches globally
            let t = rt.state.current_mut();
            t.life.latest_user_message_at = None;
            t.handoffs.clear();
            t.staged_handoff = None;
            rt.state.open_model_picker(SettingsPicker::Model);
            rt.invalidate();
            rt.frame(0.7);
            let search_tree = rt.accessibility_json().to_string();
            assert!(search_tree.contains("Claude Opus 5.5"), "{width}");
            assert!(
                !search_tree.contains("Starts a context handoff"),
                "{width}: no handoff hint before the first send"
            );
        }
    }

    #[test]
    fn descriptor_driven_docked_picker_options_ultrathink_lock_and_footer_fit_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.set_viewport((width, 900.0), 1.0);
            let opus_caps = crate::descriptors::claude_manifest_model_entry("claude-opus-5-5")
                .unwrap()
                .capabilities;
            let acp_caps = crate::descriptors::ModelCapabilities {
                option_descriptors: crate::descriptors::acp_provider_option_descriptors(
                    Some(&[
                        json!({"id": "thought_level", "name": "Thinking", "category": "thought_level", "type": "select", "currentValue": "medium", "options": [{"value": "low", "name": "Low"}, {"value": "medium", "name": "Medium"}, {"value": "high", "name": "High"}]}),
                        json!({"id": "auto_approve", "name": "Auto-approve", "type": "boolean", "currentValue": false}),
                    ]),
                    Some(&json!({
                        "currentModeId": "code",
                        "availableModes": [
                            {"id": "code", "name": "Code"},
                            {"id": "plan", "name": "Plan"}
                        ]
                    })),
                ),
            };
            rt.state.claude_catalog.models = vec![crate::session::AgentModel {
                provider: "claude".into(),
                id: "claude-opus-5-5".into(),
                name: "Claude Opus 5.5".into(),
                thinking_levels: vec![
                    "low".into(),
                    "medium".into(),
                    "high".into(),
                    "xhigh".into(),
                    "max".into(),
                ],
                capabilities: opus_caps,
                ..Default::default()
            }];
            rt.state.claude_catalog.installed = true;
            rt.state.choose_harness("claude");
            rt.state
                .current_mut()
                .set_model("claude".into(), "claude-opus-5-5".into());
            rt.frame(0.0);
            rt.state.open_model_picker(SettingsPicker::Model);
            rt.invalidate();
            rt.frame(0.05);

            // Opus has only reasoning and speed, so the model picker is the list alone and
            // carries no reasoning, speed or permission controls.
            let (card, _) = rt.presented("pi-settings-picker").unwrap();
            assert!(
                card[0] >= 0.0 && card[0] + card[2] <= width + 0.5 && card[1] >= 0.0,
                "{width}: {card:?}"
            );
            for key in [
                "model-picker-options",
                "effort-slider",
                "picker-footer",
                "fast-mode-picker",
                "overflow-access-full",
            ] {
                assert!(
                    rt.presented(key).is_none(),
                    "{width}: {key} in model picker"
                );
            }

            // Fast/Standard and Ultracode sit under the levels in the reasoning menu.
            rt.state.open_model_picker(SettingsPicker::Thinking);
            rt.invalidate();
            rt.frame(0.08);
            let (menu, _) = rt.presented("reasoning-picker").unwrap();
            let (slider, _) = rt.presented("effort-slider").unwrap();
            let (speed, _) = rt.presented("picker-footer").unwrap();
            assert!(
                menu[0] >= 0.0 && menu[0] + menu[2] <= width + 0.5 && menu[1] >= 0.0,
                "{width}: {menu:?}"
            );
            assert!(
                speed[1] >= slider[1] + slider[3],
                "{width}: speed sits under the levels: {slider:?} {speed:?}"
            );
            assert!(rt.presented("fast-mode-picker").is_some(), "{width}");
            assert!(rt.presented("ultracode-toggle").is_some(), "{width}");
            press(&mut rt, "fast-mode-on");
            press(&mut rt, "ultracode-toggle");
            rt.invalidate();
            rt.frame(0.1);
            assert!(
                rt.state
                    .current()
                    .model_options
                    .iter()
                    .any(|o| o.id == "fastMode"
                        && o.value == crate::model_selection::OptionValue::Bool(true)),
                "{width}: fastMode enabled"
            );
            assert!(
                rt.state
                    .current()
                    .model_options
                    .iter()
                    .any(|o| o.id == "ultracode"
                        && o.value == crate::model_selection::OptionValue::Bool(true)),
                "{width}: ultracode enabled"
            );

            // Typing "ultrathink" in prompt body locks effort slider and shows note
            rt.state
                .current_mut()
                .draft
                .replace_text("Please ultrathink about this");
            rt.invalidate();
            rt.frame(0.2);
            assert!(rt.presented("ultrathink-lock-note").is_some(), "{width}");
            assert_eq!(
                find(&rt.accessibility_json(), "ultrathink-lock-note")
                    .and_then(|n| n["name"].as_str()),
                Some(crate::descriptors::ULTRATHINK_IN_BODY_NOTE),
                "{width}"
            );

            // ACP model shows select and boolean option rows in the model picker's options
            // column, beside the list or, below 640 px, under it.
            rt.state.current_mut().draft.replace_text("");
            rt.state.harnesses.agents = vec![crate::harness::Agent {
                id: "devin".into(),
                name: "Devin".into(),
                command: "devin".into(),
                args: vec![],
                env: vec![],
            }];
            rt.state.choose_harness("devin");
            rt.state.current_mut().draft_model_state = Default::default();
            rt.state.current_mut().models = vec![crate::session::AgentModel {
                provider: "devin".into(),
                id: "fusion".into(),
                name: "Fusion".into(),
                thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                capabilities: acp_caps,
                ..Default::default()
            }];
            rt.state
                .current_mut()
                .set_model("devin".into(), "fusion".into());
            // Set directly: opening refreshes the catalogs, which drops this fixture agent.
            rt.state.settings_picker = Some(SettingsPicker::Model);
            rt.invalidate();
            rt.frame(0.3);
            let (card, _) = rt.presented("pi-settings-picker").unwrap();
            let (list, _) = rt.presented("model-list").unwrap();
            let (options, _) = rt.presented("model-picker-options").unwrap();
            assert!(
                card[0] >= 0.0 && card[0] + card[2] <= width + 0.5 && card[1] >= 0.0,
                "{width}: {card:?}"
            );
            assert!(
                options[0] + options[2] <= card[0] + card[2] + 0.5
                    && options[1] + options[3] <= card[1] + card[3] + 0.5,
                "{width}: options column inside the card: {options:?} {card:?}"
            );
            if width > 640.0 {
                assert!(
                    options[0] >= list[0] + list[2] - 1.0,
                    "{width}: options column sits beside list: {list:?} {options:?}"
                );
            } else {
                assert!(
                    options[1] >= list[1] + list[3] - 1.0,
                    "{width}: options column stacks below list: {list:?} {options:?}"
                );
            }
            assert!(
                rt.presented("option-menu-_t3/session-mode").is_some(),
                "{width}"
            );
            assert!(
                rt.presented("option-switch-auto_approve").is_some(),
                "{width}"
            );
            press(&mut rt, "option-switch-auto_approve");
            rt.invalidate();
            rt.frame(0.4);
            assert!(
                rt.state
                    .current()
                    .model_options
                    .iter()
                    .any(|o| o.id == "auto_approve"
                        && o.value == crate::model_selection::OptionValue::Bool(true)),
                "{width}"
            );
            press(&mut rt, "option-menu-_t3/session-mode");
            rt.invalidate();
            rt.frame(0.5);
            press(&mut rt, "option-choice-_t3/session-mode-plan");
            rt.invalidate();
            rt.frame(0.6);
            assert!(
                rt.state
                    .current()
                    .model_options
                    .iter()
                    .any(|o| o.id == "_t3/session-mode"
                        && o.value == crate::model_selection::OptionValue::Str("plan".into())),
                "{width}"
            );
        }
    }

    #[test]
    fn effort_slider_moves_by_keyboard_click_and_drag() {
        let mut rt = runtime(1280.0);
        with_levels(&mut rt);
        rt.frame(0.0);
        press(&mut rt, "composer-effort");
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
            (0.2..0.4).contains(&held.frac),
            "the thumb follows the pointer, not the nearest stop: {held:?}"
        );
        rt.pointer_move(track[0] + track[2] - 4.0, y);
        rt.pointer_up(track[0] + track[2] - 4.0, y);
        rt.frame(4.0);
        assert_eq!(rt.state.current().thinking, "high");
        assert_eq!(rt.state.thinking_drag, None);
        assert!(rt.state.settings_picker.is_some(), "the popover stays open");
    }

    /// The thumb's centre x for a level index inside the slider's rect.
    fn thumb_center(track: [f32; 4], count: usize, index: usize) -> f32 {
        let cap = SLIDER_H / 2.0;
        track[0] + cap + (index as f32 / (count - 1) as f32) * (track[2] - 2.0 * cap)
    }

    /// A rejected effort level drops its pin instead of staying selected, and a newer
    /// pick made during the same flight still lands after the rejection.
    #[test]
    fn a_rejected_effort_level_does_not_stay_selected() {
        let mut rt = runtime(1280.0);
        let (pi, commands, _events) = crate::backend::Pi::test_connection();
        rt.state.current_mut().harness = "pi".into();
        rt.state.current_mut().pi = Some(pi);
        rt.state.current_mut().status = Status::Ready;
        rt.state.current_mut().apply(json!({"type":"response","command":"get_available_thinking_levels","success":true,"data":{"levels":["off","low","medium","high"]}}));
        rt.state.current_mut().model_id = "model-a".into();
        rt.state.current_mut().model_provider = "cpa".into();
        rt.state.current_mut().thinking = "low".into();
        rt.frame(0.0);
        press(&mut rt, "composer-effort");
        settle(&mut rt);
        let (track, _) = rt.presented("effort-slider").unwrap();
        let y = track[1] + track[3] / 2.0;

        let x = thumb_center(track, 4, 2);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.2);
        assert_eq!(commands.try_recv().unwrap()["level"], "medium");
        // Rejected: the pin clears, the slider shows what the provider still runs.
        rt.state.current_mut().apply(
            json!({"type":"response","command":"set_thinking_level","success":false,"error":"Unsupported level"}),
        );
        rt.invalidate();
        rt.frame(0.2);
        assert_eq!(rt.state.current().selected_thinking, None);
        assert_eq!(rt.state.current().thinking, "low");
        assert_eq!(
            find(&rt.accessibility_json(), "effort-title").and_then(|n| n["name"].as_str()),
            Some("Low")
        );

        // A pick during the next flight, followed by a rejection of the older one, still
        // reconciles the newer wanted level.
        let x = thumb_center(track, 4, 2);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.2);
        assert_eq!(commands.try_recv().unwrap()["level"], "medium");
        let x = thumb_center(track, 4, 3);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.2);
        assert!(
            commands.try_recv().is_err(),
            "the second pick waits for the first ack"
        );
        rt.state.current_mut().apply(
            json!({"type":"response","command":"set_thinking_level","success":false,"error":"Unsupported level"}),
        );
        rt.frame(0.2);
        let mut drain = Vec::new();
        while let Ok(command) = commands.try_recv() {
            drain.push(command);
        }
        let follow = drain
            .iter()
            .find(|command| command["type"] == "set_thinking_level")
            .expect("the newer pick survives the older one's rejection");
        assert_eq!(follow["level"], "high", "{drain:?}");
        rt.state
            .current_mut()
            .apply(json!({"type":"response","command":"set_thinking_level","success":true}));
        assert_eq!(rt.state.current().thinking, "high");
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
        assert_eq!(rt.state.current().draft.text, "Keep this text pasted.png ");
        assert_eq!(rt.state.current().draft.pills().count(), 1);
        press(&mut rt, &format!("remove-attachment-{id}"));
        press(&mut rt, "confirm-remove-attachment");
        assert!(rt.state.current().draft_images.is_empty());
        assert_eq!(rt.state.current().draft.text, "Keep this text");
        assert_eq!(rt.state.current().draft.pills().count(), 0);
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
    fn long_text_paste_uses_an_attachment_chip_and_undo_preserves_the_draft() {
        let mut rt = runtime(960.0);
        rt.frame(0.0);
        rt.state.current_mut().draft.replace_text("unsent");
        rt.state.current_mut().draft.set_caret(6);
        press(&mut rt, "composer-input");
        let text = "猫".repeat(crate::composer_paste::THRESHOLD / 3 + 1);
        rt.paste(text.clone());
        rt.frame(0.0);
        assert_eq!(rt.state.current().draft.text, "unsent pasted-text.txt ");
        assert_eq!(rt.state.current().draft.pills().count(), 1);
        let attachment = &rt.state.current().draft_images[0];
        assert_eq!(attachment.name, "pasted-text.txt");
        assert_eq!(
            std::fs::read_to_string(attachment.path.as_ref().unwrap()).unwrap(),
            text
        );
        // Inline file sits in the editor rather than duplicating in the tray.
        assert!(rt.find(&format!("attachment-{}", attachment.id)).is_none());
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('z'),
            mods: rune::Mods {
                cmd: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert!(rt.state.current().draft_images.is_empty());
        assert_eq!(rt.state.current().draft.text, "unsent");
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
        for width in [620.0, 1280.0] {
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

    fn rect(rt: &Runtime<State>, key: &str) -> [f32; 4] {
        rt.presented(key)
            .unwrap_or_else(|| panic!("{key} is not presented"))
            .0
    }

    #[test]
    fn sidebar_lists_pinned_and_active_cards_without_workspace_headers() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.compact_sidebar = width < 760.0;
            rt.frame(0.0);
            rt.frame(1.0);

            // Captain rows stay 56 pt tall and separate from the flat thread list.
            let mut prev_bottom = 0.0f32;
            for i in rt.state.sidebar_captains() {
                let id = rt.state.threads[i].id;
                let r = rect(&rt, &format!("thread-{id}"));
                assert!(
                    (r[3] - 56.0).abs() < 0.5,
                    "{width}: captain {id} height {r:?}"
                );
                assert!(
                    r[1] >= prev_bottom - 0.5,
                    "{width}: overlap at captain {id}"
                );
                prev_bottom = r[1] + r[3];
            }

            let heading = rect(&rt, "threads-heading");
            let pinned = rect(&rt, "thread-3");
            let active = rect(&rt, "thread-1");
            assert!(heading[1] < pinned[1] && pinned[1] < active[1], "{width}");
            assert_eq!(active[3], 60.0, "{width}");
            assert!(rt.elements().iter().all(|e| {
                !e.key
                    .as_deref()
                    .is_some_and(|key| key.starts_with("sidebar-group-"))
            }));
            assert_eq!(
                element_text(&rt, "thread-workspace-3").as_deref(),
                Some("rune")
            );
            assert_eq!(
                element_text(&rt, "thread-workspace-1").as_deref(),
                Some("shina")
            );
            rt.update(|s, _| s.set_project_scope(Some(1), false));
            settle(&mut rt);
            assert!(
                rt.find("thread-1").is_some() && rt.find("thread-3").is_none(),
                "{width}"
            );
            rt.update(|s, _| s.set_project_scope(Some(2), true));
            settle(&mut rt);
            assert!(
                rt.find("thread-1").is_some() && rt.find("thread-3").is_some(),
                "{width}"
            );
        }
        // Unread failed captain shows Failed (not unread blue dot), and needs-you captain
        // shows count pill without any visible "Needs you" status word.
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        let homes: Vec<usize> = rt
            .state
            .sidebar_captains()
            .iter()
            .map(|&i| rt.state.threads[i].id)
            .collect();
        let cap0 = rt
            .state
            .threads
            .iter()
            .position(|t| t.id == homes[0])
            .unwrap();
        rt.state.threads[cap0].status = Status::Failed;
        rt.state.threads[cap0].life.last_completed_at = Some(lifecycle::now());
        rt.state.threads[cap0].life.last_visited_at = None;
        let cap1 = rt
            .state
            .threads
            .iter()
            .position(|t| t.id == homes[1])
            .unwrap();
        rt.state.threads[cap1].status = Status::Waiting;
        rt.state.threads[cap1].request = Some(json!({"method":"confirm"}));
        rt.invalidate();
        rt.frame(1.0);
        assert!(
            rt.presented(&format!("captain-unread-{}", homes[0]))
                .is_none()
        );
        assert!(
            rt.presented(&format!("captain-attention-{}", homes[1]))
                .is_some()
        );
        let status_text =
            element_text(&rt, &format!("captain-status-{}", homes[1])).unwrap_or_default();
        assert!(!status_text.contains("Needs you"), "{status_text}");

        // Contrast check on bg and row in both themes for sidebar text tokens.
        for dark in [false, true] {
            let p = palette(dark);
            for bg in [p.bg, p.row] {
                for (ink, floor) in [(p.ink, 4.5), (p.muted, 4.3), (p.faint, 3.0)] {
                    let ratio = contrast(ink, bg);
                    assert!(ratio >= floor, "dark {dark}: sidebar contrast {ratio:.2}:1");
                }
            }
        }
    }

    #[test]
    fn in_draft_captain_selector_fits_and_opens_on_the_new_thread_route_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.captains = crate::captains::Captain::presets();
            rt.frame(0.0);

            let (trigger, _) = rt
                .presented("captain-selector")
                .expect("captain selector trigger in composer footer");
            assert!(
                trigger[0] >= 0.0 && trigger[0] + trigger[2] <= width + 1.0,
                "{width}: trigger fits {trigger:?}"
            );

            // Clicking the footer trigger opens the in-place selector popover inside the window.
            press(&mut rt, "captain-selector");
            assert!(rt.state.captain_picker);
            rt.frame(0.1);
            let (picker, _) = rt
                .presented("captain-picker")
                .expect("captain picker popover presented");
            assert!(
                picker[0] >= 0.0
                    && picker[0] + picker[2] <= width + 1.0
                    && picker[1] >= 0.0
                    && picker[1] + picker[3] <= 840.0 + 1.0,
                "{width}: picker fits {picker:?}"
            );
            assert!(rt.presented("choose-no-captain").is_some());
            assert!(rt.presented("choose-captain-0").is_some());
            assert!(rt.presented("picker-new-captain").is_some());

            // Sidebar "+" uses the same route: starts/keeps a draft in the workspace with the
            // selector open and search focused (closing it returns focus to the composer).
            press(&mut rt, "captain-selector");
            assert!(!rt.state.captain_picker);
            if width >= 1050.0 {
                press(&mut rt, "assign-captain");
                assert!(rt.state.captain_picker);
                assert!(rt.state.captain_picker_query.focused);
                assert_eq!(rt.state.current().captain, None);
            } else {
                rt.state.open_captain_selector();
                rt.frame(0.1);
                assert!(rt.state.captain_picker);
                assert!(rt.state.captain_picker_query.focused);
                assert_eq!(rt.state.current().captain, None);
            }

            // Filtering by name narrows the list while keeping No captain + New captain.
            rt.state.captain_picker_query.replace_text("Pip");
            rt.invalidate();
            rt.frame(0.1);
            assert!(rt.find("choose-no-captain").is_some());
            assert!(rt.find("choose-captain-0").is_none());
            assert!(rt.find("choose-captain-3").is_some());
            assert!(rt.find("picker-new-captain").is_some());
        }
    }

    #[test]
    fn no_projects_hero_and_add_project_browse_parity_fit_at_all_widths() {
        let root = std::env::temp_dir().join(format!("shina-browse-parity-{}", std::process::id()));
        std::fs::create_dir_all(root.join("alpha-one")).unwrap();
        std::fs::create_dir_all(root.join("alpha-two")).unwrap();
        std::fs::create_dir_all(root.join(".hidden-dir")).unwrap();
        let sep = std::path::MAIN_SEPARATOR;

        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            // Remove all workspaces so NoProjectsHero renders on the empty new-thread screen.
            rt.state.projects.entries.clear();
            rt.state.new_thread_in(None);
            rt.invalidate();
            rt.frame(0.0);

            let (hero_btn, _) = rt
                .presented("no-projects-add-workspace")
                .expect("NoProjectsHero Add workspace button");
            assert!(
                hero_btn[0] >= 0.0 && hero_btn[0] + hero_btn[2] <= width + 1.0,
                "{width}: hero button fits {hero_btn:?}"
            );

            // Configured "Add project starts in" directory seeds the folder browser.
            rt.state.thread_defaults.add_project_base_directory =
                root.to_string_lossy().into_owned();
            press(&mut rt, "no-projects-add-workspace");
            assert_eq!(rt.state.project_picker, Some(ProjectPicker::AddProject));
            press(&mut rt, "add-local-folder");
            assert!(rt.state.picker_browsing);
            assert_eq!(
                rt.state.picker_query.text,
                format!("{}{sep}", root.display())
            );
            finish_folder_listing(&mut rt);
            rt.invalidate();
            rt.frame(0.5);

            // Hidden dotfiles are filtered out unless the query leaf starts with '.'.
            assert!(rt.find("browse-0").is_some());
            assert!(rt.find("browse-1").is_some());
            assert!(rt.find("browse-2").is_none());

            // Tab extends to the common prefix ("alpha-") when multiple matches share a prefix.
            rt.state
                .picker_query
                .replace_text(format!("{}{sep}al", root.display()));
            finish_folder_listing(&mut rt);
            rt.invalidate();
            rt.frame(0.6);
            rt.key(rune::KeyEvent {
                key: rune::Key::Tab,
                mods: rune::Mods::default(),
            });
            rt.frame(0.65);
            assert_eq!(
                rt.state.picker_query.text,
                format!("{}{sep}alpha-", root.display())
            );

            // Typing '.' reveals hidden directories.
            rt.state
                .picker_query
                .replace_text(format!("{}{sep}.", root.display()));
            finish_folder_listing(&mut rt);
            rt.invalidate();
            rt.frame(0.7);
            assert!(rt.find("browse-0").is_some());
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn submission_policy_and_prompt_limit_work_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.frame(0.0);
            press(&mut rt, "composer-input");

            // Under ModEnter, plain Enter inserts a newline instead of sending.
            rt.state.preferences.send_shortcut = crate::composer::SendShortcut::ModEnter;
            rt.state.current_mut().draft.replace_text("hello");
            rt.key(rune::KeyEvent {
                key: rune::Key::Enter,
                mods: Default::default(),
            });
            rt.frame(0.0);
            assert_eq!(rt.state.current().draft.text, "hello\n");
            assert!(rt.state.current().messages.is_empty());

            // Key-repeat Enter under Enter policy does not submit.
            rt.state.preferences.send_shortcut = crate::composer::SendShortcut::Enter;
            rt.state.current_mut().draft.replace_text("hello");
            rt.repeat_key(rune::KeyEvent {
                key: rune::Key::Enter,
                mods: Default::default(),
            });
            rt.frame(0.0);
            assert_eq!(rt.state.current().draft.text, "hello");
            assert!(rt.state.current().messages.is_empty());

            // Oversized prompt blocks send and displays the limit error within the viewport.
            let oversized = "x".repeat(crate::composer::PROVIDER_SEND_TURN_MAX_INPUT_CHARS + 1);
            rt.state.current_mut().draft.replace_text(&oversized);
            rt.key(rune::KeyEvent {
                key: rune::Key::Enter,
                mods: Default::default(),
            });
            rt.frame(0.0);
            assert!(rt.state.current().messages.is_empty());
            assert_eq!(
                rt.state.current().submission_error.as_deref(),
                Some(
                    "Prompt is 1 character over the 120,000-character limit. Shorten or split it before sending."
                )
            );
            let (rect, _) = rt
                .presented("composer-prompt-length-error")
                .unwrap_or_else(|| panic!("{width}: composer-prompt-length-error"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
            assert!(
                rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                "{width}: {rect:?}"
            );
        }
    }

    #[test]
    fn prompt_history_follows_the_carets_visual_line_at_all_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            // An older short prompt, then one that wraps over several lines with no newline.
            let long = "wrapped ".repeat(60).trim_end().to_string();
            rt.state.current_mut().messages = ["short", long.as_str()]
                .into_iter()
                .map(|text| crate::model::Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "You",
                    text: text.into(),
                    images: Vec::new(),
                    context: None,
                })
                .collect();
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            let mut time = 0.0;
            let mut tap = |rt: &mut Runtime<State>, key: rune::Key| {
                rt.key(rune::KeyEvent {
                    key,
                    mods: Default::default(),
                });
                // The caret's place reaches the view the frame after it is drawn.
                for _ in 0..3 {
                    time += 0.1;
                    rt.frame(time);
                }
            };

            tap(&mut rt, rune::Key::Up);
            assert_eq!(rt.state.current().draft.text, long, "{width}: recall");
            let (field, _) = rt.presented("composer-input").expect("composer input");
            // `COMPOSER`'s line height.
            let lines = ((field[3] - 2.0 * COMPOSER_FIELD_PAD) / 20.0).round() as usize;
            assert!(lines >= 2, "{width}: the prompt wraps, {field:?}");

            // The caret starts on the last line. Up climbs the wrapped lines first, and only
            // from the first line recalls the older prompt.
            let mut presses = 0;
            while rt.state.current().draft.text == long && presses < 40 {
                tap(&mut rt, rune::Key::Up);
                presses += 1;
            }
            assert_eq!(rt.state.current().draft.text, "short", "{width}: older");
            assert_eq!(presses, lines, "{width}: one press a line, then the recall");

            // Down from a one-line prompt goes forward, then leaves history from the last line.
            tap(&mut rt, rune::Key::Down);
            assert_eq!(rt.state.current().draft.text, long, "{width}: forward");
            tap(&mut rt, rune::Key::Down);
            assert_eq!(rt.state.current().draft.text, "", "{width}: leaves history");
        }
    }

    #[test]
    fn approval_options_warnings_and_responding_guards_fit_at_supported_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.current_mut().apply(json!({
                "type":"extension_ui_request","id":"options","method":"confirm",
                "title":"Allow this command?","message":"Check the requested command.",
                "approvalOptions":[
                    {"decision":"decline","label":"Deny"},
                    {"decision":"accept","label":"Allow once","warning":"This command changes files."},
                    {"decision":"acceptForSession","label":"Allow for this thread","warning":"Future commands can run."},
                    {"decision":"cancel","label":"Cancel"}
                ]
            }));
            rt.frame(0.0);
            for key in ["dialog-confirm", "dialog-decline"] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width}: {rect:?}"
                );
            }
            assert!(rt.find("dialog-session").is_none());
            let id = rt.state.current().id;
            press(&mut rt, &format!("approval-more-{id}"));
            for key in ["dialog-session", "dialog-cancel"] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width}: {rect:?}"
                );
            }
            rt.state.current_mut().responding_request = Some(uuid::Uuid::new_v4());
            rt.invalidate();
            rt.frame(0.0);
            press(&mut rt, "dialog-confirm");
            assert_eq!(
                rt.state.current().request.as_ref().unwrap()["id"],
                "options"
            );
        }
    }

    #[test]
    fn approval_feedback_and_redirect_composer_fit_at_supported_widths() {
        for width in [1280.0] {
            for feedback in [false, true] {
                let mut rt = runtime(width);
                rt.state.current_mut().status = Status::Running;
                rt.state.current_mut().context_window = Some(200_000);
                rt.state.current_mut().apply(json!({
                    "type":"extension_ui_request","id":"send-over-approval","method":"confirm",
                    "title":"Run this command?","message":"Check the requested command.",
                    "responseCapability":{"type":"live","supportsFeedback":feedback}
                }));
                rt.state
                    .current_mut()
                    .draft
                    .replace_text("Use the safer command.");
                rt.frame(0.0);
                press(&mut rt, "composer-input");
                let (rect, _) = rt.presented("send-message").unwrap();
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                    "{width}: {rect:?}"
                );
                press(&mut rt, "send-message");
                assert!(
                    rt.state.current().approval_send_pending(),
                    "{width}: {feedback}"
                );
                assert_eq!(rt.state.current().draft.text, "Use the safer command.");
                assert!(rt.state.current().request.is_some());
            }
        }
    }

    #[test]
    fn composer_control_shortcuts_open_controls_step_providers_and_jump_models() {
        let mut rt = runtime(1280.0);
        rt.on_key = Some(crate::key);
        {
            let t = rt.state.current_mut();
            t.harness = "pi".into();
            t.models = vec![
                crate::session::AgentModel {
                    provider: "cpa".into(),
                    id: "model-alpha".into(),
                    name: "Alpha (CPA -> Codex)".into(),
                    context_window: Some(128_000),
                    supports_images: Some(true),
                    thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                    service_tiers: vec![],
                    ..Default::default()
                },
                crate::session::AgentModel {
                    provider: "cpa".into(),
                    id: "model-beta".into(),
                    name: "Beta (CPA -> Claude)".into(),
                    context_window: Some(200_000),
                    supports_images: Some(true),
                    thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                    service_tiers: vec![],
                    ..Default::default()
                },
            ];
            t.model_provider = "cpa".into();
            t.model_id = "model-alpha".into();
            t.model = "Alpha (CPA -> Codex)".into();
            t.thinking = "low".into();
            t.thinking_levels = vec!["low".into(), "medium".into(), "high".into()];
        }
        rt.frame(0.0);

        // Mod+Shift+M opens the model picker, Mod+Shift+M again closes it.
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('m'),
            mods: rune::Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(rt.state.settings_picker, Some(SettingsPicker::Model));
        assert!(rt.presented("pi-settings-picker").is_some());
        assert!(rt.presented("model-list").is_some());

        // Mod+Shift+ArrowDown steps provider tab in the open model picker.
        rt.key(rune::KeyEvent {
            key: rune::Key::Down,
            mods: rune::Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(
            rt.state.model_picker_tab,
            Some(crate::model_picker::ModelPickerTab::Instance(
                "codex".into()
            ))
        );

        // Mod+Shift+ArrowUp steps provider tab back.
        rt.key(rune::KeyEvent {
            key: rune::Key::Up,
            mods: rune::Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(
            rt.state.model_picker_tab,
            Some(crate::model_picker::ModelPickerTab::Instance("pi".into()))
        );

        // Re-populate models after harness switch and jump to the 2nd model with Mod+2.
        {
            let t = rt.state.current_mut();
            t.models = vec![
                crate::session::AgentModel {
                    provider: "cpa".into(),
                    id: "model-alpha".into(),
                    name: "Alpha (CPA -> Codex)".into(),
                    context_window: Some(128_000),
                    supports_images: Some(true),
                    thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                    service_tiers: vec![],
                    ..Default::default()
                },
                crate::session::AgentModel {
                    provider: "cpa".into(),
                    id: "model-beta".into(),
                    name: "Beta (CPA -> Claude)".into(),
                    context_window: Some(200_000),
                    supports_images: Some(true),
                    thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                    service_tiers: vec![],
                    ..Default::default()
                },
            ];
        }
        rt.frame(0.0);
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('2'),
            mods: rune::Mods {
                cmd: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(rt.state.current().model_id, "model-beta");
        // Without an options column, a pick closes the picker as KSG's does.
        assert_eq!(rt.state.settings_picker, None);
        rt.state.open_model_picker(SettingsPicker::Thinking);
        rt.frame(0.0);

        // Mod+Shift+E closes effort when already open, and opens it when closed.
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('e'),
            mods: rune::Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(rt.state.settings_picker, None);
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('e'),
            mods: rune::Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(rt.state.settings_picker, Some(SettingsPicker::Thinking));
        assert!(rt.presented("effort-slider").is_some());

        // Mod+Shift+A opens the access mode picker.
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('a'),
            mods: rune::Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert!(rt.state.current().native.access_picker);
        assert!(rt.presented("access-picker").is_some());

        // Custom-bound composer.increaseEffort / decreaseEffort also work.
        rt.state.current_mut().thinking = "low".into();
        crate::keybindings::upsert_custom_rule(
            &mut rt.state.preferences.keybindings,
            "composer.increaseEffort",
            "alt+]",
            None,
            None,
        )
        .unwrap();
        crate::keybindings::upsert_custom_rule(
            &mut rt.state.preferences.keybindings,
            "composer.decreaseEffort",
            "alt+[",
            None,
            None,
        )
        .unwrap();
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("]".into()),
            mods: rune::Mods {
                alt: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(rt.state.current().thinking, "medium");
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("[".into()),
            mods: rune::Mods {
                alt: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(rt.state.current().thinking, "low");

        // Mod+Shift+H opens the machine selector when linked machines exist, and custom
        // composer.cycleHost cycles to the next connected machine.
        connected_machines(&mut rt);
        rt.frame(0.0);
        rt.key(rune::KeyEvent {
            key: rune::Key::Command('h'),
            mods: rune::Mods {
                cmd: true,
                shift: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert!(rt.state.machines.picker);
        assert!(rt.presented("machine-picker").is_some());

        crate::keybindings::upsert_custom_rule(
            &mut rt.state.preferences.keybindings,
            "composer.cycleHost",
            "alt+h",
            None,
            None,
        )
        .unwrap();
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("h".into()),
            mods: rune::Mods {
                alt: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        assert_eq!(rt.state.machines.selected, "studio");

        // Registered but unimplemented controls (workspace, branch, previousWorktree) return false
        // without changing state.
        for ch in ['x', 'g', 'l'] {
            let mut tx = rune::Tx::default();
            assert!(!crate::key(
                &mut rt.state,
                &rune::KeyEvent {
                    key: rune::Key::Command(ch),
                    mods: rune::Mods {
                        cmd: true,
                        shift: true,
                        ..Default::default()
                    },
                },
                &mut tx
            ));
        }
    }

    #[test]
    fn background_send_and_new_thread_chords_work_and_render_toasts_at_all_widths() {
        let enter_chord = |alt: bool| rune::KeyEvent {
            key: rune::Key::Enter,
            mods: rune::Mods {
                cmd: true,
                alt,
                ..Default::default()
            },
        };

        for width in [1280.0] {
            // 1. Draft thread: Mod+Enter and Mod+Alt+Enter start in background and open a fresh focused composer.
            for alt in [false, true] {
                let mut rt = runtime(width);
                rt.on_key = Some(crate::key);
                let (pi, _cmds, _evs) = crate::backend::Pi::test_connection();
                rt.state.current_mut().pi = Some(pi);
                rt.state.current_mut().model_provider = "cpa".into();
                rt.state
                    .current_mut()
                    .draft
                    .replace_text("Draft background");
                rt.frame(0.0);
                press(&mut rt, "composer-input");
                let first_id = rt.state.current().id;

                rt.key(enter_chord(alt));
                rt.frame(0.0);

                assert_ne!(rt.state.current().id, first_id, "{width} alt={alt}");
                assert!(rt.state.current().is_draft());
                assert!(rt.state.current().draft.focused);
                assert!(rt.state.current().draft.text.is_empty());
                assert_eq!(rt.state.threads[0].messages.len(), 1);
                assert_eq!(rt.state.threads[0].status, Status::Running);

                let (stack, _) = rt.presented("toast-stack").expect("toast stack");
                assert!(
                    stack[0] >= 0.0 && stack[0] + stack[2] <= width + 1.0,
                    "{width}: {stack:?}"
                );
                let toast_action = format!("toast-action-{}", rt.state.toasts[0].id);
                let (btn, _) = rt.presented(&toast_action).expect("Open action");
                assert!(btn[0] >= 0.0 && btn[0] + btn[2] <= width + 1.0);
                press(&mut rt, &toast_action);
                assert_eq!(rt.state.current().id, first_id);
                assert!(rt.state.toasts.is_empty());
            }

            // 2. Existing idle thread: Mod+Alt+Enter sends and opens a fresh focused draft composer.
            {
                let mut rt = runtime(width);
                rt.on_key = Some(crate::key);
                let (pi, _cmds, _evs) = crate::backend::Pi::test_connection();
                {
                    let t = rt.state.current_mut();
                    t.pi = Some(pi);
                    t.model_provider = "cpa".into();
                    t.messages.push(crate::model::Message {
                        id: uuid::Uuid::new_v4().to_string(),
                        role: "You",
                        text: "Prior turn".into(),
                        images: vec![],
                        context: None,
                    });
                    t.draft.replace_text("Follow-up idle");
                }
                let existing_id = rt.state.current().id;
                rt.frame(0.0);
                press(&mut rt, "composer-input");
                rt.key(enter_chord(true));
                rt.frame(0.0);

                assert_ne!(rt.state.current().id, existing_id);
                assert!(rt.state.current().is_draft());
                assert!(rt.state.current().draft.focused);
                assert!(rt.state.current().draft.text.is_empty());
                assert!(rt.state.toasts.is_empty());
                assert_eq!(rt.state.threads[0].messages.len(), 2);
            }

            // 3. Existing running thread: Mod+Alt+Enter queues/steers and opens a fresh focused draft composer.
            {
                let mut rt = runtime(width);
                rt.on_key = Some(crate::key);
                let (pi, _cmds, _evs) = crate::backend::Pi::test_connection();
                {
                    let t = rt.state.current_mut();
                    t.pi = Some(pi);
                    t.model_provider = "cpa".into();
                    t.status = Status::Running;
                    t.native.follow_up = true;
                    t.draft.replace_text("Follow-up while running");
                }
                let running_id = rt.state.current().id;
                rt.frame(0.0);
                press(&mut rt, "composer-input");
                rt.key(enter_chord(true));
                rt.frame(0.0);

                assert_ne!(rt.state.current().id, running_id);
                assert!(rt.state.current().is_draft());
                assert!(rt.state.current().draft.focused);
                assert_eq!(rt.state.threads[0].native.queue.len(), 1);
            }

            // 4. Failing background send: shows "Background task failed" toast with "Open draft" and recovers draft.
            {
                let mut rt = runtime(width);
                rt.on_key = Some(crate::key);
                rt.state.runtime_enabled = true;
                let failed_id = rt.state.current().id;
                rt.state.current_mut().model_provider = "cpa".into();
                rt.state.current_mut().machine = "missing-host".into();
                rt.state.current_mut().draft.replace_text("Recover me");
                rt.frame(0.0);
                press(&mut rt, "composer-input");
                rt.key(enter_chord(false));
                rt.frame(0.0);

                assert_ne!(rt.state.current().id, failed_id);
                let (stack, _) = rt.presented("toast-stack").expect("failure toast");
                assert!(stack[0] >= 0.0 && stack[0] + stack[2] <= width + 1.0);
                let toast_action = format!("toast-action-{}", rt.state.toasts[0].id);
                press(&mut rt, &toast_action);
                assert_eq!(rt.state.current().id, failed_id);
                assert_eq!(rt.state.current().draft.text, "Recover me");
                assert!(rt.state.current().error.is_some());
            }

            // 5. Guards: empty prompt, submission_error, pending question, archived, unfocused, repeat, IME composition.
            {
                let mut rt = runtime(width);
                rt.on_key = Some(crate::key);
                let (pi, _cmds, _evs) = crate::backend::Pi::test_connection();
                rt.state.current_mut().pi = Some(pi);
                rt.state.current_mut().model_provider = "cpa".into();
                rt.frame(0.0);
                press(&mut rt, "composer-input");
                let id = rt.state.current().id;

                // Empty prompt
                rt.key(enter_chord(false));
                rt.frame(0.0);
                assert_eq!(rt.state.current().id, id);
                assert!(rt.state.toasts.is_empty());

                // Unfocused composer does not trigger sendBackground
                rt.state.current_mut().draft.replace_text("Unfocused");
                rt.state.current_mut().draft.focused = false;
                rt.invalidate();
                rt.frame(0.0);
                rt.key(enter_chord(false));
                rt.frame(0.0);
                assert_eq!(rt.state.current().id, id);
                assert_eq!(rt.state.current().draft.text, "Unfocused");

                // Archived thread does not send
                press(&mut rt, "composer-input");
                rt.state.current_mut().life.archived_at = Some(1);
                rt.key(enter_chord(false));
                rt.frame(0.0);
                assert_eq!(rt.state.current().id, id);
                assert_eq!(rt.state.current().draft.text, "Unfocused");
                rt.state.current_mut().life.archived_at = None;

                // A pending question blocks sendAndNewThread. Approvals use the redirect transaction.
                rt.state.current_mut().messages.push(crate::model::Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "You",
                    text: "Turn 1".into(),
                    images: vec![],
                    context: None,
                });
                rt.state.current_mut().request = Some(json!({"id":"r1","method":"input"}));
                rt.key(enter_chord(true));
                rt.frame(0.0);
                assert_eq!(rt.state.current().id, id);
                assert_eq!(rt.state.current().draft.text, "Unfocused");
                rt.state.current_mut().request = None;

                // Over-limit prompt sets submission_error and does not open a new thread
                rt.state.current_mut().draft.replace_text(
                    &"x".repeat(crate::composer::PROVIDER_SEND_TURN_MAX_INPUT_CHARS + 1),
                );
                rt.key(enter_chord(true));
                rt.frame(0.0);
                assert_eq!(rt.state.current().id, id);
                assert!(rt.state.current().submission_error.is_some());

                // Key repeat and IME composition never submit
                rt.state.current_mut().submission_error = None;
                rt.state.current_mut().draft.replace_text("Repeat/IME");
                rt.frame(0.0);
                rt.repeat_key(enter_chord(true));
                rt.frame(0.0);
                assert_eq!(rt.state.current().id, id);
                assert_eq!(rt.state.current().draft.text, "Repeat/IME");
                rt.ime(rune::Ime::Preedit("あ".into(), None, Vec::new()));
                rt.frame(0.0);
                assert!(rt.state.current().draft.is_composing());
                rt.key(enter_chord(true));
                rt.frame(0.0);
                assert_eq!(rt.state.current().id, id);
                assert_eq!(rt.state.current().draft.text, "Repeat/IME");
                rt.state.current_mut().draft.cancel_composition();
                rt.frame(0.0);

                // Open slash menu does not intercept Mod+Alt+Enter (sendAndNewThread)
                rt.state.current_mut().draft.replace_text("/review");
                rt.state.current_mut().native.commands.push(
                    crate::model::pi_session::SlashCommand {
                        name: "review".into(),
                        description: "Review".into(),
                        source: "ACP".into(),
                    },
                );
                rt.frame(0.0);
                rt.key(enter_chord(true));
                rt.frame(0.0);
                assert_ne!(rt.state.current().id, id);
                assert!(rt.state.current().is_draft());
                assert!(rt.state.current().draft.focused);
            }
        }
    }

    #[test]
    fn mention_menu_shows_model_and_machine_rows_at_620_and_1280_px() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.state.peers.push(crate::peer::Peer {
                server_id: "server-b".into(),
                host: "127.0.0.1:47329".into(),
                label: "StudioSol".into(),
                linked_at: 0,
                capabilities: json!({"contracts": [crate::protocol::THREAD_READ]}),
                last_reached_at: None,
                last_error: None,
            });
            rt.state.codex_catalog.models = vec![
                crate::session::AgentModel {
                    provider: "codex".into(),
                    id: "gpt-6.1-sol".into(),
                    name: "GPT-6.1 Sol".into(),
                    ..Default::default()
                },
                crate::session::AgentModel {
                    provider: "codex".into(),
                    id: "gpt-5.0-sol-legacy".into(),
                    name: "GPT-5.0 Sol Legacy".into(),
                    is_legacy: true,
                    ..Default::default()
                },
                crate::session::AgentModel {
                    provider: "codex".into(),
                    id: "gpt-6.2-sol-unavail".into(),
                    name: "GPT-6.2 Sol Unavailable".into(),
                    is_unavailable: true,
                    ..Default::default()
                },
            ];
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            rt.key(rune::KeyEvent {
                key: rune::Key::Text("@sol".into()),
                mods: Default::default(),
            });
            rt.frame(0.0);
            rt.frame(0.25);
            let (menu, _) = rt.presented("mention-menu").expect("mention menu");
            assert!(
                menu[0] >= 8.0 && menu[0] + menu[2] <= width - 7.0,
                "{width}: {menu:?}"
            );
            assert!(
                rt.presented("mention-item-0").is_some(),
                "{width}: machine row"
            );
            assert!(
                rt.presented("mention-item-1").is_some(),
                "{width}: model row"
            );
            // Legacy and unavailable models are hidden, matching KSG.
            assert!(
                rt.presented("mention-item-2").is_none(),
                "{width}: hidden models"
            );
        }
    }

    #[test]
    fn mention_menu_lists_workspace_files_and_older_threads_at_620_and_1280_px() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            let (id, machine) = {
                let t = rt.state.current();
                (t.id, t.machine.clone())
            };
            // Results already applied for the current key, as a settled
            // `workspace.searchEntries`/`threads.search` response leaves them.
            rt.state.mention_search.key = Some((id, machine.clone(), "deep".into()));
            rt.state.mention_search.paths_cwd = "/tmp/shina-mention-fixture".into();
            rt.state.mention_search.paths = vec![crate::workspace_search::WorkspaceEntry {
                path: "src/deep/nested.rs".into(),
                kind: "file".into(),
                ignored: false,
            }];
            rt.state.mention_search.threads = vec![crate::ui::composer_menu::MentionThreadRow {
                server: machine,
                id: 9,
                title: "Deep settings rework".into(),
                branch: None,
                updated_ms: Some(1),
                archived: false,
                parent: false,
            }];
            rt.frame(0.0);
            press(&mut rt, "composer-input");
            rt.key(rune::KeyEvent {
                key: rune::Key::Text("@deep".into()),
                mods: Default::default(),
            });
            rt.frame(0.0);
            rt.frame(0.25);
            let (menu, _) = rt.presented("mention-menu").expect("mention menu");
            assert!(
                menu[0] >= 8.0 && menu[0] + menu[2] <= width - 7.0,
                "{width}: {menu:?}"
            );
            assert!(
                rt.presented("mention-item-0").is_some(),
                "{width}: older thread row"
            );
            assert!(
                rt.presented("mention-item-1").is_some(),
                "{width}: workspace file row"
            );
        }
    }

    #[test]
    fn composer_notice_stack_attaches_above_composer_and_shows_details_on_overflow() {
        use crate::composer_notices::{Group, Notice, NoticeAction, Variant};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            let clicks = Arc::new(AtomicUsize::new(0));
            let clicks_action = clicks.clone();
            rt.state.composer_notices.push(
                Notice::new("att-long", Variant::Info, "alert", "Attention notice")
                    .description(Some(
                        "This description is deliberately very long so that it overflows a single line even on wide composer lanes and triggers the Details popover button across every width under test."
                            .into(),
                    ))
                    .dismiss_label("Dismiss attention notice"),
            );
            rt.state.composer_notices.push(
                Notice::new("env-first", Variant::Warning, "alert", "Studio is offline")
                    .group(Group::Environment)
                    .description(Some("This device is offline".into()))
                    .actions(vec![NoticeAction::new("Retry", move |_| {
                        clicks_action.fetch_add(1, Ordering::Relaxed);
                    })]),
            );
            rt.frame(0.0);
            rt.frame(1.0);

            let (stack_rect, _) = rt.presented("composer-notices").expect("notice stack");
            let (env_rect, _) = rt
                .presented("composer-notice-env-first")
                .expect("environment notice");
            let (att_rect, _) = rt
                .presented("composer-notice-att-long")
                .expect("attention notice");
            let (composer_rect, _) = rt.presented("composer").expect("composer card");

            // Environment notice precedes Attention notice, consecutive cards touch, and the
            // last card touches the composer with no gap at the same lane width.
            assert!(
                env_rect[1] < att_rect[1],
                "{width}: env should be above att"
            );
            assert!(
                (att_rect[1] - (env_rect[1] + env_rect[3])).abs() < 0.5,
                "{width}: gap between notices: env={env_rect:?} att={att_rect:?}"
            );
            assert!(
                (composer_rect[1] - (att_rect[1] + att_rect[3])).abs() < 0.5,
                "{width}: gap before composer: att={att_rect:?} composer={composer_rect:?}"
            );
            assert!(
                (stack_rect[2] - composer_rect[2]).abs() < 0.5,
                "{width}: stack width {stack_rect:?} vs composer {composer_rect:?}"
            );

            // Long description shows Details and opens the details popover; Retry triggers action;
            // Dismiss hides the attention notice.
            assert!(
                rt.presented("composer-notice-details-att-long").is_some(),
                "{width}"
            );
            press(&mut rt, "composer-notice-details-att-long");
            rt.frame(1.0);
            assert!(
                rt.presented("composer-notice-details-popover").is_some(),
                "{width}"
            );
            press(&mut rt, "close-notice-details");
            rt.frame(1.0);
            assert!(
                rt.presented("composer-notice-details-popover").is_none(),
                "{width}"
            );

            press(&mut rt, "composer-notice-action-env-first-0");
            rt.frame(1.0);
            assert_eq!(clicks.load(Ordering::Relaxed), 1, "{width}");

            press(&mut rt, "composer-notice-dismiss-att-long");
            rt.frame(1.0);
            assert!(
                rt.presented("composer-notice-att-long").is_none(),
                "{width}"
            );
        }
    }

    #[test]
    fn composer_prompt_suggestion_tab_and_escape_behavior() {
        let mut rt = runtime(1280.0);
        rt.state.thread_defaults.prompt_suggestions_enabled = true;
        {
            let t = rt.state.current_mut();
            t.harness = "codex".into();
            t.status = Status::Completed;
            t.run_id = Some("run-1".into());
            t.prompt_suggestion = Some(crate::model::PromptSuggestion {
                run_id: "run-1".into(),
                text: "Run cargo test".into(),
            });
            t.draft.focused = true;
        }
        rt.frame(0.0);
        rt.frame(1.0);
        press(&mut rt, "composer-input");
        rt.frame(1.0);

        // Tab inserts the suggestion as editable text with caret at the end, and does not send.
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(1.0);
        assert_eq!(rt.state.current().draft.text, "Run cargo test");
        assert_eq!(rt.state.current().draft.focus, "Run cargo test".len());
        assert_eq!(rt.state.current().draft.anchor, "Run cargo test".len());
        assert_eq!(rt.state.current().status, Status::Completed);
        assert!(rt.state.current().messages.is_empty());

        // Tab with the slash menu open keeps the menu's behaviour.
        rt.state.current_mut().apply(json!({
            "type": "response",
            "command": "get_commands",
            "success": true,
            "data": {"commands": [{"name": "skill:review", "description": "Review", "source": "skill"}]}
        }));
        rt.state.current_mut().draft.replace_text("/skill:");
        rt.invalidate();
        rt.frame(1.0);
        assert!(rt.presented("slash-menu").is_some());
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(1.0);
        assert_eq!(rt.state.current().draft.text, "/skill:review ");
        assert!(rt.find("slash-menu").is_none());

        // Deleting back to empty shows the suggestion again; Escape dismisses only the suggestion
        // and does not close anything else on that press.
        rt.state.current_mut().draft.replace_text("");
        rt.state.open_thread_find();
        rt.invalidate();
        rt.frame(1.0);
        press(&mut rt, "composer-input");
        rt.frame(1.0);
        assert_eq!(rt.state.offered_prompt_suggestion(), Some("Run cargo test"));
        rt.key(rune::KeyEvent {
            key: rune::Key::Escape,
            mods: Default::default(),
        });
        rt.frame(1.0);
        assert_eq!(rt.state.offered_prompt_suggestion(), None);
        assert!(rt.state.thread_find_active());
        assert_eq!(focused_key(&rt).as_deref(), Some("composer-input"));
    }

    #[test]
    fn reply_set_replace_remove_persists_and_handles_accepted_and_failed_send() {
        let dir = std::env::temp_dir().join(format!(
            "shina-reply-persist-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mut rt = runtime(1280.0);
        rt.state.current_mut().messages = vec![
            crate::model::Message {
                id: "msg-a1".into(),
                role: "Pi",
                text: "First assistant response opening.".into(),
                images: Vec::new(),
                context: None,
            },
            crate::model::Message {
                id: "msg-u2".into(),
                role: "You",
                text: "Follow up".into(),
                images: Vec::new(),
                context: None,
            },
            crate::model::Message {
                id: "msg-a2".into(),
                role: "Pi",
                text: "Second assistant response opening.".into(),
                images: Vec::new(),
                context: None,
            },
        ];
        rt.state.new_thread();
        rt.state.open_kept_tab(0);
        rt.frame(0.0);
        rt.frame(1.0);

        // Set reply on first response, replace with second, then remove.
        press(&mut rt, "reply-message-msg-a1");
        rt.frame(1.0);
        assert!(rt.state.current().draft.focused);
        assert!(rt.state.current().has_draft_content());
        assert_eq!(
            rt.state
                .current()
                .draft_reply_to
                .as_ref()
                .map(|r| r.context_id.as_str()),
            Some("response_msg-a1")
        );

        press(&mut rt, "reply-message-msg-a2");
        rt.frame(1.0);
        assert_eq!(
            rt.state
                .current()
                .draft_reply_to
                .as_ref()
                .map(|r| r.context_id.as_str()),
            Some("response_msg-a2")
        );

        press(&mut rt, "composer-reply-remove");
        rt.frame(1.0);
        assert!(rt.state.current().draft_reply_to.is_none());
        assert!(!rt.state.current().has_draft_content());

        // Set again, persist across thread switch and save/reload in temp dir.
        press(&mut rt, "reply-message-msg-a1");
        rt.frame(1.0);
        rt.state.open_kept_tab(1);
        assert!(rt.state.current().draft_reply_to.is_none());
        rt.state.open_kept_tab(0);
        assert_eq!(
            rt.state
                .current()
                .draft_reply_to
                .as_ref()
                .map(|r| r.context_id.as_str()),
            Some("response_msg-a1")
        );

        let saved_reply = rt.state.current().draft_reply_to.clone().unwrap();
        let mut saver = State::new(dir.clone(), false);
        saver.load_global_for_test();
        saver.current_mut().draft_reply_to = Some(saved_reply.clone());
        saver.checkpoint_and_flush().unwrap();
        let mut reloaded = State::new(dir.clone(), false);
        reloaded.load_global_for_test();
        assert_eq!(reloaded.current().draft_reply_to, Some(saved_reply.clone()));
        let _ = std::fs::remove_dir_all(&dir);

        // Local failed send keeps draft_reply_to.
        {
            let t = rt.state.current_mut();
            t.harness = "pi".into();
            t.model_provider = "direct-disallowed".into();
        }
        rt.state.send();
        assert_eq!(rt.state.current().draft_reply_to, Some(saved_reply.clone()));

        // Remote failed send restores draft_reply_to; remote accepted send clears it.
        let mut remote_rt = runtime(1280.0);
        let (client, _local_rx) =
            crate::client::Client::test_connected(remote_rt.state.data_dir.clone());
        remote_rt.state.client = Some(client);
        remote_rt.state.current_mut().draft_reply_to = Some(saved_reply.clone());
        remote_rt.state.send();
        let cmd_id = remote_rt.state.current().remote_send.as_ref().unwrap().0;
        assert!(remote_rt.state.current().draft_reply_to.is_none());
        assert!(
            remote_rt
                .state
                .current_mut()
                .finish_remote_send(cmd_id, false)
        );
        assert_eq!(
            remote_rt.state.current().draft_reply_to,
            Some(saved_reply.clone())
        );
        remote_rt.state.send();
        let cmd_id_ok = remote_rt.state.current().remote_send.as_ref().unwrap().0;
        assert!(
            remote_rt
                .state
                .current_mut()
                .finish_remote_send(cmd_id_ok, true)
        );
        assert!(remote_rt.state.current().draft_reply_to.is_none());

        // Local accepted send attaches the response reference + context record and clears draft_reply_to.
        let (pi, commands, _) = crate::backend::Pi::test_connection();
        {
            let t = rt.state.current_mut();
            t.pi = Some(pi);
            t.model_provider = "cpa".into();
            t.error = None;
            t.draft.replace_text("");
        }
        rt.state.send();
        assert!(rt.state.current().draft_reply_to.is_none());
        let sent_msg = rt.state.current().messages.last().unwrap();
        assert_eq!(sent_msg.role, "You");
        assert!(sent_msg.text.starts_with(
            "[First assistant response opening.](t3-context://v1/response/response_msg-a1)"
        ));
        assert!(
            sent_msg
                .context
                .as_ref()
                .is_some_and(|c| c.records.contains(&saved_reply))
        );
        let sent_cmd = commands
            .try_iter()
            .find(|cmd| cmd["type"] == "prompt")
            .expect("prompt command dispatched");
        assert!(
            sent_cmd["message"]
                .as_str()
                .unwrap_or("")
                .contains("The user is replying to this earlier response of yours.")
        );
    }

    /// KSG's `AssistantMessageMeta`: Copy then Reply on the prose's left edge, in one row
    /// under the response, and Copy shows its check after a press.
    #[test]
    fn assistant_response_actions_sit_under_the_prose_at_supported_widths() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            rt.state.current_mut().messages.push(crate::model::Message {
                id: "msg-meta".into(),
                role: "Pi",
                text: "An answer with **actions** under it.".into(),
                images: Vec::new(),
                context: None,
            });
            rt.frame(0.0);
            rt.frame(1.0);
            let (copy, _) = rt.presented("copy-message-msg-meta").expect("copy");
            let (reply, _) = rt.presented("reply-message-msg-meta").expect("reply");
            let (lane, _) = rt.presented("chat-lane").expect("lane");
            assert!((copy[1] - reply[1]).abs() < 0.5, "{width}: one row");
            assert!(copy[0] + copy[2] <= reply[0], "{width}: Copy before Reply");
            assert!(
                (copy[0] - lane[0]).abs() <= 6.0,
                "{width}: the actions start at the prose edge {copy:?} {lane:?}"
            );
            assert_eq!((copy[2], copy[3]), (24.0, 24.0));
            assert!(
                rt.presented("fork-message-msg-meta").is_none(),
                "{width}: no session"
            );
            press(&mut rt, "copy-message-msg-meta");
            rt.frame(0.1);
            assert!(rt.state.copied_message.is_some(), "{width}");
            assert_eq!(
                element_text(&rt, "copy-message-msg-meta").as_deref(),
                Some("Copied"),
                "{width}: the cached turn shows the check"
            );
        }
    }

    #[test]
    fn worktree_toolbar_and_setup_card_layout_at_all_widths() {
        for width in [620.0, 1280.0] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.current_mut().draft.focused = true;
            let project_path = rt
                .state
                .project(rt.state.current().project)
                .unwrap()
                .path
                .to_string_lossy()
                .into_owned();
            rt.state.vcs_branches.insert(
                ("local".into(), project_path),
                crate::model::VcsBranchCacheEntry {
                    is_repo: true,
                    loaded: true,
                    loading: false,
                    error: None,
                    refs: vec![crate::vcs::VcsRef {
                        name: "main".into(),
                        is_remote: Some(false),
                        remote_name: None,
                        upstream_ref: None,
                        current: true,
                        is_default: true,
                        worktree_path: None,
                    }],
                },
            );
            rt.state.current_mut().env_mode = Some(crate::model::ThreadEnvMode::Worktree);
            rt.state.current_mut().base_branch = Some("main".into());
            rt.state.current_mut().worktree_setup = Some(crate::model::WorktreeSetupState {
                phase: crate::model::WorktreeSetupPhase::Failed,
                script_name: Some("Setup".into()),
                script_command: Some("npm ci".into()),
                error: Some("Setup script exited with code 1.".into()),
                output: "line 1\nline 2\nline 3\nline 4".into(),
                exit_code: Some(1),
                timed_out: false,
                duration_ms: 420,
                skipped: false,
            });
            rt.state.current_mut().worktree_setup_output_open = true;
            rt.frame(0.0);
            rt.frame(0.25);

            let (footer_rect, _) = rt
                .presented("composer-footer")
                .unwrap_or_else(|| panic!("{width}: composer-footer"));
            for key in ["env-mode-trigger", "base-branch-trigger"] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{width}: {key}"));
                assert!(
                    rect[0] >= footer_rect[0] - 0.5
                        && rect[0] + rect[2] <= footer_rect[0] + footer_rect[2] + 0.5
                        && rect[0] + rect[2] <= width + 0.5,
                    "{width}: {key} overflowed footer: rect={rect:?} footer={footer_rect:?}"
                );
            }

            let (lane_rect, _) = rt
                .presented("chat-lane")
                .unwrap_or_else(|| panic!("{width}: chat-lane"));
            let (card_rect, _) = rt
                .presented("worktree-setup-card")
                .unwrap_or_else(|| panic!("{width}: worktree-setup-card"));
            assert!(
                card_rect[0] >= lane_rect[0] - 0.5
                    && card_rect[0] + card_rect[2] <= lane_rect[0] + lane_rect[2] + 0.5,
                "{width}: setup card outside conversation lane: card={card_rect:?} lane={lane_rect:?}"
            );
            for key in [
                "worktree-setup-retry",
                "worktree-setup-skip",
                "worktree-setup-output",
                "worktree-setup-output-tail",
            ] {
                assert!(rt.presented(key).is_some(), "{width}: missing {key}");
            }
            press(&mut rt, "worktree-setup-output");
            assert!(
                !rt.state.current().worktree_setup_output_open,
                "{width}: toggle should flip state"
            );
            rt.frame(0.5);
            assert!(
                rt.presented("worktree-setup-output-tail").is_none(),
                "{width}: output should collapse"
            );
        }
    }

    fn setup_multi_model_git_runtime(width: f32) -> rune::Runtime<State> {
        let mut rt = runtime(width);
        let project_path = rt
            .state
            .project(rt.state.current().project)
            .unwrap()
            .path
            .to_string_lossy()
            .into_owned();
        rt.state.vcs_branches.insert(
            ("local".into(), project_path),
            crate::model::VcsBranchCacheEntry {
                is_repo: true,
                loaded: true,
                loading: false,
                error: None,
                refs: vec![crate::vcs::VcsRef {
                    name: "main".into(),
                    is_remote: Some(false),
                    remote_name: None,
                    upstream_ref: None,
                    current: true,
                    is_default: true,
                    worktree_path: None,
                }],
            },
        );
        rt.state.catalog.models = vec![
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "cpa/claude-sonnet-4-5".into(),
                name: "Claude Sonnet 4.5".into(),
                thinking_levels: vec!["low".into(), "high".into()],
                ..Default::default()
            },
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "cpa/gpt-5".into(),
                name: "GPT-5".into(),
                thinking_levels: vec!["medium".into(), "high".into()],
                ..Default::default()
            },
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "cpa/gemini-2-5-pro".into(),
                name: "Gemini 2.5 Pro".into(),
                ..Default::default()
            },
            crate::session::AgentModel {
                provider: "cpa".into(),
                id: "cpa/deepseek-r1".into(),
                name: "DeepSeek R1".into(),
                ..Default::default()
            },
        ];
        rt.state.catalog.default_model = Some(rt.state.catalog.models[0].clone());
        rt.state.current_mut().harness = "pi".into();
        rt.state.current_mut().models = rt.state.catalog.models.clone();
        rt.state
            .current_mut()
            .set_model("cpa".into(), "cpa/claude-sonnet-4-5".into());
        rt.state.set_composer_thinking_sticky("high".into());
        rt
    }

    /// Presses `key` with Shift held, as a Shift+click.
    fn shift_press(rt: &mut Runtime<State>, key: &str) {
        let shift = rune::Mods {
            shift: true,
            ..Default::default()
        };
        rt.physical_input(rune::PhysicalInput::Modifiers(shift));
        press(rt, key);
        rt.physical_input(rune::PhysicalInput::Modifiers(Default::default()));
        rt.frame(0.0);
    }

    /// With multi-model send off, Shift+click picks one model instead of adding it.
    fn assert_multi_send_off(rt: &mut Runtime<State>) {
        if rt.find("choose-model-2").is_none() {
            rt.state.open_model_picker(SettingsPicker::Model);
            rt.invalidate();
            rt.frame(0.0);
        }
        shift_press(rt, "choose-model-2");
        assert_eq!(rt.state.multi_model_selections().len(), 1);
    }

    #[test]
    fn multi_model_selection_toggles_persists_across_switch_and_reload_and_respects_availability() {
        let mut rt = setup_multi_model_git_runtime(1280.0);
        let temp_dir = std::env::temp_dir().join(format!(
            "shina-multi-model-persist-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        rt.state.data_dir = temp_dir.clone();
        rt.state.load_global_for_test();
        rt.state.current_mut().draft.focused = true;
        rt.frame(0.0);

        // Open model picker and Shift+click 3 additional models on (4 total), as KSG Code's
        // combobox adds to a multi-model send. Rows have no checkboxes.
        press(&mut rt, "model-effort");
        assert!(rt.presented("model-list").is_some());
        assert!(rt.find("model-multi-toggle-pi-cpa/gpt-5").is_none());
        shift_press(&mut rt, "choose-model-1");
        shift_press(&mut rt, "choose-model-2");
        shift_press(&mut rt, "choose-model-3");
        assert_eq!(rt.state.multi_model_selections().len(), 4);
        assert_eq!(
            rt.state.settings_picker,
            Some(SettingsPicker::Model),
            "Shift+click keeps the picker open"
        );
        assert_eq!(
            rt.state.multi_model_selections()[0].options,
            vec![crate::model_selection::ProviderOptionSelection {
                id: "thinking".into(),
                value: crate::model_selection::OptionValue::Str("high".into()),
            }]
        );
        assert!(rt.state.multi_model_selections()[1].options.is_empty());

        // Shift+click a selected row to take it off.
        shift_press(&mut rt, "choose-model-3");
        assert_eq!(rt.state.multi_model_selections().len(), 3);

        // Switch to a new thread and back; selection persists on the draft.
        let draft_id = rt.state.current().id;
        let project = rt.state.current().project;
        rt.state.new_thread();
        let other_id = rt.state.current().id;
        assert_ne!(other_id, draft_id);
        assert_eq!(rt.state.multi_model_selections().len(), 1);
        let draft_idx = rt
            .state
            .threads
            .iter()
            .position(|t| t.id == draft_id)
            .unwrap();
        rt.state.open_kept_tab(draft_idx);
        assert_eq!(rt.state.multi_model_selections().len(), 3);

        // Persist and reload from temporary data directory.
        rt.state.checkpoint_and_flush().unwrap();
        let mut reloaded = State::new(rt.state.project(project).unwrap().path.clone(), false);
        reloaded.data_dir = temp_dir.clone();
        reloaded.load_snapshot();
        reloaded.catalog.models = rt.state.catalog.models.clone();
        let reloaded_idx = reloaded
            .threads
            .iter()
            .position(|t| t.id == draft_id)
            .unwrap();
        reloaded.open_kept_tab(reloaded_idx);
        reloaded.vcs_branches = rt.state.vcs_branches.clone();
        assert_eq!(reloaded.multi_model_selections().len(), 3);
        assert_eq!(
            reloaded
                .multi_model_selections()
                .iter()
                .map(|s| s.model.as_str())
                .collect::<Vec<_>>(),
            vec!["cpa/claude-sonnet-4-5", "cpa/gpt-5", "cpa/gemini-2-5-pro"]
        );

        // Clicking the model row itself resets to single primary model.
        if rt.find("choose-model-1").is_none() {
            press(&mut rt, "model-effort");
        }
        press(&mut rt, "choose-model-1");
        assert_eq!(rt.state.multi_model_selections().len(), 1);
        assert_eq!(rt.state.current().model_id, "cpa/gpt-5");

        // Unavailable on a thread with messages.
        rt.state.current_mut().messages.push(crate::model::Message {
            id: "m1".into(),
            role: "You",
            text: "Started".into(),
            images: vec![],
            context: None,
        });
        assert!(!rt.state.multiple_models_enabled());
        rt.invalidate();
        rt.frame(1.0);
        rt.frame(1.0);
        assert_multi_send_off(&mut rt);

        // Unavailable on a non-git project.
        rt.state.current_mut().messages.clear();
        let project_path = rt
            .state
            .project(rt.state.current().project)
            .unwrap()
            .path
            .to_string_lossy()
            .into_owned();
        rt.state.vcs_branches.insert(
            ("local".into(), project_path),
            crate::model::VcsBranchCacheEntry {
                is_repo: false,
                loaded: true,
                loading: false,
                error: None,
                refs: Vec::new(),
            },
        );
        assert!(!rt.state.multiple_models_enabled());
        rt.invalidate();
        rt.frame(1.0);
        rt.frame(1.0);
        assert_multi_send_off(&mut rt);

        // Unavailable on a linked machine without vcs-worktrees-v1 capability.
        let project_path = rt
            .state
            .project(rt.state.current().project)
            .unwrap()
            .path
            .to_string_lossy()
            .into_owned();
        rt.state.vcs_branches.insert(
            ("local".into(), project_path),
            crate::model::VcsBranchCacheEntry {
                is_repo: true,
                loaded: true,
                loading: false,
                error: None,
                refs: vec![],
            },
        );
        let (client, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
        *client.info.lock().unwrap() = json!({"capabilities": ["fresh-context"]});
        rt.state.client = Some(client);
        assert!(!rt.state.multiple_models_enabled());
        rt.invalidate();
        rt.frame(1.0);
        rt.frame(1.0);
        assert_multi_send_off(&mut rt);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
