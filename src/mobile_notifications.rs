//! Durable, privacy-minimized mobile notification events.
//!
//! `Database::commit` derives edges inside its own transaction, so a restart can
//! never see committed thread state the feed cannot explain. Payloads carry only
//! opaque and numeric identifiers: never prompt, title, tool, path or token text.
//! The feed is catch-up storage for suspended user devices. The separate opt-in
//! push outbox protects its unfinished event rows from retention.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Retained feed depth per device catch-up, not a history of record.
pub(crate) const RETAIN_EVENTS: i64 = 2048;
pub(crate) const DEFAULT_LIMIT: usize = 50;
pub(crate) const MAX_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `method:"confirm"` needs a decision on the user's device.
    Approval,
    /// Any other resumable request: select, input, editor, or a newer kind.
    UserInput,
    RunCompleted,
    RunFailed,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Approval => "approval",
            Self::UserInput => "user_input",
            Self::RunCompleted => "run_completed",
            Self::RunFailed => "run_failed",
        }
    }
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "approval" => Self::Approval,
            "user_input" => Self::UserInput,
            "run_completed" => Self::RunCompleted,
            "run_failed" => Self::RunFailed,
            _ => return None,
        })
    }
    fn terminal(status: &str) -> Option<Self> {
        match status {
            "Completed" => Some(Self::RunCompleted),
            "Failed" => Some(Self::RunFailed),
            _ => None,
        }
    }
}

/// One committed edge. `source` is the dedupe identity: server-native run or
/// request ids plus the thread, never prompt or title text.
#[derive(Clone, Debug)]
pub struct Event {
    pub kind: Kind,
    pub thread_id: u64,
    pub run_id: Option<String>,
    /// The raw wire request id so its JSON type survives for `request.respond`.
    pub request_id: Option<Value>,
    pub generation: Option<String>,
    source: String,
}

impl Event {
    /// Wire shape: opaque event id, monotonic sequence, kind, routing ids, time.
    fn to_wire(&self, seq: i64, id: &str, created_at: i64) -> Value {
        let mut event = json!({
            "id": id, "seq": seq, "kind": self.kind.name(),
            "threadId": self.thread_id, "createdAt": created_at,
        });
        if let Some(run) = &self.run_id {
            event["runId"] = json!(run);
        }
        if let Some(request) = &self.request_id {
            event["requestId"] = request.clone();
        }
        if let Some(generation) = &self.generation {
            event["generation"] = json!(generation);
        }
        event
    }
}

/// The notification-relevant slice of one committed thread view.
#[derive(Default)]
pub(crate) struct ThreadState {
    status: Option<String>,
    /// `run_id` for routing. Resumable/continuation ids only fill the identity.
    run_id: Option<String>,
    /// `run_id`, else `resumable_run_id`/`continuation_of_run_id`: the dedupe
    /// identity for the run itself, distinct from the request `generation`.
    run_key: Option<String>,
    /// `run_key`, else the request generation: the dedupe identity for views
    /// that never had a run. A generation alone must not mint a second
    /// completion edge when the session merely reconnects.
    run_identity: Option<String>,
    generation: Option<String>,
    /// Actionable request identities: (JSON-encoded id, runtime generation).
    requests: BTreeSet<(String, String)>,
}

fn run_identity(view: &Value) -> (Option<String>, Option<String>) {
    let key = ["run_id", "resumable_run_id", "continuation_of_run_id"]
        .iter()
        .find_map(|key| {
            view[key]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        });
    let identity = key.clone().or_else(|| {
        view["generation"]
            .as_str()
            .filter(|generation| !generation.is_empty())
            .map(str::to_owned)
    });
    (key, identity)
}

/// An actionable request identity. `approval::can_respond` is the wire contract:
/// `canRespond:false` or `responseCapability:"not_resumable"` cannot be answered
/// from a device, so it never notifies. `method` picks the kind; the runtime
/// generation distinguishes a provider id reused across runtime generations.
fn request_identity(request: &Value, view_generation: &str) -> Option<(String, String)> {
    if !crate::approval::can_respond(request) {
        return None;
    }
    let id = request.get("id").filter(|id| !id.is_null())?;
    Some((
        id.to_string(),
        request["runtimeGeneration"]
            .as_str()
            .unwrap_or(view_generation)
            .to_owned(),
    ))
}

