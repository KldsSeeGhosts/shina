# Thread management in Shina 0.9

The port follows [the source specification](t3-thread-management.md) and the user's later captain clarification. The source checkout was KSG Code at `684aebf1395fca2239dd59198668f6577daedee6`. The Rust pin-key algorithm, comparators, calendar snooze rules, placement priority, working grace, drop decisions and undo plans derive from T3 Code. Its [MIT notice](t3-code-license.txt) accompanies the port.

## Captains

Every non-delegated record with `captainHome` is a coordinator home, not a lone thread. The original ID, Pi session, profile, project, machine and parent links survive migration. Project `captain` and `captainThread` fields remain defaults for existing project actions. They no longer determine whether another home is a captain.

Several captains can share a folder. Captains without a project use their existing scratch-chat directory behavior. The Captains section is independent of the lone-thread project filter and search. Crew appears beneath its selected captain. Input, editor and select questions go to the coordinator once per request. The coordinator answers with `task_send`; select answers must match an offered option. Only approval requests roll up, and only the user can approve them. Crew delegated by a lone thread gets a regular sidebar row and appears in its parent's MCP listing, so the user can select, stop, archive, or delete it.

Captain homes cannot pin, snooze or settle. The minute sweep and run-end sweep exclude them. Archived captain history stays archived and remains recoverable by opening its profile. New-captain creation suggests an unoccupied project, and its project chooser also permits occupied projects or no project.

## Local storage and timers

The version 4 index stores millisecond lifecycle timestamps in a separate `lifecycle` object. Version 3 and legacy workspace imports preserve pin and archive flags as timestamps. An unread legacy record gets a synthetic completion and a watermark one millisecond earlier. Malformed input prevents both index and UI-state saves.

The existing atomic store writes both files through separate workers. `ui-state.json` owns shelf expansion, scope, sidebar visibility and width, and composer drafts. Thread metadata, including unsent thread IDs, stays in the index. Undo holds restore plans in memory for five seconds and preserves old pin keys and future snooze times.

The main loop schedules the next minute tick, snooze boundary, working-exit deadline and undo expiry through Rune's wake mechanism. A run ending also triggers the eligibility sweep. Approval and question edges, or a failure newer than the snooze, record an early wake once. Completion does not wake a snoozed thread.

The PR integration seam is `pullRequestFinishedAt` plus `pr_settlement_ready`. A future integration must provide a finished timestamp only when it has snapshots for all relevant links and none is open. This release does not discover links or watch PRs.

## Adaptations and deferred spec items

- There is one local process, not server commands, optimistic reconciliation or remote paging. The index holds all settled records; rendering still pages 10, then 25.
- Working is always enabled. Active stays time-ordered, so the optional working-disabled and boring-folder comparators are not exposed.
- Rune supports pointer drag on Windows. Native section drops and fractional pin reordering are implemented. Context dragging, file drops and action-sweep dragging are deferred.
- Unsent prompt text lives in UI state, but unsent thread metadata stays in the index for stable IDs and tabs. Discarding an unsent draft retains its metadata in the archive, and Undo restores it.
- Rename uses Shina's existing dialog, with F2 and double-click entry points. Inline rename and blur-to-commit are deferred.
- Menus include pin, settle, snooze, rename, mark unread, project scope, auto-settle, copy ID/path, archive, delete and draft discard, plus bulk lifecycle actions. Title regeneration, project settings, guest sharing, branch/PR actions and copying full thread transcripts are deferred.
- Rows use card and slim layouts, project, pin, runtime status, Done/Woke, age, hover controls and machine indication. Branch, PR, terminal and diff metadata, provider stacks, rich hover cards and held-modifier jump hints are deferred because Shina has no corresponding sidebar data subscriptions.
- Search covers titles and messages already loaded in memory. Scanning saved Pi logs, ranked results, highlighted result navigation and server message search are deferred.
- New-thread project selection, folder import, clone and project-less scratch folders keep Shina's existing flow. Creating a new Git repository and promoting a scratch chat into a project are deferred.
- Remote machines remain unsupported by the backend. Their records remain visible with Active/Pinned fallback; machine-dependent lifecycle actions refuse to run locally.
- GUI automation and paid CPA integration tests are outside this validation. No visible Shina window was opened.

## Headless validation

`--snap` loads an isolated profile read-only, without setup, Pi, catalog discovery or a store writer. The real-data render uses copies of the workstation's three legacy profile files, not the real profile. Their SHA-256 hashes were unchanged after rendering. It validates sidebar migration without loading transcripts through Pi.

Render paths are under `artifacts/threads-v0.9/`, which Git ignores:

- `example.png`, light, 1280 by 840
- `dark.png`, dark, 1280 by 840
- `sidebar-narrow.png`, 620 by 720
- `shelves-expanded.png`, all shelves expanded, 1280 by 1040 so every shelf row is visible
- `snooze-menu.png`
- `undo-notice.png`
- `real-copy.png`, migrated copy of the workstation's legacy data

Use `--at 0.5` for the layout to process its initial geometry callbacks before capture. All render commands set `LOCALAPPDATA` to an isolated profile under the artifact directory.

Final checks passed: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build` and `git diff --check`. The isolated-profile test run passed 127 tests and skipped seven opt-in integration tests. Native pointer-drag, resize, shelves, paging, menus and text-field undo tests use Rune's headless runtime.
