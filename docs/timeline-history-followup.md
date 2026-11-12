# Timeline history follow-up

Reference root: `/Volumes/DevDrive/AiStack/ksgcode`.

## Remembered positions

| KSG source and rule | Shina implementation |
| --- | --- |
| `apps/web/src/components/chat/timelineScrollAnchoring.ts:115-150`: client-local position by scoped thread identity, row ID, offset within row, raw scroll offset, at-end flag, at most 100 positions, oldest write evicted | `timeline_position::Positions::open`, `Visit::position`. The cache is part of each client State, never persisted or sent to the server. |
| `MessagesTimeline.tsx:972-1025`: measure the first visible row relative to the viewport, do not save a partially restored position | `Visit::position` reads Rune's mounted row layouts and `ScrollPos`. `Visit::restore` protects the saved position before the opening layout runs. |
| `MessagesTimeline.logic.ts:326-337`: 40 px end re-arm region | `Visit::position` uses `max - offset <= 40`. |
| `useTimelineFollow.ts:186-190`: a previously free-scrolling thread does not start following on return | `conversation` reveals the saved row and uses `Tx::scroll_key_to` to place it, which leaves Rune's stick-bottom mode unpinned. |
| `useTimelinePositionRestore.ts:105-118`: restore the row, fall back to the raw offset if absent, otherwise open at the end | `conversation` opening layout handler, `scroll_into_view_in`, `scroll_key_to`, `scroll_to_y`, `scroll_to_bottom_instant`. An unseen thread uses Rune's initial stick-bottom layout. |
| `MessagesTimeline.tsx:423-465`, `timelineScrollAnchoring.ts:120-127`: restore disclosures | Shina's disclosures already live on retained `Thread.work` and `Thread.tools` records, not on the mounted view. Switching threads does not reset them. |

KSG's `useTimelinePositionRestore.ts:7-41,119-145` corrects estimated browser layout until two stable animation frames, with a 60-frame ceiling and 1 px tolerance. Rune measures the revealed virtual row and reapplies the keyed destination synchronously in `runtime/scroll/offset.rs:24-85`. Shina uses that native operation rather than adding a browser animation-frame loop. Later measurement and width changes retain Rune's existing virtual-row anchor. There is no asynchronous restore operation to cancel after the opening commit. Subsequent user input takes control immediately.

Headless regression `thread_switch_restores_first_visible_timeline_row_and_offset` first failed on the original implementation. It verifies the first visible row's exact offset after switching away and back at 620, 960, 1280 and 1920 px, then verifies that a thread left at the end returns to a newly appended end. Existing anchor tests cover prepend, fold expansion, image size and width changes.

## Find item kinds

| KSG source and kind | Shina storage and function |
| --- | --- |
| `apps/server/src/orchestration-v2/ThreadFind.ts:53-80,318-323`: `user_message` | `messages`, role `You` or `user`, `Streams::find_thread` |
| Same lines: `assistant_message` | `messages`, role `Pi` or `assistant`, `Streams::find_thread` |
| Same lines: `proposed_plan` | `tools`, kind `plan`, Markdown in `input.markdown`, or the existing `text` and string `plan` aliases. `thread_find::proposed_plan_markdown`, `Streams::find_thread` |
| `ThreadFind.ts:348-361`: `user_input_request` is read to suppress matching `async-answer:<requestId>` user messages, then removed before counting | Shina's transient questionnaire is not a searchable document. Shina does not retain answered question cards or hide their answer messages inside such cards, so there is no equivalent hidden answer document to count. |
| `apps/server/src/orchestration-v2/ThreadSearch.ts:37-41,188-260`, `messageSearchIndex.ts:29-47`: settled user and assistant messages, excluding generated user notices and attachment placeholders | Existing `messages` scan in `Streams::search_threads`. No new transcript kind is added there. KSG does not index `proposed_plan`, `todo_list`, question cards, reasoning, command output, file changes, HTML or other tool output in sidebar search. |

`packages/shared/src/threadFindText.ts:226-232` indexes a plan's displayed title and Markdown body separately. `proposedPlanText.ts:1-22` selects the first heading as the title or defaults to "Proposed plan", removes a leading heading and then a leading Summary heading from the displayed body. `thread_find::plan_parts` and `count_document_occurrences` use those same regions. Todo steps and tool output remain excluded.

`ProposedPlanCard.tsx:77-90,170-222` includes the plan title and body in find and reveals the body on navigation. `work_ui::plan_card` renders and highlights the same regions with Shina's existing colors. `State::consume_pending_find_reveal` recognizes a plan's stable tool ID, loads older history through that ID if absent, opens its work section, and centers the active highlight. Local find uses the same `Streams::find_thread` implementation as the server.

No method or field was added to the protocol for this correction. `thread-find` already returns an unrestricted stable `entryId`, and `throughEntryId` already accepts tool IDs. A peer without `thread-find` keeps the existing explicit update-required state. A build that advertises that capability but predates this bug fix will still omit plans until updated.

New tests cover plan ordering, multiple occurrences, hidden Summary headings, default titles, Markdown/input aliases, exclusion of todo steps and tool output, and sidebar exclusion of plan text. The headless plan reveal/highlight test covers 620, 960, 1280 and 1920 px. The existing 2,700-item linked-server find integration fixture now includes an old plan, checks server title/body matches and loading its history page, and searches the plan again across a new connection.
