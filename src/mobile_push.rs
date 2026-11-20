//! Opt-in Expo push delivery for the mobile notification feed.
//!
//! Registration is explicit and per authenticated device. The Expo token lives
//! only in `mobile_push_registrations`: never in projections, receipts, staged
//! documents, logs, `Debug` output or error text. Every stored failure reason is
//! a fixed classification string, so provider and transport detail that could
//! echo a token back is dropped on the floor.
//!
//! `mobile_notifications::record` creates one outbox row per opted-in active
//! device inside the projection commit that records the event. Jobs therefore
//! exist only for committed edges; a registration added later never backfills
//! history. `Worker` claims committed jobs under an expiring lease, posts a
//! generic payload (`data` is exactly `{serverId, eventId}`, both UUIDs) and
//! treats Expo ticket acceptance as submission, not delivery. Delivery is
//! proven only by the receipt endpoint; `DeviceNotRegistered` deactivates the
//! token's registration generation and cancels its queued work.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub const SEND_URL: &str = "https://exp.host/--/api/v2/push/send";
pub const RECEIPT_URL: &str = "https://exp.host/--/api/v2/push/getReceipts";

const TOKEN_INNER_MIN: usize = 8;
const TOKEN_INNER_MAX: usize = 192;
const MAX_REGISTRATIONS: i64 = 256;
const PENDING_PER_DEVICE: i64 = 256;
const UNFINISHED_GLOBAL: i64 = 8192;
const MAX_ATTEMPTS: i64 = 8;
const BASE_BACKOFF_MS: i64 = 30_000;
const MAX_BACKOFF_MS: i64 = 900_000;
const MAX_JOB_AGE_MS: i64 = 86_400_000;
const RECEIPT_WINDOW_MS: i64 = 3_600_000;
const RECEIPT_POLL_MS: i64 = 60_000;
const LEASE_MS: i64 = 120_000;
const CLAIM_BATCH: i64 = 64;
const TICKET_BATCH: i64 = 100;
const SEND_BATCH: usize = 64;
const TERMINAL_KEEP: i64 = 256;
const MAX_RESPONSE: u64 = 64 * 1024;
const IDLE_TICK: Duration = Duration::from_secs(5);
const ACTIVE_TICK: Duration = Duration::from_millis(750);

/// Expo accepts `ExponentPushToken[...]` and `ExpoPushToken[...]` wrappers.
/// Anything else is not an Expo push token and cannot reach `SEND_URL`.
fn valid_token(token: &str) -> bool {
    let inner = ["ExponentPushToken[", "ExpoPushToken["]
        .iter()
        .find_map(|prefix| token.strip_prefix(prefix))
        .and_then(|rest| rest.strip_suffix(']'));
    match inner {
        Some(inner) => {
            (TOKEN_INNER_MIN..=TOKEN_INNER_MAX).contains(&inner.len())
                && inner
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        }
        None => false,
    }
}

/// A registration change staged by a runtime command. Applied inside
/// `Database::commit` so the accepted receipt and the registration are atomic.
/// The token inside `Register` never reaches a result, log or `Debug`.
pub enum Staged {
    Register {
        device: String,
        platform: String,
        token: String,
    },
    Unregister {
        device: String,
    },
}

/// Validate registration args and the caller's own device under the batch lock.
/// Returns the staged operation; `commit` performs the write.
pub(crate) fn registration(
    connection: &Connection,
    device: &str,
    args: &Value,
) -> Result<Staged, String> {
    if let Some(other) = args.get("deviceId").and_then(Value::as_str)
        && other != device
    {
        return Err("Push registration belongs to the calling device.".into());
    }
    let platform = args["platform"]
        .as_str()
        .filter(|platform| matches!(*platform, "ios" | "android"))
        .ok_or("Push platform must be ios or android.")?;
    let token = args["token"]
        .as_str()
        .ok_or("An Expo push token is required.")?;
    if !valid_token(token) {
        return Err("Invalid Expo push token.".into());
    }
    let live: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND revoked_at IS NULL)",
            [device],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !live {
        return Err("Unknown or revoked device.".into());
    }
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM mobile_push_registrations WHERE device_id=?1)",
            [device],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        let registered: i64 = connection
            .query_row(
                "SELECT count(*) FROM mobile_push_registrations WHERE active=1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if registered >= MAX_REGISTRATIONS {
            return Err("Push registration limit reached.".into());
        }
    }
    Ok(Staged::Register {
        device: device.into(),
        platform: platform.into(),
        token: token.into(),
    })
}