/// `view` is one wire thread view; `approvals` carries `pending_approvals`
/// entries stored as separate rows for views whose arrays were exploded.
fn snapshot(view: &Value, approvals: &[Value]) -> ThreadState {
    let view_generation = view["generation"].as_str().unwrap_or("");
    let mut requests = BTreeSet::new();
    for request in std::iter::once(&view["request"])
        .chain(view["pending_approvals"].as_array().into_iter().flatten())
        .chain(approvals.iter())
    {
        if let Some(identity) = request_identity(request, view_generation) {
            requests.insert(identity);
        }
    }
    let (run_key, run_id_chain) = run_identity(view);
    ThreadState {
        status: view["status"].as_str().map(str::to_owned),
        run_id: view["run_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        run_key,
        run_identity: run_id_chain,
        generation: (!view_generation.is_empty()).then(|| view_generation.to_owned()),
        requests,
    }
}

/// New edges between the last committed thread states and this projection.
/// `workspace["threads"]` holds complete views; `_partial` projections simply
/// contain fewer of them, so untouched threads never compare here.
pub(crate) fn detect(prior: &BTreeMap<u64, ThreadState>, workspace: &Value) -> Vec<Event> {
    let mut events = Vec::new();
    for view in workspace["threads"].as_array().into_iter().flatten() {
        let Some(thread_id) = view["id"].as_u64() else {
            continue;
        };
        let old = prior.get(&thread_id);
        let now = snapshot(view, &[]);
        let view_generation = now.generation.as_deref().unwrap_or("");
        let mut emitted = BTreeSet::new();
        for request in std::iter::once(&view["request"])
            .chain(view["pending_approvals"].as_array().into_iter().flatten())
        {
            let Some((id, generation)) = request_identity(request, view_generation) else {
                continue;
            };
            if !emitted.insert((id.clone(), generation.clone())) {
                continue;
            }
            if old.is_some_and(|old| old.requests.contains(&(id.clone(), generation.clone()))) {
                continue;
            }
            let kind = if request["method"] == "confirm" {
                Kind::Approval
            } else {
                Kind::UserInput
            };
            events.push(Event {
                kind,
                thread_id,
                run_id: None,
                request_id: request.get("id").cloned(),
                generation: Some(generation.clone()),
                source: format!("request:{thread_id}:{id}:{generation}"),
            });
        }
        // A run that ends between two committed views. `Ready` (cancelled) is not
        // terminal, and a thread that was never committed is materialization,
        // not an edge: historical completions must not notify.
        if let (Some(old), Some(kind)) = (old, Kind::terminal(now.status.as_deref().unwrap_or("")))
            && (old.status != now.status
                || (now.run_key.is_some() && old.run_identity != now.run_identity))
        {
            events.push(Event {
                kind,
                thread_id,
                run_id: now.run_id.clone(),
                request_id: None,
                generation: now.generation.clone(),
                source: format!(
                    "run:{thread_id}:{}:{}",
                    now.run_identity.as_deref().unwrap_or(""),
                    kind.name()
                ),
            });
        }
    }
    events
}

/// The committed baseline: each `threads/*` view header plus its exploded
/// `pending_approvals` rows, never message or tool payloads. Only the scopes
/// the incoming projection actually carries are read, so partial commits do
/// not rescan every thread header.
fn prior_threads(
    connection: &Connection,
    ids: &[u64],
) -> Result<BTreeMap<u64, ThreadState>, String> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "SELECT scope,key,payload,schema FROM projection_records
         WHERE document='workspace' AND scope IN ({placeholders})
           AND (key='$' OR key GLOB 'pending_approvals/*')"
    );
    let scopes: Vec<String> = ids.iter().map(|id| format!("threads/{id}")).collect();
    let mut statement = connection.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(scopes.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut bodies = BTreeMap::new();
    let mut approvals: BTreeMap<u64, Vec<Value>> = BTreeMap::new();
    for (scope, key, text, schema) in rows {
        if schema != 1 {
            return Err("Projection row requires a newer Shina binary.".into());
        }
        let Some(id) = scope
            .strip_prefix("threads/")
            .and_then(|id| id.parse::<u64>().ok())
        else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if key == "$" {
            bodies.insert(id, value);
        } else if let Some(item) = value.get("item") {
            approvals.entry(id).or_default().push(item.clone());
        }
    }
    Ok(bodies
        .into_iter()
        .map(|(id, body)| {
            (
                id,
                snapshot(&body, approvals.get(&id).map(Vec::as_slice).unwrap_or(&[])),
            )
        })
        .collect())
}

