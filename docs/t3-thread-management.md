# How T3 Code manages threads in its sidebar

This spec describes the thread sidebar in T3 Code, as forked in KSG Code at `D:\AiStack\ksgcode`. Every claim cites the code it was read from. Claims marked **(inferred)** come from reading around the code or from the absence of code, not from a line that states them.

Path prefixes used below:

- `web/` is `apps/web/src/`
- `cr/` is `packages/client-runtime/src/`
- `server/` is `apps/server/src/orchestration-v2/`
- `contracts/` is `packages/contracts/src/`

The sidebar described here is the default one. Two other modes exist: the "boring sidebar" (project folders, setting `boringSidebarEnabled`, default off, `contracts/settings.ts:479`) and a retired legacy sidebar (`contracts/settings.ts:490-492`). Boring-mode differences are noted where they matter.

## 1. Data model

### Thread fields that drive the sidebar

The client works from an `EnvironmentThreadShell`, built from the server's `OrchestrationV2ThreadShell` by `presentThreadShell` (`cr/state/models.ts:257-340`). The fields that decide placement and appearance:

| Field | Meaning | Source |
|---|---|---|
| `environmentId` + `id` | Thread identity. Ids are unique only within an environment, so every key is `environmentId:threadId`. | `cr/state/threadSort.ts:268-270`, `web/components/Sidebar.tsx:370-372` |
| `projectId` | Owning project, scoped to the environment. | `cr/state/models.ts:140` |
| `archivedAt` | Archived threads never appear in the sidebar. | `web/components/Sidebar.logic.ts:627-629` |
| `settledOverride` | `"settled"`, `"active"` or `null`. Only `"settled"` puts a row in the Settled shelf. `"active"` is a sticky "user un-settled this" flag that blocks auto-settle. | `cr/state/models.ts:175`, `web/components/Sidebar.tsx:683`, `server/ThreadSettlementService.ts:150` |
| `settledAt` | When it settled. Sorts the Settled shelf and drives the older-settled paging cut. | `cr/state/threadSort.ts:26-47`, `server/ShellStream.ts:102-112` |
| `unsettledAt` | When it last left Settled. Anchors Active sorting. | `cr/state/threadSort.ts:116-124` |
| `snoozedUntil`, `snoozedAt` | Wake time and when the snooze was set. | `cr/state/models.ts:178-179` |
| `lastSnoozeWakeAt` (server only) | When a snooze last ended. Counts as activity for auto-settle. | `contracts/orchestrationV2.ts:487`, `server/ThreadSettlementService.ts:239-240` |
| `pinnedAt`, `pinOrderKey` | Pinned flag and fractional sort key inside Pinned. | `cr/state/models.ts:181-183` |
| `activeOrderKey` | Manual sort key inside Active, used only when the Working shelf is off. | `cr/state/models.ts:184`, `web/components/Sidebar.tsx:698-702` |
| `autoSettleDisabledAt` | Per-thread opt-out of auto-settle. | `cr/state/models.ts:182`, `server/ThreadSettlementService.ts:151` |
| `lastVisitedAt` | Server read watermark. Drives unread ("Done") and "Woke". `undefined` means an old server, and the client falls back to a local watermark. | `cr/state/models.ts:188-193`, `web/components/Sidebar.logic.ts:712-720` |
| `latestRun` (`status`, `requestedAt`, `startedAt`, `completedAt`) | Last run summary. Drives status, unread and sort times. | `cr/state/models.ts:263-278` |
| `runtime` (`status`, `activeRunId`, `lastError`, `lastErrorClass`) | Live execution state. | `cr/state/models.ts:223-246` |
| `hasPendingApprovals`, `hasPendingUserInput`, `pendingSubagentRequest`, `pendingGuestMessageCount`, `hasActionableProposedPlan`, `pendingBackgroundTasks`, `blockedOnThreadWait` | Attention and waiting states. | `cr/state/models.ts:156-167`, `cr/state/models.ts:309-318` |
| `latestUserMessageAt`, `latestUserAuthoredMessageAt` | The second excludes automatic wakes and agent messages. | `contracts/orchestrationV2.ts:2113-2115` |
| `lineage` (`parentThreadId`, `relationshipToParent` = `fork` or `subagent`, `rootThreadId`, `sideChat`) | Subagents and side chats are never listed. | `contracts/orchestrationV2.ts:154-164` |
| `delegatedFrom` | A task a linked environment runs for a thread here. Also hidden from the list. | `cr/state/models.ts:151-152`, `cr/state/threadShell.ts:64-70` |
| `pullRequests`, `linkedPullRequest`, `branchPullRequest`, `branch`, `worktreePath` | PR badge, branch line and PR-based auto-settle. | `cr/state/models.ts:146-147`, `cr/state/models.ts:185-187` |
| `titleRegeneration` | Dims the title while a new one is generated. | `web/components/sidebar/SidebarThreadRow.tsx:665`, `:1039` |
| `tags` | Threads with hidden tags drop out when `hideTaggedThreads` is on. | `contracts/orchestrationV2.ts:494-495`, `contracts/settings.ts:1293-1304`, `cr/state/threadShell.ts:698-705` |

There is no `unread` boolean. Unread is computed: the latest run completed after `lastVisitedAt` (`web/components/Sidebar.logic.ts:722-731`). "Mark unread" rewinds `lastVisitedAt` to one millisecond before the latest completion, and the server refuses it when no run has completed (`server/Orchestrator.ts:3518-3536`). The first real message to a thread with `lastVisitedAt === null` sets the watermark to one millisecond before now, so imported history stays read and the first completion shows as unread (`server/Orchestrator.ts:5561-5580`).

### Where each piece is stored

Server, in the SQLite projection (`payload_json`, read with `json_extract`, for example `server/ProjectionStore.ts:3999`): every field in the table above. All lifecycle changes are commands that the server validates and turns into events (`server/Orchestrator.ts:3100-3700`). The client applies them optimistically and drops the optimistic copy once the server snapshot reaches the command's sequence (`cr/state/threadLifecycle.ts:18-102`, optimistic shapes in `cr/state/threadCommands.ts:377-454`).

Server settings (shared by all clients of that server): `sidebarAutoSettleAfterDays` (default 3), `sidebarAutoSettleOnMerge` (default true), `sidebarAutoSettleKeepPullRequests` (default false), per-project overrides of those three, and `snoozePresets` (`contracts/settings.ts:87`, `:1281-1292`, `:1312-1315`, `:1118-1120`).

Client local storage:

- Shelf expansion: `t3code:sidebar:settled-expanded`, `t3code:sidebar:snoozed-expanded`, `t3code:sidebar:working-expanded`, all default `false` (`web/components/Sidebar.tsx:331-333`, `:464-478`).
- UI state store: `projectExpandedById` (folder open state), `projectOrder`, `threadLastVisitedAtById` (fallback watermark for old servers), `sidebarProjectScopeKeys` (the project filter) (`web/uiStateStore.ts:22-62`, `:213-230`).
- Sidebar width under `THREAD_SIDEBAR_WIDTH_STORAGE_KEY`, and open/closed in a `sidebar_state` cookie with a 7-day lifetime (`web/components/AppSidebarLayout.tsx:461-469`, `web/components/ui/sidebar.tsx:26-27`, `:109-110`).
- Client settings: `sidebarWorkingShelfEnabled` (default true), `boringSidebarEnabled`, `sidebarThreadSortOrder` (default `updated_at`), `sidebarProjectSortOrder`, `confirmThreadArchive` (false), `confirmThreadDelete` (true), `confirmThreadUnpin` (false) (`contracts/settings.ts:419-421`, `:479-506`). Where the client persists these was not checked **(inferred: local, per client)**.
- Unsent drafts live in the client's composer draft store, not on the server, until first send (`web/components/sidebar/SidebarDraftBlock.tsx:150-203`).

