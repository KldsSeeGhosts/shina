//! Ported from KSG client-runtime state/threads.ts and threadHistoryMerge.ts.
use crate::{history::Row, protocol};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};

#[derive(Default)]
pub(crate) struct ProjectionCache {
    pub(crate) state: Value,
    pub(crate) seq: Option<u64>,
    pub(crate) epoch: Value,
    pub(crate) snapshot_pages: String,
    pub(crate) histories: HashMap<u64, (String, Value, usize)>,
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
}

#[derive(Default)]
struct View {
    sequence: u64,
    epoch: String,
    revision: String,
    rows: HashMap<String, (u64, Row)>,
    removed: HashMap<String, u64>,
    fields: Value,
    fields_sequence: u64,
    cursor: Option<String>,
    ready: bool,
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
    fragment: Value,
    text: String,
    waiting: bool,
    pending: Vec<Value>,
    pending_bytes: usize,
}

impl ProjectionCache {
    pub(crate) fn reconnect(&mut self, hello: &Value) -> Result<(), String> {
        self.inflight = None;
        self.subscribed = None;
        self.snapshot_pages.clear();
        self.stream_pages.clear();
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
        self.dirty = true;
    }

    #[cfg(test)]
    pub(crate) fn complete(&self, thread: u64) -> bool {
        self.views
            .get(&thread)
            .is_some_and(|view| view.ready && view.cursor.is_none())
    }

