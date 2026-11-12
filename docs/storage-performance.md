# Scoped persistence and streaming

## Layout and transaction boundary

Previously `documents` held complete index and server-input JSON documents. The index embedded every thread view. `projection` held another complete workspace projection. A streaming reducer batch captured and serialized those documents repeatedly, rebuilt the workspace projection and hashed each history before producing its small wire patch.

Schema 5 adds `projection_records`, keyed by document, scope and record key. Root properties are separate rows. Thread scopes contain metadata or a view header. History item rows use the field and durable identity as their key. Their versioned payload contains the item, ordinal, identity, allocation position and last-change sequence. Runtime inputs and pending messages have thread scopes too. Recovery hydrates the existing reducer's input shape from these rows. `projection` retains the committed workspace sequence, rather than a full JSON workspace.

Projection rows and thread event envelopes use schema 1. Unknown newer database, row or event schemas fail closed. Additive wire fields remain compatible within protocol major 1, and optional capabilities negotiate independently at protocol minor 9. These tests cover contract fixtures, not a deployment of two different binaries.

`thread_events` stores versioned, thread-scoped item batches in the same transaction. Workspace replay retains the existing 1024-sequence policy. Thread replay retains its existing 128-event and 1 MiB admission limits. Both stored and in-memory thread replay have 1024-batch and 4 MiB limits. The post-commit publication batch is separate, so a large item updates the live projection even when replay cannot retain its event. Old cursors receive a bounded scoped catch-up snapshot when the retained range is insufficient. An idle thread at its high-water mark synchronizes without one.

Commands still commit state, receipt, captured effects and replay data atomically before dispatch or publication. Receipt history and its device/command primary key remain independent of replay retention. Uncertain process-bound effects become interrupted on recovery. Neither recovery nor a failed commit retries a provider send.

## KSG mapping

KSG paths below are relative to `/Volumes/DevDrive/AiStack/ksgcode`.

| Mechanism | KSG source | Shina implementation |
| --- | --- | --- |
| Identity-keyed item rows and ordinals | `apps/server/src/orchestration-v2/ProjectionStore.ts`, `turn-item.updated` | `projection_store::write`, `history::stamp`, `ThreadView::capture` |
| Stored item identities across recovery | `apps/server/src/orchestration-v2/Orchestrator.ts`, item ID allocation | `Message::id`, `Thread::apply` history reconciliation |
| Idempotent per-thread item position allocation | `apps/server/src/orchestration-v2/TurnItemPositionStore.ts` | `history::stamp`, persisted `historyNextPosition` |
| Events, projections, receipts and effects in one transaction | `apps/server/src/orchestration-v2/EventSink.ts` | `Database::commit`, `RuntimeState::persist` |
| Post-commit item publication | `apps/server/src/orchestration-v2/Orchestrator.ts` | `Streams::ingest`, server runtime publication |
| Durable scoped high-water marks and bounded replay | `apps/server/src/orchestration-v2/ThreadStream.ts`, `ProjectionStore.ts` | `Database::history_events_after`, `Streams::restore`, `Streams::replay` |
| Identity-anchored recent and older windows | `apps/server/src/orchestration-v2/threadHistoryPaging.ts` | `Streams::page` |
| Stable item start timestamps | `packages/contracts/src/orchestrationV2.ts`, turn-item fields | `ThreadView::capture`, `ThreadView::apply` |
| Latest-value cache persistence, 500 ms first save and ten-second cadence | `packages/client-runtime/src/state/cachePersistence.ts` | `cache_persistence::Writer` |
| Share in-flight asset work by environment and immutable resource identity | `packages/client-runtime/src/state/assets.ts` | `client::fetch_assets`, `client::cache_images` |

Shina keeps its existing reducer rather than replacing it with KSG's typed domain-event reducer. Polling records dirty thread scopes, and the row writer compares the selected thread's records. It does not serialize or hash unchanged history items. Unversioned standalone fixtures retain the history-hash fallback; durable runtime projections always supply stamped revisions. The poller still scans lightweight thread metadata for lifecycle decisions. Task routing now resolves only actual task parents instead of every saved thread. Global commands and startup may still inspect the full workspace. The legacy execution mode retains its existing JSON stores.

