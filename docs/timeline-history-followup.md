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

## Deferred work

Paging alone was not equivalent. The 500-item turn budget can admit hundreds of small work records when their user turn fits in the page. The implementation therefore defers work rather than relying on the 75-item non-conversation limit.

| KSG source and rule | Shina implementation |
| --- | --- |
| `apps/server/src/orchestration-v2/threadHistoryPaging.ts:176-193`: command execution, reasoning, file changes, file search, web search and dynamic tools are deferrable only when completed and without persistent output | `history_work::deferrable` handles `thoughts` and the corresponding `tools` kinds. Failed and running tools remain inline. Plans and compactions are conversation records, not deferrable work. |
| `ProjectionStore.ts:2024-2027`: HTML, `t3McpApp`, viewed-image output and delegate-task output stay inline | `history_work::deferrable` retains those payloads and Shina's equivalent HTML and subagent cards and attachment-backed viewed images. |
| `threadHistoryPaging.ts:196-227`: only completed runs or superseded attempts; interrupt requests/results and top-level failed errors block a run; groups are separated by source thread, run, user-message segment and attempt | `history_work::groups` separates Shina's assistant work sections by their message start. `AssistantWork::outcome`, `Thread::finish_run`, history reconstruction and restart recovery distinguish completed, failed, active, stopped and unknown sections. Stopped, failed, active and unknown sections do not defer. |
| `threadHistoryPaging.ts:234-253`: 150 stays inline, above 150 retains the first item with the group's count and inclusive cursor at its last item, omitting the rest | `history_work::deferred`, `INLINE_WORK_ITEMS = 150`. Only the anchor is cloned. Other history payloads remain borrowed while selecting a page. |
| `threadHistoryPaging.ts:255-283`: work reads start at the anchor, stay inside its work group, and page backwards with the ordinary 75-row and byte budgets | `history_work::page`, `Streams::page_with_work`. Cursors carry durable identities and an ordering fallback for a removed cursor row. Shina retains its existing negotiated 64 KiB default, 256 KiB maximum fragment budget rather than KSG's 1 MiB browser page budget. Oversized items still make progress through UTF-8 fragmentation. |
| `apps/web/src/components/chat/MessagesTimeline.tsx:1693-1731`: opening or restoring an expanded fold loads every deferred anchor; a failure says "Couldn't load work. Click to retry."; retry leaves the fold open | `State::load_deferred_work`, `assistant_turn` opening and layout handlers, `ProjectionCache::request_work`. Work reads share the existing single-in-flight history transport. |
| `MessagesTimeline.tsx:1733-1747`: loading status with three skeleton rows, 12 px squares, widths 2/5, 3/5 and 1/3, 8 px gaps, top padding 12 px, bottom and horizontal padding 4 px | `assistant_turn`'s `work-loading-*` element uses those values with Shina's palette. |
| `packages/client-runtime/src/state/threads.ts:710-845`: work cursors are independent of older-history cursors; changed or non-advancing cursors fail explicitly; continue until the work cursor is null | `ProjectionCache::merge_page`, `receive`, `next_history`. Work loading leaves the older-history cursor and replay sequence unchanged. A normal stream update preserves a deferred anchor, and newer live rows win over older page rows. Find waits until a deferred work fold mounts its requested highlight. |

Protocol minor 11 advertises `deferred-work`. It covers `workAnchorId` on `thread.history`, the anchor's `deferredWork` descriptor, and the optional persisted work outcome. Servers only defer for connections that negotiated this capability. A client connected to an older peer enters the explicit `inline-legacy` work-loading state, never sends a work request, and uses ordinary inline history and fold expansion. Downgrading a retained connection discards unresolved deferred views and requests an inline scoped snapshot. Missing old work outcomes deserialize as unknown and stay inline. They are not guessed from elapsed timers.

Item 3 is partial under the no-gaps requirement. Shina has flat work sections, not KSG's separate run-attempt projection. There are no independently identified superseded attempts in Shina's stored transcript to defer through KSG's alternative `attemptStatus === "superseded"` path. The implemented path uses known completed sections. This is a model difference, not a Rune limitation. Exact behavior for a future separate superseded-attempt model is not implemented or verified.

The isolated 500-record integration fixture measured a 3,266-byte initial page and 3,154-byte installed history. It retained one work record rather than transmitting the remaining 499 payloads. The headless test independently verifies zero mounted folded work rows at 620, 960, 1280 and 1920 px, then loading and retry states. The fixture loads all 500 records after an explicit work request, reconnects during the cursor walk, injects a recoverable request failure and retries, and checks protocol-10 inline fallback. Additional unit tests cover the 150/151 boundary, reasoning and tool kinds, persistent and conversation records, failed/active/unknown/stopped outcomes, separate turns, malformed cursors and deletion of a cursor row.

The final verification also added a failing regression for equal thread IDs on different machines. Rune retained the old scroll node when only the server changed. `conversation` now adds a server-scoped parent without changing any existing keys. The regression verifies an unseen server opens at the end and returning to the original server restores its row and offset at all four widths.

## Verification limits

All process fixtures use temporary profiles. No installed app, user process, real Shina profile or CPA service was changed. Native Windows/Linux builds, manual GUI interaction, real providers and two separately compiled historical binaries remain unverified. Version-skew tests negotiate older capabilities against the fixture server. Legacy histories without known completion outcomes intentionally remain inline. The native synchronous scroll restore and flat work-section model differ from KSG's browser frame settling and run-attempt projection as described above.
