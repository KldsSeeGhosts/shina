# Pixel Voyage: Shina's visual redesign

Status: implemented, 2026-11-11, with the revisions listed at the end. Owner of design
decisions: the orchestrator. Implementers follow this document and raise conflicts instead of
improvising.

## Why

The app icon (a 32x32 pixel sailboat under a night sky, `src/brand.rs`) and the captain
portraits (`assets/crew/*`) are pixel art with flat, saturated colour and hard edges. The app
around them is a warm clone of KSG Code: soft radii, hairline Lucide icons, one teal accent.
The art and the app do not look like the same product. This redesign makes the whole window
look like it was drawn by the same hand as the icon.

KSG Code remains the source for structure and behaviour (what goes where, what a click does,
the lane widths). This document replaces KSG's visual skin. That is a deliberate, user-requested
departure from the "port, do not invent" rule in `AGENTS.md`; update `AGENTS.md` to say so.

## The rule that keeps it tasteful

Pixel art is for chrome and identity. Reading text is not pixel art. The transcript is a reading
area and does not shout.

- Pixel treatment: icons, section labels, counters, timers, status chips, keycaps, frames,
  dividers, scene art, the wordmark.
- Never pixel: message prose, thread titles, the composer text, code blocks, dialog body copy.
  These stay Inter, Source Serif 4 (headlines only) and the system mono.

## 1. Colour

All colours come from the icon's two scenes. Coral is the icon's hull and flag and is the one
colour the app has never used. It becomes the primary accent. Teal moves to "working".

Add fields to `Palette` as needed and remove the ones that become unused.

| Role | Night (dark) | Day (light) | Source in the icon |
| --- | --- | --- | --- |
| `bg` (window, sidebar) | `#101727` | `#d5e8ef` | night sky bottom / day sky, between the icon's `#b9e0ee` and white |
| `panel` (chat pane) | `#162036` | `#f8f4ea` | sky / sail |
| `card` (composer, menus) | `#1d2a45` | `#fffdf7` | sky top |
| `ink` | `#f4efe3` | `#1f2538` | sail / night sky |
| `muted` | `#b8c7e0` | `#565d71` | darkened from `#5d6478`, which was 4.25:1 on a hovered sidebar row |
| `line` (hairlines) | `#f4efe3` at 12% | `#1f2538` at 14% | |
| `frame` (2 px pixel borders) | `#52688e` | `#1f2538` | `#0a0f1c` vanished on the night surfaces, and `#3a4a6c` blurred into them |
| `portrait_frame` | `#3a4a6c` | `#1f2538` | portraits keep the darker night outline so a face reads apart from buttons, tabs and the composer |
| `faint` (tool details, paths, machine names) | `#98aac5` | `#595f73` | opaque, 4.5:1 or better on every surface text sits on; never `muted` with an alpha |
| `coral` (primary accent) | `#e07a54` | `#d2643c` | hull, flag |
| `coral_bg` | `#e07a54` at 16% | `#d2643c` at 12% | |
| `on_coral` | `#1a0f0a` | `#1a0f0a` | cream on `#d2643c` is 3.7:1; near-black is 5.2:1 |
| `on_brass` | `#1a0f0a` | `#fffdf7` | ink on brass fills such as count badges |
| `brass` (needs you) | `#d3ba6b` | `#8a6a2f` | mast |
| `sea` (working, links, updates, done) | `#7ab4d2` | `#1f6d62` | jib / water; lightened and darkened so `sea` text reaches 4.5:1 on `sea_bg`, `bg` and the cards |
| `sea_bg` | `#24406a` | `#d6ebe4` | water |
| `danger` | `#f0a39c` | `#a33e38` | |
| `message_bg` (user bubble) | `#24406a` | `#e0d6ba` | water / sail shade |
| `code_bg` | `#0c1220` | `#ebe6d3` | |

Usage:

- Coral: the send button, the active tab marker, the selected sidebar row marker, primary
  buttons, focus rings, the unread dot, links on hover. One coral element per region at rest.
