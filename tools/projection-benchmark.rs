//! Shared test-only driver, compiled against both historical and current code.
//! Server fixture generation and correctness checks stay outside timed regions.
use super::*;
use crate::{
    client_history::ProjectionCache,
    history::{Fragments, Streams},
    protocol,
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, process::Command, time::Instant};

fn workspace(sequence: u64, threads: &[Value]) -> Value {
    let index: Vec<_> = threads
        .iter()
        .map(|thread| {
            json!({"id":thread["id"],"machine":"local",
                "title":format!("Thread {}",thread["id"])})
        })
        .collect();
    json!({"epoch":"benchmark","seq":sequence,
        "state":{"index":{"active":threads.last().unwrap()["id"],"captains":[],
            "projects":[],"threads":index},"threads":threads}})
}

fn answer(cache: &mut ProjectionCache, streams: &Streams, fragments: &mut Fragments) {
    let request = cache.next_history().expect("pending history request");
    let params = &request["params"];
    let thread = params["threadId"].as_u64().unwrap();
    let mut result = fragments
        .read(thread, params, Instant::now(), || {
            streams.page(thread, params["historyCursor"].as_str(), 64 << 10)
        })
        .unwrap();
    if request["method"] == "thread.subscribe" {
        result["kind"] = json!("snapshot");
        result["streamEpoch"] = json!("stream");
    }
    cache
        .receive(&json!({"id":"history","result":result}))
        .unwrap();
}

fn fully_load(
    cache: &mut ProjectionCache,
    streams: &Streams,
    fragments: &mut Fragments,
    thread: u64,
) {
    cache.desired_thread = Some(thread);
    answer(cache, streams, fragments);
    for _ in 0..100_000 {
        if cache.complete(thread) {
            return;
        }
        cache.request_earlier(thread, None);
        answer(cache, streams, fragments);
    }
    panic!("history did not converge");
}

fn git(args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[(sorted.len() * percentile / 100).min(sorted.len() - 1)]
}

#[derive(Default)]
struct Measurements {
    ns: Vec<u64>,
    allocations: Vec<u64>,
    bytes: Vec<u64>,
}

impl Measurements {
    fn report(&self) -> Value {
        json!({
            "updates":self.ns.len(),
            "p50_us":percentile(&self.ns,50) as f64 / 1000.0,
            "p95_us":percentile(&self.ns,95) as f64 / 1000.0,
            "allocations_p50":percentile(&self.allocations,50),
            "allocations_p95":percentile(&self.allocations,95),
            "allocated_bytes_p50":percentile(&self.bytes,50),
            "allocated_bytes_p95":percentile(&self.bytes,95),
        })
    }
}

