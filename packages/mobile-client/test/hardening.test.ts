import { test } from "node:test";
import assert from "node:assert/strict";
import { PairLink, redeemPairLink } from "../src/pairing.ts";
import { PairingError, ProtocolError, RefusedError, UncertainCommandError, IntegrityError } from "../src/errors.ts";
import {
  connect, credential, fakeTransport, fixture, harness, makeUuid,
  MemoryStore, rejectsErr, tick, helloResult,
} from "./fakes.ts";

const code = "0123ABCDEF45";
const link = (host = "studio-mac") => PairLink.parse(
  `shina://pair/v1?host=${host}&port=47329&serverId=${fixture.serverId}&expiresAt=9999999999999#code=${code}`,
);

test("pairing drops adapter errors that can echo the one-time code", async () => {
  const adapter = {
    transport: async () => { throw new Error(`request body code=${code}`); },
    store: new MemoryStore(), uuid: makeUuid(), now: () => 1000,
  };
  const error = await rejectsErr(redeemPairLink(link("100.64.0.10"), adapter), PairingError);
  assert.ok(!error.message.includes(code));
});

test("a malformed DNS answer fails closed instead of being filtered out", async () => {
  await assert.rejects(
    link().resolve(async () => ["100.64.0.10", "not-an-address"]),
    PairingError,
  );
});

test("oversized pairing responses cannot enter the JSON decoder", async () => {
  let posts = 0;
  await assert.rejects(redeemPairLink(link("100.64.0.10"), {
    transport: async (_address, request) => {
      if (request.method === "POST") posts++;
      assert.equal(request.maxBody, 16 * 1024);
      return { status: 200, headers: {}, body: new Uint8Array(16 * 1024 + 1) };
    },
    store: new MemoryStore(), uuid: makeUuid(), now: () => 1000,
  }), PairingError);
  assert.equal(posts, 0);
});

test("credentials with header controls are refused before opening a socket", async () => {
  const { client, sockets } = harness();
  await assert.rejects(
    client.attach(credential({ token: "token\r\nInjected: value" })),
    ProtocolError,
  );
  assert.equal(sockets.sockets.length, 0);
});

test("a socket factory failure cannot echo a credential or leave connecting state", async () => {
  const secret = credential().token;
  const { client } = harness({
    socket: async () => { throw new Error(`Authorization: Bearer ${secret}`); },
  });
  const error = await rejectsErr(client.attach(credential()), PairingError);
  assert.ok(!error.message.includes(secret));
  assert.equal(client.phase, "suspended");
});

test("unpair keeps the session when the server refuses revocation", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets);
  const attempt = client.unpair();
  await tick();
  const request = socket.lastFrame("devices.revoke")!;
  socket.receive({
    jsonrpc: "2.0", id: request["id"],
    result: { status: "rejected", result: { error: "refused" } },
  });
  await assert.rejects(attempt, RefusedError);
  assert.equal(client.phase, "ready");
  client.suspend();
});

test("blob pages require forward progress and real server filename identities", async () => {
  const blobId = `${"a".repeat(64)}.png`;
  const { transport, calls } = fakeTransport({
    [`GET /v1/blobs/${blobId}`]: {
      status: 200,
      headers: { "x-blob-length": "4", "x-next-offset": "0" },
      body: new Uint8Array([1]),
    },
  });
  const { client, sockets } = harness({ transport });
  await connect(client, sockets);
  await assert.rejects(client.readBlob(blobId), ProtocolError);
  assert.equal(calls.length, 1);
  for (const id of ["../secret.png", "a".repeat(64), "unsafe?x.png"]) {
    await assert.rejects(client.readBlob(id), ProtocolError);
  }
  assert.equal(calls.length, 1);
  client.suspend();
});

test("an uncertain retry keeps the original payload despite caller edits", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets);
  const args = { threadId: 1, text: "Original brief" };
  const send = client.mutate("thread.send", args, { timeoutMs: 5 });
  await tick();
  const id = socket.lastFrame("thread.send")!["id"] as string;
  args.text = "Changed after dispatch";
  await assert.rejects(send, UncertainCommandError);
  const shown = client.commandState(id)!;
  shown.args["text"] = "Changed through commandState";
  const retry = client.retryCommand(id);
  await tick();
  const frame = socket.lastFrame("thread.send")!;
  assert.equal((frame["params"] as { args: { text: string } }).args.text, "Original brief");
  socket.receive({ jsonrpc: "2.0", id, result: { status: "accepted", commandId: id } });
  await retry;
  client.suspend();
});

test("a native send exception keeps the original command identity uncertain", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets);
  socket.send = () => { throw new Error("Native send failed after admission"); };
  const error = await rejectsErr(client.mutate("thread.rename", { threadId: 1, title: "Title" }), UncertainCommandError);
  assert.equal(client.commandState(error.commandId)?.state, "uncertain");
  client.suspend();
});

test("one client cannot reuse another server's cache or command registry", async () => {
  const { client, sockets } = harness();
  await connect(client, sockets);
  await assert.rejects(
    client.attach(credential({ serverId: "00000000-0000-4000-8000-ffffffffffff" })),
    IntegrityError,
  );
  assert.equal(sockets.sockets.length, 1);
  assert.equal(client.phase, "ready");
  client.suspend();
});

test("re-pairing cannot retry a command from the previous device namespace", async () => {
  const { client, sockets } = harness();
  const socket = await connect(client, sockets);
  const send = client.mutate("thread.rename", { threadId: 1, title: "Original" });
  await tick();
  const id = socket.lastFrame("thread.rename")!["id"] as string;
  socket.drop();
  await assert.rejects(send, UncertainCommandError);
  const fresh = credential({ deviceId: "00000000-0000-4000-8000-000000000042", token: "repaired_token" });
  const replacement = await connect(client, sockets, {
    cred: fresh, hello: helloResult({ deviceId: fresh.deviceId }),
  });
  assert.equal(client.commandState(id), null);
  await assert.rejects(client.retryCommand(id), ProtocolError);
  assert.equal(replacement.frames("thread.rename").length, 0);
  client.suspend();
});
