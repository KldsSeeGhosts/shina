import { test } from "node:test";
import assert from "node:assert/strict";
import {
  AuthDeniedError,
  CapabilityError,
  OfflineError,
  SuspendedError,
  UncertainCommandError,
} from "../src/errors.ts";
import { ShinaClient } from "../src/session.ts";
import { type JsonObject } from "../src/protocol.ts";
import {
  connect,
  credential,
  fakeTransport,
  fixture,
  harness,
  helloResult,
  rejectsErr,
  subscribeReply,
  tick,
} from "./fakes.ts";

const EMPTY_WORKSPACE: JsonObject = {
  jsonrpc: "2.0",
  method: "subscription.snapshot",
  params: {
    state: { index: { active: null }, threads: [] },
    seq: 7,
    epoch: "workspace-epoch-1",
    maintenance: null,
  },
};

test("attach validates server identity from hello", async () => {
  const { client, sockets } = harness();
  const attach = client.attach(credential());
  await tick();
  const socket = sockets.last();
  assert.equal(socket.request.url, "ws://100.64.0.10:47329/v1/ws");
  assert.equal(
    socket.request.headers["authorization"],
    "Bearer tok_normalized",
    "credential must travel in the upgrade header, not a query",
  );
  socket.open();
  await tick();
  socket.receive(
    helloResult({ serverId: "00000000-0000-4000-8000-ffffffffffff" }),
  );
  await assert.rejects(attach, /not the server this device paired/);
  assert.equal(client.phase, "suspended");
});

test("device id mismatch marks the credential denied until re-pair", async () => {
  const { client, sockets } = harness();
  const attach = client.attach(credential());
  await tick();
  const socket = sockets.last();
  socket.open();
  await tick();
  socket.receive(helloResult({ deviceId: "00000000-0000-4000-8000-ffffffffffff" }));
  await assert.rejects(attach, AuthDeniedError);
  assert.equal(client.phase, "auth-denied");
  // The same credential cannot retry: re-enrollment is never silent.
  await assert.rejects(client.attach(credential()), AuthDeniedError);
  // A freshly paired credential (new device id/token) may attach.
  const fresh = credential({
    deviceId: "00000000-0000-4000-8000-00000000cc03",
    token: "tok_new",
  });
  const reattach = client.attach(fresh);
  await tick();
  const socket2 = sockets.last();
  socket2.open();
  await tick();
  socket2.receive(helloResult({ deviceId: fresh.deviceId }));
  await tick();
  const sub = socket2.lastFrame("workspace.subscribe");
  socket2.receive(subscribeReply(sub?.["id"]));
  await reattach;
  assert.equal(client.phase, "ready");
  assert.equal(client.info?.deviceId, fresh.deviceId);
});

test("protocol major mismatch refuses the session", async () => {
  const { client, sockets } = harness();
  const attach = client.attach(credential());
  await tick();
  const socket = sockets.last();
  socket.open();
  await tick();
  socket.receive(helloResult({ protocolMajor: 2 }));
  await assert.rejects(attach, /protocol major/);
});

test("mutation wire shape is {commandId,args} with the request id", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const send = client.sendMessage(1, { text: "hello", mode: "prompt" });
  await tick();
  const frame = socket.lastFrame("thread.send") as JsonObject;
  const params = frame["params"] as JsonObject;
  assert.equal(frame["id"], params["commandId"], "request id must equal commandId");
  assert.deepEqual(params["args"], {
    threadId: 1,
    text: "hello",
    attachments: [],
    mode: "prompt",
  });
  const commandId = params["commandId"] as string;
  socket.receive({
    jsonrpc: "2.0",
    id: commandId,
    result: {
      commandId,
      deviceId: fixture.deviceId,
      hash: "h",
      status: "accepted",
      result: { threadId: 1 },
      seq: 9,
    },
  });
  const receipt = await send;
  assert.equal(receipt.status, "accepted");
  assert.equal(receipt.commandId, commandId);
});

