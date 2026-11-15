//! Connection/session core.
//!
//! - `attach(credential)` opens the pinned socket, sends hello, validates the
//!   exact server identity and the negotiated capability set, then
//!   resynchronizes the workspace subscription from stored cursors.
//! - `suspend()` closes the socket only; it never emits stop commands, and all
//!   in-flight reads/commands are fenced to their socket generation.
//! - `resume()` reuses the same credential and projection cursors; replay
//!   falls back to bounded snapshots when the server cannot replay.
//! - Authentication denial is terminal for the session until the caller pairs
//!   again and attaches a new credential; the client never re-enrolls.
//! - Mutations travel as `{commandId,args}` with the request id equal to the
//!   command id. A dropped acknowledgement rejects with an
//!   `UncertainCommandError` carrying the original id; nothing is resent with
//!   a fresh id. `receipt()`/`resolveCommand()`/`retryCommand()` provide the
//!   durable-resolution path.

import {
  AuthDeniedError,
  CapabilityError,
  IntegrityError,
  OfflineError,
  PairingError,
  ProtocolError,
  RefusedError,
  ShinaError,
  SuspendedError,
  TimeoutError,
  UncertainCommandError,
} from "./errors.ts";
import { ProjectionCache, ProjectionGapError } from "./projection.ts";
import {
  CAP_NOTIFICATIONS,
  CAP_PUSH_NOTIFICATIONS,
  DEFAULT_OUTSTANDING_COMMANDS,
  MAX_CONTROL,
  MAX_SNAPSHOT_BYTES,
  NOTIFICATIONS_UPDATE_REQUIRED,
  PROTOCOL_MAJOR,
  PROTOCOL_MINOR,
  PROJECTION_SCHEMA,
  isCommandMethod,
  isObject,
  isReadMethod,
  cloneJson,
  utf8Decode,
  utf8Length,
  type CommandMethod,
  type JsonObject,
  type JsonValue,
  type ReadMethod,
  type Receipt,
} from "./protocol.ts";
import {
  checkedUuid,
  hostIsLiteral,
  isCanonicalUuid,
  isCredentialToken,
  isTailnetIp,
  parsePairHost,
  parseIpLiteral,
  socketHostLiteral,
  type DeviceCredentialLike,
  type PinnedHttpRequest,
  type MobileSocket,
  type PinnedAddress,
  type SessionAdapters,
} from "./transport.ts";
import type { DeviceCredential } from "./pairing.ts";

export type SessionPhase =
  | "idle"
  | "connecting"
  | "ready"
  | "suspended"
  | "auth-denied"
  | "closed";

export interface SessionLimits {
  controlBytes: number;
  outstandingCommands: number;
  pageBytes: number;
  fileAttachmentBytes: number | null;
  historyLeaseCount: number | null;
  historyLeaseBytes: number | null;
  historyLeaseIdleSeconds: number | null;
  threadReplayEvents: number | null;
  threadReplayBytes: number | null;
}

export interface SessionInfo {
  serverId: string;
  deviceId: string;
  instanceId: string | null;
  epoch: string | null;
  binaryVersion: string | null;
  protocolMinor: number;
  serverTime: number | null;
  canOperate: boolean;
  canApprove: boolean;
  canUpdate: boolean;
  capabilities: ReadonlySet<string>;
  limits: SessionLimits;
}

export interface NotificationEvent {
  id: string;
  seq: number;
  kind: string;
  threadId: number;
  createdAt: number | null;
  runId: string | null;
  /** Request identity as the server stored it: a JSON value, not only a string. */
  requestId: JsonValue | null;
  generation: string | number | null;
  /** The raw feed event. */
  raw: JsonObject;
}

function parseNotificationEvent(event: JsonObject): NotificationEvent {
  const generation = event["generation"];
  return {
    id: typeof event["id"] === "string" ? event["id"] : "",
    seq: typeof event["seq"] === "number" ? event["seq"] : 0,
    kind: typeof event["kind"] === "string" ? event["kind"] : "",
    threadId: typeof event["threadId"] === "number" ? event["threadId"] : 0,
    createdAt: typeof event["createdAt"] === "number" ? event["createdAt"] : null,
    runId: typeof event["runId"] === "string" ? event["runId"] : null,
    requestId: event["requestId"] ?? null,
    generation:
      typeof generation === "number" || typeof generation === "string"
        ? generation
        : null,
    raw: event,
  };
}

export interface NotificationPage {
  events: NotificationEvent[];
  nextCursor: number | null;
  hasMore: boolean;
  cursorExpired: boolean;
  oldestSeq: number | null;
  serverId: string | null;
}

/** Push payload data is exactly `{serverId,eventId}` UUIDs; nothing else is
 * ever carried or trusted. */
export interface PushNavigationPayload {
  serverId: string;
  eventId: string;
}

export interface NavigationTarget {
  threadId: number;
  kind: string;
  eventId: string;
  seq: number;
  runId: string | null;
  requestId: JsonValue | null;
  generation: string | number | null;
  /** The raw feed event, for fields this typed surface does not yet cover. */
  raw: JsonObject;
}

export type SessionEvent =
  | { type: "phase"; phase: SessionPhase }
  | { type: "projected"; state: JsonObject }
  | { type: "notice"; text: string }
  | { type: "maintenance"; state: JsonValue };

interface PendingRequest {
  id: string;
  kind: "read" | "command" | "hello" | "subscribe";
  method: string;
  commandId: string | null;
  args: JsonValue | undefined;
  resolve: (value: JsonValue) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout> | null;
}

interface CommandRecord {
  method: CommandMethod;
  args: JsonObject;
  state: "sent" | "resolved" | "uncertain";
  result: JsonValue;
}

const COMMAND_REGISTRY_LIMIT = 128;
const RESUBSCRIBE_ATTEMPT_LIMIT = 3;
const LATE_RESPONSE_LIMIT = 64;
const NOTIFICATION_PAGE_MAX = 100;
const PUSH_SCAN_PAGE_LIMIT = 25;

function isAuthDenial(message: string): boolean {
  return (
    message.startsWith("Authentication denied") ||
    message.includes("Device unknown or revoked")
  );
}

export class ShinaClient {
  readonly cache = new ProjectionCache();