/// Called by `Database::commit` inside the state transaction, before the new
/// `workspace` document replaces the committed one it is compared against.
/// `INSERT OR IGNORE` on `source_key` makes restarts and replayed edges
/// idempotent inside the retained window. Each new event also creates one
/// durable push outbox job per opted-in device in the same transaction.
pub(crate) fn record(connection: &Connection, workspace: &Value) -> Result<(), String> {
    // The first committed workspace has no baseline. Materializing it must not
    // report old completions or long-pending requests as if they just happened.
    let committed: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projection_records WHERE document='workspace')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !committed {
        return Ok(());
    }
    let ids: Vec<u64> = workspace["threads"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|view| view["id"].as_u64())
        .collect();
    let prior = prior_threads(connection, &ids)?;
    let now = crate::lifecycle::now() as i64;
    let mut emitted = Vec::new();
    for event in detect(&prior, workspace) {
        connection
            .execute(
                "INSERT OR IGNORE INTO mobile_notification_events
                 (id,kind,thread_id,run_id,request_id,generation,source_key,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    event.kind.name(),
                    event.thread_id as i64,
                    event.run_id,
                    event.request_id.as_ref().map(Value::to_string),
                    event.generation,
                    event.source,
                    now,
                ],
            )
            .map_err(|e| e.to_string())?;
        if connection.changes() == 0 {
            // A replayed edge keeps its first id and seq, and an event that
            // predates a registration never becomes a push job later.
            continue;
        }
        let stored: (i64, String) = connection
            .query_row(
                "SELECT seq,id FROM mobile_notification_events WHERE source_key=?1",
                [&event.source],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        emitted.push((stored.0, stored.1, event.kind.name()));
    }
    for (seq, id, kind) in emitted {
        crate::mobile_push::enqueue(connection, seq, &id, kind, now)?;
    }
    crate::mobile_push::bound_global(connection, now)?;
    // Events whose only jobs were just fenced count as settled for retention.
    crate::mobile_push::settle_events(connection, now)?;
    // Bound catch-up storage to the newest events. Rows the push outbox marked
    // but has not settled are exempt from the count window.
    let floor: i64 = connection
        .query_row(
            "SELECT COALESCE(MIN(seq),0) FROM (SELECT seq FROM mobile_notification_events
             ORDER BY seq DESC LIMIT ?1)",
            [RETAIN_EVENTS],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let removed: Option<i64> = connection
        .query_row(
            "SELECT MAX(seq) FROM mobile_notification_events WHERE seq<?1
             AND NOT (push_required=1 AND delivered_at IS NULL)",
            [floor],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if let Some(removed) = removed {
        let through = removed.max(pruned_through(connection)?.unwrap_or(0));
        connection
            .execute(
                "INSERT INTO metadata(key,value) VALUES('mobile-notifications-pruned-through',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [through.to_string()],
            )
            .map_err(|e| e.to_string())?;
        connection
            .execute(
                "DELETE FROM mobile_notification_events WHERE seq<?1
                 AND NOT (push_required=1 AND delivered_at IS NULL)",
                [floor],
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn pruned_through(connection: &Connection) -> Result<Option<i64>, String> {
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM metadata WHERE key='mobile-notifications-pruned-through'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    value
        .map(|text| {
            text.parse::<i64>()
                .ok()
                .filter(|sequence| *sequence >= 0)
                .ok_or_else(|| "Invalid notification retention marker.".to_owned())
        })
        .transpose()
}

/// A bounded page of events after `after` (exclusive), oldest first.
/// `after:None` reads from the start. Cursors are `seq` values: one beyond the
/// feed's AUTOINCREMENT high-water mark is impossible, one below the retained
/// floor only means the missed range was pruned, which `cursorExpired` reports.
pub(crate) fn page(
    connection: &Connection,
    after: Option<u64>,
    limit: usize,
) -> Result<Value, String> {
    let after = after.unwrap_or(0);
    if after > i64::MAX as u64 {
        return Err("Invalid notification cursor.".into());
    }
    let limit = if limit == 0 {
        DEFAULT_LIMIT
    } else {
        limit.min(MAX_LIMIT)
    };
    let (oldest, highest): (Option<i64>, Option<i64>) = connection
        .query_row(
            "SELECT MIN(seq),MAX(seq) FROM mobile_notification_events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    // A cursor above every live row can never have been issued: dedupe
    // `INSERT OR IGNORE` still burns AUTOINCREMENT seqs, so `sqlite_sequence`
    // is only the ceiling once the table itself was pruned to empty.
    let ceiling = match highest {
        Some(high) => high,
        None => connection
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='mobile_notification_events'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or(0),
    };
    if after as i64 > ceiling {
        return Err("Invalid notification cursor.".into());
    }
    // A pending push may preserve a very old row below a pruned range. MIN(seq)
    // alone cannot report that gap. Older databases without the marker retain
    // the previous floor-based fallback.
    let pruned = pruned_through(connection)?
        .unwrap_or_else(|| oldest.map(|sequence| sequence - 1).unwrap_or(ceiling));
    let mut statement = connection
        .prepare(
            "SELECT seq,id,schema,kind,thread_id,run_id,request_id,generation,created_at
             FROM mobile_notification_events WHERE seq>?1 ORDER BY seq LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = statement
        .query_map(params![after as i64, limit as i64 + 1], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, i64>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next = has_more.then(|| rows.last().map(|row| row.0)).flatten();
    let mut events = Vec::new();
    for (seq, id, schema, kind, thread_id, run_id, request_id, generation, created_at) in rows {
        if schema != 1 {
            return Err("Notification events require a newer Shina binary.".into());
        }
        let event = Event {
            kind: Kind::from_name(&kind).ok_or("Unknown notification event kind.")?,
            thread_id: thread_id as u64,
            run_id,
            request_id: request_id
                .map(|text| serde_json::from_str(&text).unwrap_or(Value::String(text))),
            generation,
            source: String::new(),
        };
        events.push(event.to_wire(seq, &id, created_at));
    }
    Ok(json!({
        "events": events,
        "nextCursor": next.map(Value::from).unwrap_or(Value::Null),
        "hasMore": has_more,
        "cursorExpired": after > 0 && (after as i64) < pruned,
        "oldestSeq": oldest,
    }))
}

/// One event by its opaque id, for push-tap navigation (`notifications.get`).
/// A missing or pruned id is a clear error, never fabricated data.
pub(crate) fn get(connection: &Connection, event_id: &str) -> Result<Value, String> {
    let parsed =
        uuid::Uuid::parse_str(event_id).map_err(|_| "Invalid notification event id.".to_owned())?;
    type EventRow = (
        i64,
        String,
        u32,
        String,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    );
    let row: Option<EventRow> = connection
        .query_row(
            "SELECT seq,id,schema,kind,thread_id,run_id,request_id,generation,created_at
             FROM mobile_notification_events WHERE id=?1",
            [parsed.to_string()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (seq, id, schema, kind, thread_id, run_id, request_id, generation, created_at) =
        row.ok_or("Notification event is no longer available.")?;
    if schema != 1 {
        return Err("Notification events require a newer Shina binary.".into());
    }
    let event = Event {
        kind: Kind::from_name(&kind).ok_or("Unknown notification event kind.")?,
        thread_id: thread_id as u64,
        run_id,
        request_id: request_id
            .map(|text| serde_json::from_str(&text).unwrap_or(Value::String(text))),
        generation,
        source: String::new(),
    };
    Ok(event.to_wire(seq, &id, created_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(id: u64, status: &str) -> Value {
        json!({"id":id,"status":status,"generation":format!("gen-{id}")})
    }

    fn committed(threads: Vec<Value>) -> Value {
        json!({"threads":threads})
    }

    fn prior_of(workspace: &Value) -> BTreeMap<u64, ThreadState> {
        workspace["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|view| (view["id"].as_u64().unwrap(), snapshot(view, &[])))
            .collect()
    }

    fn kinds(events: &[Event]) -> Vec<Kind> {
        events.iter().map(|event| event.kind).collect()
    }

    fn in_memory() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE documents(name TEXT PRIMARY KEY,payload TEXT);
             CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);",
        )
        .unwrap();
        db.execute_batch(
            "CREATE TABLE devices(id TEXT PRIMARY KEY,token_hash TEXT NOT NULL UNIQUE,
             created_at INTEGER NOT NULL,revoked_at INTEGER);",
        )
        .unwrap();
        db.execute_batch(include_str!("migrations/005.sql"))
            .unwrap();
        db.execute_batch(include_str!("migrations/006.sql"))
            .unwrap();
        db.execute_batch(include_str!("migrations/007.sql"))
            .unwrap();
        db
    }

    #[test]
    fn approval_and_input_requests_emit_once_and_dedupe_by_source_key() {
        let mut running = view(1, "Running");
        running["request"] = json!({"id":"req-1","method":"select","options":["a","b"],
            "title":"Pick a secret option","prefill":"typed answer"});
        running["pending_approvals"] = json!([{"id":"perm-1","method":"confirm",
            "title":"Run rm -rf /","tool":"shell"}]);
        let baseline = committed(vec![view(1, "Running"), running.clone()]);
        let mut prior = prior_of(&baseline);
        assert!(detect(&prior, &baseline).is_empty(), "no baseline edges");
        let detected = detect(&prior_of(&committed(vec![view(1, "Running")])), &baseline);
        assert_eq!(kinds(&detected), [Kind::UserInput, Kind::Approval]);
        assert_eq!(detected[0].request_id, Some(json!("req-1")));
        assert_eq!(detected[0].generation.as_deref(), Some("gen-1"));
        // An identical later commit detects nothing: prior covers both requests.
        prior = prior_of(&baseline);
        assert!(detect(&prior, &baseline).is_empty());
    }

    #[test]
    fn a_reused_provider_id_in_a_new_runtime_generation_is_a_distinct_request() {
        let mut old = view(1, "Waiting");
        old["request"] = json!({"id":"req-1","method":"confirm","runtimeGeneration":"g1"});
        let prior = prior_of(&committed(vec![old]));
        let mut incoming = view(1, "Waiting");
        incoming["request"] = json!({"id":"req-1","method":"confirm","runtimeGeneration":"g2"});
        let events = detect(&prior, &committed(vec![incoming]));
        assert_eq!(kinds(&events), [Kind::Approval]);
        assert_eq!(events[0].generation.as_deref(), Some("g2"));
        assert_ne!(events[0].source, format!("request:1:{}:g1", json!("req-1")));
    }

    #[test]
    fn completed_and_failed_runs_emit_but_cancellation_and_ready_do_not() {
        let mut running = view(1, "Running");
        running["run_id"] = json!("run-a");
        let prior = prior_of(&committed(vec![running.clone()]));
        for (status, expected) in [
            ("Completed", Some(Kind::RunCompleted)),
            ("Failed", Some(Kind::RunFailed)),
            ("Ready", None),
            ("Paused", None),
        ] {
            let mut incoming = running.clone();
            incoming["status"] = json!(status);
            let events = detect(&prior, &committed(vec![incoming]));
            assert_eq!(
                kinds(&events),
                expected.into_iter().collect::<Vec<_>>(),
                "{status}"
            );
        }
        // The same terminal state committed twice emits nothing.
        let mut done = running.clone();
        done["status"] = json!("Completed");
        let prior = prior_of(&committed(vec![done.clone()]));
        assert!(detect(&prior, &committed(vec![done.clone()])).is_empty());
        // A view without run fields only has the request generation as
        // identity; a session reconnect that rotates it is not a new run.
        let mut idle = view(9, "Completed");
        idle["generation"] = json!("gen-old");
        let idle_prior = prior_of(&committed(vec![idle.clone()]));
        idle["generation"] = json!("gen-new");
        assert!(detect(&idle_prior, &committed(vec![idle])).is_empty());
        // A next run finishing inside one commit still edges on its new run id:
        // same terminal status, different run identity.
        let mut again = done;
        again["run_id"] = json!("run-b");
        let events = detect(&prior, &committed(vec![again]));
        assert_eq!(kinds(&events), [Kind::RunCompleted]);
        assert_eq!(events[0].run_id.as_deref(), Some("run-b"));
    }

    #[test]
    fn partial_projections_compare_only_the_threads_they_carry() {
        let mut one = view(1, "Running");
        one["run_id"] = json!("r1");
        let mut two = view(2, "Waiting");
        two["request"] = json!({"id":"q","method":"input","runtimeGeneration":"gen-2"});
        let prior = prior_of(&committed(vec![one.clone(), two]));
        one["status"] = json!("Completed");
        let partial = json!({"_partial":true,"index":{},"threads":[one]});
        let events = detect(&prior, &partial);
        assert_eq!(kinds(&events), [Kind::RunCompleted]);
        assert_eq!(events[0].thread_id, 1);
    }

    #[test]
    fn snapshot_materialization_never_reports_old_completions() {
        // A thread that was never committed is not an edge for its terminal
        // state, but a genuinely new pending request on it is actionable now.
        let mut done = view(7, "Completed");
        done["run_id"] = json!("old-run");
        let mut waiting = view(8, "Waiting");
        waiting["request"] = json!({"id":"now","method":"confirm"});
        let events = detect(&BTreeMap::new(), &committed(vec![done, waiting]));
        assert_eq!(kinds(&events), [Kind::Approval]);
        assert_eq!(events[0].thread_id, 8);
    }

    #[test]
    fn non_actionable_requests_never_emit_even_when_new() {
        for request in [
            json!({"id":"x","method":"confirm","canRespond":false}),
            json!({"id":"y","method":"confirm","responseCapability":{"type":"not_resumable"}}),
            json!({"id":"z","method":"select","responseCapability":"not_resumable"}),
            json!({"method":"confirm"}),
        ] {
            let mut incoming = view(1, "Waiting");
            incoming["request"] = request.clone();
            assert!(
                detect(&BTreeMap::new(), &committed(vec![incoming])).is_empty(),
                "{request}"
            );
        }
    }

    #[test]
    fn request_content_never_reaches_event_payloads_or_dedupe_keys() {
        let mut waiting = view(1, "Waiting");
        waiting["request"] = json!({"id":"req","method":"confirm","title":"Delete /Users/secret",
            "tool":{"command":"rm -rf ~"},"prompt":"user prompt text","token":"abc123"});
        let mut done = view(2, "Completed");
        done["run_id"] = json!("r2");
        done["title"] = json!("a private title");
        let events = detect(&BTreeMap::new(), &committed(vec![waiting, done]));
        let text = json!(
            events
                .iter()
                .map(|event| event.to_wire(1, "e", 2))
                .collect::<Vec<_>>()
        )
        .to_string();
        let sources = events
            .iter()
            .map(|event| event.source.as_str())
            .collect::<String>();
        for forbidden in [
            "Delete /Users/secret",
            "rm -rf ~",
            "user prompt text",
            "abc123",
            "a private title",
        ] {
            assert!(
                !text.contains(forbidden) && !sources.contains(forbidden),
                "{forbidden}"
            );
        }
        let event = events[0].to_wire(9, "opaque-id", 42);
        let mut keys: Vec<_> = event.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "createdAt",
                "generation",
                "id",
                "kind",
                "requestId",
                "seq",
                "threadId"
            ]
        );
    }

    #[test]
    fn record_feed_paging_and_pruning_share_the_projection_store() {
        let db = in_memory();
        crate::projection_store::write(&db, "workspace", &committed(vec![view(1, "Running")]))
            .unwrap();
        // First commit after the baseline exists: a new actionable request.
        let mut incoming = view(1, "Waiting");
        incoming["run_id"] = json!("r1");
        incoming["request"] = json!({"id":"req-1","method":"confirm","runtimeGeneration":"g1"});
        record(&db, &committed(vec![incoming.clone()])).unwrap();
        // The same edge replayed before the workspace write dedupes on source_key.
        record(&db, &committed(vec![incoming.clone()])).unwrap();
        // After the view commits, the identical next commit detects nothing.
        crate::projection_store::write(&db, "workspace", &committed(vec![incoming])).unwrap();
        record(&db, &committed(vec![view(1, "Waiting")])).unwrap();
        crate::projection_store::write(&db, "workspace", &committed(vec![view(1, "Waiting")]))
            .unwrap();
        let feed = page(&db, None, 0).unwrap();
        assert_eq!(feed["events"].as_array().unwrap().len(), 1);
        assert_eq!(feed["events"][0]["kind"], "approval");
        assert_eq!(feed["events"][0]["requestId"], "req-1");
        assert_eq!(feed["nextCursor"], Value::Null);
        assert_eq!(feed["hasMore"], false);
        assert_eq!(feed["cursorExpired"], false);
        assert_eq!(feed["oldestSeq"], 1);
        assert!(page(&db, Some(u64::MAX), 10).is_err());
        assert!(
            page(&db, Some(2), 10).is_err(),
            "beyond the high-water mark"
        );
        // Pruning keeps the newest rows and marks pruned cursors, never reuse.
        for i in 0..2100 {
            db.execute(
                "INSERT INTO mobile_notification_events(id,kind,thread_id,source_key,created_at)
                 VALUES(?1,'run_completed',1,?2,?3)",
                params![format!("fill-{i}"), format!("fill:{i}"), i],
            )
            .unwrap();
        }
        record(&db, &committed(vec![view(1, "Waiting")])).unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM mobile_notification_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, RETAIN_EVENTS);
        let last: i64 = db
            .query_row("SELECT MAX(seq) FROM mobile_notification_events", [], |r| {
                r.get(0)
            })
            .unwrap();
        let feed = page(&db, Some(1), 10).unwrap();
        assert_eq!(feed["cursorExpired"], true);
        assert_eq!(feed["oldestSeq"], last - RETAIN_EVENTS + 1);
        // The exclusive cursor immediately before the retained floor has no
        // gap; one earlier still does.
        let floor = (last - RETAIN_EVENTS) as u64;
        let feed = page(&db, Some(floor), 10).unwrap();
        assert_eq!(feed["cursorExpired"], false);
        assert_eq!(feed["events"][0]["seq"], floor + 1);
        let feed = page(&db, Some(floor - 1), 10).unwrap();
        assert_eq!(feed["cursorExpired"], true);
        db.execute("DELETE FROM mobile_notification_events", [])
            .unwrap();
        db.execute(
            "INSERT INTO mobile_notification_events(id,kind,thread_id,source_key,created_at)
             VALUES('after-prune','run_failed',1,'after-prune',0)",
            [],
        )
        .unwrap();
        let seq: i64 = db
            .query_row(
                "SELECT seq FROM mobile_notification_events WHERE id='after-prune'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(seq > last, "AUTOINCREMENT never reuses a pruned seq");
        let feed = page(&db, Some(last as u64), 10).unwrap();
        assert_eq!(feed["events"][0]["id"], "after-prune");
        let empty = page(&db, None, 5).unwrap();
        assert_eq!(empty["events"].as_array().unwrap().len(), 1);
        db.execute("DELETE FROM mobile_notification_events", [])
            .unwrap();
        let empty = page(&db, None, 5).unwrap();
        assert_eq!(empty["events"], json!([]));
        assert_eq!(empty["hasMore"], false);
        assert_eq!(empty["nextCursor"], Value::Null);
        assert_eq!(empty["cursorExpired"], false);
        assert_eq!(empty["oldestSeq"], Value::Null);
    }
}
