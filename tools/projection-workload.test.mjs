import assert from "node:assert/strict";
import test from "node:test";
import { fingerprint, makeWorkload } from "./projection-workload.mjs";
import { KSG_PIN, quantile, requirePin } from "./ksg-projection-reference.mjs";

test("every strategy receives the same deterministic counts, content and item updates", () => {
  const first = makeWorkload([[2, 20, 6]], 20);
  assert.equal(JSON.stringify(first), JSON.stringify(makeWorkload([[2, 20, 6]], 20)));
  const sample = first.cases[0];
  assert.equal(sample.total_items, 52);
  assert.equal(sample.threads.flatMap((thread) => thread.messages.concat(thread.tools)).length, 52);
  assert.equal(new Set(sample.threads.flatMap((thread) =>
    thread.messages.concat(thread.tools).map((item) => item.id))).size, 52);
  for (const [index, change] of sample.changes.entries()) {
    assert.equal(change.field, index % 2 === 0 ? "messages" : "tools");
    const original = sample.threads.at(-1)[change.field][change.index];
    assert.equal(change.item.id, original.id);
    const content = change.field === "messages" ? "text" : "output";
    assert.notEqual(change.item[content], original[content]);
    const { [content]: before, ...oldRest } = original;
    const { [content]: after, ...newRest } = change.item;
    assert.deepEqual(oldRest, newRest);
    assert.equal(change.thread_id, sample.thread_count);
  }
  assert.equal(fingerprint(Buffer.from("abc")),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
});

test("the default workload has the documented total item counts", () => {
  assert.deepEqual(makeWorkload().cases.map((sample) => sample.total_items), [520, 5200, 20800]);
});

test("invalid counts and too few updates fail before producing a workload", () => {
  for (const size of [[1, 20, 6], [2, 20, 11], [2, 20, 0], [2, 1.5, 1]]) {
    assert.throws(() => makeWorkload([size]));
  }
  assert.throws(() => makeWorkload([[2, 20, 6]], 11));
});

test("KSG refuses a stale revision label or an edited checkout", () => {
  assert.doesNotThrow(() => requirePin(`${KSG_PIN}\n`, ""));
  assert.throws(() => requirePin("another-revision", ""));
  assert.throws(() => requirePin(KSG_PIN, " M packages/client-runtime/src/state/threads.ts"));
  assert.equal(quantile([5, 1, 2, 3, 4], 50), 3);
  assert.equal(quantile([5, 1, 2, 3, 4], 95), 5);
});
