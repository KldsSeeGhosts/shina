//! Durable Shina JSON-RPC contract constants, ported from src/protocol.rs.
//! The client negotiates by advertised capability names, never by minor equality:
//! a later minor only adds capability strings to the hello result.

export const PROTOCOL_MAJOR = 1;
export const PROTOCOL_MINOR = 28;
export const PROJECTION_SCHEMA = 1;

export const CAP_HISTORY_ITEMS = "history-items";
export const CAP_DEFERRED_WORK = "deferred-work";
export const CAP_NOTIFICATIONS = "notification-feed-v1";
export const CAP_FILE_ATTACHMENTS = "file-attachments-v1";
export const CAP_STAGED_ATTACHMENTS = "staged-attachments-v1";
export const CAP_ATTACHMENT_LIMITS = "attachment-limits-v1";
export const CAP_PASTED_TEXT_ATTACHMENTS = "pasted-text-attachments-v1";
export const CAP_VCS_WORKTREES = "vcs-worktrees-v1";
export const CAP_REVIEW_DIFF = "review-diff-v1";
export const CAP_WORKSPACE_SEARCH = "workspace-search-v1";
export const CAP_WORKSPACE_READ_FILE = "workspace-read-file-v1";
export const CAP_THREAD_TITLE_SEARCH = "thread-title-search-v1";
export const CAP_FILESYSTEM_BROWSE = "filesystem-browse";
export const CAP_RUN_CONTROL = "run-control";
export const CAP_COMPOSER_REQUESTS = "composer-requests-v1";
export const CAP_QUESTION_CONTRACT = "question-contract-v1";
export const CAP_QUESTION_ATTACHMENTS = "question-attachments-v1";
export const CAP_QUESTION_DISMISS = "question-dismiss-v1";
export const CAP_APPROVAL_FEEDBACK = "approval-feedback-v1";
export const CAP_PROVIDER_HANDOFF = "provider-handoff-v1";
export const CAP_DRAFT_ROUTING = "draft-routing-v1";
export const CAP_FRESH_CONTEXT = "fresh-context";
export const CAP_PROJECT_WRITE_FILE = "project-write-file";
export const CAP_USAGE_LIMITS = "usage-limits";
export const CAP_MODEL_OPTIONS = "model-options";
export const CAP_MODEL_FAVORITES = "model-favorites";
export const CAP_MODEL_CATALOG_REFRESH = "model-catalog-refresh";
export const CAP_THREAD_FIND = "thread-find";
export const CAP_THREAD_SEARCH = "thread-search";
export const CAP_COMPOSER_CONTEXT = "composer-context-v1";
export const CAP_COMPOSER_TRANSACTIONS = "composer-transactions";
export const CAP_PROJECTION_PATCHES = "projection-patches";
export const CAP_SNAPSHOT_SUBSCRIPTIONS = "snapshot-subscriptions";
/// Opt-in mobile push worker (protocol minor 28+). The lookup and register
/// methods check this exact advertised string; they are never gated on minor.
export const CAP_PUSH_NOTIFICATIONS = "mobile-push-v1";

export const MAX_CONTROL = 1024 * 1024;
export const PAGE_BYTES = 64 * 1024;
export const MAX_PAGE_BYTES = 256 * 1024;
export const MAX_SNAPSHOT_BYTES = 64 * 1024 * 1024;
export const DEFAULT_OUTSTANDING_COMMANDS = 32;

export const HISTORY_LEASE_GONE_CODE = -32010;
export const HISTORY_UPDATE_REQUIRED_CODE = -32012;
export const HISTORY_LEASE_GONE =
  "History fragment lease expired or evicted. Reload the scoped snapshot.";
export const HISTORY_UPDATE_REQUIRED =
  "Update required. Update Shina on the client and server to use scoped history and live items.";
export const NOTIFICATIONS_UPDATE_REQUIRED =
  "Update required. Update Shina on the server to read mobile notifications.";

export const HISTORY_FIELDS = [
  "messages",
  "tools",
  "thoughts",
  "work",
  "diagnostics",
  "compactions",
  "fork_points",
  "commands",
  "models",
  "queue",
  "queue_drafts",
  "steering",
  "follow_ups",
  "stats",
] as const;
export type HistoryField = (typeof HISTORY_FIELDS)[number];
const HISTORY_FIELD_SET: ReadonlySet<string> = new Set(HISTORY_FIELDS);
export function isHistoryField(field: string): field is HistoryField {
  return HISTORY_FIELD_SET.has(field);
}

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