  #adapters: SessionAdapters;
  #socket: MobileSocket | null = null;
  #phase: SessionPhase = "idle";
  #generation = 0;
  #credential: DeviceCredentialLike | null = null;
  #pinned: PinnedAddress | null = null;
  #info: SessionInfo | null = null;
  #pending = new Map<string, PendingRequest>();
  #late = new Map<string, PendingRequest>();
  #commands = new Map<string, CommandRecord>();
  #listeners = new Set<(event: SessionEvent) => void>();
  #resubscribeAttempts = 0;
  #closed = false;

  constructor(adapters: SessionAdapters) {
    this.#adapters = adapters;
    if (typeof adapters.socket !== "function") {
      throw new PairingError(
        "Missing required adapter: socket factory. The client core has no WebSocket default; browser WebSocket cannot set the Authorization header.",
      );
    }
    if (typeof adapters.uuid !== "function") {
      throw new PairingError(
        "Missing required adapter: uuid source. Supply crypto.randomUUID or an equivalent.",
      );
    }
  }

  get phase(): SessionPhase {
    return this.#phase;
  }

  get info(): SessionInfo | null {
    return this.#info;
  }

  supports(capability: string): boolean {
    return this.#info?.capabilities.has(capability) ?? false;
  }

  onEvent(listener: (event: SessionEvent) => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  #emit(event: SessionEvent): void {
    for (const listener of this.#listeners) {
      try {
        listener(event);
      } catch {
        // A listener must not break the session loop.
      }
    }
  }

