//! Live acceptance coverage: the real client core against a real `shina
//! serve` process and its fake Pi harness. The Rust driver
//! (tests/support/mobile_sdk_runtime.rs) launches serve, mints the pairing
//! code and writes a user-private config file this test reads through the
//! SHINA_LIVE_CONFIG variable. Without that variable the test skips itself,
//! so ordinary `npm test` stays zero-network. Config values - the pairing
//! link, the issued token, the dial mapping - are never printed.
//!
//! The adapters below exist only here. They assert every pinned address is
//! the one synthetic tailnet literal from the config and map exactly that
//! target to the loopback port the real server chose; anything else fails
//! closed. Production pinning rules are unchanged.

import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import http from "node:http";
import { test } from "node:test";
import { setTimeout as delay } from "node:timers/promises";
import WebSocket from "ws";
import {
  PairLink,
  credentialKey,
  loadCredential,
  redeemPairLink,
  type DeviceCredential,
} from "../src/pairing.ts";
import {
  CAP_NOTIFICATIONS,
  CAP_PUSH_NOTIFICATIONS,
  isObject,
  type JsonObject,
} from "../src/protocol.ts";
import {
  SuspendedError,
  UncertainCommandError,
} from "../src/errors.ts";
import { ShinaClient } from "../src/session.ts";
import {
  isTailnetIp,
  type MobileSocket,
  type PinnedAddress,
  type PinnedRequest,
  type PinnedResponse,
  type SocketRequest,
} from "../src/transport.ts";
import { MemoryStore, rejectsErr } from "./fakes.ts";

const CONFIG_ENV = "SHINA_LIVE_CONFIG";
const FAKE_PUSH_TOKEN = "ExponentPushToken[fixture-live-0000000001]";
const PUSH_SKIP =
  "server did not negotiate mobile-push-v1; it requires client protocol minor >= 28";
const PROMPTS = [
  "sdk-live-fixture",
  "sdk-uncertain-fixture",
  "history-stream-fixture",
  "remote-confirm-fixture",
];

interface LiveConfig {
  pairLink: string;
  pinnedIp: string;
  dialHost: string;
  promptLog: string;
}

function loadConfig(): LiveConfig | null {
  const path = process.env[CONFIG_ENV];
  if (path === undefined || path === "") {
    return null;
  }
  const raw: unknown = JSON.parse(readFileSync(path, "utf8"));
  assert.ok(isObject(raw), "fixture config must be a JSON object");
  const config = raw as JsonObject;
  for (const key of ["pairLink", "pinnedIp", "dialHost", "promptLog"]) {
    assert.equal(typeof config[key], "string", `fixture config needs ${key}`);
  }
  return config as unknown as LiveConfig;
}

/** Map the one fixture tailnet target to the loopback port. Everything else
 * fails closed: this adapter never resolves or dials arbitrary hosts. */
function dialTarget(pinned: PinnedAddress, config: LiveConfig): { host: string; port: number } {
  if (!isTailnetIp(pinned.ip) || pinned.ip !== config.pinnedIp) {
    throw new Error(`Refusing to dial unpinned address ${pinned.ip}:${pinned.port}`);
  }
  return { host: config.dialHost, port: pinned.port };
}

function fixtureTransport(config: LiveConfig) {
  return (address: PinnedAddress, request: PinnedRequest): Promise<PinnedResponse> => {
    const target = dialTarget(address, config);
    const maxBody = request.maxBody ?? 8 * 1024 * 1024;
    return new Promise((resolve, reject) => {
      const headers: Record<string, string> = { ...(request.headers ?? {}) };
      let body: string | Uint8Array | undefined;
      if (request.json !== undefined) {
        headers["content-type"] = "application/json";
        body = JSON.stringify(request.json);
      } else if (request.body !== undefined) {
        body = request.body;
      }
      const req = http.request(
        {
          host: target.host,
          port: target.port,
          path: request.path,
          method: request.method,
          headers,
          timeout: request.timeoutMs ?? 15_000,
        },
        (res) => {
          const chunks: Buffer[] = [];
          let size = 0;
          res.on("data", (chunk: Buffer) => {
            size += chunk.length;
            if (size > maxBody) {
              req.destroy(new Error("Response exceeded the pinned byte budget."));
              return;
            }
            chunks.push(chunk);
          });
          res.on("end", () => {
            const responseHeaders: Record<string, string> = {};
            for (const [key, value] of Object.entries(res.headers)) {
              if (typeof value === "string") {
                responseHeaders[key] = value;
              }
            }
            resolve({
              status: res.statusCode ?? 0,
              headers: responseHeaders,
              body: new Uint8Array(Buffer.concat(chunks)),
            });
          });
        },
      );
      req.on("timeout", () => req.destroy(new Error("Pinned transport timed out.")));
      req.on("error", reject);
      if (body !== undefined) {
        req.end(body);
      } else {
        req.end();
      }
    });
  };
}

