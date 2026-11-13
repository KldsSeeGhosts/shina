//! Pixel icons for the Pixel Voyage skin. Most are Pixelarticons by Gerrit Halfmann (MIT, see
//! `assets/icons-LICENSE.txt`), 24 x 24 grid, drawn from 2 px squares. The sidebar, panel and
//! dot glyphs are drawn here on the same grid.

use rune::Icon;

macro_rules! glyph {
    ($name:ident,$body:expr) => {
        pub(super) static $name: Icon = Icon {
            name: stringify!($name),
            view: 24.0,
            body: concat!(r#"<g fill="currentColor">"#, $body, "</g>"),
        };
    };
}

glyph!(PLUS, r#"<path d="M13 11h7v2h-7v7h-2v-7H4v-2h7V4h2v7Z"/>"#);
// Pixelarticons reload.svg, MIT, covered by assets/icons-LICENSE.txt.
glyph!(
    RELOAD,
    r#"<path d="M16 4h2v6h-2zm-2-2h2v2h-2zm0 2h2v8h-2zM4 8H2v5h2z"/><path d="M4 6h16v2H4zm4 14H6v-6h2zm2 2H8v-2h2zm0-2H8v-8h2zm10-4h2v-5h-2z"/><path d="M20 18H4v-2h16z"/>"#
);
// Chevrons are 2 unit blocks stepping 2 units, as heavy as the other strokes.
glyph!(
    CHEVRON_RIGHT,
    r#"<path d="M8 4h2v4h-2zM10 6h2v4h-2zM12 8h2v4h-2zM14 10h2v4h-2zM12 12h2v4h-2zM10 14h2v4h-2zM8 16h2v4h-2z"/>"#
);
glyph!(
    CHEVRON_LEFT,
    r#"<path d="M14 4h2v4h-2zM12 6h2v4h-2zM10 8h2v4h-2zM8 10h2v4h-2zM10 12h2v4h-2zM12 14h2v4h-2zM14 16h2v4h-2z"/>"#
);
glyph!(
    CHEVRON_DOWN,
    r#"<path d="M4 8h4v2h-4zM16 8h4v2h-4zM6 10h4v2h-4zM14 10h4v2h-4zM8 12h4v2h-4zM12 12h4v2h-4zM10 14h4v2h-4z"/>"#
);
glyph!(
    CHEVRON_UP,
    r#"<path d="M4 14h4v2H4zM16 14h4v2h-4zM6 12h4v2H6zM14 12h4v2h-4zM8 10h4v2H8zM12 10h4v2h-4zM10 8h4v2h-4z"/>"#
);
glyph!(
    CLOSE,
    r#"<path d="M7 19H5V17H7V19ZM19 19H17V17H19V19ZM9 15V17H7V15H9ZM17 17H15V15H17V17ZM11 15H9V13H11V15ZM15 15H13V13H15V15ZM13 13H11V11H13V13ZM11 11H9V9H11V11ZM15 11H13V9H15V11ZM9 9H7V7H9V9ZM17 9H15V7H17V9ZM7 7H5V5H7V7ZM19 7H17V5H19V7Z"/>"#
);
// The check is 4 unit blocks stepping 2 units, as heavy as the chevrons.
glyph!(
    CHECK,
    r#"<path d="M4 11h4v2h-4zM6 13h4v2h-4zM8 15h4v2h-4zM10 13h4v2h-4zM12 11h4v2h-4zM14 9h4v2h-4zM16 7h4v2h-4z"/>"#
);
glyph!(
    SEARCH,
    r#"<path d="M22 22h-2v-2h2v2Zm-2-2h-2v-2h2v2Zm-6-2H6v-2h8v2Zm4 0h-2v-2h2v2ZM6 16H4v-2h2v2Zm10 0h-2v-2h2v2ZM4 14H2V6h2v8Zm14 0h-2V6h2v8ZM6 6H4V4h2v2Zm10 0h-2V4h2v2Zm-2-2H6V2h8v2Z"/>"#
);
glyph!(
    FOLDER,
    r#"<path d="M4 4h6v2H4zm0 14h16v2H4zM20 8h2v10h-2zM2 6h2v12H2zm8 0h10v2H10z"/>"#
);
glyph!(
    FOLDER_PLUS,
    r#"<path d="M4 4h6v2H4zm0 14h10v2H4zM20 8h2v6h-2zM2 6h2v12H2zm8 0h10v2H10zm12 12v2h-6v-2z"/><path d="M18 16h2v6h-2z"/>"#
);
glyph!(
    COMPOSE,
    r#"<path d="M5 3h6v2H5zM3 5h2v14H3zm16 8h2v6h-2zM5 19h14v2H5zm3-9h2v6H8zm2 4h4v2h-4zm0-6h2v2h-2zm2-2h2v2h-2zm2-2h2v2h-2zm2-2h2v2h-2zm2 2h2v2h-2zm2 2h2v2h-2zm-2 2h2v2h-2zm-2 2h2v2h-2zm-2 2h2v2h-2zm-4 0h2v2h-2z"/>"#
);
glyph!(
    PENCIL,
    r#"<path d="M4 16H6V18H8V20H10V22H2V14H4V16ZM12 20H10V18H12V20ZM14 18H12V16H14V18ZM10 16H8V14H10V16ZM16 16H14V14H16V16ZM6 14H4V12H6V14ZM12 14H10V12H12V14ZM18 14H16V12H18V14ZM8 12H6V10H8V12ZM14 12H12V10H14V12ZM20 12H18V10H20V12ZM10 10H8V8H10V10ZM18 10H16V8H18V10ZM22 10H20V8H22V10ZM12 8H10V6H12V8ZM16 8H14V6H16V8ZM20 8H18V6H20V8ZM14 6H12V4H14V6ZM18 6H16V4H18V6ZM16 4H14V2H16V4Z"/>"#
);
glyph!(
    SEND,
    r#"<path d="M11 20h2V4h-2zm2-12h2V6h-2zm2 2h2V8h-2zm2 2h2v-2h-2zm-6-4H9V6h2z"/><path d="M15 10H7V8h8zm2 2H5v-2h12z"/>"#
);
// Pixelarticons play.svg, used for KSG Code's Resume thread action.
glyph!(
    PLAY,
    r#"<path d="M15 11h-2V9h2zm0 4h-2v-2h2zm-2 2h-2v-2h2zm0-8h-2V7h2zm-2-2H9V5h2zM9 21H7V3h2zm6-8h2v-2h-2zm-6 4h2v2H9z"/>"#
);
glyph!(
    BUBBLE,
    r#"<path d="M20 2H4v2h16zm0 14H6v2h14zm2-12h-2v12h2zM4 4H2v18h2zm2 14H4v2h2zm0-6h4v2H6zm0-4h8v2H6z"/>"#
);
glyph!(
    CLOUD,
    r#"<path d="M22 10h-4v2h4v-2Zm2 2h-2v6h2v-6Zm-2 6H2v2h20v-2ZM2 12H0v6h2v-6Zm2-2H2v2h2v-2Zm4-2H4v2h4V8Zm8-4h-6v2h6V4Zm-6 2H8v2h2V6Zm0 4H8v2h2v-2Zm8-4h-2v2h2V6Z"/><path d="M20 8h-2v4h2V8Zm-2 4h-2v2h2v-2Z"/>"#
);
glyph!(
    THEME,
    r#"<path d="M4 2h16v2H4zm0 18h16v2H4zM2 4h2v16H2zm18 0h2v16h-2zM4 18h2v2H4zm0-2h4v2H4zm0-2h6v2H4zm0-2h8v2H4zm0-2h10v2H4zm0-2h12v2H4zm0-2h14v2H4zm0-2h16v2H4z"/>"#
);
// A gear on 2 unit cells: a one cell ring with an open centre, four wide teeth and four corner
// teeth. It carries about as much ink as ARCHIVE, where the Pixelarticons cog fills in to a grey
// knot at 18 pt.
glyph!(
    SETTINGS,
    r#"<path d="M10 0h4v4h-4zM4 4h2v2h-2zM8 4h8v2h-8zM18 4h2v2h-2zM6 6h2v2h-2zM16 6h2v2h-2zM4 8h2v2h-2zM18 8h2v2h-2zM0 10h6v4h-6zM18 10h6v4h-6zM4 14h2v2h-2zM18 14h2v2h-2zM6 16h2v2h-2zM16 16h2v2h-2zM4 18h2v2h-2zM8 18h8v2h-8zM18 18h2v2h-2zM10 20h4v4h-4z"/>"#
);
glyph!(
    FILTER,
    r#"<path d="M8 14H7v6H5v-6H2v-2h6v2Zm5 6h-2V10h2v10Zm9-2h-3v2h-2v-2h-1v-2h6v2Zm-3-4h-2V4h2v10ZM7 10H5V4h2v6Zm6-4h2v2H9V6h2V4h2v2Z"/>"#
);
glyph!(
    SUN,
    r#"<path d="M13 22h-2v-3h2v3Zm-6-3H5v-2h2v2Zm12 0h-2v-2h2v2Zm-4-2H9v-2h6v2Zm-6-2H7V9h2v6Zm8 0h-2V9h2v6ZM5 13H2v-2h3v2Zm17 0h-3v-2h3v2Zm-7-4H9V7h6v2ZM7 7H5V5h2v2Zm12 0h-2V5h2v2Zm-6-2h-2V2h2v3Z"/>"#
);
glyph!(
    MOON,
    r#"<path d="M18 22H8v-2h10v2ZM8 20H6v-2h2v2Zm12 0h-2v-2h2v2ZM6 18H4v-2h2v2Zm16 0h-2v-4h-2v-2h2v-2h2v8ZM4 16H2V6h2v10Zm14 0h-6v-2h6v2Zm-6-2h-2v-2h2v2Zm-2-2H8V6h2v6ZM6 6H4V4h2v2Zm8-2h-2v2h-2V4H6V2h8v2Z"/>"#
);
glyph!(
    ALERT,
    r#"<path d="M4 2h16v2H4zm0 18h16v2H4zM20 4h2v16h-2zM2 4h2v16H2zm9 2h2v8h-2zm0 10h2v2h-2z"/>"#
);
glyph!(
    CLOCK,
    r#"<path d="M6 2h12v2h-12zM4 4h4v2h-4zM16 4h4v2h-4zM2 6h2v12h-2zM10 6h2v4h-2zM20 6h2v12h-2zM10 10h6v2h-6zM4 18h4v2h-4zM16 18h4v2h-4zM6 20h12v2h-12z"/>"#
);
// Three solid 4 unit squares.
glyph!(MORE, r#"<path d="M2 10h4v4H2zm8 0h4v4h-4zm8 0h4v4h-4z"/>"#);
glyph!(
    LIST,
    r#"<path d="M10 5h12v2H10zm0 4h8v2h-8zm0 4h12v2H10zm0 4h8v2h-8zm-4-6H4V9h2v2ZM4 9H2V7h2v2Zm4 0H6V7h2v2ZM6 7H4V5h2v2Zm-2 6h2v2H4zm0 4h2v2H4zm-2 0v-2h2v2zm4 0v-2h2v2z"/>"#
);
glyph!(
    BRANCH,
    r#"<path d="M4 14h4v2H4zm0 6h4v2H4zm-2-4h2v4H2zm6 0h2v4H8zm8-14h4v2h-4zm0 6h4v2h-4zm-2-4h2v4h-2zm6 0h2v4h-2zm-8 13h5v2h-5zm5-5h2v5h-2zM5 2h2v10H5z"/>"#
);
glyph!(STOP, r#"<path d="M20 20H4V4H20V20ZM10 14H14V10H10V14Z"/>"#);
glyph!(
    GRID,
    r#"<path d="M4 2h16v2H4zM2 4h2v16H2zm2 7h16v2H4zm16-7h2v16h-2z"/><path d="M11 4h2v18h-2z"/><path d="M4 20h16v2H4z"/>"#
);
glyph!(
    ARCHIVE,
    r#"<path d="M3 2h18v2H3zm0 5h18v2H3zM1 4h2v3H1zm20 0h2v3h-2zm-2 5h2v11h-2zM3 9h2v11H3zm2 11h14v2H5zm4-9h6v2H9z"/>"#
);
glyph!(
    LOCK,
    r#"<path d="M5 8h14v2H5zm0 12h14v2H5zM3 10h2v10H3zm16 0h2v10h-2zM7 4h2v4H7zm2-2h6v2H9zm6 2h2v4h-2z"/>"#
);
glyph!(
    FILE,
    r#"<path d="M6 4H4v16h2zm10-2H6v2h10zm4 4h-2v14h2zm-2 14H6v2h12zM16 4h2v2h-2zm-4 0h2v6h-2z"/><path d="M12 8h6v2h-6zm-4 8h8v2H8zm0-4h8v2H8zm0-4h2v2H8z"/>"#
);
glyph!(
    TERMINAL,
    r#"<path d="M4 2h16v2H4zm0 18h16v2H4zM2 4h2v16H2zm18 0h2v16h-2zM6 16h2v2H6zm2-2h2v2H8zm-2-2h2v2H6z"/>"#
);
glyph!(
    EXPAND,
    r#"<path d="M4 13h16v-2H4zm7-8h2V3h-2zM9 7h4V5H9zm4 0h2V5h-2zm2 2h2V7h-2zM7 9h8V7H7zm4 10h2v2h-2zm-2-2h4v2H9zm4 0h2v2h-2zm2-2h2v2h-2zm-8 0h8v2H7z"/>"#
);
glyph!(
    MINIMIZE,
    r#"<path d="M7 19h2v-2h2v-2h2v2h2v2h2v2H7v-2Zm13-6H4v-2h16v2Zm-3-8h-2v2h-2v2h-2V7H9V5H7V3h10v2Z"/>"#
);
glyph!(
    IMAGE,
    r#"<path d="M4 2h16v2H4zm0 18h16v2H4zM2 4h2v16H2zm18 0h2v16h-2zm-4 8h2v2h-2zm-2 2h2v2h-2zm4 0h2v2h-2zm-8 0h2v2h-2zm2 2h2v2h-2zm2 2h2v2h-2z"/><path d="M20 16h2v2h-2zM8 16h2v2H8zm-2 2h2v2H6zM8 6h2v2H8zM6 8h2v2H6zm2 2h2v2H8zm2-2h2v2h-2z"/>"#
);
glyph!(
    EYE,
    r#"<path d="M16 20H8v-2h8v2Zm-8-2H4v-2h4v2Zm12 0h-4v-2h4v2ZM4 16H2v-2h2v2Zm10-6h-2v2h2v-2h2v4h-2v2h-4v-2H8v-4h2V8h4v2Zm8 6h-2v-2h2v2ZM2 14H0v-4h2v4Zm22 0h-2v-4h2v4ZM4 10H2V8h2v2Zm18 0h-2V8h2v2ZM8 8H4V6h4v2Zm12 0h-4V6h4v2Zm-4-2H8V4h8v2Z"/>"#
);
glyph!(
    CREW,
    r#"<path d="M5 2h6v2H5zm10 0h4v2h-4zM5 10h6v2H5zm10 0h4v2h-4zm4-6h2v6h-2zm-8 0h2v6h-2zM3 4h2v6H3zM0 18h2v4H0zm14 0h2v4h-2zm8 0h2v4h-2zM4 14h8v2H4zm12 0h4v2h-4zM2 16h2v2H2zm10 0h2v2h-2zm8 0h2v2h-2z"/>"#
);
glyph!(
    COPY,
    r#"<path d="M8 6h12v2H8zM4 2h12v2H4zm2 6h2v12H6zM2 4h2v12H2zm6 16h12v2H8zM20 8h2v12h-2zm-4-4h2v2h-2zM4 16h2v2H4z"/>"#
);
glyph!(
    WRENCH,
    r#"<path d="M9 22H7v-2h2v2Zm12-6h2v6h-6v-6h2v-6h2v6Zm-2 2v2h2v-2h-2ZM7 20H5v-8h2v8Zm4 0H9v-8h2v8Zm-6-8H3v-2h2v2Zm8 0h-2v-2h2v2ZM3 10H1V4h2v6Zm12 0h-2V4h2v6Zm4 0h-2V4h2v6Zm4 0h-2V4h2v6ZM7 6h2V2h4v2h-2v4H5V4H3V2h4v4Zm14-2h-2V2h2v2Z"/>"#
);
// An upright paperclip on 2 unit cells: one wire, three nested strands. A tilted clip turns to
// a staircase of stripes at 18 pt, so this one stays vertical.
glyph!(
    PAPERCLIP,
    r#"<path d="M8 0h6v2h-6zM6 2h2v2h-2zM14 2h2v2h-2zM6 4h2v2h-2zM14 4h2v2h-2zM6 6h2v2h-2zM10 6h2v2h-2zM14 6h2v2h-2zM6 8h2v2h-2zM10 8h2v2h-2zM14 8h2v2h-2zM6 10h2v2h-2zM10 10h2v2h-2zM14 10h2v2h-2zM6 12h2v2h-2zM10 12h2v2h-2zM14 12h2v2h-2zM6 14h2v2h-2zM10 14h2v2h-2zM14 14h2v2h-2zM6 16h2v2h-2zM10 16h2v2h-2zM14 16h2v2h-2zM6 18h2v2h-2zM10 18h6v2h-6zM6 20h2v2h-2zM8 22h6v2h-6z"/>"#
);
glyph!(
    MONITOR,
    r#"<path d="M4 2h16v2H4zm0 14h16v2H4zM2 4h2v12H2zm18 0h2v12h-2zm-9 14h2v2h-2zm-3 2h8v2H8z"/>"#
);
// A pushpin: head, body, flange and needle. Pinned is the solid body.
glyph!(
    PIN,
    r#"<path d="M8 3h8v2H8zM9 5h2v6H9zM13 5h2v6h-2zM6 11h12v2H6zM11 13h2v8h-2z"/>"#
);
glyph!(
    LIGHTBULB,
    r#"<path d="M2 2h2v2h-2zM8 2h8v2h-8zM20 2h2v2h-2zM8 4h2v2h-2zM14 4h2v2h-2zM6 6h2v8h-2zM16 6h2v8h-2zM0 8h2v2h-2zM22 8h2v2h-2zM8 14h2v2h-2zM14 14h2v2h-2zM8 16h8v4h-8zM10 20h4v2h-4z"/>"#
);
glyph!(
    BOLT,
    r#"<path d="M4 13h8v6h2v2h-2v2h-2v-8H2v-4h2v2Zm12 6h-2v-2h2v2Zm2-2h-2v-2h2v2Zm2-2h-2v-2h2v2Zm-6-6h8v4h-2v-2h-8V5h-2V3h2V1h2v8Zm-8 2H4V9h2v2Zm2-2H6V7h2v2Zm2-2H8V5h2v2Z"/>"#
);
glyph!(
    ISSUE,
    r#"<path d="M5 1h14v2H5zM3 3h2v2H3zm0 16h2v2H3zm16 0h2v2h-2zm0-16h2v2h-2zm2 2h2v14h-2zM5 21h14v2H5zM1 5h2v14H1zm8 0h6v2H9zM5 9h2v6H5zm4 8h6v2H9zm8-8h2v6h-2zm-6 0h2v2h-2zM7 7h2v2H7zm0 8h2v2H7zm8 0h2v2h-2zm0-8h2v2h-2zm-6 4h2v2H9zm2 2h2v2h-2zm2-2h2v2h-2z"/>"#
);
// Drawn on the same 24 grid from 2 px squares; Pixelarticons has no sidebar layout glyph.
glyph!(
    SIDEBAR,
    r#"<path d="M3 4h18v2H3zM3 18h18v2H3zM3 6h2v12H3zM19 6h2v12h-2zM9 6h2v12H9z"/>"#
);
glyph!(
    PANEL,
    r#"<path d="M3 4h18v2H3zM3 18h18v2H3zM3 6h2v12H3zM19 6h2v12h-2zM13 6h2v12h-2z"/>"#
);
glyph!(
    TRASH,
    r#"<path d="M18 22H6V20H18V22ZM9 6H15V4H17V6H22V8H20V20H18V8H6V20H4V8H2V6H7V4H9V6ZM15 4H9V2H15V4Z"/>"#
);
glyph!(
    CALENDAR,
    r#"<path d="M5 4h14v2H5zm0 16h14v2H5zM3 10h2v10H3zm0-4h2v2H3zm16 0h2v2h-2zm0 4h2v10h-2zM3 8h18v2H3zm12-6h2v2h-2zM7 2h2v2H7z"/>"#
);
// A pinned pin is the same pin with a filled head.
glyph!(
    PIN_ON,
    r#"<path d="M8 3h8v2H8zM9 5h6v6H9zM6 11h12v2H6zM11 13h2v8h-2z"/>"#
);
// Every shape below sits on whole 2 unit cells, so each edge lands on a device pixel at 2x.
// A crescent moon for "away", solid and open to the upper right.
glyph!(
    CRESCENT,
    r#"<path d="M8 2h8v2h-8zM4 4h8v2h-8zM4 6h6v2h-6zM2 8h8v6h-8zM18 12h2v2h-2zM2 14h10v2h-10zM16 14h4v2h-4zM4 16h18v2h-18zM6 18h14v2h-14z"/>"#
);
// A chip for a model with no vendor: a one cell outline, a 2 cell die and eight pins.
glyph!(
    CHIP,
    r#"<path d="M8 2h2v2h-2zM14 2h2v2h-2zM4 4h16v2h-16zM4 6h2v2h-2zM18 6h2v2h-2zM2 8h4v2h-4zM18 8h4v2h-4zM4 10h2v2h-2zM10 10h4v4h-4zM18 10h2v2h-2zM2 12h4v2h-4zM18 12h4v2h-4zM4 14h2v2h-2zM18 14h2v2h-2zM4 16h16v2h-16zM8 18h2v2h-2zM14 18h2v2h-2z"/>"#
);
glyph!(DOT, r#"<path d="M8 8h8v8H8z"/>"#);
// Dashed circle/ring on whole 2-unit cells for unresolved composer context chips.
glyph!(
    UNRESOLVED,
    r#"<path d="M8 2h4v2H8zm6 0h4v2h-4zM4 4h2v2H4zm14 0h2v2h-2zM2 8h2v4H2zm18 0h2v4h-2zM2 14h2v4H2zm18 0h2v4h-2zM4 18h2v2H4zm14 0h2v2h-2zM8 20h4v2H8zm6 0h4v2h-4z"/>"#
);
glyph!(
    WORKFLOW,
    r#"<path d="M2 2h8v8H2zM14 2h8v8h-8zM14 14h8v8h-8zM4 4v4h4V4zM16 4v4h4V4zM16 16v4h4v-4zM10 5h4v2h-4zM5 10h2v8H5zM7 17h7v2H7z"/>"#
);
glyph!(
    STAR,
    r#"<path d="M10 2h4v4h-4zM8 6h8v2H8zM2 8h20v2H2zM4 10h16v2H4zM6 12h12v4H6zM4 16h6v4H4zM14 16h6v4h-6zM2 20h4v2H2zM18 20h4v2h-4z"/>"#
);
glyph!(
    STAR_OFF,
    r#"<path d="M10 2h4v2h-4zM8 4h2v4H8zM14 4h2v4h-2zM2 8h6v2H2zM16 8h6v2h-6zM4 10h2v2H4zM18 10h2v2h-2zM6 12h2v4H6zM16 12h2v4h-2zM10 14h4v2h-4zM4 16h4v4H4zM16 16h4v4h-4zM2 20h4v2H2zM18 20h4v2h-4z"/>"#
);
// Two opposing arrows for a provider switch, KSG Code's ArrowRightLeft.
glyph!(
    SWAP,
    r#"<path d="M4 8h12v2H4zM16 6h2v2h-2zM18 8h2v2h-2zM16 10h2v2h-2zM8 14h12v2H8zM6 12h2v2H6zM4 14h2v2H4zM6 16h2v2H6z"/>"#
);

/// The pixel glyph for an icon name. Unknown names draw a check.
pub(super) fn glyph(kind: &str) -> &'static Icon {
    match kind {
        "plus" => &PLUS,
        "reload" => &RELOAD,
        "minimize" => &MINIMIZE,
        "paperclip" => &PAPERCLIP,
        "back" => &CHEVRON_LEFT,
        "monitor" => &MONITOR,
        "sidebar" => &SIDEBAR,
        "compose" => &COMPOSE,
        "edit" => &PENCIL,
        "send" => &SEND,
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
        "more" => &MORE,
        "list" => &LIST,
        "branch" => &BRANCH,
        "stop" => &STOP,
        "grid" => &GRID,
        "archive" => &ARCHIVE,
        "folder" => &FOLDER,
        "search" => &SEARCH,
        "chevron" => &CHEVRON_DOWN,
        "chevron-up" => &CHEVRON_UP,
        "open" => &CHEVRON_RIGHT,
        "lock" => &LOCK,
        "close" => &CLOSE,
        "file" => &FILE,
        "terminal" => &TERMINAL,
        "expand" => &EXPAND,
        "image" => &IMAGE,
        "eye" => &EYE,
        "brain" => &LIGHTBULB,
        "wrench" => &WRENCH,
        "crew" => &CREW,
        "copy" => &COPY,
        "unresolved" => &UNRESOLVED,
        "star" => &STAR,
        "star-off" => &STAR_OFF,
        "workflow" => &WORKFLOW,
        "swap" => &SWAP,
        _ => &CHECK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every number in `body`, for a path made only of `M`, `h`, `v` and `z` style commands.
    fn numbers(body: &str) -> Vec<f32> {
        let mut found = Vec::new();
        let mut current = String::new();
        for c in body.chars().chain(std::iter::once(' ')) {
            if c.is_ascii_digit() || c == '.' || (c == '-' && current.is_empty()) {
                current.push(c);
            } else {
                if let Ok(n) = current.parse() {
                    found.push(n);
                }
                current.clear();
                if c == '-' {
                    current.push(c);
                }
            }
        }
        found
    }

    #[test]
    fn strict_glyphs_are_built_from_whole_two_unit_cells() {
        // At 18 pt on a 2x display one unit is 1.5 device pixels, so an odd coordinate lands
        // on half a pixel and blurs. Every coordinate and length in these glyphs is even.
        let strict: [(&str, &Icon); 9] = [
            ("clock", &CLOCK),
            ("lightbulb", &LIGHTBULB),
            ("pencil", &PENCIL),
            ("paperclip", &PAPERCLIP),
            ("crescent", &CRESCENT),
            ("chip", &CHIP),
            ("settings", &SETTINGS),
            ("moon", &MOON),
            ("unresolved", &UNRESOLVED),
        ];
        for (name, glyph) in strict {
            let all = numbers(glyph.body);
            assert!(!all.is_empty(), "{name} has no path");
            // The wrapper's own `<g fill="currentColor">` carries no digits.
            for n in all {
                assert!(n % 2.0 == 0.0, "{name} has the odd coordinate {n}");
            }
        }
    }
}