- Brass: anything waiting on the user.
- Sea: running work, timers while running, "Done" checks, text links.
- Check contrast: body text at least 7:1 on its surface, muted and faint text at least 4.5:1.
  A test covers every text and surface pair, including hovered rows and the brass badge's
  hover and pressed fills.
- Pair washes with their own accent: `coral_bg` with `coral`, `sea_bg` with `sea`. Updates and
  "done" are sea. Primary actions and selection are coral. Where a coral tile carries a letter,
  it is a solid `coral` fill with `on_coral` text, because coral text on its own wash is 3.2:1.

## 2. Typography

Three faces, each with one job.

- **Departure Mono** (pixel, SIL OFL, bundle `DepartureMono-Regular.otf` or `.ttf` with its
  licence under `assets/fonts/`): section labels, counts, timers, status words, badges, keycaps,
  tab workspace prefix, the "Shina" wordmark, tool-row verbs. It is designed for 11 px and its
  multiples. Use it only at 11 px (or 22 px for the wordmark-scale uses) so its pixels land on
  the grid. Uppercase is only for structure: see "Case" below.
- **Source Serif 4**: the greeting and captain hero headline, dialog titles, the captain's name
  in the hero and the editor. Weight 500 to 600, tight tracking.
- **Inter**: everything people read or type.

Add a `PIXEL` family constant next to `SANS_FAMILY` and a `pixel(size)` text style helper.

**Case.** Uppercase Departure Mono (with `tracking(0.6)`) is only for sidebar section headers
(`CAPTAINS`, `DRAFTS`, `SNOOZED`, `SETTLED`), dialog form labels, and small badges such as `DEMO`
and a code block's language. Everything else in pixel type is sentence case: captain and thread
status words ("Ready", "Needs you", "Working", "Done", "Failed"), work-log rows ("Worked for 42s",
"Read 2 files", "Thought 18s"), the effort name beside the model, tab workspace prefixes, timers
and ages.

**No middle-dot joins.** A status word and its detail are two labels with an 8 px gap: the word
in pixel type coloured by status, the detail in Inter, muted. Accessibility labels use a comma.

## 3. Icons

Replace the hairline icon set in `fn icon` with **Pixelarticons** (MIT,
`github.com/halfmage/pixelarticons`, 24x24 grid, drawn from 2 px squares). Take the SVG path
data for each glyph and render it as filled geometry. Icons draw at 18 pt, not 16: each 2 unit square then lands on 3 device pixels at 2x and keeps
hard edges (16 pt lands on 2.67 and blurs). A few tiny uses may draw at 12 pt, which is 1 device
pixel per unit at 2x. Add the MIT
licence under `assets/` or `docs/` and credit it in the README's acknowledgements if one exists.

Pixelarticons has no pushpin, a thin chevron and a thin check, and its gear reads as a chip. The
settings icon is its cog. The pushpin, the chevrons and the check are drawn on the same grid from
2 unit blocks at the weight of the other strokes. The paperclip is its clip leaning 45 degrees.
The sidebar, panel and dot glyphs are drawn here too.

