//! Zero-network test adapters. Every transport/socket/store is in-process and
//! records its calls, so tests can assert exactly what the client sent.

import { readFileSync } from "node:fs";
import type { DeviceCredential } from "../src/pairing.ts";
import type {
  JsonObject,
  JsonValue,
} from "../src/protocol.ts";
import type {
  MobileSocket,
  PairingAdapters,
  PinnedAddress,
  PinnedRequest,
  PinnedResponse,
  PinnedTransport,
  SecureStore,
  SessionAdapters,
  SocketRequest,
  TailnetResolver,
  UuidSource,
} from "../src/transport.ts";
import { ShinaClient } from "../src/session.ts";

export const fixture = JSON.parse(
  readFileSync(new URL("./fixtures/session.json", import.meta.url), "utf8"),
) as {
  serverId: string;
  deviceId: string;
  hello: { jsonrpc: string; id: string; result: JsonObject };
  workspace: JsonObject;
  threadSnapshotPage: JsonObject;
  threadStreamEvent: JsonObject;
  notifications: JsonObject;
};

export const tick = (): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, 0));

/** Capture a rejection (node's assert.rejects resolves undefined). */
export async function rejectsErr<T extends Error>(
  promise: Promise<unknown>,
  ctor?: new (...args: never[]) => T,
): Promise<T> {
  try {
    await promise;
  } catch (error) {
    if (ctor !== undefined && !(error instanceof ctor)) {
      throw new Error(
        `Expected ${ctor.name}, got ${(error as Error)?.constructor?.name ?? "unknown"}.`,
      );
    }
    return error as T;
  }
  throw new Error("Expected the promise to reject.");
}

/** Capture a synchronous throw. */
export function throwsErr<T extends Error>(
  fn: () => unknown,
  ctor?: new (...args: never[]) => T,
): T {
  try {
    fn();
  } catch (error) {
    if (ctor !== undefined && !(error instanceof ctor)) {
      throw new Error(
        `Expected ${ctor.name}, got ${(error as Error)?.constructor?.name ?? "unknown"}.`,
      );
    }
    return error as T;
  }
  throw new Error("Expected the function to throw.");
}

export function makeUuid(): UuidSource {
  let next = 0;
  return () =>
    `00000000-0000-4000-8000-${(++next).toString(16).padStart(12, "0")}`;
}

export const uuid1 = "00000000-0000-4000-8000-000000000001";

export function credential(overrides: Partial<DeviceCredential> = {}): DeviceCredential {
  return {
    serverId: fixture.serverId,
    deviceId: fixture.deviceId,
    token: "tok_normalized",
    host: "100.64.0.10",
    port: 47329,
    pairedAt: 0,
    ...overrides,
  };
}

export class FakeSocket implements MobileSocket {
  readonly request: SocketRequest;
  sent: JsonObject[] = [];
  readyState = 0;
  closedWith: {
    code?: number | undefined;
    reason?: string | undefined;
  } | null = null;
  dropped = false;
  onopen: MobileSocket["onopen"] = null;
  onmessage: MobileSocket["onmessage"] = null;
  onclose: MobileSocket["onclose"] = null;
  onerror: MobileSocket["onerror"] = null;

  constructor(request: SocketRequest) {
    this.request = request;
  }

  send(text: string): void {
    this.sent.push(JSON.parse(text) as JsonObject);
  }

  close(code?: number, reason?: string): void {
    this.closedWith = { code, reason };
    if (this.readyState === 1 || this.readyState === 0) {
      this.readyState = 3;
      this.onclose?.({ code: code ?? 1000 });
    }
  }

  /** Simulate the socket upgrade succeeding. */
  open(): void {
    if (this.readyState !== 0) {
      return;
    }
    this.readyState = 1;
    this.onopen?.();
  }

  /** Simulate an upgrade refusal (optionally with an HTTP status). */
  refuseUpgrade(httpStatus?: number): void {
    this.readyState = 3;
    this.onclose?.({ code: 1006, httpStatus });
  }

  /** Simulate transport loss on an established socket. */
  drop(): void {
    this.dropped = true;
    this.readyState = 3;
    this.onclose?.({ code: 1006 });
  }

  /** Deliver a server message (object or raw text). */
  receive(value: unknown): void {
    if (this.readyState !== 1) {
      return;
    }
    this.onmessage?.({
      data: typeof value === "string" ? value : JSON.stringify(value),
    });
  }

  frames(method?: string): JsonObject[] {
    return this.sent.filter(
      (frame) => method === undefined || frame["method"] === method,
    );
  }

