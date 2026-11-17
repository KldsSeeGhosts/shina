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

Remaining acceptance work is visible-turn memoization with complete
presentation/disclosure invalidation, stable handler verification and the
scroll/stream/prepend benchmark with view, mount and key-call counters. Existing
search, selection, sticky-bottom and anchor tests must still pass. This first
change alone does not complete #25 and makes no native frame-rate claim.
