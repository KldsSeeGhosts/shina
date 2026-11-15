//! Platform adapter contracts. Every credential-bearing transport is injected;
//! there are no default resolver, fetch, WebSocket, or storage fallbacks, so the
//! core fails closed when an adapter is missing instead of silently using a
//! browser or unsafe platform primitive.

import { PairingError } from "./errors.ts";
import { type JsonValue, isObject } from "./protocol.ts";

/** A validated tailnet address the transport must dial literally. */
export interface PinnedAddress {
  /** Numeric IPv4/IPv6 literal (already verified as a tailnet address). */
  ip: string;
  port: number;
}

/**
 * Resolve a MagicDNS name to numeric IP literals. The adapter MUST resolve via
 * the tailnet (the OS resolver on a Tailscale-configured device) and MUST
 * return literals, never forward to a proxy. The core validates that every
 * answer is a Tailscale CGNAT/ULA address before use; an adapter that cannot
 * guarantee tailnet-only answers must reject instead. Never resolve through
 * DoH/public DNS.
 */
export type TailnetResolver = (host: string) => Promise<readonly string[]>;

export interface PinnedRequest {
  method: "GET" | "POST" | "PUT" | "DELETE";
  /** Absolute path on the server, e.g. "/v1/identity". */
  path: string;
  /** Extra headers. May carry `authorization` for authenticated calls. */
  headers?: Record<string, string>;
  /** UTF-8 JSON body; sets content-type application/json when present. */
  json?: JsonValue;
  /** Raw body for upload chunks. */
  body?: Uint8Array;
  timeoutMs?: number;
  /** Response byte budget. The native adapter must enforce it while reading. */
  maxBody?: number;
}

export interface PinnedResponse {
  status: number;
  headers: Record<string, string>;
  body: Uint8Array;
}

/**
 * HTTP transport pinned to a literal tailnet address. Requirements:
 * - Dial `address.ip:address.port` directly; do not re-resolve, proxy, or use
 *   the link's original hostname.
 * - Never follow redirects. Surface 3xx as a response; the core refuses them.
 * - Never send an `Origin` header (the server refuses any request that has one).
 * - Bound response bodies to `maxBody` on the request or the documented limit.
 */
export type PinnedTransport = (
  address: PinnedAddress,
  request: PinnedRequest,
) => Promise<PinnedResponse>;

export interface PinnedHttpRequest extends PinnedRequest {
  /** Response byte budget; adapters must fail rather than exceed it. */
  maxBody?: number;
}

/** Minimal client socket, matching the React Native WebSocket surface. */
export interface MobileSocket {
  /** Only JSON text frames are used; implementations must reject oversized sends. */
  send(text: string): void;
  close(code?: number, reason?: string): void;
  readonly readyState: number;
  onopen: (() => void) | null;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose:
    | ((event: {
        code?: number;
        reason?: string;
        /** HTTP status when the upgrade was refused, if the platform reports it. */
        httpStatus?: number | undefined;
      }) => void)
    | null;
  onerror: ((event: unknown) => void) | null;
}

export interface SocketRequest {
  /** ws:// URL with a numeric tailnet literal host, e.g. ws://100.64.1.2:47329/v1/ws */
  url: string;
  /** Headers the adapter MUST send in the upgrade, including `authorization`.
   * A browser WebSocket cannot set these and is not a valid adapter. */
  headers: Record<string, string>;
  /** The pinned address the URL host came from, for adapters that pin sockets. */
  pinned: PinnedAddress;
}

/**
 * Open a WebSocket to the pinned tailnet address with the given upgrade
 * headers. The adapter MUST NOT inject an Origin header and MUST answer
 * protocol-level pings (React Native's WebSocket does both natively). It may
 * surface the HTTP status of a refused upgrade through `onclose`'s
 * `httpStatus` so the core can mark the credential terminally denied.
 */
export type SocketFactory = (request: SocketRequest) => MobileSocket | Promise<MobileSocket>;

/**
 * Hardware-backed or OS keychain storage for the device credential. Required;
//! the core never writes credentials to disk or memory-mapped storage itself.
 */
export interface SecureStore {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  delete(key: string): Promise<void>;
}

