# Shina agent guide

Shina is a native Rust app built on [Rune](https://github.com/anomalyco/rune). Almost all UI lives in `src/ui.rs`.

## Visual skin: Pixel Voyage

The visual skin follows `docs/ui/pixel-voyage.md`: the palette, Departure Mono labels, Pixelarticons icons, 4 px and 6 px corners, 2 px pixel frames with hard shadows, and the horizon band and sea strip. Structure and behaviour are still ported from KSG Code, as described below. Where this section and the KSG Code section disagree about colour, type, radius or icons, the design document wins. Where they disagree about what goes where or what a click does, KSG Code wins.

Notes for the skin in code:

- `Palette` in `src/ui.rs` holds the colours. `pixel(size)`, `PIXEL` and `PIXEL_LABEL` are the Departure Mono styles. Use the face at 11 px or 22 px only, and only for labels, counts, timers, status words, badges and keycaps. Prose, titles, composer text and code stay Inter, Source Serif 4 and the system mono.
- `framed`, `hard_shadow` and `pixel_portrait` draw the pixel frame. Rune has no blur in this skin and cannot move a button on press, so a pressed framed button darkens. Use `framed` (frame and shadow) only for things you act on. `outlined` (frame only) is for keycaps, counts and badges that only inform. Text colours are `ink`, `muted` and `faint`, all opaque; do not put an alpha on text.
- `fn icon` takes names from `src/pixel_icons.rs`. The glyphs sit on a 24 unit grid and draw at `ICON` (18 pt), because 2 unit squares then land on whole device pixels at 2x. Pixelarticons is MIT; its licence is `assets/icons-LICENSE.txt`. Departure Mono is SIL OFL; its licence is next to the font.
- `src/crisp.rs` scales pixel art to whole device pixels, because Rune samples images with a filter. Never hand Rune a portrait or the mark at a size that is not a whole multiple of its cells. Ask `Artwork::face_pixels` or `Artwork::mark_pixels` (and `Portraits::pixels` for animated stages) for the size in points, and keep slot sizes at multiples of the art where you can (40 pt for the 40 cell busts). The view tells `Artwork` the device scale each frame.
- `src/horizon.rs` builds the scenery from the app mark's palette. Tests there and in `src/ui.rs` pin its geometry.

## Port UI from KSG Code. Do not invent it.

KSG Code, the user's T3 Code fork, already has the interface the user wants. Its source is at `D:\AiStack\ksgcode` (web UI in `apps/web/src/components`). For any UI work, read the matching KSG Code component first and port its layout, spacing, typography, and behaviour into Rune. Design something new only when KSG Code has no equivalent, and say so when you do.

Workflow:

1. Find the component in KSG Code. Search for its visible text (`rg "What should we work on" apps/web/src`) or its name.
2. Read its markup and Tailwind classes, plus any CSS variables in `apps/web/src/index.css`.
3. Convert the classes to Rune values with the table below. Copy KSG's proportions and structure. Keep Shina's palette (`Palette` in `src/ui.rs`) and its fonts.
4. Put a short comment on the Rune function that names the KSG source, such as "ported from KSG Code's `ComposerAttachmentStrip`".
5. Add or update a headless layout test in `src/ui.rs` (`runtime(width)`, `rt.presented(key)`). Check at least 620, 960, 1280 and 1920 px wide.

Already ported:

| Shina (`src/ui.rs`) | KSG Code source |
| --- | --- |
| `empty_thread`, `draft_headline` | `components/ChatView.tsx` (draft hero overlay), `components/NoProjectsHero.tsx`, `chat/DraftHeroHeadline.tsx`, `chat/DraftHeroProjectHeading.tsx` |
| `add_project_palette`, `browse_tab_complete`, `browse_into` | `components/CommandPalette.tsx` (`add-project` flow, `getAddProjectInitialQueryForEnvironment`, `getFilesystemBrowseTabCompletion`, `browseTo`) |
| `new_thread_click` (`src/model.rs`) | `components/Sidebar.logic.ts` (`shouldCreateNewThreadInCurrentProject`), `lib/chatThreadActions.ts` (`startNewThreadFromContext`) |
| `composer` card and footer | `chat/ComposerSurface.tsx`, `chat/ChatComposer.tsx` |
| `attachment_chip` | `chat/ComposerAttachmentStrip.tsx` |
| `model_effort_trigger`, `settings_picker`, `model_list` | `chat/ComposerModelEffort.tsx`, `chat/ModelPickerContent.tsx`, `chat/ModelListRow.tsx` |
| `context_ring`, `context_popover` | `chat/ContextWindowMeter.tsx`, `chat/ContextWindowMeter.logic.ts`, `ui/popover.tsx` |
| `measure`, `lane_gutter`, `conversation`, `composer` lane | `index.css` (`.chat-content-lane`, `.chat-composer-lane`), `chat/MessagesTimeline.tsx`, `chat/TimelineUserMessageRow.tsx` |
| `captain_editor` portrait grid | `settings/ProjectIconPickerDialog.tsx` (emoji grid) |
| `provider_icon` | `components/Icons.tsx` (`OpenAI`, `ClaudeAI`, `AntigravityIcon`), `chat/ProviderInstanceIcon.tsx` |
| `crew_ui::needs_you` | `chat/ChildThreadApprovals.tsx` |
| `crew_ui::parent_bar`, `crew_ui::task_card` meta line | `chat/ProviderSubagentBar.tsx` |
| `crew_ui::crew_tray` placement | `chat/QueuedRunsControl.tsx` |
| `captain_hero` | `chat/DraftHeroHeadline.tsx` |
| `captain_selector_trigger`, `captain_picker` | `chat/BranchToolbarEnvironmentSelector.tsx`, `chat/ModelPickerContent.tsx` (with Shina-only captain rows) |
| `harness_icon` and the harness "ACP" tag | `chat/ModelListRow.tsx`, `chat/ProviderInstanceIcon.tsx` |

The effort slider (`effort_slider`, `slider_curve`, `sparkles`) is ported from the Codex desktop app instead. Its source ships uncompressed in `C:\Program Files\WindowsApps\OpenAI.Codex_*\app\resources\app.asar`. Search it as text for `data-reasoning-slider`, `model-picker-power-slider` and the CSS classes it names. Read it there, never run it. Its values: a 24 px track in a 28 px row, a 28 px white thumb with a hairline edge, motion of 0.3 s on `cubic-bezier(.23, 1, .32, 1)` (0.15 s while dragging), a thumb that follows the pointer with no detents, and a closed-hand cursor only while the slider is pressed or dragged (hover keeps the arrow). winit shows `Cursor::Grabbing` as the four-way move arrow on Windows, so the hand is an image cursor, `assets/cursors/grabbing.png`, drawn by `tools/cursors.py`. Rune lands drag commits at once, so each drag phase sets `tx.motion_curve(...)`.

Removed at the user's request. Do not add them back:

- Starter prompt buttons ("Understand project", "Plan a change") on the new-thread screen.
- The "No sandbox" note under the composer.

### Tailwind to Rune

1 rem is 16 px. Tailwind spacing is 4 px per step.

| Tailwind | Rune |
| --- | --- |
| `p-2`, `px-4`, `pt-3.5`, `gap-2` | `.p(8.0)`, `.px(16.0)`, `.pt(14.0)`, `.gap(8.0)` |
| `rounded-lg`, `rounded-xl`, `rounded-2xl`, `rounded-full` | `.radius(8.0)`, `.radius(12.0)`, `.radius(16.0)`, half the size |
| `text-xs`, `text-sm`, `text-base` | `TextStyle::new(12.0, ..)`, `14.0`, `16.0` |
| `text-2xl`, `text-3xl` | `24.0` with `lh(32.0)`, `30.0` with `lh(36.0)` |
| `font-normal`, `font-medium`, `font-semibold` | weight `400.0`, `500.0`, `600.0` |
| `tracking-tight` | `.tracking(-0.025 * size)` |
| `size-16` / `h-16 w-16` | `.size(64.0, 64.0)` |
| `absolute right-1 top-1` | `.pin(None, Some(4.0), Some(4.0), None)` |
| `object-cover` image | `.pixels_fit(pixels, rune::SurfaceFit::Fill)` |
| `text-center` on wrapping text | `.align(rune::Align::Center).wrap().w_full()` |
| `max-w-(--chat-content-max-width)` (46 rem) | `measure(wide)` |
| `flex-1 basis-0` | `.h(0.0).grow(1.0).min_h(0.0)` (Rune has no `basis`) |

## Checks

- `cargo build`, `cargo test`, and `cargo fmt` must pass. Remove helpers your change leaves unused.
- To look at a dev build, run `target\debug\shina.exe` with `LOCALAPPDATA` set to a temporary folder. That keeps it away from the user's real threads in `%LOCALAPPDATA%\Shina`.
- Tests find elements by `.key(...)`. Keep existing keys when restyling, or update every test that uses them.
