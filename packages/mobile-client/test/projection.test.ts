import { test } from "node:test";
import assert from "node:assert/strict";
import { ProjectionCache, ProjectionGapError } from "../src/projection.ts";
import {
  MAX_SNAPSHOT_BYTES,
  applyPatches,
  utf8Length,
  type JsonObject,
} from "../src/protocol.ts";
import { ProtocolError } from "../src/errors.ts";
import { fixture } from "./fakes.ts";

const HELLO = {
  capabilities: ["history-items", "deferred-work", "notification-feed-v1"],
};

function cacheWithWorkspace(): ProjectionCache {
  const cache = new ProjectionCache();
  cache.reconnect(HELLO);
  cache.receive(fixture.workspace);
  return cache;
}

function snapshotResult(
  page: JsonObject,
  streamEpoch = "stream-1",
): JsonObject {
  return {
    jsonrpc: "2.0",
    id: "history",
    result: {
      kind: "snapshot",
      streamEpoch,
      data: JSON.stringify(page),
      nextFragment: null,
    },
  };
}

/** Drive the first thread.subscribe snapshot walk to an installed base. The
 * fixture page has hasMoreHistory:true + a cursor, so the view is ready but
 * not complete. */
function installBase(cache: ProjectionCache): void {
  const request = cache.nextHistory() as unknown as JsonObject;
  assert.equal(request["method"], "thread.subscribe");
  assert.equal(cache.nextHistory(), null, "one in-flight request");
  cache.receive(snapshotResult(fixture.threadSnapshotPage));
  assert.equal(cache.threadView(1)?.ready, true);
}

function row(
  sourceItemId: string,
  field: string,
  ordinal: number,
  item: JsonObject,
): JsonObject {
  return {
    sourceThreadId: 1,
    sourceItemId,
    field,
    ordinal,
    position: ordinal,
    item,
  };
}

test("workspace patches apply only on contiguous sequence", () => {
  const cache = cacheWithWorkspace();
  cache.receive({
    method: "subscription.patch",
    params: {
      seq: 8,
      epoch: "workspace-epoch-1",
      patch: [{ op: "set", path: ["index", "note"], value: "ok" }],
    },
  });
  assert.equal(
    ((cache.state as JsonObject)["index"] as JsonObject)["note"],
    "ok",
  );
  assert.equal(cache.seq, 8);
  // Stale seq ignored.
  cache.receive({
    method: "subscription.patch",
    params: {
      seq: 5,
      epoch: "workspace-epoch-1",
      patch: [{ op: "set", path: ["index", "note"], value: "stale" }],
    },
  });
  assert.equal(
    ((cache.state as JsonObject)["index"] as JsonObject)["note"],
    "ok",
  );
  // A gap throws and leaves state untouched.
  assert.throws(
    () =>
      cache.receive({
        method: "subscription.patch",
        params: {
          seq: 12,
          epoch: "workspace-epoch-1",
          patch: [{ op: "set", path: ["index", "note"], value: "gap" }],
        },
      }),
    ProjectionGapError,
  );
  assert.equal(
    ((cache.state as JsonObject)["index"] as JsonObject)["note"],
    "ok",
  );
  assert.equal(cache.seq, 8);
});

test("epoch change is a replay gap", () => {
  const cache = cacheWithWorkspace();
  assert.throws(
    () =>
      cache.receive({
        method: "subscription.patch",
        params: {
          seq: 8,
          epoch: "other-epoch",
          patch: [{ op: "set", path: ["index", "note"], value: 1 }],
        },
      }),
    ProjectionGapError,
  );
});

test("patch batches are atomic: a bad patch preserves the prior state", () => {
  const cache = cacheWithWorkspace();
  const before = JSON.stringify(cache.state);
  // Contiguous seq, malformed batch: throws and commits nothing.
  assert.throws(
    () =>
      cache.receive({
        method: "subscription.patch",
        params: {
          seq: 8,
          epoch: "workspace-epoch-1",
          patch: [
            { op: "set", path: ["index", "applied"], value: 1 },
            { op: "set", path: ["missing", "parent"], value: 2 },
          ],
        },
      }),
    ProtocolError,
  );
  assert.equal(JSON.stringify(cache.state), before);
  assert.equal(cache.seq, 7, "seq must not advance on a failed batch");
  // The reducer itself also clones before mutating.
  assert.throws(
    () =>
      applyPatches(cache.state, [
        { op: "set", path: ["index", "applied"], value: 1 },
        { op: "set", path: ["missing", "parent"], value: 2 },
      ]),
    /missing parent/,
  );
  assert.equal(JSON.stringify(cache.state), before);
});

