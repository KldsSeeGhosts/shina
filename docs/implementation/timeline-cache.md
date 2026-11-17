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

The four-width cached-turn regression passes. Background streaming rebuilds
zero completed turns, keys or indexes; disclosure, theme and width changes
invalidate the cached presentation. The existing combined reconnect/cache-reset
regression and independent tool/group-output test also pass.

`timeline_view_benchmark` is an opt-in headless CPU benchmark with separate
scroll, stream-while-scroll and prepend phases and view/key/row/turn/index
counters. It does not measure server receive, GPU timing or FPS.

Remaining acceptance work includes running and recording that benchmark,
checking stable selection identities during prepends and the normal CI/release
gates. This branch does not yet complete #25.