## 2. Sections

### Order on screen

The list is built as a flat array of rows and markers (`web/components/Sidebar.tsx:1924-1965`):

1. Draft block (unsent drafts), then a divider (`web/components/Sidebar.tsx:3429-3438`, `web/components/sidebar/SidebarDraftBlock.tsx:204-231`)
2. `pinned-header` marker, Pinned rows
3. `pinned-divider` marker, `active-placeholder` marker, Active rows
4. `working-header` and Working rows, only if any thread is working
5. `snoozed-header` and Snoozed rows, only if any thread is snoozed
6. `settled-header`, `settled-placeholder`, Settled rows, then a "Show N more" button

The Pinned and Active labels are invisible except during a drag (`SidebarDragBoundary` with `visible={from !== null}`, `web/components/Sidebar.tsx:3473-3494`). So at rest Pinned and Active read as one continuous list. Working, Snoozed and Settled have real collapsible headers with a count (`web/components/sidebar/SidebarListMarkers.tsx:110-144`). The first present shelf header gets `mt-auto`, which pushes the shelves to the bottom of the sidebar (`web/components/Sidebar.tsx:3518`, `:3533`, `:3548-3550`).

If no thread exists in any section, the list is empty and an empty state shows instead (section 10).

### Membership rules

Before partitioning, `filterSidebarV2VisibleThreads` keeps a thread only if it is not archived, not a subagent or side chat, not a linked-environment task shown under its parent, and inside the project scope (`web/components/Sidebar.logic.ts:618-638`). Threads with hidden tags were already removed upstream (`cr/state/threadShell.ts:698-705`).

Each remaining thread then lands in exactly one section, decided in this priority (`web/components/Sidebar.logic.ts:177-186`, applied at `web/components/Sidebar.tsx:681-693`):

1. **Snoozed** if `effectiveSnoozed` is true and the environment's server config is loaded.
2. **Settled** if `settledOverride === "settled"` and the config is loaded.
3. **Pinned** if `pinnedAt != null`.
4. Otherwise **Working** if it belongs in the Working shelf, else **Active**.

Snoozed outranks Settled, and both outrank Pinned. On the server, settling clears pin and snooze, and pinning clears settle and snooze, so a thread is normally in only one of these states (`server/Orchestrator.ts:3550-3627`).

`effectiveSnoozed` is true when `snoozedUntil` parses, is in the future, and the thread has not "raised its hand" (`cr/state/threadSettled.ts:137-146`). Raising a hand means a pending approval, a pending user-input question, a pending runtime request, a pending subagent request, a waiting guest message, or a failure newer than `snoozedAt` (`cr/state/threadSettled.ts:86-97`). A completed run never wakes a snoozed thread early (`cr/state/threadSettled.ts:81-85`).

**Working shelf.** A thread goes to Working when the shelf is enabled, its environment is connected with a live shell, and `isWorking` is true (`web/components/Sidebar.logic.ts:196-209`). The shelf is enabled when the client setting `sidebarWorkingShelfEnabled` is on and boring mode is off (`web/components/Sidebar.tsx:442-444`). `isThreadWorking` is false while the thread has a pending approval, question or subagent request; false unless the runtime is active or `idle` (idle with awaited background work); and false for a plan-mode thread whose run finished with an actionable plan (`cr/state/threadInbox.ts:17-31`). So a thread that needs the user always leaves Working and shows in Active.

A thread that stops working stays in Working for a 2-second grace period, because an orchestrator run can end just before its continuation starts (`cr/state/threadInbox.ts:110-115`). The grace is skipped if the thread became actionable (approval, question, guest message, actionable plan, error, failed run), was moved by the user (its pin, settle, snooze, archive or order fields changed), or is now pinned, settled, snoozed, archived or deleted (`cr/state/threadInbox.ts:188-224`). When a thread finally leaves Working, the tracker records a "returned at" time, which then sorts it to the top of Active (`cr/state/threadInbox.ts:223`, `:38-55`). The tracker lives at module scope so this survives remounts (`cr/state/threadInbox.ts:117-121`, `web/components/Sidebar.tsx:335`).

**Pinned** has no extra rules. A pinned thread that is running stays in Pinned, not Working (`web/components/Sidebar.tsx:686-693`).

**Settled** shows the local settled threads plus older ones paged from the server. The server keeps the newest 25 settled top-level threads in the shell and omits older ones when the client asks with `omitOlderSettled`, sending per-project counts of what it left out (`server/ShellStream.ts:92-127`, `contracts/orchestrationV2.ts:2182-2210`). The shelf shows 10 rows at first, and each "Show more" adds 25 (`web/components/Sidebar.tsx:329-330`, `:593-597`). Older pages are read 25 at a time, only while the shelf is expanded (`cr/state/olderSettledThreads.ts:25-26`, `web/state/olderSettledShelf.ts:24-99`). The header count is local settled threads plus the unloaded older count (`web/components/Sidebar.tsx:515-516`).

**Collapsed shelves.** All three shelves start collapsed. A collapsed shelf renders only the currently open thread, if it is in that shelf (`web/components/Sidebar.tsx:530-538`, `routeThreadOnly` at `:406-413`). The Settled tail also always includes the open thread even past the visible count (`web/components/Sidebar.tsx:517-529`).

### Project scope

The project filter is a multi-select of logical project groups. "All projects" is the empty selection. Shift+click or Shift+Enter toggles one project into the selection, and a plain click selects only that one (`web/components/Sidebar.tsx:3013-3031`, `:3095-3099`). The selection persists in `sidebarProjectScopeKeys` (`web/components/Sidebar.tsx:1027-1028`, `:1089-1093`). Changing it clears the multi-selection and scrolls the list to the top (`web/components/Sidebar.tsx:1087-1093`). Archived projects cannot be scope choices, and a scope whose project is gone reads as all projects once every snapshot has loaded, without deleting the stored keys (`web/components/Sidebar.tsx:1094-1106`).

The scope filters thread rows, the draft block, the older-settled pages and server message search (`web/components/Sidebar.logic.ts:635-636`, `web/components/sidebar/SidebarDraftBlock.tsx:183-188`, `web/components/Sidebar.tsx:735-749`, `:1201-1207`). If the user opens a thread (from history, a link or the palette) that the scope hides, the scope widens to all projects and that is written back to storage (`web/components/Sidebar.tsx:1136-1165`).

The context menu has "Filter by <project>" and "Show all projects" (`web/components/threadActionMenu.logic.ts:184-194`, `web/components/Sidebar.tsx:2677-2685`).

### Multi-environment behavior

