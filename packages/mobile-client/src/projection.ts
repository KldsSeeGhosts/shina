//! Workspace projection and thread history cache, ported from
//! src/client_history.rs (itself ported from KSG client-runtime).
//!
//! Properties kept from the reference:
//! - Workspace patches require contiguous `seq` in the same epoch and apply
//!   atomically; a gap or a malformed batch fails without touching the cache.
//! - Snapshot and stream fragments reassemble under MAX_SNAPSHOT_BYTES (64 MiB).
//! - Thread rows merge by source item id + sequence so live rows and removals
//!   win over concurrently read older pages.
//! - Unknown stream event kinds advance sequence; malformed known rows fail.
//! - Installed histories are LRU-evicted when they exceed 64 MiB total.
//! - History is walked lazily: a snapshot page first, older pages only on
//!   explicit request. The full log is never pulled eagerly.

import {
  CAP_DEFERRED_WORK,
  CAP_HISTORY_ITEMS,
  HISTORY_FIELDS,
  HISTORY_LEASE_GONE_CODE,
  HISTORY_UPDATE_REQUIRED,
  HISTORY_UPDATE_REQUIRED_CODE,
  MAX_SNAPSHOT_BYTES,
  PAGE_BYTES,
  applyPatches,
  cloneJson,
  isObject,
  utf8Length,
  type JsonObject,
  type JsonValue,
} from "./protocol.ts";
import { ProtocolError } from "./errors.ts";

/** One history row on the wire; the source identity drives merge order. */
export interface HistoryRow {
  sourceThreadId: number;
  sourceItemId: string;
  field: string;
  ordinal: number;
  position: number;
  item: JsonValue;
}

function parseRow(value: unknown): HistoryRow {
  if (!isObject(value)) {
    throw new ProtocolError("Invalid history row.");
  }
  const sourceThreadId = value["sourceThreadId"];
  const sourceItemId = value["sourceItemId"];
  const field = value["field"];
  const ordinal = value["ordinal"];
  const position = value["position"];
  const item = value["item"];
  if (
    !Number.isSafeInteger(sourceThreadId) ||
    (sourceThreadId as number) < 0 ||
    typeof sourceItemId !== "string" ||
    typeof field !== "string" ||
    !Number.isSafeInteger(ordinal) ||
    (ordinal as number) < 0 ||
    !Number.isSafeInteger(position) ||
    (position as number) < 0 ||
    item === undefined
  ) {
    throw new ProtocolError("Invalid history row.");
  }
  return {
    sourceThreadId: sourceThreadId as number,
    sourceItemId,
    field,
    ordinal: ordinal as number,
    position: position as number,
    item: item as JsonValue,
  };
}

export interface ThreadViewState {
  sequence: number;
  epoch: string;
  revision: string;
  hasMore: boolean;
  loading: boolean;
  error: string | null;
  expanded: boolean;
  ready: boolean;
  cursor: string | null;
  loadRequested: boolean;
  throughEntryId: string | null;
  workRequests: string[];
  workErrors: Record<string, string>;
}

interface View {
  sequence: number;
  epoch: string;
  revision: string;
  rows: Map<string, [number, HistoryRow]>;
  removed: Map<string, number>;
  fields: JsonObject;
  fieldsSequence: number;
  cursor: string | null;
  hasMore: boolean;
  loading: boolean;
  error: string | null;
  expanded: boolean;
  loadRequested: boolean;
  throughEntryId: string | null;
  ready: boolean;
  workRequests: string[];
  workErrors: Map<string, string>;
}

function emptyView(): View {
  return {
    sequence: 0,
    epoch: "",
    revision: "",
    rows: new Map(),
    removed: new Map(),
    fields: {},
    fieldsSequence: 0,
    cursor: null,
    hasMore: false,
    loading: false,
    error: null,
    expanded: false,
    loadRequested: false,
    throughEntryId: null,
    ready: false,
    workRequests: [],
    workErrors: new Map(),
  };
}

interface Retry {
  attempts: number;
  restarts: number;
  blocked: boolean;
  error: string | null;
}

interface HistoryWalk {
  thread: number;
  subscribe: boolean;
  snapshot: boolean;
  cursor: string | null;
  throughEntryId: string | null;
  fragment: JsonValue;
  text: string;
  waiting: boolean;
  pending: JsonObject[];
  pendingBytes: number;
  workAnchor: string | null;
}

const HISTORY_REQUEST_ID = "history";
const NOTICE_LIMIT = 256;

export interface HistoryRequest {
  jsonrpc: "2.0";
  id: string;
  method: "thread.subscribe" | "thread.history";
  params: JsonObject;
}

/** Raised when the workspace stream cannot continue from cursors; the session
 * recovers by resubscribing without lastSeq/epoch. */
