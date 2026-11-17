# Long-turn virtualization

Issue #26. This branch starts from #25's unpublished `work/timeline-structure-cache`
at `f3294b6`, merged as `ea68d9c`. It changes no palette, fonts, radii or icons.

KSG Code's `MessagesTimeline`, `useTimelineRows` and `TimelineWorkGroupRows` split a
turn into separate work, live, toggle and prose rows. KSG's expanded work group uses a
nested `LegendList`. Issue #26 asks for one scroll viewport instead, so Shina keeps a
single Rune virtual list and gives it finer rows.

## What changed

A turn stays one row until it has 48 work entries, and splits only while it runs or
while its "Worked for" fold is open with its history loaded. A split turn becomes:

- a head row, keyed `turn-{thread}-{message}` as before, with the captain line, the
  preamble and the "Worked for" toggle;
- work slices of 4 entries, keyed by the first entry's stable id (tool id, thought
  block and offset, or message id and byte offset), in `work_items` order;
- a tail row with the live status line, or with lifted cards, the answer and the reply
  action.

An entry is one transcript part, or the heading and each row of an expanded tool
group or thought run. `timeline_structure` builds the rows with #25's indexes, so the
row collection and Rune's `virtual_rows_version` change only with structure or with a
long turn's fold flags. `disclosed_timeline_structure` compares those flags with the
ones the rows were built from, so handlers, restored projections and find's reveal
need no extra bookkeeping. Short turns' folds never rebuild the indexes.

Rune measures a row's border box and stacks rows 20 pt apart. Slices and tails rise
into that gap with a negative margin inside their own row box, so measured heights
still match where rows sit. Threads without split turns have the same geometry as
before. `split_turns_keep_the_unsplit_layout` compares every mounted entry's position
in a split turn with the same turn unsplit, live and completed, at all four widths.

Slices memoize on a fingerprint of their entries' visible fields. A streamed fragment
or a new step rebuilds none of the mounted completed slices. Offscreen slices are not
built, so their markdown and tool output are not parsed or laid out.

Find's reveal and citation navigation use `timeline_text_row` to scroll to the head,
slice or tail that holds the text. Handlers in entries keep resolving tools, thoughts
and messages by id.

## Acceptance

| Acceptance | Evidence |
| --- | --- |
| Live and completed-expanded fixtures with at least 1,000 parts and large outputs | `long_turn_fixture`: 1,000 alternating thought and command parts, 20 two-command groups, prose between the halves, 21 KB of output per command and every fifth output open. Scenes `long-turn-live` and `long-turn-expanded` use it. |
| Mounted work proportional to the viewport | `long_turns_mount_work_in_proportion_to_the_viewport`: at 620, 960, 1280 and 1920 px, fewer than 400 entries mount, and 2,000 parts mount within 8 entries of 1,000 parts. A theme change rebuilds under half the slices. Headless 1280 × 720: about 1,060 mounted entries before, 45 to 55 after. |
| Scroll stability: streaming, disclosure, deferred load, resize, search | `long_turns_hold_the_reader_through_streaming_disclosure_and_resize` and `expanded_long_turns_load_deferred_work_and_reveal_search_hits_in_place`, at four widths. A visible row stays put through streamed text and a new step, a thought opening, and a tool output opening and closing. A resize keeps it on screen. Loading deferred work keeps the toggle in place. Find lands the active match on screen in the slice that holds it. |
| Copy, export and search keep complete content | Rows hold indices and keys only. `long_turns_keep_complete_logical_content` checks that the slices cover every entry once, in order, and that the thread keeps all text, tools and output. Search runs on thread data. Copy and export read the thread, as before. Text selection covers built rows only, as it already did at turn level. |
| Small turns keep their layout | `small_turns_keep_one_row`, the parity test, and the existing timeline, find, citation, reconnect, anchor and work tests (334 selected by name) pass. |

## Measurements

The raw numbers are in `long-turn-benchmark.json`. Same host for all runs: a Mac
Studio with an Apple M5 Max on macOS 27.2.0. "Before" is `ea68d9c` with only the
fixture, scenes and benchmark added, in a separate worktree.

Headless, release, 1280 × 720, `long_turn_view_benchmark`, 40 samples per phase. This
measures view, commit and layout CPU, plus allocator calls from the test build's
counting allocator. It does not measure GPU, presentation or FPS.

| Fixture, phase | Frame p50 / p95 ms, before | After | Allocations p50, before | After |
| --- | --- | --- | --- | --- |
| Live, scroll | 1.090 / 1.243 | 0.152 / 0.167 | 82 | 120 |
| Live, stream while scrolling | 5.434 / 7.300 | 0.697 / 0.890 | 41,198 | 19,693 |
| Live, open and close an output | 6.503 / 7.567 | 0.874 / 0.945 | 43,498 | 20,516 |
| Expanded, scroll | 1.027 / 1.081 | 0.131 / 0.134 | 75 | 75 |
| Expanded, open and close an output | 6.634 / 7.787 | 0.920 / 1.075 | 48,761 | 26,653 |

A scroll frame now makes about 40 more allocator calls in the live fixture. It mounts
more rows, each with its own layout callback.

Native, release with `--features rune/frame-profile`, a 1280 × 720 window, isolated
`XDG_DATA_HOME`. Computer Use sent the same 60 real scroll events to each build. These
numbers cover only the frames that handled input.

| Scene | Frame CPU p50 / p95 ms, before | After | Nodes p50, before | After |
| --- | --- | --- | --- | --- |
| `long-turn-live` | 8.223 / 9.029 | 1.166 / 1.473 | 8,812 | 582 |
| `long-turn-expanded` | 2.477 / 3.474 | 0.571 / 0.987 | 8,916 | 663 |

## Limits

- The display reports one mode, 1920 × 1080 at 60.00 Hz. No high-refresh run was
  possible, and these logs have no presentation timestamps, so this establishes no
  achieved frame rate. The native frame log has no per-frame allocation counts.
  Allocation figures come only from the headless counting allocator.
- The native runs cover scrolling. Streaming and disclosure have no native input path
  without a running agent, so their before/after figures are headless only.
- No KSG measurement was taken. Comparing with KSG needs the same 1,000-part content
  seeded into a KSG server and its browser timeline measured on this host. This record
  makes no speed claim relative to KSG.
- Rune chooses a window from its 90 pt row estimate before rows are measured, so
  mounted entries depend on the viewport and that estimate, not on the turn's length.
- A completed long turn with no prose, no cards and no reply action (demo or archived)
  gets 20 pt of extra space after its last slice.