test("offline and suspended mutations are rejected, never queued", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const sentBefore = socket.sent.length;
  client.suspend();
  await assert.rejects(client.sendMessage(1, { text: "x" }), SuspendedError);
  await assert.rejects(client.read("threads.search", {}), SuspendedError);
  assert.equal(socket.sent.length, sentBefore, "suspend queued frames");
  const { client: idle } = harness();
  await assert.rejects(idle.sendMessage(1, { text: "x" }), OfflineError);
});

test("suspend closes the socket without issuing stop commands", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const before = socket.sent.length;
  client.suspend();
  assert.equal(socket.sent.length, before);
  assert.equal(socket.closedWith?.code, 1000);
  assert.equal(client.phase, "suspended");
});

test("stale socket callbacks cannot mutate the resumed session", async () => {
  const { client, sockets } = harness();
  const first = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  // Capture the generation-one handler; suspend nulls socket.onmessage, but a
  // platform that queues events can still invoke the captured closure. The
  // generation fence inside the wiring must reject it either way.
  const staleHandler = first.onmessage;
  client.suspend();
  staleHandler?.({
    data: JSON.stringify({
      jsonrpc: "2.0",
      method: "subscription.patch",
      params: {
        seq: 8,
        epoch: "workspace-epoch-1",
        patch: [{ op: "set", path: ["injected"], value: true }],
      },
    }),
  });
  first.receive({
    jsonrpc: "2.0",
    method: "subscription.patch",
    params: {
      seq: 8,
      epoch: "workspace-epoch-1",
      patch: [{ op: "set", path: ["injected2"], value: true }],
    },
  });
  const resuming = client.resume();
  await tick();
  const second = sockets.last();
  second.open();
  await tick();
  second.receive(helloResult());
  await tick();
  const sub = second.lastFrame("workspace.subscribe");
  // The resume must carry the cursor + epoch it learned before suspending.
  assert.deepEqual(sub?.["params"], { lastSeq: 7, epoch: "workspace-epoch-1" });
  second.receive(EMPTY_WORKSPACE);
  second.receive(subscribeReply(sub?.["id"]));
  await tick();
  await resuming;
  // More stale traffic on generation one; still fenced.
  staleHandler?.({
    data: JSON.stringify({
      jsonrpc: "2.0",
      method: "subscription.patch",
      params: {
        seq: 8,
        epoch: "workspace-epoch-1",
        patch: [{ op: "set", path: ["injected3"], value: true }],
      },
    }),
  });
  const state = client.workspace();
  assert.equal((state as JsonObject)["injected"], undefined);
  assert.equal(client.phase, "ready");
});

test("lost acknowledgement exposes the uncertain command id, no resend", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const send = client.sendMessage(1, { text: "work" });
  await tick();
  const commandId = (socket.lastFrame("thread.send") as JsonObject)["id"] as string;
  socket.drop();
  const err = await rejectsErr(send);
  assert.ok(err instanceof UncertainCommandError);
  assert.equal(err.commandId, commandId);
  assert.equal(err.kind, "uncertain");
  assert.equal(
    socket.frames("thread.send").length,
    1,
    "an uncertain command must not be resent with a fresh id",
  );
  assert.equal(client.commandState(commandId)?.state, "uncertain");
});