    fn install(&mut self, thread: u64) -> Result<(), String> {
        let Some(view) = self.views.get(&thread).filter(|view| view.ready) else {
            return Ok(());
        };
        let mut history = view.fields.clone();
        history["id"] = json!(thread);
        let mut rows: Vec<_> = view.rows.values().map(|(_, row)| row).collect();
        rows.sort_by(|a, b| {
            a.field
                .cmp(&b.field)
                .then(a.ordinal.cmp(&b.ordinal))
                .then(a.source_item_id.cmp(&b.source_item_id))
        });
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
            history[&row.field]
                .as_array_mut()
                .unwrap()
                .push(row.item.clone());
        }
        let bytes = history.to_string().len();
        if bytes > protocol::MAX_SNAPSHOT_BYTES {
            return Err("Thread history exceeded the client memory budget.".into());
        }
        self.histories
            .insert(thread, (view.revision.clone(), history, bytes));
        self.history_order.retain(|id| *id != thread);
        self.history_order.push_back(thread);
        while self
            .histories
            .values()
            .map(|(_, _, bytes)| *bytes)
            .sum::<usize>()
            > protocol::MAX_SNAPSHOT_BYTES
        {
            let Some(oldest) = self.history_order.pop_front() else {
                break;
            };
            self.histories.remove(&oldest);
            self.views.remove(&oldest);
        }
        self.dirty = true;
        Ok(())
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
            self.histories.remove(&thread);
            self.history_order.retain(|id| *id != thread);
            self.dirty = true;
            return Ok(());
        }
        if kind == "event" {
            for event in item["events"].as_array().ok_or("Invalid thread events.")? {
                match event["type"].as_str() {
                    Some("item.updated") => {
                        let row: Row = serde_json::from_value(event["row"].clone())
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
                        view.removed.remove(&row.source_item_id);
                        view.rows
                            .insert(row.source_item_id.clone(), (sequence, row));
                    }
                    Some("item.removed") => {
                        let id = event["sourceItemId"]
                            .as_str()
                            .ok_or("Invalid removed item.")?;
                        if view.rows.get(id).is_some_and(|(seq, _)| *seq > sequence) {
                            continue;
                        }
                        view.rows.remove(id);
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
        for row in page["items"].as_array().ok_or("Invalid history items.")? {
            let row: Row = serde_json::from_value(row.clone()).map_err(|e| e.to_string())?;
            if row.source_thread_id != thread {
                return Err("Cross-thread history item.".into());
            }
            if !protocol::HISTORY_FIELDS.contains(&row.field.as_str()) {
                continue;
            }
            let id = &row.source_item_id;
            // Newer live records and removals win over concurrently read older pages.
            if view.rows.get(id).is_some_and(|(seq, _)| *seq >= sequence)
                || view.removed.get(id).is_some_and(|seq| *seq >= sequence)
            {
                continue;
            }
            view.rows
                .insert(row.source_item_id.clone(), (sequence, row));
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
        view.cursor = page["nextCursor"].as_str().map(str::to_owned);
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
                protocol::apply(&mut self.state, params["patch"].clone())?;
                self.seq = Some(seq);
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
                if !value["error"].is_null() {
                    let code = value["error"]["code"].as_i64();
                    if code == Some(protocol::HISTORY_UPDATE_REQUIRED_CODE) {
                        self.update_required = true;
                    }
                    self.failed(
                        walk.thread,
                        value["error"]["message"]
                            .as_str()
                            .unwrap_or("History read failed."),
                        code == Some(protocol::HISTORY_LEASE_GONE_CODE),
                    );
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
                    let page = serde_json::from_str(&walk.text).map_err(|e| e.to_string())?;
                    self.merge_page(
                        walk.thread,
                        page,
                        walk.snapshot,
                        walk.cursor.as_deref().unwrap_or(""),
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
            let subscribe = self.subscribed != Some(thread);
            let cursor = self.views.get(&thread).and_then(|view| view.cursor.clone());
            if !subscribe && cursor.is_none() {
                return None;
            }
            self.inflight = Some(History {
                thread,
                subscribe,
                snapshot: false,
                cursor,
                fragment: Value::Null,
                text: String::new(),
                waiting: false,
                pending: Vec::new(),
                pending_bytes: 0,
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
            }
            "thread.history"
        };
        Some(json!({"jsonrpc":"2.0","id":"history","method":method,"params":params}))
    }

    pub(crate) fn projected(&mut self) -> Option<Value> {
        if !self.dirty || self.seq.is_none() {
            return None;
        }
        self.dirty = false;
        if let Some(threads) = self.state["threads"].as_array() {
            self.histories
                .retain(|id, _| threads.iter().any(|t| t["id"] == *id));
            self.views
                .retain(|id, _| threads.iter().any(|t| t["id"] == *id));
            self.retries
                .retain(|id, _| threads.iter().any(|t| t["id"] == *id));
            self.history_order
                .retain(|id| self.histories.contains_key(id));
        }
        let mut state = self.state.clone();
        state["historyUpdateRequired"] = json!(self.update_required);
        if let Some(threads) = state["threads"].as_array_mut() {
            for thread in threads {
                let Some(id) = thread["id"].as_u64() else {
                    continue;
                };
                if let Some((_, history, _)) = self.histories.get(&id) {
                    for field in protocol::HISTORY_FIELDS {
                        thread[field] = history[field].clone();
                    }
                }
                if let Some(retry) = self.retries.get(&id) {
                    thread["historyRetry"] = json!({"count":retry.attempts,"limit":1,"fullRestarts":retry.restarts,
                        "stopped":retry.blocked,"error":retry.error});
                }
            }
        }
        Some(state)
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
                        streams.page(thread, None, 1024)
                    })
                    .unwrap();
                page["kind"] = json!("snapshot");
                page["streamEpoch"] = json!("stream");
                page
            }
        } else {
            fragments
                .read(thread, params, Instant::now(), || {
                    streams.page(thread, params["historyCursor"].as_str(), 1024)
                })
                .unwrap()
        };
        cache
            .receive(&json!({"id":"history","result":result}))
            .unwrap();
        request
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
            cache.histories[&1].1.to_string().as_bytes(),
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
            cache.histories[&1].1.to_string().as_bytes(),
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
        assert_eq!(cache.histories[&1].1["messages"][0]["text"], "advancing");
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
            assert_eq!(cache.histories[&1].1["messages"][0]["text"], "kept");
            if attempt == 0 {
                // A successful recent window must not reset the retry budget
                // while the same older-history walk remains incomplete.
                let partial = Streams::new(&super::tests::snapshot(
                    2,
                    json!({"id":1,"messages":[{"text":"x".repeat(200000)},{"text":"kept"}]}),
                ));
                answer(&mut cache, &partial, &mut Fragments::default());
                assert!(!cache.complete(1));
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
        assert_eq!(cache.histories[&1].1["messages"][0]["text"], "after");
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
        assert_eq!(cache.histories[&1].1["messages"][0]["text"], "update-200");
        assert!(cache.next_history().is_none());
    }
}
