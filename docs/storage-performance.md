# Scoped persistence and streaming

## Layout and transaction boundary

Previously `documents` held complete index and server-input JSON documents. The index embedded every thread view. `projection` held another complete workspace projection. A streaming reducer batch captured and serialized those documents repeatedly, rebuilt the workspace projection and hashed each history before producing its small wire patch.

Schema 5 adds `projection_records`, keyed by document, scope and record key. Root properties are separate rows. Thread scopes contain metadata or a view header. History item rows use the field and durable identity as their key. Their versioned payload contains the item, ordinal, identity and last-change sequence. Runtime inputs and pending messages have thread scopes too. Recovery hydrates the existing reducer's input shape from these rows. `projection` retains the committed workspace sequence, rather than a full JSON workspace.

`thread_events` stores versioned, thread-scoped item batches in the same transaction. Workspace replay retains the existing 1024-sequence policy. Thread replay retains its existing 128-event and 1 MiB admission limits, and its 1024-batch, 4 MiB memory limit. Old cursors receive a bounded scoped catch-up snapshot when the retained range is insufficient. An idle thread at its high-water mark synchronizes without one.

Commands still commit state, receipt, captured effects and replay data atomically before dispatch or publication. Receipt history and its device/command primary key remain independent of replay retention. Uncertain process-bound effects become interrupted on recovery. Neither recovery nor a failed commit retries a provider send.

## KSG mapping

| Mechanism | KSG source | Shina implementation |
| --- | --- | --- |
| Identity-keyed item rows and ordinals | `apps/server/src/orchestration-v2/ProjectionStore.ts`, `turn-item.updated` | `projection_store::write`, `history::stamp`, `MessageView::capture` |
| Stored item identities across recovery | `apps/server/src/orchestration-v2/Orchestrator.ts`, item ID allocation | `Message::id`, `Thread::apply` history reconciliation |
| Events, projections, receipts and effects in one transaction | `apps/server/src/orchestration-v2/EventSink.ts` | `Database::commit`, `RuntimeState::persist` |
| Post-commit item publication | `apps/server/src/orchestration-v2/Orchestrator.ts` | `Streams::ingest`, server runtime publication |
| Durable scoped high-water marks and bounded replay | `apps/server/src/orchestration-v2/ThreadStream.ts`, `ProjectionStore.ts` | `Database::history_events_after`, `Streams::restore`, `Streams::replay` |
| Identity-anchored recent and older windows | `apps/server/src/orchestration-v2/threadHistoryPaging.ts` | `Streams::page` |
| Stable item start timestamps | `packages/contracts/src/orchestrationV2.ts`, turn-item fields | `ThreadView::capture`, `ThreadView::apply` |

Shina keeps its existing reducer rather than replacing it with KSG's typed domain-event reducer. Polling records dirty thread scopes, and the row writer compares the selected thread's records. It does not serialize or hash unchanged history items. Global commands and startup may still inspect the full workspace. The legacy execution mode retains its existing JSON stores.

Shina has several transcript arrays rather than KSG's unified turn-item model. It therefore stores a field beside each item and retains the current scoped-history wire contract. Messages get UUIDs when created. Existing intrinsic IDs identify tools and other identified records. Unidentified auxiliary records retain persisted sidecar identities across changes and reorderings. Provider history reconciliation preserves saved message IDs before publishing a replacement transcript.

Shina uses the workspace's durable sequence as the position shared by a batch's changed thread items. Positions within a thread need not be contiguous. KSG also scopes replay by thread over persisted domain-event positions. Shina retains hard byte fragmentation for oversized rows instead of KSG's oversized single response.

## Measurement

Run `cargo test --test server_runtime streaming_storage_scaling -- --nocapture`.

The isolated process fixture uses `assets/tests/fake_pi.cjs`, which emits one text delta every 40 ms. It imports 10, 100 or 500 saved threads. Each inactive thread has 4096 bytes of transcript text. The imported active view has 190000 bytes before the fake provider reconciles its initially empty history. Each case waits three seconds for startup, measures three idle seconds and then three streaming seconds.

Encoded bytes count persistence serialization in the store/database path, excluding wire encoding and allocations. Storage bytes are bytes appended to SQLite's WAL during commits, including page and frame overhead. Debug measurement disables automatic WAL checkpoints so recycling cannot hide writes. These are logical WAL write bytes, not operating-system physical-device write counters. The observer log itself is outside the measured database. A sequence advance is one committed batch, not necessarily one provider delta.

Before the change, the same fixture measured:

| Saved threads | Commits in three streaming seconds | Encoded bytes per commit | Appended WAL bytes per commit | Idle WAL bytes |
| --- | ---: | ---: | ---: | ---: |
| 10 | 73 | 314439 | 152383 | 0 |
| 100 | 17 | 2996526 | 1241331 | 0 |
| 500 | 3 | 14954048 | 6093480 | 0 |

The regression requires the 500-thread encoded-byte and WAL-byte costs to remain within twice the 10-thread costs. It also requires zero WAL growth over three idle seconds. The test runs in the normal suite.

After the change, the isolated run measured:

| Saved threads | Commits in three streaming seconds | Encoded bytes per commit | Appended WAL bytes per commit | Idle WAL bytes |
| --- | ---: | ---: | ---: | ---: |
| 10 | 72 | 6738 | 48295 | 0 |
| 100 | 72 | 6738 | 56650 | 0 |
| 500 | 62 | 6483 | 55154 | 0 |

The 500-thread case uses 0.96 times the 10-thread encoded bytes and 1.14 times its WAL bytes per commit. Its encoded cost falls by 99.96 percent relative to the baseline, and its WAL cost falls by 99.09 percent. Scheduling and page allocation affect the exact commit counts and WAL bytes.
