//! Pairing link parsing and redemption, ported from src/mobile_pairing.rs.
//!
//! The pairing code lives only in the URL fragment; it never reaches HTTP
//! paths, queries, logs, or serialized state. Redemption always verifies
//! `GET /v1/identity` before the code is sent to `POST /v1/pair`, over a
//! caller-supplied pinned transport.

import { PairingError, ProtocolError } from "./errors.ts";
import {
  type PairingAdapters,
  type PinnedAddress,
  type PinnedTransport,
  checkedUuid,
  hostIsLiteral,
  isTailnetIp,
  isCanonicalUuid,
  isCredentialToken,
  parseIpLiteral,
  parsePairHost,
  requireAdapter,
  requireUuid,
} from "./transport.ts";
import { type JsonObject, isObject, utf8Decode } from "./protocol.ts";

export const PAIR_LINK_PREFIX = "shina://pair/v1?";
export const PAIR_LINK_MAX_LEN = 512;
export const PAIR_CODE_LEN = 12;
const PAIR_FIELDS = ["host", "port", "serverId", "expiresAt"] as const;
const U64_MAX = (1n << 64n) - 1n;

export const CREDENTIAL_KEY_PREFIX = "shina.credential.";

export interface DeviceCredential {
  serverId: string;
  deviceId: string;
  token: string;
  host: string;
  port: number;
  pairedAt: number;
}

export function credentialKey(serverId: string): string {
  return `${CREDENTIAL_KEY_PREFIX}${serverId}`;
}

/** Parsed link. `code` is private and deliberately absent from toJSON. */
export class PairLink {
  readonly host: string;
  readonly port: number;
  readonly serverId: string;
  readonly expiresAt: bigint;
  readonly #code: string;

  private constructor(
    host: string,
    port: number,
    serverId: string,
    expiresAt: bigint,
    code: string,
  ) {
    this.host = host;
    this.port = port;
    this.serverId = serverId;
    this.expiresAt = expiresAt;
    this.#code = code;
  }

  /** Redemption is the only read path for the code. */
  codeForRedemption(): string {
    return this.#code;
  }

  expired(nowMs: number): boolean {
    return BigInt(Math.trunc(nowMs)) >= this.expiresAt;
  }

  toJSON(): JsonObject {
    return {
      host: this.host,
      port: this.port,
      serverId: this.serverId,
      expiresAt: this.expiresAt.toString(),
      code: "<redacted>",
    };
  }

  static parse(text: string): PairLink {
    const link = text.trim();
    const invalid = (): PairingError =>
      new PairingError("Not a Shina mobile pairing link.");
    if (link.length === 0 || link.length > PAIR_LINK_MAX_LEN) {
      throw invalid();
    }
    for (const ch of link) {
      const code = ch.codePointAt(0) as number;
      // Printable ASCII only; `%` is rejected outright so percent-encoded
      // ambiguities cannot slip a second meaning into any field.
      if (code < 0x21 || code >= 0x7f || code === 0x25) {
        throw invalid();
      }
    }
    if (!link.startsWith(PAIR_LINK_PREFIX)) {
      throw invalid();
    }
    const rest = link.slice(PAIR_LINK_PREFIX.length);
    const fragmentAt = rest.indexOf("#");
    if (fragmentAt < 0) {
      throw invalid();
    }
    const query = rest.slice(0, fragmentAt);
    const fragment = rest.slice(fragmentAt + 1);

    const slots = new Map<string, string>();
    for (const field of query.split("&")) {
      const eq = field.indexOf("=");
      if (eq < 0) {
        throw invalid();
      }
      const key = field.slice(0, eq);
      const value = field.slice(eq + 1);
      if (value === "") {
        throw invalid();
      }
      if (key === "code") {
        throw new PairingError("The pairing code belongs in the link fragment.");
      }
      if (slots.has(key) || !(PAIR_FIELDS as readonly string[]).includes(key)) {
        throw invalid();
      }
      slots.set(key, value);
    }
    if (!fragment.startsWith("code=")) {
      throw invalid();
    }
    slots.set("code", fragment.slice("code=".length));
    for (const key of [...PAIR_FIELDS, "code"] as const) {
      if (!slots.has(key)) {
        throw invalid();
      }
    }
    const fail = (field: string): PairingError =>
      new PairingError(`Invalid ${field} in the pairing link.`);

    const raw = (key: string): string => slots.get(key) as string;

    const host = parsePairHost(raw("host"));
    if (host === null) {
      throw fail("host");
    }

    const portRaw = raw("port");
    if (!/^[0-9]+$/.test(portRaw)) {
      throw fail("port");
    }
    const port = Number(portRaw);
    if (
      port === 0 ||
      port > 65535 ||
      String(port) !== portRaw ||
      !Number.isSafeInteger(port)
    ) {
      throw fail("port");
    }

    const serverId = raw("serverId");
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(serverId)) {
      throw fail("serverId");
    }