export class ProjectionGapError extends ProtocolError {
  constructor(message: string) {
    super(message);
    this.name = "ProjectionGapError";
  }
}

export class ProjectionCache {
  state: JsonValue = null;
  seq: number | null = null;
  epoch: JsonValue = null;
  maintenance: JsonValue = null;
  desiredThread: number | null = null;
  dirty = false;
  readonly notices: string[] = [];
  updateRequired = false;
  deferredWork = false;

  private snapshotPages = "";
  private streamPages = "";
  private histories = new Map<number, [string, JsonObject, number]>();
  private historyOrder: number[] = [];
  private views = new Map<number, View>();
  private subscribed: number | null = null;
  private inflight: HistoryWalk | null = null;
  private retries = new Map<number, Retry>();

  /** Reset transport-scoped cursors on (re)connect and validate the negotiated
   * capability set. Missing `history-items` marks the projection unusable
   * (capability skew) without wiping thread metadata. */
  reconnect(hello: unknown): void {
    this.inflight = null;
    this.subscribed = null;
    this.snapshotPages = "";
    this.streamPages = "";
    const capabilities =
      isObject(hello) && Array.isArray(hello["capabilities"])
        ? (hello["capabilities"] as JsonValue[])
        : [];
    const deferred = capabilities.includes(CAP_DEFERRED_WORK);
    if (this.deferredWork && !deferred) {
      // An older build must re-send inline history, not inherit unresolved anchors.
      this.views.clear();
    }
    this.deferredWork = deferred;
    if (!capabilities.includes(CAP_HISTORY_ITEMS)) {
      this.updateRequired = true;
      this.dirty = true;
      this.pushNotice(HISTORY_UPDATE_REQUIRED);
      throw new ProtocolError(HISTORY_UPDATE_REQUIRED);
    }
    this.updateRequired = false;
  }

  /** The socket went away: cursors survive for replay, the in-flight walk and
   * live subscription do not. */
  transportLost(): void {
    this.inflight = null;
    this.subscribed = null;
    this.snapshotPages = "";
    this.streamPages = "";
  }

  private pushNotice(text: string): void {
    this.notices.push(text);
    while (this.notices.length > NOTICE_LIMIT) {
      this.notices.shift();
    }
  }

  drainNotices(): string[] {
    return this.notices.splice(0, this.notices.length);
  }

  private retryFor(thread: number): Retry {
    let retry = this.retries.get(thread);
    if (!retry) {
      retry = { attempts: 0, restarts: 0, blocked: false, error: null };
      this.retries.set(thread, retry);
    }
    return retry;
  }

  private failed(thread: number, message: string, recoverable: boolean): void {
    const retry = this.retryFor(thread);
    retry.error = message;
    if (recoverable && retry.attempts < 1) {
      retry.attempts += 1;
      retry.restarts += 1;
      this.subscribed = null;
      // Do not replay from the incomplete or unavailable base on the retry.
      const view = this.views.get(thread);
      if (view) {
        view.epoch = "";
      }
      this.pushNotice(`Thread ${thread} history retry ${retry.attempts}/1. ${message}`);
    } else {
      retry.blocked = true;
      this.pushNotice(
        `Thread ${thread} history stopped after ${retry.attempts}/1 retries. ${message}`,
      );
    }
    this.dirty = true;
  }

  /** Test/inspection helper: the thread has a finished snapshot base. */
  complete(thread: number): boolean {
    const view = this.views.get(thread);
    return !!view && view.ready && view.cursor === null;
  }

  /** Ask for the next older page. No-ops unless a snapshot base exists with a
   * cursor and nothing is loading; the driver issues at most one page at a
   * time, so a full-history walk needs a call per page. */
  requestEarlier(thread: number, throughEntryId: string | null = null): void {
    const view = this.views.get(thread);
    if (!view || !view.hasMore || view.cursor === null || view.loading) {
      return;
    }
    view.loading = true;
    view.error = null;
    view.loadRequested = true;
    view.throughEntryId = throughEntryId;
    this.dirty = true;
  }

  /** Queue expansion of a deferred-work marker row. */
  requestWork(thread: number, anchor: string): void {
    if (!this.deferredWork) {
      return;
    }
    const view = this.views.get(thread);
    if (!view) {
      return;
    }
    const row = view.rows.get(anchor);
    if (
      !row ||
      !isObject(row[1].item) ||
      !isObject(row[1].item["deferredWork"]) ||
      view.workRequests.includes(anchor)
    ) {
      return;
    }
    view.workErrors.delete(anchor);
    view.workRequests.push(anchor);
    try {
      this.install(thread);
    } catch (error) {
      this.failed(thread, (error as Error).message, false);
    }
  }