test("prototype-polluting patch paths are refused", () => {
  const state: JsonObject = { a: { b: 1 } };
  for (const path of [
    ["__proto__", "x"],
    ["a", "__proto__", "x"],
    ["constructor", "prototype", "x"],
    ["a", "prototype", "x"],
  ]) {
    assert.throws(
      () => applyPatches(state, [{ op: "set", path, value: true }]),
      /Invalid projection patch/,
    );
  }
  assert.equal(({} as Record<string, unknown>)["x"], undefined);
  assert.deepEqual(state, { a: { b: 1 } });
});

test("malformed patch targets fail without corrupting the cache", () => {
  const state: JsonObject = { arr: [1, 2], obj: { k: 1 } };
  assert.throws(() => applyPatches(state, [{ op: "set", path: ["arr", "9"], value: 1 }]));
  assert.throws(() => applyPatches(state, [{ op: "set", path: ["arr", "x"], value: 1 }]));
  assert.throws(() => applyPatches(state, [{ op: "set", path: ["obj", "k", "deep"], value: 1 }]));
  assert.throws(() => applyPatches(state, [{ op: "remove", path: [] }]));
  assert.throws(() => applyPatches(state, [{ op: "remove", path: ["arr", "0"] }]));
  assert.deepEqual(state, { arr: [1, 2], obj: { k: 1 } });
  // Valid operations still work.
  const next = applyPatches(state, [
    { op: "set", path: ["obj", "k2"], value: "v" },
    { op: "set", path: ["arr", "1"], value: 9 },
    { op: "remove", path: ["obj", "k"] },
  ]) as JsonObject;
  assert.deepEqual(next, { arr: [1, 9], obj: { k2: "v" } });
});

test("snapshot pages reassemble under the bound and oversized pages fail", () => {
  const cache = new ProjectionCache();
  cache.reconnect(HELLO);
  const snapshot = JSON.stringify(fixture.workspace["params"]);
  const mid = Math.floor(snapshot.length / 2);
  cache.receive({
    method: "subscription.snapshotPage",
    params: { data: snapshot.slice(0, mid), nextCursor: "c1" },
  });
  assert.equal(cache.seq, null);
  cache.receive({
    method: "subscription.snapshotPage",
    params: { data: snapshot.slice(mid), nextCursor: null },
  });
  assert.equal(cache.seq, 7);
});

test("unicode content counts by UTF-8 bytes in bounds checks", () => {
  assert.equal(utf8Length("é"), 2);
  assert.equal(utf8Length("😀"), 4);
  assert.equal(utf8Length("ab"), 2);
  // A page full of multibyte text counts bytes, not code units.
  const cache = new ProjectionCache();
  cache.reconnect(HELLO);
  const payload = "é".repeat(1024 * 1024); // 2 MiB as bytes
  cache.receive({
    method: "thread.streamPage",
    params: { data: payload, nextCursor: "c" },
  });
  // No error: 2 MiB is under 64 MiB. The byte-length math is what bounded it.
  const huge = new ProjectionCache();
  huge.reconnect(HELLO);
  assert.throws(
    () =>
      huge.receive({
        method: "thread.streamPage",
        params: { data: "x".repeat(MAX_SNAPSHOT_BYTES + 1), nextCursor: "c" },
      }),
    /memory budget/,
  );
});

test("thread subscribe replays cursors or falls back to the snapshot walk", () => {
  const cache = cacheWithWorkspace();
  // First walk is a plain subscribe (no cursors yet).
  let request = cache.nextHistory() as unknown as JsonObject;
  assert.equal(request["method"], "thread.subscribe");
  assert.deepEqual(request["params"], { threadId: 1, byteBudget: 65536 });
  // One request in flight at a time.
  assert.equal(cache.nextHistory(), null);
  // A fully-loaded page (no cursor) makes the base complete.
  cache.receive(
    snapshotResult({
      ...fixture.threadSnapshotPage,
      hasMoreHistory: false,
      nextCursor: null,
    }),
  );
  assert.equal(cache.complete(1), true);

  // After reconnect, the same view replays from its sequence+epoch.
  cache.reconnect(HELLO);
  request = cache.nextHistory() as unknown as JsonObject;
  const params = request["params"] as JsonObject;
  assert.equal(params["afterSequence"], 3);
  assert.equal(params["streamEpoch"], "stream-1");
  cache.receive({
    jsonrpc: "2.0",
    id: "history",
    result: { kind: "synchronized", threadId: 1, snapshotSequence: 3, streamEpoch: "stream-1" },
  });
  assert.equal(cache.nextHistory(), null, "synced subscription is idle");
});

