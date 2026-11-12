//! KSG's scoped history windows, item identities, sequence positions and bounded replay.
//! Only byte fragmentation uses a lease. KSG admits an oversized row, but Shina's
//! 64 KiB data budget must also hold for a single oversized message.
use crate::protocol;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

pub const LEASE_COUNT: usize = 4;
pub const LEASE_BYTES: usize = 128 * 1024 * 1024;
pub const LEASE_IDLE: Duration = Duration::from_secs(30);
pub const REPLAY_EVENTS: usize = 128;
pub const REPLAY_BYTES: usize = 1024 * 1024;
const RETAIN_EVENTS: usize = 1024;
const RETAIN_BYTES: usize = 4 * 1024 * 1024;
const PAGE_ITEMS: usize = 75;
const PAGE_USER_TURNS_INITIAL: usize = 10;
const PAGE_USER_TURNS_OLDER: usize = 20;
const PAGE_TURN_ITEMS: usize = 500;
const PAGE_RAW_TURNS: usize = 150;
#[cfg(test)]
pub const PAGE_MAX_ENCODED_BYTES: usize = 1_048_576;

fn is_timeline_field(field: &str) -> bool {
    matches!(
        field,
        "messages" | "tools" | "thoughts" | "work" | "compactions"
    )
}

fn is_user_turn_row(row: &Row) -> bool {
    row.field == "messages" && matches!(row.item["role"].as_str(), Some("You" | "user"))
}

fn timeline_sort_key(row: &Row, total_messages: usize) -> (usize, u8, u64, usize, usize) {
    match row.field.as_str() {
        "work" => (
            row.item["start"].as_u64().map(|m| m as usize).unwrap_or(0),
            0,
            row.item["stopped_order"].as_u64().unwrap_or(0),
            row.ordinal,
            row.position,
        ),
        "thoughts" => (
            row.item["anchor"]["message"]
                .as_u64()
                .map(|m| m as usize)
                .unwrap_or(0),
            1,
            row.item["order"].as_u64().unwrap_or(row.ordinal as u64),
            row.ordinal,
            row.position,
        ),
        "tools" => (
            row.item["anchor"]["message"]
                .as_u64()
                .map(|m| m as usize)
                .unwrap_or(0),
            2,
            row.item["order"].as_u64().unwrap_or(row.ordinal as u64),
            row.ordinal,
            row.position,
        ),
        "compactions" => (
            row.item["anchor"]["message"]
                .as_u64()
                .map(|m| m as usize)
                .unwrap_or(total_messages),
            3,
            row.item["order"].as_u64().unwrap_or(row.ordinal as u64),
            row.ordinal,
            row.position,
        ),
        "messages" => (row.ordinal, 4, 0, row.ordinal, row.position),
        _ => (usize::MAX, 5, 0, row.ordinal, row.position),
    }
}

