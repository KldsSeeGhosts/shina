// KSG projection-update timings on the same transcript bytes as the Rust runs.
// This does not measure a UI apply/render and is not a cross-runtime speed ratio.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { performance } from "node:perf_hooks";
import { pathToFileURL } from "node:url";
import { fingerprint, FORMAT } from "./projection-workload.mjs";

export const KSG_PIN = "4807bb1e7e74baae3d0fe9c442c67abadd174a36";

export function requirePin(revision, dirty, expected = KSG_PIN) {
  if (revision.trim() !== expected || dirty.trim() !== "") {
    throw new Error(`KSG must be clean at ${expected}; observed ${revision.trim()}`);
  }
}

export function quantile(values, percentile) {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * percentile / 100))];
}

async function run(file) {
  if (!file) throw new Error("Usage: node tools/ksg-projection-reference.mjs WORKLOAD.json");
  const root = process.env.KSG_ROOT ?? "/Volumes/DevDrive/AiStack/ksgcode";
  const git = (...args) => execFileSync("git", ["-C", root, ...args], { encoding: "utf8" });
  requirePin(git("rev-parse", "HEAD"), git("status", "--porcelain"));
  const bytes = await readFile(file);
  const workload = JSON.parse(bytes);
  assert.equal(workload.format, FORMAT);
  const source = `${root}/packages/client-runtime/src/state`;
  const require = createRequire(`${root}/packages/client-runtime/package.json`);
  const { applyOrchestrationV2ProjectionEvent } =
    await import(pathToFileURL(`${source}/orchestrationV2Projection.ts`));
  const { v2Projection, v2Now } =
    await import(pathToFileURL(`${source}/orchestrationV2TestFixtures.ts`));
  const provenance = {
    fixture_sha256: fingerprint(bytes), ksg_revision: KSG_PIN,
    lock_sha256: fingerprint(await readFile(`${root}/pnpm-lock.yaml`)),
    effect_version: JSON.parse(await readFile(require.resolve("effect/package.json"), "utf8")).version,
    node: process.version, platform: process.platform, arch: process.arch,
    phases: ["applyOrchestrationV2ProjectionEvent"], allocation_counts: null,
  };
  const threadId = (id) => `thread-${id}`;
  const itemBase = (item, id, ordinal) => ({
    id: item.id, threadId: threadId(id), runId: `run-${id}`,
    nodeId: null, providerThreadId: null, providerTurnId: null, nativeItemRef: null, parentItemId: null,
    ordinal, status: "completed", title: null,
    startedAt: v2Now, completedAt: v2Now, updatedAt: v2Now,
  });
  const toMessage = (message, id, index) => ({
    ...itemBase(message, id, index * 2), messageId: message.id, text: message.text,
    ...(message.role === "You"
      ? { type: "user_message", inputIntent: "turn_start", createdBy: "user",
        creationSource: "web", workflowScript: null, retryOfRunId: null,
        freshContext: false, attachments: [] }
      : { type: "assistant_message", streaming: false, attachments: [] }),
  });
  const toTool = (tool, id) => ({
    ...itemBase(tool, id, tool.anchor.message * 2 + 1),
    type: "command_execution", input: `${tool.name} ${tool.detail}`,
    output: tool.output, exitCode: tool.exit_code,
  });
  for (const sample of workload.cases) {
    const projections = new Map(sample.threads.map((thread) => {
      const turnItems = [
        ...thread.messages.map((message, index) => toMessage(message, thread.id, index)),
        ...thread.tools.map((tool) => toTool(tool, thread.id)),
      ].sort((a, b) => a.ordinal - b.ordinal);
      return [thread.id, {
        ...v2Projection, thread: { ...v2Projection.thread, id: threadId(thread.id) },
        runs: [{ id: `run-${thread.id}`, threadId: threadId(thread.id), status: "completed",
          title: null, providerInstanceId: "codex", startedAt: v2Now, completedAt: v2Now }],
        turnItems, visibleTurnItems: turnItems.map((item, position) => ({
          position, visibility: "local", sourceThreadId: item.threadId, sourceItemId: item.id, item,
        })),
      }];
    }));
    assert.equal([...projections.values()].reduce((n, p) => n + p.turnItems.length, 0),
      sample.total_items);
    const untouched = projections.get(1);
    const times = { messages: [], tools: [] };
    for (const [index, change] of sample.changes.entries()) {
      const before = projections.get(change.thread_id);
      const payload = change.field === "messages"
        ? toMessage(change.item, change.thread_id, change.index)
        : toTool(change.item, change.thread_id);
      const event = {
        id: `event-${index}`, type: "turn-item.updated", threadId: payload.threadId,
        runId: payload.runId, occurredAt: v2Now, payload,
      };
      const start = performance.now();
      const after = applyOrchestrationV2ProjectionEvent(before, event);
      const elapsed = (performance.now() - start) * 1000;
      projections.set(change.thread_id, after);
      if (index >= workload.warmup) times[change.field].push(elapsed);
      assert.equal(after.turnItems.length, before.turnItems.length);
      assert.equal(after.visibleTurnItems.length, before.visibleTurnItems.length);
      const updated = after.turnItems.find((item) => item.id === payload.id);
      assert.equal(change.field === "messages" ? updated.text : updated.output,
        change.field === "messages" ? change.item.text : change.item.output);
      assert.equal(projections.get(1), untouched);
    }
    console.log(JSON.stringify({
      ...provenance, threads: sample.thread_count,
      messages_per_thread: sample.messages_per_thread, tools_per_thread: sample.tools_per_thread,
      total_items: sample.total_items,
      timings: Object.fromEntries(Object.entries(times).map(([field, values]) => [field, {
        updates: values.length, p50_us: quantile(values, 50), p95_us: quantile(values, 95),
      }])),
    }));
  }
  // Detect a concurrent checkout change rather than attach a stale pin label.
  requirePin(git("rev-parse", "HEAD"), git("status", "--porcelain"));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await run(process.argv[2]);
}