test("resolveCommand prefers the durable receipt, resending same id otherwise", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const send = client.sendMessage(1, { text: "work" });
  await tick();
  const commandId = (socket.lastFrame("thread.send") as JsonObject)["id"] as string;
  socket.drop();
  await assert.rejects(send, UncertainCommandError);

  // Resume against a fresh socket.
  const resuming = client.resume();
  await tick();
  const socket2 = sockets.last();
  socket2.open();
  await tick();
  socket2.receive(helloResult());
  await tick();
  const sub = socket2.lastFrame("workspace.subscribe");
  socket2.receive(EMPTY_WORKSPACE);
  socket2.receive(subscribeReply(sub?.["id"]));
  await resuming;
  await tick();

  // Case 1: the server stored the receipt; no resend happens.
  let resolving = client.resolveCommand(commandId);
  await tick();
  const getFrame = socket2.lastFrame("command.get") as JsonObject;
  assert.equal((getFrame["params"] as JsonObject)["commandId"], commandId);
  socket2.receive({
    jsonrpc: "2.0",
    id: getFrame["id"],
    result: {
      commandId,
      deviceId: fixture.deviceId,
      hash: "h",
      status: "accepted",
      result: { threadId: 1 },
      seq: 11,
    },
  });
  const receipt = await resolving;
  assert.equal((receipt as { status: string }).status, "accepted");
  assert.equal(socket2.frames("thread.send").length, 0);

  // Case 2: no receipt; the same command id is resent, never a fresh one.
  const send2 = client.sendMessage(1, { text: "again" });
  await tick();
  const commandId2 = (socket2.lastFrame("thread.send") as JsonObject)["id"] as string;
  socket2.drop();
  await assert.rejects(send2, UncertainCommandError);

  const resuming3 = client.resume();
  await tick();
  const socket3 = sockets.last();
  socket3.open();
  await tick();
  socket3.receive(helloResult());
  await tick();
  const sub3 = socket3.lastFrame("workspace.subscribe");
  socket3.receive(EMPTY_WORKSPACE);
  socket3.receive(subscribeReply(sub3?.["id"]));
  await resuming3;
  await tick();

  resolving = client.resolveCommand(commandId2);
  await tick();
  const getFrame2 = socket3.lastFrame("command.get") as JsonObject;
  socket3.receive({
    jsonrpc: "2.0",
    id: getFrame2["id"],
    error: { code: -32603, message: "No receipt was stored for this command." },
  });
  await tick();
  const resent = socket3.lastFrame("thread.send") as JsonObject;
  assert.equal(resent["id"], commandId2, "retry must reuse the original id");
  assert.equal((resent["params"] as JsonObject)["commandId"], commandId2);
  socket3.receive({
    jsonrpc: "2.0",
    id: commandId2,
    result: {
      commandId: commandId2,
      deviceId: fixture.deviceId,
      hash: "h",
      status: "accepted",
      result: {},
      seq: 12,
    },
  });
  await resolving;
});

test("a command timing out is uncertain, and a late ack resolves the record", async () => {
  const { client, sockets } = harness({
    timeouts: { commandMs: 10, readMs: 10, helloMs: 500 },
  });
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const send = client.sendMessage(1, { text: "slow" });
  const err = await rejectsErr(send);
  assert.ok(err instanceof UncertainCommandError);
  const record = client.commandState(err.commandId);
  assert.equal(record?.state, "uncertain");
  socket.receive({
    jsonrpc: "2.0",
    id: err.commandId,
    result: {
      commandId: err.commandId,
      deviceId: fixture.deviceId,
      hash: "h",
      status: "accepted",
      result: {},
      seq: 20,
    },
  });
  assert.equal(client.commandState(err.commandId)?.state, "resolved");
});

test("auth denial mid-session is terminal", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const send = client.sendMessage(1, { text: "x" });
  await tick();
  const frame = socket.lastFrame("thread.send") as JsonObject;
  socket.receive({
    jsonrpc: "2.0",
    id: frame["id"],
    error: {
      code: -32603,
      message: "Authentication denied. Device unknown or revoked.",
    },
  });
  await assert.rejects(send, AuthDeniedError);
  assert.equal(client.phase, "auth-denied");
  await assert.rejects(client.resume(), AuthDeniedError);
  await assert.rejects(client.sendMessage(1, { text: "x" }), AuthDeniedError);
});

test("capability skew: notifications gated on the advertised string", async () => {
  const noCaps = {
    capabilities: (fixture.hello.result["capabilities"] as string[]).filter(
      (cap: string) => cap !== "notification-feed-v1",
    ),
  } as JsonObject;
  const { client, sockets } = harness();
  await connect(client, sockets, {
    workspace: EMPTY_WORKSPACE,
    hello: helloResult(noCaps),
  });
  await assert.rejects(client.notifications(), CapabilityError);
  await assert.rejects(
    client.resolvePushNavigation({ eventId: "n1" }),
    CapabilityError,
  );
});

