import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { distribution, parseArgs, summarizeFile, summarizeFrames } from "./frame-profile.mjs";

// Synthetic schema fixtures, not measured performance results.
const lines = rows => rows.map(row => JSON.stringify(row));
const headless = (k, ms, extra = {}) => ({ k, ms, update_ms: ms, encode_ms: 0, ...extra });
const trace = (t, extra = {}) => ({ t, n: 1, quality: { cpu_ms: 2 }, ...extra });
const header = clock => ({ type: "header", clock, hz: 120 });

test("nearest-rank quantiles keep units, missing counts and the reference budget", () => {
  assert.deepEqual(distribution([3, 1, 2, 100], "ms", 2, 16), {
    unit: "ms", samples: 4, missing: 2, p50: 2, p95: 100, p99: 100, max: 100,
    referenceBudgetMs: 16, samplesAboveReferenceBudget: 1,
  });
  assert.deepEqual(distribution([], "count", 4), { unit: "count", samples: 0, missing: 4 });
});

test("headless warmup, allocation declaration and closing count stay explicit", async () => {
  const input = lines([
    headless(0, 999, { allocations: 999 }),
    headless(1, 2, { allocations: 0, commits: 0 }),
    headless(2, 4, { allocations: 12, commits: 1 }),
    { end: true, frames: 3, resident: 1000 },
  ]);
  const unverified = await summarizeFrames(input, { kind: "headless" });
  assert.equal(unverified.measuredFrames, 2);
  assert.equal(unverified.metrics.ms.max, 4);
  assert.equal(unverified.metrics.allocations, undefined);
  assert.equal(unverified.metrics.cpu_ms.samples, 0);
  assert.equal(unverified.closingRecordObserved, true);
  const counted = await summarizeFrames(input, { kind: "headless", countedAllocations: true });
  assert.equal(counted.metrics.allocations.max, 12);
  assert.equal(counted.metrics.allocations.p50, 0);
  assert.equal(counted.allocationCounter, "declared-active-not-independently-verified");
});

test("native phase nanoseconds become milliseconds without adding overlapping stages", async () => {
  const report = await summarizeFrames(lines([
    { t: 1, total: 20e6, build: 12e6, layout: 8e6, text: 4e6, cpu: 10e6, nodes: 200 },
    { t: 2, total: 10e6, build: 8e6, layout: 6e6, text: 3e6, cpu: 7e6, nodes: 100 },
  ]), { kind: "native", skip: 0, refreshHz: 60 });
  assert.equal(report.metrics.total.p95, 20);
  assert.equal(report.metrics.total.samplesAboveReferenceBudget, 1);
  assert.equal(report.metrics.build.p95, 12);
  assert.equal(report.metrics.nodes.p95, 200);
  assert.equal(report.metrics.input.samples, 0);
  assert.equal(report.presentation, null);
  assert.equal(report.metrics.allocations, undefined);
});

test("trace cadence uses only recorded continuous actual presentation intervals", async () => {
  const report = await summarizeFrames(lines([
    header("wall"),
    trace(0, { present_ms: 1 }),
    trace(8, { present_ms: 9, present_dt_ms: 8 }),
    trace(1000, { present_ms: 1001 }), // idle gap, not a hitch sample
    trace(1008, { present_ms: null }), // skipped drawable breaks cadence
    trace(1016), // no supported presentation observation
    trace(1024, { present_ms: 1025 }),
  ]), { kind: "trace", skip: 0, refreshHz: 120 });
  assert.deepEqual(report.presentation, { presented: 4, skipped: 1, unknown: 1 });
  assert.equal(report.metrics.present_dt_ms.samples, 1);
  assert.equal(report.metrics.present_dt_ms.missing, 5);
  assert.equal(report.metrics.present_dt_ms.max, 8);
  assert.equal(report.metrics.submit_work_wall_ms.p50, 2);
  assert.equal(report.fps, undefined);
});