#[test]
#[ignore = "release-only isolated benchmark; requires SHINA_PROJECTION_WORKLOAD"]
fn matched_transcript_replay() {
    assert!(!cfg!(debug_assertions), "measure a release build");
    let fixture = std::fs::read(
        std::env::var_os("SHINA_PROJECTION_WORKLOAD").expect("SHINA_PROJECTION_WORKLOAD"),
    )
    .unwrap();
    let input: Value = serde_json::from_slice(&fixture).unwrap();
    assert_eq!(input["format"], "shina-projection-workload-v1");
    let warmup = input["warmup"].as_u64().unwrap() as usize;
    let fixture_sha = format!("{:x}", Sha256::digest(&fixture));
    let harness_sha = format!(
        "{:x}",
        Sha256::digest(include_bytes!("projection-benchmark.rs"))
    );
    let allocator_sha = format!(
        "{:x}",
        Sha256::digest(include_bytes!("projection-benchmark-alloc.rs"))
    );
    let revision = git(&["rev-parse", "HEAD"]);
    assert_eq!(
        revision,
        std::env::var("SHINA_PROJECTION_EXPECT_REVISION").expect("expected source revision"),
        "do not attach a stale revision label"
    );
    let source_changes = git(&["diff", "--name-only"]);
    for sample in input["cases"].as_array().unwrap() {
        let mut threads: Vec<Value> = sample["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|thread| {
                let id = thread["id"].as_u64().unwrap();
                let mut record =
                    serde_json::to_value(ThreadView::capture(&Thread::new(id as usize))).unwrap();
                record["messages"] = thread["messages"].clone();
                record["tools"] = thread["tools"].clone();
                crate::history::stamp(&Value::Null, &mut record, 1);
                record
            })
            .collect();
        let mut snapshot = workspace(1, &threads);
        let mut streams = Streams::new(&snapshot);
        let mut cache = ProjectionCache::default();
        cache
            .reconnect(&json!({"capabilities":[protocol::HISTORY_ITEMS]}))
            .unwrap();
        cache
            .receive(&protocol::notification(
                "subscription.snapshot",
                json!({"epoch":"benchmark","seq":1,"state":protocol::compact(&snapshot["state"])}),
            ))
            .unwrap();
        let mut fragments = Fragments::default();
        for thread in 1..=threads.len() as u64 {
            fully_load(&mut cache, &streams, &mut fragments, thread);
        }
        let mut client = State::headless(PathBuf::new());
        client.apply_projection(cache.projected().unwrap()).unwrap();
        let mut messages = Measurements::default();
        let mut tools = Measurements::default();
        let changes = sample["changes"].as_array().unwrap();
        assert!(changes.len() > warmup + 1);
        for (index, change) in changes.iter().enumerate() {
            let thread_id = change["thread_id"].as_u64().unwrap();
            let at = threads
                .iter()
                .position(|thread| thread["id"] == thread_id)
                .unwrap();
            let old = threads[at].clone();
            let field = change["field"].as_str().unwrap();
            let item_index = change["index"].as_u64().unwrap() as usize;
            threads[at][field][item_index] = change["item"].clone();
            let sequence = index as u64 + 2;
            crate::history::stamp(&old, &mut threads[at], sequence);
            let after = workspace(sequence, &threads);
            let mut notifications = vec![protocol::notification(
                "subscription.patch",
                json!({"epoch":"benchmark","seq":sequence,"patch":protocol::diff(
                    &protocol::compact(&snapshot["state"]),&protocol::compact(&after["state"]))}),
            )];
            streams.publish(&after);
            let events = streams.replay(thread_id, sequence - 1).unwrap();
            assert!(!events.is_empty(), "measure a real update, not a no-op");
            for mut event in events {
                event["streamEpoch"] = json!("stream");
                notifications.push(protocol::notification("thread.stream", event));
            }
            snapshot = after;

            // Both builds execute their own unchanged receive/install,
            // projected and client apply implementations with this same driver.
            crate::test_alloc::reset();
            let start = Instant::now();
            for notification in &notifications {
                cache.receive(notification).unwrap();
            }
            client.apply_projection(cache.projected().unwrap()).unwrap();
            let elapsed = start.elapsed().as_nanos() as u64;
            let (allocation_count, allocated_bytes) = crate::test_alloc::counts();
            if index >= warmup {
                let measurements = if field == "messages" {
                    &mut messages
                } else {
                    &mut tools
                };
                measurements.ns.push(elapsed);
                measurements.allocations.push(allocation_count as u64);
                measurements.bytes.push(allocated_bytes as u64);
            }

            let target = client
                .threads
                .iter()
                .find(|thread| thread.id as u64 == thread_id)
                .unwrap();
            if field == "messages" {
                assert_eq!(
                    target.messages[item_index].text,
                    change["item"]["text"].as_str().unwrap()
                );
            } else {
                assert_eq!(
                    target.tools[item_index].output,
                    change["item"]["output"].as_str().unwrap()
                );
            }
            assert_eq!(
                client
                    .threads
                    .iter()
                    .map(|thread| thread.messages.len() + thread.tools.len())
                    .sum::<usize>(),
                sample["total_items"].as_u64().unwrap() as usize
            );
        }
        println!(
            "PROJECTION_BENCH {}",
            json!({
                "source_revision":revision,"source_changes":source_changes,
                "fixture_sha256":fixture_sha,"harness_sha256":harness_sha,
                "allocator_sha256":allocator_sha,
                "platform":std::env::consts::OS,"arch":std::env::consts::ARCH,
                "profile":"release","phases":["receive/install","projected","State::apply_projection"],
                "threads":sample["thread_count"],"messages_per_thread":sample["messages_per_thread"],
                "tools_per_thread":sample["tools_per_thread"],"total_items":sample["total_items"],
                "timings":{"messages":messages.report(),"tools":tools.report()}
            })
        );
    }
    assert_eq!(revision, git(&["rev-parse", "HEAD"]));
    assert_eq!(source_changes, git(&["diff", "--name-only"]));
}