  /** Feed one decoded JSON-RPC message (notification or the history response
   * with id `"history"`). Throws on cursor gaps and protocol violations; the
   * session decides whether to resubscribe. */
  receive(message: unknown): void {
    const params = isObject(message) ? message["params"] : undefined;
    const method = isObject(message) ? message["method"] : undefined;
    switch (method) {
      case "subscription.snapshotPage": {
        if (!isObject(params) || typeof params["data"] !== "string") {
          throw new ProtocolError("Invalid snapshot page.");
        }
        this.snapshotPages += params["data"];
        if (utf8Length(this.snapshotPages) > MAX_SNAPSHOT_BYTES) {
          throw new ProtocolError(
            "Workspace metadata exceeded the client memory budget.",
          );
        }
        if (params["nextCursor"] === null || params["nextCursor"] === undefined) {
          let snapshot: unknown;
          try {
            snapshot = JSON.parse(this.snapshotPages);
          } catch (error) {
            throw new ProtocolError((error as Error).message);
          } finally {
            this.snapshotPages = "";
          }
          this.receive({ method: "subscription.snapshot", params: snapshot });
        }
        return;
      }
      case "subscription.snapshot": {
        if (!isObject(params)) {
          return;
        }
        this.state = cloneJson(params["state"] ?? null);
        this.seq = typeof params["seq"] === "number" ? params["seq"] : null;
        this.epoch = cloneJson(params["epoch"] ?? null);
        this.maintenance = cloneJson(params["maintenance"] ?? null);
        this.dirty = true;
        return;
      }
      case "subscription.ready": {
        if (isObject(params) && params["maintenance"] !== undefined) {
          this.maintenance = cloneJson(params["maintenance"]);
        }
        return;
      }
      case "subscription.patch": {
        if (!isObject(params)) {
          throw new ProtocolError("Invalid projection patch.");
        }
        const seq = params["seq"];
        if (typeof seq !== "number" || !Number.isSafeInteger(seq)) {
          throw new ProtocolError("Invalid patch sequence.");
        }
        if (this.seq !== null && seq <= this.seq) {
          return;
        }
        if (
          this.seq === null ||
          this.seq + 1 !== seq ||
          !epochEqual(this.epoch ?? null, params["epoch"] ?? null)
        ) {
          throw new ProjectionGapError(
            "Projection replay gap. Reconnecting for a snapshot.",
          );
        }
        try {
          this.state = applyPatches(this.state, params["patch"]);
        } catch (error) {
          throw new ProtocolError((error as Error).message);
        }
        this.seq = seq;
        this.dirty = true;
        return;
      }
      case "thread.streamPage": {
        if (!isObject(params) || typeof params["data"] !== "string") {
          throw new ProtocolError("Invalid stream page.");
        }
        this.streamPages += params["data"];
        if (utf8Length(this.streamPages) > MAX_SNAPSHOT_BYTES) {
          throw new ProtocolError(
            "Thread stream exceeded the client memory budget.",
          );
        }
        if (params["nextCursor"] === null || params["nextCursor"] === undefined) {
          let item: unknown;
          try {
            item = JSON.parse(this.streamPages);
          } catch (error) {
            throw new ProtocolError((error as Error).message);
          } finally {
            this.streamPages = "";
          }
          this.receive({ method: "thread.stream", params: item });
        }
        return;
      }
      case "thread.stream": {
        if (!isObject(params)) {
          return;
        }
        const walk = this.inflight;
        if (
          walk &&
          (walk.subscribe || walk.snapshot) &&
          params["threadId"] === walk.thread
        ) {
          walk.pendingBytes += utf8Length(JSON.stringify(params));
          if (walk.pendingBytes > MAX_SNAPSHOT_BYTES) {
            throw new ProtocolError(
              "Pending thread stream exceeded the client memory budget.",
            );
          }
          walk.pending.push(cloneJson(params));
        } else {
          this.stream(params);
        }
        return;
      }
      default:
        break;
    }
    if (isObject(message) && message["id"] === HISTORY_REQUEST_ID) {
      this.receiveHistory(message);
    }
  }