test("capability skew: missing history-items marks update required", async () => {
  const noHistory = {
    capabilities: (fixture.hello.result["capabilities"] as string[]).filter(
      (cap: string) => cap !== "history-items",
    ),
  } as JsonObject;
  const { client, sockets } = harness();
  const notices: string[] = [];
  client.onEvent((event) => {
    if (event.type === "notice") {
      notices.push(event.text);
    }
  });
  const socket = await connect(client, sockets, { hello: helloResult(noHistory) });
  const state = client.workspace() as JsonObject;
  assert.equal(state["historyUpdateRequired"], true);
  assert.ok(notices.some((text) => text.includes("Update required")));
  // No history requests are issued under skew.
  assert.equal(socket.frames("thread.subscribe").length, 0);
});

test("typed read sends plain params", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const searching = client.read("threads.search", { query: "abc", limit: 5 });
  await tick();
  const frame = socket.lastFrame("threads.search") as JsonObject;
  assert.deepEqual(frame["params"], { query: "abc", limit: 5 });
  socket.receive({
    jsonrpc: "2.0",
    id: frame["id"],
    result: { hits: [] },
  });
  assert.deepEqual(await searching, { hits: [] });
});

test("notification feed reads pages when negotiated", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const page = client.notifications({ afterSequence: 40, limit: 50 });
  await tick();
  const frame = socket.lastFrame("notifications.list") as JsonObject;
  assert.deepEqual(frame["params"], { afterSequence: 40, limit: 50 });
  socket.receive({
    jsonrpc: "2.0",
    id: frame["id"],
    result: fixture.notifications,
  });
  const result = await page;
  assert.equal(result.events.length, 2);
  assert.equal(result.events[0]?.kind, "approval");
  assert.equal(result.events[0]?.requestId, "req-1");
  assert.equal(result.hasMore, false);
});

const PUSH_EVENT_ID = "00000000-0000-4000-8000-00000000ee01";

const PUSH_EVENT = {
  id: PUSH_EVENT_ID,
  seq: 44,
  kind: "approval",
  threadId: 1,
  createdAt: 1700000004000,
  runId: "run-2",
  requestId: "req-9",
  generation: "gen-1",
};

function withPush(): JsonObject {
  return {
    capabilities: [
      ...(fixture.hello.result["capabilities"] as string[]),
      "mobile-push-v1",
    ],
  } as JsonObject;
}

test("push navigation fetches routing state by event id", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, {
    workspace: EMPTY_WORKSPACE,
    hello: helloResult(withPush()),
  });
  const target = client.resolvePushNavigation({
    serverId: fixture.serverId,
    eventId: PUSH_EVENT_ID,
  });
  await tick();
  const frame = socket.lastFrame("notifications.get") as JsonObject;
  assert.deepEqual(frame["params"], { eventId: PUSH_EVENT_ID });
  socket.receive({
    jsonrpc: "2.0",
    id: frame["id"],
    result: { event: PUSH_EVENT },
  });
  const resolved = await target;
  assert.equal(resolved?.threadId, 1);
  assert.equal(resolved?.kind, "approval");
  assert.equal(resolved?.eventId, PUSH_EVENT_ID);
  assert.equal(resolved?.requestId, "req-9");
});

test("push navigation rejects malformed or foreign payloads before sending", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, {
    workspace: EMPTY_WORKSPACE,
    hello: helloResult(withPush()),
  });
  const sentBefore = socket.sent.length;
  await assert.rejects(
    client.resolvePushNavigation({ serverId: "not-a-uuid", eventId: PUSH_EVENT_ID }),
    /Malformed/,
  );
  await assert.rejects(
    client.resolvePushNavigation({ serverId: fixture.serverId }),
    /Malformed/,
  );
  await assert.rejects(
    client.resolvePushNavigation({
      serverId: fixture.serverId,
      eventId: PUSH_EVENT_ID,
      extra: "field",
    }),
    /Malformed/,
  );
  await assert.rejects(
    client.resolvePushNavigation({
      serverId: "00000000-0000-4000-8000-ffffffffffff",
      eventId: PUSH_EVENT_ID,
    }),
    /not paired/,
  );
  assert.equal(
    socket.sent.length,
    sentBefore,
    "a rejected payload must not reach the network",
  );
});

