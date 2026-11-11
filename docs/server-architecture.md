# Headless server and clients

## Implementation status

Slice 1 is implemented behind `--server-client`; legacy mode remains the default. The same binary's `serve` command owns the existing reducer, subprocess workers, JSON stores, MCP queue and background scheduling. `runtime.rs` serializes commands and ticks without initializing artwork, portrait animation or image caches. This deliberately moves ownership before attempting a pure reducer.

`server_lock.rs` uses the never-unlinked `server-owner.sqlite` exclusive transaction in both execution modes. `server_auth.rs` publishes loopback discovery only after recovery/listener setup, with stable server and epoch IDs, a fresh instance ID and a 256-bit OS-random bootstrap token. Windows applies a protected current-user-only DACL without changing ownership, auditing privileges or authentication. Native bootstrap exchange issues ephemeral device credentials. Origins are refused in this native-only slice.

Tokio/Axum handles authenticated HTTP and WebSockets; Tokio-Tungstenite shares Axum's wire implementation with the native client. Existing blocking harness workers remain unchanged. A bounded 32-command queue feeds the runtime, and a latest-value snapshot channel prevents slow clients from blocking harness drains. Slice 1 sends full projected snapshots after changes and on reconnect. It limits control requests to 1 MiB and encoded snapshots to 2 MiB; larger profiles need slice 2 paging.

`State` remains the UI facade. `client_state.rs` projects records and forwards execution methods, including direct `Thread` controls. Existing captain scope, memory, task and routine getters remain callable. Their mutations return local enqueue errors immediately; server validation/results arrive through receipts. Returned IDs from asynchronous facade mutations are provisional until their projection arrives. Editors and disclosure state are never part of a server projection. Concurrent desktop processes reserve separate persistent `clients/<slot>/ui-state.json` files. The original legacy UI file is retained.

Receipts, pending accepted inputs and uncertain command intents are part of the atomically replaced JSON index. Dispatch follows a flushed intent; duplicates are keyed by device and command ID and checked against the canonical payload hash. A crash leaves an interrupted receipt and never triggers automatic redispatch. Existing Pi/task recovery still decides what history proves. This is not a multi-file transaction or a SQLite effect outbox. The server also checkpoints transcript DTOs for ACP agents without replay. A later legacy-mode launch preserves those records rather than discarding the server extension fields.

The desktop attaches first or supervises its own `serve` child with 0.5 to 10 second backoff. Quit stops only that owned child, with five seconds before termination. An attached headless server remains alive. Windows tray opt-in keeps the desktop alive after an idle close.

Slice 3 implements `shina service install|uninstall|status|start|stop` in `service.rs`, ported from KSG's service and boot-service modules. Linux installs a systemd user unit, macOS a LaunchAgent, and Windows a least-privilege per-user logon task with hidden startup. `--enable-linger` is the only path that changes lingering, and never uses sudo. Desktop login startup remains off. Reinstallation reconciles executable and discovery paths. The service inherits host environment and reads the host's existing `service.env`; generated files contain no provider credentials. Normal server exit does not trigger a manager restart, so explicit Stop server remains effective. Unexpected failures restart.

`maintenance.rs` owns leased prepare/cancel/commit transitions. Runtime hooks check existing thread, queue, approval, clone and routine blockers, flush before preparation, fence all new mutations and skip reducer ticks and routine dispatch while draining. Clients receive `server.draining`, flush their own stores and acknowledge with `server.clientReady`; save failure cancels. A preparation expires after 30 seconds without stopping work. `server.commitRestart` requires every registered local client's acknowledgement, flushes again and broadcasts `server.restarting` before idle-process shutdown.

The separate known-working helper waits for local process handles, including the service launcher, and the server owner lock before replacement. It requires a final-save acknowledgement for the prepared update ID. It retains a SQLite-consistent and JSON state backup, reuses Windows `ReplaceFileW` and macOS atomic bundle exchange, restarts the installed service or independent `serve`, checks authenticated readiness and expected binary version, then reopens clients. A validation marker fences dispatch in the replacement until `server.validateRestart`. A desktop-owned server uses a paused validation process, then reopened desktops spawn and supervise their own child as before. Updating does not opt the desktop into an always-on service. Startup failure restores the previous binary and its matched state, preserving the failed schema separately. Backups remain for rollback. The server also backs up before opening runtime state on a binary-version change, before slice 2's schema migration can run.

