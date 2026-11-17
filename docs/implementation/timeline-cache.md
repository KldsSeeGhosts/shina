# Timeline structure cache

Issue #25 is in progress. This branch starts from merged main `14191af`.
It changes no layout, fonts, palette or interaction design.

The reference is KSG Code's `useIncrementalTimelineEntries`,
`useIncrementalTimelineRows` and memoized `MessagesTimeline`, read from the
existing pinned source copies in `/tmp/ksg-pin`.

The first change caches row keys, ordered tool/thought/compaction/retry indexes
and boundary events per thread. The input uses #24's structural revision plus
the small record fields that add, remove or position rows. Text changes retain
the same row collection and Rune virtual-row version. A turn queries its
indexed event range instead of scanning and sorting every tool and thought.

The cache contains keys and ordinals only. It does not keep old transcript
payloads alive. Native headless fixtures and untracked demo threads retain the
complete rebuild fallback. The normal desktop receives revisioned server
projections.

Structural invalidation also includes tool-name and stopped-work changes.
History prepends, retry/compaction changes, role/anchor/order changes and
snapshot replacement rebuild the indexes. Existing row keys remain unchanged.

Completed visible turns now use Rune memos. Projection item deserialization
notifies only the owning turn, so it stays correct across coalesced emissions
without rescanning other turns. Work, tool/group/raw and thought disclosure
handlers update that turn's presentation revision. Width, theme, role,
archival, Captain header, search/citation and compaction disclosure also join
the memo version. Live turns and externally changing task/plan/HTML cards
remain dynamic. Work records use the same indexed lookup.

The raw tool handler now resolves its stable tool id, not its captured vector
index. Missing message identities no longer make thought/work handlers act on
an unrelated positional replacement.

Message prose, user bubbles and work containers now use stable message ids in
their element keys. Citation selection resolves that id after a prepend, and
Cite resolves the current message by id instead of using a captured ordinal.
The cached four-width regression retains native selected text across a
prepend. The existing citation round-trip test now prepends history before
Cite and still inserts the right message's citation.

Live status reads a cached set of unfinished tools. Item updates maintain that
set without rescanning completed tools. Deferred-work flags use the structural
index, and nonempty conversations do not scan tools for the empty-state check.

The four-width cached-turn regression passes. Background streaming rebuilds
zero completed turns, keys or indexes; disclosure, theme and width changes
invalidate the cached presentation. The existing combined reconnect/cache-reset
regression and independent tool/group-output test also pass.

`timeline_view_benchmark` is an opt-in headless CPU benchmark with separate
scroll, stream-while-scroll and prepend phases and view/key/row/turn/index
counters. It does not measure server receive, GPU timing or FPS.

The first benchmark exposed an existing scroll-only view invalidation:
`Positions::open` read every row's window geometry on each ordinary build,
then discarded the result. Those observations made scrolling rebuild the
view. Position capture now runs only when switching threads or preserving
an anchor before history unloads. Normal scroll frames do not observe row
window positions through `Cx`. Row and viewport layout callbacks keep only
their latest rectangles, and the visible-row callback keeps their current
identities. Those callbacks update bookkeeping without rebuilding the view.
This also avoids querying geometry that Rune has not yet recorded on the
first unloaded frame. The combined reconnect/cache-reset regression passes
with this callback-based anchor capture.

The recorded run at `2f32971` is in `timeline-benchmark.json`. It uses 100,
1,000 and 5,000 turns with 20 samples per phase. Scrolling makes two view
builds across 20 samples, not one per wheel frame. Both scroll phases perform
zero key walks, index rebuilds or completed-turn builds. The final mounted
window is 30 rows at all sizes. Prepending rebuilds the structure as required.

| Turns | Scroll p50/p95 ms | Stream + scroll p50/p95 ms | Prepend p50/p95 ms |
| --- | --- | --- | --- |
| 100 | 2.393 / 7.353 | 4.441 / 4.971 | 13.021 / 16.190 |
| 1,000 | 1.946 / 5.664 | 4.388 / 4.702 | 17.058 / 19.674 |
| 5,000 | 2.078 / 6.801 | 4.811 / 5.181 | 34.935 / 40.928 |

These are debug-build CPU measurements, not production frame-rate claims.
The existing search-navigation and sticky-bottom tests pass, as do the
four-width cached-turn test, citation round-trip and combined reconnect/cache
reset regression. All-target checking, formatting and diff checks pass.

| Acceptance | Evidence |
| --- | --- |
| Scroll without a full view/key walk per wheel frame | Callback-based position capture and benchmark view/key/index counters |
| Active-turn content does not rebuild completed turns or row keys | Per-item owning-turn revisions, cached unfinished tools and cached-turn counters |
| Prepends, boundaries, retries, compactions and return/reconnect invalidate correctly | Structural input keys, indexed events, structure regression and existing reconnect/anchor coverage |
| Stable handlers after prepending | Stable message/tool ids; native selection/prepend and citation/reply coverage |
| Separate scroll, stream and prepend benchmark | Recorded fixture sizes, CPU p50/p95 and view/key/row/turn/index counts |
| Search, selection, sticky bottom and reader anchor | Existing search/sticky-bottom tests plus four-width cache and reconnect regressions |

Exact-head CI and downloaded release verification remain required. Issue25
stays open. This does not claim finer tool-row virtualization or completion
of issue26.