/** Test-only socket adapter on `ws`: sends the authorization header in the
 * upgrade, injects no Origin, and reports a refused upgrade's HTTP status
 * through onclose. Production platforms ship their own adapters. */
class WsSocket implements MobileSocket {
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose:
    | ((event: { code?: number; reason?: string; httpStatus?: number | undefined }) => void)
    | null = null;
  onerror: ((event: unknown) => void) | null = null;
  #ws: WebSocket;
  #reportedClose = false;

  constructor(request: SocketRequest, config: LiveConfig) {
    const target = dialTarget(request.pinned, config);
    const url = new URL(request.url);
    if (url.hostname !== request.pinned.ip || url.protocol !== "ws:") {
      throw new Error("Socket URL must name the pinned literal address.");
    }
    this.#ws = new WebSocket(`ws://${target.host}:${target.port}${url.pathname}`, {
      headers: request.headers,
    });
    this.#ws.on("open", () => this.onopen?.());
    this.#ws.on("message", (data, isBinary) => {
      this.onmessage?.({ data: isBinary ? data : data.toString() });
    });
    this.#ws.on("error", (error) => this.onerror?.(error));
    this.#ws.on("close", (code, reason) => {
      this.#close({ code, reason: reason.toString() });
    });
    this.#ws.on("unexpected-response", (_request, response) => {
      this.#close({ code: 1006, httpStatus: response.statusCode });
      response.socket.destroy();
    });
  }

  #close(event: { code?: number; reason?: string; httpStatus?: number | undefined }): void {
    if (this.#reportedClose) {
      return;
    }
    this.#reportedClose = true;
    this.onclose?.(event);
  }

  get readyState(): number {
    return this.#ws.readyState;
  }

  send(text: string): void {
    this.#ws.send(text);
  }

  close(code?: number, reason?: string): void {
    this.#ws.close(code, reason);
  }
}

function prompts(path: string): string[] {
  if (!existsSync(path)) {
    return [];
  }
  return readFileSync(path, "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => {
      try {
        const parsed: unknown = JSON.parse(line);
        return typeof parsed === "string" ? parsed : line;
      } catch {
        return line;
      }
    });
}

function promptCount(path: string, text: string): number {
  return prompts(path).filter((prompt) => prompt === text).length;
}

async function waitFor(
  what: string,
  check: () => boolean | Promise<boolean>,
  timeoutMs = 20_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (await check()) {
      return;
    }
    if (Date.now() >= deadline) {
      throw new Error(`Timed out waiting for ${what}.`);
    }
    await delay(25);
  }
}

async function waitForValue<T>(
  what: string,
  check: () => T | null | undefined | Promise<T | null | undefined>,
  timeoutMs = 20_000,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await check();
    if (value !== null && value !== undefined) {
      return value;
    }
    if (Date.now() >= deadline) {
      throw new Error(`Timed out waiting for ${what}.`);
    }
    await delay(25);
  }
}

function threadView(client: ShinaClient, threadId: number): JsonObject | null {
  const threads = client.workspace()?.["threads"];
  if (!Array.isArray(threads)) {
    return null;
  }
  const found = threads.find((thread) => isObject(thread) && thread["id"] === threadId);
  return isObject(found) ? found : null;
}

function indexTitle(client: ShinaClient, threadId: number): unknown {
  const index = client.workspace()?.["index"];
  const threads = isObject(index) ? index["threads"] : undefined;
  if (!Array.isArray(threads)) {
    return undefined;
  }
  const found = threads.find((thread) => isObject(thread) && thread["id"] === threadId);
  return isObject(found) ? found["title"] : undefined;
}

function historyText(client: ShinaClient, threadId: number): string {
  const history = client.threadHistory(threadId);
  return history === null ? "" : JSON.stringify(history["messages"] ?? []);
}

function occurrences(text: string, needle: string): number {
  return text.split(needle).length - 1;
}