Shina has several transcript arrays rather than KSG's unified turn-item model. It therefore stores a field beside each item and retains the current scoped-history wire contract. Messages get UUIDs when created. Existing intrinsic IDs identify tools and other identified records. Unidentified auxiliary records retain persisted sidecar identities across changes and reorderings. Provider history reconciliation preserves saved message IDs before publishing a replacement transcript.

Each item has a durable per-thread allocation position, separate from its last-change sequence. Allocation is idempotent by identity, and deletion never reuses an allocation position. Cursor version 2 uses this position when an anchor disappears. Readers still accept version 1 cursors. Shina has no unified historical run-item projection, so it uses a flat thread allocator rather than KSG's million-slot run ranges.

The workspace's durable sequence groups a batch's changed items. A scoped thread's change sequences need not be contiguous. KSG also scopes replay over persisted domain-event sequences. Shina retains hard byte fragmentation for oversized rows instead of KSG's oversized single response.

Singleton memory, profile, routine and Honcho stores increment an in-memory document revision when they write. The next runtime batch refreshes their shared projection fields without capturing any thread views. Quiet ticks compare the revision counter rather than serializing these documents.

The native cached view has `schemaVersion: 1`. Its single-slot queue replaces pending views without encoding them on the receive path. The worker performs JSON encoding and atomic-file durability. It follows KSG's initial 500 ms delay and ten-second save cadence, retries failed writes without replacing newer queued state, and reports errors through the existing client error channel. Native shutdown flushes the final queued cache immediately, an addition to KSG's browser cache loop.

KSG caches asset URL requests by environment/resource. Shina has no URL-minting browser layer, so its native adaptation retains one download consumer per linked server and caches completed image and HTML blobs on disk by identity. A newer projection signals the consumer without aborting its transfer. After completion it hydrates only the latest view. The worker retains that source independently of the UI's consumable pending snapshot. Cache validation uses the explicitly selected profile, not a worker thread's default profile. Authentication denial stops asset retries and reports the required manual reconnect.

The server's exclusive SQLite owner lock makes the legacy run-marker heartbeat unnecessary. The marker still distinguishes clean and unclean exits, but a durable owner never rewrites it on an idle minute tick.

## Measurement

Run `cargo test --test server_runtime streaming_storage_scaling -- --nocapture`.

The isolated process fixture uses `assets/tests/fake_pi.cjs`, which emits one text delta every 40 ms. It imports 10, 100 or 500 saved threads. Each inactive thread has 4096 bytes of transcript text. The imported active view has 190000 bytes before the fake provider reconciles its initially empty history. Each case waits three seconds for startup, measures three idle seconds and then three streaming seconds. Both runs negotiate protocol 1.7 subscriptions. The baseline uses source commit `e770532e02d1ca6e6bc30e5212af3e7652610942` in a temporary archive with only byte counters and the same measurement fixture added.

Encoded bytes count persistence serialization in the store/database path, excluding wire encoding and allocations. Storage bytes are bytes appended to SQLite's WAL during commits, including page and frame overhead. Debug measurement disables automatic WAL checkpoints so recycling cannot hide writes. These are logical WAL write bytes, not operating-system physical-device write counters. The observer log itself is outside the measured database. A sequence advance is one committed batch, not necessarily one provider delta.

Before the change, the same fixture measured:

| Saved threads | Commits in three streaming seconds | Encoded bytes per commit | Appended WAL bytes per commit | Idle WAL bytes |
| --- | ---: | ---: | ---: | ---: |
| 10 | 73 | 314440 | 152383 | 0 |
| 100 | 14 | 2996491 | 1241591 | 0 |
| 500 | 3 | 14954077 | 6093480 | 0 |

The regression requires the 500-thread encoded-byte and WAL-byte costs to remain within twice the 10-thread costs. It also requires zero WAL growth over three idle seconds. The test runs in the normal suite.

After the change, the isolated run measured:

| Saved threads | Commits in three streaming seconds | Encoded bytes per commit | Appended WAL bytes per commit | Idle WAL bytes |
| --- | ---: | ---: | ---: | ---: |
| 10 | 74 | 8102 | 57680 | 0 |
| 100 | 72 | 8108 | 62715 | 0 |
| 500 | 62 | 7850 | 64059 | 0 |

