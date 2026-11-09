use crate::model::{SettingsPicker, State, Status, Thread, Tool, truncate};
use rune::{Color, Cx, El, Icon, ScrollSpec, TextStyle, col, el, memo, row, text, text_input};
use rune_kit::{Markdown, MarkdownStyle};
use serde_json::json;

// Capy's compact neutral hierarchy, redrawn with native Rune elements.
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
            bg: color("#161616"),
            panel: color("#242424"),
            card: color("#2d2d2d"),
            line: Color::WHITE.alpha(0.12),
            ink: rune::color::oklch(0.9404, 0.0013, 286.4),
            muted: color("#a1a1a4"),
            selected: Color::WHITE.alpha(0.08),
            teal: color("#329f98"),
            teal_bg: color("#1d3b39"),
            gold: color("#d3ba6b"),
            danger: color("#f0a39c"),
            on_teal: color("#0d1f1e"),
            badge_ring: color("#161616"),
            teal_text: color("#55bdb4"),
            field_line: color("#56565c"),
        }
    } else {
        Palette {
            bg: color("#ededf0"),
            panel: rune::color::oklch(0.985, 0.0015, 290.0),
            card: Color::WHITE,
            line: Color::BLACK.alpha(0.12),
            ink: color("#202022"),
            muted: color("#6a6a70"),
            selected: Color::BLACK.alpha(0.055),
            teal: color("#11827c"),
            teal_bg: color("#e0efed"),
            gold: color("#8a701f"),
            danger: color("#a33e38"),
            on_teal: Color::WHITE,
            badge_ring: color("#f4f4f6"),
            teal_text: color("#11827c"),
            field_line: color("#c4c4ca"),
        }
    }
}
const BODY: TextStyle = TextStyle::new(13.0, 400.0).lh(19.5).tracking(-0.13);
/// Wide windows (1600 px and up) get a longer measure and larger body text.
const WIDE_WINDOW: f32 = 1600.0;
fn body_style(wide: bool) -> TextStyle {
    if wide {
        TextStyle::new(14.0, 400.0).lh(21.0).tracking(-0.14)
    } else {
        BODY
    }
}
fn measure(wide: bool) -> f32 {
    if wide { 896.0 } else { 768.0 }
}
const LABEL: TextStyle = TextStyle::new(13.0, 420.0).tracking(-0.13);
const SMALL: TextStyle = TextStyle::new(12.0, 420.0);

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
        .label(title)
        .focusable(true)
        .center_y()
        .gap(7.0)
        .px(10.0)
        .h(24.0)
        .radius(5.0)
        .hover(p.selected)
        .on_press(action)
        .child(label(title, TextStyle::new(12.0, 450.0), p.ink))
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
    let glyph = match kind {
        "plus" => &PLUS,
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
        "sun" => &SUN,
        "moon" => &MOON,
        "alert" => &ALERT,
        "pin" => &PIN,
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
/// A compact picker trigger: the text truncates, the chevron always stays visible.
fn dropdown(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    row()
        .key(key)
        .role("button")
        .label(title)
        .focusable(true)
        .center_y()
        .gap(3.0)
        .h(24.0)
        .pl(5.0)
        .pr(3.0)
        .radius(5.0)
        .shrink()
        .min_w(0.0)
        .hover(p.selected)
        .on_press(action)
        .child(
            label(title, TextStyle::new(12.0, 450.0), p.ink)
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(icon("chevron", p.muted))
}
fn thinking_label(level: &str) -> String {
    if level.is_empty() {
        return "Default thinking".into();
    }
    let mut chars = level.chars();
    let first = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    format!("{first}{} thinking", chars.as_str())
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
        .label(title)
        .focusable(true)
        .size(24.0, 24.0)
        .center()
        .radius(5.0)
        .hover(p.selected)
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
        Some(captain) if t.captain_home => s.captain_status(captain),
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
        Status::Settled => icon("check", ink).size(size + 5.0, size + 5.0),
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
    if matches!(status, Status::Ready | Status::Settled) {
        return el();
    }
    row()
        .center_y()
        .gap(6.0)
        .child(status_mark(status, p, 6.0))
        .child(label(status.label(), SMALL, status_ink(status, p)))
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let p = palette(s.dark);
    cx.foreground(p.ink);
    cx.focus_ring(2.0, 2.0, p.teal);
    cx.caption_color(p.muted);
    let narrow = cx
        .layout_with("workspace-root", |r| r[2] < 1050.0)
        .unwrap_or(false);
    // Below this width the sidebar stops docking and opens on demand instead.
    let compact = cx
        .layout_with("workspace-root", |r| r[2] < 760.0)
        .unwrap_or(false);
    let wide = cx
        .layout_with("workspace-root", |r| r[2] >= WIDE_WINDOW)
        .unwrap_or(false);
    let docked = (s.sidebar && !compact) || (compact && s.compact_sidebar);
    let chrome = cx.chrome_insets();
    // Zoom draws the whole UI k times larger. The root is laid out at the window size divided
    // by k and scaled back up, so layout, hit testing and text stay native at every step.
    let zoom = s.zoom.clamp(0.5, 2.0);
    let frame = cx.layout_with("zoom-frame", |r| (r[2], r[3]));
    // One strip at the titlebar's height holds the sidebar header, tabs, actions, and the
    // window's caption buttons, so they share a centre line and never overlap the pane.
    let mut top = row()
        .key("top-strip")
        .h(chrome.height.max(40.0 * zoom) / zoom)
        .window_drag()
        .center_y()
        .pl(8.0)
        .pr((chrome.trailing / zoom).max(8.0));
    if docked {
        top = top.child(el().w(if narrow { 216.0 } else { 232.0 }));
    }
    top = top.child(header(s, p, narrow, compact).grow(1.0).min_w(0.0));
    let mut workspace = row().grow(1.0).min_h(0.0).px(8.0).pb(8.0);
    if docked {
        workspace = workspace.child(sidebar(s, p, narrow));
    }
    let mut pane = col()
        .key("chat-pane")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .radius(12.0)
        .fill(p.panel)
        .border(0.5, p.line)
        .clip_rounded(12.0);
    if s.show_activity && (narrow || s.activity_full) {
        pane = pane.child(activity(s, p, true));
    } else {
        let mut body = row().grow(1.0).min_h(0.0);
        let main = col()
            .grow(1.0)
            .min_w(0.0)
            .min_h(0.0)
            .child(chat_header(s, p, narrow, compact));
        let main = if s.current().messages.is_empty() && s.current().tools.is_empty() {
            main.items_center()
                .child(spacer())
                .child(conversation(s, p, wide))
                .child(composer(s, p, narrow, wide))
                .child(spacer())
        } else {
            main.child(conversation(s, p, wide))
                .child(composer(s, p, narrow, wide))
        };
        body = body.child(main);
        if s.show_activity {
            body = body
                .child(el().w(1.0).h_full().fill(p.line))
                .child(activity(s, p, false));
        }
        pane = pane.child(body);
    }
    let mut root = col()
        .key("workspace-root")
        .fill(p.bg)
        .child(top)
        .child(workspace.child(pane));
    if s.captain_editor.is_some() {
        let height = cx.layout_with("workspace-root", |r| r[3]).unwrap_or(840.0);
        root = root.child(captain_editor(s, p, height));
    } else if s.settings_picker.is_some() {
        root = root.child(settings_picker(s, p));
    } else if s.captain_picker {
        root = root.child(captain_picker(s, p));
    } else if s.projects.clone_open {
        root = root.child(clone_dialog(s, p));
    } else if s.projects.menu.is_some() && docked {
        root = root.child(project_menu(s, p));
    } else if s.thread_menu {
        root = root.child(thread_menu(s, p));
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

fn settings_picker(s: &State, p: Palette) -> El {
    let t = s.current();
    let is_model = s.settings_picker == Some(SettingsPicker::Model);
    let title = if is_model {
        "Choose CPA model"
    } else {
        "Choose reasoning level"
    };
    let mut card = col()
        .key("pi-settings-picker")
        .role("dialog")
        .label(title)
        .w(392.0)
        .max_h(460.0)
        .padding(16.0)
        .gap(10.0)
        .radius(10.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(8.0, 24.0, Color::BLACK.alpha(0.15))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .center_y()
                .child(label(title, LABEL, p.ink))
                .child(spacer())
                .child(icon_button(
                    "close-pi-settings",
                    "Close Pi settings",
                    "close",
                    p,
                    |s| s.settings_picker = None,
                )),
        );
    if is_model {
        card = card.child(
            text_input(&s.model_search, BODY)
                .key("model-search")
                .label("Filter Pi models")
                .placeholder("Filter by model or provider", p.muted)
                .color(p.ink)
                .w_full()
                .on_edit(|s: &mut State| &mut s.model_search),
        );
    }
    let mut choices = col().w_full().max_h(300.0).gap(4.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    let can_choose = !t.status.busy() && !t.configuring;
    if is_model {
        let query = s.model_search.text.to_lowercase();
        let mut count = 0;
        for (index, model) in t.models.iter().enumerate().filter(|(_, m)| {
            format!("{} {} {}", m.provider, m.id, m.name)
                .to_lowercase()
                .contains(&query)
        }) {
            count += 1;
            let provider = model.provider.clone();
            let id = model.id.clone();
            let selected = t.model_provider == model.provider && t.model_id == model.id;
            choices = choices.child(
                button(
                    &format!("choose-model-{index}"),
                    &format!("{} / {}", model.provider, model.name),
                    p,
                    move |s| {
                        s.current_mut().set_model(provider.clone(), id.clone());
                        s.settings_picker = None;
                        s.checkpoint();
                    },
                )
                .h(32.0)
                .w_full()
                .clip_rounded(5.0)
                .fill(if selected { p.teal_bg } else { p.card })
                .disabled(!can_choose),
            );
        }
        if count == 0 {
            choices = choices.child(
                label(
                    if t.models.is_empty() && s.catalog.remaining().is_some() {
                        "Reading the CPA catalog..."
                    } else if t.models.is_empty() {
                        "No CPA models are available."
                    } else {
                        "No matching configured models."
                    },
                    BODY,
                    p.muted,
                )
                .wrap(),
            );
        }
    } else {
        for (index, level) in t.thinking_levels.iter().enumerate() {
            let value = level.clone();
            choices = choices.child(
                button(&format!("choose-thinking-{index}"), level, p, move |s| {
                    s.current_mut().set_thinking(value.clone());
                    s.settings_picker = None;
                    s.checkpoint();
                })
                .fill(if *level == t.thinking {
                    p.teal_bg
                } else {
                    p.card
                })
                .disabled(!can_choose),
            );
        }
        if t.thinking_levels.is_empty() {
            choices = choices.child(label(
                "Reading the model's supported levels...",
                BODY,
                p.muted,
            ));
        }
    }
    card = card.child(choices).child(
        label(
            "Models come from your shared CPA catalog. Credentials stay in Pi.",
            SMALL,
            p.muted,
        )
        .wrap(),
    );
    if let Some(error) = &t.error {
        card = card.child(label(error, SMALL, p.danger).wrap_anywhere());
    }
    if let Some(error) = &s.catalog.error {
        card = card
            .child(label(error, SMALL, p.danger).wrap_anywhere())
            .child(button("retry-catalog", "Retry", p, |s| s.refresh_catalog()));
    }
    el().key("pi-settings-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.28 }))
        .center()
        .modal(false, |s: &mut State| s.settings_picker = None)
        .on_press(|s: &mut State| s.settings_picker = None)
        .child(card)
}

/// Shell-local controls, separate from the draggable window title strip.
fn sidebar_header(s: &State, p: Palette) -> El {
    row()
        .h(36.0)
        .center_y()
        .px(8.0)
        .child(
            el().size(24.0, 24.0)
                .radius(6.0)
                .clip_rounded(6.0)
                .border(0.5, p.line)
                .pixels(
                    if s.dark {
                        &s.artwork.mark_dark
                    } else {
                        &s.artwork.mark
                    }
                    .pixels(),
                ),
        )
        .child(spacer())
        .child(icon_button(
            "search-toggle",
            "Search threads",
            "search",
            p,
            |s| {
                s.search_open = !s.search_open;
                s.search.focused = s.search_open;
            },
        ))
        .child(icon_button(
            "sidebar-toggle",
            "Toggle sidebar",
            "sidebar",
            p,
            |s| {
                if s.compact_sidebar {
                    s.compact_sidebar = false;
                } else {
                    s.sidebar = !s.sidebar;
                }
            },
        ))
}

fn sidebar(s: &State, p: Palette, narrow: bool) -> El {
    let mut side = col()
        .key("sidebar")
        .w(if narrow { 216.0 } else { 232.0 })
        .h_full()
        .min_h(0.0)
        .pr(8.0);
    side = side
        .child(sidebar_header(s, p))
        .child(nav("new-thread", "New thread", "compose", p, false, |s| {
            s.new_thread()
        }))
        .child(nav("threads-nav", "Threads", "list", p, false, |s| {
            s.show_archive = false;
        }));
    side = side.child(nav(
        "show-archive",
        if s.show_archive {
            "Back to threads"
        } else {
            "Archive"
        },
        "archive",
        p,
        s.show_archive,
        |s| s.show_archive = !s.show_archive,
    ));
    if s.search_open {
        side = side.child(
            row()
                .h(30.0)
                .center_y()
                .px(8.0)
                .gap(7.0)
                .radius(6.0)
                .fill(p.panel)
                .child(icon("search", p.muted))
                .child(
                    text_input(&s.search, LABEL)
                        .key("thread-search")
                        .label("Search threads")
                        .placeholder("Search threads", p.muted)
                        .color(p.ink)
                        .single_line()
                        .grow(1.0)
                        .min_w(0.0)
                        .on_edit(|s: &mut State| &mut s.search),
                ),
        );
    }
    let mut content = col().pb(12.0).gap(1.0);
    let search = s.search.text.to_lowercase();
    if !s.show_archive {
        content = content.child(
            section_title("Captains", p)
                .child(icon_button(
                    "browse-captains",
                    "Browse captains",
                    "grid",
                    p,
                    |s| s.captain_picker = true,
                ))
                .child(icon_button("new-captain", "New Captain", "plus", p, |s| {
                    s.begin_new_captain()
                })),
        );
        for captain in 0..s.captains.len() {
            let state = s.captain_status(captain);
            let selected = s.current().captain == Some(captain);
            let unread = s
                .threads
                .iter()
                .filter(|t| t.captain == Some(captain) && t.unread && !t.archived)
                .count();
            let mut captain_row = row()
                .key(format!("captain-{captain}"))
                .role("button")
                .label(format!(
                    "Open Captain {}, {}",
                    s.captains[captain].name,
                    state.label()
                ))
                .focusable(true)
                .h(32.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .hover(p.selected)
                .on_press(move |s: &mut State| s.open_captain(captain))
                .child(avatar(s, p, captain, 24.0, state))
                .child(label(
                    &s.captains[captain].name,
                    TextStyle::new(
                        13.0,
                        if selected && s.current().captain_home {
                            560.0
                        } else {
                            450.0
                        },
                    ),
                    p.ink,
                ))
                .child(spacer());
            if unread > 0 {
                captain_row = captain_row.child(counter(unread, p));
            }
            if selected && s.current().captain_home {
                captain_row = captain_row.fill(p.selected);
            }
            if selected {
                captain_row = captain_row.child(icon_button(
                    &format!("rename-captain-{captain}"),
                    &format!("Rename or edit {}", s.captains[captain].name),
                    "edit",
                    p,
                    move |s| s.edit_captain(captain),
                ));
            }
            if state == Status::Waiting {
                captain_row = captain_row.child(label("Needs you", SMALL, status_ink(state, p)));
            }
            content = content.child(captain_row);
            if selected {
                let mut nested = col()
                    .ml(19.0)
                    .pl(13.0)
                    .gap(1.0)
                    .border_sides([0.0, 0.0, 0.0, 1.0], p.line);
                let mut any = false;
                for (index, _) in s.threads.iter().enumerate().filter(|(_, t)| {
                    (t.captain == Some(captain)
                        || sidebar_parent(s, t).is_some_and(|parent| {
                            s.threads[parent].captain_home
                                && s.threads[parent].captain == Some(captain)
                        }))
                        && !t.captain_home
                        && (!t.pinned || sidebar_parent(s, t).is_some())
                        && !t.archived
                        && thread_matches(s, t, &search)
                        && sidebar_parent(s, t).is_none_or(|parent| s.threads[parent].captain_home)
                }) {
                    nested = nested.child(thread_group(s, index, p, true, &search));
                    any = true;
                }
                if any {
                    content = content.child(nested);
                }
            }
        }
    }
    content = content.child(section_title(
        if s.show_archive {
            "Archived threads"
        } else {
            "Threads"
        },
        p,
    ));
    let pinned: Vec<_> = s
        .threads
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            t.pinned
                && t.archived == s.show_archive
                && !t.captain_home
                && sidebar_parent(s, t).is_none()
                && thread_matches(s, t, &search)
        })
        .map(|(i, _)| i)
        .collect();
    if !pinned.is_empty() {
        content = content.child(
            row()
                .h(24.0)
                .center_y()
                .gap(6.0)
                .px(8.0)
                .child(label("Pinned", TextStyle::new(11.5, 520.0), p.muted))
                .child(spacer())
                .child(el().w(24.0).center().child(label(
                    pinned.len().to_string(),
                    SMALL,
                    p.muted.alpha(0.7),
                ))),
        );
        for index in pinned {
            content = content.child(thread_group(s, index, p, false, &search));
        }
    }
    content = content.child(project_switcher(s, p));
    let mut count = 0;
    for (index, thread) in s.threads.iter().enumerate().filter(|(_, t)| {
        t.archived == s.show_archive
            && !t.captain_home
            && !t.pinned
            && sidebar_parent(s, t).is_none()
            && thread_matches(s, t, &search)
    }) {
        if thread.captain == s.current().captain && thread.captain.is_some() && !s.show_archive {
            continue;
        }
        count += 1;
        content = content.child(thread_group(s, index, p, false, &search));
    }
    if count == 0 && !search.is_empty() {
        content = content.child(label("No matching threads", SMALL, p.muted).padding(8.0));
    }
    side = side.child(
        col()
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
    side = side.child(row().px(8.0).w_full().child(rule(p))).child(
        row()
            .h(48.0)
            .px(8.0)
            .gap(8.0)
            .center_y()
            .child(
                col()
                    .gap(1.0)
                    .grow(1.0)
                    .min_w(0.0)
                    .child(label("Shina", TextStyle::new(12.0, 560.0), p.ink))
                    .child(label(
                        if s.demo {
                            "Demo workspace"
                        } else {
                            "Local workspace"
                        },
                        SMALL,
                        p.muted,
                    )),
            )
            .child(icon_button(
                "theme-toggle",
                if s.dark {
                    "Switch to light theme"
                } else {
                    "Switch to dark theme"
                },
                if s.dark { "sun" } else { "moon" },
                p,
                |s| {
                    s.dark = !s.dark;
                    s.checkpoint();
                },
            )),
    );
    side
}

/// The sidebar's one entry point for changing project: shows the open folder, opens the menu.
fn project_switcher(s: &State, p: Palette) -> El {
    let name = s.project_name();
    let open = s.projects.menu.is_some();
    row()
        .key("project-switcher")
        .role("button")
        .label(format!("Switch project, current project {name}"))
        .focusable(true)
        .h(31.0)
        .center_y()
        .gap(8.0)
        .px(8.0)
        .radius(6.0)
        .fill(if open { p.selected } else { Color::TRANSPARENT })
        .hover(p.selected)
        .on_press(|s: &mut State| {
            if s.projects.menu.is_some() {
                s.close_project_menu();
            } else {
                s.open_project_menu();
            }
        })
        .child(icon("folder", p.muted))
        .child(
            label(name, TextStyle::new(12.0, 520.0), p.ink)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(icon("chevron", p.muted))
}

fn menu_item(key: &str, title: &str, glyph: &str, p: Palette) -> El {
    row()
        .key(key)
        .role("menuitem")
        .label(title)
        .focusable(true)
        .h(30.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(5.0)
        .hover(p.selected)
        .child(icon(glyph, p.muted))
        .child(label(title, TextStyle::new(12.5, 450.0), p.ink).grow(1.0))
}

/// Recent projects plus the two ways to add one, under the switcher.
fn project_menu(s: &State, p: Palette) -> El {
    let projects = s.projects.menu.as_deref().unwrap_or_default();
    let mut list = col().gap(1.0).shrink().min_h(0.0).scroll(ScrollSpec {
        stick_bottom: false,
    });
    for (index, project) in projects.iter().enumerate() {
        let current = if s.demo {
            index == 0
        } else {
            project.path == s.workspace
        };
        let path = project.path.clone();
        let mut item = row()
            .key(format!("project-{index}"))
            .role("menuitem")
            .label(if current {
                format!("{}, current project", project.name)
            } else {
                format!("Open {}", project.name)
            })
            .focusable(true)
            .px(8.0)
            .py(5.0)
            .gap(8.0)
            .center_y()
            .radius(5.0)
            .hover(p.selected)
            .on_press(move |s: &mut State| s.open_project(path.clone()))
            .child(
                col()
                    .gap(1.0)
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        label(&project.name, TextStyle::new(12.5, 520.0), p.ink)
                            .truncate()
                            .min_w(0.0),
                    )
                    .child(
                        label(project.path.display().to_string(), SMALL, p.muted)
                            .truncate()
                            .min_w(0.0),
                    ),
            );
        if current {
            item = item.fill(p.selected).child(icon("check", p.teal));
        }
        list = list.child(item);
    }
    if projects.is_empty() {
        list = list.child(
            label("No other recent projects", SMALL, p.muted)
                .px(8.0)
                .py(6.0),
        );
    }
    let mut menu = col()
        .key("project-menu")
        .role("menu")
        .label("Projects")
        .w(280.0)
        .max_h(440.0)
        .padding(5.0)
        .gap(1.0)
        .radius(8.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(6.0, 18.0, Color::BLACK.alpha(0.14))
        .anchored_fit(
            "project-switcher",
            rune::Side::Bottom,
            rune::Align::Start,
            4.0,
        )
        .modal(true, |s: &mut State| s.close_project_menu())
        .child(row().h(24.0).px(8.0).center_y().child(label(
            "Recent projects",
            TextStyle::new(11.5, 520.0),
            p.muted,
        )))
        .child(list)
        .child(el().h(1.0).w_full().my(4.0).fill(p.line))
        .child(
            menu_item("project-open-folder", "Open folder...", "folder", p)
                .on_press_tx(crate::projects::pick_folder)
                .child(label("Ctrl+O", SMALL, p.muted)),
        )
        .child(
            menu_item("project-clone", "Clone repository...", "branch", p)
                .on_press(|s: &mut State| s.open_clone_dialog()),
        );
    if let Some(error) = &s.projects.menu_error {
        menu = menu.child(label(error, SMALL, p.danger).wrap().px(8.0).py(6.0));
    }
    menu
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

/// Shown only when there is something to do: an update to fetch, one downloading, or one
/// ready to install with a restart.
fn update_row(s: &State, p: Palette) -> Option<El> {
    use crate::update::Status;
    type Action = Option<fn(&mut State)>;
    let (key, title, action): (&str, String, Action) = match &s.updater.status {
        Status::Available { tag } => (
            "update-download",
            format!("Update to {tag}"),
            Some(|s: &mut State| s.updater.download(s.wake.clone())),
        ),
        Status::Downloading { tag } => ("update-progress", format!("Downloading {tag}"), None),
        Status::Ready { tag, .. } => (
            "update-restart",
            format!("Restart to install {tag}"),
            Some(|s: &mut State| s.restart_for_update = true),
        ),
        _ => return None,
    };
    let mut button = row()
        .key(key)
        .role("button")
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
        .child(label(title, TextStyle::new(12.5, 520.0), p.teal_text));
    if let Some(action) = action {
        button = button.focusable(true).hover(p.selected).on_press(action);
    }
    Some(button)
}
fn nav(
    key: &str,
    title: &str,
    glyph: &str,
    p: Palette,
    active: bool,
    action: impl Fn(&mut State) + 'static,
) -> El {
    row()
        .key(key)
        .role("button")
        .label(title)
        .focusable(true)
        .h(28.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .fill(if active {
            p.selected
        } else {
            Color::TRANSPARENT
        })
        .hover(p.selected)
        .on_press(action)
        .child(icon(glyph, if active { p.ink } else { p.muted }))
        .child(label(
            title,
            if active {
                TextStyle::new(13.0, 520.0).tracking(-0.13)
            } else {
                LABEL
            },
            if active { p.ink } else { p.muted },
        ))
}
fn section_title(title: &str, p: Palette) -> El {
    row()
        .h(36.0)
        .pt(8.0)
        .px(8.0)
        .center_y()
        .child(label(title, SMALL, p.muted))
        .child(spacer())
}
fn face_for(status: Status) -> crate::visuals::Face {
    use crate::visuals::Face;
    match status {
        Status::Ready => Face::Normal,
        Status::Running => Face::Determined,
        Status::Waiting => Face::Surprised,
        Status::Settled => Face::Happy,
        Status::Failed => Face::Dizzy,
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
fn counter(count: usize, p: Palette) -> El {
    el().min_w(16.0)
        .h(15.0)
        .px(4.0)
        .radius(8.0)
        .fill(p.teal)
        .center()
        .child(label(
            count.to_string(),
            TextStyle::new(10.0, 560.0),
            Color::WHITE,
        ))
}
fn sidebar_parent(s: &State, thread: &crate::model::Thread) -> Option<usize> {
    s.threads.iter().position(|parent| {
        Some(parent.id) == thread.parent
            && parent.archived == s.show_archive
            && (!parent.captain_home || (!s.show_archive && parent.captain == s.current().captain))
    })
}

fn thread_matches(s: &State, thread: &crate::model::Thread, search: &str) -> bool {
    thread.title.to_lowercase().contains(search)
        || s.threads.iter().any(|child| {
            child.parent == Some(thread.id)
                && child.archived == s.show_archive
                && child.title.to_lowercase().contains(search)
        })
}

fn thread_group(s: &State, index: usize, p: Palette, nested: bool, search: &str) -> El {
    let mut group = col().child(thread_row(s, index, p, nested));
    let mut children = col()
        .ml(19.0)
        .pl(13.0)
        .gap(1.0)
        .border_sides([0.0, 0.0, 0.0, 1.0], p.line);
    let mut any = false;
    for (child, thread) in s.threads.iter().enumerate() {
        if sidebar_parent(s, thread) == Some(index)
            && thread.archived == s.show_archive
            && thread.title.to_lowercase().contains(search)
        {
            children = children.child(thread_row(s, child, p, true));
            any = true;
        }
    }
    if any {
        group = group.child(children);
    }
    group
}

fn thread_row(s: &State, index: usize, p: Palette, nested: bool) -> El {
    let thread = &s.threads[index];
    let selected = index == s.active;
    let item = col()
        .key(format!("thread-{}", thread.id))
        .role("button")
        .label(format!("{}, {}", thread.title, thread.status.label()))
        .focusable(true)
        .px(8.0)
        .py(if nested { 5.0 } else { 6.0 })
        .gap(2.0)
        .radius(6.0)
        .min_h(30.0)
        .justify_center()
        .fill(if selected {
            p.selected
        } else {
            Color::TRANSPARENT
        })
        .hover(p.selected)
        .on_press(move |s: &mut State| {
            s.active = index;
            s.current_mut().unread = false;
        });

    let mut title = row().gap(8.0).center_y();
    if !nested {
        title = title.child(icon(
            if thread.pinned { "pin" } else { "thread" },
            if selected { p.ink } else { p.muted },
        ));
    }
    title = title.child(
        label(&thread.title, LABEL, if selected { p.ink } else { p.muted })
            .truncate()
            .grow(1.0),
    );
    if thread.unread {
        title = title.child(counter(1, p));
    }
    let detailed = matches!(
        thread.status,
        Status::Running | Status::Waiting | Status::Failed
    );
    if !detailed {
        return item.child(title);
    }
    let mut meta = row()
        .gap(5.0)
        .center_y()
        .pl(if nested { 0.0 } else { 24.0 });
    meta = meta.child(status_mark(thread.status, p, 6.0)).child(label(
        thread.status.label(),
        SMALL,
        status_ink(thread.status, p),
    ));
    item.child(title).child(meta)
}

fn captain_picker(s: &State, p: Palette) -> El {
    let mut card = col()
        .key("captain-picker")
        .role("dialog")
        .label("Open Captain")
        .w(392.0)
        .padding(16.0)
        .gap(10.0)
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
                        .child(label("Open Captain", TextStyle::new(14.0, 560.0), p.ink))
                        .child(label("Open a captain's conversation.", SMALL, p.muted)),
                )
                .child(spacer())
                .child(icon_button(
                    "close-captain-picker",
                    "Close Captain picker",
                    "close",
                    p,
                    |s| s.captain_picker = false,
                )),
        );
    for captain in 0..s.captains.len() {
        let state = s.captain_status(captain);
        let open = s.current().captain == Some(captain) && s.current().captain_home;
        card = card.child(
            row()
                .key(format!("choose-captain-{captain}"))
                .role("button")
                .label(format!(
                    "Open Captain {}, {}",
                    s.captains[captain].name,
                    state.label()
                ))
                .focusable(true)
                .gap(11.0)
                .padding(9.0)
                .radius(6.0)
                .fill(if open { p.selected } else { Color::TRANSPARENT })
                .hover(p.selected)
                .center_y()
                .on_press(move |s: &mut State| s.open_captain(captain))
                .child(avatar(s, p, captain, 40.0, state))
                .child(
                    col()
                        .gap(3.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .child(label(
                            &s.captains[captain].name,
                            TextStyle::new(13.0, 560.0),
                            p.ink,
                        ))
                        .child(label(&s.captains[captain].role, SMALL, p.muted)),
                )
                .child(captain_status_chip(state, p))
                .child(icon_button(
                    &format!("edit-captain-{captain}"),
                    &format!("Rename or edit {}", s.captains[captain].name),
                    "edit",
                    p,
                    move |s| {
                        s.captain_picker = false;
                        s.edit_captain(captain);
                    },
                )),
        );
    }
    card = card.child(
        row()
            .key("picker-new-captain")
            .role("button")
            .label("New Captain")
            .focusable(true)
            .h(36.0)
            .px(10.0)
            .gap(8.0)
            .center_y()
            .radius(6.0)
            .border(0.5, p.line)
            .hover(p.selected)
            .on_press(|s: &mut State| s.begin_new_captain())
            .child(icon("plus", p.muted))
            .child(label("New Captain", LABEL, p.ink)),
    ).child(rule(p)).child(
        label(
            "Each Captain is a separate Pi conversation. Automatic crew delegation is not connected.",
            SMALL,
            p.muted,
        )
        .wrap(),
    );
    el().key("captain-picker-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.28 }))
        .center()
        .modal(false, |s: &mut State| s.captain_picker = false)
        .on_press(|s: &mut State| s.captain_picker = false)
        .child(card)
}

fn captain_editor(s: &State, p: Palette, height: f32) -> El {
    let editor = s.captain_editor.as_ref().unwrap();
    let editing = editor.editing.is_some();
    let profile = editor.profile();
    let field = if s.dark { p.bg } else { p.panel };
    let mut portraits = row().gap(8.0).w_full();
    for portrait in 0..crate::visuals::CAPTAIN_NAMES.len() {
        portraits = portraits.child(
            col()
                .key(format!("captain-portrait-{portrait}"))
                .role("button")
                .label(format!(
                    "Use {} portrait",
                    crate::visuals::PORTRAIT_NAMES[portrait]
                ))
                .focusable(true)
                .grow(1.0)
                .padding(7.0)
                .gap(5.0)
                .items_center()
                .radius(7.0)
                .border(
                    if editor.portrait == portrait {
                        2.0
                    } else {
                        0.5
                    },
                    if editor.portrait == portrait {
                        p.teal
                    } else {
                        p.line
                    },
                )
                .fill(if editor.portrait == portrait {
                    p.teal_bg
                } else {
                    Color::TRANSPARENT
                })
                .hover(p.selected)
                .on_press(move |s: &mut State| {
                    s.captain_editor.as_mut().unwrap().portrait = portrait;
                })
                .child(portrait_avatar(s, p, portrait, 48.0, Status::Ready))
                .child(label(
                    crate::visuals::PORTRAIT_NAMES[portrait],
                    SMALL,
                    p.ink,
                )),
        );
    }
    let mut body = col().key("captain-form-body").gap(12.0).min_h(0.0)
        .max_h((height - 180.0).max(180.0))
        .scroll(ScrollSpec { stick_bottom: false })
        .child(col().gap(6.0).child(label("Portrait", SMALL, p.muted)).child(portraits))
        .child(col().gap(5.0).child(label("Name", SMALL, p.muted))
            .child(text_input(&editor.name, BODY).key("captain-name").label("Captain name")
                .placeholder("e.g. Atlas", p.muted).single_line().color(p.ink)
                .caret(p.teal, p.teal_bg).w_full().padding(8.0).radius(6.0).fill(field).border(1.0, p.field_line)
                .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().name)))
        .child(col().gap(5.0).child(label("Role", SMALL, p.muted))
            .child(text_input(&editor.role, BODY).key("captain-role").label("Captain role")
                .placeholder("e.g. Code review", p.muted).single_line().color(p.ink)
                .caret(p.teal, p.teal_bg).w_full().padding(8.0).radius(6.0).fill(field).border(1.0, p.field_line)
                .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().role)))
        .child(col().gap(5.0).child(label("Instructions for Pi", SMALL, p.muted))
            .child(text_input(&editor.instructions, BODY).key("captain-instructions").label("Captain instructions")
                .placeholder("What should this captain focus on? Optional.", p.muted).color(p.ink)
                .caret(p.teal, p.teal_bg).w_full().min_h(76.0).max_h(140.0).padding(8.0)
                .radius(6.0).fill(field).border(1.0, p.field_line)
                .on_edit(|s: &mut State| &mut s.captain_editor.as_mut().unwrap().instructions)))
        .child(label(
            "Saved to this workspace. Each conversation uses these instructions and Pi's configured CPA model. Choose a different model in the composer.",
            SMALL, p.muted).wrap());
    if let Some(error) = &editor.error {
        body = body.child(label(error, SMALL, p.danger).wrap());
    }
    let title = if editing {
        "Edit Captain"
    } else {
        "New Captain"
    };
    let card = col()
        .key("captain-editor")
        .role("dialog")
        .label(title)
        .w(432.0)
        .max_h((height - 32.0).max(240.0))
        .padding(16.0)
        .gap(14.0)
        .radius(10.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(8.0, 24.0, Color::BLACK.alpha(0.15))
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
                        .child(label("Give a captain a name and a job.", SMALL, p.muted)),
                )
                .child(spacer())
                .child(icon_button(
                    "close-captain-editor",
                    "Close",
                    "close",
                    p,
                    |s| s.captain_editor = None,
                )),
        )
        .child(body)
        .child(rule(p))
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(button("cancel-captain", "Cancel", p, |s| {
                    s.captain_editor = None
                }))
                .child(spacer())
                .child(if profile.validate().is_ok() {
                    primary_button(
                        "save-captain",
                        if editing {
                            "Save changes"
                        } else {
                            "Create Captain"
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
                            "Create Captain"
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
        .modal(false, |s: &mut State| s.captain_editor = None)
        .on_press(|s: &mut State| s.captain_editor = None)
        .child(card)
}

fn header(s: &State, p: Palette, narrow: bool, _: bool) -> El {
    // Only tabs live in the title strip. Native caption buttons occupy its right edge.
    row()
        .h(36.0)
        .window_drag()
        .center_y()
        .gap(6.0)
        .child(
            row()
                .key("active-tab")
                .center_y()
                .gap(8.0)
                .px(10.0)
                .h(26.0)
                .radius(6.0)
                .shrink()
                .max_w(if narrow { 220.0 } else { 320.0 })
                .fill(p.card)
                .border(0.5, p.line)
                .child(if let Some(captain) = s.current().captain {
                    avatar(s, p, captain, 20.0, shown_status(s))
                } else {
                    icon("thread", p.muted)
                })
                .child(label(&s.current().title, TextStyle::new(12.0, 520.0), p.ink).truncate()),
        )
        .child(icon_button("new-tab", "New thread", "plus", p, |s| {
            s.new_thread()
        }))
        .child(spacer())
}

fn chat_header(s: &State, p: Palette, narrow: bool, compact: bool) -> El {
    let docked = if compact {
        s.compact_sidebar
    } else {
        s.sidebar
    };
    // The tab strip already names the thread, so the header only holds thread actions:
    // status, a labeled "More actions" menu and the session panel toggle.
    let mut header = row()
        .key("chat-header")
        .w_full()
        .h(36.0)
        .px(10.0)
        .center_y()
        .gap(4.0);
    if !docked {
        header = header.child(icon_button(
            "toggle-sidebar",
            "Show sidebar",
            "sidebar",
            p,
            move |s| {
                if compact {
                    s.compact_sidebar = true;
                } else {
                    s.sidebar = true;
                }
            },
        ));
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
        status(s.current().status, p)
    });
    if !(narrow && s.current().captain_home) {
        header = header.child(toggle_button(
            "thread-actions",
            "More actions",
            "more",
            s.thread_menu,
            p,
            |s| s.thread_menu = !s.thread_menu,
        ));
    }
    if !s.show_activity {
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
    let thread = s.current();
    let mut menu = col()
        .key("thread-menu")
        .role("menu")
        .label("Thread actions")
        .w(200.0)
        .padding(5.0)
        .gap(1.0)
        .radius(8.0)
        .fill(p.card)
        .border(0.5, p.line)
        .shadow(6.0, 18.0, Color::BLACK.alpha(0.14))
        .anchored_fit("thread-actions", rune::Side::Bottom, rune::Align::End, 4.0)
        .modal(true, |s: &mut State| s.thread_menu = false);
    if !thread.captain_home {
        menu = menu.child(
            menu_item(
                "menu-pin-thread",
                if thread.pinned {
                    "Unpin thread"
                } else {
                    "Pin thread"
                },
                "pin",
                p,
            )
            .on_press(|s: &mut State| {
                s.thread_menu = false;
                s.toggle_pin();
            }),
        );
    }
    let busy = thread.status.busy();
    let mut archive = menu_item(
        "menu-archive-thread",
        if thread.archived {
            "Restore thread"
        } else {
            "Archive thread"
        },
        "archive",
        p,
    );
    archive = if busy {
        archive.child(label("Working", SMALL, p.muted))
    } else {
        archive.on_press(|s: &mut State| {
            s.thread_menu = false;
            s.archive();
        })
    };
    menu.child(archive)
}

fn tool_kind(name: &str) -> (&'static str, &'static str, &'static str) {
    match name.to_ascii_lowercase().as_str() {
        "read" | "read_file" => ("file", "Read", "file"),
        "edit" | "write" | "edit_file" | "write_file" | "apply_patch" => ("edit", "Edited", "file"),
        "bash" | "powershell" | "shell" | "exec" | "exec_command" => ("terminal", "Ran", "command"),
        "grep" | "rg" | "search" | "find" => ("search", "Searched", "query"),
        "ls" | "list" | "list_directory" => ("folder", "Listed", "directory"),
        _ => ("bolt", "Called", "tool"),
    }
}

fn tool_title(tool: &Tool) -> String {
    let (_, verb, noun) = tool_kind(&tool.name);
    let name = if noun == "tool" {
        format!("{verb} {}", tool.name)
    } else {
        verb.into()
    };
    if tool.detail.is_empty() {
        name
    } else {
        format!("{name} {}", tool.detail)
    }
}

fn tool_summary(tools: &[&Tool]) -> String {
    let mut counts: Vec<(&str, &str, usize)> = vec![];
    for tool in tools {
        let (_, verb, noun) = tool_kind(&tool.name);
        if let Some(entry) = counts.iter_mut().find(|entry| entry.0 == verb) {
            entry.2 += 1;
        } else {
            counts.push((verb, noun, 1));
        }
    }
    counts
        .iter()
        .enumerate()
        .map(|(index, (verb, noun, count))| {
            let verb = if index == 0 {
                verb.to_string()
            } else {
                verb.to_lowercase()
            };
            let noun = if *count == 1 {
                noun.to_string()
            } else if *noun == "directory" {
                "directories".into()
            } else {
                format!("{noun}s")
            };
            format!("{verb} {count} {noun}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
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

fn inline_tool(s: &State, p: Palette, index: usize) -> El {
    let tool = &s.current().tools[index];
    let title = tool_title(tool);
    let ink = if tool.failed { p.danger } else { p.muted };
    let mut content = col().gap(5.0).child(
        row()
            .key(format!("inline-tool-{}-{index}", s.current().id))
            .role("button")
            .label(format!(
                "{} output for {title}",
                if tool.expanded { "Hide" } else { "Show" }
            ))
            .focusable(true)
            .center_y()
            .gap(7.0)
            .min_h(28.0)
            .px(6.0)
            .radius(5.0)
            .hover(p.selected)
            .on_press(move |s: &mut State| {
                let tool = &mut s.current_mut().tools[index];
                tool.expanded = !tool.expanded;
            })
            .child(tool_icon(tool_kind(&tool.name).0, ink))
            .child(label(&title, SMALL, ink).grow(1.0).min_w(0.0).truncate())
            .child(label(
                if tool.failed {
                    "Failed"
                } else if tool.done {
                    "Done"
                } else {
                    "Running"
                },
                TextStyle::new(11.0, 420.0),
                ink,
            ))
            .child(if let Some(ms) = tool.duration_ms {
                label(duration(ms), TextStyle::new(11.0, 420.0), p.muted)
            } else {
                el()
            })
            .child(icon(
                if tool.expanded { "chevron" } else { "open" },
                p.muted,
            )),
    );
    if tool.expanded {
        content = content.child(
            col()
                .key(format!("tool-output-{}-{index}", s.current().id))
                .role("group")
                .label(format!("Output for {title}"))
                .ml(28.0)
                .padding(10.0)
                .gap(8.0)
                .radius(6.0)
                .fill(p.card)
                .border(0.5, p.line)
                .child(
                    label(&tool.detail, SMALL, p.muted)
                        .wrap_anywhere()
                        .selectable(true),
                )
                .child(
                    col()
                        .max_h(220.0)
                        .scroll(ScrollSpec {
                            stick_bottom: !tool.done,
                        })
                        .child(
                            label(
                                if tool.output.is_empty() {
                                    if tool.done {
                                        "No text output."
                                    } else {
                                        "Waiting for output..."
                                    }
                                } else {
                                    &tool.output
                                },
                                TextStyle::new(11.5, 400.0)
                                    .family(rune::Family::SystemMono)
                                    .lh(17.0),
                                p.ink,
                            )
                            .wrap_anywhere()
                            .selectable(true),
                        ),
                ),
        );
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
    let running = tools.iter().any(|tool| !tool.done);
    let known_duration: Option<u64> = tools.iter().map(|tool| tool.duration_ms).sum();
    let ink = if failed > 0 { p.danger } else { p.muted };
    let summary = tool_summary(&tools);
    let mut group = col()
        .key(format!("tool-group-{}-{first}", t.id))
        .gap(3.0)
        .child(
            row()
                .key(format!("toggle-tool-group-{}-{first}", t.id))
                .role("button")
                .label(format!(
                    "{} {summary}",
                    if expanded { "Collapse" } else { "Expand" }
                ))
                .focusable(true)
                .center_y()
                .gap(7.0)
                .min_h(28.0)
                .px(6.0)
                .radius(5.0)
                .hover(p.selected)
                .on_press(move |s: &mut State| {
                    let tool = &mut s.current_mut().tools[first];
                    tool.group_expanded = !tool.group_expanded;
                })
                .child(icon(
                    if failed > 0 {
                        "alert"
                    } else if running {
                        "dot"
                    } else {
                        "check"
                    },
                    ink,
                ))
                .child(label(summary, SMALL, ink).grow(1.0).min_w(0.0).wrap())
                .child(label(
                    if failed > 0 {
                        format!("{failed} failed")
                    } else if running {
                        "Running".into()
                    } else {
                        "Done".into()
                    },
                    TextStyle::new(11.0, 420.0),
                    ink,
                ))
                .child(if let Some(ms) = known_duration {
                    label(duration(ms), TextStyle::new(11.0, 420.0), p.muted)
                } else {
                    el()
                })
                .child(icon(if expanded { "chevron" } else { "open" }, p.muted)),
        );
    if expanded {
        let mut rows = col().grow(1.0).min_w(0.0).gap(2.0);
        for index in indices {
            rows = rows.child(inline_tool(s, p, *index));
        }
        group = group.child(
            row()
                .gap(10.0)
                .pl(12.0)
                .child(el().w(1.0).h_full().fill(p.line))
                .child(rows),
        );
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
    let title = match (thought.done, timing) {
        (true, Some(ms)) => format!("Thought for {}s", ms / 1000),
        (false, Some(ms)) => format!("Thinking for {}s", ms / 1000),
        (true, None) => "Thought".into(),
        (false, None) => "Thinking".into(),
    };
    let mut content = col().gap(5.0).child(
        row()
            .key(format!("inline-thought-{}-{index}", t.id))
            .role("button")
            .label(format!(
                "{} thinking text",
                if thought.expanded { "Hide" } else { "Show" }
            ))
            .focusable(true)
            .center_y()
            .gap(7.0)
            .min_h(28.0)
            .px(6.0)
            .radius(5.0)
            .hover(p.selected)
            .on_press(move |s: &mut State| {
                let thought = &mut s.current_mut().thoughts[index];
                thought.expanded = !thought.expanded;
            })
            .child(icon("dot", p.muted))
            .child(label(title, SMALL, p.muted).grow(1.0))
            .child(icon(
                if thought.expanded { "chevron" } else { "open" },
                p.muted,
            )),
    );
    if thought.expanded {
        content = content.child(
            col()
                .key(format!("thought-output-{}-{index}", t.id))
                .role("group")
                .label("Thinking text")
                .ml(28.0)
                .padding(10.0)
                .radius(6.0)
                .fill(p.card)
                .border(0.5, p.line)
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
                        SMALL.lh(18.0),
                        p.muted,
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
        TranscriptPart::Tools(indices) => inline_tools(s, p, indices),
        TranscriptPart::Thought(index) => inline_thought(s, p, *index),
        TranscriptPart::Text { message, from, to } => {
            let chunk = &t.messages[*message].text[*from..*to];
            let streaming = t.status == Status::Running && message + 1 == t.messages.len();
            let style = MarkdownStyle::neutral(s.dark)
                .text(p.ink)
                .muted(p.muted)
                .body_font(body_style(wide))
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
                (chunk, s.dark, streaming),
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

fn assistant_turn(s: &State, p: Palette, wide: bool, start: usize, end: usize) -> El {
    let t = s.current();
    let parts = turn_parts(t, start, end);
    let last_work = parts
        .iter()
        .rposition(|part| !matches!(part, TranscriptPart::Text { .. }));
    // Initial thinking may precede the preamble on the wire. Keep that thinking in
    // the work section, and the first text before subsequent work above its heading.
    let preamble = last_work.and_then(|last| {
        parts[..last]
            .iter()
            .position(|part| matches!(part, TranscriptPart::Text { .. }))
    });
    let live = end == t.messages.len() && t.status.busy();
    let work = t.work.iter().find(|work| work.start == start);
    let expanded = work.is_none_or(|work| work.expanded);
    let mut response = col()
        .key(format!("assistant-turn-{}-{start}", t.id))
        .gap(16.0);
    let mut heading = row().center_y().gap(7.0);
    if let Some(captain) = t.captain {
        heading = heading.child(avatar(s, p, captain, 20.0, t.status));
    }
    response = response.child(
        heading.child(label(
            t.captain
                .map(|id| format!("Captain {}", s.captains[id].name))
                .unwrap_or("Pi".into()),
            TextStyle::new(13.0, 600.0),
            p.ink,
        )),
    );
    if let Some(index) = preamble {
        response = response.child(transcript_part(s, p, wide, &parts[index]));
    } else if live && last_work.is_none() {
        // Until Pi emits work, streamed text is the preamble. Keep the live
        // heading after it so the first tool does not move that text.
        for part in &parts {
            response = response.child(transcript_part(s, p, wide, part));
        }
    }
    if last_work.is_some() || live {
        let seconds = if live {
            t.started.map(|start| start.elapsed().as_secs())
        } else {
            work.and_then(|work| work.elapsed)
        };
        let title = match (live, seconds) {
            (true, Some(seconds)) => format!("Working for {seconds}s"),
            (true, None) => "Working".into(),
            (false, Some(seconds)) => format!("Worked for {seconds}s"),
            (false, None) => "Worked".into(),
        };
        let mut section = col()
            .key(format!("work-section-{}-{start}", t.id))
            .gap(8.0)
            .child(
                row()
                    .key(format!("toggle-work-{}-{start}", t.id))
                    .role("button")
                    .label(format!(
                        "{} work section. {title}",
                        if expanded { "Collapse" } else { "Expand" }
                    ))
                    .focusable(true)
                    .center_y()
                    .gap(10.0)
                    .min_h(28.0)
                    .radius(5.0)
                    .hover(p.selected)
                    .on_press(move |s: &mut State| {
                        if let Some(work) = s
                            .current_mut()
                            .work
                            .iter_mut()
                            .find(|work| work.start == start)
                        {
                            work.expanded = !work.expanded;
                        }
                    })
                    .child(el().h(0.5).grow(1.0).fill(p.line))
                    .child(label(title, TextStyle::new(11.0, 420.0), p.muted))
                    .child(icon(if expanded { "chevron" } else { "open" }, p.muted))
                    .child(el().h(0.5).grow(1.0).fill(p.line)),
            );
        if expanded {
            let mut rows = col().key(format!("work-rows-{}-{start}", t.id)).gap(4.0);
            if let Some(last) = last_work {
                for (index, part) in parts[..=last].iter().enumerate() {
                    if Some(index) != preamble {
                        let row = transcript_part(s, p, wide, part);
                        // Prose between tool rows needs the same breathing room as paragraphs.
                        rows = rows.child(if matches!(part, TranscriptPart::Text { .. }) {
                            row.py(8.0)
                        } else {
                            row
                        });
                    }
                }
            }
            if live {
                rows = rows.child(
                    label(
                        if t.phase.is_empty() {
                            "Waiting for the model"
                        } else {
                            &t.phase
                        },
                        SMALL,
                        p.muted,
                    )
                    .px(6.0),
                );
            }
            section = section.child(rows);
        }
        response = response.child(section);
    }
    if !live || last_work.is_some() {
        for part in &parts[last_work.map_or(0, |index| index + 1)..] {
            response = response.child(transcript_part(s, p, wide, part));
        }
    }
    response
}

fn conversation(s: &State, p: Palette, wide: bool) -> El {
    let t = s.current();
    let mut messages = col()
        .w_full()
        .max_w(measure(wide))
        .px(16.0)
        .pt(20.0)
        .pb(24.0)
        .gap(18.0);
    if t.messages.is_empty() && t.tools.is_empty() && !t.status.busy() {
        let mut introduction = col()
            .key("empty-state")
            .w_full()
            .max_w(measure(wide))
            .px(16.0)
            .pb(12.0)
            .gap(16.0);
        if let Some(captain) = t.captain {
            introduction = introduction.child(
                row()
                    .gap(12.0)
                    .center_y()
                    .child(avatar(s, p, captain, 40.0, shown_status(s)))
                    .child(
                        col()
                            .gap(3.0)
                            .child(
                                row()
                                    .gap(8.0)
                                    .center_y()
                                    .child(label(
                                        format!("Captain {}", s.captains[captain].name),
                                        TextStyle::new(16.0, 560.0),
                                        p.ink,
                                    ))
                                    .child({
                                        let working = s
                                            .threads
                                            .iter()
                                            .filter(|thread| {
                                                thread.captain == Some(captain)
                                                    && !thread.captain_home
                                                    && !thread.archived
                                                    && thread.status.busy()
                                            })
                                            .count();
                                        if working == 0 {
                                            status(shown_status(s), p)
                                        } else {
                                            row()
                                                .center_y()
                                                .gap(6.0)
                                                .child(status_mark(Status::Running, p, 6.0))
                                                .child(label(
                                                    format!(
                                                        "{working} {} working",
                                                        if working == 1 {
                                                            "thread"
                                                        } else {
                                                            "threads"
                                                        }
                                                    ),
                                                    SMALL,
                                                    status_ink(Status::Running, p),
                                                ))
                                        }
                                    }),
                            )
                            .child(label(
                                format!(
                                    "{} · {}",
                                    s.captains[captain].role,
                                    s.captains[captain]
                                        .instructions
                                        .lines()
                                        .next()
                                        .unwrap_or("")
                                ),
                                SMALL,
                                p.muted,
                            )),
                    ),
            );
        } else {
            introduction = introduction.child(
                row()
                    .gap(6.0)
                    .center_y()
                    .child(icon("folder", p.muted))
                    .child(label(
                        if s.demo {
                            "native-workspace".into()
                        } else {
                            s.workspace
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        },
                        SMALL,
                        p.muted,
                    )),
            );
        }
        if t.restoring_history() {
            introduction = introduction.child(label("Restoring conversation...", SMALL, p.muted));
        }
        return introduction.child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(suggestion(
                    "suggest-review",
                    "Understand project",
                    "Read this project and explain its architecture. Do not change any files.",
                    p,
                )
                .opacity(if s.demo { 0.5 } else { 1.0 })
                .disabled(s.demo))
                .child(suggestion(
                    "suggest-plan",
                    "Plan a change",
                    "Help me plan a change to this project. Start by asking what I want to build.",
                    p,
                )
                .opacity(if s.demo { 0.5 } else { 1.0 })
                .disabled(s.demo)),
        );
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
                row().justify_end().child(
                    label(&message.text, body_style(wide), p.ink)
                        .key(format!("{key}-text"))
                        .wrap_anywhere()
                        .shrink()
                        .selectable(true)
                        .max_w(if wide { 660.0 } else { 620.0 })
                        .px(14.0)
                        .py(9.0)
                        .radius(10.0)
                        .fill(if s.dark { p.card } else { color("#ededf0") }),
                )
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
        .items_center()
        .child(messages)
}

fn suggestion(key: &str, title: &str, prompt: &'static str, p: Palette) -> El {
    button(key, title, p, move |s| {
        s.current_mut().draft.replace_text(prompt)
    })
    .self_start()
    .h(30.0)
    .px(12.0)
    .radius(15.0)
    .fill(p.card)
    .border(0.5, p.line)
}

fn composer(s: &State, p: Palette, narrow: bool, wide: bool) -> El {
    let t = s.current();
    let mut area = col()
        .w_full()
        .max_w(measure(wide))
        .px(16.0)
        .pb(12.0)
        .gap(6.0);
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
    if t.request.is_some() {
        area = area.child(dialog(s, p));
    }
    let can_send = !s.demo
        && !t.archived
        && !t.status.busy()
        && !t.configuring
        && !t.draft.text.trim().is_empty();
    let mut send = el()
        .key("send-message")
        .role("button")
        .label("Send message")
        .size(28.0, 28.0)
        .radius(7.0)
        .center()
        .fill(if can_send {
            p.teal
        } else if s.dark {
            Color::WHITE.alpha(0.12)
        } else {
            p.selected
        })
        .child(icon("send", if can_send { p.card } else { p.muted }))
        .disabled(!can_send);
    if can_send {
        send = send.focusable(true).on_press(|s: &mut State| s.send());
    }
    let action = if t.status.busy() && !s.demo {
        icon_button("stop-agent", "Stop agent", "stop", p, |s| {
            s.current_mut().stop()
        })
        .border(1.0, p.line)
    } else {
        send
    };
    let locked = s.demo || t.archived || t.status.busy() || t.configuring;
    let identity = if let Some(captain) = t.captain {
        avatar(s, p, captain, 16.0, shown_status(s))
    } else {
        el().size(16.0, 16.0)
            .center()
            .child(icon("thread", p.muted))
    };
    area = area.child(
        col()
            .key("composer")
            .radius(12.0)
            .fill(p.card)
            .border(if s.dark { 1.0 } else { 0.5 }, p.line)
            .shadow(
                2.0,
                6.0,
                Color::BLACK.alpha(if s.dark { 0.12 } else { 0.06 }),
            )
            .px(14.0)
            .pt(12.0)
            .pb(10.0)
            .gap(10.0)
            .child(
                text_input(&t.draft, body_style(wide))
                    .key("composer-input")
                    .label("Message Pi")
                    .placeholder(
                        if s.demo {
                            "Open a real workspace to send messages"
                        } else {
                            "Ask anything..."
                        },
                        p.muted,
                    )
                    .color(p.ink)
                    .caret(p.teal, p.teal_bg)
                    .w_full()
                    .min_h(24.0)
                    .max_h(170.0)
                    .on_edit(|s: &mut State| &mut s.current_mut().draft)
                    .on_submit(|s: &mut State, _| {
                        if !s.demo && !s.current().archived {
                            s.send();
                        }
                    }),
            )
            .child(
                row()
                    .center_y()
                    .gap(5.0)
                    .child(identity)
                    .child(
                        dropdown("choose-pi-model", &t.model, p, |s| {
                            s.model_search.replace_text("");
                            s.settings_picker = Some(SettingsPicker::Model);
                            s.refresh_catalog();
                        })
                        .max_w(if narrow { 112.0 } else { 230.0 })
                        .opacity(if locked { 0.65 } else { 1.0 })
                        .disabled(locked),
                    )
                    .child(
                        dropdown("choose-pi-thinking", &thinking_label(&t.thinking), p, |s| {
                            s.settings_picker = Some(SettingsPicker::Thinking);
                            s.refresh_catalog();
                        })
                        .opacity(if locked { 0.65 } else { 1.0 })
                        .disabled(locked),
                    )
                    .child(spacer())
                    .child(action),
            ),
    );
    area = area.child(
        row()
            .h(20.0)
            .center_y()
            .gap(5.0)
            .child(icon("folder", p.muted))
            .child(label("Local", SMALL, p.muted))
            .child(label("·", SMALL, p.muted))
            .child(
                label(
                    if s.demo { "Demo data" } else { "No sandbox" },
                    SMALL,
                    p.muted,
                )
                .wrap()
                .grow(1.0),
            )
            .child(if s.demo {
                row()
                    .key("leave-example")
                    .role("button")
                    .label("Open a real workspace")
                    .focusable(true)
                    .center_y()
                    .h(20.0)
                    .px(6.0)
                    .radius(5.0)
                    .hover(p.teal_bg)
                    .on_press(|s: &mut State| {
                        let path = s.workspace.clone();
                        s.switch_workspace(path);
                    })
                    .child(label(
                        "Open a real workspace",
                        TextStyle::new(11.5, 500.0),
                        p.teal,
                    ))
            } else {
                label("Shift Enter for newline", SMALL, p.muted)
            }),
    );
    col().w_full().items_center().child(area)
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
            actions = actions.child(button("dialog-confirm", "Confirm", p, |s| {
                s.current_mut().respond(json!({"confirmed":true}))
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
    use rune::{Env, Runtime};
    fn runtime(width: f32) -> Runtime<State> {
        Runtime::new(
            State::new(".".into(), false),
            view,
            (width, 720.0),
            Env::new(false, 1.0),
        )
    }
    fn press(rt: &mut Runtime<State>, key: &str) {
        let (x, y) = rt.find(key).unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
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
    fn new_thread_and_suggestion_are_native_interactions() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        press(&mut rt, "suggest-review");
        assert!(
            rt.state
                .current()
                .draft
                .text
                .starts_with("Read this project")
        );
        assert!(rt.state.current().pi.is_none());
        press(&mut rt, "new-thread");
        assert_eq!(rt.state.threads.len(), 2);
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
        press(&mut rt, "toggle-sidebar");
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
    fn tool_groups_and_individual_outputs_expand_independently() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        assert!(rt.find("toggle-tool-group-1-0").is_some());
        assert!(rt.find("inline-tool-1-0").is_none());
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
    fn shell_controls_are_below_the_title_strip_and_panel_controls_stay_inside() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        let (strip, _) = rt.presented("top-strip").unwrap();
        for key in ["sidebar-toggle", "thread-actions", "open-session-panel"] {
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
        });
        rt.frame(0.0);
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
    fn streamed_preamble_stays_above_the_heading_when_the_first_tool_arrives() {
        let mut rt = runtime(1280.0);
        rt.state.current_mut().apply(json!({"type":"agent_start"}));
        rt.state
            .current_mut()
            .apply(json!({"type":"message_update","assistantMessageEvent":{
                "type":"text_delta","contentIndex":0,"delta":"I'll check the file."
            }}));
        rt.frame(0.0);
        let header_y = rt.presented("toggle-work-1-0").unwrap().0[1];
        assert!(rt.presented("message-1-0-body-0").unwrap().0[1] < header_y);
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
        assert_eq!(rt.presented("toggle-work-1-0").unwrap().0[1], header_y);
        assert!(rt.presented("inline-tool-1-0").is_some());
    }

    #[test]
    fn a_running_turn_keeps_the_same_work_header_after_settlement() {
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
        let (header, _) = rt.presented("toggle-work-1-0").unwrap();
        let (thought, _) = rt.presented("inline-thought-1-0").unwrap();
        assert!(header[1] < thought[1]);
        rt.state
            .current_mut()
            .apply(json!({"type":"agent_settled"}));
        rt.invalidate();
        rt.frame(1.0);
        assert_eq!(rt.presented("toggle-work-1-0").unwrap().0[1], header[1]);
        assert!(rt.state.current().work[0].expanded);
    }
    #[test]
    fn captain_picker_opens_a_separate_conversation_without_starting_pi() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        press(&mut rt, "browse-captains");
        assert!(rt.state.captain_picker);
        rt.frame(1.0 / 60.0);
        press(&mut rt, "choose-captain-2");
        assert!(!rt.state.captain_picker);
        assert_eq!(rt.state.current().captain, Some(2));
        assert!(rt.state.current().captain_home);
        assert!(rt.state.current().pi.is_none());
        press(&mut rt, "new-thread");
        assert_eq!(rt.state.current().captain, Some(2));
        assert!(!rt.state.current().captain_home);
    }
    #[test]
    fn fourth_captain_opens_from_the_picker() {
        let mut rt = runtime(1280.0);
        rt.frame(0.0);
        press(&mut rt, "browse-captains");
        rt.frame(1.0 / 60.0);
        press(&mut rt, "choose-captain-3");
        assert_eq!(rt.state.current().captain, Some(3));
        assert!(rt.state.current().captain_home);
        assert_eq!(rt.state.current().title, "Pip");
    }
    #[test]
    fn new_captain_button_creates_a_profile_instead_of_reopening_a_preset() {
        let mut rt = runtime(1280.0);
        rt.state.current_mut().draft.replace_text("Keep this draft");
        rt.frame(0.0);
        press(&mut rt, "new-captain");
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
        assert!(rt.find("captain-4").is_some());
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
    fn delegated_children_follow_their_parent_in_the_sidebar() {
        let mut rt = runtime(1280.0);
        rt.state.threads[0].title = "Parent".into();
        rt.state.threads[0].pinned = true;
        let mut child = crate::model::Thread::new(2);
        child.title = "Delegated child".into();
        child.parent = Some(1);
        child.delegated = true;
        child.captain = Some(2);
        child.pinned = true;
        rt.state.threads.push(child);
        rt.frame(0.0);
        let (parent, _) = rt.presented("thread-1").unwrap();
        let (child, _) = rt.presented("thread-2").unwrap();
        assert!(child[0] > parent[0] && child[1] > parent[1]);
        rt.state.search.replace_text("Delegated");
        rt.frame(0.0);
        assert!(
            rt.presented("thread-1").is_some(),
            "a matching child keeps its parent visible"
        );
        press(&mut rt, "thread-2");
        assert_eq!(rt.state.current().id, 2);
        assert!(rt.presented("thread-2").is_some());
    }

    #[test]
    fn cross_captain_children_of_a_home_are_visible_and_archived_children_still_have_rows() {
        let mut rt = runtime(1280.0);
        rt.state.open_captain(0);
        let parent = rt.state.current().id;
        let mut child = crate::model::Thread::new(3);
        child.title = "Cross captain child".into();
        child.parent = Some(parent);
        child.delegated = true;
        child.captain = Some(1);
        rt.state.threads.push(child);
        rt.frame(0.0);
        assert!(rt.presented("thread-3").is_some());
        rt.state.threads.last_mut().unwrap().archived = true;
        rt.state.show_archive = true;
        rt.frame(0.0);
        assert!(rt.presented("thread-3").is_some());
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
        assert!(rt.state.current().pinned);
        assert_eq!(rt.state.current().draft.text, "Keep this draft");
        assert!(rt.find("thread-1").is_some());
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
    fn composer_has_no_connection_controls_and_pickers_work_without_a_process() {
        let mut rt = runtime(1280.0);
        rt.state.current_mut().apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"provider":"cpa","id":"model-a","name":"Model A","reasoning":true,
             "thinkingLevelMap":{"minimal":null,"low":null,"medium":null}}
        ]}}));
        rt.frame(0.0);
        assert!(rt.find("connect-pi").is_none());
        assert!(rt.find("disconnect-pi").is_none());
        press(&mut rt, "choose-pi-model");
        rt.frame(1.0 / 60.0);
        press(&mut rt, "choose-model-0");
        assert!(rt.state.current().pi.is_none());
        assert_eq!(rt.state.current().model_id, "model-a");
        rt.frame(0.0);
        press(&mut rt, "choose-pi-thinking");
        rt.frame(1.0 / 60.0);
        press(&mut rt, "choose-thinking-1");
        assert_eq!(rt.state.current().thinking, "high");
        assert!(rt.state.current().pi.is_none());
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
        press(&mut rt, "choose-pi-model");
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
        rt.frame(0.0);
        press(&mut rt, "choose-pi-thinking");
        rt.frame(1.0 / 60.0);
        press(&mut rt, "choose-thinking-1");
        let thinking = commands.try_recv().unwrap();
        assert_eq!(thinking["type"], "set_thinking_level");
        assert_eq!(thinking["level"], "high");
        rt.state
            .current_mut()
            .apply(json!({"type":"response","command":"set_thinking_level","success":true}));
        assert_eq!(rt.state.current().thinking, "high");
    }
}