/// KSG stores the identity and sequence beside each projected turn item.
/// Match known records before allocating IDs, including adapter history refreshes.
/// The resulting metadata is persisted in the same item row as its payload.
pub fn stamp(before: &Value, after: &mut Value, sequence: u64) {
    let mut identities = json!({});
    let mut changed = false;
    let intrinsic = |field: &str, item: &Value| {
        item.get("id").filter(|id| !id.is_null()).map(|id| {
            format!(
                "{field}:{}",
                id.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| id.to_string())
            )
        })
    };
    // Ported from KSG TurnItemPositionStore.allocate. Allocation is idempotent
    // by item identity, and deleted positions are never reused.
    let mut next = before["historyNextPosition"].as_u64().unwrap_or(0);
    for field in protocol::HISTORY_FIELDS {
        for item in before["_history"][field].as_array().into_iter().flatten() {
            if let Some(position) = item["position"].as_u64() {
                next = next.max(
                    position
                        .checked_add(1)
                        .expect("History position exhausted."),
                );
            }
        }
    }
    let mut positions = HashMap::new();
    for field in protocol::HISTORY_FIELDS {
        for (ordinal, item) in before[field].as_array().into_iter().flatten().enumerate() {
            let stored = &before["_history"][field][ordinal];
            let id = stored["id"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| intrinsic(field, item));
            if let Some(id) = id {
                let position = stored["position"].as_u64().unwrap_or_else(|| {
                    let position = next;
                    next = next.checked_add(1).expect("History position exhausted.");
                    position
                });
                positions.insert(id, position);
            }
        }
    }
    for field in protocol::HISTORY_FIELDS {
        let Some(items) = after[field].as_array() else {
            changed |= before[field] != after[field];
            continue;
        };
        let old = before[field].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let mut used = HashSet::new();
        let mut issued = HashSet::new();
        let mut rows = Vec::new();
        for (ordinal, item) in items.iter().enumerate() {
            let exact = old
                .get(ordinal)
                .filter(|old| !used.contains(&ordinal) && *old == item)
                .map(|_| ordinal)
                .or_else(|| {
                    old.iter()
                        .enumerate()
                        .find(|(i, old)| !used.contains(i) && *old == item)
                        .map(|(i, _)| i)
                });
            let matched = exact.or_else(|| {
                old.get(ordinal)
                    .filter(|old| {
                        !used.contains(&ordinal)
                            && old.get("id") == item.get("id")
                            && old.get("role") == item.get("role")
                    })
                    .map(|_| ordinal)
            });
            let stored_identity = matched.and_then(|index| {
                used.insert(index);
                before["_history"][field][index]["id"]
                    .as_str()
                    .map(str::to_owned)
            });
            let native_identity = intrinsic(field, item);
            let candidates = if field == "messages" {
                [native_identity, stored_identity]
            } else {
                [stored_identity, native_identity]
            };
            let identity = candidates
                .into_iter()
                .flatten()
                .find(|id| !issued.contains(id))
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            issued.insert(identity.clone());
            let unchanged = matched.is_some_and(|i| {
                i == ordinal && old[i] == *item && before["_history"][field][i]["id"] == identity
            });
            let position = if unchanged {
                matched
                    .and_then(|i| before["_history"][field][i]["sequence"].as_u64())
                    .unwrap_or(sequence)
            } else {
                sequence
            };
            changed |= !unchanged;
            let ordinal_position = positions.get(&identity).copied().unwrap_or_else(|| {
                let position = next;
                next = next.checked_add(1).expect("History position exhausted.");
                position
            });
            rows.push(json!({"id":identity,"sequence":position,"position":ordinal_position}));
        }
        changed |= old.len() != items.len();
        identities[field] = json!(rows);
    }
    after["_history"] = identities;
    after["historyNextPosition"] = json!(next);
    let high = if changed {
        sequence
    } else {
        before["historySequence"].as_u64().unwrap_or(sequence)
    };
    after["historySequence"] = json!(high);
    after["historyRevision"] = json!(high.to_string());
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub source_thread_id: u64,
    pub source_item_id: String,
    pub field: String,
    /// Ordinal within Shina's existing field, not a timestamp or message hash.
    pub ordinal: usize,
    /// Durable per-thread allocation. Removal or insertion never renumbers items.
    pub position: usize,
    pub item: Value,
}

struct Projection {
    sequence: u64,
    rows: Vec<Row>,
    fields: Value,
    revision: String,
}

struct Event {
    thread: u64,
    sequence: u64,
    value: Value,
    bytes: usize,
    count: usize,
}

/// In-memory indexes rebuilt from durable item rows and committed replay batches.
#[derive(Default)]
pub struct Streams {
    threads: HashMap<u64, Projection>,
    events: VecDeque<Event>,
    pruned: HashMap<u64, u64>,
    bytes: usize,
    next_id: u64,
    preparing_commit: bool,
}

impl Streams {
    /// A transaction's publication batch is independent of replay retention.
    pub fn changes(before: &Value, after: &Value) -> Vec<Value> {
        let mut streams = Self::new(before);
        streams.preparing_commit = true;
        streams.publish(after);
        streams.events.into_iter().map(|e| e.value).collect()
    }
    pub fn new(snapshot: &Value) -> Self {
        let mut streams = Self::default();
        streams.publish(snapshot);
        streams.events.clear();
        streams.bytes = 0;
        streams
    }

