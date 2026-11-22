//! Shina's icons: Lucide outlines (ISC licence, see `assets/icons-LICENSE.txt`) on Lucide's
//! 24 unit grid, stroked in `currentColor` with round caps and joins, as KSG Code draws them
//! through `lucide-react`. A few glyphs (the filled dot, stop square and pinned pin) are filled.

use rune::Icon;

macro_rules! glyph {
    ($name:ident,$body:expr) => {
        pub(super) static $name: Icon = Icon {
            name: stringify!($name),
            view: 24.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</g>"
            ),
        };
    };
}

glyph!(PLUS, r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#);
glyph!(
    RELOAD,
    r#"<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>"#
);
glyph!(
    MINIMIZE,
    r#"<path d="M4 14h6v6"/><path d="M20 10h-6V4"/><path d="m14 10 7-7"/><path d="m3 21 7-7"/>"#
);
glyph!(
    PAPERCLIP,
    r#"<path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/>"#
);
glyph!(CHEVRON_LEFT, r#"<path d="m15 18-6-6 6-6"/>"#);
glyph!(CHEVRON_RIGHT, r#"<path d="m9 18 6-6-6-6"/>"#);
glyph!(
    AGENTS,
    r#"<path d="M12 8V4H8"/><rect x="4" y="8" width="16" height="12" rx="2"/><path d="M2 14h2m16 0h2m-13-1v2m6-2v2"/>"#
);
glyph!(
    ARROW_UP_CIRCLE,
    r#"<circle cx="12" cy="12" r="10"/><path d="m16 12-4-4-4 4m4-4v8"/>"#
);
glyph!(
    LOCK_OPEN,
    r#"<rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 9.9-1"/>"#
);
glyph!(
    CORNER_DOWN_LEFT,
    r#"<path d="m9 10-5 5 5 5m11-16v7a4 4 0 0 1-4 4H4"/>"#
);
glyph!(
    EXTERNAL_LINK,
    r#"<path d="M15 3h6v6m0-6-9 9m6 1v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#
);
glyph!(
    PULL_REQUEST,
    r#"<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M6 9v6m12 0V9a3 3 0 0 0-3-3h-2m2-3-3 3 3 3"/>"#
);
glyph!(
    REFRESH,
    r#"<path d="M20 7v5h-5M4 17v-5h5m-5 0a8 8 0 0 1 13.7-5.7L20 9m0 3a8 8 0 0 1-13.7 5.7L4 15"/>"#
);
glyph!(CHEVRON_DOWN, r#"<path d="m6 9 6 6 6-6"/>"#);
glyph!(CHEVRON_UP, r#"<path d="m18 15-6-6-6 6"/>"#);
glyph!(
    MONITOR,
    r#"<rect width="20" height="14" x="2" y="3" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/>"#
);
glyph!(
    SERVER,
    r#"<rect width="20" height="8" x="2" y="2" rx="2"/><rect width="20" height="8" x="2" y="14" rx="2"/><path d="M6 6h.01M6 18h.01"/>"#
);
glyph!(
    SIDEBAR,
    r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>"#
);
glyph!(
    PANEL,
    r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M15 3v18"/>"#
);
glyph!(
    COMPOSE,
    r#"<path d="M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"/>"#
);
glyph!(
    PENCIL,
    r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#
);
glyph!(SEND, r#"<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>"#);
glyph!(
    LIST_PLUS,
    r#"<path d="M11 12H3"/><path d="M16 6H3"/><path d="M16 18H3"/><path d="M18 9v6"/><path d="M21 12h-6"/>"#
);
glyph!(
    CORNER_UP_RIGHT,
    r#"<path d="m15 14 5-5-5-5"/><path d="M4 20v-7a4 4 0 0 1 4-4h12"/>"#
);
glyph!(PLAY, r#"<path d="M6 3v18l14-9z"/>"#);
glyph!(
    BUBBLE,
    r#"<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>"#
);
glyph!(
    CLOUD,
    r#"<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/>"#
);
glyph!(
    ISSUE,
    r#"<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="1"/>"#
);
glyph!(
    BOLT,
    r#"<path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/>"#
);
glyph!(
    DOT,
    r#"<circle cx="12" cy="12" r="4" fill="currentColor" stroke="none"/>"#
);
glyph!(
    THEME,
    r#"<circle cx="12" cy="12" r="10"/><path d="M12 18a6 6 0 0 0 0-12v12z"/>"#
);
glyph!(
    SETTINGS,
    r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"#
);
glyph!(
    FILTER,
    r#"<path d="M21 4h-7"/><path d="M10 4H3"/><path d="M21 12h-9"/><path d="M8 12H3"/><path d="M21 20h-5"/><path d="M12 20H3"/><path d="M14 2v4"/><path d="M8 10v4"/><path d="M16 18v4"/>"#
);
glyph!(
    FOLDER,
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#
);
glyph!(
    FOLDER_PLUS,
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/><path d="M12 10v6"/><path d="M9 13h6"/>"#
);
glyph!(
    SUN,
    r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>"#
);
glyph!(MOON, r#"<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>"#);
glyph!(
    CRESCENT,
    r#"<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/><path d="M19 3v4"/><path d="M21 5h-4"/>"#
);
glyph!(
    CHIP,
    r#"<rect width="16" height="16" x="4" y="4" rx="2"/><rect width="6" height="6" x="9" y="9" rx="1"/><path d="M15 2v2"/><path d="M15 20v2"/><path d="M2 15h2"/><path d="M2 9h2"/><path d="M20 15h2"/><path d="M20 9h2"/><path d="M9 2v2"/><path d="M9 20v2"/>"#
);
glyph!(
    ALERT,
    r#"<circle cx="12" cy="12" r="10"/><path d="M12 8v4"/><path d="M12 16h.01"/>"#
);
glyph!(
    PIN,
    r#"<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>"#
);
glyph!(
    PIN_ON,
    r#"<path d="M12 17v5"/><path fill="currentColor" d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>"#
);
glyph!(
    TRASH,
    r#"<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/><path d="M10 11v6"/><path d="M14 11v6"/>"#
);
glyph!(
    CALENDAR,
    r#"<path d="M8 2v4"/><path d="M16 2v4"/><rect width="18" height="18" x="3" y="4" rx="2"/><path d="M3 10h18"/>"#
);
glyph!(
    CLOCK,
    r#"<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>"#
);
glyph!(
    HISTORY,
    r#"<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>"#
);
glyph!(
    MORE,
    r#"<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>"#
);
glyph!(
    LIST,
    r#"<path d="M3 12h.01"/><path d="M3 18h.01"/><path d="M3 6h.01"/><path d="M8 12h13"/><path d="M8 18h13"/><path d="M8 6h13"/>"#
);
glyph!(
    BRANCH,
    r#"<path d="M6 3v12"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>"#
);
glyph!(
    STOP,
    r#"<rect width="12" height="12" x="6" y="6" rx="2" fill="currentColor" stroke="none"/>"#
);
glyph!(
    GRID,
    r#"<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>"#
);
glyph!(
    ARCHIVE,
    r#"<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>"#
);
glyph!(
    SEARCH,
    r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#
);
glyph!(
    LOCK,
    r#"<rect width="18" height="11" x="3" y="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>"#
);
glyph!(CLOSE, r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#);
glyph!(
    VOLUME,
    r#"<path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z"/><path d="M16 9a5 5 0 0 1 0 6"/><path d="M19.364 18.364a9 9 0 0 0 0-12.728"/>"#
);
glyph!(
    VOLUME_MUTED,
    r#"<path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z"/><path d="m22 9-6 6"/><path d="m16 9 6 6"/>"#
);
glyph!(
    FILE,
    r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>"#
);
glyph!(
    TERMINAL,
    r#"<path d="m7 11 2-2-2-2"/><path d="M11 13h4"/><rect width="18" height="18" x="3" y="3" rx="2"/>"#
);
glyph!(
    EXPAND,
    r#"<path d="M15 3h6v6"/><path d="M9 21H3v-6"/><path d="M21 3l-7 7"/><path d="M3 21l7-7"/>"#
);
glyph!(
    IMAGE,
    r#"<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#
);
glyph!(
    EYE,
    r#"<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/>"#
);
glyph!(
    LIGHTBULB,
    r#"<path d="M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5"/><path d="M9 18h6"/><path d="M10 22h4"/>"#
);
glyph!(
    WRENCH,
    r#"<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>"#
);
glyph!(
    CREW,
    r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>"#
);
glyph!(
    USER,
    r#"<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>"#
);
glyph!(
    COPY,
    r#"<rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>"#
);
glyph!(
    UNRESOLVED,
    r#"<path d="M10.1 2.182a10 10 0 0 1 3.8 0"/><path d="M13.9 21.818a10 10 0 0 1-3.8 0"/><path d="M17.609 3.721a10 10 0 0 1 2.69 2.7"/><path d="M2.182 13.9a10 10 0 0 1 0-3.8"/><path d="M20.279 17.609a10 10 0 0 1-2.7 2.69"/><path d="M21.818 10.1a10 10 0 0 1 0 3.8"/><path d="M3.721 6.391a10 10 0 0 1 2.7-2.69"/><path d="M6.391 20.279a10 10 0 0 1-2.69-2.7"/>"#
);
glyph!(
    STAR,
    r#"<path fill="currentColor" d="M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z"/>"#
);
glyph!(
    STAR_OFF,
    r#"<path d="M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z"/>"#
);
glyph!(
    WORKFLOW,
    r#"<rect width="8" height="8" x="3" y="3" rx="2"/><path d="M7 11v4a2 2 0 0 0 2 2h4"/><rect width="8" height="8" x="13" y="13" rx="2"/>"#
);
glyph!(
    SWAP,
    r#"<path d="M8 3 4 7l4 4"/><path d="M4 7h16"/><path d="m16 21 4-4-4-4"/><path d="M20 17H4"/>"#
);
glyph!(
    DIFF,
    r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M9 10h6"/><path d="M12 13V7"/><path d="M9 17h6"/>"#
);
glyph!(CHECK, r#"<path d="M20 6 9 17l-5-5"/>"#);
glyph!(
    CIRCLE_CHECK,
    r#"<circle cx="12" cy="12" r="10"/><path d="m16 9-5.5 5.5L8 12"/>"#
);
glyph!(
    ALARM_CLOCK,
    r#"<circle cx="12" cy="13" r="8"/><path d="M12 9v4l2 2"/><path d="M5 3 2 6"/><path d="m22 6-3-3"/><path d="M6.38 18.7 4 21"/><path d="M17.64 18.67 20 21"/>"#
);
glyph!(
    SHIELD_QUESTION,
    r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="M9 9.5a3 2.5 0 0 1 6 0c0 2-2 2-3 3.5"/><path d="M12 17h.01"/>"#
);
glyph!(
    MESSAGE_QUESTION,
    r#"<path d="M2.992 16.342a2 2 0 0 1 .094 1.167l-1.065 3.29a1 1 0 0 0 1.236 1.168l3.413-.998a2 2 0 0 1 1.099.092 10 10 0 1 0-4.777-4.719"/><path d="M9 9.5a3 2.5 0 0 1 6 0c0 2-2 2-3 3.5"/><path d="M12 17h.01"/>"#
);
glyph!(
    USER_ROUND,
    r#"<circle cx="12" cy="8" r="5"/><path d="M20 21a8 8 0 0 0-16 0"/>"#
);
glyph!(
    FOLDER_GIT2,
    r#"<path d="M18 19a5 5 0 0 1-5-5v8"/><path d="M9 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v5"/><circle cx="13" cy="12" r="2"/><circle cx="20" cy="19" r="2"/>"#
);
glyph!(
    SHIELD,
    r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>"#
);
glyph!(
    REPLY,
    r#"<path d="m9 17-5-5 5-5"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>"#
);
glyph!(
    FORK,
    r#"<circle cx="12" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><path d="M18 9v2c0 .6-.4 1-1 1H7c-.6 0-1-.4-1-1V9"/><path d="M12 12v3"/>"#
);
glyph!(
    GAUGE,
    r#"<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>"#
);
glyph!(
    INFO,
    r#"<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>"#
);
glyph!(
    BOOK,
    r#"<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20"/>"#
);
glyph!(
    WRAP_TEXT,
    r#"<path d="M3 6h18"/><path d="M3 12h15a3 3 0 1 1 0 6h-4"/><path d="m16 16-2 2 2 2"/><path d="M3 18h7"/>"#
);

/// Every name `glyph` knows, for tests.
#[cfg(test)]
pub(super) const NAMES: &[&str] = &[
    "plus",
    "reload",
    "minimize",
    "paperclip",
    "back",
    "monitor",
    "server",
    "sidebar",
    "compose",
    "edit",
    "pencil",
    "send",
    "arrow-up",
    "list-plus",
    "corner-up-right",
    "play",
    "thread",
    "cloud",
    "panel",
    "issue",
    "bolt",
    "dot",
    "theme",
    "settings",
    "filter",
    "folder-plus",
    "sun",
    "moon",
    "crescent",
    "chip",
    "alert",
    "pin",
    "pin-on",
    "trash",
    "calendar",
    "clock",
    "history",
    "agents",
    "arrow-up-circle",
    "lock-open",
    "corner-down-left",
    "external-link",
    "pull-request",
    "refresh",
    "more",
    "list",
    "branch",
    "stop",
    "grid",
    "archive",
    "folder",
    "search",
    "chevron",
    "chevron-down",
    "chevron-up",
    "open",
    "chevron-right",
    "chevron-left",
    "lock",
    "close",
    "volume",
    "volume-muted",
    "file",
    "terminal",
    "expand",
    "image",
    "eye",
    "brain",
    "wrench",
    "crew",
    "user",
    "copy",
    "unresolved",
    "star",
    "star-off",
    "workflow",
    "swap",
    "diff",
    "check",
    "circle-check",
    "alarm-clock",
    "shield-question",
    "message-question",
    "user-round",
    "folder-git2",
    "shield",
    "reply",
    "fork",
    "gauge",
    "info",
    "book",
    "wrap-text",
];

/// The glyph for an icon name. Unknown names draw a check.
pub(super) fn glyph(kind: &str) -> &'static Icon {
    match kind {
        "plus" => &PLUS,
        "reload" => &RELOAD,
        "minimize" => &MINIMIZE,
        "paperclip" => &PAPERCLIP,
        "back" | "chevron-left" => &CHEVRON_LEFT,
        "monitor" => &MONITOR,
        "server" => &SERVER,
        "sidebar" => &SIDEBAR,
        "compose" => &COMPOSE,
        "edit" | "pencil" => &PENCIL,
        "send" | "arrow-up" => &SEND,
        "list-plus" => &LIST_PLUS,
        "corner-up-right" => &CORNER_UP_RIGHT,
        "play" => &PLAY,
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
        "crescent" => &CRESCENT,
        "chip" => &CHIP,
        "alert" => &ALERT,
        "pin" => &PIN,
        "pin-on" => &PIN_ON,
        "trash" => &TRASH,
        "calendar" => &CALENDAR,
        "clock" => &CLOCK,
        "history" => &HISTORY,
        "agents" | "bot" => &AGENTS,
        "arrow-up-circle" => &ARROW_UP_CIRCLE,
        "lock-open" => &LOCK_OPEN,
        "corner-down-left" => &CORNER_DOWN_LEFT,
        "external-link" => &EXTERNAL_LINK,
        "pull-request" => &PULL_REQUEST,
        "refresh" => &REFRESH,
        "more" => &MORE,
        "list" => &LIST,
        "branch" => &BRANCH,
        "stop" => &STOP,
        "grid" => &GRID,
        "archive" => &ARCHIVE,
        "folder" => &FOLDER,
        "search" => &SEARCH,
        "chevron" | "chevron-down" => &CHEVRON_DOWN,
        "chevron-up" => &CHEVRON_UP,
        "open" | "chevron-right" => &CHEVRON_RIGHT,
        "lock" => &LOCK,
        "close" => &CLOSE,
        "volume" => &VOLUME,
        "volume-muted" => &VOLUME_MUTED,
        "file" => &FILE,
        "terminal" => &TERMINAL,
        "expand" => &EXPAND,
        "image" => &IMAGE,
        "eye" => &EYE,
        "brain" => &LIGHTBULB,
        "wrench" => &WRENCH,
        "crew" => &CREW,
        "user" => &USER,
        "copy" => &COPY,
        "unresolved" => &UNRESOLVED,
        "star" => &STAR,
        "star-off" => &STAR_OFF,
        "workflow" => &WORKFLOW,
        "swap" => &SWAP,
        "diff" => &DIFF,
        "circle-check" => &CIRCLE_CHECK,
        "alarm-clock" => &ALARM_CLOCK,
        "shield-question" => &SHIELD_QUESTION,
        "message-question" => &MESSAGE_QUESTION,
        "user-round" => &USER_ROUND,
        "folder-git2" => &FOLDER_GIT2,
        "shield" => &SHIELD,
        "reply" => &REPLY,
        "fork" => &FORK,
        "gauge" => &GAUGE,
        "info" => &INFO,
        "book" => &BOOK,
        "wrap-text" => &WRAP_TEXT,
        _ => &CHECK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_has_its_own_stroked_lucide_glyph() {
        for name in NAMES {
            let icon = glyph(name);
            assert_eq!(icon.view, 24.0, "{name}");
            assert!(
                icon.body
                    .starts_with(r#"<g fill="none" stroke="currentColor""#),
                "{name} is not a stroked glyph"
            );
            assert!(icon.body.ends_with("</g>"), "{name}");
            assert!(
                icon.body.contains("<path")
                    || icon.body.contains("<rect")
                    || icon.body.contains("<circle"),
                "{name} draws nothing"
            );
            // Only "check" may fall back to the check mark.
            if *name != "check" {
                assert!(
                    !std::ptr::eq(icon, &CHECK),
                    "{name} falls through to the check"
                );
            }
        }
    }

    #[test]
    fn every_glyph_parses_as_svg() {
        for name in NAMES {
            let icon = glyph(name);
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">{}</svg>"#,
                icon.body
            );
            let mut depth = 0i32;
            for (i, _) in svg.match_indices('<') {
                let rest = &svg[i..];
                let end = rest.find('>').expect("closed tag");
                let tag = &rest[..=end];
                if tag.starts_with("</") {
                    depth -= 1;
                } else if !tag.ends_with("/>") {
                    depth += 1;
                }
            }
            assert_eq!(depth, 0, "{name} has unbalanced tags");
        }
    }
}
