//! Ported from KSG client-runtime state/threads.ts and threadHistoryMerge.ts.
use crate::{history::Row, protocol};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

/// Process-local revision tokens cannot repeat after a cache replacement or
/// history snapshot. They are invalidation tokens, never durable sequence IDs.
fn next_revision() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static REVISION: AtomicU64 = AtomicU64::new(0);
    REVISION
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("Transcript revision exhausted")
        + 1
}

/// Ordered items for one history field, shared between emissions. `ids` are
/// the server's durable source item ids, parallel to `items`; `ords` are the
/// server ordinals in the same order. A whole `FieldItems` is immutable once
/// emitted, so an `Arc` clone is the cheap "field is identical" share.
#[derive(Clone, Default)]
pub(crate) struct FieldItems {
    /// Stable item ids parallel to `items`; `None` when the source record had
    /// no durable identities (a plain `Value` projection).
    pub(crate) ids: Option<Vec<Arc<str>>>,
    /// Server ordinals parallel to `items` when `ids` is set.
    pub(crate) ords: Vec<usize>,
    pub(crate) items: Vec<Arc<Value>>,
    /// Serialized payload estimate. Used without walking unchanged payloads.
    bytes: usize,
}

/// Content-changed item ids per field since a thread's previous emission.
pub(crate) type FieldChanges = HashMap<&'static str, HashSet<Arc<str>>>;

/// One projected thread: a compact record plus item-level transcript fields.
///
/// Contract for the timeline consumer (#25/#26): `structure` bumps when item
/// membership, order, anchors or deferred-work markers change (prepends,
/// removals, snapshots); `content` bumps when any item payload or non-item
/// record field changes. A pure content append bumps `content` only and keeps
/// every unchanged item's `Arc` identity, so a consumer keyed on `ids` keeps
/// its row objects. `structural` reports whether structure moved since this
/// thread's previous emission and `items_changed` names the payloads that
/// moved content (`None` = everything may differ: snapshot, first emission,
/// untracked source). `record_changed` covers history meta, retry and
/// record-level fields. Change sets refer to consecutive emissions only.
/// A coalescing consumer must compare shared item Arcs with its last applied
/// projection rather than using a skipped emission's change set as its base.
/// Revision tokens do not repeat when a cache or history snapshot is replaced.
/// Thread keys stay the numeric projection ids.
pub(crate) struct ThreadProjection {
    /// Compact record: workspace fields + non-array history fields +
    /// `historyMeta`/`historyRetry`. `HISTORY_FIELDS` arrays stay empty here;
    /// transcript payloads live in `fields`.
    pub(crate) record: Arc<Value>,
    pub(crate) fields: HashMap<&'static str, Arc<FieldItems>>,
    /// A hydrated empty transcript is distinct from an unloaded one.
    pub(crate) history_loaded: bool,
    pub(crate) structure: u64,
    pub(crate) content: u64,
    /// #25 seam: whether structure moved and whether record-level fields did.
    /// The apply path reads item identity diffs; the timeline reads these.
    #[allow(dead_code)]
    pub(crate) structural: bool,
    #[allow(dead_code)]
    pub(crate) items_changed: Option<Arc<FieldChanges>>,
    #[allow(dead_code)]
    pub(crate) record_changed: bool,
    record_bytes: usize,
}

/// One projected workspace. `state` holds the complete workspace state but its
/// `threads` records keep only compact fields plus `historyMeta`/`historyRetry`;
/// the emitted thread projections live in `views` and keep their `Arc`
/// allocation between projections when the thread did not change.
pub(crate) struct Projection {
    pub(crate) state: Value,
    pub(crate) views: HashMap<u64, Arc<ThreadProjection>>,
    /// Ids whose emitted record changed since the previous projection. `None`
    /// means every record was rebuilt: snapshot, reset, or first emission.
    pub(crate) changed: Option<Arc<HashSet<u64>>>,
}

impl Projection {
    /// The wire-shaped projection with every item field materialized back into
    /// each thread record. Only remote persistence and asset fetching need
    /// this; applying a projection reads `views` directly.
    pub(crate) fn full(&self) -> Value {
        let mut state = self.state.clone();
        if let Some(threads) = state["threads"].as_array_mut() {
            for record in threads {
                if let Some(view) = record["id"].as_u64().and_then(|id| self.views.get(&id)) {
                    let mut full = (*view.record).clone();
                    for (field, items) in &view.fields {
                        full[field] =
                            Value::Array(items.items.iter().map(|item| (**item).clone()).collect());
                    }
                    *record = full;
                }
            }
        }
        state
    }
}

impl Clone for Projection {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            views: self.views.clone(),
            changed: self.changed.clone(),
        }
    }
}

