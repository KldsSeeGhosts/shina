use crate::model::{SettingsPicker, State, Status, truncate};
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
            line: Color::WHITE.alpha(0.10),
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
            bg: rune::color::oklch(0.962, 0.0025, 290.0),
            panel: rune::color::oklch(0.985, 0.0015, 290.0),
            card: Color::WHITE,
            line: Color::BLACK.alpha(0.08),
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
const LABEL: TextStyle = TextStyle::new(13.0, 420.0).tracking(-0.13);
const SMALL: TextStyle = TextStyle::new(11.5, 420.0);

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
        "lock" => &rune_kit::glyphs::LOCK,
        "close" => &rune_kit::glyphs::CLOSE,
        "activity" => &PANEL,
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
        Status::Running | Status::Settled => p.teal_text,
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
fn status(status: Status, p: Palette) -> El {
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
    let docked = (s.sidebar && !compact) || (compact && s.compact_sidebar);
    let chrome = cx.chrome_insets();
    // One strip at the titlebar's height holds the sidebar header, tabs, actions, and the
    // window's caption buttons, so they share a centre line and never overlap the pane.
    let mut top = row()
        .key("top-strip")
        .h(chrome.height.max(40.0))
        .window_drag()
        .center_y()
        .pl(8.0)
        .pr(chrome.trailing.max(8.0));
    if docked {
        top = top.child(
            sidebar_header(s, p)
                .w(if narrow { 216.0 } else { 232.0 })
                .pr(8.0),
        );
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
    if s.show_activity && narrow {
        pane = pane.child(activity(s, p, true));
    } else {
        let mut body = row().grow(1.0).min_h(0.0);
        let main = col().grow(1.0).min_w(0.0).min_h(0.0);
        let main = if s.current().messages.is_empty() {
            main.items_center()
                .child(spacer())
                .child(conversation(s, p))
                .child(composer(s, p, narrow))
                .child(spacer())
        } else {
            main.child(conversation(s, p)).child(composer(s, p, narrow))
        };
        body = body.child(main);
        if s.show_activity {
            body = body
                .child(el().w(1.0).h_full().fill(p.line))
                .child(activity(s, p, false));
        }
        pane = pane.child(body);
    }
    let root = col()
        .key("workspace-root")
        .w_full()
        .h_full()
        .fill(p.bg)
        .child(top)
        .child(workspace.child(pane));
    if s.captain_editor.is_some() {
        let height = cx.layout_with("workspace-root", |r| r[3]).unwrap_or(840.0);
        root.child(captain_editor(s, p, height))
    } else if s.settings_picker.is_some() {
        root.child(settings_picker(s, p))
    } else if s.captain_picker {
        root.child(captain_picker(s, p))
    } else {
        root
    }
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
    let can_choose = !t.status.busy() && !t.configuring && t.pi.is_some();
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
                    if t.status == Status::Connecting {
                        "Reading Pi's configured models..."
                    } else if t.pi.is_none() {
                        "Connect Pi to read its configured models."
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
    el().key("pi-settings-backdrop")
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(Color::BLACK.alpha(0.40))
        .center()
        .modal(false, |s: &mut State| s.settings_picker = None)
        .on_press(|s: &mut State| s.settings_picker = None)
        .child(card)
}

/// Logo, search, and sidebar toggle. Lives in the window's top strip above the sidebar.
fn sidebar_header(s: &State, p: Palette) -> El {
    row()
        .h(36.0)
        .window_drag()
        .center_y()
        .px(6.0)
        .child(
            el().size(24.0, 24.0).radius(6.0).clip_rounded(6.0).pixels(
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
                .px(7.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .hover(p.selected)
                .on_press(move |s: &mut State| s.open_captain(captain))
                .child(avatar(s, p, captain, 24.0, state))
                .child(label(
                    &s.captains[captain].name,
                    TextStyle::new(13.0, if selected { 560.0 } else { 450.0 }),
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
            if state == Status::Settled {
                captain_row = captain_row.child(icon("check", p.teal));
            }
            content = content.child(captain_row);
            if selected {
                let mut nested = col()
                    .ml(19.0)
                    .pl(6.0)
                    .gap(1.0)
                    .border_sides([0.0, 0.0, 0.0, 1.0], p.line);
                let mut any = false;
                for (index, _) in s.threads.iter().enumerate().filter(|(_, t)| {
                    t.captain == Some(captain)
                        && !t.captain_home
                        && !t.pinned
                        && !t.archived
                        && t.title.to_lowercase().contains(&search)
                }) {
                    nested = nested.child(thread_row(s, index, p, true));
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
                && t.title.to_lowercase().contains(&search)
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
                .child(label(pinned.len().to_string(), SMALL, p.muted.alpha(0.7)).pr(6.0)),
        );
        for index in pinned {
            content = content.child(thread_row(s, index, p, false));
        }
    }
    content = content.child(
        row()
            .h(31.0)
            .center_y()
            .gap(6.0)
            .px(8.0)
            .child(icon("folder", p.muted))
            .child(
                label(
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
                )
                .truncate()
                .grow(1.0),
            ),
    );
    let mut count = 0;
    for (index, thread) in s.threads.iter().enumerate().filter(|(_, t)| {
        t.archived == s.show_archive
            && !t.captain_home
            && !t.pinned
            && t.title.to_lowercase().contains(&search)
    }) {
        if thread.captain == s.current().captain && thread.captain.is_some() && !s.show_archive {
            continue;
        }
        count += 1;
        content = content.child(thread_row(s, index, p, false));
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
    side = side.child(rule(p)).child(
        row()
            .h(48.0)
            .px(6.0)
            .gap(8.0)
            .center_y()
            .child(
                el().key("open-workspace")
                    .role("button")
                    .label("Open folder")
                    .focusable(true)
                    .size(28.0, 28.0)
                    .radius(7.0)
                    .fill(p.teal_bg)
                    .center()
                    .on_press_tx(|s: &mut State, tx| {
                        tx.open_folder(
                            rune::FileDialogOptions {
                                title: "Open a project folder".into(),
                                directory: Some(s.workspace.clone()),
                                ..Default::default()
                            },
                            |s: &mut State, result, _| match result {
                                Ok(Some(path)) => s.switch_workspace(path),
                                Ok(None) => {}
                                Err(error) => {
                                    s.current_mut().error =
                                        Some(format!("Could not open the folder picker: {error}"))
                                }
                            },
                        )
                    })
                    .child(icon("folder", p.teal)),
            )
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
        Status::Offline => Face::Sad,
        Status::Connecting | Status::Ready => Face::Normal,
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
    let mut portrait = el()
        .key(format!("avatar-{captain}"))
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
    if status == Status::Offline {
        portrait = portrait.opacity(0.62);
    }
    if matches!(status, Status::Running | Status::Waiting | Status::Failed) {
        // A backing disc in the surface color keeps the mark readable over the artwork.
        let (backing, mark) = if size >= 30.0 {
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
        .child(label(
            if status == Status::Offline {
                "Not started"
            } else {
                status.label()
            },
            SMALL,
            status_ink(status, p),
        ))
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

    let mut title = row().gap(6.0).center_y();
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
    let detailed = selected
        || matches!(
            thread.status,
            Status::Running | Status::Waiting | Status::Failed | Status::Connecting
        );
    if !detailed {
        return item.child(title);
    }
    let mut meta = row()
        .gap(5.0)
        .center_y()
        .pl(if nested { 0.0 } else { 20.0 });
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
        let open = s.current().captain == Some(captain);
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
        .fill(Color::BLACK.alpha(0.40))
        .center()
        .modal(false, |s: &mut State| s.captain_picker = false)
        .on_press(|s: &mut State| s.captain_picker = false)
        .child(card)
}

fn captain_editor(s: &State, p: Palette, height: f32) -> El {
    let editor = s.captain_editor.as_ref().unwrap();
    let editing = editor.editing.is_some();
    let profile = editor.profile();
    let field = if s.dark { p.bg } else { p.card };
    let mut portraits = row().gap(8.0).w_full();
    for portrait in 0..crate::visuals::CAPTAIN_NAMES.len() {
        portraits = portraits.child(
            col()
                .key(format!("captain-portrait-{portrait}"))
                .role("button")
                .label(format!(
                    "Use {} portrait",
                    crate::visuals::CAPTAIN_NAMES[portrait]
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
                .child(label(crate::visuals::CAPTAIN_NAMES[portrait], SMALL, p.ink)),
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
        .fill(Color::BLACK.alpha(0.40))
        .center()
        .modal(false, |s: &mut State| s.captain_editor = None)
        .on_press(|s: &mut State| s.captain_editor = None)
        .child(card)
}

fn header(s: &State, p: Palette, narrow: bool, compact: bool) -> El {
    // Placed in the window's top strip; the strip handles dragging and caption-button room.
    let docked = if compact {
        s.compact_sidebar
    } else {
        s.sidebar
    };
    row()
        .h(36.0)
        .window_drag()
        .px(6.0)
        .center_y()
        .gap(6.0)
        .child(if docked {
            el()
        } else {
            icon_button("toggle-sidebar", "Toggle sidebar", "sidebar", p, move |s| {
                if compact {
                    s.compact_sidebar = true;
                } else {
                    s.sidebar = true;
                }
            })
        })
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
        .child(if s.demo {
            row()
                .key("demo-badge")
                .h(20.0)
                .px(8.0)
                .radius(10.0)
                .center_y()
                .fill(p.gold.alpha(0.12))
                .label("Demo data, read-only")
                .child(label("Demo", TextStyle::new(11.0, 560.0), p.gold))
        } else {
            status(s.current().status, p)
        })
        .child(if narrow || s.current().captain_home {
            el()
        } else {
            icon_button(
                "pin-thread",
                if s.current().pinned {
                    "Unpin thread"
                } else {
                    "Pin thread"
                },
                "pin",
                p,
                |s| s.toggle_pin(),
            )
        })
        .child(icon_button(
            "toggle-activity",
            "Toggle activity",
            "activity",
            p,
            |s| s.show_activity = !s.show_activity,
        ))
        .child(if s.current().status.busy() {
            el()
        } else {
            icon_button(
                "archive-thread",
                if s.current().archived {
                    "Restore thread"
                } else {
                    "Archive thread"
                },
                "archive",
                p,
                |s| s.archive(),
            )
        })
}

fn conversation(s: &State, p: Palette) -> El {
    let t = s.current();
    let mut messages = col()
        .w_full()
        .max_w(768.0)
        .px(16.0)
        .pt(20.0)
        .pb(24.0)
        .gap(18.0);
    if t.messages.is_empty() {
        let mut introduction = col()
            .key("empty-state")
            .w_full()
            .max_w(768.0)
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
                                    .child(status(shown_status(s), p)),
                            )
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
                                    el()
                                } else {
                                    row()
                                        .gap(6.0)
                                        .center_y()
                                        .child(el().size(5.0, 5.0).radius(2.5).fill(p.teal))
                                        .child(label(
                                            format!(
                                                "{working} {} working",
                                                if working == 1 {
                                                    "thread is"
                                                } else {
                                                    "threads are"
                                                }
                                            ),
                                            SMALL,
                                            p.teal,
                                        ))
                                }
                            })
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
        if t.session.is_some() {
            introduction = introduction.child(label(
                "Connect Pi to resume this conversation.",
                SMALL,
                p.muted,
            ));
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
                ))
                .child(suggestion(
                    "suggest-plan",
                    "Plan a change",
                    "Help me plan a change to this project. Start by asking what I want to build.",
                    p,
                )),
        );
    } else {
        let start = t.messages.len().saturating_sub(100);
        if start > 0 {
            messages = messages.child(label("Showing the latest 100 messages.", SMALL, p.muted));
        }
        for (index, message) in t.messages.iter().enumerate().skip(start) {
            let key = format!("message-{}-{index}", t.id);
            let body = if message.role == "You" {
                row().justify_end().child(
                    label(&message.text, BODY, p.ink)
                        .key(format!("{key}-text"))
                        .wrap_anywhere()
                        .shrink()
                        .selectable(true)
                        .max_w(620.0)
                        .px(14.0)
                        .py(9.0)
                        .radius(10.0)
                        .fill(if s.dark { p.card } else { color("#ededf0") }),
                )
            } else {
                let style = MarkdownStyle::neutral(s.dark)
                    .text(p.ink)
                    .muted(p.muted)
                    .body_font(if s.dark {
                        TextStyle::new(13.0, 380.0).lh(19.5).tracking(-0.13)
                    } else {
                        BODY
                    })
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
                let streaming = t.status == Status::Running && index + 1 == t.messages.len();
                let mut response = col().gap(10.0);
                if index == 0 || t.messages[index - 1].role != "Pi" {
                    let mut heading = row().center_y().gap(7.0);
                    if let Some(captain) = t.captain {
                        heading = heading.child(avatar(s, p, captain, 20.0, t.status));
                    }
                    heading = heading
                        .child(label(
                            t.captain
                                .map(|id| format!("Captain {}", s.captains[id].name))
                                .unwrap_or("Pi".into()),
                            TextStyle::new(13.0, 600.0),
                            p.ink,
                        ))
                        .child(label(&t.model, SMALL, p.muted));
                    response = response.child(heading);
                } else if let Some(seconds) = t.elapsed {
                    response = response.child(disclosure(
                        "worked-summary",
                        &format!("Worked for {seconds}s"),
                        p,
                        |s| s.show_activity = !s.show_activity,
                    ));
                }
                response.child(memo(
                    format!("{key}-body"),
                    (&message.text, s.dark, streaming),
                    || {
                        Markdown::new(&message.text)
                            .streaming(streaming)
                            .scroll_wide(true)
                            .render(&style)
                    },
                ))
            };
            messages = messages.child(body.key(key));
        }
        if !t.tools.is_empty() {
            let count = t.tools.len();
            messages = messages.child(
                disclosure("tool-summary", &format!("{count} tool calls"), p, |s| {
                    s.show_activity = true
                })
                .mt(-8.0),
            );
        }
        if t.status == Status::Running {
            messages = messages.child(label(
                if t.phase.is_empty() {
                    "Pi is working"
                } else {
                    &t.phase
                },
                SMALL,
                p.muted,
            ));
        }
    }
    col()
        .key(format!("conversation-{}", t.id))
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: !t.messages.is_empty(),
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

/// A secondary, collapsible summary such as "Worked for 42s".
fn disclosure(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    row()
        .key(key)
        .role("button")
        .label(title)
        .focusable(true)
        .self_start()
        .center_y()
        .gap(4.0)
        .h(22.0)
        .pl(6.0)
        .pr(4.0)
        .ml(-6.0)
        .radius(6.0)
        .hover(p.selected)
        .on_press(action)
        .child(label(title, TextStyle::new(12.0, 420.0), p.muted))
        .child(icon("chevron", p.muted))
}

fn composer(s: &State, p: Palette, narrow: bool) -> El {
    let t = s.current();
    let mut area = col()
        .w_full()
        .max_w(768.0)
        .px(if narrow { 12.0 } else { 16.0 })
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
                ),
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
        .fill(if can_send { p.teal } else { p.selected })
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
            .border(0.5, p.line)
            .shadow(
                2.0,
                6.0,
                Color::BLACK.alpha(if s.dark { 0.12 } else { 0.035 }),
            )
            .px(14.0)
            .pt(12.0)
            .pb(10.0)
            .gap(10.0)
            .child(
                text_input(&t.draft, BODY)
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
                    .child(
                        el().size(20.0, 24.0)
                            .center()
                            .child(icon("plus", p.muted))
                            .role("button")
                            .label("Attachments are not implemented")
                            .opacity(0.4)
                            .disabled(true),
                    )
                    .child(identity)
                    .child(
                        dropdown("choose-pi-model", &t.model, p, |s| {
                            s.model_search.replace_text("");
                            s.settings_picker = Some(SettingsPicker::Model);
                            s.connect();
                        })
                        .max_w(if narrow { 112.0 } else { 230.0 })
                        .disabled(s.demo || t.archived || t.status.busy() || t.configuring),
                    )
                    .child(
                        dropdown("choose-pi-thinking", &thinking_label(&t.thinking), p, |s| {
                            s.settings_picker = Some(SettingsPicker::Thinking);
                            s.connect();
                        })
                        .disabled(s.demo || t.archived || t.status.busy() || t.configuring),
                    )
                    .child(spacer())
                    .child(if s.demo {
                        el()
                    } else if t.pi.is_none() {
                        button("connect-pi", "Connect Pi", p, |s| s.connect()).fill(p.teal_bg)
                    } else if !t.status.busy() {
                        button("disconnect-pi", "Disconnect", p, |s| {
                            s.current_mut().disconnect()
                        })
                    } else {
                        el()
                    })
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
    let mut pane = col()
        .key("activity-pane")
        .min_h(0.0)
        .gap(10.0)
        .padding(12.0);
    pane = if narrow {
        pane.grow(1.0)
    } else {
        pane.w(320.0).h_full()
    };
    pane = pane.child(
        row()
            .center_y()
            .gap(6.0)
            .child(icon("list", p.muted))
            .child(label("Overview", TextStyle::new(12.0, 450.0), p.ink))
            .child(spacer())
            .child(status(t.status, p)),
    );
    pane = pane
        .child(
            label("Session", SMALL, p.muted)
                .w_full()
                .fill(p.selected)
                .px(6.0)
                .py(5.0),
        )
        .child(label(&t.model, LABEL, p.ink).wrap())
        .child(
            label(
                if t.session.is_some() {
                    "Pi session saved on this computer"
                } else {
                    "Session not yet saved"
                },
                SMALL,
                p.muted,
            )
            .wrap(),
        )
        .child(
            label("Tool activity", SMALL, p.ink)
                .w_full()
                .fill(p.selected)
                .px(6.0)
                .py(5.0),
        );
    let mut tools = col().gap(10.0);
    for diagnostic in &t.diagnostics {
        tools = tools.child(
            label(truncate(diagnostic, 600), SMALL, p.muted)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    if t.tools.is_empty() {
        tools = tools.child(
            label(
                "Tool calls will appear here when Pi works on your project.",
                BODY,
                p.muted,
            )
            .wrap(),
        );
    }
    for tool in t.tools.iter().rev().take(50) {
        let ink = if tool.failed { p.danger } else { p.muted };
        let mut item = col()
            .key(format!("tool-{}", tool.id))
            .gap(6.0)
            .px(6.0)
            .py(7.0)
            .child(
                row()
                    .center_y()
                    .gap(7.0)
                    .child(icon(if tool.done { "check" } else { "dot" }, ink))
                    .child(label(&tool.name, LABEL, p.ink))
                    .child(spacer())
                    .child(label(
                        if tool.failed {
                            "Failed"
                        } else if tool.done {
                            "Done"
                        } else {
                            "Running"
                        },
                        SMALL,
                        ink,
                    )),
            )
            .child(
                label(&tool.detail, SMALL, p.muted)
                    .wrap_anywhere()
                    .selectable(true),
            );
        if t.expanded_tools && !tool.output.is_empty() {
            item = item.child(
                col()
                    .max_h(200.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(
                        label(&tool.output, SMALL, p.ink)
                            .wrap_anywhere()
                            .selectable(true),
                    ),
            );
        }
        tools = tools.child(item);
    }
    pane.child(button(
        "expand-tool-output",
        if t.expanded_tools {
            "Hide tool output"
        } else {
            "Show tool output"
        },
        p,
        |s| s.current_mut().expanded_tools = !s.current().expanded_tools,
    ))
    .child(
        col()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(tools),
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
mod tests {
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
        for key in ["toggle-activity", "archive-thread", "leave-example"] {
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
    fn pinned_captain_child_has_one_row_and_retains_its_draft() {
        let mut rt = runtime(1280.0);
        rt.state.populate_demo();
        rt.frame(0.0);
        rt.state.current_mut().draft.replace_text("Keep this draft");
        press(&mut rt, "pin-thread");
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
    fn native_model_and_reasoning_pickers_send_real_rpc_commands() {
        let mut rt = runtime(1280.0);
        let (pi, commands, _events) = crate::backend::Pi::test_connection();
        rt.state.current_mut().pi = Some(pi);
        rt.state.current_mut().status = Status::Ready;
        rt.state.current_mut().apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"provider":"cpa","id":"model-a","name":"Model A"}
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