test(
  "mobile client core against a live shina serve",
  { timeout: 240_000 },
  async (t) => {
    const config = loadConfig();
    if (config === null) {
      t.skip(
        `${CONFIG_ENV} is not set; the Rust live test writes the fixture config and runs this file`,
      );
      return;
    }

    const link = PairLink.parse(config.pairLink);
    const store = new MemoryStore();
    const transport = fixtureTransport(config);
    const client = new ShinaClient({
      socket: (request) => new WsSocket(request, config),
      transport,
      uuid: randomUUID,
      timeouts: { helloMs: 10_000, readMs: 20_000, commandMs: 20_000 },
    });
    const phases: string[] = [];
    client.onEvent((event) => {
      if (event.type === "phase") {
        phases.push(event.phase);
      }
    });
    let credential: DeviceCredential;
    t.after(async () => {
      await client.close();
    });

    await t.test("redeems the pairing link into a stored credential", async () => {
      credential = await redeemPairLink(link, { transport, store, uuid: randomUUID });
      assert.equal(credential.serverId, link.serverId);
      assert.equal(credential.host, config.pinnedIp);
      assert.equal(credential.port, link.port);
      const stored = await loadCredential(store, link.serverId);
      assert.equal(stored?.deviceId, credential.deviceId);
      assert.equal(stored?.serverId, credential.serverId);
      assert.equal(stored?.host, credential.host);
      assert.equal(stored?.port, credential.port);
      // Token equality only as a boolean: never print the value.
      assert.equal(stored?.token === credential.token, true);
      assert.equal(store.values.has(credentialKey(link.serverId)), true);
    });

    await t.test("attaches with a pidless hello and negotiates the mobile surface", () => {
      return client.attach(credential).then(() => {
        assert.equal(client.phase, "ready");
        const info = client.info;
        assert.ok(info, "hello info present");
        assert.equal(info.serverId, credential.serverId);
        assert.equal(info.deviceId, credential.deviceId);
        assert.equal(info.canOperate, true);
        assert.equal(info.canApprove, true);
        assert.equal(info.canUpdate, true);
        for (const cap of ["projection-patches", "history-items", CAP_NOTIFICATIONS]) {
          assert.ok(info.capabilities.has(cap), `missing capability ${cap}`);
        }
        assert.ok(phases.includes("connecting"));
        assert.ok(phases.includes("ready"));
      });
    });

    await t.test("installs the compact workspace and scoped thread history", async () => {
      const thread = threadView(client, 1);
      assert.ok(thread, "seeded thread 1 in the compact workspace");
      assert.equal(thread["status"], "Ready");
      assert.ok(typeof thread["historyRevision"] === "string");
      const compactMessages = thread["messages"];
      assert.ok(
        !Array.isArray(compactMessages) || compactMessages.length === 0,
        "compact threads carry no transcript rows",
      );
      client.openThread(1);
      await waitFor("scoped history for thread 1", () => client.threadHistory(1) !== null);
      await waitFor("history paging complete", () => client.cache.complete(1));
      const history = client.threadHistory(1) as JsonObject;
      assert.equal(history["id"], 1);
      assert.ok(Array.isArray(history["messages"]));
    });

    const pushCapable = client.supports(CAP_PUSH_NOTIFICATIONS);
    t.diagnostic(pushCapable ? "push-negotiated" : "push-not-negotiated");

    await t.test("mutations ride the {commandId,args} envelope to durable receipts", async () => {
      const receipt = (await client.mutate("thread.rename", {
        threadId: 1,
        title: "sdk-live",
      })) as JsonObject;
      assert.equal(receipt["status"], "accepted");
      assert.equal(receipt["deviceId"], credential.deviceId);
      assert.equal(typeof receipt["commandId"], "string");
      assert.equal(typeof receipt["seq"], "number");
      await waitFor("renamed title in the projection", () => indexTitle(client, 1) === "sdk-live");
    });

    await t.test("uploads over the pinned transport and reads back through readBlob", async () => {
      // The wire identity must survive: the commit names the blob
      // `file-<sha256>.bin`, and readBlob accepts exactly that id shape.
      const bytes = new Uint8Array(300_000);
      for (let i = 0; i < bytes.length; i += 1) {
        bytes[i] = i % 251;
      }
      const sha256 = createHash("sha256").update(bytes).digest("hex");
      const pinned: PinnedAddress = { ip: credential.host, port: credential.port };
      const auth = { authorization: `Bearer ${credential.token}` };

      const started = await transport(pinned, {
        method: "POST",
        path: "/v1/uploads",
        headers: auth,
        json: {
          type: "file",
          name: "sdk-live.bin",
          mimeType: "application/octet-stream",
          length: bytes.length,
          sha256,
        },
        maxBody: 16 * 1024,
      });
      assert.equal(started.status, 200, "upload start refused");
      const start = JSON.parse(new TextDecoder().decode(started.body)) as JsonObject;
      const uploadId = start["uploadId"];
      assert.equal(typeof uploadId, "string");
      assert.equal(start["offset"], 0);

      // Resume at the acknowledged offset, exactly like a dropped write retry.
      const split = 200_000;
      const first = await transport(pinned, {
        method: "PUT",
        path: `/v1/uploads/${uploadId}?offset=0`,
        headers: auth,
        body: bytes.subarray(0, split),
        maxBody: 16 * 1024,
      });
      assert.equal(first.status, 200);
      const second = await transport(pinned, {
        method: "PUT",
        path: `/v1/uploads/${uploadId}?offset=${split}`,
        headers: auth,
        body: bytes.subarray(split),
        maxBody: 16 * 1024,
      });
      assert.equal(second.status, 200);

      const committed = await transport(pinned, {
        method: "POST",
        path: `/v1/uploads/${uploadId}/commit`,
        headers: auth,
        maxBody: 64 * 1024,
      });
      assert.equal(committed.status, 200, "upload commit refused");
      const done = JSON.parse(new TextDecoder().decode(committed.body)) as JsonObject;
      const blobId = `file-${sha256}.bin`;
      assert.equal(done["id"], blobId);

      // Whole read pages twice (300 KB over the 256 KB page limit) and lands
      // byte-identical; a single-page read reports the offset contract.
      const whole = await client.readBlob(blobId);
      assert.equal(whole.length, bytes.length);
      assert.deepEqual(whole.bytes, bytes);
      const page = await client.readBlob(blobId, { offset: split, limit: 4096 });
      assert.equal(page.length, bytes.length);
      assert.equal(page.nextOffset, split + 4096);
      assert.deepEqual(page.bytes, bytes.subarray(split, split + 4096));
      // Ids outside the served shapes are refused before any request.
      await rejectsErr(client.readBlob(`${sha256}`));
      await rejectsErr(client.readBlob("../../etc/passwd"));
    });

    await t.test("thread.send accepts once; replaying the same UUID returns the same receipt", async () => {
      const commandId = randomUUID();
      const first = await client.sendMessage(1, {
        text: "sdk-live-fixture",
        commandId,
      });
      assert.equal(first.status, "accepted");
      await waitFor("the first prompt reaching the fake harness",
        () => promptCount(config.promptLog, "sdk-live-fixture") >= 1);
      const replay = await client.retryCommand(commandId);
      assert.deepEqual(replay, first);
      await delay(300);
      assert.equal(
        promptCount(config.promptLog, "sdk-live-fixture"),
        1,
        "the server must dedupe the replayed command id",
      );
      assert.equal(client.commandState(commandId)?.state, "resolved");
      const stored = await client.receipt(commandId);
      assert.deepEqual(stored, first);
    });

    await t.test("a dropped ack is uncertain; resume resolves the durable outcome once", async () => {
      const commandId = randomUUID();
      const pending = client.sendMessage(1, {
        text: "sdk-uncertain-fixture",
        commandId,
      });
      client.suspend();
      const error = await rejectsErr(pending, UncertainCommandError);
      assert.equal(error.commandId, commandId);
      assert.equal(client.commandState(commandId)?.state, "uncertain");
      const refused = await rejectsErr(client.sendMessage(1, { text: "while-suspended" }), SuspendedError);
      assert.ok(refused instanceof SuspendedError);
      await client.resume();
      assert.equal(client.phase, "ready");
      const resolved = await client.resolveCommand(commandId);
      assert.ok(isObject(resolved));
      assert.equal(resolved["status"], "accepted");
      await waitFor("the uncertain prompt reaching the fake harness exactly once",
        () => promptCount(config.promptLog, "sdk-uncertain-fixture") >= 1);
      await delay(300);
      assert.equal(promptCount(config.promptLog, "sdk-uncertain-fixture"), 1);
    });

    await t.test("suspend and resume keep accepted work running and merge history", async () => {
      const sent = await client.sendMessage(1, { text: "history-stream-fixture" });
      assert.equal(sent.status, "accepted");
      await waitFor("stream start", () => threadView(client, 1)?.["status"] === "Running");
      await delay(1_200);
      client.suspend();
      await client.resume();
      assert.equal(client.phase, "ready");
      await waitFor(
        "streamed history to finish after resume",
        () => occurrences(historyText(client, 1), "stream-249") === 1,
        45_000,
      );
      await waitFor("thread completed", () => threadView(client, 1)?.["status"] === "Completed");
      const text = historyText(client, 1);
      assert.equal(occurrences(text, "history-stream-fixture"), 1);
      assert.equal(occurrences(text, "stream-249"), 1);
    });

    await t.test("the notification feed reports real committed events", async () => {
      const first = await client.notifications({ limit: 100 });
      assert.equal(first.serverId, credential.serverId);
      const completed = await waitForValue("run_completed feed event", async () => {
        const page = await client.notifications({ limit: 100 });
        return page.events.find(
          (event) => event.kind === "run_completed" && event.threadId === 1,
        );
      });
      assert.ok(completed.seq > 0);
    });

    await t.test("approval lands in the feed and answers over the wire", async () => {
      const sent = await client.sendMessage(1, { text: "remote-confirm-fixture" });
      assert.equal(sent.status, "accepted");
      await waitFor("confirm request in the projection", () => {
        const request = threadView(client, 1)?.["request"];
        return isObject(request) && request["method"] === "confirm";
      });
      const request = threadView(client, 1)?.["request"];
      assert.ok(isObject(request));
      assert.equal(request["id"], "remote-confirm");

      const approval = await waitForValue("approval feed event", async () => {
        const page = await client.notifications({ limit: 100 });
        return page.events.find(
          (event) =>
            event.kind === "approval" &&
            event.threadId === 1 &&
            event.requestId === "remote-confirm",
        );
      });

      if (pushCapable) {
        const got = await client.getNotification(approval.id);
        assert.equal(got?.kind, "approval");
        assert.equal(got?.threadId, 1);
        const nav = await client.resolvePushNavigation({
          serverId: credential.serverId,
          eventId: approval.id,
        });
        assert.equal(nav?.threadId, 1);
        assert.equal(nav?.kind, "approval");
        assert.equal(nav?.requestId, "remote-confirm");
        assert.equal(String(nav?.generation), String(request["runtimeGeneration"]));
        // Malformed and foreign payloads are refused without any send.
        await rejectsErr(
          client.resolvePushNavigation({ serverId: credential.deviceId, eventId: approval.id }),
        );
        await rejectsErr(client.resolvePushNavigation({ eventId: approval.id }));
      }

      const answered = await client.respondToRequest(
        1,
        "remote-confirm",
        String(request["runtimeGeneration"]),
        { requestContract: true, response: { decision: "accept" } },
      );
      assert.equal(answered.status, "accepted");
      await waitFor("request cleared from the projection",
        () => !isObject(threadView(client, 1)?.["request"]));
    });

    // Last, after every fixture turn has settled: register, read status and
    // unregister immediately, with no state-changing commands in between, so
    // no notification event can ever commit while the fake token is live.
    // Push jobs are only created for feed events, so the worker can never
    // carry this token to the real Expo endpoint.
    await t.test(
      "push register, status and unregister land as receipts",
      { skip: pushCapable ? false : PUSH_SKIP },
      async () => {
        const registered = await client.registerPushNotifications("android", FAKE_PUSH_TOKEN);
        assert.equal(registered.status, "accepted");
        const active = (await client.pushStatus()) as JsonObject;
        assert.equal(active["registered"], true);
        assert.equal(active["platform"], "android");
        assert.equal(
          JSON.stringify(active).includes("ExponentPushToken"),
          false,
          "push status must never echo the token",
        );
        const unregistered = await client.unregisterPushNotifications();
        assert.equal(unregistered.status, "accepted");
        const after = (await client.pushStatus()) as JsonObject;
        assert.equal(after["registered"], false);
        // A second unregister is a plain receipt, not an error path.
        const twice = await client.unregisterPushNotifications();
        assert.ok(twice.status === "accepted" || twice.status === "rejected");
      },
    );

    t.diagnostic(`fake prompts observed: ${prompts(config.promptLog).length}`);
  },
);