/// A plain `Value` projection, as tests and remote asset responses produce, is a
/// full projection: every thread record counts as changed and no item carries
/// a durable id.
impl From<Value> for Projection {
    fn from(state: Value) -> Self {
        let views = state["threads"]
            .as_array()
            .map(|threads| {
                threads
                    .iter()
                    .filter_map(|thread| {
                        thread["id"]
                            .as_u64()
                            .map(|id| (id, Arc::new(ThreadProjection::from_record(thread.clone()))))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            state,
            views,
            changed: None,
        }
    }
}

impl ThreadProjection {
    /// An absent thread for UI fallbacks that need a `&ThreadProjection`.
    pub(crate) fn empty(record: &Value) -> Self {
        Self::from_record(record.clone())
    }

    /// Split a wire-shaped thread record into a compact record plus item
    /// fields. No durable identities exist here, so consumers take the
    /// everything-changed path.
    fn from_record(mut record: Value) -> Self {
        let revision = next_revision();
        let mut fields = HashMap::new();
        for field in protocol::HISTORY_FIELDS {
            if let Some(items) = record.get_mut(field).and_then(Value::as_array_mut) {
                let items = std::mem::take(items);
                fields.insert(
                    field,
                    Arc::new(FieldItems {
                        ids: None,
                        ords: Vec::new(),
                        items: items.into_iter().map(Arc::new).collect(),
                        // Untracked full projections are not retained by the
                        // history cache and do not need its byte accounting.
                        bytes: 0,
                    }),
                );
            }
        }
        let record_bytes = record.to_string().len();
        Self {
            record: Arc::new(record),
            fields,
            history_loaded: true,
            structure: revision,
            content: revision,
            structural: true,
            items_changed: None,
            record_changed: true,
            record_bytes,
        }
    }
}

/// `projection["index"]` keeps working for workspace fields; thread records are
/// read through `views` because `state["threads"]` stays compact.
impl std::ops::Index<&str> for Projection {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        &self.state[key]
    }
}

impl std::ops::IndexMut<&str> for Projection {
    fn index_mut(&mut self, key: &str) -> &mut Value {
        &mut self.state[key]
    }
}

/// Counts proving an unchanged thread record is shared, not re-materialized.
#[derive(Default, Clone, Copy)]
pub(crate) struct ProjectionCounters {
    pub(crate) projections: u64,
    /// Emissions that rebuilt every thread record versus ones that shared them.
    pub(crate) full: u64,
    pub(crate) delta: u64,
    /// Thread records rebuilt into a new allocation versus reused unchanged.
    pub(crate) threads_rebuilt: u64,
    pub(crate) threads_shared: u64,
    /// Item payloads written into installed views (new or replaced).
    pub(crate) items_written: u64,
    pub(crate) items_removed: u64,
    /// Field item vectors cloned for a splice versus carried by `Arc` into a
    /// new emission.
    pub(crate) fields_spliced: u64,
    pub(crate) fields_shared: u64,
}

#[derive(Default)]
pub(crate) struct ProjectionCache {
    pub(crate) state: Value,
    pub(crate) seq: Option<u64>,
    pub(crate) epoch: Value,
    pub(crate) snapshot_pages: String,
    history_order: VecDeque<u64>,
    pub(crate) desired_thread: Option<u64>,
    inflight: Option<History>,
    pub(crate) dirty: bool,
    pub(crate) maintenance: Value,
    views: HashMap<u64, View>,
    subscribed: Option<u64>,
    stream_pages: String,
    pub(crate) notices: VecDeque<String>,
    retries: HashMap<u64, Retry>,
    update_required: bool,
    deferred_work: bool,
    /// The last emitted projection per thread, shared between projections.
    emitted: HashMap<u64, Arc<ThreadProjection>>,
    /// Threads whose emitted record changed since the last `projected()`.
    touched: HashSet<u64>,
    /// The next projection must rebuild every record.
    rebuild: bool,
    pub(crate) counters: ProjectionCounters,
}

/// Item changes accumulated between two emissions of the same thread.
#[derive(Default)]
struct PendingDelta {
    /// Membership, order, anchors or deferred markers moved.
    structural: bool,
    /// Non-item record fields or history meta changed.
    record: bool,
    /// The installed view was rebuilt, so every item may differ.
    all: bool,
    /// Per-field sids whose payload changed.
    items: HashMap<&'static str, HashSet<Arc<str>>>,
}

/// Where one installed item sits inside its field.
struct Slot {
    field: &'static str,
    ordinal: usize,
    /// Serialized size contribution for the eviction budget.
    bytes: usize,
}

/// The assembled per-thread history, kept incrementally instead of rebuilt on
/// every stream event. `fields` items are `Arc`-shared into emissions, so an
/// unchanged item is never cloned.
#[derive(Default)]
struct Installed {
    /// Non-array history fields (`id`, `stats`, `harness_state`, ...).
    scalars: Value,
    /// `View::fields_sequence` when `scalars` was captured.
    scalars_sequence: u64,
    scalars_bytes: usize,
    /// Ordered shared items per history field.
    fields: HashMap<&'static str, Arc<FieldItems>>,
    /// sid → splice position key and serialized size.
    slot: HashMap<Arc<str>, Slot>,
    /// Anchored rows dropped because their anchor is not loaded yet.
    held: HashSet<Arc<str>>,
    /// sids whose row currently carries a `deferredWork` marker.
    markers: HashSet<Arc<str>>,
    /// The computed `history["deferredWork"]` list.
    deferred: Vec<Value>,
    item_bytes: usize,
    bytes: usize,
}

#[derive(Default)]
struct View {
    sequence: u64,
    epoch: String,
    revision: String,
    rows: HashMap<String, (u64, Row)>,
    /// Source rows remain separate from their locally remapped item payloads.
    row_bytes: usize,
    removed: HashMap<String, u64>,
    /// sids created, updated or removed since the last `install`.
    reconcile: HashSet<String>,
    installed: Option<Installed>,
    /// Change markers accumulated since this thread's last emission.
    pending: PendingDelta,
    fields: Value,
    fields_sequence: u64,
    cursor: Option<String>,
    has_more: bool,
    loading: bool,
    error: Option<String>,
    expanded: bool,
    load_requested: bool,
    through_entry_id: Option<String>,
    ready: bool,
    work_requests: VecDeque<String>,
    work_errors: HashMap<String, String>,
}

impl View {
    fn insert_row(&mut self, sequence: u64, row: Row) {
        self.row_bytes += serde_json::to_string(&row).expect("history row").len();
        if let Some((_, previous)) = self
            .rows
            .insert(row.source_item_id.clone(), (sequence, row))
        {
            self.row_bytes = self
                .row_bytes
                .saturating_sub(serde_json::to_string(&previous).expect("history row").len());
        }
    }

    fn remove_row(&mut self, sid: &str) {
        if let Some((_, previous)) = self.rows.remove(sid) {
            self.row_bytes = self
                .row_bytes
                .saturating_sub(serde_json::to_string(&previous).expect("history row").len());
        }
    }
}

#[derive(Default)]
struct Retry {
    attempts: u8,
    restarts: u32,
    blocked: bool,
    error: Option<String>,
}

struct History {
    thread: u64,
    subscribe: bool,
    snapshot: bool,
    cursor: Option<String>,
    through_entry_id: Option<String>,
    fragment: Value,
    text: String,
    waiting: bool,
    pending: Vec<Value>,
    pending_bytes: usize,
    work_anchor: Option<String>,
}

impl ProjectionCache {
    pub(crate) fn reconnect(&mut self, hello: &Value) -> Result<(), String> {
        self.inflight = None;
        self.subscribed = None;
        self.snapshot_pages.clear();
        self.stream_pages.clear();
        let deferred = hello["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::DEFERRED_WORK));
        if self.deferred_work && !deferred {
            // An older build must re-send inline history, not inherit unresolved work anchors.
            self.views.clear();
        }
        if self.deferred_work != deferred {
            // `workLoading` in emitted records depends on the negotiated mode.
            self.rebuild = true;
        }
        self.deferred_work = deferred;
        if !hello["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::HISTORY_ITEMS))
        {
            self.update_required = true;
            self.dirty = true;
            self.notices
                .push_back(protocol::HISTORY_UPDATE_REQUIRED.into());
            return Err(protocol::HISTORY_UPDATE_REQUIRED.into());
        }
        self.update_required = false;
        Ok(())
    }

    fn failed(&mut self, thread: u64, message: &str, recoverable: bool) {
        let retry = self.retries.entry(thread).or_default();
        retry.error = Some(message.into());
        if recoverable && retry.attempts < 1 {
            retry.attempts += 1;
            retry.restarts += 1;
            self.subscribed = None;
            // Do not replay from the incomplete or unavailable base on the retry.
            if let Some(view) = self.views.get_mut(&thread) {
                view.epoch.clear();
            }
            self.notices.push_back(format!(
                "Thread {thread} history retry {}/1. {message}",
                retry.attempts
            ));
        } else {
            retry.blocked = true;
            self.notices.push_back(format!(
                "Thread {thread} history stopped after {}/1 retries. {message}",
                retry.attempts
            ));
        }
        self.touched.insert(thread);
        self.dirty = true;
    }

    #[cfg(test)]
    pub(crate) fn complete(&self, thread: u64) -> bool {
        self.views
            .get(&thread)
            .is_some_and(|view| view.ready && view.cursor.is_none())
    }

    pub(crate) fn request_earlier(&mut self, thread: u64, through_entry_id: Option<String>) {
        let Some(view) = self.views.get_mut(&thread) else {
            return;
        };
        if !view.has_more || view.cursor.is_none() || view.loading {
            return;
        }
        view.loading = true;
        view.error = None;
        view.load_requested = true;
        view.through_entry_id = through_entry_id;
        view.pending.record = true;
        self.touched.insert(thread);
        self.dirty = true;
    }

    pub(crate) fn request_work(&mut self, thread: u64, anchor: String) {
        if !self.deferred_work {
            return;
        }
        let Some(view) = self.views.get_mut(&thread) else {
            return;
        };
        if !view
            .rows
            .get(&anchor)
            .is_some_and(|(_, row)| row.item["deferredWork"].is_object())
            || view.work_requests.contains(&anchor)
        {
            return;
        }
        view.work_errors.remove(&anchor);
        view.work_requests.push_back(anchor);
        view.pending.record = true;
        if let Err(error) = self.install(thread) {
            self.failed(thread, &error, false);
        }
    }

    /// Sorted server ordinals of loaded message rows; index = local position.
    /// Derived from the installed `messages` field so every message splice
    /// keeps the map in step — incremental installs never rebuild it.
    fn msg_ords(installed: &Installed) -> &[usize] {
        installed
            .fields
            .get("messages")
            .map(|items| items.ords.as_slice())
            .unwrap_or(&[])
    }

    /// Server ordinal → loaded local message position. `None` when the anchor
    /// sits inside the unloaded earlier range, matching the dropped-item
    /// behaviour of the full assembly path.
    fn remap_msg(msg_ords: &[usize], server_ord: usize) -> Option<usize> {
        match msg_ords.binary_search(&server_ord) {
            Ok(local) => Some(local),
            Err(_) if msg_ords.is_empty() => Some(server_ord),
            Err(local) if local == msg_ords.len() => Some(local),
            Err(_) => None,
        }
    }

    /// Index of `(ordinal, sid)` inside a field's sorted item list.
    fn item_pos(field: &FieldItems, ordinal: usize, sid: &str) -> Option<usize> {
        let ids = field.ids.as_ref()?;
        field
            .ords
            .iter()
            .zip(ids.iter())
            .position(|(ord, id)| *ord == ordinal && &**id == sid)
    }

    /// Insert position for `(ordinal, sid)` keeping the (ordinal, sid) order.
    fn item_insert_pos(field: &FieldItems, ordinal: usize, sid: &str) -> usize {
        let ids = field.ids.as_ref().expect("installed ids");
        field
            .ords
            .iter()
            .zip(ids.iter())
            .take_while(|(ord, id)| **ord < ordinal || (**ord == ordinal && sid > &***id))
            .count()
    }

    fn field_static(field: &str) -> Option<&'static str> {
        protocol::HISTORY_FIELDS
            .iter()
            .copied()
            .find(|known| *known == field)
    }

    /// Fields whose items anchor to a message ordinal: `tools`/`thoughts`/
    /// `compactions` at `anchor.message`, `work` at `start`.
    fn anchored_field(field: &str) -> bool {
        matches!(field, "tools" | "thoughts" | "compactions" | "work")
    }

    /// One item enters or replaces its slot inside `installed`. Existing sids
    /// keep their slot or move positions; new sids mark a structural change.
    fn upsert_item(
        installed: &mut Installed,
        pending: &mut PendingDelta,
        counters: &mut ProjectionCounters,
        field: &'static str,
        sid: Arc<str>,
        ordinal: usize,
        item: Value,
    ) {
        if item["deferredWork"].is_object() {
            installed.markers.insert(sid.clone());
        } else {
            installed.markers.remove(&sid);
        }
        let bytes = serde_json::to_string(&item)
            .map(|text| text.len())
            .unwrap_or(0);
        counters.items_written += 1;
        if let Some(slot) = installed.slot.get(&sid) {
            if slot.field == field && slot.ordinal == ordinal {
                let items = Arc::make_mut(installed.fields.get_mut(field).unwrap());
                counters.fields_spliced += 1;
                let pos = Self::item_pos(items, ordinal, &sid).expect("installed position");
                pending.structural |= [
                    "anchor",
                    "start",
                    "offset",
                    "block",
                    "order",
                    "deferredWork",
                    "role",
                    "kind",
                    "name",
                    "stopped",
                    "stopped_order",
                ]
                .iter()
                .any(|key| items.items[pos].get(*key) != item.get(*key));
                installed.item_bytes = installed.item_bytes.saturating_sub(slot.bytes) + bytes;
                items.bytes = items.bytes.saturating_sub(slot.bytes) + bytes;
                items.items[pos] = Arc::new(item);
                installed.slot.get_mut(&sid).unwrap().bytes = bytes;
                pending.items.entry(field).or_default().insert(sid);
                return;
            }
            Self::remove_item(installed, pending, counters, &sid);
        }
        let entry = installed.fields.entry(field).or_insert_with(|| {
            Arc::new(FieldItems {
                ids: Some(Vec::new()),
                ords: Vec::new(),
                items: Vec::new(),
                bytes: 0,
            })
        });
        let items = Arc::make_mut(entry);
        counters.fields_spliced += 1;
        let at = Self::item_insert_pos(items, ordinal, &sid);
        items.ords.insert(at, ordinal);
        items.ids.as_mut().unwrap().insert(at, sid.clone());
        items.items.insert(at, Arc::new(item));
        items.bytes += bytes;
        installed.slot.insert(
            sid.clone(),
            Slot {
                field,
                ordinal,
                bytes,
            },
        );
        installed.item_bytes += bytes;
        installed.held.remove(&sid);
        pending.items.entry(field).or_default().insert(sid);
        pending.structural = true;
    }

    /// Drop `sid` from its field list; returns whether it was installed.
    fn remove_item(
        installed: &mut Installed,
        pending: &mut PendingDelta,
        counters: &mut ProjectionCounters,
        sid: &Arc<str>,
    ) -> bool {
        installed.held.remove(sid);
        installed.markers.remove(sid);
        let Some(slot) = installed.slot.remove(sid) else {
            return false;
        };
        let items = Arc::make_mut(installed.fields.get_mut(slot.field).unwrap());
        counters.fields_spliced += 1;
        counters.items_removed += 1;
        if let Some(pos) = Self::item_pos(items, slot.ordinal, sid) {
            items.ords.remove(pos);
            items.ids.as_mut().unwrap().remove(pos);
            items.items.remove(pos);
        }
        installed.item_bytes = installed.item_bytes.saturating_sub(slot.bytes);
        items.bytes = items.bytes.saturating_sub(slot.bytes);
        pending.structural = true;
        true
    }

    /// Apply one reconciled row: remap its anchor to a loaded local position
    /// when the field anchors to messages, then splice it in. Rows whose
    /// anchor sits in the unloaded range stay `held` until the anchor arrives.
    fn upsert_row(
        view: &mut View,
        installed: &mut Installed,
        counters: &mut ProjectionCounters,
        sid: &Arc<str>,
    ) {
        let Some((_, row)) = view.rows.get(sid.as_ref()) else {
            Self::remove_item(installed, &mut view.pending, counters, sid);
            return;
        };
        let Some(field) = Self::field_static(&row.field) else {
            return;
        };
        let mut item = row.item.clone();
        if Self::anchored_field(field) {
            let server_ord = if field == "work" {
                item.get("start").and_then(Value::as_u64)
            } else {
                item.get("anchor")
                    .and_then(|anchor| anchor.get("message"))
                    .and_then(Value::as_u64)
            };
            if let Some(server_ord) = server_ord {
                let Some(local) = Self::remap_msg(Self::msg_ords(installed), server_ord as usize)
                else {
                    Self::remove_item(installed, &mut view.pending, counters, sid);
                    installed.held.insert(sid.clone());
                    return;
                };
                if field == "work" {
                    item["start"] = json!(local);
                } else {
                    item["anchor"]["message"] = json!(local);
                }
            }
        }
        installed.held.remove(sid);
        Self::upsert_item(
            installed,
            &mut view.pending,
            counters,
            field,
            sid.clone(),
            row.ordinal,
            item,
        );
    }

    /// Message membership moved, so every anchored item and `workRetries`
    /// entry remaps or drops, and `held` rows retry against the new window.
    /// Runs once per install, after message splices.
    fn reanchor(
        view: &mut View,
        installed: &mut Installed,
        counters: &mut ProjectionCounters,
        structural: &mut bool,
    ) {
        for field in ["tools", "thoughts", "compactions", "work"] {
            let Some(items) = installed.fields.get_mut(field) else {
                continue;
            };
            let mut items = std::mem::take(items);
            {
                let items = Arc::make_mut(&mut items);
                let mut index = 0;
                while index < items.items.len() {
                    let Some(ids) = items.ids.as_ref() else {
                        break;
                    };
                    let sid = ids[index].clone();
                    let item = (*items.items[index]).clone();
                    // The installed anchor is a local position; the source row
                    // keeps the server ordinal to remap against.
                    let server_ord = view.rows.get(sid.as_ref()).and_then(|(_, row)| {
                        if field == "work" {
                            row.item.get("start").and_then(Value::as_u64)
                        } else {
                            row.item
                                .get("anchor")
                                .and_then(|anchor| anchor.get("message"))
                                .and_then(Value::as_u64)
                        }
                    });
                    let Some(server_ord) = server_ord else {
                        index += 1;
                        continue;
                    };
                    match Self::remap_msg(Self::msg_ords(installed), server_ord as usize) {
                        None => {
                            items.ords.remove(index);
                            items.ids.as_mut().unwrap().remove(index);
                            items.items.remove(index);
                            if let Some(slot) = installed.slot.remove(&sid) {
                                installed.item_bytes =
                                    installed.item_bytes.saturating_sub(slot.bytes);
                                items.bytes = items.bytes.saturating_sub(slot.bytes);
                            }
                            installed.held.insert(sid);
                            view.pending.structural = true;
                            counters.items_removed += 1;
                        }
                        Some(local) => {
                            let installed_ord = if field == "work" {
                                item["start"].as_u64()
                            } else {
                                item["anchor"]["message"].as_u64()
                            };
                            if installed_ord == Some(local as u64) {
                                index += 1;
                                continue;
                            }
                            let mut item = item;
                            if field == "work" {
                                item["start"] = json!(local);
                            } else {
                                item["anchor"]["message"] = json!(local);
                            }
                            let bytes = serde_json::to_string(&item).map(|t| t.len()).unwrap_or(0);
                            if let Some(slot) = installed.slot.get_mut(&sid) {
                                installed.item_bytes =
                                    installed.item_bytes.saturating_sub(slot.bytes) + bytes;
                                items.bytes = items.bytes.saturating_sub(slot.bytes) + bytes;
                                slot.bytes = bytes;
                            }
                            items.items[index] = Arc::new(item);
                            view.pending.structural = true;
                            view.pending.items.entry(field).or_default().insert(sid);
                            counters.items_written += 1;
                            index += 1;
                        }
                    }
                }
            }
            installed.fields.insert(field, items);
        }
        *structural |= view.pending.structural;
        // Anchored rows that were held back retry against the new window.
        let held: Vec<Arc<str>> = installed.held.iter().cloned().collect();
        for sid in held {
            Self::upsert_row(view, installed, counters, &sid);
        }
        *structural |= view.pending.structural;
    }

    /// Scalar refreshes carry raw server anchors even when no message item
    /// changed. Always remap from the raw source, never an already-local index.
    fn remap_work_retries(view: &View, installed: &mut Installed) {
        // `harness_state.workRetries` rides inside the record-level scalars.
        // Installed scalars carry local anchors, so restore the raw entries
        // before remapping against the new window.
        if let Some(raw) = view
            .fields
            .get("harness_state")
            .and_then(|hs| hs.get("workRetries"))
            .cloned()
            && let Some(hs) = installed
                .scalars
                .get_mut("harness_state")
                .and_then(Value::as_object_mut)
        {
            hs.insert("workRetries".into(), raw);
        }
        let msg_ords = Self::msg_ords(installed).to_vec();
        if let Some(retries) = installed
            .scalars
            .get_mut("harness_state")
            .and_then(|hs| hs.get_mut("workRetries"))
            .and_then(Value::as_array_mut)
        {
            retries.retain_mut(|retry| {
                if let Some(server_ord) = retry["anchor"]["message"].as_u64() {
                    let Some(local) = Self::remap_msg(&msg_ords, server_ord as usize) else {
                        return false;
                    };
                    retry["anchor"]["message"] = json!(local);
                }
                true
            });
        }
    }

    /// Recompute the `history["deferredWork"]` list from marker rows, sorted
    /// the way the full assembly emitted them: (field, ordinal, sid).
    fn recompute_deferred(view: &View, installed: &mut Installed, deferred_work: bool) {
        if !deferred_work {
            installed.deferred.clear();
            return;
        }
        let mut sids: Vec<Arc<str>> = installed.markers.iter().cloned().collect();
        sids.sort_by(|a, b| {
            let (af, ao) = installed
                .slot
                .get(a)
                .map(|slot| (slot.field, slot.ordinal))
                .unwrap_or(("", usize::MAX));
            let (bf, bo) = installed
                .slot
                .get(b)
                .map(|slot| (slot.field, slot.ordinal))
                .unwrap_or(("", usize::MAX));
            af.cmp(bf).then(ao.cmp(&bo)).then(a.cmp(b))
        });
        installed.deferred = sids
            .into_iter()
            .filter_map(|sid| {
                let (_, row) = view.rows.get(sid.as_ref())?;
                let mut marker = row
                    .item
                    .get("deferredWork")
                    .filter(|marker| marker.is_object())?
                    .clone();
                marker["sourceItemId"] = json!(row.source_item_id);
                marker["start"] = json!(Self::remap_msg(
                    Self::msg_ords(installed),
                    marker["start"].as_u64()? as usize
                )?);
                marker["loading"] = json!(view.work_requests.contains(&row.source_item_id));
                marker["error"] = json!(view.work_errors.get(&row.source_item_id));
                Some(marker)
            })
            .collect();
    }

    /// Build the installed state from every loaded row: the path snapshots,
    /// epoch resets and first loads take.
    fn fresh_install(
        view: &mut View,
        installed: &mut Installed,
        counters: &mut ProjectionCounters,
    ) {
        let mut rows: Vec<_> = view.rows.values().map(|(_, row)| row).collect();
        rows.sort_by(|a, b| {
            a.field
                .cmp(&b.field)
                .then(a.ordinal.cmp(&b.ordinal))
                .then(a.source_item_id.cmp(&b.source_item_id))
        });
        for row in rows {
            let Some(field) = Self::field_static(&row.field) else {
                continue;
            };
            let mut item = row.item.clone();
            if Self::anchored_field(field) {
                let server_ord = if field == "work" {
                    item.get("start").and_then(Value::as_u64)
                } else {
                    item.get("anchor")
                        .and_then(|anchor| anchor.get("message"))
                        .and_then(Value::as_u64)
                };
                if let Some(server_ord) = server_ord {
                    let Some(local) =
                        Self::remap_msg(Self::msg_ords(installed), server_ord as usize)
                    else {
                        installed
                            .held
                            .insert(Arc::from(row.source_item_id.as_str()));
                        continue;
                    };
                    if field == "work" {
                        item["start"] = json!(local);
                    } else {
                        item["anchor"]["message"] = json!(local);
                    }
                }
            }
            Self::upsert_item(
                installed,
                &mut view.pending,
                counters,
                field,
                Arc::from(row.source_item_id.as_str()),
                row.ordinal,
                item,
            );
        }
    }

    fn install(&mut self, thread: u64) -> Result<(), String> {
        let Some(view) = self.views.get_mut(&thread).filter(|view| view.ready) else {
            return Ok(());
        };
        let mut installed = view.installed.take().unwrap_or_default();
        let mut structural = false;
        let fresh = installed.scalars.is_null();
        let mut remap_retries = fresh || installed.scalars_sequence != view.fields_sequence;
        if fresh {
            installed.scalars = view.fields.clone();
            installed.scalars_sequence = view.fields_sequence;
            installed.scalars_bytes = view.fields.to_string().len();
            Self::fresh_install(view, &mut installed, &mut self.counters);
            view.pending.all = true;
            view.pending.structural = true;
            view.pending.record = true;
            structural = true;
        }
        if installed.scalars_sequence != view.fields_sequence {
            installed.scalars = view.fields.clone();
            installed.scalars_sequence = view.fields_sequence;
            installed.scalars_bytes = view.fields.to_string().len();
            view.pending.record = true;
        }
        let reconcile = std::mem::take(&mut view.reconcile);
        if !reconcile.is_empty() {
            // Messages and plain fields splice first so anchored items see the
            // settled ordinal map, then anchored fields run their splices.
            let mut anchored = Vec::new();
            // Fresh assembly sorts compactions before messages. Its anchors
            // need one final remap after the loaded message window is known.
            let mut messages_moved = fresh;
            for sid in reconcile {
                let sid: Arc<str> = Arc::from(sid.as_str());
                match view.rows.get(sid.as_ref()) {
                    Some((_, row)) if Self::anchored_field(&row.field) => anchored.push(sid),
                    Some((_, row)) => {
                        messages_moved |= match installed.slot.get(&sid) {
                            Some(slot) => {
                                (slot.field == "messages" || row.field == "messages")
                                    && (slot.field != row.field || slot.ordinal != row.ordinal)
                            }
                            None => row.field == "messages",
                        };
                        Self::upsert_row(view, &mut installed, &mut self.counters, &sid);
                    }
                    None => {
                        messages_moved |= installed
                            .slot
                            .get(&sid)
                            .is_some_and(|slot| slot.field == "messages");
                        Self::remove_item(
                            &mut installed,
                            &mut view.pending,
                            &mut self.counters,
                            &sid,
                        );
                    }
                }
            }
            if messages_moved {
                Self::reanchor(view, &mut installed, &mut self.counters, &mut structural);
                remap_retries = true;
            }
            for sid in anchored {
                Self::upsert_row(view, &mut installed, &mut self.counters, &sid);
            }
            structural |= view.pending.structural;
        }
        if remap_retries {
            Self::remap_work_retries(view, &mut installed);
        }
        Self::recompute_deferred(view, &mut installed, self.deferred_work);
        view.pending.structural |= structural;
        installed.bytes = installed
            .scalars_bytes
            .saturating_add(installed.item_bytes)
            .saturating_add(
                installed
                    .deferred
                    .iter()
                    .map(|m| m.to_string().len())
                    .sum::<usize>(),
            )
            .saturating_add(installed.slot.len().saturating_mul(2))
            .saturating_add(installed.fields.len().saturating_mul(8))
            .saturating_add(64);
        if installed
            .bytes
            .saturating_add(view.row_bytes)
            .saturating_add(installed.scalars_bytes)
            > protocol::MAX_SNAPSHOT_BYTES
        {
            self.evict(thread);
            return Err("Thread history exceeded the client memory budget.".into());
        }
        view.installed = Some(installed);
        self.history_order.retain(|id| *id != thread);
        self.history_order.push_back(thread);
        while self.retained_history_bytes() > protocol::MAX_SNAPSHOT_BYTES {
            let Some(oldest) = self.history_order.pop_front() else {
                break;
            };
            self.evict(oldest);
        }
        self.touched.insert(thread);
        self.dirty = true;
        Ok(())
    }

    fn evict(&mut self, thread: u64) {
        self.views.remove(&thread);
        self.emitted.remove(&thread);
        self.history_order.retain(|id| *id != thread);
        self.touched.insert(thread);
        self.dirty = true;
        if self.subscribed == Some(thread) {
            self.subscribed = None;
        }
        // A waiting RPC uses the shared history response id. Drain its
        // response before sending another; an unsent walk can be dropped.
        if self
            .inflight
            .as_ref()
            .is_some_and(|walk| walk.thread == thread && !walk.waiting)
        {
            self.inflight = None;
        }
    }

    /// Conservative serialized-history retention, not a process RSS limit.
    /// Count source rows, installed payloads and old emitted field versions.
    /// Shared installed/emitted field allocations count once. A replaced field
    /// counts in full until its old emission drops, without scanning its items.
    /// External mailbox/UI owners release old Arcs when they apply an eviction
    /// emission; the cache cannot account for arbitrary external snapshots.
    fn retained_history_bytes(&self) -> usize {
        let installed = self
            .views
            .values()
            .map(|view| {
                view.row_bytes.saturating_add(
                    view.installed
                        .as_ref()
                        .map_or(0, |i| i.bytes.saturating_add(i.scalars_bytes)),
                )
            })
            .sum::<usize>();
        self.emitted.iter().fold(installed, |bytes, (id, emitted)| {
            emitted.fields.iter().fold(
                bytes.saturating_add(emitted.record_bytes),
                |bytes, (field, items)| {
                    if self
                        .views
                        .get(id)
                        .and_then(|view| view.installed.as_ref())
                        .and_then(|installed| installed.fields.get(field))
                        .is_some_and(|installed| Arc::ptr_eq(installed, items))
                    {
                        bytes
                    } else {
                        bytes.saturating_add(items.bytes)
                    }
                },
            )
        })
    }

    /// How many threads hold an installed history view.
    #[cfg(test)]
    pub(crate) fn installed_count(&self) -> usize {
        self.views
            .values()
            .filter(|view| view.installed.is_some())
            .count()
    }

    /// Revision, assembled record and byte estimate for the installed view.
    /// Used by the integration test crate, which sees a fresh crate root.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn installed_info(&self, thread: u64) -> Option<(String, Value, usize)> {
        let view = self.views.get(&thread)?;
        let installed = view.installed.as_ref()?;
        Some((
            view.revision.clone(),
            self.assembled(thread)?,
            installed.bytes,
        ))
    }

    /// The assembled wire-shaped history record, materialized on demand for
    /// tests and the full-snapshot convergence check.
    #[cfg(test)]
    pub(crate) fn assembled(&self, thread: u64) -> Option<Value> {
        let view = self.views.get(&thread)?;
        let installed = view.installed.as_ref()?;
        let mut history = installed.scalars.clone();
        history["id"] = json!(thread);
        for field in protocol::HISTORY_FIELDS {
            if history.get(field).is_none() {
                history[field] = json!([]);
            }
            if let Some(items) = installed.fields.get(field) {
                history[field] =
                    Value::Array(items.items.iter().map(|item| (**item).clone()).collect());
            }
        }
        if self.deferred_work {
            history["deferredWork"] = json!(installed.deferred);
        }
        Some(history)
    }

    /// Unknown event kinds advance the sequence without changing known records, as in
    /// KSG's unknown-event stream member. Malformed known records still fail validation.
    fn stream(&mut self, item: &Value) -> Result<(), String> {
        let thread = item["threadId"]
            .as_u64()
            .ok_or("Invalid thread stream id.")?;
        let kind = item["kind"].as_str().ok_or("Invalid thread stream kind.")?;
        if kind == "snapshot-required" {
            if self.subscribed == Some(thread) {
                self.failed(
                    thread,
                    "Thread replay is unavailable. Reloading a scoped snapshot.",
                    true,
                );
            }
            return Ok(());
        }
        let view = self.views.entry(thread).or_default();
        if item["streamEpoch"].as_str() != Some(view.epoch.as_str()) {
            return Ok(());
        }
        let sequence = item["sequence"]
            .as_u64()
            .ok_or("Invalid thread stream sequence.")?;
        if sequence <= view.sequence {
            return Ok(());
        }
        if kind == "deleted" {
            self.views.remove(&thread);
            self.emitted.remove(&thread);
            self.history_order.retain(|id| *id != thread);
            self.dirty = true;
            return Ok(());
        }
        if kind == "event" {
            for event in item["events"].as_array().ok_or("Invalid thread events.")? {
                match event["type"].as_str() {
                    Some("item.updated") => {
                        let mut row: Row = serde_json::from_value(event["row"].clone())
                            .map_err(|e| e.to_string())?;
                        if row.source_thread_id != thread {
                            return Err("Cross-thread live item.".into());
                        }
                        if !protocol::HISTORY_FIELDS.contains(&row.field.as_str()) {
                            continue;
                        }
                        if view
                            .rows
                            .get(&row.source_item_id)
                            .is_some_and(|(seq, _)| *seq > sequence)
                            || view
                                .removed
                                .get(&row.source_item_id)
                                .is_some_and(|seq| *seq > sequence)
                        {
                            continue;
                        }
                        if let Some((_, previous)) = view.rows.get(&row.source_item_id)
                            && let Some(marker) = previous.item.get("deferredWork")
                            && row.item.get("deferredWork").is_none()
                        {
                            row.item["deferredWork"] = marker.clone();
                        }
                        view.removed.remove(&row.source_item_id);
                        view.reconcile.insert(row.source_item_id.clone());
                        view.insert_row(sequence, row);
                    }
                    Some("item.removed") => {
                        let id = event["sourceItemId"]
                            .as_str()
                            .ok_or("Invalid removed item.")?;
                        if view.rows.get(id).is_some_and(|(seq, _)| *seq > sequence) {
                            continue;
                        }
                        view.remove_row(id);
                        view.reconcile.insert(id.into());
                        view.removed.insert(id.into(), sequence);
                    }
                    Some("fields.updated") => {
                        if sequence >= view.fields_sequence {
                            view.fields = event["fields"]
                                .as_object()
                                .ok_or("Invalid history fields.")?
                                .clone()
                                .into();
                            view.fields_sequence = sequence;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(revision) = item["historyRevision"].as_str() {
                view.revision = revision.into();
            }
        }
        view.sequence = sequence;
        if self.inflight.is_none() {
            view.removed.clear();
        }
        self.install(thread)
    }

    fn merge_page(
        &mut self,
        thread: u64,
        page: Value,
        snapshot: bool,
        epoch: &str,
        work_anchor: Option<&str>,
    ) -> Result<(), String> {
        if page["threadId"] != thread {
            return Err("Cross-thread history page.".into());
        }
        let sequence = page["snapshotSequence"]
            .as_u64()
            .ok_or("Invalid history sequence.")?;
        if snapshot {
            self.views.insert(
                thread,
                View {
                    epoch: epoch.into(),
                    sequence,
                    ..View::default()
                },
            );
        }
        let view = self.views.get_mut(&thread).ok_or("Missing history base.")?;
        let previous_cursor = view.cursor.clone();
        let previous_through = view.through_entry_id.clone();
        let items = page["items"].as_array().ok_or("Invalid history items.")?;
        let mut page_has_turn_start = false;
        for row in items {
            let row: Row = serde_json::from_value(row.clone()).map_err(|e| e.to_string())?;
            if row.source_thread_id != thread {
                return Err("Cross-thread history item.".into());
            }
            if !protocol::HISTORY_FIELDS.contains(&row.field.as_str()) {
                continue;
            }
            if row.field == "messages" && matches!(row.item["role"].as_str(), Some("You" | "user"))
            {
                page_has_turn_start = true;
            }
            let id = &row.source_item_id;
            // Newer live records and removals win over concurrently read older pages.
            if view.rows.get(id).is_some_and(|(seq, _)| *seq >= sequence)
                || view.removed.get(id).is_some_and(|seq| *seq >= sequence)
            {
                continue;
            }
            view.reconcile.insert(row.source_item_id.clone());
            view.insert_row(sequence, row);
        }
        if sequence >= view.fields_sequence {
            view.fields = page["fields"]
                .as_object()
                .ok_or("Invalid history fields.")?
                .clone()
                .into();
            view.fields_sequence = sequence;
        }
        if sequence >= view.sequence {
            view.revision = page["historyRevision"]
                .as_str()
                .ok_or("Invalid history revision.")?
                .into();
        }
        if let Some(anchor) = work_anchor {
            let next = page["nextCursor"].as_str();
            if let Some((sequence, mut row)) = view.rows.get(anchor).cloned() {
                if let Some(next) = next {
                    row.item["deferredWork"]["cursor"] = json!(next);
                } else {
                    row.item.as_object_mut().unwrap().remove("deferredWork");
                }
                view.reconcile.insert(anchor.into());
                view.insert_row(sequence, row);
            }
            if next.is_none() {
                view.work_requests.retain(|request| request != anchor);
            }
            view.expanded = true;
            view.pending.record = true;
            return self.install(thread);
        }
        view.cursor = page["nextCursor"].as_str().map(str::to_owned);
        view.has_more = page["hasMoreHistory"]
            .as_bool()
            .unwrap_or(view.cursor.is_some());
        if snapshot {
            view.loading = false;
            view.error = None;
            view.expanded = false;
            view.load_requested = false;
            view.through_entry_id = None;
        } else {
            view.expanded = true;
            view.error = None;
            let keep_paging = if let Some(target) = &previous_through {
                let target_loaded = view.rows.values().any(|(_, r)| {
                    r.source_item_id == *target || r.item["id"].as_str() == Some(target.as_str())
                });
                !target_loaded && view.has_more && view.cursor.is_some()
            } else {
                view.has_more && view.cursor.is_some() && !page_has_turn_start
            };
            if keep_paging && (view.cursor.is_none() || view.cursor == previous_cursor) {
                view.loading = false;
                view.load_requested = false;
                view.through_entry_id = None;
                view.error = Some("History did not advance.".into());
            } else {
                view.loading = keep_paging;
                view.load_requested = keep_paging;
                if !keep_paging {
                    view.through_entry_id = None;
                }
            }
        }
        view.ready = true;
        // Older history reads do not advance the live replay cursor.
        self.install(thread)
    }

    pub(crate) fn receive(&mut self, value: &Value) -> Result<(), String> {
        let params = &value["params"];
        match value["method"].as_str() {
            Some("subscription.snapshotPage") => {
                self.snapshot_pages
                    .push_str(params["data"].as_str().ok_or("Invalid snapshot page.")?);
                if self.snapshot_pages.len() > protocol::MAX_SNAPSHOT_BYTES {
                    return Err("Workspace metadata exceeded the client memory budget.".into());
                }
                if params["nextCursor"].is_null() {
                    let snapshot = serde_json::from_str(&std::mem::take(&mut self.snapshot_pages))
                        .map_err(|e| e.to_string())?;
                    self.receive(&protocol::notification("subscription.snapshot", snapshot))?;
                }
            }
            Some("subscription.snapshot") => {
                self.state = params["state"].clone();
                self.seq = params["seq"].as_u64();
                self.epoch = params["epoch"].clone();
                self.maintenance = params["maintenance"].clone();
                self.rebuild = true;
                self.dirty = true;
            }
            Some("subscription.ready") if params.get("maintenance").is_some() => {
                self.maintenance = params["maintenance"].clone();
            }
            Some("subscription.patch") => {
                let seq = params["seq"].as_u64().ok_or("Invalid patch sequence.")?;
                if self.seq.is_some_and(|old| seq <= old) {
                    return Ok(());
                }
                if self.seq.and_then(|seq| seq.checked_add(1)) != Some(seq)
                    || self.epoch != params["epoch"]
                {
                    return Err("Projection replay gap. Reconnecting for a snapshot.".into());
                }
                // Path indexes name positions in the array after each op runs;
                // resolving them post-apply marks the record actually touched.
                let patch = params["patch"].clone();
                protocol::apply(&mut self.state, patch.clone())?;
                self.seq = Some(seq);
                let mut all_threads = false;
                let mut positions = Vec::new();
                for op in patch.as_array().into_iter().flatten() {
                    match op["path"].as_array().map(Vec::as_slice) {
                        None | Some([]) => all_threads = true,
                        Some([first]) if first == "threads" => all_threads = true,
                        Some([first, position, ..]) if first == "threads" => {
                            if let Some(index) =
                                position.as_str().and_then(|i| i.parse::<usize>().ok())
                            {
                                positions.push(index);
                            }
                        }
                        _ => {}
                    }
                }
                let marked: Vec<u64> = if all_threads {
                    self.state["threads"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|thread| thread["id"].as_u64())
                        .collect()
                } else {
                    positions
                        .into_iter()
                        .filter_map(|index| self.state["threads"][index]["id"].as_u64())
                        .collect()
                };
                for id in marked {
                    if let Some(view) = self.views.get_mut(&id) {
                        view.pending.record = true;
                    }
                    self.touched.insert(id);
                }
                self.dirty = true;
            }
            Some("thread.streamPage") => {
                self.stream_pages
                    .push_str(params["data"].as_str().ok_or("Invalid stream page.")?);
                if self.stream_pages.len() > protocol::MAX_SNAPSHOT_BYTES {
                    return Err("Thread stream exceeded the client memory budget.".into());
                }
                if params["nextCursor"].is_null() {
                    let item = serde_json::from_str(&std::mem::take(&mut self.stream_pages))
                        .map_err(|e| e.to_string())?;
                    self.receive(&protocol::notification("thread.stream", item))?;
                }
            }
            Some("thread.stream") => {
                if let Some(walk) = self
                    .inflight
                    .as_mut()
                    .filter(|walk| walk.subscribe || walk.snapshot)
                    .filter(|walk| params["threadId"] == walk.thread)
                {
                    walk.pending_bytes += params.to_string().len();
                    if walk.pending_bytes > protocol::MAX_SNAPSHOT_BYTES {
                        return Err(
                            "Pending thread stream exceeded the client memory budget.".into()
                        );
                    }
                    walk.pending.push(params.clone());
                } else {
                    self.stream(params)?;
                }
            }
            _ if value["id"] == "history" => {
                let Some(mut walk) = self.inflight.take() else {
                    return Ok(());
                };
                if self.desired_thread.is_some_and(|id| id != walk.thread)
                    || self.state["threads"].as_array().is_some_and(|threads| {
                        !threads.iter().any(|thread| thread["id"] == walk.thread)
                    })
                {
                    return Ok(());
                }
                if !walk.subscribe && !walk.snapshot && !self.views.contains_key(&walk.thread) {
                    // An earlier/work page that was in flight at eviction no
                    // longer has a base. Drain it and request a fresh snapshot.
                    return Ok(());
                }
                if !value["error"].is_null() {
                    let code = value["error"]["code"].as_i64();
                    let message = value["error"]["message"]
                        .as_str()
                        .unwrap_or("History read failed.");
                    if let Some(anchor) = &walk.work_anchor {
                        if let Some(view) = self.views.get_mut(&walk.thread) {
                            view.work_requests.retain(|request| request != anchor);
                            view.work_errors.insert(anchor.clone(), message.into());
                        }
                        self.install(walk.thread)?;
                        return Ok(());
                    }
                    if code == Some(protocol::HISTORY_UPDATE_REQUIRED_CODE) {
                        self.update_required = true;
                    }
                    if !walk.subscribe
                        && !walk.snapshot
                        && code != Some(protocol::HISTORY_LEASE_GONE_CODE)
                    {
                        if let Some(view) = self.views.get_mut(&walk.thread) {
                            view.loading = false;
                            view.load_requested = false;
                            view.through_entry_id = None;
                            view.error = Some(message.into());
                        }
                        self.touched.insert(walk.thread);
                        self.dirty = true;
                    } else {
                        self.failed(
                            walk.thread,
                            message,
                            code == Some(protocol::HISTORY_LEASE_GONE_CODE),
                        );
                    }
                    return Ok(());
                }
                let result = &value["result"];
                if result["kind"] == "synchronized" {
                    self.subscribed = Some(walk.thread);
                    for item in walk.pending {
                        self.stream(&item)?;
                    }
                    return Ok(());
                }
                if walk.subscribe {
                    walk.subscribe = false;
                    walk.snapshot = true;
                    // The stream epoch stays fixed through this fragmented snapshot.
                    walk.cursor = result["streamEpoch"].as_str().map(str::to_owned);
                }
                walk.text
                    .push_str(result["data"].as_str().ok_or("Invalid history fragment.")?);
                if walk.text.len() > protocol::MAX_SNAPSHOT_BYTES {
                    return Err("Thread history exceeded the client memory budget.".into());
                }
                walk.fragment = result["nextFragment"].clone();
                if walk.fragment.is_null() {
                    let page: Value =
                        serde_json::from_str(&walk.text).map_err(|e| e.to_string())?;
                    if let Some(anchor) = &walk.work_anchor {
                        let cursor = self
                            .views
                            .get(&walk.thread)
                            .and_then(|view| view.rows.get(anchor))
                            .and_then(|(_, row)| row.item["deferredWork"]["cursor"].as_str());
                        if cursor != walk.cursor.as_deref() {
                            if let Some(view) = self.views.get_mut(&walk.thread) {
                                view.work_requests.retain(|request| request != anchor);
                                view.work_errors.insert(
                                    anchor.clone(),
                                    "Work history changed. Retry loading it.".into(),
                                );
                            }
                            self.install(walk.thread)?;
                            return Ok(());
                        }
                        if page["nextCursor"].as_str() == walk.cursor.as_deref() {
                            if let Some(view) = self.views.get_mut(&walk.thread) {
                                view.work_requests.retain(|request| request != anchor);
                                view.work_errors
                                    .insert(anchor.clone(), "Work history did not advance.".into());
                            }
                            self.install(walk.thread)?;
                            return Ok(());
                        }
                    }
                    self.merge_page(
                        walk.thread,
                        page,
                        walk.snapshot,
                        walk.cursor.as_deref().unwrap_or(""),
                        walk.work_anchor.as_deref(),
                    )?;
                    if walk.snapshot {
                        self.subscribed = Some(walk.thread);
                    }
                    for item in walk.pending {
                        self.stream(&item)?;
                    }
                    if self.subscribed == Some(walk.thread)
                        && self
                            .views
                            .get(&walk.thread)
                            .is_some_and(|view| view.ready && view.cursor.is_none())
                        && let Some(retry) = self.retries.get_mut(&walk.thread)
                    {
                        retry.attempts = 0;
                        retry.error = None;
                    }
                    if let Some(view) = self.views.get_mut(&walk.thread) {
                        view.removed.clear();
                    }
                } else {
                    walk.waiting = false;
                    self.inflight = Some(walk);
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn next_history(&mut self) -> Option<Value> {
        if self.update_required {
            return None;
        }
        if self.inflight.is_none() {
            let desired = self
                .desired_thread
                .or_else(|| self.state["index"]["active"].as_u64());
            let thread = self.state["threads"]
                .as_array()?
                .iter()
                .find(|thread| desired.is_none_or(|id| thread["id"] == id))?["id"]
                .as_u64()?;
            if self.retries.get(&thread).is_some_and(|r| r.blocked) {
                return None;
            }
            let subscribe = self.subscribed != Some(thread)
                || !self
                    .views
                    .get(&thread)
                    .is_some_and(|view| view.ready && view.installed.is_some());
            if let Some(view) = self.views.get_mut(&thread) {
                view.work_requests.retain(|anchor| {
                    view.rows
                        .get(anchor)
                        .is_some_and(|(_, row)| row.item["deferredWork"].is_object())
                });
            }
            let view = self.views.get(&thread);
            let work_anchor = if !subscribe {
                view.and_then(|view| view.work_requests.front().cloned())
            } else {
                None
            };
            let cursor = if let Some(anchor) = &work_anchor {
                view.and_then(|view| view.rows.get(anchor))
                    .and_then(|(_, row)| {
                        row.item["deferredWork"]["cursor"]
                            .as_str()
                            .map(str::to_owned)
                    })
            } else {
                view.and_then(|view| view.cursor.clone())
            };
            let want_earlier =
                view.is_some_and(|view| view.load_requested && view.cursor.is_some());
            if !subscribe && !want_earlier && work_anchor.is_none() {
                return None;
            }
            let through_entry_id = view.and_then(|view| view.through_entry_id.clone());
            self.inflight = Some(History {
                thread,
                subscribe,
                snapshot: false,
                cursor,
                through_entry_id,
                fragment: Value::Null,
                text: String::new(),
                waiting: false,
                pending: Vec::new(),
                pending_bytes: 0,
                work_anchor,
            });
        }
        let walk = self.inflight.as_mut()?;
        if walk.waiting {
            return None;
        }
        walk.waiting = true;
        let mut params = json!({"threadId":walk.thread,"byteBudget":protocol::PAGE_BYTES});
        let method = if walk.subscribe {
            if let Some(view) = self
                .views
                .get(&walk.thread)
                .filter(|view| view.ready && !view.epoch.is_empty())
            {
                params["afterSequence"] = json!(view.sequence);
                params["streamEpoch"] = json!(view.epoch);
            }
            "thread.subscribe"
        } else {
            if !walk.fragment.is_null() {
                params["fragmentCursor"] = walk.fragment.clone();
            } else if let Some(cursor) = &walk.cursor {
                params["historyCursor"] = json!(cursor);
                if let Some(anchor) = &walk.work_anchor {
                    params["workAnchorId"] = json!(anchor);
                }
                if let Some(through) = &walk.through_entry_id {
                    params["throughEntryId"] = json!(through);
                }
            }
            "thread.history"
        };
        Some(json!({"jsonrpc":"2.0","id":"history","method":method,"params":params}))
    }

    /// The workspace projection, split so an unchanged thread's emitted record
    /// is shared by `Arc` identity instead of cloned. `changed` names the ids
    /// whose record differs from the previous emission; `None` rebuilds all.
    pub(crate) fn projected(&mut self) -> Option<Projection> {
        if !self.dirty || self.seq.is_none() {
            return None;
        }
        self.dirty = false;
        let live: HashSet<u64> = self.state["threads"]
            .as_array()
            .map(|threads| {
                threads
                    .iter()
                    .filter_map(|thread| thread["id"].as_u64())
                    .collect()
            })
            .unwrap_or_default();
        self.views.retain(|id, _| live.contains(id));
        self.retries.retain(|id, _| live.contains(id));
        self.emitted.retain(|id, _| live.contains(id));
        self.history_order.retain(|id| self.views.contains_key(id));
        self.touched.retain(|id| live.contains(id));
        self.counters.projections += 1;
        let rebuild = self.rebuild || (self.emitted.is_empty() && !live.is_empty());
        self.rebuild = false;
        if rebuild {
            self.counters.full += 1;
            self.touched.clear();
        } else {
            self.counters.delta += 1;
        }
        let changed = (!rebuild).then(|| Arc::new(std::mem::take(&mut self.touched)));
        let mut views = HashMap::with_capacity(live.len());
        let mut compact = Vec::with_capacity(live.len());
        if let Some(threads) = self.state["threads"].as_array() {
            for record in threads {
                let Some(id) = record["id"].as_u64() else {
                    compact.push(record.clone());
                    continue;
                };
                let share = changed
                    .as_ref()
                    .is_some_and(|changed| !changed.contains(&id));
                let emitted = match self.emitted.get(&id) {
                    Some(emitted) if share => {
                        self.counters.threads_shared += 1;
                        emitted.clone()
                    }
                    _ => {
                        self.counters.threads_rebuilt += 1;
                        let mut thread = record.clone();
                        for field in protocol::HISTORY_FIELDS {
                            thread[field] = if field == "stats" {
                                json!({})
                            } else {
                                json!([])
                            };
                        }
                        let mut fields = HashMap::new();
                        let mut history_loaded = false;
                        let mut deferred: Option<Vec<Value>> = None;
                        let mut structural = true;
                        let mut items_changed = None;
                        let mut record_changed = true;
                        if let Some(view) = self.views.get_mut(&id) {
                            if let Some(installed) = view.installed.as_ref() {
                                history_loaded = true;
                                if let Some(scalars) = installed.scalars.as_object() {
                                    for (key, value) in scalars {
                                        thread[key] = value.clone();
                                    }
                                }
                                fields = installed.fields.clone();
                                deferred = Some(installed.deferred.clone());
                            }
                            thread["historyMeta"] = json!({
                                "hasMoreHistory": view.has_more,
                                "loading": view.loading,
                                "error": view.error,
                                "expanded": view.expanded,
                                "deferredWork": deferred,
                                "workLoading": if self.deferred_work { "deferred" } else { "inline-legacy" },
                            });
                            let pending = std::mem::take(&mut view.pending);
                            structural = pending.structural;
                            items_changed = (!pending.all).then(|| Arc::new(pending.items));
                            record_changed = pending.record;
                        }
                        if let Some(retry) = self.retries.get(&id) {
                            thread["historyRetry"] = json!({"count":retry.attempts,"limit":1,"fullRestarts":retry.restarts,
                                "stopped":retry.blocked,"error":retry.error});
                        }
                        // Invalidation is relative to emitted state, while UI
                        // application compares Arcs to its own last apply.
                        // A full snapshot invalidates even unchanged item IDs.
                        structural |= rebuild || !self.emitted.contains_key(&id);
                        if let Some(previous) = self.emitted.get(&id) {
                            structural |= previous.record["harness_state"]["workRetries"]
                                != thread["harness_state"]["workRetries"];
                            structural |= previous.record["historyMeta"]["deferredWork"]
                                != thread["historyMeta"]["deferredWork"];
                            // The record is compact. Comparing it includes
                            // history loading/errors and retry transitions
                            // without walking or serializing item payloads.
                            record_changed |= previous.record.as_ref() != &thread;
                        }
                        if rebuild {
                            items_changed = None;
                            record_changed = true;
                        }
                        let content_rev = next_revision();
                        let structure_rev = if structural {
                            content_rev
                        } else {
                            self.emitted
                                .get(&id)
                                .map_or(content_rev, |old| old.structure)
                        };
                        if let Some(previous) = self.emitted.get(&id) {
                            for (name, items) in &fields {
                                if previous
                                    .fields
                                    .get(name)
                                    .is_some_and(|old| Arc::ptr_eq(old, items))
                                {
                                    self.counters.fields_shared += 1;
                                }
                            }
                        }
                        let record_bytes = thread.to_string().len();
                        let emitted = Arc::new(ThreadProjection {
                            record: Arc::new(thread),
                            fields,
                            history_loaded,
                            structure: structure_rev,
                            content: content_rev,
                            structural,
                            items_changed,
                            record_changed,
                            record_bytes,
                        });
                        self.emitted.insert(id, emitted.clone());
                        emitted
                    }
                };
                // `state["threads"]` stays compact; the transcript lives in `views`.
                let mut entry = record.clone();
                for field in protocol::HISTORY_FIELDS {
                    entry[field] = if field == "stats" {
                        json!({})
                    } else {
                        json!([])
                    };
                }
                if let Some(meta) = emitted.record.get("historyMeta") {
                    entry["historyMeta"] = meta.clone();
                }
                if let Some(retry) = emitted.record.get("historyRetry") {
                    entry["historyRetry"] = retry.clone();
                }
                views.insert(id, emitted);
                compact.push(entry);
            }
        }
        let mut state = self.state.clone();
        state["historyUpdateRequired"] = json!(self.update_required);
        state["threads"] = json!(compact);
        Some(Projection {
            state,
            views,
            changed,
        })
    }

    /// Pinned pre-#24 baseline (269a2d0), ported verbatim for benchmark
    /// provenance: every stream event or page merge rebuilt the thread's
    /// complete wire record from `view.rows`, cloned every row payload, and
    /// serialized the whole record to estimate bytes.
    #[cfg(test)]
    pub(crate) fn legacy_install(&self, thread: u64) -> Option<(String, Value, usize)> {
        let view = self.views.get(&thread).filter(|view| view.ready)?;
        let mut history = view.fields.clone();
        history["id"] = json!(thread);
        let mut rows: Vec<_> = view.rows.values().map(|(_, row)| row).collect();
        rows.sort_by(|a, b| {
            a.field
                .cmp(&b.field)
                .then(a.ordinal.cmp(&b.ordinal))
                .then(a.source_item_id.cmp(&b.source_item_id))
        });
        let mut ordinal_to_local = HashMap::new();
        let mut min_msg_ordinal = None::<usize>;
        let mut max_msg_ordinal = None::<usize>;
        let mut local_msg_count = 0usize;
        for row in &rows {
            if row.field == "messages" {
                ordinal_to_local.insert(row.ordinal, local_msg_count);
                min_msg_ordinal = Some(min_msg_ordinal.map_or(row.ordinal, |m| m.min(row.ordinal)));
                max_msg_ordinal = Some(max_msg_ordinal.map_or(row.ordinal, |m| m.max(row.ordinal)));
                local_msg_count += 1;
            }
        }
        let remap_msg = |server_ord: usize| -> Option<usize> {
            if let Some(&local) = ordinal_to_local.get(&server_ord) {
                Some(local)
            } else if max_msg_ordinal.is_some_and(|max| server_ord > max) {
                Some(local_msg_count)
            } else if min_msg_ordinal.is_some() {
                None
            } else {
                Some(server_ord)
            }
        };
        let deferred: Vec<Value> = rows
            .iter()
            .filter_map(|row| {
                let mut marker = row
                    .item
                    .get("deferredWork")
                    .filter(|marker| marker.is_object())?
                    .clone();
                marker["sourceItemId"] = json!(row.source_item_id);
                marker["start"] = json!(remap_msg(marker["start"].as_u64()? as usize)?);
                marker["loading"] = json!(view.work_requests.contains(&row.source_item_id));
                marker["error"] = json!(view.work_errors.get(&row.source_item_id));
                Some(marker)
            })
            .collect();
        if self.deferred_work {
            history["deferredWork"] = json!(deferred);
        }
        for field in protocol::HISTORY_FIELDS {
            if history.get(field).is_none() {
                history[field] = json!([]);
            }
        }
        for row in rows {
            if !protocol::HISTORY_FIELDS.contains(&row.field.as_str()) {
                continue;
            }
            if !history[&row.field].is_array() {
                history[&row.field] = json!([]);
            }
            let mut item = row.item.clone();
            match row.field.as_str() {
                "tools" | "thoughts" | "compactions" => {
                    if let Some(server_ord) = item["anchor"]["message"].as_u64() {
                        let Some(local) = remap_msg(server_ord as usize) else {
                            continue;
                        };
                        item["anchor"]["message"] = json!(local);
                    }
                }
                "work" => {
                    if let Some(server_ord) = item["start"].as_u64() {
                        let Some(local) = remap_msg(server_ord as usize) else {
                            continue;
                        };
                        item["start"] = json!(local);
                    }
                }
                _ => {}
            }
            history[&row.field].as_array_mut().unwrap().push(item);
        }
        if let Some(retries) = history
            .get_mut("harness_state")
            .and_then(|hs| hs.get_mut("workRetries"))
            .and_then(Value::as_array_mut)
        {
            retries.retain_mut(|retry| {
                if let Some(server_ord) = retry["anchor"]["message"].as_u64() {
                    let Some(local) = remap_msg(server_ord as usize) else {
                        return false;
                    };
                    retry["anchor"]["message"] = json!(local);
                }
                true
            });
        }
        let bytes = history.to_string().len();
        if bytes > protocol::MAX_SNAPSHOT_BYTES {
            return None;
        }
        Some((view.revision.clone(), history, bytes))
    }

    /// Pinned pre-#24 baseline (269a2d0): every emission cloned the workspace
    /// state and spliced a fresh copy of every loaded history field into each
    /// thread record.
    #[cfg(test)]
    pub(crate) fn legacy_projected(
        &self,
        histories: &HashMap<u64, (String, Value, usize)>,
    ) -> Value {
        let mut state = self.state.clone();
        state["historyUpdateRequired"] = json!(self.update_required);
        if let Some(threads) = state["threads"].as_array_mut() {
            for thread in threads {
                let Some(id) = thread["id"].as_u64() else {
                    continue;
                };
                if let Some((_, history, _)) = histories.get(&id) {
                    for field in protocol::HISTORY_FIELDS {
                        thread[field] = history[field].clone();
                    }
                }
            }
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Fragments, Streams};
    use std::time::Instant;

    fn snapshot(seq: u64, thread: Value) -> Value {
        json!({"epoch":"workspace","seq":seq,"state":{"index":{"active":1},"threads":[thread]}})
    }

    fn cache(snapshot: &Value) -> ProjectionCache {
        let mut cache = ProjectionCache::default();
        cache
            .reconnect(&json!({"capabilities":[protocol::HISTORY_ITEMS,"future-feature"]}))
            .unwrap();
        cache
            .receive(&protocol::notification(
                "subscription.snapshot",
                protocol::compact(snapshot),
            ))
            .unwrap();
        cache
    }

    fn answer(cache: &mut ProjectionCache, streams: &Streams, fragments: &mut Fragments) -> Value {
        answer_with_budget(cache, streams, fragments, 1024)
    }

    fn answer_with_budget(
        cache: &mut ProjectionCache,
        streams: &Streams,
        fragments: &mut Fragments,
        budget: usize,
    ) -> Value {
        let request = cache.next_history().expect("history request");
        assert!(cache.next_history().is_none(), "one in-flight request");
        let params = &request["params"];
        let thread = params["threadId"].as_u64().unwrap();
        let result = if request["method"] == "thread.subscribe" {
            if params["afterSequence"].is_u64()
                && params["streamEpoch"] == "stream"
                && let Some(events) =
                    streams.replay(thread, params["afterSequence"].as_u64().unwrap())
            {
                for mut event in events {
                    event["streamEpoch"] = json!("stream");
                    cache
                        .receive(&protocol::notification("thread.stream", event))
                        .unwrap();
                }
                json!({"kind":"synchronized","threadId":thread,"snapshotSequence":streams.high_water(thread),"streamEpoch":"stream"})
            } else {
                let mut page = fragments
                    .read(thread, params, Instant::now(), || {
                        streams.page(thread, None, budget)
                    })
                    .unwrap();
                page["kind"] = json!("snapshot");
                page["streamEpoch"] = json!("stream");
                page
            }
        } else {
            fragments
                .read(thread, params, Instant::now(), || {
                    streams.page(thread, params["historyCursor"].as_str(), budget)
                })
                .unwrap()
        };
        cache
            .receive(&json!({"id":"history","result":result}))
            .unwrap();
        request
    }

    fn large_page(thread: u64, bytes: usize) -> Value {
        json!({
            "threadId":thread, "snapshotSequence":1, "historyRevision":"1",
            "nextCursor":null, "hasMoreHistory":false, "fields":{"id":thread,"stats":{}},
            "items":[{"sourceThreadId":thread, "sourceItemId":format!("messages:{thread}"),
                "field":"messages", "ordinal":0, "position":0,
                "item":{"id":format!("m-{thread}"),"role":"Pi","text":"x".repeat(bytes)}}]
        })
    }

    #[test]
    fn global_eviction_releases_emissions_and_reloads_a_subscribed_thread() {
        let mut cache = cache(&json!({"epoch":"workspace","seq":1,
            "state":{"index":{"active":1},"threads":[{"id":1},{"id":2},{"id":3}]}}));
        let size = 12 * 1024 * 1024;
        cache
            .merge_page(1, large_page(1, size), true, "stream", None)
            .unwrap();
        cache.subscribed = Some(1);
        let first = cache.projected().unwrap();
        let retained = Arc::downgrade(&first.views[&1].fields["messages"].items[0]);
        drop(first);
        for thread in [2, 3] {
            cache
                .merge_page(thread, large_page(thread, size), true, "stream", None)
                .unwrap();
            drop(cache.projected().unwrap());
        }
        assert!(
            !cache.views.contains_key(&1),
            "source and installed copies count toward the budget"
        );
        assert!(
            retained.upgrade().is_none(),
            "the cache must drop its emitted item too"
        );

        cache
            .receive(&protocol::notification(
                "subscription.patch",
                json!({"epoch":"workspace","seq":2,
                "patch":[{"op":"set","path":["threads","0","title"],"value":"still live"}]}),
            ))
            .unwrap();
        let evicted = cache.projected().unwrap();
        assert!(evicted.views[&1].fields.is_empty());
        assert!(evicted.views[&1].structural);
        assert_eq!(evicted.views[&1].record["title"], "still live");
        drop(evicted);
        let request = cache
            .next_history()
            .expect("the evicted subscription must reload");
        assert_eq!(request["method"], "thread.subscribe");
        assert!(
            request["params"]["afterSequence"].is_null(),
            "never replay from an evicted base"
        );
        assert!(request["params"]["streamEpoch"].is_null());

        let page = large_page(1, size);
        cache
            .receive(&json!({"id":"history","result":{"kind":"snapshot",
            "streamEpoch":"stream","data":page.to_string(),"nextFragment":null}}))
            .unwrap();
        let recovered = cache.projected().unwrap();
        let mut fresh = self::cache(&json!({"epoch":"workspace","seq":2,
            "state":{"index":{"active":1},"threads":[{"id":1,"title":"still live"},{"id":2},{"id":3}]}}));
        fresh.merge_page(1, page, true, "stream", None).unwrap();
        let full = fresh.projected().unwrap();
        assert_eq!(recovered.full()["threads"][0], full.full()["threads"][0]);
        assert!(
            cache.next_history().is_none(),
            "the recovered subscription is current"
        );
        assert!(cache.retained_history_bytes() <= protocol::MAX_SNAPSHOT_BYTES);
    }

    #[test]
    fn replaced_emitted_fields_count_until_the_next_emission() {
        let mut cache = cache(&snapshot(1, json!({"id":1})));
        cache
            .merge_page(1, large_page(1, 8192), true, "stream", None)
            .unwrap();
        let first = cache.projected().unwrap();
        let old_item = Arc::downgrade(&first.views[&1].fields["messages"].items[0]);
        let mut row = large_page(1, 32)["items"][0].clone();
        row["item"]["text"] = json!("small");
        cache
            .stream(
                &json!({"kind":"event","threadId":1,"streamEpoch":"stream","sequence":2,
            "events":[{"type":"item.updated","row":row}]}),
            )
            .unwrap();
        let before_emission = cache.retained_history_bytes();
        let latest = cache.projected().unwrap();
        assert!(before_emission > cache.retained_history_bytes() + 8000);
        assert!(
            old_item.upgrade().is_some(),
            "external clients own their last-applied version"
        );
        drop(first);
        assert!(old_item.upgrade().is_none());
        assert_eq!(
            latest.views[&1].fields["messages"].items[0]["text"],
            "small"
        );
    }

    #[test]
    fn eviction_drains_an_outstanding_page_before_starting_a_fresh_subscription() {
        let mut cache = cache(&snapshot(1, json!({"id":1})));
        let mut page = large_page(1, 32);
        page["nextCursor"] = json!("earlier");
        page["hasMoreHistory"] = json!(true);
        cache.merge_page(1, page, true, "stream", None).unwrap();
        cache.subscribed = Some(1);
        cache.projected().unwrap();
        cache.request_earlier(1, None);
        assert_eq!(cache.next_history().unwrap()["method"], "thread.history");
        cache.evict(1);
        assert!(
            cache.next_history().is_none(),
            "do not reuse an outstanding RPC id"
        );
        cache
            .receive(&json!({"id":"history","result":{
                "data":large_page(1, 64).to_string(),"nextFragment":null
            }}))
            .unwrap();
        assert!(
            !cache.views.contains_key(&1),
            "a stale page must not recreate a partial base"
        );
        let request = cache.next_history().unwrap();
        assert_eq!(request["method"], "thread.subscribe");
        assert!(request["params"]["afterSequence"].is_null());
    }

    #[test]
    fn fresh_compaction_anchors_remap_after_the_message_window_is_installed() {
        let mut cache = cache(&snapshot(1, json!({"id":1})));
        let mut page = large_page(1, 32);
        page["items"][0]["ordinal"] = json!(11);
        page["items"].as_array_mut().unwrap().push(json!({
            "sourceThreadId":1,"sourceItemId":"compactions:c","field":"compactions",
            "ordinal":0,"position":1,"item":{"anchor":{"message":11,"offset":0},"order":1}
        }));
        cache.merge_page(1, page, true, "stream", None).unwrap();
        assert_eq!(
            cache.assembled(1).unwrap()["compactions"][0]["anchor"]["message"],
            0
        );
    }

    #[test]
    fn scalar_only_retry_updates_remap_to_the_loaded_message_window() {
        let messages: Vec<_> = (0..30)
            .map(|i| {
                json!({
                    "id":format!("m-{i}"), "role":if i % 2 == 0 {"You"} else {"Pi"}, "text":"text"
                })
            })
            .collect();
        let mut initial = snapshot(1, json!({"id":1,"messages":messages,"stats":{}}));
        crate::history::stamp(&Value::Null, &mut initial["state"]["threads"][0], 1);
        let streams = Streams::new(&initial);
        let mut cache = cache(&initial);
        answer_with_budget(
            &mut cache,
            &streams,
            &mut Fragments::default(),
            crate::history::PAGE_MAX_ENCODED_BYTES,
        );
        assert_eq!(
            cache.views[&1].installed.as_ref().unwrap().fields["messages"].ords[0],
            10
        );
        cache.projected().unwrap();
        for sequence in [2, 3] {
            cache
                .receive(&protocol::notification(
                    "thread.stream",
                    json!({
                        "kind":"event","threadId":1,"streamEpoch":"stream","sequence":sequence,
                        "events":[{"type":"fields.updated","fields":{"id":1,"harness_state":{
                            "workRetries":[
                                {"id":"visible","anchor":{"message":29,"offset":0}},
                                {"id":"unloaded","anchor":{"message":1,"offset":0}}
                            ]
                        }}}]
                    }),
                ))
                .unwrap();
            let history = cache.assembled(1).unwrap();
            let retries = history["harness_state"]["workRetries"].as_array().unwrap();
            assert_eq!(retries.len(), 1);
            assert_eq!(retries[0]["anchor"]["message"], 19);
        }
    }

    fn deferred_marker_cache() -> ProjectionCache {
        let initial = snapshot(1, json!({"id":1,"messages":[],"stats":{}}));
        let mut cache = cache(&initial);
        cache
            .reconnect(&json!({"capabilities":[protocol::HISTORY_ITEMS,protocol::DEFERRED_WORK]}))
            .unwrap();
        cache
            .merge_page(
                1,
                json!({
                    "threadId":1,"snapshotSequence":1,"historyRevision":"1","nextCursor":null,
                    "hasMoreHistory":false,"fields":{"id":1,"stats":{}},
                    "items":[
                        {"sourceThreadId":1,"sourceItemId":"messages:m","field":"messages",
                            "ordinal":0,"position":0,"item":{"id":"m","role":"Pi","text":"answer"}},
                        {"sourceThreadId":1,"sourceItemId":"work:w","field":"work",
                            "ordinal":0,"position":1,"item":{"start":0,
                                "deferredWork":{"start":0,"cursor":"work-cursor","count":10}}}
                    ]
                }),
                true,
                "stream",
                None,
            )
            .unwrap();
        cache.subscribed = Some(1);
        cache
    }

    #[test]
    fn deferred_loading_errors_and_removal_invalidate_structure_and_record() {
        let mut cache = deferred_marker_cache();
        let before = cache.projected().unwrap();
        cache.request_work(1, "work:w".into());
        let loading = cache.projected().unwrap();
        assert!(loading.views[&1].structural);
        assert!(loading.views[&1].record_changed);
        assert!(loading.views[&1].structure > before.views[&1].structure);
        assert_eq!(
            loading.views[&1].record["historyMeta"]["deferredWork"][0]["loading"],
            true
        );
        assert_eq!(cache.next_history().unwrap()["method"], "thread.history");
        cache
            .receive(&protocol::response(
                json!("history"),
                Err("Work fetch failed".into()),
            ))
            .unwrap();
        let error = cache.projected().unwrap();
        assert!(error.views[&1].structural);
        assert!(error.views[&1].record_changed);
        assert!(error.views[&1].structure > loading.views[&1].structure);
        assert_eq!(
            error.views[&1].record["historyMeta"]["deferredWork"][0]["error"],
            "Work fetch failed"
        );
        cache
            .stream(
                &json!({"kind":"event","threadId":1,"streamEpoch":"stream","sequence":2,
            "events":[{"type":"item.removed","sourceItemId":"work:w"}]}),
            )
            .unwrap();
        let removed = cache.projected().unwrap();
        assert!(removed.views[&1].structural);
        assert!(removed.views[&1].record_changed);
        assert_eq!(
            removed.views[&1].record["historyMeta"]["deferredWork"],
            json!([])
        );
    }

    #[test]
    fn history_error_and_retry_transitions_mark_record_changes() {
        let mut cache = deferred_marker_cache();
        cache.projected().unwrap();
        let view = cache.views.get_mut(&1).unwrap();
        view.has_more = true;
        view.cursor = Some("earlier".into());
        cache.request_earlier(1, None);
        cache.projected().unwrap();
        cache.next_history().unwrap();
        cache
            .receive(&protocol::response(
                json!("history"),
                Err("History fetch failed".into()),
            ))
            .unwrap();
        let error = cache.projected().unwrap();
        assert!(error.views[&1].record_changed);
        assert_eq!(
            error.views[&1].record["historyMeta"]["error"],
            "History fetch failed"
        );
        cache.failed(1, "Replay unavailable", true);
        let retry = cache.projected().unwrap();
        assert!(retry.views[&1].record_changed);
        assert_eq!(retry.views[&1].record["historyRetry"]["count"], 1);
    }

    #[test]
    fn incremental_items_equal_full_history_while_older_fragment_walk_is_active() {
        let mut thread = json!({"id":1,"messages":[{"role":"user","text":"😀".repeat(40000)},{"role":"assistant","text":"a"}],
            "tools":[{"id":"tool-1","text":"old"}],"thoughts":[],"stats":{"tokens":1},"queue":["queued"]});
        let initial = snapshot(1, thread.clone());
        let mut streams = Streams::new(&initial);
        let mut cache = cache(&initial);
        let mut fragments = Fragments::default();
        answer(&mut cache, &streams, &mut fragments);
        let mut sequence = 1;
        let mut steps = 0;
        while !cache.complete(1) {
            cache.request_earlier(1, None);
            // Revisions during a walk require no additional download or request.
            sequence += 1;
            thread["messages"][1]["text"] = json!(format!("live-{sequence}"));
            thread["tools"][0]["text"] = json!(format!("tool-{sequence}"));
            thread["stats"]["tokens"] = json!(sequence);
            streams.publish(&snapshot(sequence, thread.clone()));
            for mut event in streams.replay(1, sequence - 1).unwrap() {
                event["streamEpoch"] = json!("stream");
                cache
                    .receive(&protocol::notification("thread.stream", event))
                    .unwrap();
            }
            let request = answer(&mut cache, &streams, &mut fragments);
            assert_ne!(request["method"], "thread.subscribe");
            steps += 1;
            assert!(steps < 200);
        }
        assert!(steps > 1);
        assert_eq!(
            cache.assembled(1).unwrap().to_string().as_bytes(),
            protocol::history_payload(&thread).to_string().as_bytes()
        );
        assert!(cache.next_history().is_none());
        // Reorder and remove items, then append a new item. Identity and ordinal win.
        sequence += 1;
        thread["messages"] =
            json!([{"role":"assistant","text":"final"},{"role":"user","text":"next"}]);
        thread["tools"] = json!([]);
        thread["queue"] = json!([]);
        streams.publish(&snapshot(sequence, thread.clone()));
        for mut event in streams.replay(1, sequence - 1).unwrap() {
            event["streamEpoch"] = json!("stream");
            cache
                .receive(&protocol::notification("thread.stream", event))
                .unwrap();
        }
        assert_eq!(
            cache.assembled(1).unwrap().to_string().as_bytes(),
            protocol::history_payload(&thread).to_string().as_bytes()
        );
        assert!(cache.next_history().is_none());
    }

    #[test]
    fn unknown_event_kinds_and_fields_do_not_break_known_conversation_or_replay_position() {
        let snapshot = snapshot(
            1,
            json!({"id":1,"messages":[{"role":"assistant","text":"known"}]}),
        );
        let streams = Streams::new(&snapshot);
        let mut cache = cache(&snapshot);
        answer(&mut cache, &streams, &mut Fragments::default());
        cache.receive(&protocol::notification("thread.stream",json!({
            "kind":"future-kind","threadId":1,"streamEpoch":"stream","sequence":2,"futureField":{"anything":true}
        }))).unwrap();
        cache.receive(&protocol::notification("thread.stream",json!({
            "kind":"event","threadId":1,"streamEpoch":"stream","sequence":3,"events":[
                {"type":"future-event","extra":true},
                {"type":"item.updated","row":{"sourceThreadId":1,"sourceItemId":"messages:1",
                    "field":"messages","ordinal":0,"position":0,"item":{"role":"assistant","text":"advancing"},"futureField":true}}
            ]
        }))).unwrap();
        assert_eq!(
            cache.assembled(1).unwrap()["messages"][0]["text"],
            "advancing"
        );
        cache
            .reconnect(&json!({"capabilities":[protocol::HISTORY_ITEMS],"futureField":true}))
            .unwrap();
        assert_eq!(cache.next_history().unwrap()["params"]["afterSequence"], 3);
        // An unknown kind must not turn a malformed known event into a valid one.
        assert!(
            cache
                .stream(
                    &json!({"kind":"event","threadId":1,"streamEpoch":"stream","sequence":4,
            "events":[{"type":"item.updated","row":{}}]})
                )
                .is_err()
        );
    }

    #[test]
    fn missing_baseline_capability_has_a_visible_update_required_state() {
        let snapshot = snapshot(1, json!({"id":1,"messages":[]}));
        let mut cache = cache(&snapshot);
        assert_eq!(
            cache
                .reconnect(&json!({"capabilities":["snapshot-subscriptions"]}))
                .unwrap_err(),
            protocol::HISTORY_UPDATE_REQUIRED
        );
        assert!(cache.next_history().is_none());
        assert_eq!(cache.projected().unwrap()["historyUpdateRequired"], true);
        assert!(
            cache
                .notices
                .back()
                .unwrap()
                .starts_with("Update required.")
        );
    }

    #[test]
    fn fragment_failure_retries_once_reports_the_count_and_keeps_the_installed_view() {
        let snapshot = snapshot(1, json!({"id":1,"messages":[{"text":"kept"}]}));
        let streams = Streams::new(&snapshot);
        let mut cache = cache(&snapshot);
        answer(&mut cache, &streams, &mut Fragments::default());
        cache
            .reconnect(&json!({"capabilities":[protocol::HISTORY_ITEMS]}))
            .unwrap();
        for attempt in 0..2 {
            assert!(cache.next_history().is_some());
            cache
                .receive(&protocol::response(
                    json!("history"),
                    Err(protocol::HISTORY_LEASE_GONE.into()),
                ))
                .unwrap();
            assert_eq!(cache.assembled(1).unwrap()["messages"][0]["text"], "kept");
            if attempt == 0 {
                // A successful recent window must not reset the retry budget
                // while the same older-history walk remains incomplete.
                let partial = Streams::new(&super::tests::snapshot(
                    2,
                    json!({"id":1,"messages":[{"text":"x".repeat(200000)},{"text":"kept"}]}),
                ));
                answer(&mut cache, &partial, &mut Fragments::default());
                assert!(!cache.complete(1));
                cache.request_earlier(1, None);
            }
        }
        assert!(cache.next_history().is_none());
        let retry = cache.projected().unwrap()["threads"][0]["historyRetry"].clone();
        assert_eq!(retry["count"], 1);
        assert_eq!(retry["limit"], 1);
        assert_eq!(retry["fullRestarts"], 1);
        assert_eq!(retry["stopped"], true);
        assert!(cache.notices.front().unwrap().contains("retry 1/1"));
    }

    #[test]
    fn new_server_epoch_and_unreplayable_positions_take_one_new_scoped_snapshot() {
        let snapshot = snapshot(1, json!({"id":1,"messages":[{"text":"before"}]}));
        let mut streams = Streams::new(&snapshot);
        let mut cache = cache(&snapshot);
        let mut fragments = Fragments::default();
        answer(&mut cache, &streams, &mut fragments);
        cache
            .reconnect(&json!({"capabilities":[protocol::HISTORY_ITEMS]}))
            .unwrap();
        let request = cache.next_history().unwrap();
        assert_eq!(request["params"]["afterSequence"], 1);
        // A replacement server's epoch has no transport replay log.
        cache.inflight = None;
        cache.views.get_mut(&1).unwrap().epoch = "old-instance".into();
        streams.publish(&super::tests::snapshot(
            2,
            json!({"id":1,"messages":[{"text":"after"}]}),
        ));
        answer(&mut cache, &streams, &mut fragments);
        assert_eq!(cache.assembled(1).unwrap()["messages"][0]["text"], "after");
        assert!(cache.next_history().is_none());
        // The same process also snapshots when a retained replay exceeds KSG's limit.
        for sequence in 3..=200 {
            streams.publish(&super::tests::snapshot(
                sequence,
                json!({"id":1,"messages":[{"text":format!("update-{sequence}")}]}),
            ));
        }
        cache
            .reconnect(&json!({"capabilities":[protocol::HISTORY_ITEMS]}))
            .unwrap();
        answer(&mut cache, &streams, &mut fragments);
        assert_eq!(
            cache.assembled(1).unwrap()["messages"][0]["text"],
            "update-200"
        );
        assert!(cache.next_history().is_none());
    }

    #[test]
    fn initial_snapshot_does_not_auto_fetch_older_history_until_requested() {
        let mut messages = Vec::new();
        for turn in 0..15usize {
            messages
                .push(json!({"id":format!("u-{turn}"),"role":"You","text":format!("User {turn}")}));
            messages.push(
                json!({"id":format!("a-{turn}"),"role":"Pi","text":format!("Answer {turn}")}),
            );
        }
        let mut initial = snapshot(1, json!({"id":1,"messages":messages,"stats":{}}));
        crate::history::stamp(&Value::Null, &mut initial["state"]["threads"][0], 1);
        let streams = Streams::new(&initial);
        let mut cache = cache(&initial);
        let mut fragments = Fragments::default();

        // Initial thread.subscribe delivers recent 10 turns only, then stops!
        let sub = answer_with_budget(
            &mut cache,
            &streams,
            &mut fragments,
            crate::history::PAGE_MAX_ENCODED_BYTES,
        );
        assert_eq!(sub["method"], "thread.subscribe");
        assert!(
            cache.next_history().is_none(),
            "client must not eagerly walk older history before user requests it"
        );
        let projected = cache.projected().unwrap();
        assert_eq!(
            projected["threads"][0]["historyMeta"]["hasMoreHistory"],
            true
        );
        assert_eq!(projected["threads"][0]["historyMeta"]["loading"], false);
        assert_eq!(projected.views[&1].fields["messages"].items.len(), 20);

        // User requests earlier turns -> triggers thread.history and reaches end of history.
        cache.request_earlier(1, None);
        let loading_view = cache.projected().unwrap();
        assert_eq!(loading_view["threads"][0]["historyMeta"]["loading"], true);
        let req = answer_with_budget(
            &mut cache,
            &streams,
            &mut fragments,
            crate::history::PAGE_MAX_ENCODED_BYTES,
        );
        assert_eq!(req["method"], "thread.history");
        assert!(cache.next_history().is_none());
        let final_view = cache.projected().unwrap();
        assert_eq!(
            final_view["threads"][0]["historyMeta"]["hasMoreHistory"],
            false
        );
        assert_eq!(final_view["threads"][0]["historyMeta"]["loading"], false);
        assert_eq!(final_view.views[&1].fields["messages"].items.len(), 30);
    }

    #[test]
    fn load_earlier_keeps_error_state_allows_retry_and_remaps_tool_anchors() {
        let mut messages = Vec::new();
        let mut tools = Vec::new();
        for turn in 0..12usize {
            messages
                .push(json!({"id":format!("u-{turn}"),"role":"You","text":format!("User {turn}")}));
            let a_idx = messages.len();
            messages.push(
                json!({"id":format!("a-{turn}"),"role":"Pi","text":format!("Answer {turn}")}),
            );
            tools.push(json!({"id":format!("t-{turn}"),"name":"read","detail":"f","done":true,"failed":false,"output":"","images":[],"structured":null,"parent_call":null,"exit_code":0,"subagents":[],"anchor":{"message":a_idx,"offset":0},"block":0,"order":1}));
        }
        let mut initial = snapshot(
            1,
            json!({"id":1,"messages":messages,"tools":tools,"stats":{}}),
        );
        crate::history::stamp(&Value::Null, &mut initial["state"]["threads"][0], 1);
        let streams = Streams::new(&initial);
        let mut cache = cache(&initial);
        let mut fragments = Fragments::default();
        answer_with_budget(
            &mut cache,
            &streams,
            &mut fragments,
            crate::history::PAGE_MAX_ENCODED_BYTES,
        );

        // Tools loaded in initial window (turns 2..12) must have anchor.message remapped to 0..20!
        let assembled = cache.assembled(1).unwrap();
        let installed_tools = assembled["tools"].as_array().unwrap();
        assert_eq!(installed_tools.len(), 10);
        assert_eq!(installed_tools[0]["anchor"]["message"], 1);
        assert_eq!(installed_tools[9]["anchor"]["message"], 19);

        // Simulate failure when loading earlier turns -> error state set, retry works.
        cache.request_earlier(1, None);
        assert!(cache.next_history().is_some());
        cache
            .receive(&protocol::response(
                json!("history"),
                Err("Network timeout".into()),
            ))
            .unwrap();
        let errored = cache.projected().unwrap();
        assert_eq!(errored["threads"][0]["historyMeta"]["loading"], false);
        assert_eq!(
            errored["threads"][0]["historyMeta"]["error"],
            "Network timeout"
        );
        assert!(cache.next_history().is_none());

        // Retry succeeds and loads remaining turns 0..2.
        cache.request_earlier(1, None);
        answer_with_budget(
            &mut cache,
            &streams,
            &mut fragments,
            crate::history::PAGE_MAX_ENCODED_BYTES,
        );
        let done = cache.projected().unwrap();
        assert_eq!(done["threads"][0]["historyMeta"]["hasMoreHistory"], false);
        assert!(done["threads"][0]["historyMeta"]["error"].is_null());
        assert_eq!(
            cache.assembled(1).unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            24
        );
        assert_eq!(
            cache.assembled(1).unwrap()["tools"]
                .as_array()
                .unwrap()
                .len(),
            12
        );
    }
}
