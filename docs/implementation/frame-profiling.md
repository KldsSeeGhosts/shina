# Frame profiling evidence

`node tools/frame-profile.mjs` summarizes JSONL from the pinned Rune revision,
`9a486a4608c06973b52c2388fe213ff6cc175590`. It does not launch an app, alter a
profile, or establish a performance improvement. Its tests use synthetic
records, not benchmark results.

The three input formats have different meanings:

| Kind | Input | Units and limits |
| --- | --- | --- |
| `headless` | `RUNE_FRAME_LOG` from a build with `rune/frame-profile` | Timings are milliseconds. Allocation counts require an active counting allocator. Virtual `--fps` is not an observed display rate. |
| `native` | `RUNE_FRAME_LOG` from the same feature in a live window | Timings are nanoseconds, converted to milliseconds. `nodes`, `shaped` and `instances` are counts. This log has no per-frame allocation count or actual presentation timestamps. |
| `trace` | `--trace` JSONL | `clock: wall` identifies native submissions. Actual `present_ms` observations are separate and may be unavailable on the platform. `quality.cpu_ms` measures wall time despite its name. |

Example commands, after collecting the recordings:

```sh
node tools/frame-profile.mjs --kind headless --input /tmp/headless.jsonl --skip 1
node tools/frame-profile.mjs --kind native --input /tmp/native.jsonl --skip 1 --refresh-hz 60
node tools/frame-profile.mjs --kind trace --input /tmp/trace.jsonl --skip 1 --refresh-hz 60
```

Add `--counted-allocations` only after verifying that the measured application
installed `procwatch::alloc::Counting` as its allocator. The report labels this
as a caller declaration, not independent verification. Without it the report
excludes allocation fields, even if the input contains zeros.

The report includes the exact input SHA-256, sample counts, missing counts,
nearest-rank p50/p95/p99 and maximum. Warmup removal defaults to one frame.
`--refresh-hz` is the caller's declared reference budget, not a measurement.
Counts above that budget are raw threshold comparisons, not inferred dropped
frames. In particular, rounded nominal presentation intervals can exceed an
exact fractional budget by a fraction of a millisecond.

Native cadence uses only Rune's recorded `present_dt_ms`. Rune emits that
field for adjacent continuous presentations. The tool never derives it from
CPU submission time, display-link targets, or a gap across idle, skipped or
unobserved frames. Missing presentation evidence remains missing. Phase times
overlap and must not be added.

Keep each original recording with its source commit, binary hash, Rune
revision, fixture size and content identity, build mode, allocator, host,
display mode, window size, scale and scenario. The report's `analysisHost`
describes the machine processing the file, which need not be its capture host.
A file hash does not establish capture completion or matched inputs.

For #25/#26, record scroll-only, streaming while scrolling, history prepend,
and expansion separately. Test both live and completed expanded turns with
at least 1,000 tool/thought parts. Include view/key/index counters and mounted
work elements. Compare the same content and hardware before and after. Rune's
script replay is headless-only, so a native interaction recording needs actual
input. Always use an isolated `XDG_DATA_HOME` on macOS/Linux or `LOCALAPPDATA`
on Windows; do not rely on a scene name alone to protect the user's profile.

A same-host KSG comparison also requires matched seeded content and scenarios.
Keep its browser measurements separate from Rune's phases. Do not report a
speed ratio from source inspection or unmatched counters. The Mac Studio's
display inventory on November 16, 2026 reports 1920 × 1080 at 60 Hz. That is
evidence for an available 60 Hz mode, not evidence for high-refresh support or
any app's achieved cadence.

The parser rejects mixed formats, malformed records and recordings over
100,000 frames. It does not silently treat absent metrics as zero.

## Format smoke checks

Root tested the tool against real traces from the unchanged `work-running`
demo at source commit `4567c79b530202e6a2324b9a123010f28285496c`, using a
throwaway `XDG_DATA_HOME`. The three-frame headless trace reported a virtual
clock. The twelve-frame native trace reported a wall clock, but every
presentation observation was null. The tool reported no observed presentation
intervals and warned that submission timing cannot prove display cadence.
Both processes exited. These short debug-build checks validate parsing only.
They do not establish a native frame rate, an allocation profile, or any
before/after improvement. A later native benchmark still needs usable
presentation observations or an explicitly limited timing claim.