  private receiveHistory(value: JsonObject): void {
    const walk = this.inflight;
    this.inflight = null;
    if (!walk) {
      return;
    }
    if (
      (this.desiredThread !== null && this.desiredThread !== walk.thread) ||
      (Array.isArray((this.state as JsonObject | null)?.["threads"]) &&
        !((this.state as JsonObject)["threads"] as JsonValue[]).some(
          (thread) => isObject(thread) && thread["id"] === walk.thread,
        ))
    ) {
      return;
    }
    if (value["error"] !== null && value["error"] !== undefined) {
      const error = isObject(value["error"]) ? value["error"] : {};
      const code = typeof error["code"] === "number" ? error["code"] : null;
      const message =
        typeof error["message"] === "string" ? error["message"] : "History read failed.";
      if (walk.workAnchor !== null) {
        const view = this.views.get(walk.thread);
        if (view) {
          view.workRequests = view.workRequests.filter(
            (request) => request !== walk.workAnchor,
          );
          view.workErrors.set(walk.workAnchor, message);
        }
        this.install(walk.thread);
        return;
      }
      if (code === HISTORY_UPDATE_REQUIRED_CODE) {
        this.updateRequired = true;
      }
      if (!walk.subscribe && !walk.snapshot && code !== HISTORY_LEASE_GONE_CODE) {
        const view = this.views.get(walk.thread);
        if (view) {
          view.loading = false;
          view.loadRequested = false;
          view.throughEntryId = null;
          view.error = message;
        }
        this.dirty = true;
      } else {
        this.failed(walk.thread, message, code === HISTORY_LEASE_GONE_CODE);
      }
      return;
    }
    const result = value["result"];
    if (!isObject(result)) {
      this.inflight = walk;
      throw new ProtocolError("Invalid history response.");
    }
    if (result["kind"] === "synchronized") {
      this.subscribed = walk.thread;
      for (const item of walk.pending) {
        this.stream(item);
      }
      return;
    }
    if (walk.subscribe) {
      walk.subscribe = false;
      walk.snapshot = true;
      // The stream epoch stays fixed through this fragmented snapshot.
      walk.cursor =
        typeof result["streamEpoch"] === "string" ? result["streamEpoch"] : null;
    }
    if (typeof result["data"] !== "string") {
      this.inflight = walk;
      throw new ProtocolError("Invalid history fragment.");
    }
    walk.text += result["data"];
    if (utf8Length(walk.text) > MAX_SNAPSHOT_BYTES) {
      this.inflight = walk;
      throw new ProtocolError("Thread history exceeded the client memory budget.");
    }
    walk.fragment = result["nextFragment"] ?? null;
    if (walk.fragment === null) {
      let page: unknown;
      try {
        page = JSON.parse(walk.text);
      } catch (error) {
        throw new ProtocolError((error as Error).message);
      }
      if (!isObject(page)) {
        throw new ProtocolError("Invalid history page.");
      }
      if (walk.workAnchor !== null) {
        const view = this.views.get(walk.thread);
        const row = view?.rows.get(walk.workAnchor);
        const item = row ? row[1].item : null;
        const deferred = isObject(item) ? item["deferredWork"] : undefined;
        const cursor = isObject(deferred) ? deferred["cursor"] : undefined;
        if (cursor !== walk.cursor) {
          if (view) {
            view.workRequests = view.workRequests.filter(
              (request) => request !== walk.workAnchor,
            );
            view.workErrors.set(walk.workAnchor, "Work history changed. Retry loading it.");
          }
          this.install(walk.thread);
          return;
        }
        if (page["nextCursor"] === walk.cursor) {
          if (view) {
            view.workRequests = view.workRequests.filter(
              (request) => request !== walk.workAnchor,
            );
            view.workErrors.set(walk.workAnchor, "Work history did not advance.");
          }
          this.install(walk.thread);
          return;
        }
      }
      this.mergePage(walk.thread, page, walk.snapshot, walk.cursor ?? "", walk.workAnchor);
      if (walk.snapshot) {
        this.subscribed = walk.thread;
        for (const item of walk.pending) {
          this.stream(item);
        }
        if (this.subscribed === walk.thread) {
          const view = this.views.get(walk.thread);
          const retry = this.retries.get(walk.thread);
          if (view?.ready && view.cursor === null && retry) {
            retry.attempts = 0;
            retry.error = null;
          }
        }
        const view = this.views.get(walk.thread);
        if (view) {
          view.removed.clear();
        }
      } else {
        walk.waiting = false;
        this.inflight = walk;
      }
    } else {
      walk.waiting = false;
      this.inflight = walk;
    }
  }