The 500-thread case uses 0.97 times the 10-thread encoded bytes and 1.11 times its WAL bytes per commit. Its encoded cost falls by 99.95 percent relative to the baseline, and its WAL cost falls by 98.95 percent. Scheduling and page allocation affect the exact commit counts and WAL bytes. These final measurements use the merged source at `9d22b3b`.

## Validation

The branch incorporates main's remote project scope, composer submission/history, transactional queue editing, stop/resume, keybindings, Markdown editing and composer resting-layout changes. Protocol minor remains 9. Every implementation and merge commit runs `cargo fmt --check`, `cargo build` and `cargo test`. The suite includes the existing recovery, receipt, fail-closed storage, local-send and remote-launch deduplication, and process-crash boundary cases.

Final checks passed on `9d22b3b`, after merging main at `e31bf9ca8107433f641527f399910e2f18e5dfb9`: 935 unit tests and all 45 integration-target tests passed, with 11 existing opt-in unit tests ignored. The complete integration target passed 20 consecutive runs on that exact source with its normal parallel execution, 900 successful test executions. An earlier 20-run pass preceded main's final settings follow-up. An additional full-suite check before the report commit also passed.

New coverage includes persisted item identity/sequence/order round trips over 500 saved threads, unchanged-row write counts, newer row-schema rejection, restart replay without a scoped resnapshot, stable running timestamps, clocks in partial thought windows, singleton-document invalidation, scaling and idle WAL checks. Oversized-item tests cover live publication and persisted replay limits. Another regression includes a quiet thread's newly captured effect in a different thread's streaming checkpoint. Process fixtures require synchronization markers to cover the actual installed snapshot, including a snapshot newer than a coalesced watch notification. Client tests block a cache write while submitting 999 newer views, verify only the latest survives, verify failure handling and exit flushing, and delay a multi-page image while the UI consumes pending snapshots and 29 newer projections arrive. The image test requires one fetch start, one request per byte page and hydration of the newest view.

### Disconnect replay diagnosis

The intermittent `streaming_history_resumes_after_disconnect` failure came from the fixture's reconnect ordering, not a missing server event. Its convenience constructor subscribed at protocol 1.0 and consumed a fresh snapshot before the retained `ProjectionCache` was attached. After negotiating modern patches, the socket received the next sequence for that consumed snapshot. The retained cache still had its older cursor and correctly rejected the gap.

The fixture now opens an unsubscribed socket, installs the retained workspace/cache, negotiates hello, and subscribes once with that cache's epoch and sequence. This follows the native client's reconnect ordering. It does not ignore gaps or relax validation. `reconnect_attaches_the_retained_cache_before_any_workspace_subscription` deterministically reproduces the discarded-snapshot case, proves the wire patch is contiguous with the consumed snapshot, then verifies the corrected reconnect replays without a snapshot. The streaming test still checks the exact complete 250-delta Unicode transcript.

A separate server synchronization-marker issue also received a fix. If committed SQLite state was ahead of the coalesced watch notification, the marker could advertise the old watch cursor after sending a newer snapshot. It now reports the snapshot's actual cursor.

The repeated suite also exposed a stale predicate in the queue-edit fixture. Its second one-item-queue wait could match the earlier edited queue before the new command's projection arrived. It now waits for the receipt's committed sequence and the replacement text.

### Review commits

| Commit | Change |
| --- | --- |
| `bd5a04d` | Scoped item storage, durable identity and restart replay |
| `2eaa688` | Coalesced client cache and durable item positions |
| `931189d` | Main merge, including durable run metadata and cold repository preparation |
| `894efa5` | Auxiliary row storage, independent asset delivery and reconnect fixture diagnosis |
| `7538783` | Main keybindings and Markdown editing merge |
| `9f8f4b2` | Singleton-document projection invalidation |
| `aba554d` | Main composer behavior merge |
| `3767e64` | Queue fixture waits for its committed replacement |
| `9d22b3b` | Main keybinding settings follow-up, with client-only file polling |

No installed app, running user process, real profile or CPA service is part of these tests. Windows/Linux native builds, real provider runs, physical-device I/O counters and deployed mixed-build peers remain unverified.