    /// Ported from KSG Orchestrator's turn-item.updated publication after commit.
    pub fn publish(&mut self, snapshot: &Value) {
        let sequence = snapshot["seq"].as_u64().unwrap_or(0);
        let Some(threads) = snapshot["state"]["threads"].as_array() else {
            return;
        };
        let mut retained = HashSet::new();
        for thread in threads {
            let Some(id) = thread["id"].as_u64() else {
                continue;
            };
            retained.insert(id);
            let revision = thread["historyRevision"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| protocol::revision(&protocol::history_payload(thread)));
            if self
                .threads
                .get(&id)
                .is_some_and(|old| old.revision == revision)
            {
                continue;
            }
            let before = self.threads.remove(&id);
            let history = protocol::history_payload(thread);
            let mut rows = Vec::new();
            let mut fields = json!({"id":id});
            for field in protocol::HISTORY_FIELDS {
                let Some(items) = history[field].as_array() else {
                    fields[field] = history[field].clone();
                    continue;
                };
                let old: Vec<_> = before
                    .as_ref()
                    .map(|old| old.rows.iter().filter(|row| row.field == field).collect())
                    .unwrap_or_default();
                // Unversioned projections and standalone fixtures retain the old
                // matching fallback. Runtime projections supply stored identities.
                let mut used = HashSet::new();
                let mut exact: HashMap<String, VecDeque<&Row>> = HashMap::new();
                let durable = thread["_history"][field].is_array();
                for row in old.iter().filter(|_| !durable) {
                    exact
                        .entry(row.item.to_string())
                        .or_default()
                        .push_back(row);
                }
                let mut identities = Vec::new();
                for item in items {
                    let found = if durable {
                        None
                    } else {
                        exact
                            .get_mut(&item.to_string())
                            .and_then(VecDeque::pop_front)
                    };
                    if let Some(row) = found {
                        used.insert(row.source_item_id.clone());
                    }
                    identities.push(found.map(|row| row.source_item_id.clone()));
                }
                for (ordinal, item) in items.iter().enumerate() {
                    let identity = thread["_history"][field][ordinal]["id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            identities[ordinal]
                                .take()
                                .or_else(|| {
                                    let intrinsic = item.get("id").filter(|id| !id.is_null());
                                    let candidate = if let Some(intrinsic) = intrinsic {
                                        old.iter()
                                            .find(|row| row.item.get("id") == Some(intrinsic))
                                            .copied()
                                    } else {
                                        old.get(ordinal)
                                            .copied()
                                            .filter(|row| row.item["role"] == item["role"])
                                    };
                                    candidate
                                        .filter(|row| !used.contains(&row.source_item_id))
                                        .map(|row| row.source_item_id.clone())
                                })
                                .unwrap_or_else(|| {
                                    self.next_id += 1;
                                    format!("{field}:{}", self.next_id)
                                })
                        });
                    used.insert(identity.clone());
                    rows.push(Row {
                        source_thread_id: id,
                        source_item_id: identity,
                        field: field.into(),
                        ordinal,
                        position: thread["_history"][field][ordinal]["position"]
                            .as_u64()
                            .map(|p| p as usize)
                            .unwrap_or(rows.len()),
                        item: item.clone(),
                    });
                }
            }
            rows.sort_by_key(|row| row.position);
            let mut changes = Vec::new();
            if let Some(before) = &before {
                let old: HashMap<_, _> = before
                    .rows
                    .iter()
                    .map(|row| (&row.source_item_id, row))
                    .collect();
                let ids: HashSet<_> = rows.iter().map(|row| &row.source_item_id).collect();
                for row in &before.rows {
                    if !ids.contains(&row.source_item_id) {
                        changes
                            .push(json!({"type":"item.removed","sourceItemId":row.source_item_id}));
                    }
                }
                for row in &rows {
                    if old.get(&row.source_item_id).is_none_or(|old| *old != row) {
                        changes.push(json!({"type":"item.updated","row":row}));
                    }
                }
                if before.fields != fields {
                    changes.push(json!({"type":"fields.updated","fields":fields}));
                }
            } else {
                for row in &rows {
                    changes.push(json!({"type":"item.updated","row":row}));
                }
                changes.push(json!({"type":"fields.updated","fields":fields}));
            }
            if !changes.is_empty() {
                let count = changes.len();
                let value = json!({"kind":"event","threadId":id,"sequence":sequence,
                    "historyRevision":revision,"events":changes});
                let bytes = value.to_string().len();
                self.bytes += bytes;
                self.events.push_back(Event {
                    thread: id,
                    sequence: thread["historySequence"].as_u64().unwrap_or(sequence),
                    value,
                    bytes,
                    count,
                });
            }
            self.threads.insert(
                id,
                Projection {
                    sequence: thread["historySequence"].as_u64().unwrap_or(sequence),
                    rows,
                    fields,
                    revision,
                },
            );
        }
        let deleted: Vec<_> = self
            .threads
            .keys()
            .filter(|id| !retained.contains(id))
            .copied()
            .collect();
        for id in deleted {
            self.threads.remove(&id);
            let value = json!({"kind":"deleted","threadId":id,"sequence":sequence});
            let bytes = value.to_string().len();
            self.bytes += bytes;
            self.events.push_back(Event {
                thread: id,
                sequence,
                value,
                bytes,
                count: 1,
            });
        }
        while !self.preparing_commit
            && (self.events.len() > RETAIN_EVENTS || self.bytes > RETAIN_BYTES)
        {
            let event = self.events.pop_front().expect("replay budget");
            self.bytes -= event.bytes;
            self.pruned.insert(event.thread, event.sequence);
        }
    }

    pub fn restore(&mut self, events: Vec<Value>) {
        self.events.clear();
        self.bytes = 0;
        for (id, projection) in &self.threads {
            self.pruned.insert(*id, projection.sequence);
        }
        for value in events {
            let thread = value["threadId"].as_u64().unwrap();
            let sequence = value["sequence"].as_u64().unwrap();
            self.pruned
                .entry(thread)
                .and_modify(|p| *p = (*p).min(sequence.saturating_sub(1)));
            let bytes = value.to_string().len();
            self.bytes += bytes;
            let count = value["events"].as_array().map(Vec::len).unwrap_or(1);
            self.events.push_back(Event {
                thread,
                sequence,
                value,
                bytes,
                count,
            });
        }
        while self.events.len() > RETAIN_EVENTS || self.bytes > RETAIN_BYTES {
            let event = self.events.pop_front().unwrap();
            self.bytes -= event.bytes;
            self.pruned.insert(event.thread, event.sequence);
        }
    }

    /// Publish persisted deltas without inspecting inactive thread histories.
    pub fn ingest(&mut self, events: Vec<Value>) -> Result<(), String> {
        for value in events {
            let thread = value["threadId"]
                .as_u64()
                .ok_or("Invalid committed thread.")?;
            let sequence = value["sequence"]
                .as_u64()
                .ok_or("Invalid committed sequence.")?;
            if value["kind"] == "deleted" {
                self.threads.remove(&thread);
            } else {
                let projection = self.threads.entry(thread).or_insert_with(|| Projection {
                    sequence: 0,
                    rows: vec![],
                    fields: json!({"id":thread}),
                    revision: String::new(),
                });
                if sequence <= projection.sequence {
                    continue;
                }
                for event in value["events"]
                    .as_array()
                    .ok_or("Invalid committed events.")?
                {
                    match event["type"].as_str() {
                        Some("item.updated") => {
                            let row: Row = serde_json::from_value(event["row"].clone())
                                .map_err(|e| e.to_string())?;
                            if let Some(old) = projection
                                .rows
                                .iter_mut()
                                .find(|r| r.source_item_id == row.source_item_id)
                            {
                                *old = row;
                            } else {
                                projection.rows.push(row);
                            }
                        }
                        Some("item.removed") => projection
                            .rows
                            .retain(|r| event["sourceItemId"] != r.source_item_id),
                        Some("fields.updated") => projection.fields = event["fields"].clone(),
                        _ => {}
                    }
                }
                projection.rows.sort_by_key(|r| r.position);
                projection.sequence = sequence;
                projection.revision = value["historyRevision"].as_str().unwrap_or("").into();
            }
            let bytes = value.to_string().len();
            let count = value["events"].as_array().map(Vec::len).unwrap_or(1);
            self.bytes += bytes;
            self.events.push_back(Event {
                thread,
                sequence,
                value,
                bytes,
                count,
            });
        }
        while self.events.len() > RETAIN_EVENTS || self.bytes > RETAIN_BYTES {
            let event = self.events.pop_front().unwrap();
            self.bytes -= event.bytes;
            self.pruned.insert(event.thread, event.sequence);
        }
        Ok(())
    }

    /// Ported from KSG ThreadStream.decideThreadResume and ws.subscribeThread.
    pub fn replay(&self, thread: u64, after: u64) -> Option<Vec<Value>> {
        let high = self.threads.get(&thread).map(|p| p.sequence).or_else(|| {
            self.events
                .iter()
                .rev()
                .find(|e| e.thread == thread)
                .map(|e| e.sequence)
        })?;
        if after > high || (after < high && after < self.pruned.get(&thread).copied().unwrap_or(0))
        {
            return None;
        }
        let events: Vec<_> = self
            .events
            .iter()
            .filter(|e| e.thread == thread && e.sequence > after)
            .collect();
        if events.iter().map(|e| e.count).sum::<usize>() > REPLAY_EVENTS
            || events.iter().map(|e| e.bytes).sum::<usize>() > REPLAY_BYTES
        {
            return None;
        }
        Some(events.into_iter().map(|e| e.value.clone()).collect())
    }

    pub fn high_water(&self, thread: u64) -> Option<u64> {
        self.threads.get(&thread).map(|p| p.sequence)
    }

    /// Ported from KSG ProjectionStore.getThreadHistoryPage and threadHistoryPaging.
    /// Page backwards by source identity. A content revision never invalidates the anchor.
    pub fn page(&self, thread: u64, cursor: Option<&str>, budget: usize) -> Result<Value, String> {
        self.page_with_target(thread, cursor, budget, None)
    }

    /// Ported from KSG ProjectionStore.getThreadHistoryPage and olderHistoryPagePolicy.
    pub fn page_with_target(
        &self,
        thread: u64,
        cursor: Option<&str>,
        budget: usize,
        through_entry_id: Option<&str>,
    ) -> Result<Value, String> {
        let projection = self.threads.get(&thread).ok_or("Unknown thread.")?;
        let total_messages = projection
            .rows
            .iter()
            .filter(|row| row.field == "messages")
            .count();
        let mut timeline: Vec<&Row> = projection
            .rows
            .iter()
            .filter(|row| is_timeline_field(&row.field))
            .collect();
        timeline.sort_by_key(|row| timeline_sort_key(row, total_messages));
        let control: Vec<&Row> = projection
            .rows
            .iter()
            .filter(|row| !is_timeline_field(&row.field))
            .collect();

        let end = if let Some(cursor) = cursor {
            if cursor.len() > 4096 {
                return Err("Invalid history cursor.".into());
            }
            let anchor: Value =
                serde_json::from_str(cursor).map_err(|_| "Invalid history cursor.")?;
            if !matches!(anchor["v"].as_u64(), Some(1 | 2))
                || anchor["st"] != thread
                || !anchor["si"].is_string()
                || !anchor["seq"].is_u64()
                || !anchor["p"].is_u64()
            {
                return Err("Invalid history cursor.".into());
            }
            let si = anchor["si"].as_str().unwrap();
            timeline
                .iter()
                .position(|row| row.source_item_id == si)
                .unwrap_or_else(|| {
                    if anchor["v"] == 2 {
                        let p = anchor["p"].as_u64().unwrap() as usize;
                        timeline.iter().take_while(|row| row.position < p).count()
                    } else {
                        (anchor["p"].as_u64().unwrap() as usize).min(timeline.len())
                    }
                })
        } else {
            timeline.len()
        };

        let base_user_turns = if cursor.is_some() {
            PAGE_USER_TURNS_OLDER
        } else {
            PAGE_USER_TURNS_INITIAL
        };
        let (max_user_turns, max_items, max_turn_items) = if let Some(target_id) = through_entry_id
            && let Some(target_index) = timeline[..end]
                .iter()
                .position(|row| row.source_item_id == target_id || row.item["id"] == target_id)
        {
            let turns_between = timeline[target_index..end]
                .iter()
                .filter(|row| is_user_turn_row(row))
                .count();
            let extra = usize::from(!is_user_turn_row(timeline[target_index]));
            (
                (PAGE_USER_TURNS_OLDER * 2).min(PAGE_USER_TURNS_OLDER.max(turns_between + extra)),
                PAGE_ITEMS * 2,
                PAGE_TURN_ITEMS * 2,
            )
        } else {
            (base_user_turns, PAGE_ITEMS, PAGE_TURN_ITEMS)
        };

        let has_user_turns = timeline[..end].iter().any(|row| is_user_turn_row(row));
        let row_limit = if has_user_turns {
            max_turn_items
        } else {
            max_items
        };

        let mut selected = Vec::new();
        let mut bytes = 0usize;
        let mut user_turns = 0usize;
        let mut raw_turns = 0usize;
        for index in (0..end).rev() {
            let row = timeline[index];
            if !selected.is_empty()
                && (selected.len() >= row_limit
                    || (has_user_turns
                        && (user_turns >= max_user_turns || raw_turns >= PAGE_RAW_TURNS)))
            {
                break;
            }
            let size = serde_json::to_string(row).map_err(|e| e.to_string())?.len();
            if !selected.is_empty() && bytes + size > budget {
                break;
            }
            selected.push(row);
            bytes += size;
            if is_user_turn_row(row) {
                user_turns += 1;
                raw_turns += 1;
            } else if row.field == "messages" && row.item["role"] == "Shina" {
                raw_turns += 1;
            }
        }
        if user_turns == 0 {
            selected.truncate(max_items);
        }
        selected.reverse();

        let oldest_index = selected
            .first()
            .and_then(|oldest| {
                timeline
                    .iter()
                    .position(|row| row.source_item_id == oldest.source_item_id)
            })
            .unwrap_or(0);
        let has_more = oldest_index > 0;
        let next = has_more.then(|| {
            let row = timeline[oldest_index];
            json!({"v":2,"seq":projection.sequence,"st":thread,"si":row.source_item_id,"p":row.position})
                .to_string()
        });

        let mut items: Vec<Row> = selected.into_iter().cloned().collect();
        if cursor.is_none() {
            items.extend(control.into_iter().cloned());
        }
        Ok(
            json!({"threadId":thread,"snapshotSequence":projection.sequence,
            "historyRevision":projection.revision,"items":items,
            "fields":projection.fields,"nextCursor":next,"hasMoreHistory":has_more}),
        )
    }
}