  /** The next request for the history driver, or null when there is nothing to
   * do. One in-flight walk at a time; callers loop until this returns null. */
  nextHistory(): HistoryRequest | null {
    if (this.updateRequired) {
      return null;
    }
    if (this.inflight === null) {
      const threads = (this.state as JsonObject | null)?.["threads"];
      if (!Array.isArray(threads)) {
        return null;
      }
      const desired =
        this.desiredThread ??
        (isObject((this.state as JsonObject | null)?.["index"])
          ? ((this.state as JsonObject)["index"] as JsonObject)["active"]
          : undefined);
      const entry = threads.find(
        (thread) =>
          isObject(thread) &&
          (desired === undefined || desired === null || thread["id"] === desired),
      );
      const thread = isObject(entry) && typeof entry["id"] === "number" ? entry["id"] : null;
      if (thread === null) {
        return null;
      }
      const retry = this.retries.get(thread);
      if (retry?.blocked) {
        return null;
      }
      const subscribe = this.subscribed !== thread;
      const view = this.views.get(thread);
      if (view) {
        view.workRequests = view.workRequests.filter((anchor) => {
          const row = view.rows.get(anchor);
          return (
            !!row &&
            isObject(row[1].item) &&
            isObject(row[1].item["deferredWork"])
          );
        });
      }
      const workAnchor =
        !subscribe && view && view.workRequests.length > 0
          ? (view.workRequests[0] as string)
          : null;
      const cursor =
        workAnchor !== null
          ? (() => {
              const row = view?.rows.get(workAnchor);
              const item = row ? row[1].item : null;
              const deferred = isObject(item) ? item["deferredWork"] : undefined;
              const c = isObject(deferred) ? deferred["cursor"] : undefined;
              return typeof c === "string" ? c : null;
            })()
          : (view?.cursor ?? null);
      const wantEarlier = !!view && view.loadRequested && view.cursor !== null;
      if (!subscribe && !wantEarlier && workAnchor === null) {
        return null;
      }
      this.inflight = {
        thread,
        subscribe,
        snapshot: false,
        cursor,
        throughEntryId: view?.throughEntryId ?? null,
        fragment: null,
        text: "",
        waiting: false,
        pending: [],
        pendingBytes: 0,
        workAnchor,
      };
    }
    const walk = this.inflight;
    if (walk.waiting) {
      return null;
    }
    walk.waiting = true;
    const params: JsonObject = { threadId: walk.thread, byteBudget: PAGE_BYTES };
    let method: HistoryRequest["method"];
    if (walk.subscribe) {
      const view = this.views.get(walk.thread);
      if (view && view.ready && view.epoch !== "") {
        params["afterSequence"] = view.sequence;
        params["streamEpoch"] = view.epoch;
      }
      method = "thread.subscribe";
    } else {
      if (walk.fragment !== null) {
        params["fragmentCursor"] = walk.fragment;
      } else if (walk.cursor !== null) {
        params["historyCursor"] = walk.cursor;
        if (walk.workAnchor !== null) {
          params["workAnchorId"] = walk.workAnchor;
        }
        if (walk.throughEntryId !== null) {
          params["throughEntryId"] = walk.throughEntryId;
        }
      }
      method = "thread.history";
    }
    return { jsonrpc: "2.0", id: HISTORY_REQUEST_ID, method, params };
  }

  private mergePage(
    thread: number,
    page: JsonObject,
    snapshot: boolean,
    epoch: string,
    workAnchor: string | null,
  ): void {
    if (page["threadId"] !== thread) {
      throw new ProtocolError("Cross-thread history page.");
    }
    const sequence = page["snapshotSequence"];
    if (typeof sequence !== "number" || !Number.isSafeInteger(sequence)) {
      throw new ProtocolError("Invalid history sequence.");
    }
    if (snapshot) {
      const fresh = emptyView();
      fresh.epoch = epoch;
      fresh.sequence = sequence;
      this.views.set(thread, fresh);
    }
    const view = this.views.get(thread);
    if (!view) {
      throw new ProtocolError("Missing history base.");
    }
    const previousCursor = view.cursor;
    const previousThrough = view.throughEntryId;
    const items = page["items"];
    if (!Array.isArray(items)) {
      throw new ProtocolError("Invalid history items.");
    }
    let pageHasTurnStart = false;
    for (const raw of items) {
      const row = parseRow(raw);
      if (row.sourceThreadId !== thread) {
        throw new ProtocolError("Cross-thread history item.");
      }
      if (!(HISTORY_FIELDS as readonly string[]).includes(row.field)) {
        continue;
      }
      if (
        row.field === "messages" &&
        isObject(row.item) &&
        (row.item["role"] === "You" || row.item["role"] === "user")
      ) {
        pageHasTurnStart = true;
      }
      const id = row.sourceItemId;
      const existing = view.rows.get(id);
      const removedAt = view.removed.get(id);
      // Newer live records and removals win over concurrently read older pages.
      if ((existing !== undefined && existing[0] >= sequence) || (removedAt !== undefined && removedAt >= sequence)) {
        continue;
      }
      view.rows.set(row.sourceItemId, [sequence, row]);
    }
    if (sequence >= view.fieldsSequence) {
      const fields = page["fields"];
      if (!isObject(fields)) {
        throw new ProtocolError("Invalid history fields.");
      }
      view.fields = cloneJson(fields);
      view.fieldsSequence = sequence;
    }
    if (sequence >= view.sequence) {
      const revision = page["historyRevision"];
      if (typeof revision !== "string") {
        throw new ProtocolError("Invalid history revision.");
      }
      view.revision = revision;
    }
    if (workAnchor !== null) {
      const next = page["nextCursor"];
      const row = view.rows.get(workAnchor);
      if (row) {
        if (typeof next === "string") {
          const item = row[1].item;
          if (isObject(item) && isObject(item["deferredWork"])) {
            (item["deferredWork"] as JsonObject)["cursor"] = next;
          }
        } else if (isObject(row[1].item)) {
          delete (row[1].item as JsonObject)["deferredWork"];
        }
      }
      if (typeof next !== "string") {
        view.workRequests = view.workRequests.filter((request) => request !== workAnchor);
      }
      view.expanded = true;
      this.install(thread);
      return;
    }
    view.cursor = typeof page["nextCursor"] === "string" ? page["nextCursor"] : null;
    view.hasMore =
      typeof page["hasMoreHistory"] === "boolean"
        ? page["hasMoreHistory"]
        : view.cursor !== null;
    if (snapshot) {
      view.loading = false;
      view.error = null;
      view.expanded = false;
      view.loadRequested = false;
      view.throughEntryId = null;
    } else {
      view.expanded = true;
      view.error = null;
      const keepPaging = (() => {
        if (previousThrough !== null) {
          const targetLoaded = [...view.rows.values()].some(
            ([, row]) =>
              row.sourceItemId === previousThrough ||
              (isObject(row.item) && row.item["id"] === previousThrough),
          );
          return !targetLoaded && view.hasMore && view.cursor !== null;
        }
        return view.hasMore && view.cursor !== null && !pageHasTurnStart;
      })();
      if (keepPaging && (view.cursor === null || view.cursor === previousCursor)) {
        view.loading = false;
        view.loadRequested = false;
        view.throughEntryId = null;
        view.error = "History did not advance.";
      } else {
        view.loading = keepPaging;
        view.loadRequested = keepPaging;
        if (!keepPaging) {
          view.throughEntryId = null;
        }
      }
    }
    view.ready = true;
    // Older history reads do not advance the live replay cursor.
    this.install(thread);
  }