Threads from all connected environments merge into the same sections. Pinned, Working and Active tie-break by id, then environment id, so every client renders the same order (`cr/state/threadSort.ts:268-295`, `cr/state/threadInbox.ts:79-89`).

A project group can span environments (`web/components/Sidebar.logic.ts:553-559`). When groups span environments, project rows in the filter show an environment badge, and threads in a scratch ("No project") project show the machine label (`web/components/Sidebar.tsx:1167-1180`, `:3151-3157`). A thread from a non-primary environment shows a machine icon on its row (`web/components/sidebar/SidebarThreadRow.tsx:802`, `:1127-1132`).

Offline rules:

- A thread whose environment config has not loaded cannot be Snoozed or Settled for placement, and falls to Pinned or Active (`web/components/Sidebar.tsx:661`, `:681-685`).
- A disconnected environment's threads never enter Working, so they show in Active (`web/components/Sidebar.tsx:447-456`, `web/components/Sidebar.logic.ts:188-209`). The first connection attempt keeps cached placement (`web/components/Sidebar.logic.ts:188-194`).
- Rows are draggable only when the environment is connected and its config is present (`web/components/Sidebar.logic.ts:343-367`).
- Settle, snooze and pin controls require a loaded config (`web/components/Sidebar.tsx:3354-3356`, `web/hooks/useThreadActions.ts:654-664`).

## 3. Settle

### What auto-settles a thread

Auto-settle runs on the server in `ThreadSettlementServiceV2`. It does nothing unless auto-settle is configured globally or for some project (`server/ThreadSettlementService.ts:254-263`, `:314-317`).

It sweeps all candidates every minute, on settings changes, and on every PR merge event (`server/ThreadSettlementService.ts:684-700`). It also sweeps a single thread when that thread's work may have ended: a run completed, interrupted, failed, was cancelled or rolled back; a provider session stopped; background work ended; a subagent finished; or a PR link changed or synced (`server/ThreadSettlementService.ts:628-674`).

A thread is a candidate only if all of these hold (`server/ThreadSettlementService.ts:146-160`):

- not archived, and `settledOverride === null` (so a user un-settle, which sets `"active"`, blocks auto-settle)
- not pinned, and `autoSettleDisabledAt` is null
- no pending runtime request and no waiting guest message
- no activity run, and no awaited background work
- no queued turn start, meaning no user message from the last 2 minutes still waiting for its run (`server/ThreadSettlementService.ts:98-121`)
- not snoozed, by the same rule the client uses (`server/ThreadSettlementService.ts:162-186`)

A candidate then settles by one of two rules (`server/ThreadSettlementService.ts:188-244`):

1. **PR finished.** The relevant pull request is closed, or merged while `autoSettleOnMerge` is on, and it finished at or after the later of thread creation and the last user-authored message (`server/ThreadSettlementService.ts:123-144`). Any open linked PR disables this rule. Links added after the PR finished are ignored, and a link with no snapshot yet blocks settling entirely (`server/ThreadSettlementService.ts:87-96`, `:198-217`). For threads without explicit links, the server looks up the PR for the thread's branch (`server/ThreadSettlementService.ts:367-544`).
2. **Inactivity.** The last activity (latest user message, run requested, started or completed, last snooze wake, or a snooze timer that already passed) is older than `autoSettleAfterDays` days. If `autoSettleKeepPullRequests` is on, threads with any PR never settle this way (`server/ThreadSettlementService.ts:229-243`).

The recorded `settledAt` is the thread's last activity time, not the time of the sweep (`server/ThreadSettlementService.ts:219-228`, `:241-242`). The server dispatches `thread.auto-settle` with a snapshot time, and the orchestrator rejects it if the thread changed since (`server/ThreadSettlementService.ts:345-351`, `server/Orchestrator.ts:13108-13137`).

Nothing settles a thread merely because a run completed. Completion only triggers a sweep, and the sweep still needs a finished PR or the inactivity window **(inferred from the two rules above; there is no other settle path)**.

### What happens on settle

Manual settle (`thread.settle`) is rejected while the thread has a pending approval or a non-message question, a running run, or a queued message the user sent (`server/Orchestrator.ts:3326-3368`). It cancels pending message-style questions and queued automatic runs (`server/Orchestrator.ts:3370-3393`). It then sets `settledOverride = "settled"`, keeps an existing `settledAt` or sets it to now, clears `unsettledAt`, pin, pin key, active key and snooze, and stops PR watches (`server/Orchestrator.ts:3550-3570`). It detaches live provider sessions (`server/Orchestrator.ts:4178-4255`). Afterwards it closes the thread's idle terminals and runs the project's "settle" script in the thread's worktree, once per settlement (`server/ThreadSettlementService.ts:590-626`).

The client refuses to optimistically show a settle while the thread is running or has a pending request, and otherwise mirrors the server change (`cr/state/threadCommands.ts:381-398`).

If the settled thread is the one on screen, the client navigates to the next row in sidebar order that is not settled or snoozed, wrapping around. If none exists it opens a new thread draft in the same project, and failing that the home page (`web/components/Sidebar.tsx:1594-1617`, `web/components/Sidebar.logic.ts:47-61`). Settling also marks a "Woke" notification as seen (`web/hooks/useThreadActions.ts:759-777`).

### What un-settles a thread

- **Manual un-settle** (row button, context menu, drag out of Settled, Undo) sends `reason: "user"`. The server sets `settledOverride = "active"`, clears `settledAt` and stamps `unsettledAt` (`server/Orchestrator.ts:3571-3580`, `web/hooks/useThreadActions.ts:654-671`). The `"active"` value keeps auto-settle away until real activity.
- **A new message.** Dispatching any message to a thread with a non-null override clears it to `null` and stamps `unsettledAt` (`server/Orchestrator.ts:5586-5609`). This applies to automatic wakes too; only the snooze check below exempts them **(read: the settle-clearing block has no automatic-wake exception)**.
- **Pinning** a settled thread un-settles it (`server/Orchestrator.ts:3609-3627`).

### Settle and archive

They are separate. Archived threads are invisible in the sidebar, cannot be settled, snoozed, pinned or reordered, and are never auto-settled (`web/components/Sidebar.logic.ts:629`, `server/Orchestrator.ts:3254-3272`, `server/ThreadSettlementService.ts:150`). Archive is refused while a run is active (`cr/state/models.ts:118-125`, `web/components/threadActionMenu.logic.ts:84-85`). Archive does not change `settledOverride`, so an unarchived thread returns to the section its fields describe **(inferred from `server/Orchestrator.ts:3539-3549`, which touches only `archivedAt` and related fields)**.

### Per-thread opt-out

The context menu has "Auto-settle behavior" with Enabled and Disabled, shown when the server supports it (`web/components/threadActionMenu.logic.ts:205-225`). Disabled sets `autoSettleDisabledAt` (`server/Orchestrator.ts:3601-3608`).

## 4. Snooze

### Options

The menu lists the server's `snoozePresets`, defaulting to (`contracts/settings.ts:119-125`):

- after 60 minutes
- after 180 minutes
- today at 18:00
- tomorrow at 09:00
- Monday at 09:00