  #setPhase(phase: SessionPhase): void {
    if (this.#phase !== phase) {
      this.#phase = phase;
      this.#emit({ type: "phase", phase });
    }
  }

  /** Resolve the credential host to a pinned tailnet literal. */
  async #pin(credential: DeviceCredentialLike): Promise<PinnedAddress> {
    const literal = parseIpLiteral(credential.host);
    if (literal !== null) {
      if (!isTailnetIp(literal)) {
        throw new PairingError("The stored credential host is not a tailnet address.");
      }
      return { ip: literal, port: credential.port };
    }
    if (hostIsLiteral(credential.host)) {
      // Parseable literal that failed the tailnet rule above.
      throw new PairingError("The stored credential host is not a tailnet address.");
    }
    if (typeof this.#adapters.resolve !== "function") {
      throw new PairingError(
        "Missing required adapter: tailnet resolver (needed for a MagicDNS host).",
      );
    }
    let answers: readonly string[];
    try {
      answers = await this.#adapters.resolve(credential.host);
    } catch {
      throw new PairingError("Could not resolve the server name on the tailnet.");
    }
    const literals = answers
      .map((answer) => typeof answer === "string" ? parseIpLiteral(answer.trim()) : null);
    if (literals.length === 0 || literals.some((ip) => ip === null || !isTailnetIp(ip))) {
      throw new PairingError("The server name did not resolve to a tailnet address.");
    }
    return { ip: literals[0] as string, port: credential.port };
  }

  /** Open a session. `attach` is also the explicit re-pair path: a new
   * credential always clears a prior auth denial, an identical one does not. */
  async attach(credential: DeviceCredential): Promise<void> {
    if (credential == null ||
        !isCanonicalUuid(credential.serverId) || !isCanonicalUuid(credential.deviceId) ||
        !isCredentialToken(credential.token) || typeof credential.host !== "string" ||
        parsePairHost(credential.host) === null || !Number.isSafeInteger(credential.port) ||
        credential.port < 1 || credential.port > 65535) {
      throw new ProtocolError("The stored credential is malformed. Pair again.");
    }
    if (this.#closed) {
      throw new ShinaError("closed", "The client is closed.");
    }
    if (this.#credential !== null && this.#credential.serverId !== credential.serverId) {
      throw new IntegrityError("Use a separate client instance for each paired server.");
    }
    if (this.#phase === "auth-denied" && this.#credential !== null) {
      const same =
        this.#credential.deviceId === credential.deviceId &&
        this.#credential.token === credential.token;
      if (same) {
        throw new AuthDeniedError();
      }
    }
    const replacement = this.#credential !== null &&
      (this.#credential.deviceId !== credential.deviceId || this.#credential.token !== credential.token);
    await this.#teardown("idle");
    if (replacement) {
      // Receipts belong to the original device. Retrying one under a fresh
      // pairing would create a new command instead of resolving the old one.
      this.#commands.clear();
    }
    this.#credential = {
      serverId: credential.serverId,
      deviceId: credential.deviceId,
      token: credential.token,
      host: credential.host,
      port: credential.port,
    };
    await this.#connect();
  }

  /** Reopen the socket after suspend, reusing the stored credential and
   * projection cursors. Never called implicitly. */
  async resume(): Promise<void> {
    if (this.#closed) {
      throw new ShinaError("closed", "The client is closed.");
    }
    if (this.#phase === "auth-denied") {
      throw new AuthDeniedError();
    }
    if (this.#credential === null) {
      throw new OfflineError("No stored credential. Attach or pair first.");
    }
    if (this.#phase !== "suspended" && this.#phase !== "idle") {
      return;
    }
    await this.#connect();
  }

  /** Close the transport. No stop commands are issued; state and cursors are
   * kept so a later `resume()` can replay from them. */
  suspend(): void {
    if (this.#phase !== "ready" && this.#phase !== "connecting") {
      return;
    }
    this.#generation += 1;
    this.cache.transportLost();
    const socket = this.#socket;
    this.#socket = null;
    if (socket) {
      socket.onopen = null;
      socket.onmessage = null;
      socket.onclose = null;
      socket.onerror = null;
      try {
        socket.close(1000);
      } catch {
        // Close must never throw into the caller.
      }
    }
    this.#failPending(new SuspendedError());
    this.#setPhase("suspended");
  }

  /** End the session without revoking the device credential. */
  async close(): Promise<void> {
    if (this.#closed) {
      return;
    }
    this.suspend();
    this.#closed = true;
    this.#setPhase("closed");
  }

  /** Revoke this device on the server, then end the session. Call
   * `forgetCredential` on the secure store afterwards. */
  async unpair(): Promise<void> {
    const credential = this.#credential;
    if (credential) {
      this.#requireReady();
      const receipt = await this.mutate("devices.revoke", { deviceId: credential.deviceId });
      if (!isObject(receipt) || receipt["status"] !== "accepted") {
        throw new RefusedError("The server did not acknowledge revocation. The credential was kept.");
      }
    }
    await this.close();
    this.#credential = null;
  }

  async #teardown(to: SessionPhase): Promise<void> {
    this.suspend();
    this.#setPhase(to);
  }

  async #connect(): Promise<void> {
    const credential = this.#credential;
    if (credential === null) {
      throw new OfflineError("No stored credential. Attach or pair first.");
    }
    const generation = ++this.#generation;
    this.#setPhase("connecting");
    try {
      this.#pinned = await this.#pin(credential);
    } catch (error) {
      if (generation === this.#generation) this.#setPhase("suspended");
      throw error;
    }
    const address = this.#pinned;
    const url = `ws://${socketHostLiteral(address.ip)}:${address.port}/v1/ws`;
    let socket: MobileSocket;
    try {
      socket = await this.#adapters.socket({
        url,
        headers: { authorization: `Bearer ${credential.token}` },
        pinned: address,
      });
    } catch {
      if (generation === this.#generation) this.#setPhase("suspended");
      throw new PairingError("Could not open the server connection.");
    }
    if (generation !== this.#generation) {
      try {
        socket.close(1000);
      } catch {
        // Stale socket from a superseded attempt.
      }
      return;
    }
    this.#socket = socket;
    try {
      await this.#openSocket(socket, generation);
      const hello = await this.#hello(socket, generation);
      if (generation !== this.#generation) {
        return;
      }
      const info = this.#validateHello(hello, credential);
      this.#info = info;
      try {
        this.cache.reconnect(hello);
      } catch (error) {
        // Capability skew on history items is non-fatal for the workspace
        // subscription; the cache records updateRequired.
        for (const notice of this.cache.drainNotices()) {
          this.#emit({ type: "notice", text: notice });
        }
        if (!(error instanceof ProtocolError)) {
          throw error;
        }
      }
      await this.#subscribeWorkspace();
      if (generation !== this.#generation) {
        return;
      }
      this.#setPhase("ready");
      this.#resubscribeAttempts = 0;
      this.#driveHistory();
    } catch (error) {
      if (generation !== this.#generation) {
        return;
      }
      this.#failSocket(error as Error);
      throw error;
    }
  }

  #openSocket(socket: MobileSocket, generation: number): Promise<void> {
    return new Promise((resolve, reject) => {
      let settled = false;
      const fail = (message: string, httpStatus?: number) => {
        if (settled) {
          return;
        }
        settled = true;
        if (httpStatus === 401 || httpStatus === 403) {
          reject(new AuthDeniedError());
        } else {
          reject(new ProtocolError(message));
        }
      };
      socket.onopen = () => {
        if (generation !== this.#generation || settled) {
          return;
        }
        settled = true;
        resolve();
      };
      socket.onerror = () => {
        if (generation !== this.#generation) {
          return;
        }
        fail("The server connection failed.");
      };
      socket.onclose = (event) => {
        if (generation !== this.#generation) {
          return;
        }
        fail("The server connection closed before hello.", event?.httpStatus);
      };
    });
  }

  async #hello(socket: MobileSocket, generation: number): Promise<JsonValue> {
    const timeoutMs = this.#adapters.timeouts?.helloMs ?? 15_000;
    const request: JsonObject = {
      jsonrpc: "2.0",
      id: "hello",
      method: "hello",
      params: {
        protocolMajor: PROTOCOL_MAJOR,
        protocolMinor: PROTOCOL_MINOR,
        projectionSchema: PROJECTION_SCHEMA,
      },
    };
    const reply = new Promise<JsonValue>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete("hello");
        reject(new TimeoutError("Hello timed out."));
      }, timeoutMs);
      const done = (error: Error | null, value?: JsonValue) => {
        clearTimeout(timer);
        this.#pending.delete("hello");
        if (error) {
          reject(error);
        } else {
          resolve(value as JsonValue);
        }
      };
      this.#pending.set("hello", {
        id: "hello",
        kind: "hello",
        method: "hello",
        commandId: null,
        args: undefined,
        resolve: (value) => done(null, value),
        reject: (error) => done(error),
        timer,
      });
    });
    this.#wireSocket(socket, generation);
    this.#sendFrame(request);
    return reply;
  }

  /** Bind message/close handlers for this socket generation. Every handler
   * fences on the generation captured at wiring so a closing socket cannot
   * mutate state owned by its successor. */
  #wireSocket(socket: MobileSocket, generation: number): void {
    socket.onmessage = (event) => {
      if (generation !== this.#generation) {
        return;
      }
      this.#onMessage(event?.data);
    };
    socket.onclose = (event) => {
      if (generation !== this.#generation) {
        return;
      }
      const denial =
        event?.httpStatus === 401 || event?.httpStatus === 403
          ? new AuthDeniedError()
          : null;
      this.#dropSocket(denial ?? new OfflineError("The server connection closed."));
    };
    socket.onerror = () => {
      // Post-open errors arrive before onclose on every platform; the close
      // handler performs the state transition so it is safe to ignore these.
    };
  }

  #sendFrame(frame: JsonObject): boolean {
    const socket = this.#socket;
    if (!socket || socket.readyState !== 1) {
      return false;
    }
    const text = JSON.stringify(frame);
    if (utf8Length(text) > MAX_CONTROL) {
      throw new ProtocolError("The request exceeded the server's control size limit.");
    }
    try {
      socket.send(text);
    } catch {
      throw new OfflineError("The server connection could not send the request.");
    }
    return true;
  }

  #onMessage(data: unknown): void {
    let text: string;
    if (typeof data === "string") {
      text = data;
    } else if (data instanceof ArrayBuffer) {
      text = utf8Decode(new Uint8Array(data));
    } else if (ArrayBuffer.isView(data)) {
      text = utf8Decode(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
    } else {
      return;
    }
    if (utf8Length(text) > MAX_SNAPSHOT_BYTES) {
      this.#failSocket(
        new ProtocolError("An inbound message exceeded the client memory budget."),
      );
      return;
    }
    let message: unknown;
    try {
      message = JSON.parse(text);
    } catch {
      this.#failSocket(new ProtocolError("Received a malformed protocol frame."));
      return;
    }
    if (!isObject(message)) {
      return;
    }
    const id = message["id"];
    if (typeof id === "string" || typeof id === "number") {
      const key = String(id);
      const pending = this.#pending.get(key);
      if (pending) {
        this.#settle(pending, message);
        this.#afterInbound();
        return;
      }
      const late = this.#late.get(key);
      if (late) {
        this.#late.delete(key);
        this.#recordLate(late, message);
        return;
      }
    }
    try {
      this.cache.receive(message);
    } catch (error) {
      this.#onProjectionError(error as Error);
    }
    this.#afterInbound();
  }

  #afterInbound(): void {
    const projected = this.cache.projected();
    if (projected !== null) {
      this.#emit({ type: "projected", state: projected });
    }
    if (this.cache.maintenance !== null) {
      this.#emit({ type: "maintenance", state: this.cache.maintenance });
      this.cache.maintenance = null;
    }
    for (const notice of this.cache.drainNotices()) {
      this.#emit({ type: "notice", text: notice });
    }
    this.#driveHistory();
  }

  #driveHistory(): void {
    if (this.#phase !== "ready") {
      return;
    }
    for (;;) {
      let request;
      try {
        request = this.cache.nextHistory();
      } catch (error) {
        this.#onProjectionError(error as Error);
        return;
      }
      if (!request) {
        return;
      }
      if (!this.#sendFrame(request as unknown as JsonObject)) {
        return;
      }
    }
  }

  #onProjectionError(error: Error): void {
    if (error instanceof ProjectionGapError || error instanceof ProtocolError) {
      this.#emit({ type: "notice", text: error.message });
      if (this.#resubscribeAttempts < RESUBSCRIBE_ATTEMPT_LIMIT) {
        this.#resubscribeAttempts += 1;
        this.cache.seq = null;
        this.cache.epoch = null;
        void this.#subscribeWorkspace().catch((subscribeError) => {
          if (!isAuthDenial((subscribeError as Error).message)) {
            this.#emit({
              type: "notice",
              text: `Workspace resubscribe failed: ${(subscribeError as Error).message}`,
            });
          }
        });
      }
      return;
    }
    this.#failSocket(error);
  }

  async #subscribeWorkspace(): Promise<void> {
    const params: JsonObject = {};
    if (this.cache.seq !== null) {
      params["lastSeq"] = this.cache.seq;
    }
    if (this.cache.epoch !== null) {
      params["epoch"] = this.cache.epoch;
    }
    await this.#request(
      "workspace.subscribe",
      params,
      "subscribe",
      this.#adapters.timeouts?.readMs ?? 30_000,
    );
  }

  #validateHello(hello: JsonValue, credential: DeviceCredentialLike): SessionInfo {
    if (!isObject(hello)) {
      throw new ProtocolError("Invalid hello response.");
    }
    if (hello["protocolMajor"] !== PROTOCOL_MAJOR) {
      throw new ProtocolError(
        `The server speaks protocol major ${String(hello["protocolMajor"])}; this client speaks ${PROTOCOL_MAJOR}.`,
      );
    }
    if (hello["projectionSchema"] !== PROJECTION_SCHEMA ||
        !Number.isSafeInteger(hello["protocolMinor"]) ||
        (hello["protocolMinor"] as number) < 0 ||
        (hello["protocolMinor"] as number) > PROTOCOL_MINOR) {
      throw new ProtocolError("The server returned an unsupported protocol or projection version.");
    }
    if (hello["serverId"] !== credential.serverId) {
      throw new ProtocolError(
        "The connected server is not the server this device paired with.",
      );
    }
    if (hello["deviceId"] !== credential.deviceId) {
      throw new AuthDeniedError(
        "The server attributed this connection to a different device. Pair again.",
      );
    }
    const capabilities = Array.isArray(hello["capabilities"])
      ? new Set(hello["capabilities"].filter((cap): cap is string => typeof cap === "string"))
      : new Set<string>();
    const limits = isObject(hello["limits"]) ? hello["limits"] : {};
    const numberOrNull = (key: string): number | null => {
      const value = limits[key];
      if (value === undefined || value === null) return null;
      if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
        throw new ProtocolError("The server returned invalid protocol limits.");
      }
      return value;
    };
    return {
      serverId: hello["serverId"] as string,
      deviceId: hello["deviceId"] as string,
      instanceId: typeof hello["instanceId"] === "string" ? hello["instanceId"] : null,
      epoch: typeof hello["epoch"] === "string" ? hello["epoch"] : null,
      binaryVersion:
        typeof hello["binaryVersion"] === "string" ? hello["binaryVersion"] : null,
      protocolMinor:
        typeof hello["protocolMinor"] === "number" ? hello["protocolMinor"] : 0,
      serverTime: typeof hello["serverTime"] === "number" ? hello["serverTime"] : null,
      canOperate: hello["canOperate"] === true,
      canApprove: hello["canApprove"] === true,
      canUpdate: hello["canUpdate"] === true,
      capabilities,
      limits: {
        controlBytes: numberOrNull("controlBytes") ?? MAX_CONTROL,
        outstandingCommands:
          Math.max(1, Math.min(numberOrNull("outstandingCommands") ?? DEFAULT_OUTSTANDING_COMMANDS, DEFAULT_OUTSTANDING_COMMANDS)),
        pageBytes: numberOrNull("pageBytes") ?? 256 * 1024,
        fileAttachmentBytes: numberOrNull("fileAttachmentBytes"),
        historyLeaseCount: numberOrNull("historyLeaseCount"),
        historyLeaseBytes: numberOrNull("historyLeaseBytes"),
        historyLeaseIdleSeconds: numberOrNull("historyLeaseIdleSeconds"),
        threadReplayEvents: numberOrNull("threadReplayEvents"),
        threadReplayBytes: numberOrNull("threadReplayBytes"),
      },
    };
  }

  #request(
    method: string,
    params: JsonObject,
    kind: PendingRequest["kind"],
    timeoutMs: number,
    commandId?: string,
    args?: JsonValue,
  ): Promise<JsonValue> {
    if (this.#phase !== "ready" && kind !== "subscribe") {
      if (this.#phase === "suspended") {
        return Promise.reject(new SuspendedError());
      }
      return Promise.reject(new OfflineError());
    }
    const id = commandId ?? checkedUuid(this.#adapters.uuid, "uuid");
    const frame: JsonObject = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const pending = this.#pending.get(id);
        if (!pending) {
          return;
        }
        this.#pending.delete(id);
        if (pending.kind === "command" && pending.commandId !== null) {
          this.#late.set(id, pending);
          if (this.#late.size > LATE_RESPONSE_LIMIT) {
            const oldest = this.#late.keys().next().value;
            if (oldest !== undefined) {
              this.#late.delete(oldest);
            }
          }
          const record = this.#commands.get(pending.commandId);
          if (record) {
            record.state = "uncertain";
          }
          reject(
            new UncertainCommandError(pending.commandId, pending.method),
          );
        } else {
          reject(new TimeoutError());
        }
      }, timeoutMs);
      this.#pending.set(id, {
        id,
        kind,
        method,
        commandId: commandId ?? null,
        args,
        resolve,
        reject,
        timer,
      });
      try {
        if (!this.#sendFrame(frame)) {
          throw new OfflineError();
        }
      } catch (error) {
        clearTimeout(timer);
        this.#pending.delete(id);
        if (kind === "command" && commandId && error instanceof OfflineError) {
          const record = this.#commands.get(commandId);
          if (record) record.state = "uncertain";
          reject(new UncertainCommandError(commandId, method));
        } else {
          reject(error);
        }
      }
    });
  }

  #settle(pending: PendingRequest, message: JsonObject): void {
    this.#pending.delete(pending.id);
    if (pending.timer !== null) {
      clearTimeout(pending.timer);
    }
    if (message["error"] !== null && message["error"] !== undefined) {
      const error = isObject(message["error"]) ? message["error"] : {};
      const raw =
        typeof error["message"] === "string" ? error["message"] : "Request failed.";
      const token = this.#credential?.token;
      const text = token ? raw.split(token).join("[redacted]") : raw;
      if (isAuthDenial(text)) {
        this.#denyAuth();
        pending.reject(new AuthDeniedError(text));
        return;
      }
      pending.reject(
        new RefusedError(text, typeof error["code"] === "number" ? error["code"] : undefined),
      );
      return;
    }
    const result = message["result"] ?? null;
    if (pending.commandId !== null) {
      const record = this.#commands.get(pending.commandId);
      if (record) {
        record.state = "resolved";
        record.result = cloneJson(result);
      }
    }
    pending.resolve(result);
  }

  /** A late command response still updates the command registry so the app
   * can inspect the result after an UncertainCommandError. */
  #recordLate(pending: PendingRequest, message: JsonObject): void {
    if (pending.commandId === null) {
      return;
    }
    const record = this.#commands.get(pending.commandId);
    if (!record) {
      return;
    }
    if (message["error"] !== null && message["error"] !== undefined) {
      return;
    }
    record.state = "resolved";
    record.result = cloneJson(message["result"] ?? null);
  }

  /** Socket dropped: reads fail closed, commands become uncertain. */
  #dropSocket(error: Error): void {
    this.#generation += 1;
    this.cache.transportLost();
    this.#socket = null;
    const authDenial = error instanceof AuthDeniedError;
    for (const pending of this.#pending.values()) {
      if (pending.timer !== null) {
        clearTimeout(pending.timer);
      }
      if (pending.kind === "command" && pending.commandId !== null && !authDenial) {
        const record = this.#commands.get(pending.commandId);
        if (record) {
          record.state = "uncertain";
        }
        pending.reject(
          new UncertainCommandError(pending.commandId, pending.method),
        );
      } else {
        pending.reject(authDenial ? new AuthDeniedError() : error);
      }
    }
    this.#pending.clear();
    this.#late.clear();
    if (authDenial) {
      this.#setPhase("auth-denied");
    } else {
      this.#setPhase("suspended");
    }
  }

  #failPending(error: Error): void {
    for (const pending of this.#pending.values()) {
      if (pending.timer !== null) {
        clearTimeout(pending.timer);
      }
      if (pending.kind === "command" && pending.commandId !== null) {
        const record = this.#commands.get(pending.commandId);
        if (record) {
          record.state = "uncertain";
        }
        pending.reject(new UncertainCommandError(pending.commandId, pending.method));
      } else {
        pending.reject(error);
      }
    }
    this.#pending.clear();
    this.#late.clear();
  }

  #failSocket(error: Error): void {
    const socket = this.#socket;
    this.#socket = null;
    if (socket) {
      socket.onopen = null;
      socket.onmessage = null;
      socket.onclose = null;
      socket.onerror = null;
      try {
        socket.close(1000);
      } catch {
        // Best-effort close.
      }
    }
    this.#dropSocket(error);
  }

  #denyAuth(): void {
    const socket = this.#socket;
    this.#socket = null;
    if (socket) {
      try {
        socket.close(1000);
      } catch {
        // Best-effort close.
      }
    }
    this.#dropSocket(new AuthDeniedError());
  }

  #requireReady(): void {
    if (this.#phase === "suspended") {
      throw new SuspendedError();
    }
    if (this.#phase === "auth-denied") {
      throw new AuthDeniedError();
    }
    if (this.#phase !== "ready") {
      throw new OfflineError();
    }
  }

  /** Typed read. Plain-params methods only; everything else is a mutation. */
  async read<R = JsonValue>(
    method: ReadMethod,
    params: JsonObject = {},
    options: { timeoutMs?: number } = {},
  ): Promise<R> {
    this.#requireReady();
    if (!isReadMethod(method)) {
      throw new ProtocolError(`Unknown read method ${method}.`);
    }
    const result = await this.#request(
      method,
      params,
      "read",
      options.timeoutMs ?? this.#adapters.timeouts?.readMs ?? 30_000,
    );
    return result as R;
  }

  /**
   * Typed mutation. The wire body is `{commandId,args}`; the response is the
   * method result (a durable `Receipt` for runtime-dispatched commands).
   * Rejects while offline rather than queueing. A lost acknowledgement
   * rejects with `UncertainCommandError` and is resolved through
   * `receipt`/`resolveCommand`, never by an automatic resend.
   */
  async mutate(
    method: CommandMethod,
    args: JsonObject = {},
    options: { commandId?: string | undefined; timeoutMs?: number | undefined } = {},
  ): Promise<JsonValue> {
    this.#requireReady();
    if (!isCommandMethod(method)) {
      throw new ProtocolError(`Unknown mutation method ${method}.`);
    }
    const commandId = options.commandId ?? checkedUuid(this.#adapters.uuid, "uuid");
    if (!isCanonicalUuid(commandId)) {
      throw new ProtocolError("commandId must be a canonical lowercase UUID.");
    }
    const outstanding = [...this.#pending.values()].filter(
      (pending) => pending.kind === "command",
    ).length;
    if (outstanding >= (this.#info?.limits.outstandingCommands ?? DEFAULT_OUTSTANDING_COMMANDS)) {
      throw new RefusedError("Too many commands are waiting for receipts. Retry shortly.");
    }
    const captured = cloneJson(args) as JsonObject;
    this.#commands.set(commandId, {
      method,
      args: captured,
      state: "sent",
      result: null,
    });
    if (this.#commands.size > COMMAND_REGISTRY_LIMIT) {
      const oldest = this.#commands.keys().next().value;
      if (oldest !== undefined) {
        this.#commands.delete(oldest);
      }
    }
    return this.#request(
      method,
      { commandId, args: captured },
      "command",
      options.timeoutMs ?? this.#adapters.timeouts?.commandMs ?? 30_000,
      commandId,
      captured,
    );
  }

  /** Durable receipt lookup. Returns null when the server stored none. */
  async receipt(commandId: string): Promise<Receipt | null> {
    this.#requireReady();
    try {
      const result = await this.read("command.get", { commandId });
      return isObject(result) && typeof result["commandId"] === "string"
        ? (result as unknown as Receipt)
        : null;
    } catch (error) {
      if (error instanceof RefusedError && error.message.includes("No receipt")) {
        return null;
      }
      throw error;
    }
  }

  /** Resend a previously uncertain command with the SAME command id and args. */
  async retryCommand(commandId: string): Promise<JsonValue> {
    const record = this.#commands.get(commandId);
    if (!record) {
      throw new ProtocolError("No recorded command carries this id.");
    }
    return this.mutate(record.method, record.args, { commandId });
  }

  /** Resolve an uncertain command: prefer the durable receipt; when none was
   * stored, resend with the same id so the server deduplicates. */
  async resolveCommand(commandId: string): Promise<Receipt | JsonValue> {
    const existing = await this.receipt(commandId);
    if (existing !== null) {
      return existing;
    }
    return this.retryCommand(commandId);
  }

  /** The state of a sent command as this client last knew it. */
  commandState(commandId: string): CommandRecord | null {
    const record = this.#commands.get(commandId);
    return record ? { ...record, args: cloneJson(record.args) as JsonObject, result: cloneJson(record.result) } : null;
  }

  /** The workspace projection annotated with thread histories and metadata.
   * Null before the first snapshot. */
  workspace(): JsonObject | null {
    return this.cache.projected(true);
  }

  /** The installed transcript for a thread, or null while unready/evicted. */
  threadHistory(threadId: number): JsonObject | null {
    return this.cache.history(threadId);
  }

  /** Keep a thread's stream subscription live. One thread streams at a time;
   * opening another moves the live subscription. */
  openThread(threadId: number): void {
    this.#requireReady();
    this.cache.desiredThread = threadId;
    this.#driveHistory();
  }

  releaseThread(): void {
    this.cache.desiredThread = null;
  }

  /** Request the next older history page for a thread. Lazy by design. */
  loadEarlier(threadId: number, throughEntryId: string | null = null): void {
    this.#requireReady();
    this.cache.requestEarlier(threadId, throughEntryId);
    this.#driveHistory();
  }

  /** Expand a deferred-work marker row (deferred-work capability only). */
  loadWork(threadId: number, anchorId: string): void {
    this.#requireReady();
    this.cache.requestWork(threadId, anchorId);
    this.#driveHistory();
  }

  /** The notification feed. Only usable when the server negotiated
   * `notification-feed-v1`; otherwise a CapabilityError, never a minor check. */
  async notifications(
    options: { afterSequence?: number; limit?: number } = {},
  ): Promise<NotificationPage> {
    this.#requireReady();
    if (!this.supports(CAP_NOTIFICATIONS)) {
      throw new CapabilityError(CAP_NOTIFICATIONS, NOTIFICATIONS_UPDATE_REQUIRED);
    }
    const params: JsonObject = {};
    if (options.afterSequence !== undefined) {
      params["afterSequence"] = options.afterSequence;
    }
    if (options.limit !== undefined) {
      if (!Number.isSafeInteger(options.limit) || options.limit < 1 || options.limit > NOTIFICATION_PAGE_MAX) {
        throw new ProtocolError("Notification page limit must be 1..100.");
      }
      params["limit"] = options.limit;
    }
    const result = await this.read("notifications.list", params);
    if (!isObject(result)) {
      throw new ProtocolError("Invalid notification page.");
    }
    if (
      typeof result["serverId"] === "string" &&
      this.#info !== null &&
      result["serverId"] !== this.#info.serverId
    ) {
      throw new ProtocolError("Notification feed serverId mismatch.");
    }
    return {
      events: (Array.isArray(result["events"]) ? result["events"] : [])
        .filter(isObject)
        .map((event) => parseNotificationEvent(event)),
      nextCursor: typeof result["nextCursor"] === "number" ? result["nextCursor"] : null,
      hasMore: result["hasMore"] === true,
      cursorExpired: result["cursorExpired"] === true,
      oldestSeq: typeof result["oldestSeq"] === "number" ? result["oldestSeq"] : null,
      serverId: typeof result["serverId"] === "string" ? result["serverId"] : null,
    };
  }

  /** Bounded forward scan for one feed event by its opaque id. For durable
   * catch-up persist the last event seq; `nextCursor` is pagination state only. */
  async findNotification(eventId: string): Promise<NotificationEvent | null> {
    let after: number | undefined;
    for (let pages = 0; pages < PUSH_SCAN_PAGE_LIMIT; pages += 1) {
      const page = await this.notifications(
        after === undefined ? {} : { afterSequence: after },
      );
      const found = page.events.find((event) => event.id === eventId);
      if (found) {
        return found;
      }
      if (!page.hasMore || page.nextCursor === null) {
        return null;
      }
      after = page.nextCursor;
    }
    return null;
  }

  /** Fetch one feed event by opaque id (`notifications.get`, mobile-push-v1).
   * Returns the CURRENT routing state for the event; null when the server no
   * longer retains it. */
  async getNotification(eventId: string): Promise<NotificationEvent | null> {
    this.#requireReady();
    if (!this.supports(CAP_PUSH_NOTIFICATIONS)) {
      throw new CapabilityError(
        CAP_PUSH_NOTIFICATIONS,
        "The server did not negotiate mobile push notifications.",
      );
    }
    try {
      const result = await this.read("notifications.get", { eventId });
      const event = isObject(result) && isObject(result["event"]) ? result["event"] : result;
      if (!isObject(event)) {
        return null;
      }
      return parseNotificationEvent(event);
    } catch (error) {
      if (
        error instanceof RefusedError &&
        /not found|unknown|expired|no longer available|invalid/i.test(error.message)
      ) {
        return null;
      }
      throw error;
    }
  }

  /**
   * Resolve an opaque push payload to its navigation target.
   *
   * Only usable when the server negotiated `mobile-push-v1`; without it this
   * reports unsupported WITHOUT sending anything. Payloads must be exactly
   * `{serverId,eventId}` UUIDs: malformed data and any serverId that is not
   * this device's paired server are rejected before the first request. The
   * lookup only reads routing state — it never approves, never pairs.
   */
  async resolvePushNavigation(payload: unknown): Promise<NavigationTarget | null> {
    this.#requireReady();
    if (!this.supports(CAP_PUSH_NOTIFICATIONS)) {
      throw new CapabilityError(
        CAP_PUSH_NOTIFICATIONS,
        "The server did not negotiate mobile push notifications.",
      );
    }
    if (
      !isObject(payload) ||
      typeof payload["serverId"] !== "string" ||
      typeof payload["eventId"] !== "string" ||
      Object.keys(payload).length !== 2 ||
      !isCanonicalUuid(payload["serverId"]) ||
      !isCanonicalUuid(payload["eventId"])
    ) {
      throw new ProtocolError("Malformed push navigation payload.");
    }
    if (this.#info === null || payload["serverId"] !== this.#info.serverId) {
      throw new IntegrityError(
        "The push payload names a server this device is not paired with.",
      );
    }
    const event = await this.getNotification(payload["eventId"]);
    if (!event) {
      return null;
    }
    return {
      threadId: event.threadId,
      kind: event.kind,
      eventId: event.id,
      seq: event.seq,
      runId: event.runId,
      requestId: event.requestId,
      generation: event.generation,
      raw: event.raw,
    };
  }

  /** Register this device for mobile push delivery (mobile-push-v1). */
  async registerPushNotifications(
    platform: "ios" | "android",
    token: string,
    options: { commandId?: string } = {},
  ): Promise<Receipt> {
    this.#requireReady();
    if (!this.supports(CAP_PUSH_NOTIFICATIONS)) {
      throw new CapabilityError(
        CAP_PUSH_NOTIFICATIONS,
        "The server did not negotiate mobile push notifications.",
      );
    }
    if (platform !== "ios" && platform !== "android") {
      throw new ProtocolError("Push platform must be 'ios' or 'android'.");
    }
    if (typeof token !== "string" || token.trim() === "") {
      throw new ProtocolError("A non-empty Expo push token is required.");
    }
    const result = await this.mutate(
      "notifications.push.register",
      { platform, token },
      { commandId: options.commandId },
    );
    return result as unknown as Receipt;
  }

  /** Stop push delivery for this device (mobile-push-v1). */
  async unregisterPushNotifications(
    options: { commandId?: string } = {},
  ): Promise<Receipt> {
    this.#requireReady();
    if (!this.supports(CAP_PUSH_NOTIFICATIONS)) {
      throw new CapabilityError(
        CAP_PUSH_NOTIFICATIONS,
        "The server did not negotiate mobile push notifications.",
      );
    }
    const result = await this.mutate(
      "notifications.push.unregister",
      {},
      { commandId: options.commandId },
    );
    return result as unknown as Receipt;
  }

  /** Current push registration state for this device (mobile-push-v1). */
  async pushStatus(): Promise<JsonValue> {
    this.#requireReady();
    if (!this.supports(CAP_PUSH_NOTIFICATIONS)) {
      throw new CapabilityError(
        CAP_PUSH_NOTIFICATIONS,
        "The server did not negotiate mobile push notifications.",
      );
    }
    return this.read("notifications.push.status", {});
  }

  /** Send a prompt/steer/queue message into a thread. Returns the durable
   * receipt; `status` distinguishes accepted/rejected server-side. */
  async sendMessage(
    threadId: number,
    options: {
      text: string;
      attachments?: JsonValue[];
      mode?: "prompt" | "steer" | "queue";
      draft?: JsonValue;
      context?: JsonValue;
      editorRevision?: string;
      envMode?: "local" | "worktree";
      baseBranch?: string;
      startFromOrigin?: boolean;
      freshContext?: boolean;
      compactBeforeSend?: boolean;
      stagedUploadIds?: string[];
      commandId?: string;
    },
  ): Promise<Receipt> {
    const args: JsonObject = {
      threadId,
      text: options.text,
      attachments: options.attachments ?? [],
      mode: options.mode ?? "prompt",
    };
    if (options.draft !== undefined) args["draft"] = options.draft;
    if (options.context !== undefined) args["context"] = options.context;
    if (options.editorRevision !== undefined) {
      args["editorRevision"] = options.editorRevision;
    }
    if (options.envMode !== undefined) args["envMode"] = options.envMode;
    if (options.baseBranch !== undefined) args["baseBranch"] = options.baseBranch;
    if (options.startFromOrigin !== undefined) {
      args["startFromOrigin"] = options.startFromOrigin;
    }
    if (options.freshContext !== undefined) args["freshContext"] = options.freshContext;
    if (options.compactBeforeSend !== undefined) {
      args["compactBeforeSend"] = options.compactBeforeSend;
    }
    if (options.stagedUploadIds !== undefined) {
      args["stagedUploadIds"] = options.stagedUploadIds;
    }
    const result = await this.mutate("thread.send", args, {
      commandId: options.commandId,
    });
    return result as unknown as Receipt;
  }

  /** Stop the active run. `runId` carries the observed run identity so a stale
   * caller never stops a newer run. */
  async stopThread(
    threadId: number,
    runId?: string,
    options: { commandId?: string } = {},
  ): Promise<Receipt> {
    const args: JsonObject = { threadId };
    if (runId !== undefined) {
      args["runId"] = runId;
    }
    const result = await this.mutate("thread.stop", args, {
      commandId: options.commandId,
    });
    return result as unknown as Receipt;
  }

  /** Resume a thread's provider. `generation` must be the thread's current
   * request generation, which fences resumes against runtime restarts. */
  async resumeThread(
    threadId: number,
    generation: string,
    options: {
      continuationOfRunId?: string;
      releaseQueue?: boolean;
      commandId?: string;
    } = {},
  ): Promise<Receipt> {
    const args: JsonObject = { threadId, generation };
    if (options.continuationOfRunId !== undefined) {
      args["continuationOfRunId"] = options.continuationOfRunId;
    }
    if (options.releaseQueue !== undefined) {
      args["releaseQueue"] = options.releaseQueue;
    }
    const result = await this.mutate("thread.resume", args, {
      commandId: options.commandId,
    });
    return result as unknown as Receipt;
  }

  /** Answer an approval/question request, carrying the original request
   * generation so stale answers cannot satisfy a newer request. */
  async respondToRequest(
    threadId: number,
    requestId: string,
    generation: string,
    options: {
      requestContract?: boolean;
      questionContract?: boolean;
      answers?: JsonValue;
      response?: JsonValue;
      commandId?: string;
    } = {},
  ): Promise<Receipt> {
    const args: JsonObject = { threadId, requestId, generation };
    if (options.requestContract !== undefined) {
      args["requestContract"] = options.requestContract;
    }
    if (options.questionContract !== undefined) {
      args["questionContract"] = options.questionContract;
    }
    if (options.answers !== undefined) args["answers"] = options.answers;
    if (options.response !== undefined) args["response"] = options.response;
    const result = await this.mutate("request.respond", args, {
      commandId: options.commandId,
    });
    return result as unknown as Receipt;
  }

  /** Dismiss a question/input request, generation-fenced like respond. */
  async dismissInputRequest(
    threadId: number,
    requestId: string,
    generation: string,
    options: { commandId?: string } = {},
  ): Promise<Receipt> {
    const result = await this.mutate(
      "thread.user-input.dismiss",
      { threadId, requestId, generation },
      { commandId: options.commandId },
    );
    return result as unknown as Receipt;
  }

  /** Read one blob page (or the whole blob) through the pinned transport. */
  async readBlob(
    id: string,
    options: { offset?: number; limit?: number; maxBytes?: number } = {},
  ): Promise<{ bytes: Uint8Array; length: number; nextOffset: number }> {
    const transport = this.#adapters.transport;
    if (typeof transport !== "function") {
      throw new CapabilityError(
        "pinned-transport",
        "Blob reads need the pinned HTTP transport adapter.",
      );
    }
    this.#requireReady();
    const image = id.length <= 96 && !id.startsWith(".") &&
      /^[A-Za-z0-9_.-]+\.(png|jpg|gif|webp)$/.test(id);
    const file = /^file-[0-9a-f]{64}\.bin$/.test(id);
    const html = /^html-[0-9]+-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.html$/.test(id);
    if (!image && !file && !html) {
      throw new ProtocolError("Invalid blob id.");
    }
    const offset = options.offset ?? 0;
    const max = options.maxBytes ?? MAX_SNAPSHOT_BYTES;
    if (!Number.isSafeInteger(offset) || offset < 0 ||
        !Number.isSafeInteger(max) || max < 1 || max > MAX_SNAPSHOT_BYTES ||
        options.limit !== undefined && (!Number.isSafeInteger(options.limit) || options.limit < 1)) {
      throw new ProtocolError("Invalid blob read budget or offset.");
    }
    const credential = this.#credential;
    if (!credential || !this.#pinned) {
      throw new OfflineError();
    }
    const singlePage = options.limit !== undefined;
    const pageLimit = Math.min(singlePage ? (options.limit as number) : 256 * 1024, 256 * 1024, max);
    const chunks: Uint8Array[] = [];
    let written = 0;
    let next = offset;
    let total = 0;
    for (;;) {
      const request: PinnedHttpRequest = {
        method: "GET",
        path: `/v1/blobs/${id}?offset=${next}&limit=${pageLimit}`,
        headers: { authorization: `Bearer ${credential.token}` },
        maxBody: pageLimit + 4096,
      };
      let response;
      try {
        response = await transport(this.#pinned, request);
      } catch {
        throw new OfflineError("The blob transfer was interrupted.");
      }
      if (response.status === 401 || response.status === 403) {
        this.#denyAuth();
        throw new AuthDeniedError();
      }
      if (response.status >= 300 && response.status < 400) {
        throw new ProtocolError("The server attempted a redirect. Refusing.");
      }
      if (response.status !== 200) {
        throw new RefusedError(`Blob read failed (status ${response.status}).`);
      }
      const length = Number(response.headers["x-blob-length"]);
      const nextHeader = Number(response.headers["x-next-offset"]);
      const chunk = response.body;
      if (!Number.isSafeInteger(length) || length < next ||
          !Number.isSafeInteger(nextHeader) || nextHeader !== next + chunk.length ||
          nextHeader > length || chunk.length > pageLimit ||
          chunk.length === 0 && next < length ||
          written > 0 && length !== total) {
        throw new ProtocolError("Invalid blob paging headers.");
      }
      total = length;
      if (written + chunk.length > max) {
        throw new ProtocolError("Blob exceeded the read budget.");
      }
      chunks.push(chunk);
      written += chunk.length;
      next = nextHeader;
      if (singlePage || chunk.length === 0 || next >= total) {
        break;
      }
    }
    const bytes = new Uint8Array(written);
    let at = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, at);
      at += chunk.length;
    }
    return { bytes, length: total, nextOffset: next };
  }
}