  /** Unknown event kinds advance sequence without touching known records. */
  private stream(item: JsonObject): void {
    const thread = item["threadId"];
    if (typeof thread !== "number" || !Number.isSafeInteger(thread)) {
      throw new ProtocolError("Invalid thread stream id.");
    }
    const kind = item["kind"];
    if (typeof kind !== "string") {
      throw new ProtocolError("Invalid thread stream kind.");
    }
    if (kind === "snapshot-required") {
      if (this.subscribed === thread) {
        this.failed(
          thread,
          "Thread replay is unavailable. Reloading a scoped snapshot.",
          true,
        );
      }
      return;
    }
    let view = this.views.get(thread);
    if (!view) {
      view = emptyView();
      this.views.set(thread, view);
    }
    if (item["streamEpoch"] !== view.epoch) {
      return;
    }
    const sequence = item["sequence"];
    if (typeof sequence !== "number" || !Number.isSafeInteger(sequence)) {
      throw new ProtocolError("Invalid thread stream sequence.");
    }
    if (sequence <= view.sequence) {
      return;
    }
    if (kind === "deleted") {
      this.views.delete(thread);
      this.histories.delete(thread);
      this.historyOrder = this.historyOrder.filter((id) => id !== thread);
      this.dirty = true;
      return;
    }
    if (kind === "event") {
      const events = item["events"];
      if (!Array.isArray(events)) {
        throw new ProtocolError("Invalid thread events.");
      }
      for (const event of events) {
        if (!isObject(event)) {
          throw new ProtocolError("Invalid thread event.");
        }
        switch (event["type"]) {
          case "item.updated": {
            const row = parseRow(event["row"]);
            if (row.sourceThreadId !== thread) {
              throw new ProtocolError("Cross-thread live item.");
            }
            if (!(HISTORY_FIELDS as readonly string[]).includes(row.field)) {
              continue;
            }
            const existing = view.rows.get(row.sourceItemId);
            const removedAt = view.removed.get(row.sourceItemId);
            if (
              (existing !== undefined && existing[0] > sequence) ||
              (removedAt !== undefined && removedAt > sequence)
            ) {
              continue;
            }
            if (
              existing !== undefined &&
              isObject(existing[1].item) &&
              isObject(existing[1].item["deferredWork"]) &&
              isObject(row.item) &&
              row.item["deferredWork"] === undefined
            ) {
              (row.item as JsonObject)["deferredWork"] = existing[1].item["deferredWork"] as JsonValue;
            }
            view.removed.delete(row.sourceItemId);
            view.rows.set(row.sourceItemId, [sequence, row]);
            break;
          }
          case "item.removed": {
            const id = event["sourceItemId"];
            if (typeof id !== "string") {
              throw new ProtocolError("Invalid removed item.");
            }
            const existing = view.rows.get(id);
            if (existing !== undefined && existing[0] > sequence) {
              continue;
            }
            view.rows.delete(id);
            view.removed.set(id, sequence);
            break;
          }
          case "fields.updated": {
            if (sequence >= view.fieldsSequence) {
              const fields = event["fields"];
              if (!isObject(fields)) {
                throw new ProtocolError("Invalid history fields.");
              }
              view.fields = cloneJson(fields);
              view.fieldsSequence = sequence;
            }
            break;
          }
          default:
            // Unknown event kinds still advance the stream sequence.
            break;
        }
        if (typeof item["historyRevision"] === "string") {
          view.revision = item["historyRevision"];
        }
      }
    }
    view.sequence = sequence;
    if (this.inflight === null) {
      view.removed.clear();
    }
    this.install(thread);
  }