    const expiresRaw = raw("expiresAt");
    if (!/^[0-9]+$/.test(expiresRaw)) {
      throw fail("expiresAt");
    }
    const expiresAt = BigInt(expiresRaw);
    if (expiresAt === 0n || expiresAt > U64_MAX || expiresRaw !== expiresAt.toString()) {
      throw fail("expiresAt");
    }

    const code = raw("code");
    if (!/^[0-9A-F]{12}$/.test(code)) {
      throw fail("code");
    }

    return new PairLink(host, port, serverId, expiresAt, code);
  }

  /** Numeric tailnet address to dial. A MagicDNS name resolves through the
   * required adapter; every answer must be a tailnet address. */
  async resolve(resolve?: PairingAdapters["resolve"]): Promise<PinnedAddress> {
    if (hostIsLiteral(this.host)) {
      // Stored canonical literal; tailnet membership was checked at parse.
      return { ip: this.host, port: this.port };
    }
    const resolver = requireAdapter(resolve, "tailnet resolver");
    let answers: readonly string[];
    try {
      answers = await resolver(this.host);
    } catch {
      throw new PairingError("Could not resolve the pairing host on the tailnet.");
    }
    const literals = answers
      .map((answer) => typeof answer === "string" ? parseIpLiteral(answer.trim()) : null);
    if (literals.length === 0 || literals.some((literal) => literal === null || !isTailnetIp(literal))) {
      throw new PairingError("The pairing host did not resolve to a tailnet address.");
    }
    return { ip: literals[0] as string, port: this.port };
  }
}

function refused(message: string): PairingError {
  return new PairingError(message);
}

/** Decode a small JSON body; anything else is a protocol violation. */
function decodeJson(body: Uint8Array): unknown {
  try {
    return JSON.parse(utf8Decode(body));
  } catch {
    return null;
  }
}

async function pinnedGet(
  transport: PinnedTransport,
  address: PinnedAddress,
  path: string,
  headers: Record<string, string>,
): Promise<{ status: number; json: unknown }> {
  const response = await transport(address, { method: "GET", path, headers, maxBody: 16 * 1024 });
  if (response.body.length > 16 * 1024) {
    throw refused("The pairing identity response exceeded its byte budget.");
  }
  return { status: response.status, json: decodeJson(response.body) };
}

/**
 * Redeem the link into a persisted device credential.
 *
 * Order is security-relevant: resolve and validate the destination, read
 * `/v1/identity` and compare `serverId`, and only then POST the code with
 * `{deviceId, code, expectedServerId}`. The code never travels in a path or
 * query. Redirects are never followed; a 3xx is a pairing failure. The
 * returned identity and token are validated, then persisted through the
 * required secure store. Error messages deliberately omit the code and token.
 */
