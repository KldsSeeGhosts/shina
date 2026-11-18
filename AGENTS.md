# Shina agent guide

Shina is a native Rust app built on [Rune](https://github.com/anomalyco/rune). Almost all UI lives in `src/ui.rs`.

## Visual skin: KSG Code

Since 2026-11-18 Shina matches KSG Code's own visual design (the redesign merged in `7959a26823` and its follow-ups), not the Pixel Voyage skin. The user moved away from Pixel Voyage's pixel-art chrome (pixel fonts, staircase icons, heavy 2 px frames, hard offset shadows, the horizon band and sea strip) because it tires the eyes over long sessions. `docs/ui/pixel-voyage.md` is kept as history only. Do not bring its chrome back. Colour, type, radius and icons come from KSG Code's `apps/web/src/index.css` like structure and behaviour do.

Notes for the skin in code:

- `Palette` in `src/ui.rs` holds KSG's default dark and light themes: neutral chrome (`bg`), the chat pane (`panel`), raised cards (`card`), the composer tray (`tray`), hairlines (`line`, `frame`, `field_line`), the focus ring (`ring`), and KSG's info blue in `sea`. Some field names predate the theme: `coral` is the primary key (KSG's light `surface-lit` button in dark, near-black in light) and `brass` is KSG's warning. Text colours are `ink`, `muted` and `faint`, all opaque; do not put an alpha on text. Contrast tests in `src/ui.rs` and `src/markdown_ui.rs` keep text at 4.5:1 or better.
- All UI text is Inter. `meta(size)`, `META` and `META_LABEL` are the small 12 px weight 500 styles for labels, counts, timers, status words and badges; section labels are sentence case. The system mono is only for code, inline code, keycaps, branch names and diff gutters. Prose, titles and the empty-thread greetings follow KSG's type scale (14/22.5 prose, `DraftHeroHeadline` at 30/500 with tight tracking).
- Corners follow KSG's radii: 6 for rows and chips, 8 for buttons, inputs and tabs, 12 for cards, menus, dialogs and code blocks, 16 for user bubbles and the composer tray with the 15 px main card inside it.
- `framed` is a 1 px `frame` border with KSG's soft control shadow, for things you act on. `outlined` is the border only, for keycaps, counts and badges. Menus, popovers and dialogs take `soft_shadow`. There are no hard offset shadows.
- `fn icon` takes names from `src/icons.rs`: Lucide outlines on a 24 unit grid, stroked in `currentColor` with round caps and joins, drawn at `ICON` (16 pt) as KSG's `size-4` icons. Lucide is ISC; its licence is `assets/icons-LICENSE.txt`. A test there checks every name has its own glyph.
- Captain portraits stay pixel art, framed with rounded corners and a 1 px hairline. `src/crisp.rs` scales pixel art to whole device pixels, because Rune samples images with a filter. Never hand Rune a portrait or the mark at a size that is not a whole multiple of its cells. Ask `Artwork::face_pixels` or `Artwork::mark_pixels` (and `Portraits::pixels` for animated stages) for the size in points, and keep slot sizes at multiples of the art where you can (40 pt for the 40 cell busts). The view tells `Artwork` the device scale each frame.

## Port UI from KSG Code. Do not invent it.

KSG Code, the user's T3 Code fork, already has the interface the user wants. Its source is at `D:\AiStack\ksgcode` (web UI in `apps/web/src/components`). For any UI work, read the matching KSG Code component first and port its layout, spacing, typography, and behaviour into Rune. Design something new only when KSG Code has no equivalent, and say so when you do.

Workflow:

1. Find the component in KSG Code. Search for its visible text (`rg "What should we work on" apps/web/src`) or its name.
2. Read its markup and Tailwind classes, plus any CSS variables in `apps/web/src/index.css`.
3. Convert the classes to Rune values with the table below. Copy KSG's proportions, structure and look. Use the matching `Palette` token for each KSG colour variable.
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
| `composer_shell`, `composer-prompt-body`, `composer_ui::controls` strip, `effort_trigger` | `chat/ComposerSurface.tsx`, `chat/ChatComposer.tsx`, `chat/ComposerControlsBar.tsx`, `chat/ComposerModelEffort.tsx`, `index.css` (`.composer-prompt-body`) |
| `assistant_message_meta` | `chat/TimelineAssistantRows.tsx` (`AssistantMessageMeta`) |
| `working_bar_frame`, `work_fold_toggle`, `group_work`, `work_live_row`, `activity_shimmer` | `chat/MessagesTimeline.logic.ts` (`deriveTimelineRows`, `activeWorkRow`), `chat/TimelineWorkGroupRows.tsx` (`WorkingTimelineRow`, `LiveWorkEntryTimelineRow`, `ThinkingTimelineRow`), `chat/TimelineRowParts.tsx` (`WORKING_BAR_CLASS_NAME`, `WorkFoldToggle`, `ActivityShimmerOverlay`) |
| `markdown_ui` style and `code_frame` | `ChatMarkdown.tsx`, `index.css` (`.chat-markdown`), `chat/MarkdownCodeBlock.tsx` |
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
| `max-w-(--chat-content-max-width)` (45 rem) | `measure(wide)` |
| `flex-1 basis-0` | `.h(0.0).grow(1.0).min_h(0.0)` (Rune has no `basis`) |

## Checks

- `cargo build`, `cargo test`, and `cargo fmt` must pass. Remove helpers your change leaves unused.
- To look at a dev build, run `target\debug\shina.exe` with `LOCALAPPDATA` set to a temporary folder. That keeps it away from the user's real threads in `%LOCALAPPDATA%\Shina`.
- Tests find elements by `.key(...)`. Keep existing keys when restyling, or update every test that uses them.