struct Lease {
    thread: u64,
    token: String,
    text: String,
    touched: Instant,
}

pub struct Fragments {
    leases: VecDeque<Lease>,
    byte_limit: usize,
}

impl Default for Fragments {
    fn default() -> Self {
        Self {
            leases: VecDeque::new(),
            byte_limit: LEASE_BYTES,
        }
    }
}

impl Fragments {
    pub fn expire(&mut self, now: Instant) {
        self.leases
            .retain(|lease| now.duration_since(lease.touched) < LEASE_IDLE);
    }

    pub fn read(
        &mut self,
        thread: u64,
        params: &Value,
        now: Instant,
        page: impl FnOnce() -> Result<Value, String>,
    ) -> Result<Value, String> {
        self.expire(now);
        let index = if let Some(cursor) = params["fragmentCursor"].as_str() {
            let (token, _) = cursor.split_once(':').ok_or("Invalid fragment cursor.")?;
            self.leases
                .iter()
                .position(|lease| lease.thread == thread && lease.token == token)
                .ok_or(protocol::HISTORY_LEASE_GONE)?
        } else {
            let text = page()?.to_string();
            if text.len() > protocol::MAX_SNAPSHOT_BYTES || text.len() > self.byte_limit {
                return Err("Thread history exceeded the server memory budget.".into());
            }
            while self.leases.len() >= LEASE_COUNT
                || self.leases.iter().map(|l| l.text.len()).sum::<usize>() + text.len()
                    > self.byte_limit
            {
                let oldest = self
                    .leases
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, l)| l.touched)
                    .map(|(i, _)| i)
                    .expect("lease budget");
                self.leases.remove(oldest);
            }
            self.leases.push_back(Lease {
                thread,
                token: uuid::Uuid::new_v4().to_string(),
                text,
                touched: now,
            });
            self.leases.len() - 1
        };
        let lease = &mut self.leases[index];
        let mut paging = params.clone();
        paging["cursor"] = params["fragmentCursor"].clone();
        let mut result = protocol::text_page_revision(&lease.text, &lease.token, &paging)?;
        lease.touched = now;
        result["nextFragment"] = result["nextCursor"].take();
        if let Some(object) = result.as_object_mut() {
            object.remove("nextCursor");
            object.remove("revision");
        }
        Ok(result)
    }
}

