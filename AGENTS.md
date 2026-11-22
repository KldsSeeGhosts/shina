# Shina agent guide

Shina is a native Rust app built on [Rune](https://github.com/anomalyco/rune). Almost all UI lives in `src/ui.rs`.

## Visual skin: KSG Code

Since 2026-11-18 Shina matches KSG Code's own visual design (the redesign merged in `7959a26823` and its follow-ups), not the Pixel Voyage skin. The user moved away from Pixel Voyage's pixel-art chrome (pixel fonts, staircase icons, heavy 2 px frames, hard offset shadows, the horizon band and sea strip) because it tires the eyes over long sessions. `docs/ui/pixel-voyage.md` is kept as history only. Do not bring its chrome back. Colour, type, radius and icons come from KSG Code's `apps/web/src/index.css` like structure and behaviour do.

Notes for the skin in code:

- `Palette` in `src/ui.rs` holds KSG's neutral surfaces and Wave 3's status colours. Use `lift` for an active option on a card and `row` for sidebar selection. Use `sea` for working and focus, `brass` for requests, `danger` for failure and deletions, and `ok` for success and additions. Each status has its own `_bg` tint. `coral` keeps its existing primary-button role. Text uses KSG's ramp from `index.css`: `strong` (`--strong-foreground`) for the open sidebar thread and other active labels, `ink` (`--foreground`) for prose, titles and values, `muted` (`--muted-foreground`) for tool rows, labels and meta text, and `faint` (`--placeholder`) for carets, timestamps and counts. `faint` reads 3:1, not 4.5:1, so never put text a reader needs in it. All four are opaque. Hover blends in linear light, so keep its dark wash below selection and never stack two washes. Contrast tests in `src/ui.rs` and `src/markdown_ui.rs` cover both themes and the hover composites.
- All UI text uses one stack, `SANS_FAMILY`, as KSG's `--font-sans` does: the user's installed Anthropic Sans (their KSG Appearance font) when present, else the bundled Inter, which is KSG's default. Do not pin a style to a named family. Code uses `MONO_FAMILY` (Anthropic Mono, else the system mono). `meta(size)`, `META` and `META_LABEL` are the small 12 px weight 500 styles for labels, counts, timers, status words and badges; section labels are sentence case. The system mono is only for code, inline code, keycaps, branch names and diff gutters. Prose, titles and the empty-thread greetings follow KSG's type scale (14/22.5 prose, `DraftHeroHeadline` at 30/500 with tight tracking).
- Corners follow KSG's radii: 6 for rows and chips, 8 for buttons, inputs and tabs, 12 for cards, menus, dialogs and code blocks, 16 for user bubbles and the composer tray with the 15 px main card inside it.
- `framed` is a 1 px `frame` border with KSG's soft control shadow, for things you act on. `outlined` is the border only, for keycaps, counts and badges. Use `soft_shadow` for menus and popovers and `dialog_shadow` for scrim-backed dialogs. There are no hard offset shadows.
- `fn icon` takes names from `src/icons.rs`: Lucide outlines on a 24 unit grid, stroked in `currentColor` with round caps and joins, drawn at `ICON` (16 pt) as KSG's `size-4` icons. Lucide is ISC; its licence is `assets/icons-LICENSE.txt`. A test there checks every name has its own glyph.
- Illustrated Captain portraits fill any rounded slot and use a filtered bitmap at device size. A missing expression uses the same Captain's `Normal` face and optional head crop. The legacy pixel set and app mark still go through `crisp` and whole-device-pixel sizing. Only pixel sets play strips. Use `haloed` for working, waiting and failed portraits at 30 pt and up; smaller avatars use a corner mark. The working sweep stops with Reduce Motion. `crewmate_avatar` loads provider mascots from `assets/providers/crew` when available and uses a tinted provider tile otherwise.

## Port UI from KSG Code. Do not invent it.

KSG Code, the user's T3 Code fork, already has the interface the user wants. Its source is at `D:\AiStack\ksgcode` (web UI in `apps/web/src/components`). For any UI work, read the matching KSG Code component first and port its layout, spacing, typography, and behaviour into Rune. Design something new only when KSG Code has no equivalent, and say so when you do.

Workflow:

1. Find the component in KSG Code. Search for its visible text (`rg "What should we work on" apps/web/src`) or its name.
2. Read its markup and Tailwind classes, plus any CSS variables in `apps/web/src/index.css`.
3. Convert the classes to Rune values with the table below. Copy KSG's proportions, structure and look. Use the matching `Palette` token for each KSG colour variable.
4. Put a short comment on the Rune function that names the KSG source, such as "ported from KSG Code's `ComposerAttachmentStrip`".
5. Add or update a headless layout test only when layout or behavior changes in a way users hit day to day. Check only the width or widths where the layout actually branches (such as one narrow and one wide breakpoint), not a blanket four-width loop.

Already ported:

| Shina (`src/ui.rs`) | KSG Code source |
| --- | --- |
| `empty_thread`, `draft_headline` | `components/ChatView.tsx` (draft hero overlay), `components/NoProjectsHero.tsx`, `chat/DraftHeroHeadline.tsx`, `chat/DraftHeroProjectHeading.tsx` |
| `add_project_palette`, `browse_tab_complete`, `browse_into` | `components/CommandPalette.tsx` (`add-project` flow, `getAddProjectInitialQueryForEnvironment`, `getFilesystemBrowseTabCompletion`, `browseTo`) |
| `new_thread_click` (`src/model.rs`) | `components/Sidebar.logic.ts` (`shouldCreateNewThreadInCurrentProject`), `lib/chatThreadActions.ts` (`startNewThreadFromContext`) |
| `composer` card and footer | `chat/ComposerSurface.tsx`, `chat/ChatComposer.tsx` |
| `attachment_chip` | `chat/ComposerAttachmentStrip.tsx` |
| `model_effort_trigger`, `model_picker`, `model_list`, `tallest_model_tab_h` | `chat/ProviderModelPicker.tsx`, `chat/ModelPickerContent.tsx` (`tallestTabUnits`), `chat/ModelPickerSidebar.tsx`, `chat/ModelListRow.tsx` |
| `options_column`, `option_rows`, `option_choice_menu` | `chat/ComposerModelEffort.tsx` (`ComposerEffortColumn` with `optionsOnly`, `OptionMenuRow`, `OptionSwitchRow`) |
| `reasoning_picker`, `speed_section` | `chat/ComposerModelEffort.tsx` (`ComposerReasoningPicker`), `chat/TraitsPicker.tsx` (`TraitsMenuContent` with `reasoningOnly`) |
| `context_ring`, `context_popover` | `chat/ContextWindowMeter.tsx`, `chat/ContextWindowMeter.logic.ts`, `ui/popover.tsx` |
| `measure`, `lane_gutter`, `conversation`, `composer` lane | `index.css` (`.chat-content-lane`, `.chat-composer-lane`), `chat/MessagesTimeline.tsx`, `chat/TimelineUserMessageRow.tsx` |
| `composer_shell`, `composer-prompt-body`, `composer_ui::controls` strip, `effort_trigger` | `chat/ComposerSurface.tsx`, `chat/ChatComposer.tsx`, `chat/ComposerControlsBar.tsx`, `chat/ComposerModelEffort.tsx`, `index.css` (`.composer-prompt-body`) |
| `assistant_message_meta` | `chat/TimelineAssistantRows.tsx` (`AssistantMessageMeta`) |
| `working_bar_frame`, `work_fold_toggle`, `group_work`, `work_live_row`, `activity_shimmer` | `chat/MessagesTimeline.logic.ts` (`deriveTimelineRows`, `activeWorkRow`), `chat/TimelineWorkGroupRows.tsx` (`WorkingTimelineRow`, `LiveWorkEntryTimelineRow`, `ThinkingTimelineRow`), `chat/TimelineRowParts.tsx` (`WORKING_BAR_CLASS_NAME`, `WorkFoldToggle`, `ActivityShimmerOverlay`) |
| `activity_line`, `inline_tool`, `work_part`, `work_group_head`, `work_entry_row`, `live_toggle` | `chat/WorkLog.tsx` (`WorkLogRow`, `WorkLogLine`, `WorkVerbLabel`, `useRowClassName`), `chat/TimelineWorkEntryRow.tsx` (`WorkEntryLogRow`), `chat/TimelineWorkGroupRows.tsx` (`WorkGroupSection`), `chat/MessagesTimeline.logic.ts` (`workEntryDisplayLabel`, `liveWorkEntryLabel`, `workEntryIsVisibleInGroup`) |
| `work_ui::expanded`, `work_ui::thoughts`, `work_ui::thought_head`, `work_ui::thought_body`, `work_ui::has_body` | `chat/WorkLog.tsx` (`WorkLogDetails`, `WORK_LOG_OUTPUT_CLASS_NAME`), `chat/V2ItemInspector.tsx` (`ToolCallBody`, `StructuredValue`, file_change, file_search and web_search sections), `chat/ReasoningTraceContent.tsx`, `chat/RunningFor.tsx` |
| `work::command_display_text`, `work::command_program_name`, `work::command_runs_tests`, `work::search_label`, `work::read_paths`, `work::summarize` | `work-log/commandLabel.ts`, `work-log/presentation.ts` (`commandlessCommandLabel`, `genericCommandStatusLabel`, `summarizeToolGroup`, `toolGroupActionLabel`, `toolGroupActionCount`, `summaryActionPriority`), `shared/toolActivity.ts` (`formatSearchToolLabel`, `formatReadToolLabel`, `collectToolFilePaths`), `shared/toolOutput.ts` (`commandRunsTests`) |
| `work_ui::kind_icon` and the `icons.rs` glyphs | `chat/TimelineToolActivityIcon.tsx` (`workEntryIconName`, `toolGroupSummaryIconName`, `failedToolIconClassName`) |
| `markdown_ui` style and `code_frame` | `ChatMarkdown.tsx`, `index.css` (`.chat-markdown`), `chat/MarkdownCodeBlock.tsx` |
| `captain_editor` portrait grid | `settings/ProjectIconPickerDialog.tsx` (emoji grid) |
| `provider_icon` | `components/Icons.tsx` (`OpenAI`, `ClaudeAI`, `AntigravityIcon`), `chat/ProviderInstanceIcon.tsx` |
| `crew_ui::needs_you` | `chat/ChildThreadApprovals.tsx` (one paged request card, spec §3.6) |
| `crew_ui::task_card` | `chat/ProviderSubagentBar.tsx` restyled as a single 32 pt work row (spec §3.7) |
| `crew_ui::crew_tray`, `crew_ui::now_popover` | `chat/QueuedRunsControl.tsx`, `chat/ThreadRelationshipsControl.tsx`. A flat 28 pt strip, labelled Crew on a Captain home and Now on a thread with subagents. It opens a dismissible popover and never expands in place. The popover's footer opens the Inspector's Work tab. |
| `crew_ui::parent_bar`, `crew_ui::crew_bar` | Shina-only 36 pt borderless notice bars (`p.selected`), shown only for actionable recovery/offline/pending-crew states (spec §3.8) |
| `details_ui::card` (`workspace_card`, `lineage_card`, `lineage_row`, `hover_card`), `canvas_layout` | `chat/ThreadDetailsCard.tsx`, `ThreadDetailsPanel.tsx`, `ThreadDetailsSection.tsx`, `ThreadDetailsControl.tsx`, `ThreadRelationshipsControl.tsx`, `threadDetailsPanelStyles.ts`, `threadDetailsCardLayout.ts`, `chatCanvasLayout.ts`, `index.css` (`.thread-details-stack`). A 272 pt stack of 14 pt trays docks 12 pt inside the chat column's top right corner and the lane moves 32 pt clear of it. Where the column cannot fit it, the top bar's details button opens it as a popover. The Lineage card lists crew and provider subagents in 40 pt two-line rows, the finished ones under "N previous agents" (6, then 12 more at a time), each with a hover card. A row opens the worker's thread or the subagent in the Work tab. The model, access, harness and session ID rows behind "Show N more" are Shina-only. |
| `inspector_ui` bodies: `overview_body`, `work_body`, `peer_crew_body` | `chat/ChatRightPanel.tsx`, `RightPanelTabs.tsx`. The Work tab is Shina-only: subagents (`agents_ui::view`) above the crew roster (`crew_ui::roster_row` groups and a pinned 44 pt footer), both in 44 pt two-line rows with a 24 pt avatar. |
| `captain_hero` | `chat/DraftHeroHeadline.tsx` |
| `captain_selector_trigger`, `captain_picker` | `chat/BranchToolbarEnvironmentSelector.tsx`, `chat/ModelPickerContent.tsx` (with Shina-only captain rows) |
| `harness_icon` and the harness "ACP" tag | `chat/ModelListRow.tsx`, `chat/ProviderInstanceIcon.tsx` |
| `inspector_ui::view`, `agents_ui::view` | `chat/ChatRightPanel.tsx`, `RightPanelTabs.tsx`, `chat/ProviderSubagentBar.tsx`. The strip has Work, Context and Changes on every thread, and a Captain's Brief first on its home. A thread's own details are the floating card, not a tab. Tabs are borderless and the active one takes `lift`. |
| `sidebar_ui::content`, `sidebar_row::{thread_row_el,captain_row_el}`, `sidebar_preview::view` | KSG Code's flat list and `SidebarThreadRow` card variant for active and pinned threads. Keep project and machine on each card, separate from the title. Use the model avatar, or Shina's default avatar when no model avatar matches. Only shelves and drafts use slim rows. Captain rows keep their portraits and crew attention count. |
| `pi_ui::access_picker`, `updates_ui::footer`, `updates_ui::popover` | `chat/ComposerPermissionsSection.tsx` and `sidebar/SidebarUpdatePill.tsx`, adapted to the Wave 3 access radio group and quiet footer |
| `top_bar::bar`, `quick_switcher::view` | `chat/CommandPalette.tsx`, `chat/CommandPaletteResults.tsx` (row metrics for the switcher; the single 40 pt title bar with nav buttons and the breadcrumb title is a Shina-only design) |

The model picker and the reasoning menu are separate popovers, as in KSG Code. Model rows have no checkboxes: a click picks the model and Shift+click adds it to a multi-model send. The reasoning menu keeps the effort slider below instead of KSG's list of levels, at the user's request.

The effort slider (`effort_slider`, `slider_curve`, `sparkles`) is ported from the Codex desktop app instead. Its source ships uncompressed in `C:\Program Files\WindowsApps\OpenAI.Codex_*\app\resources\app.asar`. Search it as text for `data-reasoning-slider`, `model-picker-power-slider` and the CSS classes it names. Read it there, never run it. Its values: a 24 px track in a 28 px row, a 28 px white thumb with a hairline edge, motion of 0.3 s on `cubic-bezier(.23, 1, .32, 1)` (0.15 s while dragging), a thumb that follows the pointer with no detents, and a closed-hand cursor only while the slider is pressed or dragged (hover keeps the arrow). winit shows `Cursor::Grabbing` as the four-way move arrow on Windows, so the hand is an image cursor, `assets/cursors/grabbing.png`, drawn by `tools/cursors.py`. Rune lands drag commits at once, so each drag phase sets `tx.motion_curve(...)`.

Removed at the user's request. Do not add them back:

- Starter prompt buttons ("Understand project", "Plan a change") on the new-thread screen.
- The "No sandbox" note under the composer.
- The desktop task plan panel and the Plan toggle on the crew strip. Captains keep the `task_*` MCP tools.

The user wants Shina to match KSG Code verbatim (restated 2026-11-22). When a Shina design and KSG's differ, copy KSG's values, weights and layout from `origin/ksg/dev` exactly, and keep a Shina-only piece only where KSG has nothing equivalent. Compare by screenshot: capture the running KSG Code window and Shina's `--demo --snap` at the same pixel size and theme.

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
| `max-w-(--chat-content-max-width)` (45 rem at every width) | `measure(wide)` |
| `flex-1 basis-0` | `.h(0.0).grow(1.0).min_h(0.0)` (Rune has no `basis`) |

## Checks

- Run `cargo fmt --check`, `cargo build -j 4`, and targeted tests only (`cargo test -j 4 --bin shina -- <filter> -- --test-threads=4`), one worktree at a time. Do not run the full test suite locally unless investigating a repo-wide failure; GitHub Actions CI runs the full unit and integration suites on every PR. Review agents must follow the same targeted-test limit.
- Do not write pointless tests for contrived states, styling trivia, or edge cases that day-to-day use will not encounter. Test real user workflows, data-safety invariants, and bugs that actually happened. Avoid duplicating coverage across extra widths, themes, or overlapping unit tests when one focused test covers the behavior.
- Remove helpers your change leaves unused.
- To look at a dev build, run `target\debug\shina.exe` with `LOCALAPPDATA` set to a temporary folder. That keeps it away from the user's real threads in `%LOCALAPPDATA%\Shina`.
- Tests find elements by `.key(...)`. Keep existing keys when restyling, or update every test that uses them.

## Shipping

- Prefer releases over local builds so Windows and macOS stay in sync without drift.
- After checks pass, commit and push your changes rather than leaving them only as local working-tree edits or local builds.
- For PRs that change the UI, embed labeled Before and After screenshots in the PR description. Capture the base revision and the PR revision with matching demo data, platform, viewport, scale, theme, and UI state. Prefer Shina's `--demo --snap` mode and never use a personal profile. Attach the images to the PR or link durable copies outside the code branch. If a matching Before screenshot is impossible, say why.