`shina update --yes` uses the same GitHub CLI login, asset selection and checksums, with validated tar extraction and atomic executable replacement on Linux x86_64. No new credentials are introduced. Major-version mismatch refuses commands with upgrade instructions; additive minors negotiate down. The Windows client tray shows server connection status and sends explicit Stop server through the client. Explicit stop can interrupt work; update preparation cannot.

Slice 3 tests include three platform golden files without installation, leased drain transitions, save failure, version negotiation, SQLite WAL backup and matched-schema restoration, helper sequencing and rollback with fakes, archive validation, real authenticated server restart/cancel tests, and the existing locked-Windows-executable replacement test. A native Windows temporary-profile install/status/readiness/uninstall round trip passed. A final scheduled-task query confirmed removal. macOS and Linux service activation still require native validation. Slice 4 must apply maintenance authorization to these RPCs, keep local client PID registration local-only, and preserve the `RuntimeState::maintenance_command`, `Maintenance::status` and pre-open backup hooks when integrating slice 2. Clients without restart coordination must close before updating; the server refuses to drain while one remains connected.

Validation uses real `serve` processes, authenticated WebSockets and fake Pi/ACP subprocesses in temporary profiles. It covers ownership races, zero-client MCP completion, lost acknowledgements, reconnect deduplication, restart persistence and ACP resume without history. Native Windows checks covered two attached clients, separate drafts, legacy ownership refusal, close/reattach persistence and owned-server shutdown with no submitted prompts.

Slice 2 still needs database migration, transactional receipts and effects, durable devices, bounded history/blob paging, typed incremental patches/replay, and crash-boundary/slow-subscriber tests. It must retain its own pre-schema backup as well as the slice 3 pre-open backup. No pairing, browser tickets, remote server binding or automatic continuation has been added.

## Decision and evidence

Run `shina serve` from the same binary as an independent per-user process. It owns execution and durable state. Rune, and later mobile, authenticate as clients. Closing every window leaves accepted work running. This is shared access to server-owned sessions, not replication of live agent processes.

Research baseline: Shina `2e27d22f72e91ea63f2550a26869a78f4c560356`; KSG Code `684aebf1395fca2239dd59198668f6577daedee6` at `D:\AiStack\ksgcode`. Paths below are relative to their repository. These are source-verified findings, not deployment tests.

Shina's `README.md`, `AGENTS.md`, `docs/harnesses.md` and `docs/pi-integration.md` match these implementation constraints:

- `src/model.rs` combines `State` UI controls with threads, stores, tasks and process ownership. `poll` drains MCP and harness events, advances deadlines, delivers results and sweeps lifecycle. `Thread::apply` also calls runtime methods. Neither is currently a pure reducer.
- `src/backend.rs` dispatches only `local`; unknown machines fail. Its bounded transport and session-owner locks support Pi and `src/acp.rs` through `src/harness.rs`. `src/pi_session.rs` mixes queue/approval/runtime state with disclosure and picker state.
- `src/mcp.rs` runs authenticated loopback HTTP inside the app, but queues calls for the UI poll. `src/store.rs` saves JSON and a run marker, not an exclusive owner lock. `src/tasks.rs` reconciles durable delegation against Pi history without blindly repeating work.
- `src/main.rs` shuts down on window close. `src/process.rs` kills owned descendants through Windows Job Objects or Unix process groups. `src/update.rs` assumes app exit before replacement.

At the research baseline, `docs/captains.md`, `src/background.rs`, routines and memory persistence were absent. Slice 1 now uses their merged implementations and existing schemas.

KSG Code provides the following verified precedents:

- `apps/server/src/ws.ts` uses Effect RPC with JSON WebSocket serialization, not literal JSON-RPC 2.0. `packages/contracts/src/{rpc,orchestrationV2,ipc}.ts` separate network schemas, domain records and desktop IPC. `config.ts` places `statev2.sqlite` under the state directory.
- `apps/server/src/serverOwnership.ts` holds an exclusive transaction on separate `server-owner.sqlite` until exit. The lock file must never be unlinked. `orchestration-v2/EventSink.ts` commits events, projections, effects and command receipts together, then publishes. See also `persistence/OrchestrationEventStore.ts`, `orchestration-v2/{ProjectionStore,CommandReceiptStore,EffectOutbox}.ts`.
- `orchestration-v2/ThreadStream.ts::decideThreadResume` caps replay at 128 events and 1 MiB encoded. `ws.ts` also checks raw bytes and pruned ranges, subscribes before reading, and sends replay or a bounded snapshot followed by live events.
- `orchestration-v2/ProviderRuntimeRecoveryService.ts` reconciles lost sessions and requests and retires process-bound effects. `RestartContinuation.ts` conditionally adds a guarded continuation prompt through `continueThreadsAfterServerUpdate`. It does not resurrect a process or guarantee exactly-once tools.
- `auth/{EnvironmentAuth,RpcAuthorization}.ts` distinguish client scopes and MCP credentials. `peer/PeerLinks.ts` pairs through MCP OAuth and stores tokens in the server secret store. `peer/RemoteDelegation.ts` creates ordinary remote threads, records parent tasks and follows results across restarts, retrying sleeping peers. `environment/ServerEnvironment.ts` persists identity. `background/BackgroundPolicy.ts` uses expiring activity leases for optional background work.
- `packages/client-runtime/src/{rpc/session,connection/supervisor,state/threads}.ts` reconnect and resume cached sequence cursors. `state/sharedSettings.ts` syncs selected settings. `apps/web/src/composerDraftStore.ts` persists drafts in localStorage, reconciles model choices and promotes submitted drafts against server state. Arbitrary cross-device draft-text sync was not verified.
- `apps/desktop/src/backend/{DesktopBackendPool,DesktopBackendManager}.ts` spawn scoped backends and stop them on shutdown; an owned state directory produces an error, not automatic attachment. `app/DesktopLifecycle.ts` quits after the last window closes except on macOS. Separately, `apps/server/src/serviceLauncher.ts` launches `serve` independently.

The recommendations below are Shina design decisions, not claims of KSG compatibility.

## Process ownership and updates

Add `server.rs`, `runtime.rs`, `protocol.rs`, `client.rs`, `server_lock.rs` and `server_auth.rs`. Use Tokio/Axum for HTTP/WebSockets, existing blocking harness workers, and a single serialized runtime command loop. Replace Rune wake dependencies with an `Arc<dyn Fn()>` notification adapter; never call `State::new` headlessly because it constructs artwork and image caches.

Both legacy and server modes acquire an exclusive transaction on `<data>/server-owner.sqlite` before loading writable state. After recovery and listener readiness, atomically publish `server.json` containing `serverId`, `instanceId`, PID, start time, loopback port, protocol range, binary version and an OS-generated 256-bit bootstrap token. Restrict the directory/file to the user, including Windows ACLs. The token exchanges for a local device credential. Publish no secrets in logs or argv.

Follow KSG Code's desktop model (`apps/desktop/src/backend/DesktopBackendManager.ts`). The desktop app supervises its local server as a child process: it reads this file, attaches to a ready server if one already owns the profile, otherwise spawns `shina serve`, waits for readiness and restarts it with backoff (0.5 s growing to 10 s) if it exits unexpectedly. Simultaneous launches converge through the lock. Never kill a PID or remove a lock based only on a stale port file. A held lock with no ready listener reports recovery/unavailability. Keep harness jobs owned by the server, not the UI's kill-on-close job.

As in KSG (`apps/desktop/src/app/DesktopLifecycle.ts`), quitting the desktop app stops the server it spawned, with a 5 s grace for harnesses; on Windows and Linux closing the last window quits, on macOS the app stays in the Dock. Work then resumes through durable state and restart continuation on the next launch. A server the desktop app attached to but did not spawn keeps running. Always-on machines run a headless `shina serve` installed as an opt-in boot service with `shina service install`, like KSG's `apps/server/src/cli/service.ts` and `cloud/bootService.ts`: a systemd user unit on Linux and a LaunchAgent on macOS. KSG has no Windows service; Shina adds a per-user logon scheduled task there. The opt-in tray mode in `src/background.rs` keeps the desktop app, and so its server, alive after the window closes. Server startup must load the host's existing provider environment; never copy authentication from a UI or repository.