/** Lowercase canonical-hyphenated UUID v4. `crypto.randomUUID` qualifies; a
 * custom source must still produce canonical text or pairing fails closed. */
export type UuidSource = () => string;

/** Milliseconds since the Unix epoch. */
export type Clock = () => number;

/** Stored credential shape the session pins and authenticates with. Structurally
 * the same as `DeviceCredential` in pairing.ts; kept separate so the session
 * module does not depend on the pairing path. */
export interface DeviceCredentialLike {
  serverId: string;
  deviceId: string;
  token: string;
  host: string;
  port: number;
}

export interface PairingAdapters {
  /** Required when the link host is a MagicDNS name; unused for IP literals. */
  resolve?: TailnetResolver;
  /** Required. No default exists. */
  transport: PinnedTransport;
  /** Required. The issued credential is unusable without it. */
  store: SecureStore;
  /** Required. */
  uuid: UuidSource;
  now?: Clock;
}

export interface SessionAdapters {
  /** Required. */
  socket: SocketFactory;
  /** Required when the stored host is a MagicDNS name. */
  resolve?: TailnetResolver;
  /** Optional; enables authenticated HTTP APIs (blobs, uploads) and auth
   * classification when a socket cannot connect. */
  transport?: PinnedTransport;
  /** Required. */
  uuid: UuidSource;
  now?: Clock;
  timeouts?: {
    helloMs?: number;
    readMs?: number;
    commandMs?: number;
  };
}

export function requireAdapter<T>(value: T | undefined, what: string): T {
  if (value === undefined || value === null) {
    throw new PairingError(
      `Missing required adapter: ${what}. The app must supply it; the client core has no safe default.`,
    );
  }
  return value;
}

const UUID_CANONICAL =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

export function isCanonicalUuid(text: string): boolean {
  return UUID_CANONICAL.test(text);
}

/** Bounded bearer-token syntax, without control characters or header injection. */
export function isCredentialToken(value: unknown): value is string {
  return typeof value === "string" && value.length <= 512 &&
    /^[A-Za-z0-9._~+/-]+={0,2}$/.test(value);
}

/** Parse a UUID in any accepted text form, returning its canonical lowercase
 * hyphenated string or null. Mirrors the `raw == id.to_string()` check: the
 * input must already be canonical to parse to a non-null result. */
export function parseCanonicalUuid(text: string): string | null {
  return isCanonicalUuid(text) ? text : null;
}

export function requireUuid(source: UuidSource | undefined): UuidSource {
  return requireAdapter(source, "uuid source");
}

export function checkedUuid(source: UuidSource, what: string): string {
  const id = source();
  if (!isCanonicalUuid(id)) {
    throw new PairingError(`The ${what} adapter produced a non-canonical UUID.`);
  }
  return id;
}

/**
 * Tailscale address rule, ported from peer::tailnet_address: CGNAT
 * 100.64.0.0/10 for IPv4 and the fd7a:115c:a1e0::/48 ULA prefix for IPv6.
 */
export function isTailnetIp(ip: string): boolean {
  const v4 = parseIpv4(ip);
  if (v4) {
    return v4[0] === 100 && v4[1] >= 64 && v4[1] <= 127;
  }
  const v6 = parseIpv6(ip);
  if (v6) {
    return v6[0] === 0xfd7a && v6[1] === 0x115c && v6[2] === 0xa1e0;
  }
  return false;
}

/** Strict dotted-quad IPv4. No leading zeros; each octet 0..255. Returns the
 * four octets or null. */
export function parseIpv4(text: string): [number, number, number, number] | null {
  const parts = text.split(".");
  if (parts.length !== 4) {
    return null;
  }
  const octets: number[] = [];
  for (const part of parts) {
    if (!/^(0|[1-9][0-9]{0,2})$/.test(part)) {
      return null;
    }
    const value = Number(part);
    if (value > 255) {
      return null;
    }
    octets.push(value);
  }
  return octets as [number, number, number, number];
}

/** IPv6 parser: hextets with at most one `::` compression and an optional
 * embedded dotted-quad tail. Returns 8 segments or null. */