  /** Materialize one thread's transcript from its row store, bounded. */
  private install(thread: number): void {
    const view = this.views.get(thread);
    if (!view || !view.ready) {
      return;
    }
    const history: JsonObject = cloneJson(view.fields);
    history["id"] = thread;
    const rows = [...view.rows.values()].map(([, row]) => row);
    rows.sort(
      (a, b) =>
        a.field.localeCompare(b.field) ||
        a.ordinal - b.ordinal ||
        a.sourceItemId.localeCompare(b.sourceItemId),
    );
    const ordinalToLocal = new Map<number, number>();
    let minMsgOrdinal: number | null = null;
    let maxMsgOrdinal: number | null = null;
    let localMsgCount = 0;
    for (const row of rows) {
      if (row.field === "messages") {
        ordinalToLocal.set(row.ordinal, localMsgCount);
        minMsgOrdinal = minMsgOrdinal === null ? row.ordinal : Math.min(minMsgOrdinal, row.ordinal);
        maxMsgOrdinal = maxMsgOrdinal === null ? row.ordinal : Math.max(maxMsgOrdinal, row.ordinal);
        localMsgCount += 1;
      }
    }
    const remapMsg = (serverOrd: number): number | null => {
      const local = ordinalToLocal.get(serverOrd);
      if (local !== undefined) {
        return local;
      }
      if (maxMsgOrdinal !== null && serverOrd > maxMsgOrdinal) {
        return localMsgCount;
      }
      if (minMsgOrdinal !== null) {
        return null;
      }
      return serverOrd;
    };
    const deferred: JsonObject[] = [];
    for (const row of rows) {
      const marker = isObject(row.item) ? row.item["deferredWork"] : undefined;
      if (!isObject(marker)) {
        continue;
      }
      const start = marker["start"];
      if (typeof start !== "number") {
        continue;
      }
      const remapped = remapMsg(start);
      if (remapped === null) {
        continue;
      }
      const copy: JsonObject = cloneJson(marker);
      copy["sourceItemId"] = row.sourceItemId;
      copy["start"] = remapped;
      copy["loading"] = view.workRequests.includes(row.sourceItemId);
      const error = view.workErrors.get(row.sourceItemId);
      copy["error"] = error ?? null;
      deferred.push(copy);
    }
    if (this.deferredWork) {
      history["deferredWork"] = deferred;
    }
    for (const field of HISTORY_FIELDS) {
      if (history[field] === undefined) {
        history[field] = [];
      }
    }
    for (const row of rows) {
      if (!(HISTORY_FIELDS as readonly string[]).includes(row.field)) {
        continue;
      }
      if (!Array.isArray(history[row.field])) {
        history[row.field] = [];
      }
      const item = cloneJson(row.item);
      if (
        row.field === "tools" ||
        row.field === "thoughts" ||
        row.field === "compactions"
      ) {
        const anchor = isObject(item) ? item["anchor"] : undefined;
        const serverOrd = isObject(anchor) ? anchor["message"] : undefined;
        if (typeof serverOrd === "number") {
          const local = remapMsg(serverOrd);
          if (local === null) {
            continue;
          }
          (anchor as JsonObject)["message"] = local;
        }
      } else if (row.field === "work") {
        if (isObject(item) && typeof item["start"] === "number") {
          const local = remapMsg(item["start"]);
          if (local === null) {
            continue;
          }
          item["start"] = local;
        }
      }
      (history[row.field] as JsonValue[]).push(item);
    }
    const harnessState = history["harness_state"];
    const workRetries = isObject(harnessState) ? harnessState["workRetries"] : undefined;
    if (Array.isArray(workRetries)) {
      const kept = workRetries.filter((retry) => {
        if (!isObject(retry)) {
          return true;
        }
        const anchor = retry["anchor"];
        const serverOrd = isObject(anchor) ? anchor["message"] : undefined;
        if (typeof serverOrd === "number") {
          const local = remapMsg(serverOrd);
          if (local === null) {
            return false;
          }
          (anchor as JsonObject)["message"] = local;
        }
        return true;
      });
      (harnessState as JsonObject)["workRetries"] = kept;
    }
    const bytes = utf8Length(JSON.stringify(history));
    if (bytes > MAX_SNAPSHOT_BYTES) {
      throw new ProtocolError("Thread history exceeded the client memory budget.");
    }
    this.histories.set(thread, [view.revision, history, bytes]);
    this.historyOrder = this.historyOrder.filter((id) => id !== thread);
    this.historyOrder.push(thread);
    let total = 0;
    for (const [, , size] of this.histories.values()) {
      total += size;
    }
    while (total > MAX_SNAPSHOT_BYTES && this.historyOrder.length > 0) {
      const oldest = this.historyOrder.shift() as number;
      total -= this.histories.get(oldest)?.[2] ?? 0;
      this.histories.delete(oldest);
      this.views.delete(oldest);
    }
    this.dirty = true;
  }