Users can configure up to 8 presets of two kinds: "after N minutes" (1 minute to 30 days) and "at a local time on today, tomorrow or a weekday" (`contracts/settings.ts:89-118`). A "Custom…" entry is always last, after a separator (`web/components/threadActionMenu.logic.ts:146-167`, `web/components/sidebar/SidebarThreadRow.tsx:403-427`).

### Wake time and labels

`resolveSnoozePreset` computes the wake time (`cr/state/threadSettled.ts:253-282`):

- "After" presets add the minutes to now.
- "At" presets advance whole calendar days, not 24-hour blocks, so DST changes land on the right day, then set the local time. A weekday is its next occurrence after today, and 7 days ahead if today is that weekday (`cr/state/threadSettled.ts:202-209`, `:226-233`).
- A "today" preset whose time has passed is left out (`cr/state/threadSettled.ts:266-267`).

Labels (`cr/state/threadSettled.ts:215-245`, `:270-281`):

- Durations read "In 1 hour", "In 3 hours" or "In 90 minutes", using the largest unit that divides evenly.
- Today presets read "This morning", "This afternoon" or "This evening", for hours before 12, before 17, and after.
- A 1-day gap reads "Tomorrow", Monday reads "Next week", and other weekdays read their name, or "Next <Day>" when 7 days away.
- Each item also shows the wake time, plus a short weekday when the label does not name the day and the wake is not today. The row menu shows it as `label` with a right-aligned time, and the context menu as `label (time)`.

The menu sorts by wake minute and drops duplicates (`cr/state/threadSettled.ts:284-304`). Picking a duration re-resolves it from the moment of the pick. Picking a set time keeps the time shown, and if that time has passed the client shows "That snooze time has passed. Choose another time." (`cr/state/threadSettled.ts:306-324`, `web/components/Sidebar.snooze.ts:27-47`).

Custom snooze accepts a date and time, or an amount in minutes, hours or days. It rejects times that are not in the future, dates that roll over and nonexistent DST times (`cr/state/threadSettled.ts:337-369`).

### When snooze is allowed

The client offers snooze only when there is no pending approval, question, runtime request, subagent request or guest message, and no queued turn start from the last 2 minutes (`cr/state/threadSettled.ts:111-135`). The server additionally rejects a wake time that is not in the future, any pending request, and any queued run the user sent (automatic queued runs are allowed) (`server/Orchestrator.ts:3471-3517`). The row's snooze button reappears on its own when the 2-minute gate expires (`web/components/sidebar/SidebarThreadRow.tsx:904-934`).

Snoozing sets `snoozedUntil` and `snoozedAt` to now. Re-snoozing counts as acknowledging the latest wake (`server/Orchestrator.ts:3581-3590`). Snoozing the open thread navigates away like settle does (`web/components/Sidebar.tsx:2221-2252`).

### Waking

The server has no wake timer. `snoozedUntil` stays stored after it passes, and the client treats the thread as awake once the time passes (`cr/state/threadSettled.ts:141-144`) **(inferred: no server code clears `snoozedUntil` at its wake time; the only writers are the commands listed here)**. To move a woken thread out of the shelf exactly on time, the client advances a clock to the next wake time and repartitions (`web/components/Sidebar.logic.ts:126-155`, `web/components/Sidebar.tsx:461-463`, `:502`).

A snooze ends early, without clearing the stored fields, when the thread raises its hand (approval, question, subagent or guest request, or a new failure) (`cr/state/threadSettled.ts:86-97`). A snooze is cleared outright by:

- the user waking it: `thread.unsnooze`, which stamps `lastSnoozeWakeAt` (`server/Orchestrator.ts:3591-3600`)
- a message someone actually sent. Background wakes and delegated completions keep the snooze (`server/Orchestrator.ts:5610-5633`)
- settling or pinning (`server/Orchestrator.ts:3563-3564`, `:3622-3624`)
- mark-unread after the wake time passed, which also records the wake (`server/Orchestrator.ts:3663-3675`)

A related server setting, `snoozeLimitedThreads`, snoozes a thread until its usage-limit reset time (`contracts/settings.ts:1288`, `server/Orchestrator.ts:3698-3705`).

### "Woke" and inbox returns

`threadWokeAt` returns the wake time: the scheduled time once passed, or for an early wake, the failure time or the runtime's update time (`cr/state/threadSettled.ts:148-172`). A row shows a **Woke** badge with an alarm-clock icon while `wokeAt` is later than `lastVisitedAt` and the thread is not settled (`web/components/sidebar/SidebarThreadRow.tsx:711-716`, `:767-772`). Clicking the badge dismisses it by recording a visit at `wokeAt` (`web/components/sidebar/SidebarThreadRow.tsx:826-831`, `web/hooks/useThreadActions.ts:220-233`). Opening the thread also clears it, because the visit tracker moves the watermark to the wake time at once (`cr/state/threadVisits.ts:25-31`, `:99-104`, `:133-135`).