Keep checksum verification, explicit restart approval and rollback in `src/update.rs`. The server coordinates one update. `server.prepareRestart` refuses active/configuring/queued work, approvals and clones initially. On approval, fence new mutations and routine dispatch, broadcast draining, flush, reap idle processes, and exit. The helper waits for both server and local clients before replacing Windows executables or the Mac bundle, starts `serve`, validates readiness, then reopens clients. Timeout or save failure cancels the drain; never silently abort work. Back up the database before schema upgrades and retain it for binary rollback. Protocol-major mismatch refuses normal commands with upgrade guidance; additive minor capabilities negotiate independently.

If `src/background.rs` merges, its tray controls are clients: Open window, server status and explicit Stop server. Tray opt-in must not become the condition for keeping runs alive.

## State boundaries

Server `RuntimeState` owns projects and host paths, thread metadata, transcript projection/event log, harness/session identities, processes, lifecycle, submitted queues, approvals, tasks, captain relationships, memory and routine scheduling. Preserve process-specific approval expiry and task delivery IDs. Existing CPA-only Pi behavior and ACP capability limits remain.

Client `ClientState` owns Rune `TextEdit`, hover, pickers, selection, tabs, layout, scroll, tool/thought expansion, animation, caches and unsent editor text. Split `Tool.expanded`, `Thought.expanded` and `PiSession.queue_expanded` from durable records. Visits are explicit server commands, not whatever `active` happened to mean on one client.

Drafts stay on the device that typed them, as in KSG, which persists composer drafts client-side (`apps/web/src/composerDraftStore.ts`, zustand `persist` to localStorage) and never syncs them. Shina keeps them in local `ui-state.json`. Attachments follow that draft. Send atomically captures content into a server-owned input and clears only the matching editor revision. Stop returns unsent queue entries as recoverable inputs, never inserts text into another device's editor.

Move `Thread::apply` server-side first, preserving adapter normalization. Incrementally separate `reduce` from returned effects. Clients receive typed projection updates, not raw harness records or the execution reducer. Rune can apply a bounded batch, wake once and draw immediately without duplicating approval, queue or recovery logic.

## Protocol v1

Use JSON-RPC 2.0 over `/v1/ws`, with HTTP for blobs. Native clients use bearer headers; browser-capable clients exchange credentials over HTTP for short-lived single-use WebSocket tickets. Validate Origin. `hello` negotiates protocol major/minor, projection schema and capabilities and returns identities, versions, limits and server time.

Every mutation carries a client-generated UUID `commandId`. Persist receipt keyed by authenticated device and ID, canonical request hash, accepted/rejected status, result IDs and committed sequence. Duplicate identical requests return the receipt; changed payloads fail. An accepted receipt means durable intent, not completed provider execution. Receipt lookup resolves lost responses.

Concrete methods, with capability-gated groups introduced by later slices:

| Methods | Required inputs or result |
| --- | --- |
| `hello`, `command.get` | Versions/capabilities; command ID to receipt |
| `workspace.subscribe`, `thread.subscribe`, `subscription.cancel` | Server/thread identity, optional cursor; subscription ID |
| `thread.history` | Thread, opaque cursor, byte/item budget |
| `project.put`, `project.remove`, `project.clone` | Host-local path or clone specification, project ID |
| `thread.create`, `thread.rename`, `thread.delete` | Project/harness/model/access selection; thread ID/title |
| `thread.send`, `thread.stop` | Thread, text/blob IDs, mode `prompt/steer/queue`, editor revision |
| `thread.configure`, `thread.compact`, `thread.fork`, `thread.rewind` | Typed selections/instructions or native entry ID |
| `thread.lifecycle`, `thread.visit` | Pin/archive/settle/snooze action or visit timestamp |
| `queue.edit`, `queue.remove`, `queue.sendNow`, `input.claim` | Stable input ID, expected revision, optional replacement |
| `request.respond` | Thread, request ID, runtime generation, decision/answer |
| `task.delegate`, `task.send`, `task.cancel`, `task.resume`, `task.dismiss`, `task.resumeResults` | Parent/task identity, brief/answer, optional target server/project |
| `captain.put`, `captain.remove`, `memory.put`, `memory.remove` | Scoped record ID, expected revision, content |
| `routine.put`, `routine.remove`, `routine.run` | Routine ID, revision, schedule/target or immediate invocation |
| `draft.put`, `draft.list`, `draft.get` | Device/thread, base revision, text/blob IDs |
| `device.pair`, `device.list`, `device.revoke`, `peer.link`, `peer.list`, `peer.unlink` | Scoped pairing grant, device/peer ID |
| `server.status`, `server.prepareRestart`, `server.cancelRestart`, `server.stop` | Maintenance permission, prepared-update ID where applicable |

