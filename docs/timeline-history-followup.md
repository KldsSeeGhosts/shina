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