"Inbox return" is the Working-shelf behavior from section 2. A thread that leaves Working gets a return time and sorts to the top of Active (`cr/state/threadInbox.ts:38-55`, `:223`). A woken snoozed thread rises in Active through `unsettledAt` and its latest run times, which are part of the same sort key **(inferred: a snooze wake itself does not feed `sortInboxThreadsByReturn`; only the listed timestamps and the tracker's return time do)**.

## 5. Sort order and drag and drop

### Sort within each section

| Section | Order | Source |
|---|---|---|
| Pinned | Rows with a `pinOrderKey` in ascending key order, then keyless rows newest `createdAt` first. Ties break by id, then environment. | `cr/state/threadSort.ts:263-296` |
| Active, Working shelf on (default) | Newest first by the latest of `createdAt`, `unsettledAt`, latest run `requestedAt`, latest run `completedAt` and the tracker's return time. | `cr/state/threadInbox.ts:38-55`, `web/components/Sidebar.tsx:700-701` |
| Active, Working shelf off | Rows without `activeOrderKey` first, newest by `max(createdAt, unsettledAt)`, then keyed rows by key. | `cr/state/threadSort.ts:298-331`, `web/components/Sidebar.tsx:702` |
| Active in boring mode folders | Newest first by `max(createdAt, latestUserAuthoredMessageAt)`, falling back to run `requestedAt`. | `cr/state/threadInbox.ts:62-77`, `web/components/Sidebar.tsx:698-699` |
| Working | Same "by send" order as boring folders. | `web/components/Sidebar.tsx:722` |
| Snoozed | Soonest `snoozedUntil` first. | `web/components/Sidebar.tsx:723-727` |
| Settled | Newest first by `settledAt`, falling back to the latest of user message and run times, then `updatedAt`. Ties use plain id order to match server paging. | `cr/state/threadSort.ts:21-64`, `web/components/Sidebar.tsx:728` |

The `sidebarThreadSortOrder` setting (`updated_at` or `created_at`) is used by `sortThreads`, which picks the fallback thread after a delete, not by section sorting (`cr/state/threadSort.ts:104-139`, `web/components/Sidebar.logic.ts:1213-1243`).

Projects in the filter and in boring folders sort by their newest thread's timestamp, unless the project sort order is `manual` (`web/components/Sidebar.logic.ts:1244-1339`).

### Pin order keys

Keys are base-26 strings over `a` to `z`, compared as fractions. A new key is the midpoint between its neighbors. Keys ending in `a` are treated as corrupt, and when a midpoint cannot be made the whole section is rewritten with evenly spread keys (`cr/state/threadSort.ts:165-261`). Pinning from a menu or shortcut puts the thread at the top of the keyed pinned run (`web/hooks/useThreadActions.ts:111-118`, `:701`).

### Drag and drop

Drag starts after the pointer moves 6 pixels, and is limited to the vertical axis and to below the Pinned label (`web/components/Sidebar.tsx:898`, `:1696-1702`). Rows cannot be dragged in boring mode, while renaming, when offline, without operate permission, or while a previous drop is still waiting for the server (`web/components/Sidebar.tsx:3331-3336`, `web/components/sidebar/SidebarThreadRow.tsx:456-466`).

The drop section is found by inserting the row at the hovered index and reading the nearest marker above it (`web/components/Sidebar.logic.ts:234-281`). Working and Snoozed accept no drops, because a snoozed row needs a wake time (`web/components/Sidebar.logic.ts:173`, `:264`). Working rows do not reorder at all, and dragging them only works as "drag out as chat context" (`web/components/Sidebar.logic.ts:397`, `web/components/Sidebar.tsx:1728`).

Drop semantics, from `planSidebarThreadDrop` (`web/components/Sidebar.logic.ts:369-497`) and the executor (`web/components/Sidebar.tsx:2081-2220`):

| From | To | Effect | Badge |
|---|---|---|---|
| Pinned | Pinned | Reorder: write a new `pinOrderKey`. | none |
| Active, Settled or Snoozed | Pinned | Pin at that position. Pinning also clears settle and snooze. | Pin |
| Pinned | Active | Unpin. With the Working shelf off, also write `activeOrderKey`. | Unpin |
| Settled | Active | Un-settle. | Un-settle |
| Snoozed | Active | Wake. | Wake |
| Active | Active | Reorder only when the Working shelf is off. With it on, Active is time-ordered and the drop does nothing. | none |
| any except Settled | Settled | Settle. | Settle |

The badge names are from `web/components/Sidebar.logic.ts:306-318` and `web/components/sidebar/SidebarThreadRow.tsx:486-517`. While dragging, empty Active or Settled sections show a dashed drop placeholder (`web/components/sidebar/SidebarListMarkers.tsx:48-73`, `web/components/Sidebar.tsx:3495-3512`).

The dropped row is held in its projected place until the server's state catches up or the move fails (`web/components/Sidebar.tsx:1039-1040`, `:751-820`, `:1853-1865`).

Dragging a row past the list's side edge turns it into a context drag. Dropping it on the chat attaches the thread, or all selected threads, as context (`web/components/Sidebar.tsx:1745-1780`).

Files dragged onto a row open that thread and queue the files for its composer (`web/components/Sidebar.tsx:1469-1489`).

### Action sweep

Pressing on a row's Settle, Un-settle or Wake button and dragging across other rows applies the same action to every row between them in the same section (`web/components/sidebar/SidebarThreadRow.tsx:881-885`, `web/components/Sidebar.tsx:1800-1852`, `web/components/Sidebar.logic.ts:320-330`).

## 6. Row anatomy

### Variants

Pinned, Active and Working rows use the **card** variant. Snoozed and Settled rows use the **slim** variant. Boring-mode folder rows use **folder**, or **folder-multiline** when that setting is on (`web/components/Sidebar.tsx:3317-3325`). The row's main action button follows its section: Wake for Snoozed, Un-settle for Settled, Settle otherwise (`web/components/Sidebar.tsx:3347-3353`).

### Card (about 78 pixels tall)

Line 1, left to right (`web/components/sidebar/SidebarThreadRow.tsx:1430-1574`):

- unsent-draft pen icon, when the thread has an unsent draft and is not open (`:676`, `:1075-1091`)
- project favicon
- pin icon, when pinned. It is an Unpin button whose icon becomes pin-off on hover (`:1092-1125`)
- project display name, muted and truncated
- status slot, right-aligned:
  - The top status when there is one: an icon and label, plus a live duration for Working (since start, ticking every second) and Waiting (since the last completion) (`:1466-1509`, `:163-176`).
  - Otherwise the relative time of `latestUserMessageAt ?? updatedAt`, shortened ("now" for "just now", " ago" dropped) and updated each minute (`:132-150`).
  - On hover or keyboard focus, the slot swaps to the row controls: Discard draft (X, only with a draft), the Snooze clock menu (only if snooze is allowed now), and a "Settle" button with a check icon (`:1514-1569`). The Woke badge stays visible and clickable on hover (`:1457-1461`).
  - Jump-hint key cap (for example `Ctrl+1`) while the jump modifier is held (`:1570`).

Line 2: the title, medium weight, truncated, dimmed while a title is regenerating. When line 3 would be empty, the provider icons move to this line (`:1007-1045`, `:1575-1583`, `:1182-1190`).

Line 3, the meta line, shown only if it has content (`:1133-1181`):

- branch name with a worktree indicator, or for a scratch thread the machine label
- running-terminal status button
- PR badge: number, state color, or a stack badge for several PRs. Clicking opens the PR, or the PR list in the right panel (`:1048-1067`, `:936-944`)
- diff counts `+N −M` from the latest run
- remote-machine icon and provider icon stack. Earlier providers from handoffs show smaller and grayscale (`:182-228`)

### Top status values

`resolveSidebarThreadStatus` picks one status, in this priority (`web/components/Sidebar.logic.ts:882-909`):

1. approval (own or subagent)
2. input (own or subagent question)
3. request (guest message waiting)
4. waiting (running but blocked on another thread)
5. working (preparing, queued, starting, running or waiting)
6. waiting (idle runtime)
7. failed, or limited if the error class is `usage_limit`
8. ready

The card shows (`web/components/sidebar/SidebarThreadRow.tsx:724-779`):

| Status | Label | Color |
|---|---|---|
| working | Working, or "Goal" when a goal is active | info |
| waiting | Waiting | muted |
| approval | Approval | warning |
| input | Input | indigo |
| request | Request | guest |
| limited | Limited | warning |
| failed | Failed | error |
| ready, woke unseen | Woke | warning |
| ready, unread | Done | success |
| ready, read | nothing; the relative time shows | none |

A row recedes (normal weight, muted text, and 70% opacity for working rows) when it is working or waiting, or when it is ready or awaiting approval and neither unread nor woke. Active, selected, input and request rows never recede (`web/components/Sidebar.logic.ts:850-870`, `web/components/sidebar/SidebarThreadRow.tsx:946-967`).

### Slim (36 pixels)

From `web/components/sidebar/SidebarThreadRow.tsx:1192-1388`:

- project favicon, at 40% opacity and grayscale until hover or focus
- draft icon
- title, colored by state: muted when receded, full color for active, woke, input or request, and dimmer for read rows
- pin icon
- terminal icon
- right slot: a wake countdown with an alarm icon for Snoozed (`5m`, `3h`, `2d`, from `cr/state/threadSettled.ts:326-335`); the Woke badge; or the relative time, where Settled rows show the settled time instead (`:1276-1320`)
- hover: Wake now (alarm-off icon), Un-settle (undo icon) or Settle (check icon) (`:1322-1377`)

Settled rows are additionally muted at rest (`:948-950`).

### Hover card

Hovering a row opens a card to the right with the title, project, environment and machine, branch, a "checked out on another branch" warning, model and provider, "Handed off from …", running terminals, "Usage limit reached" or "Error occurred", and the PR list (`web/components/sidebar/SidebarThreadRow.tsx:230-367`).

### Subagents

Subagent and side-chat threads have no rows (`contracts/orchestrationV2.ts:162-164`). When one is open, its nearest listed ancestor's row is highlighted instead (`web/components/Sidebar.logic.ts:594-612`, `web/components/Sidebar.tsx:1121-1133`). A subagent's pending approval flags every ancestor up the subagent chain, so the parent row shows Approval. A subagent's question does not roll up, because "the coordinator answers child questions; only approvals need user attention". Side-chat questions do roll up (`cr/state/threadShell.ts:88-140`). The chat header's breadcrumb links back to the parent (section 9).

### Draft block

Drafts that have never been sent appear above everything else, newest first, as card-sized rows with a warning tint. Each shows a pen icon, the project favicon and name, the first line of the prompt (or "N attachments"), and a Discard button on hover. A divider follows the block (`web/components/sidebar/SidebarDraftBlock.tsx:25-232`). The draft context menu has Copy (path, branch), Project settings and "Discard draft" (`web/components/threadActionMenu.logic.ts:39-68`).

### Context menu (single thread)

From `web/components/threadActionMenu.logic.ts:116-268`, in order:

1. Find in thread (only from the chat header)
2. New thread on `<branch>` (if the thread has a branch)
3. Pin thread, or Unpin thread
4. Settle thread, or Un-settle thread
5. Snooze, with the presets and Custom…, disabled when snooze is not allowed. Or Wake thread when snoozed.
6. (separator) Rename thread
7. Regenerate title, which reads "Regenerating…" and is disabled while running
8. Mark unread
9. Share with guest (when allowed)
10. Filter by `<project>`, or Show all projects
11. Turn into project (for a "No project" thread; disabled while running)
12. Auto-settle behavior, with Enabled and Disabled
13. (separator) Copy, with Path, Branch, Thread and Thread ID
14. Project settings
15. (separator) Archive thread (disabled while running)
16. Delete (destructive)

Without operate permission, everything except navigation, mark unread, sharing and copying is disabled (`web/components/threadActionMenu.logic.ts:100-114`, `:255-267`). Archive confirms only if `confirmThreadArchive` is on, and Delete confirms by default (`web/components/Sidebar.tsx:2820-2860`).

### Multi-select

Ctrl+click (Cmd+click on Mac) toggles a row into the selection, and Shift+click selects a range in sidebar order (`web/components/Sidebar.tsx:1573-1592`). Right-clicking a selected row opens the bulk menu: Unpin (n), Settle (n), Snooze (n) with a submenu, Regenerate titles (n), Mark unread (n), and Delete (n) with a confirmation (`web/components/Sidebar.tsx:2274-2531`).

### Keyboard and mouse

| Input | Action | Source |
|---|---|---|
| Click, Enter or Space | Open the thread | `web/components/sidebar/SidebarThreadRow.tsx:836-850` |
| Double-click or F2 | Rename inline. Enter commits, Escape cancels, blur commits, and an empty title is rejected. | `web/components/sidebar/SidebarThreadRow.tsx:592-630`, `:851-865`, `web/components/Sidebar.tsx:1540-1571` |
| Mod+Shift+[ and Mod+Shift+] | Previous and next thread in sidebar order | `packages/shared/src/keybindings.ts:216-217`, `web/components/Sidebar.tsx:2912-2922` |
| Mod+1 to Mod+9 (desktop app) | Jump to the Nth row. The hints appear after the modifier is held for 200 ms. | `packages/shared/src/keybindings.ts:222-226`, `web/components/Sidebar.logic.ts:63`, `:697-710` |
| Mod+Shift+S | Settle the open thread and advance, or un-settle it if settled | `packages/shared/src/keybindings.ts:219`, `web/components/chat/useThreadLifecycle.tsx:651-660` |
| Mod+Shift+P | Pin or unpin the open thread | `packages/shared/src/keybindings.ts:220`, `web/components/chat/useThreadLifecycle.tsx:661-678` |
| Mod+Z | Undo the last thread action (section 7) | `packages/shared/src/keybindings.ts:221` |
| Mod+N, Mod+Shift+O | New thread | `packages/shared/src/keybindings.ts:200-201` |
| Mod+Shift+N | New thread in the current project | `packages/shared/src/keybindings.ts:202` |
| Mod+Alt+N | New thread without a project | `packages/shared/src/keybindings.ts:203` |
| Mod+B | Toggle the sidebar | `packages/shared/src/keybindings.ts:148` |

## 7. Undo notice

Settle, snooze, unpin, archive and draft discard offer Undo (`web/hooks/showThreadUndoNotice.ts:15-21`). The notice sits in the sidebar footer's utility row beside the Settings, Pull requests and Usage icons, so it never covers the list (`web/components/sidebar/SidebarChrome.tsx:259-293`, `web/components/sidebar/SidebarThreadUndoNotice.tsx:15-62`). It reads, for example, "Settled, Ctrl+Z to undo", or "Settled 3" when several actions of the same kind happened in a row. The full sentence goes to screen readers (`web/hooks/showThreadUndoNotice.ts:31-57`, `:75-80`).

It lasts 5 seconds after the latest action. When it expires, every pending undo is dropped (`web/hooks/showThreadUndoNotice.ts:135-150`). Mod+Z triggers it when no terminal or text field has focus. A fresh notice owns Mod+Z even inside a text field until the user presses another key (`web/hooks/showThreadUndoNotice.ts:61-63`, `:124-132`). Undoing a group runs all its restores in parallel and reports failures as toasts (`web/hooks/showThreadUndoNotice.ts:84-111`).

What each Undo restores, from `planThreadUndo` and `executeThreadUndo` (`cr/state/threadUndo.ts:38-82`):

- **Settle**: un-settle with reason `"user"`; re-pin with the old pin key if it was pinned; re-snooze to the old wake time if it was snoozed and that time is still ahead.
- **Unpin**: re-pin with the old key, then re-snooze the same way.
- **Snooze**: unsnooze.
- **Archive**: unarchive (`web/hooks/useThreadActions.ts:415-420`).

Each step runs only if the previous step succeeded (`cr/state/threadUndo.ts:68-82`).

Undos for the same thread invalidate each other, so a stale Undo cannot fight a newer change (`cr/state/threadUndo.ts:1-36`):

- Pinning invalidates pending pin, settle and snooze undos.
- Settling invalidates pending pin and snooze undos.
- Un-settle, unsnooze and reorder invalidate their own kind.
- Archive and delete invalidate every placement undo (`web/hooks/useThreadActions.ts:235-240`, `:665`, `:702-705`, `:762-765`, `:817`, `:852`).

## 8. Thread creation and projects

### Where a new thread goes

The header's New thread button is disabled when there are no projects (`web/components/Sidebar.tsx:3183`). On click (`web/components/Sidebar.tsx:2954-2977`, `web/components/Sidebar.logic.ts:765-772`):

- With Shift held, or when there is exactly one non-archived project group, or none, it starts in the current context. That is the open thread's project, else the open draft's project, else the default project: the first non-archived project in the user's `projectOrder` (`web/lib/chatThreadActions.ts:70-95`, `web/hooks/useHandleNewThread.ts:426-440`). If none resolves, the "New thread in" palette opens.
- Otherwise the "New thread in" palette opens and the user picks a project.

The tooltip explains "New thread in current project: Shift+click" (`web/components/sidebar/SidebarThreadHeader.tsx:108-127`).

A new thread is a client-side draft until its first message. It lives in the composer draft store and shows in the draft block (`web/hooks/useHandleNewThread.ts:369-421`, `web/components/sidebar/SidebarDraftBlock.tsx`). Its environment is the project's environment, because a project ref is `environmentId` plus `projectId` (`web/hooks/useHandleNewThread.ts:79-95`). Environment mode (local or worktree) comes from the option or a resolved default (`web/hooks/useHandleNewThread.ts:369`).

Other entry points are the project folder's pen button and context menu in boring mode (`web/components/sidebar/SidebarProjectFolderHeader.tsx:100-118`, `web/components/Sidebar.tsx:1264-1308`), the breadcrumb's project name (section 9), and "New thread on `<branch>`", which reuses the thread's branch and worktree (`web/components/Sidebar.tsx:2693-2713`).

### No project

"No project" is a real scratch project in a server-managed scratch folder, opened with the `openScratch` command (`web/hooks/useScratchProject.ts:62-89`, `apps/server/src/ws.ts:1877-1942`). The primary local environment is preferred when it offers a scratch folder (`web/hooks/useScratchProject.ts:51-60`). A scratch project displays as "No project" (`web/components/chat/ChatHeader.tsx:87`), and such a thread can later be promoted with "Turn into project" (`web/components/threadActionMenu.logic.ts:96-97`, `web/hooks/useThreadActions.ts:968-972`). So every thread still has a project.

### Adding a project

The header's folder-plus button, and the empty state's "Add project", open the palette at `add-project` (`web/components/Sidebar.tsx:1083`, `web/components/sidebar/SidebarThreadHeader.tsx:100-107`). Per environment, it offers (`web/components/CommandPalette.tsx:990-1092`):

- "Local folder" (browse a folder on disk)
- "New project" (start a new Git repository from a name)
- clone sources: hosted repositories, or a Git URL

## 9. Title strip, breadcrumb and sidebar collapse

### Breadcrumb

The chat header shows `project / [parent thread /] thread title` (`web/components/chat/ChatHeader.tsx:291-432`):

- **Project.** Favicon and name, or "No project". Clicking it starts a new thread in that project (`:295-320`).
- **Parent thread.** Shown for child threads, clickable to open the parent. Its title collapses to "..." when the strip is too narrow (`:121-160`, `:322-355`).
- **Title.** A button that opens the thread action menu, the same menu as the sidebar plus "Find in thread". On desktop the menu opens after a short delay so a double-click can rename instead, and a hover chevron opens it at once (`:225-251`, `:378-404`). Right-click anywhere on the strip opens the menu (`:252-271`).
- **Origin badge.** "From `<machine>`" for threads started through a linked environment (`:421-430`).

### Sidebar header and footer

The top bar holds the brand ("T3 Code", linking home) and back and forward history buttons on desktop (`web/components/sidebar/SidebarChrome.tsx:44-95`). Below it, the thread header has a search field, the project filter, Add project and New thread (`web/components/sidebar/SidebarThreadHeader.tsx:59-130`). The footer holds Settings, Pull requests, Usage, update pills and the undo notice (`web/components/sidebar/SidebarChrome.tsx:259-304`).

### Collapse

The sidebar is an off-canvas panel: collapsed means fully hidden, not an icon rail (`web/components/AppSidebarLayout.tsx:454-456`). Mod+B toggles it. The open state persists in the `sidebar_state` cookie (`web/components/ui/sidebar.tsx:26-27`, `:105-112`). It is resizable between a minimum and maximum width. The width persists, a resize is refused if it would squeeze the main content below its minimum, and double-clicking the rail resets the width (`web/components/AppSidebarLayout.tsx:461-473`). On mobile, opening a thread closes the sidebar (`web/components/Sidebar.tsx:1460-1462`).

## 10. Other behavior

### Search

Typing in the search box replaces the list with ranked results (`web/components/Sidebar.tsx:3209-3278`):

- First come title, branch and PR matches, ranked like the command palette, which includes recent use.
- Then threads that match only in their messages, as found by the server in the projects in scope (`web/components/Sidebar.logic.ts:1020-1057`, `web/components/Sidebar.tsx:1201-1207`, `:1365-1419`).

Up and Down move the highlight and wrap around. Enter opens the highlighted thread, and for a message match it also opens find-in-thread on that message. Escape clears the search (`web/components/Sidebar.tsx:1499-1538`). Search covers every section, including settled threads (`web/components/Sidebar.tsx:582-588`). The empty result reads "Searching thread messages…" while pending, then "No threads found" (`web/components/Sidebar.tsx:3270-3277`).

### Counts

The Working, Snoozed and Settled headers show counts, and the Settled count includes unloaded older threads (`web/components/Sidebar.tsx:3513-3561`). Closed folders in boring mode show a hidden-thread count and a status dot: attention, failed, done or working (`web/components/sidebar/SidebarProjectFolderHeader.tsx:46-99`, `web/components/Sidebar.logic.ts:911-935`).

### Empty states

From `web/components/Sidebar.tsx:3612-3642`:

- no projects: "No projects yet" with an "Add project" button
- several projects in scope: "No threads in these projects yet"
- one project in scope: "No threads in `<project>` yet"
- otherwise: "No threads yet"

### Unread and visit tracking

While a thread is open, a tracker records one "open" and then advances `lastVisitedAt` on each shell update. Updates are throttled to once every 10 seconds during streaming, but sent at once for a new completion or wake (`cr/state/threadVisits.ts:10-13`, `:48-145`).

The server's watermark overrides the local one, including mark-unread rewinds. The local watermark is used only with servers that lack visit tracking (`web/components/Sidebar.logic.ts:712-720`).

Unseen failures count as unseen even for threads never opened, unlike completions (`web/components/Sidebar.logic.ts:733-741`).

### Performance details worth copying

- Rows keep live subscriptions, such as git status and PR status, only while within 160 pixels of the viewport (`web/components/Sidebar.logic.ts:77-100`).
- Placement is computed from "quiet" shells that hold still while a thread streams. Each row reads its own live shell (`web/components/sidebar/SidebarThreadRow.tsx:477-483`).
- Repartitioning is driven by three clocks: a minute tick, the exact next snooze wake, and the inbox tracker's grace expiry (`web/components/Sidebar.tsx:360-363`, `:502`).

## Mapping to Shina

Shina is one native process with a JSON index and no server. Threads run Pi sessions. Captains are persistent coordinator threads whose crew are delegated child threads with parent links. Threads have pinned, archived and unread booleans. Multi-project support is adding `project` and `machine` fields per thread. The left column below is the T3 behavior. The right column is what Shina needs.

### Data model

| T3 behavior | Shina needs |
|---|---|
| Timestamps, not booleans: `pinnedAt`, `archivedAt`, `settledAt`, `snoozedUntil`, `snoozedAt`, `unsettledAt`, `lastVisitedAt` | Replace `pinned`, `archived` and `unread` booleans with optional timestamps in the JSON index. A boolean pin cannot reorder, and a boolean unread cannot tell "completed after my last visit". |
| `settledOverride` with three states | Store `settled: None / Settled / Active`. `Active` means "the user un-settled this" and blocks auto-settle until the next message clears it to `None`. |
| Unread derived from `latestRun.completedAt > lastVisitedAt` | Store `last_visited_at` and `last_completed_at`. Compute unread instead of storing it. "Mark unread" sets `last_visited_at` to one millisecond before `last_completed_at`. |
| `pinOrderKey` fractional keys (`a` to `z`) | Port `pinOrderKeyBetween` and `generateSpreadPinOrderKeys` as pure functions. They are short and have tests. |
| Thread key = environment + id | With a per-thread `machine`, key threads by machine plus id if ids can collide across machines; otherwise keep `id`. Sort tie-breaks should include the machine for stable order. |
| Server validates commands and applies them as events | One function per lifecycle change (settle, unsettle, snooze, unsnooze, pin, unpin, reorder, mark unread, archive) that applies the same field changes as `server/Orchestrator.ts:3550-3675` and saves the index. There is no optimistic layer, because writes are local and synchronous. |
| Local storage for shelf expansion, scope and folder state | Put UI state (shelves expanded, project scope, sidebar width and open state) in a separate UI-state file, not the thread index. |

### Sections

| T3 behavior | Shina needs |
|---|---|
| Priority snoozed, settled, pinned, then working or active | One `section(thread, now)` function with that exact priority, recomputed when state changes, on a minute tick, and at the next snooze wake time. |
| Working shelf with a 2-second exit grace, plus a return time that sorts the thread to the top of Active | A thread is Working while its Pi session is streaming and it has no pending approval or question. Keep an in-memory map of `working` and `returned_at` per thread. Skip the grace when the thread became actionable or the user moved it. |
| Subagents never listed; approvals roll up to ancestors; questions do not | Crew threads (`parent` set, `delegated` true) should not be top-level rows. A crew thread's approval request should show as Approval on its Captain's row. A crew thread's question should not, since the Captain answers it. Opening a crew thread highlights the Captain's row. |
| Settled shelf with paging at 25 | With a local index, all settled threads are already in memory. Keep the 10-then-25 "Show more" tail for render cost, but no server paging is needed. |
| Collapsed shelves still show the open thread | Same rule: a collapsed shelf renders only the selected thread if it lives there. |
| Project scope as a persisted multi-select; widen when opening a hidden thread | Filter by `thread.project`. Store the selected project ids in UI state. Clear them when the user opens a thread outside the scope. |
| Multi-environment merge, with offline threads falling back to Active and not draggable | Per-thread `machine`. If a machine is unreachable, show its threads but disable the actions that need that machine. Show a machine icon on rows that are not on this machine. |

### Settle and snooze

| T3 behavior | Shina needs |
|---|---|
| Auto-settle sweep every minute plus on run end | A timer in the main loop, plus a check when a Pi run ends. Candidate rules as in `isAutoSettlementCandidate`, minus guests. |
| Inactivity auto-settle after N days (default 3) | A setting, default 3 days. `settled_at` is set to the last activity time, not the time of the sweep. |
| PR-merge auto-settle | Only if Shina tracks PRs. Otherwise skip this rule and note that only inactivity applies. |
| Settle refused while running or with a pending request; clears pin and snooze; stops the session | Same refusals. Settling should stop or detach the Pi session, like the server's provider detach. |
| A sent message un-settles; automatic wakes keep snooze | When the user sends to a thread, clear `settled` to `None` and stamp `unsettled_at`. Clear the snooze only for user messages, not for Captain-to-crew or notification injections. |
| Captains | Not in T3. A Captain is persistent, so pinning it or setting `auto_settle_disabled` by default prevents it from auto-settling **(recommendation)**. |
| Snooze presets, labels and DST-safe day math | Port `resolveSnoozePreset`, `resolveSnoozePresets`, `pickSnoozePreset`, `resolveCustomSnooze` and `snoozeWakeLabel`. They are pure and depend only on local-time functions. |
| No wake timer; wake is derived from the clock | Keep `snoozed_until` stored. The thread counts as awake when `now >= snoozed_until`. Schedule a repaint at the next wake time. Show "Woke" while `woke_at > last_visited_at`. |
| Early wake on approval, question or failure | The same check on the thread's pending request and error state. |
| Navigate to the next row when parking the open thread | Same: the next non-settled, non-snoozed row, wrapping. If none exists, start a new thread in the same project. |

### Sort and drag

| T3 behavior | Shina needs |
|---|---|
| Section sort keys in the table in section 5 | Port the comparators, with the id (and machine) tie-breaks. |
| Drag between Pinned, Active and Settled; Working and Snoozed refuse drops | Drop targets come from marker positions in the flat list. Port the `planSidebarThreadDrop` decision table. A native GUI can do this with hit-testing on row rectangles. |
| Action sweep across rows | Optional. It is a press on a row button, then drag; rows of the same section between start and pointer get the action. |
| Drag a row out as chat context | Optional. Dropping a row on the composer could attach a thread reference. |

### Row, undo, creation and chrome

| T3 behavior | Shina needs |
|---|---|
| Card rows for Pinned, Active and Working; slim rows for Snoozed and Settled | Two row layouts. Card: project and pin line, title line, branch, PR, diff and machine meta line. Slim: one line. |
| Status priority and labels | Map Pi state onto: approval, input, working, waiting, failed or limited, then Woke, then Done (unread). Use the recede rule for quiet rows. |
| Hover controls replace the time | Draw Settle, Snooze and Discard draft over the time slot on hover or focus. Keep Woke clickable. |
| Context menu items and order | Same list, minus guest share and turn into project where they do not apply. |
| Undo notice: 5 seconds, grouped by action, Ctrl+Z, footer placement | One undo slot holding a list of restore plans. Port `planThreadUndo` and the invalidation rules. Expire it after 5 seconds. |
| Drafts are local until first send and show in a draft block | Shina already has per-thread `draft`. A new unsent thread can live only in memory or the UI-state file until its first prompt, then enter the index. |
| New thread picks the current thread's project, else the default; a picker when ambiguous | Use `selected thread's project → last used project → picker`. Set `machine` from the project. |
| "No project" is a real scratch project | Give Shina a scratch project folder so `project` is never empty. Offer "Turn into project" later. |
| Add project: local folder, new repository, clone | A folder picker, "new repository from a name", and a clone-from-URL dialog. |
| Breadcrumb `project / parent / title` | For crew threads, the parent segment is the Captain. Clicking the project starts a new thread there. Clicking the title opens the action menu, and double-clicking it renames. |
| Off-canvas collapse, persisted width and open state, Ctrl+B | Same, stored in the UI-state file. |
| Search over titles, branch and PR, then message content | Search titles in memory. Message search can scan Pi session logs, or a small index of them, for the threads in scope. |
