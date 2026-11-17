# Matched transcript projection benchmark

The measured client pipeline is cheaper at every tested size. These are CPU
application measurements, not native frame timings or a refresh-rate claim.
The complete values and provenance are in [projection-benchmark.json](projection-benchmark.json).

## Sources and workload

The before binary compiles published main `5c6e98a`, before the #24 client
optimization. Its production sources are unchanged. Eleven added lines in
`src/client_state.rs` and `src/main.rs` attach only the shared test driver and
test allocator. The after binary compiles `3443103`; the sole uncommitted
tracked change at measurement time was implementation documentation.

Both binaries use identical copies of `tools/projection-benchmark.rs` and
`tools/projection-benchmark-alloc.rs`. The driver applies actual history
notifications from `Streams` through each version's own `receive`, `install`,
`projected` and `State::apply_projection`. Server fixture generation, loading
initial history and correctness assertions are outside the timed regions.
The old `legacy_install` reference is not used for these before measurements.

`tools/projection-workload.mjs` generates one deterministic JSON file consumed
by both Rust binaries and the KSG runner. Its SHA-256 is
`b04fa3e33dd337e89abbf9fed260bc09ee5eeda8482614453a93612a90005a99`.
The cases have 520, 5,200 and 20,800 total message/tool items, including all
loaded threads. Forty alternating message/tool content changes preserve item
membership. Ten warmup changes leave fifteen measured changes of each kind.
Both clients assert the changed payload and complete logical item count.

The same Apple M5 Max host, 18 logical CPUs, macOS 27.2.1 and Rust 1.98.1 ran
both release binaries with thin LTO and one codegen unit. Allocator counts
include allocation/reallocation calls. Byte totals count allocation sizes and
positive reallocation growth, not retained heap or process RSS. Initial load
is excluded. Counting is enabled during both timed Rust runs, so latency
includes counter overhead; uninstrumented latency was not measured. These runs
do not estimate confidence intervals or a noise floor.

## Rust results

Times are microseconds per update. Allocation and byte columns are medians;
their p95 values are equal in these samples and are retained in the JSON.

| Total items | Update | Before p50 / p95 | After p50 / p95 | Before / after allocation calls | Before / after bytes |
| ---: | --- | ---: | ---: | ---: | ---: |
| 520 | Message | 408.041 / 449.625 | 51.042 / 59.166 | 16,242 / 1,305 | 2,149,697 / 191,760 |
| 520 | Tool | 383.250 / 494.375 | 53.458 / 59.000 | 16,261 / 1,361 | 2,153,764 / 193,252 |
| 5,200 | Message | 3,418.083 / 3,612.375 | 150.125 / 179.583 | 139,884 / 3,001 | 17,975,686 / 467,812 |
| 5,200 | Tool | 3,475.000 / 3,654.541 | 147.791 / 177.375 | 139,903 / 3,057 | 17,979,757 / 437,953 |
| 20,800 | Message | 17,131.917 / 18,659.042 | 336.959 / 383.167 | 555,992 / 6,901 | 71,330,015 / 1,077,016 |
| 20,800 | Tool | 17,276.209 / 18,280.041 | 301.333 / 334.125 | 556,011 / 6,957 | 71,334,087 / 929,550 |

Including cache installation exposed an avoidable re-anchor pass on every
text-only message edit. The candidate now re-anchors only when the message
ordinal map changes, while still remapping fresh compaction anchors after
installing a nonzero message window. Regressions pin both behaviors.

## KSG reference

The runner requires a clean checkout at
`4807bb1e7e74baae3d0fe9c442c67abadd174a36` before and after measurement.
This replaces the previous unsupported claim that the installed checkout
was pinned to `87aa452b4f`. The run records the lockfile hash, Effect 4.1.3
and Node v26.9.0.

The runner maps the same messages to user/assistant turn items and the same
tool input/output records to command-execution items. Item ids, transcript
sizes, payload content and update sequence match the shared artifact.
KSG measures `applyOrchestrationV2ProjectionEvent`, not Shina's additional
client-model application. No cross-runtime speed ratio is claimed.

| Total items | Message p50 / p95, microseconds | Tool p50 / p95, microseconds |
| ---: | ---: | ---: |
| 520 | 1.166 / 4.250 | 3.458 / 5.708 |
| 5,200 | 1.209 / 3.333 | 9.666 / 19.000 |
| 20,800 | 1.917 / 7.666 | 22.000 / 32.000 |

V8 allocation-call counts were not measured. They are null, not zero.
Earlier-history/background-stream concurrency has separate correctness tests;
these numbers do not claim to measure that mixed scenario.

## Reproduction

Generate a new artifact without overwriting an existing file:

```sh
bench_dir=$(mktemp -d /tmp/shina-projection-benchmark.XXXXXX)
node tools/projection-workload.mjs "$bench_dir/workload.json"
SHINA_PROJECTION_WORKLOAD="$bench_dir/workload.json" \
SHINA_PROJECTION_EXPECT_REVISION="$(git rev-parse HEAD)" \
cargo test --offline --release --bin shina \
  model::client_state::projection_benchmark::matched_transcript_replay \
  -- --ignored --exact --test-threads=1 --nocapture
KSG_ROOT=/path/to/clean/pinned/ksgcode \
node tools/ksg-projection-reference.mjs "$bench_dir/workload.json"
```

For the before run, use a separate detached checkout of `5c6e98a`. Add the
same cfg(test) module declaration to `src/client_state.rs`, the same cfg(test)
allocator declaration to `src/main.rs`, and point them at the shared driver
and allocator files. No production implementation is copied into a proxy.
Inspect and retain that instrumentation-only diff. Run the exact same test
with expected revision `5c6e98a13bee2b7d9dcb0986418e1af1215df423`.
Require matching fixture, driver and allocator hashes in both outputs.

Keep checkout build directories isolated. Sharing one build directory reused
the historical executable during an attempted after run; the source-check
assertion rejected it before measurement. The final after run followed a
package-only clean and fresh compilation. The rejected run is excluded.

Run no paid-provider tests or real profiles. A successful result must report
one passed test, not a zero-test filter match. This opt-in benchmark is ignored
by normal suites. #25 and #26 still own timeline caching and native-frame work.
