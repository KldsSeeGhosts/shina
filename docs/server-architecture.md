# Headless server and clients

## Decision and evidence

Run `shina serve` from the same binary as an independent per-user process. It owns execution and durable state. Rune, and later mobile, authenticate as clients. Closing every window leaves accepted work running. This is shared access to server-owned sessions, not replication of live agent processes.

Research baseline: Shina `2e27d22f72e91ea63f2550a26869a78f4c560356`; KSG Code `684aebf1395fca2239dd59198668f6577daedee6` at `D:\AiStack\ksgcode`. Paths below are relative to their repository. These are source-verified findings, not deployment tests.

Shina's `README.md`, `AGENTS.md`, `docs/harnesses.md` and `docs/pi-integration.md` match these implementation constraints:

- `src/model.rs` combines `State` UI controls with threads, stores, tasks and process ownership. `poll` drains MCP and harness events, advances deadlines, delivers results and sweeps lifecycle. `Thread::apply` also calls runtime methods. Neither is currently a pure reducer.
- `src/backend.rs` dispatches only `local`; unknown machines fail. Its bounded transport and session-owner locks support Pi and `src/acp.rs` through `src/harness.rs`. `src/pi_session.rs` mixes queue/approval/runtime state with disclosure and picker state.
- `src/mcp.rs` runs authenticated loopback HTTP inside the app, but queues calls for the UI poll. `src/store.rs` saves JSON and a run marker, not an exclusive owner lock. `src/tasks.rs` reconciles durable delegation against Pi history without blindly repeating work.
- `src/main.rs` shuts down on window close. `src/process.rs` kills owned descendants through Windows Job Objects or Unix process groups. `src/update.rs` assumes app exit before replacement.

`docs/captains.md`, `src/background.rs`, routines and memory persistence are absent here. Integrate their merged implementations later; do not invent their current schemas.

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

The UI reads this file, authenticates and verifies identity/version. If unavailable, it spawns a hidden detached `shina serve`, then waits for readiness. Simultaneous launches converge through the lock. Never kill a PID or remove a lock based only on a stale port file. A held lock with no ready listener reports recovery/unavailability. Do not put the server in the UI's kill-on-close job; keep harness jobs owned by the server.

Zero clients never means shutdown. Explicit `server.stop` drains or refuses active work. Optional login startup uses a Windows per-user scheduled task, macOS LaunchAgent or Linux user service, only after user opt-in. Server startup must load the host's existing provider environment; never copy authentication from a UI or repository.

Keep checksum verification, explicit restart approval and rollback in `src/update.rs`. The server coordinates one update. `server.prepareRestart` refuses active/configuring/queued work, approvals and clones initially. On approval, fence new mutations and routine dispatch, broadcast draining, flush, reap idle processes, and exit. The helper waits for both server and local clients before replacing Windows executables or the Mac bundle, starts `serve`, validates readiness, then reopens clients. Timeout or save failure cancels the drain; never silently abort work. Back up the database before schema upgrades and retain it for binary rollback. Protocol-major mismatch refuses normal commands with upgrade guidance; additive minor capabilities negotiate independently.

If `src/background.rs` merges, its tray controls are clients: Open window, server status and explicit Stop server. Tray opt-in must not become the condition for keeping runs alive.

## State boundaries

Server `RuntimeState` owns projects and host paths, thread metadata, transcript projection/event log, harness/session identities, processes, lifecycle, submitted queues, approvals, tasks, captain relationships, memory and routine scheduling. Preserve process-specific approval expiry and task delivery IDs. Existing CPA-only Pi behavior and ACP capability limits remain.

Client `ClientState` owns Rune `TextEdit`, hover, pickers, selection, tabs, layout, scroll, tool/thought expansion, animation, caches and unsent editor text. Split `Tool.expanded`, `Thought.expanded` and `PiSession.queue_expanded` from durable records. Visits are explicit server commands, not whatever `active` happened to mean on one client.

Drafts are local and crash-saved in slice 1. Later back them up server-side by `serverId/threadId/deviceId` with revision compare-and-swap. Another device explicitly imports a draft; no last-writer-wins overwrite. Attachments follow that draft's ownership. Send atomically captures content into a server-owned input and clears only the matching editor revision. Stop returns unsent queue entries as recoverable inputs, never inserts text into another device's editor.

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
5. Captain delegation and sync, 5 to 8 days. Change peers/tasks/captains, merged memory/routines, database/client. Add home-server scheduling, durable remote results and per-device draft backup. Test duplicate launches/results, offline cancellation and conflicting drafts.
6. Mobile contract hardening, 2 to 4 days, excluding the app. Add wire fixtures, scoped pairing and bandwidth tests. Verify suspend/resume and paged history without desktop IPC.

The highest risk is splitting `model.rs` while preserving `apply_native`, task delivery, queue recovery and session transitions. First move ownership without rewriting these algorithms. Introduce plain DTOs and separate presentation fields next; make reduction pure afterward. Keep isolated-profile integration tests and the old path until slice 2 passes. Server mode disables in-app installation until slice 3.

## User decisions

- Which always-on machine should host the first captain?
- Should login startup remain off by default, as recommended?
- Is explicit import of another device's draft sufficient, or is collaborative editing required?
