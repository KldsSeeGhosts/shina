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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub source_thread_id: u64,
    pub source_item_id: String,
    pub field: String,
    /// Ordinal within Shina's existing field, not a timestamp or message hash.
    pub ordinal: usize,
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

/// Transport-only indexes. No database records, receipts or outbox formats change.
#[derive(Default)]
pub struct Streams {
    threads: HashMap<u64, Projection>,
    events: VecDeque<Event>,
    pruned: HashMap<u64, u64>,
    bytes: usize,
    next_id: u64,
}

impl Streams {
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
            let history = protocol::history_payload(thread);
            let revision = protocol::revision(&history);
            if self
                .threads
                .get(&id)
                .is_some_and(|old| old.revision == revision)
            {
                continue;
            }
            let before = self.threads.remove(&id);
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
                // Shina's message records have no durable IDs. Assign wire-only IDs,
                // retain exact matches across moves, and retain the slot on text updates.
                // Intrinsic tool/work IDs take precedence. A new server starts a new epoch.
                let mut used = HashSet::new();
                let mut exact: HashMap<String, VecDeque<&Row>> = HashMap::new();
                for row in &old {
                    exact
                        .entry(row.item.to_string())
                        .or_default()
                        .push_back(row);
                }
                let mut identities = Vec::new();
                for item in items {
                    let found = exact
                        .get_mut(&item.to_string())
                        .and_then(VecDeque::pop_front);
                    if let Some(row) = found {
                        used.insert(row.source_item_id.clone());
                    }
                    identities.push(found.map(|row| row.source_item_id.clone()));
                }
                for (ordinal, item) in items.iter().enumerate() {
                    let identity = identities[ordinal]
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
                        });
                    used.insert(identity.clone());
                    rows.push(Row {
                        source_thread_id: id,
                        source_item_id: identity,
                        field: field.into(),
                        ordinal,
                        position: rows.len(),
                        item: item.clone(),
                    });
                }
            }
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
            }
            if !changes.is_empty() {
                let count = changes.len();
                let value = json!({"kind":"event","threadId":id,"sequence":sequence,
                    "historyRevision":revision,"events":changes});
                let bytes = value.to_string().len();
                self.bytes += bytes;
                self.events.push_back(Event {
                    thread: id,
                    sequence,
                    value,
                    bytes,
                    count,
                });
            }
            self.threads.insert(
                id,
                Projection {
                    sequence,
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
        while self.events.len() > RETAIN_EVENTS || self.bytes > RETAIN_BYTES {
            let event = self.events.pop_front().expect("replay budget");
            self.bytes -= event.bytes;
            self.pruned.insert(event.thread, event.sequence);
        }
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
        let projection = self.threads.get(&thread).ok_or("Unknown thread.")?;
        let end = if let Some(cursor) = cursor {
            if cursor.len() > 4096 {
                return Err("Invalid history cursor.".into());
            }
            let anchor: Value =
                serde_json::from_str(cursor).map_err(|_| "Invalid history cursor.")?;
            if anchor["v"] != 1
                || anchor["st"] != thread
                || !anchor["si"].is_string()
                || !anchor["seq"].is_u64()
                || !anchor["p"].is_u64()
            {
                return Err("Invalid history cursor.".into());
            }
            projection
                .rows
                .iter()
                .position(|row| row.source_item_id == anchor["si"].as_str().unwrap())
                .unwrap_or_else(|| {
                    (anchor["p"].as_u64().unwrap() as usize).min(projection.rows.len())
                })
        } else {
            projection.rows.len()
        };
        let mut start = end;
        let mut bytes = 0;
        while start > 0 && end - start < PAGE_ITEMS {
            let size = serde_json::to_string(&projection.rows[start - 1])
                .map_err(|e| e.to_string())?
                .len();
            if start < end && bytes + size > budget {
                break;
            }
            bytes += size;
            start -= 1;
        }
        let next = (start > 0).then(|| {
            let row = &projection.rows[start];
            json!({"v":1,"seq":projection.sequence,"st":thread,"si":row.source_item_id,"p":start})
                .to_string()
        });
        Ok(
            json!({"threadId":thread,"snapshotSequence":projection.sequence,
            "historyRevision":projection.revision,"items":&projection.rows[start..end],
            "fields":projection.fields,"nextCursor":next,"hasMoreHistory":next.is_some()}),
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
}