test("push navigation reports unsupported without sending when capability is absent", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const sentBefore = socket.sent.length;
  await assert.rejects(
    client.resolvePushNavigation({
      serverId: fixture.serverId,
      eventId: PUSH_EVENT_ID,
    }),
    CapabilityError,
  );
  assert.equal(socket.sent.length, sentBefore);
  await assert.rejects(client.getNotification(PUSH_EVENT_ID), CapabilityError);
  await assert.rejects(
    client.registerPushNotifications("ios", "ExponentPushToken[abc]"),
    CapabilityError,
  );
  await assert.rejects(client.pushStatus(), CapabilityError);
  assert.equal(socket.sent.length, sentBefore);
});

test("push registration uses the command envelope; status is a raw read", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, {
    workspace: EMPTY_WORKSPACE,
    hello: helloResult(withPush()),
  });
  const registering = client.registerPushNotifications("ios", "ExponentPushToken[abc]");
  await tick();
  const frame = socket.lastFrame("notifications.push.register") as JsonObject;
  const params = frame["params"] as JsonObject;
  assert.equal(frame["id"], params["commandId"]);
  assert.deepEqual(params["args"], {
    platform: "ios",
    token: "ExponentPushToken[abc]",
  });
  socket.receive({
    jsonrpc: "2.0",
    id: frame["id"],
    result: {
      commandId: params["commandId"],
      deviceId: fixture.deviceId,
      hash: "h",
      status: "accepted",
      result: {},
      seq: 30,
    },
  });
  assert.equal((await registering).status, "accepted");

  const unregistering = client.unregisterPushNotifications();
  await tick();
  const unreg = socket.lastFrame("notifications.push.unregister") as JsonObject;
  socket.receive({
    jsonrpc: "2.0",
    id: unreg["id"],
    result: {
      commandId: (unreg["params"] as JsonObject)["commandId"],
      deviceId: fixture.deviceId,
      hash: "h",
      status: "accepted",
      result: {},
      seq: 31,
    },
  });
  assert.equal((await unregistering).status, "accepted");

  const status = client.pushStatus();
  await tick();
  const statusFrame = socket.lastFrame("notifications.push.status") as JsonObject;
  assert.deepEqual(statusFrame["params"], {});
  socket.receive({
    jsonrpc: "2.0",
    id: statusFrame["id"],
    result: { registered: true, platform: "ios" },
  });
  assert.deepEqual(await status, { registered: true, platform: "ios" });
});

test("readBlob pages through the pinned transport", async () => {
  const blobId = `${"a".repeat(64)}.png`;
  const { transport, calls } = fakeTransport({
    "GET /v1/blobs/{id}": { status: 200, body: new Uint8Array([1, 2, 3]) },
    [`GET /v1/blobs/${blobId}`]: (request) => {
      const offset = Number(new URLSearchParams(request.path.split("?")[1]).get("offset"));
      const body =
        offset === 0 ? new Uint8Array([1, 2]) : new Uint8Array([3, 4]);
      return {
        status: 200,
        headers: {
          "x-blob-length": "4",
          "x-next-offset": String(offset + body.length),
        },
        body,
      };
    },
  });
  const { client, sockets } = harness({ transport });
  await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  const blob = await client.readBlob(blobId);
  assert.deepEqual([...blob.bytes], [1, 2, 3, 4]);
  assert.equal(blob.length, 4);
  assert.ok(calls.every((call) => call.address.ip === "100.64.0.10"));
  assert.ok(
    calls.every(
      (call) => call.request.headers?.["authorization"] === "Bearer tok_normalized",
    ),
  );
});

test("outstanding command bound refuses the 33rd send", async () => {
  const { client, sockets } = harness();
  await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  for (let i = 0; i < 32; i += 1) {
    void client.sendMessage(1, { text: `m${i}` }).catch(() => {});
  }
  await tick();
  await assert.rejects(client.sendMessage(1, { text: "overflow" }), /Too many commands/);
  client.suspend();
});

test("inbound frame above the reassembly bound drops the socket", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets, { workspace: EMPTY_WORKSPACE });
  socket.receive("x".repeat(64 * 1024 * 1024 + 1));
  await tick();
  assert.equal(client.phase, "suspended");
});