pub fn byte_budget(params: &Value) -> usize {
    params["byteBudget"]
        .as_u64()
        .unwrap_or(protocol::PAGE_BYTES as u64)
        .clamp(1024, protocol::MAX_PAGE_BYTES as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durable_positions_and_repeated_native_ids_survive_updates_and_removals() {
        let mut before = snapshot(1, json!([{"id":"user","role":"You","text":"old"}]));
        let thread = &mut before["state"]["threads"][0];
        thread["tools"] =
            json!([{"id":"call-1","output":"first"},{"id":"call-1","output":"second"}]);
        thread["models"] =
            json!([{"id":"shared","provider":"one"},{"id":"shared","provider":"two"}]);
        stamp(&Value::Null, thread, 1);
        assert_ne!(
            thread["_history"]["tools"][0]["id"],
            thread["_history"]["tools"][1]["id"]
        );
        assert_ne!(
            thread["_history"]["models"][0]["id"],
            thread["_history"]["models"][1]["id"]
        );
        let tools = thread["_history"]["tools"].clone();
        let next = thread["historyNextPosition"].as_u64().unwrap();
        let mut after = before.clone();
        after["seq"] = json!(2);
        let thread = &mut after["state"]["threads"][0];
        thread["tools"][1]["output"] = json!("second updated");
        thread["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"answer","role":"Pi","text":"new"}));
        stamp(&before["state"]["threads"][0], thread, 2);
        for index in 0..2 {
            assert_eq!(thread["_history"]["tools"][index]["id"], tools[index]["id"]);
            assert_eq!(
                thread["_history"]["tools"][index]["position"],
                tools[index]["position"]
            );
        }
        assert_eq!(thread["_history"]["messages"][1]["position"], next);
        let streaming = json!({"seq":2,"state":{"threads":[thread.clone()]}});
        let mut live = Streams::new(&before);
        live.ingest(Streams::changes(&before, &streaming)).unwrap();
        assert_eq!(
            live.page(1, None, 1).unwrap()["items"][0]["item"]["text"],
            "new"
        );
        let previous = thread.clone();
        thread["messages"].as_array_mut().unwrap().remove(0);
        stamp(&previous, thread, 3);
        let previous = thread.clone();
        thread["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"later","role":"Pi","text":"later"}));
        stamp(&previous, thread, 4);
        assert!(
            thread["_history"]["messages"][1]["position"]
                .as_u64()
                .unwrap()
                > next
        );
    }
    #[test]
    fn an_oversized_commit_updates_live_items_even_when_replay_cannot_retain_it() {
        let mut before = snapshot(1, json!([{"id":"answer","role":"Pi","text":"old"}]));
        stamp(&Value::Null, &mut before["state"]["threads"][0], 1);
        let mut after = before.clone();
        after["seq"] = json!(2);
        after["state"]["threads"][0]["messages"][0]["text"] =
            json!("x".repeat(RETAIN_BYTES + 1024));
        stamp(
            &before["state"]["threads"][0],
            &mut after["state"]["threads"][0],
            2,
        );
        let changes = Streams::changes(&before, &after);
        assert_eq!(changes.len(), 1);
        let mut live = Streams::new(&before);
        live.ingest(changes).unwrap();
        assert_eq!(live.high_water(1), Some(2));
        assert!(live.replay(1, 1).is_none());
        assert_eq!(
            live.page(1, None, 1024).unwrap()["items"][0]["item"]["text"]
                .as_str()
                .unwrap()
                .len(),
            RETAIN_BYTES + 1024
        );
        assert!(live.bytes <= RETAIN_BYTES);
    }
    fn snapshot(seq: u64, messages: Value) -> Value {
        json!({"seq":seq,"state":{"threads":[{"id":1,"messages":messages,"stats":{}}]}})
    }
    #[test]
    fn fragments_survive_changes_expire_evict_and_reject_cross_thread() {
        let mut leases = Fragments::default();
        let now = Instant::now();
        let first = leases
            .read(1, &json!({"byteBudget":1024}), now, || {
                Ok(json!({"text":"😀".repeat(1000)}))
            })
            .unwrap();
        let params = json!({"fragmentCursor":first["nextFragment"],"byteBudget":1024});
        let next = leases
            .read(1, &params, now + Duration::from_secs(29), || {
                panic!("must not read changed page")
            })
            .unwrap();
        assert!(next["data"].as_str().unwrap().len() <= 1024);
        assert_eq!(
            leases.read(2, &params, now, || unreachable!()).unwrap_err(),
            protocol::HISTORY_LEASE_GONE
        );
        assert!(
            leases
                .read(1, &params, now + Duration::from_secs(30), || unreachable!())
                .is_ok()
        );
        assert_eq!(
            leases
                .read(1, &params, now + Duration::from_secs(60), || unreachable!())
                .unwrap_err(),
            protocol::HISTORY_LEASE_GONE
        );
        for thread in 1..=5 {
            leases
                .read(thread, &json!({}), now, || Ok(json!({})))
                .unwrap();
        }
        assert_eq!(leases.leases.len(), LEASE_COUNT);
        assert!(!leases.leases.iter().any(|l| l.thread == 1));
        assert!(leases.leases.iter().map(|l| l.text.len()).sum::<usize>() <= LEASE_BYTES);
        let mut small = Fragments {
            byte_limit: 3000,
            ..Fragments::default()
        };
        let first = small
            .read(1, &json!({"byteBudget":1024}), now, || {
                Ok(json!({"text":"x".repeat(2000)}))
            })
            .unwrap();
        small
            .read(2, &json!({}), now, || Ok(json!({"text":"x".repeat(2000)})))
            .unwrap();
        assert_eq!(small.leases.len(), 1);
        assert_eq!(
            small
                .read(
                    1,
                    &json!({"fragmentCursor":first["nextFragment"]}),
                    now,
                    || unreachable!()
                )
                .unwrap_err(),
            protocol::HISTORY_LEASE_GONE
        );
    }
    #[test]
    fn history_anchors_survive_streaming_and_item_order_is_exact() {
        let mut streams = Streams::new(&snapshot(
            1,
            json!([{"role":"user","text":"old"},{"role":"assistant","text":"a"}]),
        ));
        let first = streams.page(1, None, 1).unwrap();
        let id = first["items"][0]["sourceItemId"].clone();
        streams.publish(&snapshot(
            2,
            json!([{"role":"user","text":"old"},{"role":"assistant","text":"ab"}]),
        ));
        let events = streams.replay(1, 1).unwrap();
        assert_eq!(events[0]["events"][0]["row"]["sourceItemId"], id);
        let older = streams.page(1, first["nextCursor"].as_str(), 1).unwrap();
        assert_eq!(older["items"][0]["item"]["text"], "old");
        let mut wrong: Value = serde_json::from_str(first["nextCursor"].as_str().unwrap()).unwrap();
        wrong["st"] = json!(2);
        assert!(streams.page(1, Some(&wrong.to_string()), 1).is_err());
        assert!(streams.replay(1, 3).is_none());
    }
    #[test]
    fn replay_is_bounded_and_pruning_requires_snapshot_but_idle_cursor_still_resumes() {
        let mut streams = Streams::new(&snapshot(1, json!([])));
        for seq in 2..=1100 {
            streams.publish(&snapshot(
                seq,
                json!([{"role":"assistant","text":seq.to_string()}]),
            ));
        }
        assert!(streams.replay(1, 1).is_none());
        assert!(streams.replay(1, 900).is_none());
        assert_eq!(streams.replay(1, 1099).unwrap().len(), 1);
        assert!(streams.replay(1, 1100).unwrap().is_empty());
        assert!(streams.bytes <= RETAIN_BYTES);
        // Byte limits apply even when there is only one updated item.
        let mut large = Streams::new(&snapshot(1, json!([])));
        large.publish(&snapshot(2, json!([{"text":"x".repeat(REPLAY_BYTES + 1)}])));
        assert!(large.replay(1, 1).is_none());
        assert!(large.replay(1, 2).unwrap().is_empty());
        for seq in 3..=6 {
            large.publish(&snapshot(
                seq,
                json!([{"text":format!("{seq}{}", "x".repeat(REPLAY_BYTES + 1))}]),
            ));
        }
        assert!(large.bytes <= RETAIN_BYTES);
        assert!(large.replay(1, 2).is_none());
    }

    #[test]
    fn initial_window_respects_user_turn_budget_and_older_pages_step_through_end_of_history() {
        // 35 user turns (70 messages) + 35 tools + 35 work sections.
        let mut messages = Vec::new();
        let mut tools = Vec::new();
        let mut work = Vec::new();
        for turn in 0..35usize {
            let user_idx = messages.len();
            messages.push(
                json!({"id":format!("u-{turn}"),"role":"You","text":format!("User turn {turn}")}),
            );
            let assistant_idx = messages.len();
            messages.push(
                json!({"id":format!("a-{turn}"),"role":"Pi","text":format!("Answer {turn}")}),
            );
            tools.push(json!({"id":format!("tool-{turn}"),"name":"read","detail":"a.rs","done":true,"failed":false,"output":"ok","images":[],"structured":null,"parent_call":null,"exit_code":0,"subagents":[],"anchor":{"message":assistant_idx,"offset":0},"block":0,"order":1}));
            work.push(json!({"start":assistant_idx,"stopped":false,"stopped_order":0,"elapsed":1}));
            let _ = user_idx;
        }
        let mut snap = json!({"seq":1,"state":{"threads":[{"id":1,"messages":messages,"tools":tools,"work":work,"stats":{},"models":[{"id":"m1"}]}]}});
        stamp(&Value::Null, &mut snap["state"]["threads"][0], 1);
        let streams = Streams::new(&snap);

        // Initial window (cursor = None) stops at 10 user turns (turns 25..35), not 75 arbitrary rows.
        let initial = streams.page(1, None, 1_048_576).unwrap();
        assert_eq!(initial["hasMoreHistory"], true);
        let initial_msgs: Vec<_> = initial["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["field"] == "messages")
            .collect();
        assert_eq!(initial_msgs.len(), 20, "10 user turns = 20 messages");
        assert_eq!(initial_msgs[0]["item"]["text"], "User turn 25");
        assert_eq!(initial_msgs.last().unwrap()["item"]["text"], "Answer 34");
        // Control arrays (models) must arrive in the initial window.
        assert!(
            initial["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["field"] == "models")
        );

        // Older page 1 uses OLDER_THREAD_USER_TURN_LIMIT = 20 user turns (turns 5..25).
        let page1 = streams
            .page(1, initial["nextCursor"].as_str(), 1_048_576)
            .unwrap();
        assert_eq!(page1["hasMoreHistory"], true);
        let page1_msgs: Vec<_> = page1["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["field"] == "messages")
            .collect();
        assert_eq!(page1_msgs.len(), 40, "20 user turns = 40 messages");
        assert_eq!(page1_msgs[0]["item"]["text"], "User turn 5");
        assert_eq!(page1_msgs.last().unwrap()["item"]["text"], "Answer 24");

        // Older page 2 reaches turn 0 and sets hasMoreHistory = false, nextCursor = null.
        let page2 = streams
            .page(1, page1["nextCursor"].as_str(), 1_048_576)
            .unwrap();
        assert_eq!(page2["hasMoreHistory"], false);
        assert!(page2["nextCursor"].is_null());
        let page2_msgs: Vec<_> = page2["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["field"] == "messages")
            .collect();
        assert_eq!(page2_msgs.len(), 10, "remaining 5 user turns = 10 messages");
        assert_eq!(page2_msgs[0]["item"]["text"], "User turn 0");
    }

    #[test]
    fn empty_or_short_thread_has_no_more_history() {
        let mut empty = snapshot(1, json!([]));
        stamp(&Value::Null, &mut empty["state"]["threads"][0], 1);
        let streams = Streams::new(&empty);
        let page = streams.page(1, None, 65536).unwrap();
        assert_eq!(page["hasMoreHistory"], false);
        assert!(page["nextCursor"].is_null());
    }
}