  lastFrame(method?: string): JsonObject | null {
    const frames = this.frames(method);
    return frames.length > 0 ? (frames[frames.length - 1] as JsonObject) : null;
  }
}

export class FakeSocketFactory {
  requests: SocketRequest[] = [];
  sockets: FakeSocket[] = [];
  next: ((request: SocketRequest) => FakeSocket) | null = null;

  readonly factory = (request: SocketRequest): FakeSocket => {
    this.requests.push(request);
    const socket = this.next ? this.next(request) : new FakeSocket(request);
    this.sockets.push(socket);
    return socket;
  };

  last(): FakeSocket {
    const socket = this.sockets[this.sockets.length - 1];
    if (!socket) {
      throw new Error("No socket was opened.");
    }
    return socket;
  }
}

export class MemoryStore implements SecureStore {
  values = new Map<string, string>();
  async get(key: string): Promise<string | null> {
    return this.values.get(key) ?? null;
  }
  async set(key: string, value: string): Promise<void> {
    this.values.set(key, value);
  }
  async delete(key: string): Promise<void> {
    this.values.delete(key);
  }
}

export interface TransportCall {
  address: PinnedAddress;
  request: PinnedRequest;
}

export interface ScriptedResponse {
  status: number;
  json?: JsonValue;
  body?: Uint8Array;
  headers?: Record<string, string>;
}

export function fakeTransport(
  routes: Record<
    string,
    ScriptedResponse | ((request: PinnedRequest) => ScriptedResponse)
  >,
): { transport: PinnedTransport; calls: TransportCall[] } {
  const calls: TransportCall[] = [];
  const transport: PinnedTransport = async (address, request) => {
    calls.push({ address, request });
    const route =
      routes[`${request.method} ${request.path}`] ??
      routes[`${request.method} ${request.path.split("?")[0]}`];
    if (!route) {
      return { status: 404, headers: {}, body: new Uint8Array() };
    }
    const response = typeof route === "function" ? route(request) : route;
    let body: Uint8Array;
    if (response.body instanceof Uint8Array) {
      body = response.body;
    } else if (response.json !== undefined) {
      body = new TextEncoder().encode(JSON.stringify(response.json));
    } else {
      body = new Uint8Array();
    }
    return {
      status: response.status,
      headers: response.headers ?? {},
      body,
    };
  };
  return { transport, calls };
}

export function adapters(overrides: Partial<SessionAdapters> = {}): SessionAdapters {
  return {
    socket: new FakeSocketFactory().factory,
    uuid: makeUuid(),
    ...overrides,
  };
}

export interface Harness {
  client: ShinaClient;
  sockets: FakeSocketFactory;
  store: MemoryStore;
  uuid: UuidSource;
  calls: TransportCall[];
}

export function harness(overrides: Partial<SessionAdapters> = {}): Harness {
  const sockets = new FakeSocketFactory();
  const store = new MemoryStore();
  const uuid = makeUuid();
  const client = new ShinaClient({
    socket: sockets.factory,
    uuid,
    ...overrides,
  });
  return { client, sockets, store, uuid, calls: [] };
}

export function helloResult(overrides: JsonObject = {}): JsonObject {
  return {
    jsonrpc: "2.0",
    id: "hello",
    result: { ...fixture.hello.result, ...overrides },
  };
}

export function subscribeReply(id: unknown): JsonObject {
  return { jsonrpc: "2.0", id: id as JsonValue, result: { subscriptionId: "workspace" } };
}

/** Drive a full attach: pin, socket open, hello, workspace.subscribe. Any
 * snapshot/patch notifications to deliver before the subscribe reply go in
 * `beforeReady`. Returns the established socket. */
export async function connect(
  client: ShinaClient,
  sockets: FakeSocketFactory,
  options: {
    cred?: DeviceCredential;
    hello?: JsonObject;
    beforeReady?: unknown[];
    workspace?: JsonObject;
  } = {},
): Promise<FakeSocket> {
  const attach = client.attach(options.cred ?? credential());
  await tick();
  const socket = sockets.last();
  socket.open();
  await tick();
  socket.receive(options.hello ?? helloResult());
  await tick();
  const subscription = socket.lastFrame("workspace.subscribe");
  if (!subscription) {
    throw new Error("No workspace.subscribe was sent.");
  }
  const beforeReady = options.beforeReady ?? [
    options.workspace ?? fixture.workspace,
    {
      jsonrpc: "2.0",
      method: "subscription.ready",
      params: { subscriptionId: "workspace", seq: 7, maintenance: null },
    },
  ];
  for (const message of beforeReady) {
    socket.receive(message);
  }
  socket.receive(subscribeReply(subscription["id"]));
  await attach;
  return socket;
}