export function parseIpv6(text: string): number[] | null {
  let body = text;
  let tailV4: [number, number, number, number] | null = null;
  const lastColon = body.lastIndexOf(":");
  const lastPart = body.slice(lastColon + 1);
  if (lastPart.includes(".")) {
    tailV4 = parseIpv4(lastPart);
    if (!tailV4 || lastColon < 0) {
      return null;
    }
    body = body.slice(0, lastColon + 1) + "0:0";
    // Restore the v4 as two segments after parsing structure.
  }
  const halves = body.split("::");
  if (halves.length > 2) {
    return null;
  }
  const parseGroups = (group: string): number[] | null => {
    if (group === "") {
      return [];
    }
    const out: number[] = [];
    for (const part of group.split(":")) {
      if (!/^[0-9a-fA-F]{1,4}$/.test(part)) {
        return null;
      }
      out.push(parseInt(part, 16));
    }
    return out;
  };
  const head = parseGroups(halves[0] ?? "");
  if (head === null) {
    return null;
  }
  if (halves.length === 1) {
    const segments = head;
    if (tailV4) {
      segments.splice(segments.length - 2, 2, (tailV4[0] << 8) | tailV4[1], (tailV4[2] << 8) | tailV4[3]);
    }
    return segments.length === 8 ? segments : null;
  }
  const tail = parseGroups(halves[1] ?? "");
  if (tail === null) {
    return null;
  }
  let segments: number[];
  if (tailV4) {
    const filled = [...head, ...tail.slice(0, -2), (tailV4[0] << 8) | tailV4[1], (tailV4[2] << 8) | tailV4[3]];
    if (filled.length > 8) {
      return null;
    }
    segments = filled;
  } else {
    segments = [...head, ...tail];
  }
  const missing = 8 - segments.length;
  if (missing < 0) {
    return null;
  }
  return [...head, ...new Array(missing).fill(0), ...segments.slice(head.length)];
}

/** RFC 5952-style canonical form: lowercase, longest zero run compresses. */
export function formatIpv6(segments: readonly number[]): string {
  let bestStart = -1;
  let bestLen = 0;
  let start = -1;
  for (let index = 0; index <= 8; index += 1) {
    if (index < 8 && segments[index] === 0) {
      if (start < 0) {
        start = index;
      }
      if (index - start + 1 > bestLen) {
        bestStart = start;
        bestLen = index - start + 1;
      }
    } else {
      start = -1;
    }
  }
  if (bestLen < 2) {
    bestStart = -1;
  }
  const hextets = segments.map((segment) => segment.toString(16));
  if (bestStart < 0) {
    return hextets.join(":");
  }
  const left = hextets.slice(0, bestStart).join(":");
  const right = hextets.slice(bestStart + bestLen).join(":");
  if (left === "" && right === "") {
    return "::";
  }
  if (left === "") {
    return `::${right}`;
  }
  if (right === "") {
    return `${left}::`;
  }
  return `${left}::${right}`;
}

/** Parse an IP literal, returning its canonical string or null. */
export function parseIpLiteral(text: string): string | null {
  const v4 = parseIpv4(text);
  if (v4) {
    return v4.join(".");
  }
  const v6 = parseIpv6(text);
  if (v6) {
    return formatIpv6(v6);
  }
  return null;
}

/** `host` part of the link: tailnet literal (canonicalized) or MagicDNS name. */
export function parsePairHost(raw: string): string | null {
  const literal = parseIpLiteral(raw);
  if (literal !== null) {
    return isTailnetIp(literal) ? literal : null;
  }
  if (isDnsName(raw)) {
    return raw;
  }
  return null;
}

export function isDnsName(host: string): boolean {
  return (
    host.length <= 253 &&
    host.length > 0 &&
    host.split(".").every(
      (label) =>
        label.length > 0 &&
        label.length <= 63 &&
        /^[0-9a-zA-Z-]+$/.test(label) &&
        !label.startsWith("-") &&
        !label.endsWith("-"),
    )
  );
}

export function hostIsLiteral(host: string): boolean {
  return parseIpLiteral(host) !== null;
}

export function socketHostLiteral(ip: string): string {
  return ip.includes(":") ? `[${ip}]` : ip;
}

export function validateJsonBody(value: unknown, what: string): Record<string, JsonValue> {
  if (!isObject(value)) {
    throw new PairingError(`Invalid ${what} response.`);
  }
  return value;
}