Notifications are `subscription.snapshot`, `subscription.patch`, `subscription.ready`, `subscription.resyncRequired`, `server.draining` and `auth.revoked`. Snapshot contains `{subscriptionId, epoch, seq, state, historyCursor}`. Patch contains `{subscriptionId, epoch, prevSeq, seq, upserts, removals}` with typed thread/message/tool/request/task records. Text updates replace bounded message blocks rather than resend the transcript. Workspace subscriptions carry summaries only.

For subscribe, register a bounded live buffer, capture high-water `H`, then replay committed patches through `H` or send a consistent snapshot at `H`. Replay requires matching epoch/schema and retained history, at most 128 records and 1 MiB raw/encoded. Otherwise snapshot. Send ready at `H`, then buffered live updates above `H`. Ignore duplicates; a `prevSeq` mismatch resubscribes. A cursor is per subscription scope, never a global contiguous-number assumption. Epoch survives process restarts; restore/reset creates a new epoch.

Start with 1 MiB control messages, 256 records/2 MiB outbound per subscription, 32 outstanding commands and heartbeats. Coalesce before assigning sequences. Slow clients get resync or disconnect, never block harness drains. Page snapshots/history to 256 KiB, externalize large output and fetch thumbnails lazily.

Authenticated HTTP provides `POST /v1/uploads`, offset-checked `PUT /v1/uploads/{id}`, commit and authenticated blob GET. Stream to temporary files, hash and validate before atomic promotion. Keep current image limits, eight images, 10 MiB each, 20 MiB total. Reject arbitrary server paths; expire abandoned uploads.

## Persistence and recovery

Use `rusqlite` with bundled SQLite, WAL, foreign keys and `synchronous=FULL` on a dedicated writer. Proposed tables are `events`, `thread_projections`, `projects`, `tasks`, `captains`, `queued_inputs`, `memory`, `routines`, `drafts`, `command_receipts`, `effect_outbox`, `devices` and `peers`. Events store aggregate ID, sequence, schema, command ID and payload. Store normalized projection changes and durable intent, not secret-bearing raw RPC records.

One transaction updates events/projections, receipt and outbox; publish only after commit. Effects use stable IDs. Mark dispatching before calling a provider. After process loss, an uncertain send becomes interrupted, not automatically replayed. Reconcile Pi history using existing markers; ACP without history keeps the server transcript and reports resume limits. Persist task result delivery before parent notification. Exactly-once tool actions are not promised.

Under the owner lock, validate and import `index.json`, its embedded tasks, `ui-state.json` pending inputs and attachments in one transaction. Preserve existing legacy project/collision migration in `store.rs`, session paths, captain homes and IDs. Pending acceptedness remains uncertain until history reconciliation. Import `memory.json` and routine files only against their merged versioned schema. Unknown/corrupt data aborts import without partial writes.

Record source hashes and migration completion, verify counts/references, and retain backups and every legacy file unchanged. Keep client layout/drafts in a new device-local file. Never delete legacy data or infer missing images. Pi session files remain provider-owned. Database rollback restores a matched backup, never runs an old binary against a newer schema.

## Machines, security and mobile

Give each installation a stable server UUID. Use explicit Tailscale-only listeners on the Windows PC, both Macs and Ubuntu server, or loopback behind Tailscale HTTPS. Default remains loopback; refuse wildcard/public binding. Require application auth even on the tailnet.

Pair devices/peers with short-lived, single-use, rate-limited codes approved on the target. Issue revocable per-device tokens. Store token hashes and grants server-side, outbound peer secrets in user-protected storage outside repositories. Separate `read`, `operate`, `approve`, `maintain` and `peer-delegate` scopes. MCP tokens remain thread-bound and cannot call client/admin APIs. Agents and peers cannot approve requests, elevate access modes or mint device credentials. User responses use request ID/generation compare-and-swap so two clients cannot answer twice.