test("virtual trace Hz and CPU submissions never become native presentation evidence", async () => {
  const virtual = await summarizeFrames(lines([header("virtual"), trace(0), trace(8)]), {
    kind: "trace", skip: 0, refreshHz: 120,
  });
  assert.equal(virtual.presentation, null);
  assert.equal(virtual.metrics.present_dt_ms, undefined);
  assert.match(virtual.warnings.join(" "), /not a native refresh/);
  const unsupported = await summarizeFrames(lines([header("wall"), trace(0)]), { kind: "trace", skip: 0 });
  assert.equal(unsupported.presentation.unknown, 1);
  assert.match(unsupported.warnings.join(" "), /No actual presentations/);
  await assert.rejects(summarizeFrames(lines([header("virtual"), trace(0, { present_ms: 1 })]), {
    kind: "trace", skip: 0,
  }), /Virtual trace claims/);
});

test("invalid, mixed, truncated and empty recordings refuse a success report", async () => {
  for (const [input, options, expected] of [
    [["{"], { kind: "headless" }, /Invalid JSON/],
    [[], { kind: "headless" }, /No measured frames/],
    [lines([headless(0, 1)]), { kind: "headless" }, /No measured frames/],
    [lines([headless(0, 1), { t: 1, total: 10, build: 2 }]), { kind: "headless", skip: 0 }, /Wrong/],
    [lines([headless(0, 1), headless(0, 2)]), { kind: "headless", skip: 0 }, /Wrong/],
    [lines([headless(0, 1), { end: true, frames: 2 }]), { kind: "headless", skip: 0 }, /Invalid closing/],
    [lines([header("wall"), trace(0, { present_dt_ms: 8 })]), { kind: "trace", skip: 0 }, /without a presentation/],
    [lines([header("wall"), trace(0, { present_ms: 1, present_dt_ms: -1 })]), { kind: "trace", skip: 0 }, /Invalid present_dt_ms/],
    [lines([headless(0, 1, { commits: -1 })]), { kind: "headless", skip: 0 }, /Invalid commits/],
    [lines([headless(0, 1)]), { kind: "headless", skip: -1 }, /skip/],
    [lines([headless(0, 1)]), { kind: "headless", refreshHz: 0 }, /refreshHz/],
    [lines([headless(0, 1)]), { kind: "native", countedAllocations: true }, /Only the headless/],
  ]) await assert.rejects(summarizeFrames(input, options), expected);
});

test("file reports hash exact original bytes, including CRLF and blank lines", async () => {
  const directory = await mkdtemp(join(tmpdir(), "shina-frame-report-"));
  try {
    const path = join(directory, "trace.jsonl");
    const body = `${JSON.stringify(header("wall"))}\r\n\r\n${JSON.stringify(trace(0))}\r\n`;
    await writeFile(path, body);
    const report = await summarizeFile(path, { kind: "trace", skip: 0 });
    assert.equal(report.input.sha256, createHash("sha256").update(body).digest("hex"));
    await assert.rejects(summarizeFile(join(directory, "missing"), { kind: "trace" }), /ENOENT/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("CLI rejects unknown and incomplete options instead of silently using defaults", () => {
  assert.deepEqual(parseArgs(["--kind", "native", "--input", "frame.jsonl", "--refresh-hz", "60"]), {
    path: "frame.jsonl", options: { kind: "native", refreshHz: 60 },
  });
  assert.throws(() => parseArgs(["--input"]), /Use --kind/);
  assert.throws(() => parseArgs(["--input", "x", "--fps", "120"]), /Use --kind/);
  assert.throws(() => parseArgs(["--kind", "native"]), /input is required/);
});

test("CLI emits a complete JSON report or a failing exit, never partial success", async () => {
  const directory = await mkdtemp(join(tmpdir(), "shina-frame-cli-"));
  try {
    const path = join(directory, "headless.jsonl");
    await writeFile(path, lines([headless(0, 100), headless(1, 2)]).join("\n"));
    const run = args => spawnSync(process.execPath, [
      "tools/frame-profile.mjs", "--kind", "headless", "--input", path, ...args,
    ], { encoding: "utf8" });
    const success = run([]);
    assert.equal(success.status, 0, success.stderr);
    assert.equal(JSON.parse(success.stdout).metrics.ms.p95, 2);
    const failed = run(["--skip", "5"]);
    assert.equal(failed.status, 1);
    assert.equal(failed.stdout, "");
    assert.match(failed.stderr, /No measured frames/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