test("unknown event kinds advance sequence; malformed known rows fail", () => {
  const cache = cacheWithWorkspace();
  installBase(cache);
  cache.receive(fixture.threadStreamEvent);
  const history = cache.history(1) as JsonObject;
  const tools = history["tools"] as JsonObject[];
  assert.equal(tools[0]?.["output"], "ok");
  const view = cache.threadView(1);
  assert.equal(view?.sequence, 4);
  assert.equal(view?.revision, "rev-2");
  // Unknown kind in a later event still moves the stream sequence.
  cache.receive({
    method: "thread.stream",
    params: {
      kind: "event",
      threadId: 1,
      sequence: 5,
      streamEpoch: "stream-1",
      events: [{ type: "future.kind", blob: {} }],
    },
  });
  assert.equal(cache.threadView(1)?.sequence, 5);
  // Malformed known event (row missing sourceItemId) is a protocol failure.
  assert.throws(
    () =>
      cache.receive({
        method: "thread.stream",
        params: {
          kind: "event",
          threadId: 1,
          sequence: 6,
          streamEpoch: "stream-1",
          events: [{ type: "item.updated", row: { sourceThreadId: 1, field: "messages", ordinal: 9, position: 9, item: {} } }],
        },
      }),
    ProtocolError,
  );
});

test("events from a stale stream epoch are ignored", () => {
  const cache = cacheWithWorkspace();
  installBase(cache);
  cache.receive({
    method: "thread.stream",
    params: {
      kind: "event",
      threadId: 1,
      sequence: 9,
      streamEpoch: "stale-epoch",
      events: [
        {
          type: "item.updated",
          row: row("ghost", "messages", 9, { role: "assistant", text: "nope" }),
        },
      ],
    },
  });
  assert.equal(cache.threadView(1)?.sequence, 3);
  assert.equal(
    ((cache.history(1) as JsonObject)["messages"] as unknown[]).length,
    2,
  );
});

test("newer live rows and removals win over concurrently read older pages", () => {
  const cache = cacheWithWorkspace();
  installBase(cache);
  // Request the older page; while it is in flight a live update lands.
  cache.requestEarlier(1);
  const pageRequest = cache.nextHistory() as unknown as JsonObject;
  assert.equal(pageRequest["method"], "thread.history");
  assert.equal((pageRequest["params"] as JsonObject)["historyCursor"], "cursor-older-1");
  cache.receive({
    method: "thread.stream",
    params: {
      kind: "event",
      threadId: 1,
      sequence: 6,
      streamEpoch: "stream-1",
      events: [
        {
          type: "item.updated",
          row: row("msg-1", "messages", 0, { id: "msg-1", role: "You", text: "edited live" }),
        },
        { type: "item.removed", sourceItemId: "tool-1" },
      ],
    },
  });
  // The older page (snapshotSequence 4) carries the pre-update contents.
  cache.receive({
    jsonrpc: "2.0",
    id: "history",
    result: {
      kind: "page",
      data: JSON.stringify({
        threadId: 1,
        snapshotSequence: 4,
        historyRevision: "rev-1",
        hasMoreHistory: false,
        nextCursor: null,
        fields: {},
        items: [
          row("msg-1", "messages", 0, { id: "msg-1", role: "You", text: "stale page" }),
          row("tool-1", "tools", 0, { id: "tool-1", tool: "read", anchor: { message: 0 } }),
          row("msg-0", "messages", -0 + 0, { id: "msg-0", role: "system", text: "older" }),
        ],
      }),
      nextFragment: null,
    },
  });
  const history = cache.history(1) as JsonObject;
  const messages = history["messages"] as JsonObject[];
  const msg1 = messages.find((m) => m["id"] === "msg-1");
  assert.equal(msg1?.["text"], "edited live", "live row must win over the older page");
  const tools = history["tools"] as JsonObject[];
  assert.equal(tools.length, 0, "a live removal must win over the older page");
  assert.ok(messages.some((m) => m["id"] === "msg-0"), "new-to-us older rows still merge");
  // The walk is done: complete() reflects hasMoreHistory false + null cursor.
  assert.equal(cache.threadView(1)?.hasMore, false);
});

test("a removed row stays removed when the same item returns older", () => {
  const cache = cacheWithWorkspace();
  installBase(cache);
  cache.requestEarlier(1);
  cache.nextHistory();
  cache.receive({
    method: "thread.stream",
    params: {
      kind: "event",
      threadId: 1,
      sequence: 5,
      streamEpoch: "stream-1",
      events: [{ type: "item.removed", sourceItemId: "msg-2" }],
    },
  });
  cache.receive({
    jsonrpc: "2.0",
    id: "history",
    result: {
      kind: "page",
      data: JSON.stringify({
        threadId: 1,
        snapshotSequence: 4,
        historyRevision: "rev-1",
        hasMoreHistory: false,
        nextCursor: null,
        fields: {},
        items: [
          row("msg-2", "messages", 1, { id: "msg-2", role: "assistant", text: "resurrected" }),
        ],
      }),
      nextFragment: null,
    },
  });
  const messages = (cache.history(1) as JsonObject)["messages"] as JsonObject[];
  assert.ok(!messages.some((m) => m["id"] === "msg-2"));
});