Keep every existing icon name working (`icon("search", ..)` and so on) so call sites and tests
do not change. Map each current name to the closest Pixelarticons glyph. Where there is no good
match, draw one on the same 24 grid from 2 px squares. Nautical substitutions that stay
obvious to a first-time user are welcome where the meaning is unmistakable in context (an
anchor for "Settled", a ship's wheel is NOT a replacement for the settings gear). When in
doubt, use the conventional glyph.

Provider and harness logos stay as they are.

## 4. Shape language

- Corners: 4 px on rows, buttons, inputs, tabs, chips. 6 px on cards, menus, dialogs and the
  composer. 0 on dividers and frames. Nothing is a pill or a circle except the context ring
  and avatars' status dots, which become 2x2-pixel-stepped squares (a small square, not a disc).
- Secondary buttons beside a primary one (Cancel next to Create) have the 2 px frame and no
  shadow, so the pair reads as a set.
- Pixel frame: a 2 px solid `frame` border with a hard 2 px offset shadow below-right in
  `frame` at 100% (night: the shadow is `#060a13`; day: `#1f2538` at 30%). No blur anywhere in
  the app. Used on: portraits, the horizon band and the hero portrait, the composer card, menus
  and popovers, dialogs, primary buttons, the send button.
- Pressed state for framed buttons: translate 1 to 2 px toward the shadow and remove the
  shadow, like a key going down. If Rune cannot translate on press, darken the fill instead.
- Dividers: a 1 px `line` rule, or the dotted variant (2 px on, 2 px off) between sidebar
  sections.
- Portraits: square with 4 px radius at most (2 px for sizes under 30), pixel frame (1 px under
  30 px, and on 16 px project tiles, so the face keeps its room). Sidebar captain portraits are
  40 px in a 44 px slot. The focal captain keeps the larger size. The editor's grid and the
  captain picker use 40 px too. The selected portrait in the editor's grid turns its own 2 px
  frame coral, with no ring or wash around it, so all twelve keep the same outer bounds.
- Pixel art is drawn crisp. Rune has no nearest-neighbour sampling, so `src/crisp.rs` scales
  each portrait, strip frame and the app mark itself, to the exact device-pixel size (points
  times the window scale times the app zoom) by a whole factor, and Rune draws that buffer one
  texel to one pixel. A target that is not a whole multiple of the art's cells takes the nearest
  whole factor and either crops the art's edge or pads it with the tile colour, whichever
  changes less (a 20 pt head crop at 2x crops, the 72 pt hero crops two cells a side). 40 pt
  for a 40 cell bust is exact at every integer scale. Buffers are cached by art and size, and
  an animation stage keeps one buffer per size in use.

## 5. Scene art

Procedural, built from the palette in `brand.rs` the way the mark is. No bitmap files unless
they are generated by a checked-in tool (as `tools/cursors.py` does).

- **Horizon band.** A pixel seascape strip: sky in 4 flat steps, a few stars and a crescent moon
  (night) or a sun and one cloud (day), a two-tone sea with crest pixels along the top edge.
  Pixel size 3 pt. The night sky runs `#2d3c5f`, `#26324d`, `#1d2840`, `#121a2b` (the icon's
  top and bottom with a brighter first step, so the top is lighter than the `#162036` panel).
  The moon is the icon's crescent at twice its size. The night sea is `#2e5488` and `#203d68`
  with `#8cc0d8` crests, bright enough to read as water against the panel. Crests are one or
  two cells wide with uneven gaps of three to seven cells, from a fixed seed, so the waterline
  is not a ruler. The day sun is two steps and no halo: a 5 by 5 gold disc with its corners cut
  around a 3 by 3 pale core. The scenery is memoised by width, theme and backdrop colour. It appears:
  1. On the new-thread screen, as a band 96 to 120 pt tall across the top of the chat pane's
     content lane (rounded 6 px, pixel frame), with the Shina sailboat mark sailing on the
     waterline at about one third from the left. The greeting sits below it. On a captain's
     home, the captain's portrait (72 px, pixel frame) overlaps the band's bottom edge instead
     of the boat, like a profile banner, and the band takes its sky colour from that portrait's
     background colour.
  2. At the foot of the sidebar: a 27 pt sea strip (water and crests only) that the footer
     icons sit above. It bleeds: flush to the window's left and bottom edges, as wide as the
     sidebar, square corners. This is the "waterline" of the app, and in the light theme the
     sidebar is the sky above it.
- Static. Drifting crests and twinkling stars were left out: the scenery is shapes in the view,
  so each step would re-run the view, and an idle window would wake every 800 ms.
- The band never sits behind text.

## 6. Regions

### Top bar

- Left: sidebar toggle, the mark at 24 px with a 1 px `frame` outline (the day mark's sky is the
  chrome's sky), "Shina" in Departure Mono 22 px. The attention
  counter is a framed brass badge after the wordmark with an 8 px gap, right-aligned to the
  sidebar's edge. The mark and the wordmark are 8 px apart; the toggle and the mark touch their
  boxes. The wordmark stays at 22 px in a docked sidebar of normal width. A narrow
  sidebar (216 px) with a badge sets it at 11 px; only a truly narrow one shows the mark alone.
- Tabs: 32 px tall, 4 px radius, left-packed and at most 260 px wide. Inactive: no fill, muted. Active: `card` fill, pixel frame
  (no shadow), and a 2 px coral bar along the bottom inside the frame. Workspace prefix in
  Departure Mono 11 px muted, then the title in Inter 13/520. No "/" separator; use an 8 px gap.
  The close button appears on hover and on the active tab. Tabs share the strip evenly (260 px
  at most, 120 px at least, titles truncate, the prefix drops below 170 px). Only when every tab
  is at 120 px does the strip scroll. It scrolls to keep the active tab whole whenever the
  selection, the tab list or the tab width changes, through a 1 pt marker keyed by that state.
- The first tab starts 12 px after the attention badge. In the 40 px top strip a 32 px tab leaves
  4 px above it and 4 px between its bottom frame and the chat pane's top edge.
- The new-tab plus is a framed 24 px square button.

### Sidebar

- Section labels (`CAPTAINS`, `DRAFTS`, `SNOOZED`, `SETTLED`): Departure Mono 11 px uppercase,
  muted, with the count right-aligned in the same face. A dotted divider above each section
  except the first, and none inside a section.
- Search: a 32 px field with a 1 px `line` border and 4 px radius, so it reads as a field.
  Focus: 2 px coral border.
- Captain rows: pixel-framed portrait, name in Inter 14/600, status line with the status word
  in Departure Mono 11 px, sentence case, coloured by status ("Ready" muted, "Working" sea,
  "Needs you" brass, "Done" sea, "Failed" danger), then the detail in Inter 12.5 muted after an
  8 px gap, with no dot between them. Badges are framed squares
  with Departure Mono digits.
- Selected row (captain or thread): `card`-tinted fill plus a 3 px coral bar on the left edge,
  full row height, square ends. Hover: fill only.
- Thread cards: keep the two-line anatomy. The project tile becomes a pixel-framed 16 px square
  with the initial in Departure Mono. The age and the status word use Departure Mono 11 px, sentence case. The workspace and the
  machine are two labels with a gap, not one joined string.
- Machine marks for remote rows stay, drawn with the new icon set. A row shows the machine as
  its icon and name, only when more than one machine is linked, and never whether it is
  reachable. The workspace keeps its floor (ten characters, less in the narrowest sidebar) and
  the machine name gives way first.
- Footer: settings and archive icons above the sea strip.

### Chat pane

- The pane keeps its inset card but loses the large radius: 6 px on its top corners, square at
  the bottom, with a 1 px `line` border. It runs to the window's bottom edge, as the sidebar's
  sea strip does, so there is no gutter under it.
- User message: `message_bg` bubble, 6 px radius, 2 px `frame` border on night, no shadow.
- Assistant header: 20 px pixel-framed portrait, "Captain Ember" with the name in Inter 14/600.
- Work log rows: the verb ("Read", "Edited", "Ran", "Thought", "Worked for") in Departure Mono
  11 px, sentence case, muted; the detail in Inter or mono as today. Pixel icons. Keep the disclosure
  chevron beside the label.
- Code blocks: `code_bg`, 6 px radius, 1 px `line` border, a header strip with the language in
  Departure Mono 11 px uppercase and the copy button. Diff lines: added lines on `sea_bg`,
  removed on `coral_bg`.
- Inline code: `code_bg` chip, 3 px radius.
- Status slot and "Demo" badge: framed chips, Departure Mono.

### Composer

- `card` fill, 6 px radius, pixel frame with the hard shadow. Focus: the frame turns coral.
- A composer that cannot send (demo, archived, or its machine unreachable) has no hard shadow
  and a 1 px `line` border.
- Send: a 32 px framed square, coral fill, `on_coral` arrow. Disabled: `line` fill, muted arrow,
  no shadow. Stop: the same button with a square glyph.
- Model and effort trigger: model name Inter 13/600, effort in Departure Mono 11 px, sentence
  case.
- The row under the composer (machine selector, access mode): Departure Mono 11 px.

### Menus, popovers, dialogs, pickers

- `card` fill, 6 px radius, pixel frame with shadow, 4 px inner padding, 28 px rows with 4 px
  radius. The highlighted row gets `coral_bg` and coral text is not required.
- Dialog titles in Source Serif 4 20/600. Form labels ("Workspace", "Name") are Departure Mono
  11 px uppercase, muted. Keycaps are framed Departure Mono chips. A menu's own Cancel row is
  plain; only a dialog's Cancel takes the framed secondary button.
- The effort slider keeps its Codex-derived geometry and motion. Only recolour it: track on
  `sea_bg`, thumb `card` with a 2 px `frame` border, sparkles in brass.
- The captain editor and picker: portraits in pixel frames, the selected portrait with a coral
  frame and a coral hard shadow. Captain names in Source Serif 4.
- Every text input has a 1 px `field_line` border and a 2 px coral border with a coral caret
  while focused, from one helper (`FieldFocus::field_focus`).
  A modal's scrim covers the whole window, sidebar and top bar included.
- Settings switches: square, framed, coral when on.

### Spacing

- One spacing scale: 4, 8, 12, 16, 24, 32. Remove one-off values where they are not ported
  KSG measurements that tests pin.
- Sidebar rows share one left inset (12 px) for text and one for portraits and tiles.
- Check alignment at 620, 960, 1280 and 1920 px.

## 7. What must not change

- Behaviour, keys used by tests (`.key(...)`), accessibility labels and roles.
- Lane widths and the KSG-derived structure listed in `AGENTS.md`.
- The removed starter prompts and "No sandbox" note stay removed.
- `cargo build`, `cargo test`, `cargo fmt` pass. Update tests that pin colours, radii or sizes.

## 8. Order of work

1. Foundation: palette, fonts, `fn icon` on Pixelarticons, pixel-frame and pixel-label helpers.
2. Top bar and sidebar.
3. Chat pane, work log, markdown, composer.
4. Scene art: horizon band and sidebar sea strip.
5. Menus, dialogs, pickers, captain screens, settings.
6. Spacing and alignment sweep at the four widths, light and dark.

After each step, render scenes with
`target/debug/shina --snap out.png --scene <name> --size 1280x860` (`--list-scenes` lists them)
and look at them.

## 9. Revisions after implementation

These replace the earlier text where they differ.

- **Light chrome is the day sky.** `bg` is `#d5e8ef` (window, sidebar, tab strip). The chat pane
  stays sailcloth (`#f8f4ea`), so the new-thread band stands out from the chrome, and the sea
  strip reads as the horizon under the sidebar. The night theme is unchanged.
- **Night frames are lighter.** `frame` is `#52688e` at night so the composer, tabs, buttons and
  menus keep an outline; portraits keep `#3a4a6c`. The hard shadow stays near-black
  (`#060a13`). The light hard shadow is `#1f2538` at 30%.
- **Shadows are for things you act on.** Portraits, buttons, the composer, menus and dialogs
  have the hard shadow. Keycaps, count badges and the Demo badge only inform, so they keep a
  1 px outline and no shadow. The brass attention badge opens a menu, so it keeps both, and it
  darkens (20% on hover, 32% pressed) instead of changing to a wash.
- **Status marks are squares.** The avatar status badge is a 2 px radius square with a 1 px
  `frame` border, filled with the panel it sits on.
- **Icons.** Settings is a gear on 2 unit cells (a one cell ring with an open centre, four wide
  and four corner teeth). More is
  three solid 4 unit squares. The paperclip is drawn upright, because a tilted clip becomes a
  staircase of stripes at 18 pt. Delete uses the trash glyph. Chevrons and the check stay
  heavier than the outline icons on purpose.
- **Light surfaces.** `code_bg` is `#ebe6d3` and `message_bg` `#e0d6ba`, and the light user
  bubble has a 1 px `line` border, as the dark one has its frame.
- **Case and joins** are as set out under Typography.
- **Restraint.** The seascape band and the framed portraits are the memorable elements. Do not
  add decoration beyond this document. The extra dotted rule under the drafts list was removed.
- **Deviations accepted:** icons at 18 pt, `muted` and `on_coral` for contrast, 1 px frames on
  small tiles and portraits, the 11 px wordmark in a narrow sidebar with a badge, a fixed inline
  code radius (Rune), and static scenery.

## 10. Addendum from the third review

- **Crisp art.** Portraits, strip frames and the mark are scaled to whole device pixels by
  `src/crisp.rs`; see Portraits in section 4. This replaces the old "no nearest-neighbour
  sampling" deviation.
- **Top-bar mark.** It keeps the day and night art and takes a 1 px `frame` outline in both themes.
- **Chat header.** A 1 px `line` rule closes it, so scrolled messages have a boundary. With the
  Brief open, the header's Brief control becomes an icon-only active toggle, like the others.
- **Diff blocks.** Tinted rows run edge to edge under the header, with 6 px of padding above and below the lines. In the night theme a removed line mixes 17% coral into
  `code_bg` (near `#2f2126`) so it reads as removed.
- **Count badges.** Brass counts what needs you, a `card` fill with `ink` digits and a `frame`
  border counts what is unread, and `sea_bg` with `sea` text and a `sea` frame counts work in
  progress (the crew tray's "working"). Coral stays for the one primary action.
- **Headline.** "What should we build in" and the workspace name sit one word space apart.
- **Shadows.** The horizon band casts the hard shadow, which means its scenery clips in its own
  layer. The 72 px hero portrait casts it too.

## 11. Round 3 fixes

- **Connection state is not in the sidebar.** Rows never say "offline". Two places show a
  dropped machine: one notice (`machines.notice`, framed, a brass mark, a pixel-type label, all
  machines in one notice) that clears when the machine is ready again, and the open thread's own
  chat header and composer ("<Machine> is unreachable. Reconnecting.", send disabled). Demo mode
  never shows a disconnected machine.
- **Fixed radii.** Rows, buttons and chips are 4 px, cards 6 px.
- **Assistant label.** "Shina" above a turn is `ink` on the coral wash. A test covers the pair.
- **Interim prose.** Assistant paragraphs between tool rows share the lane's one left margin
  with the captain's name. Only an expanded work log's tool rows are indented, as a group.
- **Icons on 2 unit cells.** Clock, lightbulb, chip (a model with no vendor), settings, pencil,
  paperclip and the away crescent use only even coordinates, so every edge lands on a device
  pixel at 2x. A test pins it.
- **Away status.** Both texts are Departure Mono. "Working in the background" is `sea`; paused
  routines are `muted`.
- **Brief.** Memory edit and delete icons show on hover and keyboard focus of the row. Section
  counts sit at the heading's end. Approval cards have a 2 px brass bar on the left, and crew
  tray chips a `card` fill with a 1 px `line` border.
- **Captain home band.** It keeps the normal day or night sky. Only the lowest sky step takes
  40% of the portrait's backdrop.
- **Dark text steps.** `muted` is `#b8c7e0` (6.3:1 on a hovered card) and `faint` `#98aac5`
  (4.6:1), so they read as two steps. The night hover wash is 6%.
- **Art cache.** Pre-scaled portraits are cached for the current display scale and zoom only,
  up to 256 buffers.
