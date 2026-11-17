// Issue-24 KSG reference: matched logical workload against the pinned KSG
// client-runtime (87aa452b4f, unchanged through HEAD 64d7814e6e).
//
// Measures what the JS client pays for the same operations Shina's Rust
// `ProjectionCache` delta path now handles:
//   * `applyOrchestrationV2ProjectionEvent` for one `turn-item.updated` — the
//     live streaming update path.
//   * `mergeOlderHistoryIntoProjection` for one older-history page — the
//     "user reads older history while a background thread streams" path.
//
// These are logical matched workloads, not apples-to-apples timings: JS
// numbers and the Rust `delta_application_benchmark_report` numbers are
// reported separately and no cross-runtime speed ratio is claimed.
//
// Run: node tools/ksg-projection-reference.mjs
//   KSG_ROOT=/path/to/ksgcode overrides the default checkout location.

import { performance } from "node:perf_hooks";

const KSG_ROOT = process.env.KSG_ROOT ?? "/Volumes/DevDrive/AiStack/ksgcode";
const SRC = `${KSG_ROOT}/packages/client-runtime/src/state`;

const { applyOrchestrationV2ProjectionEvent } = await import(`file://${SRC}/orchestrationV2Projection.ts`);
const { mergeOlderHistoryIntoProjection } = await import(`file://${SRC}/threadHistoryMerge.ts`);
const { v2Projection, v2ThreadId, v2Now } = await import(`file://${SRC}/orchestrationV2TestFixtures.ts`);

const runId = "run-1";

function commandItem(id, ordinal, output = "done") {
  return {
    id: `item-${id}`,
    threadId: v2ThreadId,
    runId,
    nodeId: null,
    providerThreadId: null,
    providerTurnId: null,
    nativeItemRef: null,
    parentItemId: null,
    ordinal,
    status: "completed",
    title: null,
    startedAt: v2Now,
    completedAt: v2Now,
    updatedAt: v2Now,
    type: "command_execution",
    input: "pwd",
    output,
    exitCode: 0,
  };
}

const run = {
  id: runId,
  threadId: v2ThreadId,
  status: "completed",
  title: null,
  providerInstanceId: "codex",
  startedAt: v2Now,
  completedAt: v2Now,
};

// A fully loaded thread: `items` turn items visible from ordinal 0.
function loadedProjection(items) {
  const turnItems = [];
  const visibleTurnItems = [];
  for (let i = 0; i < items; i += 1) {
    const item = commandItem(i, i, `output ${i} ${"x".repeat(200)}`);
    turnItems.push(item);
    visibleTurnItems.push({
      position: i,
      visibility: "local",
      sourceThreadId: v2ThreadId,
      sourceItemId: item.id,
      item,
    });
  }
  return { ...v2Projection, runs: [run], turnItems, visibleTurnItems };
}

// `older` rows that precede the loaded window — what an earlier-history page
// carries: items the projection has never seen, interleaved ahead of it.
function olderPage(count) {
  const page = [];
  for (let i = 0; i < count; i += 1) {
    const item = commandItem(`older-${i}`, -count + i, `older ${i} ${"y".repeat(200)}`);
    page.push({
      position: 0,
      visibility: "local",
      sourceThreadId: v2ThreadId,
      sourceItemId: item.id,
      item,
    });
  }
  return page;
}

function pct(sorted, p) {
  return sorted[Math.floor(sorted.length * p / 100)].toFixed(1);
}

const UPDATES = 60;
console.log("issue-24 KSG reference (87aa452b4f logical workload), us/operation");
console.log(
  `${"items".padStart(6)} | ${"upd_p50".padStart(9)} ${"upd_p95".padStart(9)} | ${"merge_p50".padStart(9)} ${"merge_p95".padStart(9)}`,
);

for (const items of [260, 1300, 5200]) {
  // A loaded projection and a 40-item older page, matching the Rust fixture's
  // tool-rich sizes (2x200+60, 4x1000+300, 4x4000+1200 total items).
  const projection = loadedProjection(items);
  const page = olderPage(40);

  const upd = [];
  const merge = [];
  let current = projection;
  for (let iter = 0; iter < UPDATES; iter += 1) {
    const last = current.turnItems[current.turnItems.length - 1];
    const updated = { ...last, output: `delta ${iter} ${"z".repeat(200)}`, updatedAt: v2Now };
    const event = {
      id: `event-${iter}`,
      type: "turn-item.updated",
      threadId: v2ThreadId,
      runId,
      occurredAt: v2Now,
      payload: updated,
    };
    let start = performance.now();
    current = applyOrchestrationV2ProjectionEvent(current, event);
    upd.push((performance.now() - start) * 1000);

    start = performance.now();
    current = mergeOlderHistoryIntoProjection(current, page);
    merge.push((performance.now() - start) * 1000);
  }
  upd.sort((a, b) => a - b);
  merge.sort((a, b) => a - b);
  console.log(
    `${String(items).padStart(6)} | ${pct(upd, 50).padStart(9)} ${pct(upd, 95).padStart(9)} | ${pct(merge, 50).padStart(9)} ${pct(merge, 95).padStart(9)}`,
  );
}