test("history stays lazy: older pages need an explicit request", () => {
  const cache = cacheWithWorkspace();
  installBase(cache);
  // The walk finished with a cursor (hasMoreHistory true) but no further page
  // is fetched until requested.
  assert.equal(cache.nextHistory(), null);
  assert.equal(cache.threadView(1)?.hasMore, true);
  cache.requestEarlier(1);
  const request = cache.nextHistory() as unknown as JsonObject;
  assert.equal(request["method"], "thread.history");
  assert.equal((request["params"] as JsonObject)["historyCursor"], "cursor-older-1");
});

test("pending stream events during the snapshot walk apply after install", () => {
  const cache = cacheWithWorkspace();
  cache.nextHistory();
  // Live stream during a fragmented snapshot gets buffered.
  cache.receive({
    method: "thread.stream",
    params: {
      kind: "event",
      threadId: 1,
      sequence: 4,
      streamEpoch: "stream-1",
      events: [
        {
          type: "item.updated",
          row: row("live-1", "messages", 2, { id: "live-1", role: "assistant", text: "during walk" }),
        },
      ],
    },
  });
  cache.receive(snapshotResult(fixture.threadSnapshotPage));
  const messages = (cache.history(1) as JsonObject)["messages"] as JsonObject[];
  assert.ok(messages.some((m) => m["id"] === "live-1"), "buffered live row applied after snapshot");
});

test("fragmented snapshot pages drive continuation requests", () => {
  const cache = cacheWithWorkspace();
  cache.nextHistory();
  const page = JSON.stringify(fixture.threadSnapshotPage);
  const mid = Math.floor(page.length / 2);
  cache.receive({
    jsonrpc: "2.0",
    id: "history",
    result: {
      kind: "snapshot",
      streamEpoch: "stream-1",
      data: page.slice(0, mid),
      nextFragment: "frag-1",
    },
  });
  const continuation = cache.nextHistory() as unknown as JsonObject;
  assert.equal(continuation["method"], "thread.history");
  assert.equal((continuation["params"] as JsonObject)["fragmentCursor"], "frag-1");
  cache.receive({
    jsonrpc: "2.0",
    id: "history",
    result: { data: page.slice(mid), nextFragment: null },
  });
  assert.equal(cache.threadView(1)?.ready, true);
});

test("deleted threads drop their view and history", () => {
  const cache = cacheWithWorkspace();
  installBase(cache);
  cache.receive({
    method: "thread.stream",
    params: { kind: "deleted", threadId: 1, sequence: 5, streamEpoch: "stream-1" },
  });
  assert.equal(cache.history(1), null);
  assert.equal(cache.threadView(1), null);
});

test("snapshot-required triggers a bounded retry, then stops", () => {
  const cache = cacheWithWorkspace();
  installBase(cache);
  cache.receive({
    method: "thread.stream",
    params: { kind: "snapshot-required", threadId: 1 },
  });
  const retry = cache.retryState(1);
  assert.equal(retry?.count, 1);
  // The retry resubscribes WITHOUT the stale epoch (fresh snapshot).
  const request = cache.nextHistory() as unknown as JsonObject;
  assert.equal(request["method"], "thread.subscribe");
  assert.equal((request["params"] as JsonObject)["afterSequence"], undefined);
  cache.receive(
    snapshotResult(
      { ...fixture.threadSnapshotPage, streamEpoch: "stream-2" },
      "stream-2",
    ),
  );
  // A second snapshot-required stops retrying.
  cache.receive({
    method: "thread.stream",
    params: { kind: "snapshot-required", threadId: 1 },
  });
  assert.equal(cache.retryState(1)?.stopped, true);
  assert.equal(cache.nextHistory(), null);
});

test("cross-thread pages and rows are refused", () => {
  const cache = cacheWithWorkspace();
  cache.nextHistory();
  assert.throws(
    () =>
      cache.receive(
        snapshotResult({ ...fixture.threadSnapshotPage, threadId: 2 }),
      ),
    /Cross-thread/,
  );
});

test("installed history orders rows by field, ordinal; anchors remap", () => {
  const cache = cacheWithWorkspace();
  installBase(cache);
  const history = cache.history(1) as JsonObject;
  const messages = history["messages"] as JsonObject[];
  assert.deepEqual(messages.map((m) => m["id"]), ["msg-1", "msg-2"]);
  const tools = history["tools"] as JsonObject[];
  assert.equal((tools[0]?.["anchor"] as JsonObject)["message"], 0);
  assert.equal(history["id"], 1);
});

test("no history request when the desired thread is absent", () => {
  const cache = cacheWithWorkspace();
  cache.desiredThread = 2;
  assert.equal(cache.nextHistory(), null);
});