/// Apply a staged registration change inside the state commit transaction.
/// Rotation and unregistration both fence every unfinished job for the device.
pub(crate) fn apply_staged(
    connection: &Connection,
    staged: Staged,
    now: i64,
) -> Result<(), String> {
    match staged {
        Staged::Register {
            device,
            platform,
            token,
        } => {
            connection
                .execute(
                    "UPDATE mobile_push_jobs SET status='cancelled',last_error='rotated',updated_at=?2
                     WHERE device_id=?1 AND status IN ('pending','leased','submitted')",
                    params![device, now],
                )
                .map_err(|e| e.to_string())?;
            connection
                .execute(
                    "INSERT INTO mobile_push_registrations
                     (device_id,platform,token,generation,active,registered_at)
                     VALUES(?1,?2,?3,1,1,?4)
                     ON CONFLICT(device_id) DO UPDATE SET
                       platform=excluded.platform,token=excluded.token,
                       generation=mobile_push_registrations.generation+1,active=1,
                       registered_at=excluded.registered_at,unregistered_at=NULL,
                       deactivated_at=NULL,last_error=NULL",
                    params![device, platform, token, now],
                )
                .map_err(|e| e.to_string())?;
        }
        Staged::Unregister { device } => {
            connection
                .execute(
                    "UPDATE mobile_push_registrations SET active=0,unregistered_at=?2,
                       token='',last_error=NULL WHERE device_id=?1 AND active=1",
                    params![device, now],
                )
                .map_err(|e| e.to_string())?;
            connection
                .execute(
                    "UPDATE mobile_push_jobs SET status='cancelled',last_error='unregistered',updated_at=?2
                     WHERE device_id=?1 AND status IN ('pending','leased','submitted')",
                    params![device, now],
                )
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Called inside the state commit for every staged device revocation. Future
/// sends to a revoked device are cancelled in the same transaction.
pub(crate) fn device_revoked(
    connection: &Connection,
    device: &str,
    now: i64,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE mobile_push_registrations SET active=0,deactivated_at=?2,
               token='',last_error='revoked' WHERE device_id=?1 AND active=1",
            params![device, now],
        )
        .map_err(|e| e.to_string())?;
    connection
        .execute(
            "UPDATE mobile_push_jobs SET status='cancelled',last_error='revoked',updated_at=?2
             WHERE device_id=?1 AND status IN ('pending','leased','submitted')",
            params![device, now],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Create one durable outbox job per opted-in, live device for a freshly
/// committed feed event. Called inside the projection transaction, so a job
/// exists iff its event committed. `INSERT OR IGNORE` makes replays idempotent;
/// events that predate the registration are never enqueued. Overflow evicts
/// the oldest unfinished work for that device so a stuck phone cannot grow the
/// queue without bound.
pub(crate) fn enqueue(
    connection: &Connection,
    event_seq: i64,
    event_id: &str,
    kind: &str,
    now: i64,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(
            "SELECT r.device_id,r.generation FROM mobile_push_registrations r
             JOIN devices d ON d.id=r.device_id
             WHERE r.active=1 AND d.revoked_at IS NULL",
        )
        .map_err(|e| e.to_string())?;
    let registrations = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if registrations.is_empty() {
        return Ok(());
    }
    connection
        .execute(
            "UPDATE mobile_notification_events SET push_required=1 WHERE seq=?1",
            [event_seq],
        )
        .map_err(|e| e.to_string())?;
    for (device, generation) in registrations {
        connection
            .execute(
                "INSERT OR IGNORE INTO mobile_push_jobs
                 (event_seq,event_id,kind,device_id,generation,status,next_attempt_at,created_at,updated_at)
                 VALUES(?1,?2,?3,?4,?5,'pending',0,?6,?6)",
                params![event_seq, event_id, kind, device, generation, now],
            )
            .map_err(|e| e.to_string())?;
        connection
            .execute(
                "UPDATE mobile_push_jobs SET status='cancelled',last_error='overflow',updated_at=?2
                 WHERE seq IN (SELECT seq FROM mobile_push_jobs
                    WHERE device_id=?1 AND status IN ('pending','leased','submitted')
                    ORDER BY seq DESC LIMIT -1 OFFSET ?3)",
                params![device, now, PENDING_PER_DEVICE],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The global unfinished bound, applied at the end of `record` and at the top
/// of every worker cycle.
pub(crate) fn bound_global(connection: &Connection, now: i64) -> Result<(), String> {
    connection
        .execute(
            "UPDATE mobile_push_jobs SET status='cancelled',last_error='overflow',updated_at=?1
             WHERE seq IN (SELECT seq FROM mobile_push_jobs
                WHERE status IN ('pending','leased','submitted')
                ORDER BY seq DESC LIMIT -1 OFFSET ?2)",
            params![now, UNFINISHED_GLOBAL],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// An event's push lifecycle is settled once every job for it is terminal.
/// Until then `push_required=1 AND delivered_at IS NULL` exempts the feed row
/// from retention pruning, so a tapped notification still resolves.
pub(crate) fn settle_events(connection: &Connection, now: i64) -> Result<(), String> {
    connection
        .execute(
            "UPDATE mobile_notification_events SET delivered_at=?1
             WHERE push_required=1 AND delivered_at IS NULL
               AND NOT EXISTS(SELECT 1 FROM mobile_push_jobs j
                    WHERE j.event_seq=mobile_notification_events.seq
                      AND j.status IN ('pending','leased','submitted'))",
            [now],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Terminal jobs are audit, not history: keep a bounded tail per device.
fn prune_terminal(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM mobile_push_jobs WHERE seq IN (
                SELECT seq FROM (
                    SELECT seq, ROW_NUMBER() OVER
                        (PARTITION BY device_id ORDER BY seq DESC) AS kept
                    FROM mobile_push_jobs
                    WHERE status IN ('delivered','failed','cancelled')
                ) WHERE kept > ?1)",
            [TERMINAL_KEEP],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Status for the caller's own device. Contains no token, ever.
pub(crate) fn status(connection: &Connection, device: &str) -> Result<Value, String> {
    let live: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND revoked_at IS NULL)",
            [device],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    type RegistrationRow = (String, i64, i64, Option<i64>, Option<i64>, Option<String>);
    let row: Option<RegistrationRow> = connection
        .query_row(
            "SELECT platform,active,registered_at,unregistered_at,deactivated_at,last_error
             FROM mobile_push_registrations WHERE device_id=?1",
            [device],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let pending: i64 = connection
        .query_row(
            "SELECT count(*) FROM mobile_push_jobs
             WHERE device_id=?1 AND status IN ('pending','leased','submitted')",
            [device],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let (platform, active, registered_at, unregistered_at, deactivated_at, last_error) = match &row
    {
        Some(row) => (
            Some(row.0.clone()),
            row.1 == 1,
            Some(row.2),
            row.3,
            row.4,
            row.5.clone(),
        ),
        None => (None, false, None, None, None, None),
    };
    let state = if row.is_none() {
        "none"
    } else if active && live {
        "active"
    } else if !live {
        "revoked"
    } else if deactivated_at.is_some() {
        "deactivated"
    } else if unregistered_at.is_some() {
        "unregistered"
    } else {
        "none"
    };
    Ok(json!({
        "registered": state == "active",
        "status": state,
        "platform": platform,
        "registeredAt": registered_at,
        "pendingJobs": pending,
        "lastError": last_error,
    }))
}

/// The local bookkeeping every cycle performs before any network: recover
/// expired leases, expire jobs that outlived their retry or receipt windows,
/// eagerly cancel work whose registration fence is gone, settle delivered
/// events and prune terminal rows.
pub(crate) fn prepare(connection: &Connection, now: i64) -> Result<Survey, String> {
    let tx = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE mobile_push_jobs SET status='pending',lease_owner=NULL,lease_expires_at=NULL
         WHERE status='leased' AND lease_expires_at<?1",
        [now],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE mobile_push_jobs SET status='cancelled',last_error='expired',updated_at=?1
         WHERE status='pending' AND created_at+?2<?1",
        params![now, MAX_JOB_AGE_MS],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE mobile_push_jobs SET status='failed',last_error='receipt_timeout',updated_at=?1
         WHERE status='submitted' AND updated_at+?2<?1",
        params![now, RECEIPT_WINDOW_MS],
    )
    .map_err(|e| e.to_string())?;
    // A pending job whose registration is gone, inactive, rotated or revoked
    // can never send; cancel it now rather than at claim time.
    tx.execute(
        "UPDATE mobile_push_jobs SET status='cancelled',last_error='fenced',updated_at=?1
         WHERE status='pending' AND NOT EXISTS(
            SELECT 1 FROM mobile_push_registrations r JOIN devices d ON d.id=r.device_id
            WHERE r.device_id=mobile_push_jobs.device_id AND r.active=1
              AND r.generation=mobile_push_jobs.generation AND d.revoked_at IS NULL)",
        [now],
    )
    .map_err(|e| e.to_string())?;
    bound_global(&tx, now)?;
    settle_events(&tx, now)?;
    prune_terminal(&tx)?;
    let due_sends: i64 = tx
        .query_row(
            "SELECT count(*) FROM mobile_push_jobs WHERE status='pending' AND next_attempt_at<=?1",
            [now],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let due_tickets: i64 = tx
        .query_row(
            "SELECT count(*) FROM mobile_push_jobs WHERE status='submitted' AND next_attempt_at<=?1",
            [now],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(Survey {
        due_sends,
        due_tickets,
    })
}

pub struct Survey {
    pub due_sends: i64,
    pub due_tickets: i64,
}

/// A leased job ready to send. `token` stays in memory only.
pub struct Claimed {
    pub seq: i64,
    pub device_id: String,
    pub generation: i64,
    pub token: String,
    pub event_id: String,
    pub kind: String,
}

/// A submitted job whose Expo receipt is due for a poll.
pub struct Ticket {
    pub seq: i64,
    pub device_id: String,
    pub generation: i64,
    pub ticket_id: String,
}

pub struct ClaimedBatch {
    pub jobs: Vec<Claimed>,
    pub tickets: Vec<Ticket>,
}

/// Claim due work under `lease`. Every candidate is re-verified against the
/// live registration generation and device inside the same transaction, so a
/// revoked, rotated or unregistered device never leases work. Attempts count
/// each claim: a crash mid-flight still consumes a bounded attempt.
pub(crate) fn claim(
    connection: &Connection,
    now: i64,
    lease: &str,
) -> Result<ClaimedBatch, String> {
    let tx = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let mut statement = tx
        .prepare(
            "SELECT seq,device_id,generation,event_id,kind FROM mobile_push_jobs
             WHERE status='pending' AND next_attempt_at<=?1 ORDER BY seq LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let candidates = statement
        .query_map(params![now, CLAIM_BATCH], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    let mut jobs = Vec::new();
    for (seq, device_id, generation, event_id, kind) in candidates {
        let token: Option<String> = tx
            .query_row(
                "SELECT r.token FROM mobile_push_registrations r
                 JOIN devices d ON d.id=r.device_id
                 WHERE r.device_id=?1 AND r.active=1 AND r.generation=?2 AND d.revoked_at IS NULL",
                params![device_id, generation],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match token {
            Some(token) => {
                tx.execute(
                    "UPDATE mobile_push_jobs SET status='leased',lease_owner=?2,
                       lease_expires_at=?3,attempts=attempts+1,updated_at=?4
                     WHERE seq=?1 AND status='pending'",
                    params![seq, lease, now + LEASE_MS, now],
                )
                .map_err(|e| e.to_string())?;
                jobs.push(Claimed {
                    seq,
                    device_id,
                    generation,
                    token,
                    event_id,
                    kind,
                });
            }
            None => {
                tx.execute(
                    "UPDATE mobile_push_jobs SET status='cancelled',last_error='fenced',updated_at=?2
                     WHERE seq=?1 AND status='pending'",
                    params![seq, now],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    let mut statement = tx
        .prepare(
            "SELECT seq,device_id,generation,ticket_id FROM mobile_push_jobs
             WHERE status='submitted' AND ticket_id IS NOT NULL AND next_attempt_at<=?1
             ORDER BY seq LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let tickets = statement
        .query_map(params![now, TICKET_BATCH], |r| {
            Ok(Ticket {
                seq: r.get(0)?,
                device_id: r.get(1)?,
                generation: r.get(2)?,
                ticket_id: r.get::<_, String>(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ClaimedBatch { jobs, tickets })
}

/// What the transport observed for one send attempt.
pub enum SendOutcome {
    /// Expo accepted the ticket. Not delivered until a receipt says so.
    Submitted(String),
    /// Timeout, 429 or 5xx. Back to pending with bounded backoff.
    Retry(&'static str),
    /// Refused in a way retrying cannot fix.
    Failed(&'static str),
    /// The token is dead: deactivate this registration generation.
    DeviceGone,
}

/// What the receipt endpoint observed for one submitted job.
pub enum ReceiptOutcome {
    Delivered,
    Failed(&'static str),
    DeviceGone,
    /// No receipt yet: keep polling inside the receipt window.
    Pending,
}

pub struct SendReport {
    pub seq: i64,
    pub device_id: String,
    pub generation: i64,
    pub outcome: SendOutcome,
}

pub struct ReceiptReport {
    pub seq: i64,
    pub device_id: String,
    pub generation: i64,
    pub outcome: ReceiptOutcome,
}

fn backoff(attempts: i64) -> i64 {
    let shift = (attempts - 1).clamp(0, 20);
    (BASE_BACKOFF_MS << shift).min(MAX_BACKOFF_MS)
}

fn deactivate(
    tx: &Connection,
    device_id: &str,
    generation: i64,
    reason: &'static str,
    now: i64,
) -> Result<(), String> {
    tx.execute(
        "UPDATE mobile_push_jobs SET status='cancelled',last_error=?3,updated_at=?2
         WHERE device_id=?1 AND generation=?4 AND status IN ('pending','leased','submitted')",
        params![device_id, now, reason, generation],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE mobile_push_registrations SET active=0,deactivated_at=?2,token='',last_error=?3
         WHERE device_id=?1 AND generation=?4 AND active=1",
        params![device_id, now, reason, generation],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Apply network outcomes inside one transaction. Every write is fenced on the
/// lease/state the worker actually held, so an unregister, rotation or
/// revocation that committed mid-flight cannot be undone by a stale result.
pub(crate) fn settle(
    connection: &Connection,
    now: i64,
    lease: &str,
    sends: &[SendReport],
    receipts: &[ReceiptReport],
) -> Result<(), String> {
    let tx = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    for report in sends {
        match &report.outcome {
            SendOutcome::Submitted(ticket) => {
                tx.execute(
                    "UPDATE mobile_push_jobs SET status='submitted',ticket_id=?2,
                       next_attempt_at=?3,updated_at=?4
                     WHERE seq=?1 AND status='leased' AND lease_owner=?5",
                    params![report.seq, ticket, now + RECEIPT_POLL_MS, now, lease],
                )
                .map_err(|e| e.to_string())?;
            }
            SendOutcome::Retry(reason) => {
                let next = now + backoff(report_seq_attempts(&tx, report.seq));
                tx.execute(
                    "UPDATE mobile_push_jobs SET
                       status=CASE WHEN attempts>=?3 THEN 'failed' ELSE 'pending' END,
                       last_error=CASE WHEN attempts>=?3 THEN 'attempts' ELSE ?4 END,
                       next_attempt_at=?5,lease_owner=NULL,lease_expires_at=NULL,updated_at=?6
                     WHERE seq=?1 AND status='leased' AND lease_owner=?2",
                    params![report.seq, lease, MAX_ATTEMPTS, reason, next, now],
                )
                .map_err(|e| e.to_string())?;
            }
            SendOutcome::Failed(reason) => {
                tx.execute(
                    "UPDATE mobile_push_jobs SET status='failed',last_error=?3,
                       lease_owner=NULL,lease_expires_at=NULL,updated_at=?4
                     WHERE seq=?1 AND status='leased' AND lease_owner=?2",
                    params![report.seq, lease, reason, now],
                )
                .map_err(|e| e.to_string())?;
            }
            SendOutcome::DeviceGone => {
                deactivate(
                    &tx,
                    &report.device_id,
                    report.generation,
                    "device_not_registered",
                    now,
                )?;
            }
        }
    }
    for report in receipts {
        match &report.outcome {
            ReceiptOutcome::Delivered => {
                tx.execute(
                    "UPDATE mobile_push_jobs SET status='delivered',updated_at=?2
                     WHERE seq=?1 AND status='submitted'",
                    params![report.seq, now],
                )
                .map_err(|e| e.to_string())?;
            }
            ReceiptOutcome::Failed(reason) => {
                tx.execute(
                    "UPDATE mobile_push_jobs SET status='failed',last_error=?3,updated_at=?2
                     WHERE seq=?1 AND status='submitted'",
                    params![report.seq, now, reason],
                )
                .map_err(|e| e.to_string())?;
            }
            ReceiptOutcome::DeviceGone => {
                deactivate(
                    &tx,
                    &report.device_id,
                    report.generation,
                    "device_not_registered",
                    now,
                )?;
            }
            ReceiptOutcome::Pending => {
                tx.execute(
                    "UPDATE mobile_push_jobs SET next_attempt_at=?2
                     WHERE seq=?1 AND status='submitted'",
                    params![report.seq, now + RECEIPT_POLL_MS],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    settle_events(&tx, now)?;
    prune_terminal(&tx)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

fn report_seq_attempts(connection: &Connection, seq: i64) -> i64 {
    connection
        .query_row(
            "SELECT attempts FROM mobile_push_jobs WHERE seq=?1",
            [seq],
            |r| r.get(0),
        )
        .unwrap_or(1)
}

pub enum PostError {
    /// Timeout, DNS/connect failure, HTTP 429 or 5xx: retry with backoff.
    Transient(&'static str),
    /// A 4xx that retrying cannot fix, or a response outside the Expo schema:
    /// terminate with a generic status.
    Permanent(&'static str),
}

/// One bounded HTTPS POST returning the parsed JSON body. Implementations must
/// never put token or payload text into the error they return.
pub trait Transport: Send + Sync {
    fn post(&self, url: &str, body: &Value) -> Result<Value, PostError>;
}

/// The only production transport: fixed Expo endpoints, no proxies, no
/// redirects, short timeouts and a bounded response read.
struct ExpoTransport {
    client: reqwest::blocking::Client,
}

impl ExpoTransport {
    fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self { client }
    }
}

impl Transport for ExpoTransport {
    fn post(&self, url: &str, body: &Value) -> Result<Value, PostError> {
        use std::io::Read;
        let response = self
            .client
            .post(url)
            .json(body)
            .send()
            .map_err(|_| PostError::Transient("transport"))?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PostError::Transient("transport"))?;
        if bytes.len() as u64 > MAX_RESPONSE {
            return Err(PostError::Permanent("invalid_response"));
        }
        if status == 429 || status >= 500 {
            return Err(PostError::Transient("http_status"));
        }
        if !(200..300).contains(&status) {
            return Err(PostError::Permanent("http_status"));
        }
        serde_json::from_slice(&bytes).map_err(|_| PostError::Permanent("invalid_response"))
    }
}

fn body_for(kind: &str) -> &'static str {
    match kind {
        "approval" => "Approval needed",
        "user_input" => "Input needed",
        "run_failed" => "Run failed",
        _ => "Run finished",
    }
}

/// Parse the Expo ticket array honestly: `status:"ok"` is submission only.
/// Anything outside the array schema fails the whole batch with a generic
/// status rather than guessing at delivery.
fn ticket_outcomes(response: &Value, expected: usize) -> Option<Vec<SendOutcome>> {
    let data = response["data"].as_array()?;
    if data.len() != expected {
        return None;
    }
    Some(
        data.iter()
            .map(|ticket| match ticket["status"].as_str() {
                Some("ok") => ticket["id"]
                    .as_str()
                    .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                    .map(|id| SendOutcome::Submitted(id.to_owned()))
                    .unwrap_or(SendOutcome::Failed("invalid_ticket")),
                Some("error") => match ticket["details"]["error"].as_str() {
                    Some("DeviceNotRegistered") => SendOutcome::DeviceGone,
                    Some("MessageRateExceeded") => SendOutcome::Retry("rate_limited"),
                    _ => SendOutcome::Failed("ticket_error"),
                },
                _ => SendOutcome::Failed("invalid_ticket"),
            })
            .collect(),
    )
}

fn receipt_outcome(entry: Option<&Value>) -> ReceiptOutcome {
    let Some(entry) = entry else {
        return ReceiptOutcome::Pending;
    };
    match entry["status"].as_str() {
        Some("ok") => ReceiptOutcome::Delivered,
        Some("error") => match entry["details"]["error"].as_str() {
            Some("DeviceNotRegistered") => ReceiptOutcome::DeviceGone,
            Some("MessageRateExceeded") => ReceiptOutcome::Pending,
            _ => ReceiptOutcome::Failed("receipt_error"),
        },
        _ => ReceiptOutcome::Failed("invalid_response"),
    }
}

/// The outbox worker. `cycle` is synchronous and deterministic: the async
/// `run` wrapper only schedules it and owns the real transport.
#[derive(Clone)]
pub struct Worker {
    database: crate::database::Database,
    server_id: String,
    transport: Arc<dyn Transport>,
}

pub struct CycleReport {
    pub idle: bool,
}

impl Worker {
    pub fn new(
        database: crate::database::Database,
        server_id: String,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            database,
            server_id,
            transport,
        }
    }

    /// One bounded pass: local bookkeeping, then (behind the maintenance
    /// dispatch fence) claim committed jobs, deliver and poll receipts.
    /// Returns `idle` when no opted-in work exists so the caller can back off.
    pub fn cycle(&self, now: i64) -> Result<CycleReport, String> {
        let survey = self.database.push_prepare(now)?;
        if survey.due_sends == 0 && survey.due_tickets == 0 {
            return Ok(CycleReport { idle: true });
        }
        // Maintenance can wait for a claimed send; it must never see one start
        // after the fence drops. Local bookkeeping above has already run.
        let _dispatch = match self.database.claim_dispatch("mobile push delivery")? {
            Some(dispatch) => dispatch,
            None => return Ok(CycleReport { idle: false }),
        };
        let lease = uuid::Uuid::new_v4().to_string();
        let claimed = self.database.push_claim(now, &lease)?;
        let mut sends: Vec<SendReport> = Vec::new();
        if !claimed.jobs.is_empty() {
            for chunk in claimed.jobs.chunks(SEND_BATCH) {
                let messages: Vec<Value> = chunk
                    .iter()
                    .map(|job| {
                        json!({
                            "to": job.token,
                            "title": "Shina",
                            "body": body_for(&job.kind),
                            "priority": "high",
                            "sound": "default",
                            "data": {"serverId": self.server_id, "eventId": job.event_id},
                        })
                    })
                    .collect();
                let outcomes = match self.transport.post(SEND_URL, &json!(messages)) {
                    Ok(response) => match ticket_outcomes(&response, chunk.len()) {
                        Some(outcomes) => outcomes,
                        None => chunk
                            .iter()
                            .map(|_| SendOutcome::Failed("invalid_response"))
                            .collect(),
                    },
                    Err(PostError::Transient(reason)) => {
                        chunk.iter().map(|_| SendOutcome::Retry(reason)).collect()
                    }
                    Err(PostError::Permanent(reason)) => {
                        chunk.iter().map(|_| SendOutcome::Failed(reason)).collect()
                    }
                };
                sends.extend(chunk.iter().zip(outcomes).map(|(job, outcome)| SendReport {
                    seq: job.seq,
                    device_id: job.device_id.clone(),
                    generation: job.generation,
                    outcome,
                }));
            }
        }
        let mut receipts: Vec<ReceiptReport> = Vec::new();
        for chunk in claimed.tickets.chunks(TICKET_BATCH as usize) {
            let ids: Vec<&str> = chunk.iter().map(|t| t.ticket_id.as_str()).collect();
            match self.transport.post(RECEIPT_URL, &json!({"ids": ids})) {
                Ok(response) => {
                    receipts.extend(chunk.iter().map(|ticket| ReceiptReport {
                        seq: ticket.seq,
                        device_id: ticket.device_id.clone(),
                        generation: ticket.generation,
                        outcome: receipt_outcome(response["data"].get(&ticket.ticket_id)),
                    }));
                }
                // The send already succeeded; a poll failure is never fatal.
                Err(_) => receipts.extend(chunk.iter().map(|ticket| ReceiptReport {
                    seq: ticket.seq,
                    device_id: ticket.device_id.clone(),
                    generation: ticket.generation,
                    outcome: ReceiptOutcome::Pending,
                })),
            }
        }
        self.database.push_settle(now, &lease, &sends, &receipts)?;
        Ok(CycleReport { idle: false })
    }
}

/// Spawned inside the server's existing network runtime. Polls the durable
/// outbox; a `true` shutdown watch or an invalid server id stops it cleanly
/// and leased jobs recover on the next start.
pub async fn run(
    database: crate::database::Database,
    server_id: String,
    shutdown: tokio::sync::watch::Receiver<bool>,
) {
    if uuid::Uuid::parse_str(&server_id).is_err() {
        return;
    }
    run_with(
        database,
        server_id,
        Arc::new(ExpoTransport::new()),
        shutdown,
    )
    .await;
}

pub(crate) async fn run_with(
    database: crate::database::Database,
    server_id: String,
    transport: Arc<dyn Transport>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let worker = Worker::new(database, server_id, transport);
    let mut wait = IDLE_TICK;
    while !*shutdown.borrow() {
        let result = tokio::select! {
            _ = shutdown.changed() => break,
            _ = tokio::time::sleep(wait) => {
                let worker = worker.clone();
                tokio::task::spawn_blocking(move || {
                    worker.cycle(crate::lifecycle::now() as i64)
                })
                .await
            }
        };
        wait = match result {
            Ok(Ok(report)) if !report.idle => ACTIVE_TICK,
            _ => IDLE_TICK,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Command, Receipt, ReceiptStatus};
    use rusqlite::params;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Mutex;

    fn directory(name: &str) -> PathBuf {
        let directory = crate::store::directory().unwrap().join(name);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn open(name: &str) -> crate::database::Database {
        crate::database::Database::open(&directory(name)).unwrap()
    }

    const DEVICE: &str = "dev-mobile-1";
    const TOKEN: &str = "ExponentPushToken[abcdef1234567890ABCDEF]";
    const TOKEN2: &str = "ExponentPushToken[zzzzzz9999999999ZZZZZZ]";
    const SERVER: &str = "11111111-2222-3333-4444-555555555555";

    fn device(db: &crate::database::Database, id: &str) {
        db.issue_device(id, &format!("secret-{id}")).unwrap();
    }

    fn commit(db: &crate::database::Database, seq: u64, workspace: Value) {
        db.commit(seq, &workspace, &json!([]), None, &[]).unwrap();
    }

    fn receipt(seq: u64) -> Receipt {
        let command = Command::new("notifications.push.register", json!({}));
        Receipt {
            command_id: command.command_id,
            device_id: DEVICE.into(),
            hash: command.hash(),
            status: ReceiptStatus::Accepted,
            result: json!({}),
            seq,
        }
    }

    /// Stage registration inside a runtime batch and commit it with a receipt.
    /// The workspace committed preserves the committed baseline, not a wipe.
    fn register(
        db: &crate::database::Database,
        seq: u64,
        device: &str,
        token: &str,
        workspace: Value,
    ) -> Value {
        db.begin().unwrap();
        let result = db
            .stage_push_registration(device, &json!({"platform":"ios","token":token}))
            .unwrap();
        db.commit(seq, &workspace, &json!([]), Some(&receipt(seq)), &[])
            .unwrap();
        result
    }

    fn running() -> Value {
        json!({"threads":[{"id":1,"status":"Running","run_id":"r1","generation":"g1"}]})
    }

    fn waiting(request: &str) -> Value {
        json!({"threads":[{"id":1,"status":"Waiting","run_id":"r1","generation":"g1",
            "request":{"id":request,"method":"confirm","title":"Approve deletion of secrets",
            "tool":{"command":"rm -rf /"},"runtimeGeneration":"g1"}}]})
    }

    fn count(db: &crate::database::Database, sql: &str) -> i64 {
        db.test_connection(|c| c.query_row(sql, [], |r| r.get(0)).unwrap())
    }

    fn field(db: &crate::database::Database, sql: &str) -> String {
        db.test_connection(|c| c.query_row(sql, [], |r| r.get::<_, String>(0)).unwrap())
    }

    fn jobs(db: &crate::database::Database, status: Option<&str>) -> Vec<(i64, String, String)> {
        let (sql, args): (String, Vec<String>) = match status {
            Some(status) => (
                "SELECT seq,device_id,status FROM mobile_push_jobs WHERE status=?1 ORDER BY seq"
                    .into(),
                vec![status.into()],
            ),
            None => (
                "SELECT seq,device_id,status FROM mobile_push_jobs ORDER BY seq".into(),
                vec![],
            ),
        };
        db.test_connection(|c| {
            c.prepare(&sql)
                .unwrap()
                .query_map(rusqlite::params_from_iter(args.iter()), |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
    }

    fn insert_job(db: &crate::database::Database, event_id: &str, status: &str) {
        db.test_connection(|c| {
            c.execute(
                "INSERT INTO mobile_push_jobs
                 (event_seq,event_id,kind,device_id,generation,status,next_attempt_at,created_at,updated_at)
                 VALUES(-1,?1,'run_completed',?2,1,?3,0,1,1)",
                params![event_id, DEVICE, status],
            )
            .unwrap();
        });
    }

    struct Fake {
        requests: Mutex<Vec<(String, Value)>>,
        responses: Mutex<VecDeque<Result<Value, PostError>>>,
    }

    impl Fake {
        fn new(responses: Vec<Result<Value, PostError>>) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into()),
            })
        }
        fn ok(data: Value) -> Result<Value, PostError> {
            Ok(json!({"data":data}))
        }
        fn requests(&self) -> Vec<(String, Value)> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Transport for Fake {
        fn post(&self, url: &str, body: &Value) -> Result<Value, PostError> {
            self.requests
                .lock()
                .unwrap()
                .push((url.to_owned(), body.clone()));
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(PostError::Transient("transport")))
        }
    }

    fn worker(db: &crate::database::Database, transport: Arc<dyn Transport>) -> Worker {
        Worker::new(db.clone(), SERVER.into(), transport)
    }

    /// Baseline thread, registration gen1, one approval edge, one pending job.
    /// Returns the job seq.
    fn pending_job(db: &crate::database::Database) -> i64 {
        device(db, DEVICE);
        commit(db, 1, running());
        register(db, 2, DEVICE, TOKEN, running());
        commit(db, 3, waiting("req-1"));
        jobs(db, Some("pending"))[0].0
    }

    #[test]
    fn expo_token_validation_is_strict() {
        assert!(valid_token(TOKEN));
        assert!(valid_token("ExpoPushToken[abcd-EFGH_12345]"));
        for bad in [
            "",
            "ExponentPushToken[]",
            "ExponentPushToken[short]",
            "ExponentPushToken[has space here]",
            "ExponentPushToken[abc]extra",
            "FCMToken[abcdef12345678]",
            "abcdef12345678",
            "ExponentPushToken(abcdef12345678)",
        ] {
            assert!(!valid_token(bad), "{bad}");
        }
        assert!(!valid_token(&format!(
            "ExponentPushToken[{}]",
            "a".repeat(193)
        )));
    }

    #[test]
    fn registration_validates_and_stages_inside_a_runtime_batch() {
        let db = open("push-validate");
        device(&db, DEVICE);
        // Staging outside a runtime transaction is an autonomous write: refused.
        assert!(
            db.stage_push_registration(DEVICE, &json!({"platform":"ios","token":TOKEN}))
                .is_err()
        );
        db.begin().unwrap();
        for args in [
            json!({"platform":"web","token":TOKEN}),
            json!({"platform":"ios"}),
            json!({"platform":"ios","token":"not-a-token"}),
            json!({"platform":"ios","token":TOKEN,"deviceId":"someone-else"}),
        ] {
            assert!(db.stage_push_registration(DEVICE, &args).is_err(), "{args}");
        }
        // Unknown and revoked devices cannot register.
        assert!(
            db.stage_push_registration("ghost", &json!({"platform":"ios","token":TOKEN}))
                .is_err()
        );
        let result = db
            .stage_push_registration(DEVICE, &json!({"platform":"ios","token":TOKEN}))
            .unwrap();
        assert_eq!(result, json!({"registered":true,"platform":"ios"}));
        assert!(!result.to_string().contains("ExponentPushToken"));
        db.commit(1, &json!({}), &json!([]), Some(&receipt(1)), &[])
            .unwrap();
        let status = db.push_status(DEVICE).unwrap();
        assert_eq!(status["registered"], true);
        assert_eq!(status["status"], "active");
        assert_eq!(status["platform"], "ios");
        assert!(!status.to_string().contains("ExponentPushToken"));
        // The raw token exists only in the registrations table.
        assert_eq!(
            field(&db, "SELECT token FROM mobile_push_registrations"),
            TOKEN
        );
        // Status for a device that never registered is token-free and generic.
        let empty = db.push_status("ghost").unwrap();
        assert_eq!(empty["registered"], false);
        assert_eq!(empty["status"], "none");
    }

    #[test]
    fn registration_rolls_back_with_its_receipt_commit() {
        let db = open("push-rollback");
        device(&db, DEVICE);
        db.test_connection(|c| {
            c.execute_batch(
                "CREATE TRIGGER fail_push BEFORE INSERT ON mobile_push_registrations
                 BEGIN SELECT RAISE(ABORT,'fixture disk error'); END;",
            )
            .unwrap();
        });
        db.begin().unwrap();
        db.stage_push_registration(DEVICE, &json!({"platform":"ios","token":TOKEN}))
            .unwrap();
        assert!(
            db.commit(1, &json!({}), &json!([]), Some(&receipt(1)), &[])
                .is_err()
        );
        assert_eq!(
            count(&db, "SELECT count(*) FROM mobile_push_registrations"),
            0,
            "a failed commit publishes no registration"
        );
    }

    #[test]
    fn jobs_commit_atomically_with_events_and_only_for_opted_devices() {
        let db = open("push-jobs");
        device(&db, DEVICE);
        device(&db, "dev-2");
        commit(&db, 1, running());
        // No opt-in: an edge commits no jobs.
        commit(&db, 2, waiting("req-1"));
        assert!(jobs(&db, None).is_empty());
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM mobile_notification_events WHERE push_required=1"
            ),
            0
        );
        register(&db, 3, DEVICE, TOKEN, waiting("req-1"));
        // A new edge after opt-in creates exactly one job for the opted device.
        commit(&db, 4, running());
        commit(&db, 5, waiting("req-2"));
        let rows = jobs(&db, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, DEVICE);
        assert_eq!(rows[0].2, "pending");
        // The shared commit aborts event and job together.
        db.test_connection(|c| {
            c.execute_batch(
                "CREATE TRIGGER fail_job BEFORE INSERT ON mobile_push_jobs
                 BEGIN SELECT RAISE(ABORT,'fixture disk error'); END;",
            )
            .unwrap();
        });
        let before = count(&db, "SELECT count(*) FROM mobile_notification_events");
        assert!(
            db.commit(
                6,
                &json!({"threads":[{"id":1,"status":"Completed","run_id":"r1","generation":"g1"}]}),
                &json!([]),
                None,
                &[]
            )
            .is_err()
        );
        assert_eq!(
            before,
            count(&db, "SELECT count(*) FROM mobile_notification_events"),
            "no half-recorded notification edge"
        );
    }

    #[test]
    fn push_payload_is_generic_and_carries_only_opaque_ids() {
        let db = open("push-payload");
        pending_job(&db);
        let ticket = uuid::Uuid::new_v4().to_string();
        let fake = Fake::new(vec![Fake::ok(json!([{"status":"ok","id":ticket}]))]);
        worker(&db, fake.clone()).cycle(1_000_000).unwrap();
        let requests = fake.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, SEND_URL);
        let message = &requests[0].1[0];
        let mut keys: Vec<_> = message.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["body", "data", "priority", "sound", "title", "to"]);
        assert_eq!(message["title"], "Shina");
        assert_eq!(message["body"], "Approval needed");
        let mut data: Vec<_> = message["data"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        data.sort();
        assert_eq!(data, ["eventId", "serverId"]);
        assert_eq!(message["data"]["serverId"], SERVER);
        assert!(uuid::Uuid::parse_str(message["data"]["eventId"].as_str().unwrap()).is_ok());
        for forbidden in [
            "req-1",
            "Approve deletion",
            "rm -rf",
            "secrets",
            "threadId",
            "runId",
            "requestId",
        ] {
            assert!(
                !requests[0].1.to_string().contains(forbidden),
                "{forbidden}"
            );
        }
        // Ticket acceptance is submission, never delivery.
        assert_eq!(jobs(&db, Some("submitted")).len(), 1);
        assert!(jobs(&db, Some("delivered")).is_empty());
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM mobile_notification_events WHERE delivered_at IS NOT NULL"
            ),
            0
        );
    }

    #[test]
    fn receipts_prove_delivery_and_device_not_registered_deactivates() {
        let db = open("push-receipts");
        pending_job(&db);
        let ticket = uuid::Uuid::new_v4().to_string();
        let mut receipt_body = json!({});
        receipt_body[ticket.clone()] = json!({"status":"ok"});
        let fake = Fake::new(vec![
            Fake::ok(json!([{"status":"ok","id":ticket}])),
            Fake::ok(json!({})), // no receipt yet
            Fake::ok(receipt_body),
        ]);
        let w = worker(&db, fake);
        w.cycle(1_000_000).unwrap();
        w.cycle(1_000_000 + RECEIPT_POLL_MS).unwrap();
        assert_eq!(jobs(&db, Some("submitted")).len(), 1);
        w.cycle(1_000_000 + 2 * RECEIPT_POLL_MS).unwrap();
        assert_eq!(jobs(&db, Some("delivered")).len(), 1);
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM mobile_notification_events WHERE delivered_at IS NOT NULL"
            ),
            1
        );
        // A new edge on a rotated registration gets its own job.
        register(&db, 4, DEVICE, TOKEN2, waiting("req-1"));
        commit(&db, 5, running());
        commit(&db, 6, waiting("req-2"));
        assert_eq!(jobs(&db, Some("pending")).len(), 1);
        let ticket = uuid::Uuid::new_v4().to_string();
        let mut dead = json!({});
        dead[ticket.clone()] = json!({"status":"error","details":{"error":"DeviceNotRegistered"}});
        let fake = Fake::new(vec![
            Fake::ok(json!([{"status":"ok","id":ticket}])),
            Fake::ok(dead),
        ]);
        let w = worker(&db, fake);
        w.cycle(2_000_000).unwrap();
        w.cycle(2_000_000 + RECEIPT_POLL_MS).unwrap();
        assert_eq!(jobs(&db, Some("cancelled")).len(), 1);
        let status = db.push_status(DEVICE).unwrap();
        assert_eq!(status["registered"], false);
        assert_eq!(status["status"], "deactivated");
        assert_eq!(status["lastError"], "device_not_registered");
        assert_eq!(
            field(&db, "SELECT token FROM mobile_push_registrations"),
            ""
        );
    }

    #[test]
    fn transient_failures_retry_with_bounded_attempts_and_permanent_ones_stop() {
        let db = open("push-retry");
        pending_job(&db);
        let fake = Fake::new(vec![]); // every post is a transient failure
        let w = worker(&db, fake);
        w.cycle(1_000_000).unwrap();
        let (status, attempts, next): (String, i64, i64) = db.test_connection(|c| {
            c.query_row(
                "SELECT status,attempts,next_attempt_at FROM mobile_push_jobs",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        });
        assert_eq!(status, "pending");
        assert_eq!(attempts, 1);
        assert!(next > 1_000_000);
        // Exhaust the attempt bound: the job fails terminally, not forever.
        for step in 1..MAX_ATTEMPTS {
            w.cycle(1_000_000 + step * MAX_BACKOFF_MS).unwrap();
        }
        assert_eq!(
            field(
                &db,
                "SELECT status||':'||COALESCE(last_error,'') FROM mobile_push_jobs"
            ),
            "failed:attempts"
        );
        // A permanent refusal terminates without retrying.
        register(&db, 4, DEVICE, TOKEN2, waiting("req-1"));
        commit(&db, 5, running());
        commit(&db, 6, waiting("req-2"));
        let fake = Fake::new(vec![Err(PostError::Permanent("http_status"))]);
        worker(&db, fake.clone()).cycle(5_000_000).unwrap();
        assert_eq!(jobs(&db, Some("failed")).len(), 2);
        assert_eq!(
            fake.requests().len(),
            1,
            "permanent failures are not retried"
        );
        // Job age is bounded: ancient pending work expires without a call.
        db.test_connection(|c| {
            c.execute(
                "UPDATE mobile_push_jobs SET status='pending',attempts=0,next_attempt_at=1,
                 created_at=1,last_error=NULL WHERE status='failed'",
                [],
            )
            .unwrap();
        });
        let fake = Fake::new(vec![]);
        worker(&db, fake.clone()).cycle(2 + MAX_JOB_AGE_MS).unwrap();
        assert_eq!(jobs(&db, Some("cancelled")).len(), 2);
        assert!(
            fake.requests().is_empty(),
            "expired jobs never reach the network"
        );
    }

    #[test]
    fn ticket_rate_limits_retry_and_stale_deactivation_is_fenced() {
        let db = open("push-tickets");
        pending_job(&db);
        let fake = Fake::new(vec![Fake::ok(
            json!([{"status":"error","details":{"error":"MessageRateExceeded"}}]),
        )]);
        worker(&db, fake).cycle(1_000_000).unwrap();
        assert_eq!(jobs(&db, Some("pending")).len(), 1);
        // DeviceNotRegistered on the active generation deactivates and cancels.
        let fake = Fake::new(vec![Fake::ok(
            json!([{"status":"error","details":{"error":"DeviceNotRegistered"}}]),
        )]);
        worker(&db, fake).cycle(2_000_000).unwrap();
        assert_eq!(db.push_status(DEVICE).unwrap()["status"], "deactivated");
        assert_eq!(jobs(&db, Some("cancelled")).len(), 1);
        // A DeviceGone report for a stale generation must not kill the live one.
        register(&db, 4, DEVICE, TOKEN2, waiting("req-1"));
        db.test_connection(|c| {
            c.execute(
                "UPDATE mobile_push_registrations SET generation=generation+1",
                [],
            )
            .unwrap();
        });
        db.push_settle(
            3_000_000,
            "stale-lease",
            &[SendReport {
                seq: pending_job_seq(&db),
                device_id: DEVICE.into(),
                generation: 1,
                outcome: SendOutcome::DeviceGone,
            }],
            &[],
        )
        .unwrap();
        assert_eq!(db.push_status(DEVICE).unwrap()["status"], "active");
    }

    fn pending_job_seq(db: &crate::database::Database) -> i64 {
        db.test_connection(|c| {
            c.query_row("SELECT max(seq) FROM mobile_push_jobs", [], |r| r.get(0))
                .unwrap()
        })
    }

    #[test]
    fn unregister_rotation_and_revocation_fence_future_sends() {
        let db = open("push-fences");
        pending_job(&db);
        db.begin().unwrap();
        let result = db.stage_push_unregistration(DEVICE).unwrap();
        assert_eq!(result["registered"], false);
        db.commit(4, &waiting("req-1"), &json!([]), Some(&receipt(4)), &[])
            .unwrap();
        assert_eq!(jobs(&db, Some("cancelled")).len(), 1);
        assert_eq!(db.push_status(DEVICE).unwrap()["status"], "unregistered");
        assert_eq!(
            field(&db, "SELECT token FROM mobile_push_registrations"),
            ""
        );
        // Rotation fences the old generation's queue.
        register(&db, 5, DEVICE, TOKEN, waiting("req-1"));
        commit(&db, 6, running());
        commit(&db, 7, waiting("req-2"));
        assert_eq!(jobs(&db, Some("pending")).len(), 1);
        register(&db, 8, DEVICE, TOKEN2, waiting("req-2"));
        assert_eq!(jobs(&db, Some("cancelled")).len(), 2);
        // New edges still queue for the live registration.
        commit(&db, 9, running());
        commit(&db, 10, waiting("req-3"));
        assert_eq!(jobs(&db, Some("pending")).len(), 1);
        let generation: i64 = db.test_connection(|c| {
            c.query_row(
                "SELECT generation FROM mobile_push_registrations",
                [],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert_eq!(generation, 3);
        // Revocation cancels sends inside the state commit itself.
        db.revoke(DEVICE).unwrap();
        assert_eq!(jobs(&db, Some("cancelled")).len(), 3);
        assert_eq!(jobs(&db, Some("pending")).len(), 0);
        assert_eq!(db.push_status(DEVICE).unwrap()["status"], "revoked");
        let fake = Fake::new(vec![]);
        worker(&db, fake.clone()).cycle(9_000_000).unwrap();
        assert!(
            fake.requests().is_empty(),
            "revoked devices never reach Expo"
        );
    }

    #[test]
    fn crashed_leases_recover_and_the_same_event_never_double_sends() {
        let directory = directory("push-leases");
        {
            let db = crate::database::Database::open(&directory).unwrap();
            pending_job(&db);
            db.push_prepare(1_000_000).unwrap();
            let claimed = db.push_claim(1_000_000, "dead-worker").unwrap();
            assert_eq!(claimed.jobs.len(), 1);
        }
        // The process is gone; reopen the database like a fresh start.
        let db = crate::database::Database::open(&directory).unwrap();
        let fake = Fake::new(vec![]);
        worker(&db, fake.clone()).cycle(1_000_001).unwrap();
        assert!(
            fake.requests().is_empty(),
            "a held lease is not resent inside its window"
        );
        // After expiry the lease is reclaimed and the event sends once more.
        let fake = Fake::new(vec![Fake::ok(
            json!([{"status":"ok","id":uuid::Uuid::new_v4()}]),
        )]);
        worker(&db, fake.clone())
            .cycle(1_000_000 + LEASE_MS + 1)
            .unwrap();
        assert_eq!(fake.requests().len(), 1);
        assert_eq!(jobs(&db, Some("submitted")).len(), 1);
    }

    #[test]
    fn maintenance_fence_defers_dispatch_without_losing_jobs() {
        let db = open("push-fence");
        pending_job(&db);
        db.fence_dispatch(true);
        let fake = Fake::new(vec![]);
        worker(&db, fake.clone()).cycle(1_000_000).unwrap();
        assert!(
            fake.requests().is_empty(),
            "a fenced server dispatches nothing"
        );
        assert_eq!(jobs(&db, Some("pending")).len(), 1);
        db.fence_dispatch(false);
        let fake = Fake::new(vec![Fake::ok(
            json!([{"status":"ok","id":uuid::Uuid::new_v4()}]),
        )]);
        worker(&db, fake.clone()).cycle(1_000_000).unwrap();
        assert_eq!(fake.requests().len(), 1);
    }

    #[test]
    fn overflow_and_terminal_bounds_keep_the_outbox_finite() {
        let db = open("push-bounds");
        pending_job(&db);
        // A phone that never drains cannot grow pending jobs past the cap.
        for i in 0..300 {
            insert_job(&db, &format!("stale-{i}"), "pending");
        }
        commit(&db, 4, running());
        commit(&db, 5, waiting("req-2"));
        let pending = count(
            &db,
            "SELECT count(*) FROM mobile_push_jobs WHERE status='pending'",
        );
        assert!(pending <= PENDING_PER_DEVICE, "{pending}");
        // Terminal audit is bounded per device.
        for i in 0..300 {
            insert_job(&db, &format!("done-{i}"), "delivered");
        }
        worker(&db, Fake::new(vec![])).cycle(9_000_000).unwrap();
        let terminal = count(
            &db,
            "SELECT count(*) FROM mobile_push_jobs WHERE status='delivered'",
        );
        assert!(terminal <= TERMINAL_KEEP, "{terminal}");
    }

    #[test]
    fn pinned_push_events_do_not_hide_pruned_notification_gaps() {
        let profile = directory("push-retention-gap");
        let db = crate::database::Database::open(&profile).unwrap();
        device(&db, DEVICE);
        commit(&db, 1, running());
        register(&db, 2, DEVICE, TOKEN, running());
        commit(&db, 3, waiting("pinned-request"));
        let pinned = db.notification_page(None, 1).unwrap()["events"][0]["seq"]
            .as_u64()
            .unwrap();
        db.test_connection(|connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            for index in 0..2100 {
                transaction
                    .execute(
                        "INSERT INTO mobile_notification_events
                     (id,kind,thread_id,source_key,created_at)
                     VALUES(?1,'run_completed',1,?2,0)",
                        params![
                            uuid::Uuid::new_v4().to_string(),
                            format!("gap-fill:{index}")
                        ],
                    )
                    .unwrap();
            }
            transaction.commit().unwrap();
        });
        commit(&db, 4, waiting("pinned-request"));
        let page = db.notification_page(Some(pinned + 1), 100).unwrap();
        assert_eq!(
            page["oldestSeq"], pinned,
            "unfinished push keeps its older event"
        );
        assert_eq!(
            page["cursorExpired"], true,
            "the retained oldest row cannot hide a gap"
        );
        let through = db.test_connection(|connection| {
            connection
                .query_row(
                    "SELECT CAST(value AS INTEGER) FROM metadata
                 WHERE key='mobile-notifications-pruned-through'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
        });
        assert_eq!(
            db.notification_page(Some(through as u64), 100).unwrap()["cursorExpired"],
            false
        );
        drop(db);
        let db = crate::database::Database::open(&profile).unwrap();
        assert_eq!(
            db.notification_page(Some(pinned + 1), 100).unwrap()["cursorExpired"],
            true
        );
    }

    #[test]
    fn notification_event_lookup_is_strict_and_token_free() {
        let db = open("push-lookup");
        pending_job(&db);
        assert!(db.notification_event("not-a-uuid").is_err());
        assert!(
            db.notification_event(&uuid::Uuid::new_v4().to_string())
                .is_err(),
            "missing or pruned ids are a clear error"
        );
        let event = db.notification_page(None, 10).unwrap()["events"][0].clone();
        let found = db
            .notification_event(event["id"].as_str().unwrap())
            .unwrap();
        assert_eq!(found["kind"], "approval");
        assert_eq!(found["threadId"], 1);
        assert_eq!(found["requestId"], "req-1");
        assert!(!found.to_string().contains("Approve deletion"));
    }

    #[test]
    fn run_with_sleeps_without_registrations_and_stops_on_shutdown() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let db = open("push-run");
                let fake = Fake::new(vec![]);
                let (stop, shutdown) = tokio::sync::watch::channel(false);
                let handle =
                    tokio::spawn(run_with(db.clone(), SERVER.into(), fake.clone(), shutdown));
                tokio::time::sleep(Duration::from_millis(60)).await;
                assert!(
                    fake.requests().is_empty(),
                    "no opted-in device means no network"
                );
                stop.send_replace(true);
                tokio::time::timeout(Duration::from_secs(5), handle)
                    .await
                    .unwrap()
                    .unwrap();
            });
    }
}