A thread's authority and execution stay on its chosen server. Selecting B from client A sends creation to B, using B's projects, model availability and credentials. A client connects directly to each paired server and keys views by server/thread UUID. No filesystem path translation, automatic checkout sync, session-file copying or live migration.

A captain has one home server, which owns scheduling and memory. Remote delegation records a stable task/launch ID at home, creates a child through the peer's authorized command endpoint, then follows its result cursor. Peers never accept browser tickets as delegation credentials. Persist reconciliation and result acknowledgements; retry the same IDs after uncertain responses.

Sleeping/offline B leaves cached views labelled stale. No local fallback or queued offline approval. Only work already accepted by B can run there. Home retains pending cancellation/result delivery and reports it pending until acknowledged; it does not claim success. Retry with bounded exponential backoff, stopping on revoked auth. Closing A cannot affect B; sleeping B pauses execution until wake.

Mobile uses the same bounded projections, history paging, upload resume and device revocation. Reconnect after suspension with cursors; avoid requiring persistent foreground sockets. Later add push registration and a durable notification outbox, with opaque IDs and no prompt/tool secrets in push payloads.

## Shippable slices

Estimates are engineer-days after the concurrent branches merge, including tests.

1. Local ownership, 7 to 10 days, behind `--server-client`. Change `main.rs`, `model.rs`, `pi_session.rs`, `backend.rs`, `harness.rs`, `acp.rs`, `mcp.rs`, `tasks.rs`, `store.rs`, `process.rs`, `ui.rs`; add runtime/server/client/protocol/lock/auth modules and dependencies. Extract a headless constructor and tick, retain JSON projection storage temporarily, and journal command receipts/intents atomically in the index. Support existing local commands through typed DTOs; snapshot reconnect is sufficient initially. UI alone owns drafts/layout. Test fake Pi and ACP runs continuing after client exit, second-client attachment, single-owner races, MCP completion delivery and no duplicate send after reconnect. Never run both execution paths against one profile.
2. Durable protocol, 5 to 7 days. Add `database.rs`, migrations and outbox worker; change runtime/store/tasks/protocol/client. Atomically migrate JSON, add transactional receipts and projection patches/replay. Test lost acknowledgements, crash boundaries, corrupt imports, replay gaps and slow subscribers.
3. Service and update lifecycle, 3 to 5 days. Change `install.rs`, `update.rs`, `main.rs`, server modules and merged `background.rs`. Add opt-in login adapters, drain fencing, rollback and tray commands. Test on Windows and macOS, including locked executable replacement and cancelled drains.
4. Paired multi-server clients, 4 to 6 days. Add `peer.rs`, change auth/client/protocol and machine selection in `ui.rs`/projects. Test create/view/stop on two fixture servers, path isolation, revocation and sleep/reconnect.
5. Captain delegation and sync, 5 to 8 days. Change peers/tasks/captains, merged memory/routines, database/client. Add remote delegation like KSG's `apps/server/src/peer/RemoteDelegation.ts` and durable remote results. Test duplicate launches/results and offline cancellation.
6. Mobile contract hardening, 2 to 4 days, excluding the app. Add wire fixtures, scoped pairing and bandwidth tests. Verify suspend/resume and paged history without desktop IPC.

The highest risk is splitting `model.rs` while preserving `apply_native`, task delivery, queue recovery and session transitions. First move ownership without rewriting these algorithms. Introduce plain DTOs and separate presentation fields next; make reduction pure afterward. Keep isolated-profile integration tests and the old path until slice 2 passes. Slice 3 restores coordinated in-app installation for installed server clients.

## Decisions

These follow KSG Code, at the user's request.

- Every thread and captain belongs to the server where it was created, like KSG environments. New work defaults to the local desktop server. The composer's machine selector picks another linked server, and delegation can run a child on another machine that reports back to its parent. A captain meant to run around the clock is simply created on an always-on server.
- The desktop app spawns and supervises its local server and stops it when the app quits. Always-on hosts use the opt-in boot service. Login startup for the desktop app stays off.
- Drafts are per device and are not synced.
