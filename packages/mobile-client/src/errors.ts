//! Error taxonomy. Messages never carry credentials, pairing codes, or tokens;
//! only opaque identifiers and protocol text.

export type ShinaErrorKind =
  | "pairing"
  | "transport"
  | "protocol"
  | "integrity"
  | "capability"
  | "offline"
  | "suspended"
  | "auth-denied"
  | "uncertain"
  | "timeout"
  | "refused"
  | "closed";

export class ShinaError extends Error {
  readonly kind: ShinaErrorKind;
  readonly code: number | undefined;
  constructor(kind: ShinaErrorKind, message: string, code?: number) {
    super(message);
    this.name = "ShinaError";
    this.kind = kind;
    this.code = code;
  }
}

export class PairingError extends ShinaError {
  constructor(message: string) {
    super("pairing", message);
    this.name = "PairingError";
  }
}

export class ProtocolError extends ShinaError {
  constructor(message: string, code?: number) {
    super("protocol", message, code);
    this.name = "ProtocolError";
  }
}

export class IntegrityError extends ShinaError {
  constructor(message: string) {
    super("integrity", message);
    this.name = "IntegrityError";
  }
}

export class CapabilityError extends ShinaError {
  readonly capability: string;
  constructor(capability: string, message: string) {
    super("capability", message);
    this.name = "CapabilityError";
    this.capability = capability;
  }
}

export class OfflineError extends ShinaError {
  constructor(message = "The server is not connected. Reconnect before sending.") {
    super("offline", message);
    this.name = "OfflineError";
  }
}

export class SuspendedError extends ShinaError {
  constructor(message = "The connection is suspended.") {
    super("suspended", message);
    this.name = "SuspendedError";
  }
}

export class AuthDeniedError extends ShinaError {
  constructor(
    message = "Authentication denied. This device may have been revoked. Re-enrollment requires pairing again.",
  ) {
    super("auth-denied", message);
    this.name = "AuthDeniedError";
  }
}

export class TimeoutError extends ShinaError {
  constructor(message = "The request timed out.") {
    super("timeout", message);
    this.name = "TimeoutError";
  }
}

/** A server-refused request (JSON-RPC error result), not a transport failure. */
export class RefusedError extends ShinaError {
  constructor(message: string, code?: number) {
    super("refused", message, code);
    this.name = "RefusedError";
  }
}

/**
 * The write was dispatched but its acknowledgement never arrived. The server
 * may or may not have recorded it; `commandId` is the durable identity to use
 * with `receipt()` or `resolveCommand()`. Never resubmits with a fresh id.
 */
export class UncertainCommandError extends ShinaError {
  readonly commandId: string;
  readonly method: string;
  constructor(commandId: string, method: string, message?: string) {
    super(
      "uncertain",
      message ??
        `Command acknowledgement was lost for ${method}. Resolve the durable receipt for commandId ${commandId} before assuming it did not run.`,
    );
    this.name = "UncertainCommandError";
    this.commandId = commandId;
    this.method = method;
  }
}