export function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Decode UTF-8 bytes without a TextDecoder dependency (Hermes-safe). */
export function utf8Decode(bytes: Uint8Array): string {
  let out = "";
  let index = 0;
  while (index < bytes.length) {
    const first = bytes[index] as number;
    if (first < 0x80) {
      out += String.fromCharCode(first);
      index += 1;
    } else if (first < 0xe0) {
      out += String.fromCharCode(
        ((first & 0x1f) << 6) | ((bytes[index + 1] as number) & 0x3f),
      );
      index += 2;
    } else if (first < 0xf0) {
      out += String.fromCharCode(
        ((first & 0x0f) << 12) |
          (((bytes[index + 1] as number) & 0x3f) << 6) |
          ((bytes[index + 2] as number) & 0x3f),
      );
      index += 3;
    } else {
      const point =
        (((first & 0x07) << 18) |
          (((bytes[index + 1] as number) & 0x3f) << 12) |
          (((bytes[index + 2] as number) & 0x3f) << 6) |
          ((bytes[index + 3] as number) & 0x3f)) -
        0x10000;
      out += String.fromCharCode(
        0xd800 | (point >> 10),
        0xdc00 | (point & 0x3ff),
      );
      index += 4;
    }
  }
  return out;
}

/** UTF-8 byte length. Wire bounds count bytes, not UTF-16 code units. */
export function utf8Length(text: string): number {
  let bytes = 0;
  for (let index = 0; index < text.length; index += 1) {
    const code = text.charCodeAt(index);
    if (code < 0x80) {
      bytes += 1;
    } else if (code < 0x800) {
      bytes += 2;
    } else if (code >= 0xd800 && code <= 0xdbff && index + 1 < text.length) {
      const next = text.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

/** Methods the server reads from `params` directly. Everything else travels in
 * the `{commandId, args}` command envelope, which explicit handlers unwrap and
 * the runtime fallback turns into a durable receipt. */
const PLAIN_PARAMS_METHODS: ReadonlySet<string> = new Set([
  "hello",
  "workspace.subscribe",
  "thread.subscribe",
  "subscription.cancel",
  "thread.history",
  "thread.find",
  "notifications.list",
  "command.get",
  "server.status",
  "server.updateStatus",
]);

/** Read-grant methods (plain params). Mirrors protocol::method_grant "read". */
export const READ_METHODS = [
  "workspace.subscribe",
  "thread.subscribe",
  "subscription.cancel",
  "thread.history",
  "thread.find",
  "threads.search",
  "command.get",
  "notifications.list",
  "notifications.get",
  "notifications.push.status",
  "server.status",
  "filesystem.browse",
  "usage.limits",
  "workspace.searchEntries",
  "workspace.readFile",
  "vcs.listBranches",
  "vcs.status",
  "vcs.listWorktrees",
  "review.getBranchCommits",
  "review.getDiffPreview",
  "review.getDiffFileContents",
] as const;
export type ReadMethod = (typeof READ_METHODS)[number];
const READ_METHOD_SET: ReadonlySet<string> = new Set(READ_METHODS);

/** Non-read methods, dispatched through the durable command envelope. Mirrors
 * the operate/approve/maintain/peer-delegate grants. A mobile-paired device
 * holds read+operate+approve+update, so maintain and peer-delegate calls are
 * still refused by the server; they are typed here for completeness. */
export const COMMAND_METHODS = [
  "request.respond",
  "thread.user-input.dismiss",
  "notifications.push.register",
  "notifications.push.unregister",
  "server.requestUpdate",
  "devices.list",
  "peer.link",
  "peer.list",
  "peer.unlink",
  "devices.pair",
  "devices.revoke",
  "devices.grant",
  "devices.revokeGrant",
  "server.validateRestart",
  "server.prepareRestart",
  "server.cancelRestart",
  "server.clientReady",
  "server.commitRestart",
  "server.stop",
  "peer.capabilities",
  "peer.thread.launch",
  "peer.thread.wait",
  "peer.task.ack",
  "peer.thread.read",
  "peer.thread.list",
  "peer.thread.send",
  "peer.thread.stop",
  "thread.send",
  "thread.stop",
  "thread.resume",
  "thread.configure",
  "thread.retry",
  "thread.visit",
  "thread.create",
  "thread.rename",
  "thread.delete",
  "thread.lifecycle",
  "thread.undo",
  "thread.drop",
  "thread.compact",
  "thread.fork",
  "thread.rewind",
  "thread.forkPoints",
  "thread.worktreeSetup",
  "settings.autoSettle",
  "settings.defaultHarness",
  "settings.threadDefaults",
  "settings.modelFavorites",
  "background.keepAwake",
  "queue.remove",
  "queue.sendNow",
  "queue.edit",
  "queue.update",
  "queue.returnDraft",
  "input.claim",
  "project.put",
  "project.ensureScratch",
  "project.remove",
  "project.clone",
  "project.cancelClone",
  "catalog.refresh",
  "commands.refresh",
  "captain.open",
  "captain.put",
  "captain.scope",
  "captain.reset",
  "memory.put",
  "memory.remove",
  "profile.put",
  "routine.pause",
  "routine.remove",
  "task.resume",
  "task.dismiss",
  "task.resumeResults",
  "task.delegate",
  "task.send",
  "task.cancel",
  "project.writeFile",
  "uploads.write",
  "vcs.createWorktree",
  "vcs.removeWorktree",
  "vcs.runSetup",
] as const;
export type CommandMethod = (typeof COMMAND_METHODS)[number];
const COMMAND_METHOD_SET: ReadonlySet<string> = new Set(COMMAND_METHODS);

export function isReadMethod(method: string): method is ReadMethod {
  return READ_METHOD_SET.has(method);
}
export function isCommandMethod(method: string): method is CommandMethod {
  return COMMAND_METHOD_SET.has(method);
}
export function usesPlainParams(method: string): boolean {
  return PLAIN_PARAMS_METHODS.has(method);
}

/** One typed projection patch, `{"op":"set"|"remove","path":[...],"value"?}`. */
export type Patch =
  | { op: "set"; path: string[]; value: JsonValue }
  | { op: "remove"; path: string[] };

export const PATCH_PATH_MAX_DEPTH = 64;
export const PATCH_BATCH_MAX = 65536;
const FORBIDDEN_SEGMENTS: ReadonlySet<string> = new Set([
  "__proto__",
  "prototype",
  "constructor",
]);

function isIndex(segment: string): boolean {
  return /^[0-9]+$/.test(segment);
}

/**
 * Apply one patch batch atomically. The state is cloned first, so a malformed
 * or out-of-bounds patch leaves the prior projection untouched. Path segments
 * that could mutate prototypes are rejected outright. Ported from
 * protocol::apply with stronger atomicity and prototype-pollution fencing.
 */
export function applyPatches(state: JsonValue, patches: unknown): JsonValue {
  if (!Array.isArray(patches) || patches.length > PATCH_BATCH_MAX) {
    throw new Error("Invalid projection patch batch.");
  }
  let next: JsonValue = state === undefined ? null : (cloneJson(state) as JsonValue);
  for (const raw of patches) {
    if (!isObject(raw)) {
      throw new Error("Invalid projection patch.");
    }
    const op = raw["op"];
    const path = raw["path"];
    if (
      (op !== "set" && op !== "remove") ||
      !Array.isArray(path) ||
      path.length > PATCH_PATH_MAX_DEPTH ||
      !path.every(
        (segment): segment is string =>
          typeof segment === "string" && !FORBIDDEN_SEGMENTS.has(segment),
      )
    ) {
      throw new Error("Invalid projection patch.");
    }
    if (op === "set" && !("value" in raw)) {
      throw new Error("Invalid projection patch.");
    }
    next = applyOne(
      next,
      path,
      op === "set" ? (raw["value"] as JsonValue) : REMOVE,
      op === "set",
    );
  }
  return next;
}

const REMOVE = Symbol("remove");

function applyOne(
  root: JsonValue,
  path: string[],
  value: JsonValue | typeof REMOVE,
  isSet: boolean,
): JsonValue {
  if (path.length === 0) {
    if (!isSet) {
      throw new Error("Cannot remove projection root.");
    }
    return cloneJson(value) as JsonValue;
  }
  let target: JsonValue = root;
  for (const segment of path.slice(0, -1)) {
    if (isObject(target)) {
      const child = target[segment];
      if (child === undefined) {
        throw new Error("Projection patch has a missing parent.");
      }
      target = child;
    } else if (Array.isArray(target)) {
      if (!isIndex(segment)) {
        throw new Error("Projection patch has a missing parent.");
      }
      const child = target[Number(segment)];
      if (child === undefined) {
        throw new Error("Projection patch has a missing parent.");
      }
      target = child;
    } else {
      throw new Error("Projection patch has a missing parent.");
    }
  }
  const key = path[path.length - 1] as string;
  if (isObject(target)) {
    if (value === REMOVE) {
      delete target[key];
    } else {
      target[key] = value as JsonValue;
    }
    return root;
  }
  if (Array.isArray(target)) {
    if (!isIndex(key)) {
      throw new Error("Invalid patch array index.");
    }
    const index = Number(key);
    if (value === REMOVE) {
      // protocol::apply cannot remove array elements either.
      throw new Error("Invalid projection patch target.");
    }
    if (index >= target.length) {
      throw new Error("Patch array index out of bounds.");
    }
    target[index] = value as JsonValue;
    return root;
  }
  throw new Error("Invalid projection patch target.");
}

export function cloneJson<T>(value: T): T {
  if (value === null || typeof value !== "object") {
    return value;
  }
  return JSON.parse(JSON.stringify(value)) as T;
}

/** One durable command receipt, the result of every runtime-dispatched command. */
export interface Receipt {
  commandId: string;
  deviceId: string;
  hash: string;
  status: "accepted" | "rejected" | "interrupted";
  result: JsonValue;
  seq: number;
}

export function isReceipt(value: unknown): value is Receipt {
  return (
    isObject(value) &&
    typeof value["commandId"] === "string" &&
    typeof value["status"] === "string" &&
    ["accepted", "rejected", "interrupted"].includes(value["status"])
  );
}