  /** Installed transcript for a thread, or null while unready/evicted. */
  history(thread: number): JsonObject | null {
    const entry = this.histories.get(thread);
    return entry ? cloneJson(entry[1]) : null;
  }

  /** Live view state for a thread (cursors, loading, errors), or null. */
  threadView(thread: number): ThreadViewState | null {
    const view = this.views.get(thread);
    if (!view) {
      return null;
    }
    return {
      sequence: view.sequence,
      epoch: view.epoch,
      revision: view.revision,
      hasMore: view.hasMore,
      loading: view.loading,
      error: view.error,
      expanded: view.expanded,
      ready: view.ready,
      cursor: view.cursor,
      loadRequested: view.loadRequested,
      throughEntryId: view.throughEntryId,
      workRequests: [...view.workRequests],
      workErrors: Object.fromEntries(view.workErrors),
    };
  }

  retryState(thread: number): { count: number; limit: number; fullRestarts: number; stopped: boolean; error: string | null } | null {
    const retry = this.retries.get(thread);
    if (!retry) {
      return null;
    }
    return {
      count: retry.attempts,
      limit: 1,
      fullRestarts: retry.restarts,
      stopped: retry.blocked,
      error: retry.error,
    };
  }

  /** Workspace state annotated with per-thread history projections and meta.
   * Returns null until the first snapshot lands, or when nothing changed. */
  projected(force = false): JsonObject | null {
    if ((!this.dirty && !force) || this.seq === null) {
      return null;
    }
    this.dirty = false;
    const threads = (this.state as JsonObject | null)?.["threads"];
    if (Array.isArray(threads)) {
      const present = new Set(
        threads
          .filter(isObject)
          .map((thread) => thread["id"])
          .filter((id): id is number => typeof id === "number"),
      );
      for (const id of [...this.histories.keys()]) {
        if (!present.has(id)) {
          this.histories.delete(id);
        }
      }
      for (const id of [...this.views.keys()]) {
        if (!present.has(id)) {
          this.views.delete(id);
        }
      }
      for (const id of [...this.retries.keys()]) {
        if (!present.has(id)) {
          this.retries.delete(id);
        }
      }
      this.historyOrder = this.historyOrder.filter((id) => this.histories.has(id));
    }
    const state = cloneJson(this.state);
    if (!isObject(state)) {
      return null;
    }
    state["historyUpdateRequired"] = this.updateRequired;
    const projectedThreads = state["threads"];
    if (Array.isArray(projectedThreads)) {
      for (const thread of projectedThreads) {
        if (!isObject(thread) || typeof thread["id"] !== "number") {
          continue;
        }
        const id = thread["id"];
        const entry = this.histories.get(id);
        if (entry) {
          for (const field of HISTORY_FIELDS) {
            thread[field] = entry[1][field] ?? [];
          }
        }
        const view = this.views.get(id);
        if (view) {
          thread["historyMeta"] = {
            hasMoreHistory: view.hasMore,
            loading: view.loading,
            error: view.error,
            expanded: view.expanded,
            deferredWork: entry ? (entry[1]["deferredWork"] ?? null) : null,
            workLoading: this.deferredWork ? "deferred" : "inline-legacy",
          };
        }
        const retry = this.retries.get(id);
        if (retry) {
          thread["historyRetry"] = {
            count: retry.attempts,
            limit: 1,
            fullRestarts: retry.restarts,
            stopped: retry.blocked,
            error: retry.error,
          };
        }
      }
    }
    return state;
  }
}

function epochEqual(a: JsonValue, b: unknown): boolean {
  if (a === b) {
    return true;
  }
  if (a === null || b === null || b === undefined) {
    return false;
  }
  if (Array.isArray(a) && Array.isArray(b)) {
    return (
      a.length === b.length && a.every((value, index) => epochEqual(value, b[index]))
    );
  }
  if (isObject(a) && isObject(b)) {
    const keys = Object.keys(a);
    return (
      keys.length === Object.keys(b).length &&
      keys.every(
        (key) =>
          Object.hasOwn(b, key) && epochEqual(a[key] as JsonValue, b[key]),
      )
    );
  }
  return false;
}