export async function redeemPairLink(
  link: PairLink,
  adapters: PairingAdapters,
): Promise<DeviceCredential> {
  const now = adapters.now ?? Date.now;
  if (link.expired(now())) {
    throw refused("Pairing link expired.");
  }
  const transport = requireAdapter(adapters.transport, "pinned transport");
  const store = requireAdapter(adapters.store, "secure credential store");
  const uuid = requireUuid(adapters.uuid);
  const address = await link.resolve(adapters.resolve);

  // Identity first: the code is sent only to the pinned server.
  let identity: { status: number; json: unknown };
  try {
    identity = await pinnedGet(transport, address, "/v1/identity", {
      accept: "application/json",
    });
  } catch {
    throw refused("The pairing host did not answer identity requests.");
  }
  if (identity.status >= 300 && identity.status < 400) {
    throw refused("The pairing host attempted a redirect. Refusing to continue.");
  }
  const identityServer =
    identity.status === 200 &&
    isObject(identity.json) &&
    typeof identity.json["serverId"] === "string"
      ? (identity.json["serverId"] as string)
      : null;
  if (identityServer !== link.serverId) {
    throw refused("The pairing host is not the expected Shina server.");
  }

  const deviceId = checkedUuid(uuid, "uuid");
  let pair: { status: number; json: unknown };
  try {
    const response = await transport(address, {
      method: "POST",
      path: "/v1/pair",
      headers: { accept: "application/json" },
      json: {
        deviceId,
        code: link.codeForRedemption(),
        expectedServerId: link.serverId,
      },
      maxBody: 16 * 1024,
    });
    if (response.body.length > 16 * 1024) {
      throw refused("The pairing response exceeded its byte budget.");
    }
    pair = { status: response.status, json: decodeJson(response.body) };
  } catch {
    throw refused("Pairing failed. Request a fresh link before trying again.");
  }
  if (pair.status >= 300 && pair.status < 400) {
    throw refused("The pairing host attempted a redirect. Refusing to continue.");
  }
  if (pair.status === 409) {
    throw refused("The server identity changed while pairing. Request a fresh link.");
  }
  if (pair.status === 403) {
    throw refused("Pairing refused. The code is unknown, expired, used, or rate-limited.");
  }
  if (pair.status !== 200 || !isObject(pair.json)) {
    throw refused(`Pairing failed (status ${pair.status}).`);
  }
  const json = pair.json;
  if (json["serverId"] !== link.serverId) {
    throw refused("The paired server identity did not match the link.");
  }
  const token = json["token"];
  if (!isCredentialToken(token)) {
    throw refused("The paired server did not issue a device token.");
  }
  if (json["peer"] === true) {
    throw refused("The paired credential is a peer credential, not a device credential.");
  }

  const credential: DeviceCredential = {
    serverId: link.serverId,
    deviceId,
    token,
    host: link.host,
    port: link.port,
    pairedAt: now(),
  };
  try {
    await store.set(credentialKey(link.serverId), JSON.stringify(credential));
  } catch {
    throw refused("Could not save the paired credential in secure storage.");
  }
  return credential;
}

export async function loadCredential(
  store: Pick<PairingAdapters["store"], "get">,
  serverId: string,
): Promise<DeviceCredential | null> {
  if (!isCanonicalUuid(serverId)) {
    throw new ProtocolError("Invalid paired server identity.");
  }
  let text: string | null;
  try {
    text = await store.get(credentialKey(serverId));
  } catch {
    throw new ProtocolError("Could not read the paired credential from secure storage.");
  }
  if (text === null) {
    return null;
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (
    !isObject(value) ||
    value["serverId"] !== serverId ||
    typeof value["deviceId"] !== "string" ||
    !isCanonicalUuid(value["deviceId"]) ||
    !isCanonicalUuid(serverId) ||
    !isCredentialToken(value["token"]) ||
    typeof value["host"] !== "string" ||
    parsePairHost(value["host"]) === null ||
    typeof value["port"] !== "number" ||
    !Number.isSafeInteger(value["port"]) ||
    value["port"] < 1 || value["port"] > 65535
  ) {
    throw new ProtocolError("The stored credential is malformed. Pair again.");
  }
  return {
    serverId: value["serverId"] as string,
    deviceId: value["deviceId"] as string,
    token: value["token"] as string,
    host: value["host"] as string,
    port: value["port"] as number,
    pairedAt: typeof value["pairedAt"] === "number" ? value["pairedAt"] : 0,
  };
}

export async function forgetCredential(
  store: Pick<PairingAdapters["store"], "delete">,
  serverId: string,
): Promise<void> {
  if (!isCanonicalUuid(serverId)) {
    throw new ProtocolError("Invalid paired server identity.");
  }
  try {
    await store.delete(credentialKey(serverId));
  } catch {
    throw new ProtocolError("Could not remove the paired credential from secure storage.");
  }
}
