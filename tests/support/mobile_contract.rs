//! Non-desktop native client wire-contract hardening. A phone-class client has no
//! local process (no clientPid), suspends and reconnects while work runs, and must
//! recover device-scoped command receipts, scoped history, and bounded payloads.
//! Golden wire shapes live in tests/fixtures/mobile/ with volatile fields erased.
use super::*;

/// A native hello: current major, caller-chosen minor, no clientPid, and an
/// unknown field a newer client might send to an older server.
fn native_hello(client: &mut Client, minor: u64, id: &str) -> Value {
    client.rpc(
        "hello",
        json!({"protocolMajor":protocol::MAJOR,"protocolMinor":minor,
        "projectionSchema":1,"futureClientField":{"opaque":true}}),
        json!(id),
    )
}

fn prompts(profile: &Profile, thread: usize) -> Vec<String> {
    fs::read_to_string(
        profile
            .root
            .join(format!("Shina/chats/{thread}/fixture-prompts.jsonl")),
    )
    .map(|text| text.lines().map(str::to_owned).collect())
    .unwrap_or_default()
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap()
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// Erase fields the wire contract leaves nondeterministic: identities, sequence
/// cursors, revisions, times, paths and secrets. Structure stays exact.
fn normalize(value: &Value, root: &std::path::Path) -> Value {
    let root = std::fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .into_owned();
    fn walk(value: &Value, key: &str, root: &str) -> Value {
        match value {
            Value::Object(map) => map
                .iter()
                .filter(|(key, _)| !matches!(key.as_str(), "token" | "bootstrapToken"))
                .map(|(key, value)| (key.clone(), walk(value, key, root)))
                .collect(),
            Value::Array(items) => items.iter().map(|item| walk(item, "", root)).collect(),
            Value::String(text) => {
                if matches!(
                    key,
                    "epoch"
                        | "streamEpoch"
                        | "historyRevision"
                        | "revision"
                        | "nextCursor"
                        | "cursor"
                        | "fragmentCursor"
                        | "historyCursor"
                        | "nextFragment"
                        | "binaryVersion"
                        | "serverId"
                        | "instanceId"
                        | "environment_id"
                        | "lease_id"
                        | "environmentId"
                        | "leaseId"
                        | "hash"
                ) {
                    return json!(format!("<{key}>"));
                }
                if is_uuid(text) {
                    return json!("<uuid>");
                }
                if let Some((prefix, suffix)) = text.rsplit_once(':')
                    && is_uuid(suffix)
                {
                    return json!(format!("{prefix}:<uuid>"));
                }
                if let Some(suffix) = text.strip_prefix(root) {
                    return json!(format!("<root>{}", suffix.replace('\\', "/")));
                }
                json!(text.replace(root, "<root>"))
            }
            Value::Number(number) => {
                let epoch_like = number
                    .as_u64()
                    .or_else(|| number.as_f64().map(|n| n.max(0.0) as u64))
                    .is_some_and(|n| n >= 1_000_000_000);
                if epoch_like {
                    return json!("<time>");
                }
                if matches!(
                    key,
                    "seq"
                        | "sequence"
                        | "snapshotSequence"
                        | "position"
                        | "serverPid"
                        | "pid"
                        | "clientPid"
                        | "port"
                        | "duration_ms"
                        | "durationMs"
                        | "elapsed"
                        | "started_at"
                        | "created_at"
                        | "updated_at"
                ) {
                    return json!("<seq>");
                }
                value.clone()
            }
            _ => value.clone(),
        }
    }
    walk(value, "", &root)
}

fn assert_golden(name: &str, actual: &Value) {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mobile")
        .join(format!("{name}.json"));
    if fs::read_to_string(&fixture).is_ok_and(|expected| golden_matches(&expected, actual)) {
        return;
    }
    let actual = format!("{}\n", serde_json::to_string_pretty(actual).unwrap());
    let dump = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("mobile-golden");
    fs::create_dir_all(&dump).unwrap();
    fs::write(dump.join(format!("{name}.json")), &actual).unwrap();
    panic!(
        "golden fixture {name} does not match the normalized wire shape; actual JSON is in {}\n\
         Actual normalized JSON:\n{actual}",
        dump.display()
    );
}

fn golden_matches(expected: &str, actual: &Value) -> bool {
    serde_json::from_str::<Value>(expected).is_ok_and(|expected| expected == *actual)
}

#[test]
fn goldens_ignore_checkout_line_endings_but_keep_the_exact_json_shape() {
    let actual = json!({"answer":"line 1\nline 2","flag":true});
    let expected = serde_json::to_string_pretty(&actual).unwrap();
    assert!(golden_matches(&expected.replace('\n', "\r\n"), &actual));
    assert!(!golden_matches(
        &expected,
        &json!({"answer":"line 1\nline 2"})
    ));
    assert!(!golden_matches(
        &expected,
        &json!({"answer":"changed","flag":true})
    ));
    assert!(!golden_matches("invalid JSON", &actual));
}

#[test]
fn golden_paths_normalize_only_the_profile_prefix_and_its_separators() {
    let root = std::env::temp_dir().canonicalize().unwrap();
    let input = json!({"path":root.join("Shina").join("attachments").join("file.bin"),
        "text":r"Keep\these\backslashes"});
    let normalized = normalize(&input, &root);
    assert_eq!(normalized["path"], "<root>/Shina/attachments/file.bin");
    assert_eq!(normalized["text"], input["text"]);
}

/// Walk a fragmented thread.history/thread.subscribe response into a whole page.
/// Returns the parsed page JSON and the first response envelope, which carries
/// kind and streamEpoch for subscribes.
fn read_page(client: &mut Client, method: &str, params: Value, budget: usize) -> (Value, Value) {
    let mut text = String::new();
    let mut fragment = Value::Null;
    let mut first = Value::Null;
    loop {
        let mut request = params.clone();
        request["fragmentCursor"] = fragment.clone();
        let reply = client.rpc(
            if fragment.is_null() {
                method
            } else {
                "thread.history"
            },
            request,
            json!("history"),
        );
        assert!(reply["error"].is_null(), "{reply}");
        if first.is_null() {
            first = reply["result"].clone();
        }
        let data = reply["result"]["data"].as_str().unwrap();
        assert!(
            data.len() <= budget,
            "page fragment exceeded the requested budget"
        );
        text.push_str(data);
        fragment = reply["result"]["nextFragment"].clone();
        if fragment.is_null() {
            break;
        }
    }
    (serde_json::from_str(&text).unwrap(), first)
}

#[test]
fn mobile_hello_without_client_pid_negotiates_and_bounds() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    let hello = native_hello(&mut client, u64::from(protocol::MINOR), "native-hello");
    assert!(hello["error"].is_null(), "{hello}");
    assert_golden("hello_response", &normalize(&hello, &profile.root));
    let result = &hello["result"];
    assert_eq!(result["protocolMajor"], protocol::MAJOR);
    assert_eq!(result["protocolMinor"], protocol::MINOR);
    assert_eq!(result["deviceId"], device.to_string());
    assert_eq!(result["canOperate"], true);
    assert_eq!(result["canApprove"], true);
    for capability in [
        "projection-patches",
        protocol::HISTORY_ITEMS,
        protocol::FILE_ATTACHMENTS,
        protocol::REVIEW_DIFF,
    ] {
        assert!(
            result["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cap| cap == capability),
            "{capability}: {result}"
        );
    }
    assert_eq!(
        result["limits"]["controlBytes"],
        protocol::MAX_CONTROL as u64
    );
    assert_eq!(
        result["limits"]["pageBytes"],
        protocol::MAX_PAGE_BYTES as u64
    );
    assert_eq!(result["limits"]["outstandingCommands"], 32);
    // Unknown version majors and projection schemas refuse and fence the socket.
    for (major, schema) in [(0, 1), (2, 1), (1, 2)] {
        let refused = client.rpc(
            "hello",
            json!({"protocolMajor":major,"protocolMinor":0,"projectionSchema":schema}),
            json!("bad-version"),
        );
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("mismatch"),
            "{refused}"
        );
    }
    let fenced = client.command(
        "thread.rename",
        json!({"threadId":1,"title":"x"}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        fenced["error"]["message"]
            .as_str()
            .unwrap()
            .contains("hello")
    );
    // Older minors get the capability set of their era; future minors clamp.
    let old = native_hello(&mut client, 7, "old-hello");
    assert_eq!(old["result"]["protocolMinor"], 7);
    let capabilities = old["result"]["capabilities"].as_array().unwrap();
    assert!(
        capabilities
            .iter()
            .any(|cap| cap == protocol::HISTORY_ITEMS)
    );
    assert!(
        !capabilities
            .iter()
            .any(|cap| cap == protocol::QUESTION_DISMISS)
    );
    let future = native_hello(&mut client, 9999, "future-hello");
    assert_eq!(future["result"]["protocolMinor"], protocol::MINOR);
    // Dispatch fails closed for methods without an explicit grant.
    let unknown = client.rpc("device.teleport", json!({}), json!("unknown"));
    assert_eq!(unknown["error"]["message"], "Unknown protocol method.");
    // A pidless native client cannot coordinate or acknowledge a local restart.
    let restart = client.command("server.prepareRestart", json!({}), uuid::Uuid::new_v4());
    assert!(
        restart["error"]["message"]
            .as_str()
            .unwrap()
            .contains("older client"),
        "{restart}"
    );
    let ready = client.command(
        "server.clientReady",
        json!({"pid":1234}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        ready["error"]["message"]
            .as_str()
            .unwrap()
            .contains("registered"),
        "{ready}"
    );
    // Oversized control frames are refused by closing the socket, not truncated.
    // The server can reset the connection while the client is still writing, so
    // a failed send already proves the fence.
    let mut oversized = json!({"jsonrpc":"2.0","id":"huge","method":"thread.rename",
        "params":{"commandId":uuid::Uuid::new_v4(),
        "args":{"threadId":1,"title":"x".repeat(protocol::MAX_CONTROL)}}});
    oversized["params"]["args"]["title"] = json!("x".repeat(protocol::MAX_CONTROL));
    let mut closed = client
        .socket
        .send(Message::Text(oversized.to_string().into()))
        .is_err();
    for _ in 0..8 {
        if closed {
            break;
        }
        match client.socket.read() {
            Ok(Message::Text(_)) | Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => {
                closed = true;
                break;
            }
            Ok(_) => continue,
        }
    }
    assert!(
        closed,
        "server kept an oversized control frame's socket open"
    );
    drop(client);
    // The server is unaffected; a fresh native socket negotiates normally.
    let mut recovered = profile.client(uuid::Uuid::new_v4());
    let hello = native_hello(
        &mut recovered,
        u64::from(protocol::MINOR),
        "recovered-hello",
    );
    assert!(hello["error"].is_null(), "{hello}");
    profile.stop();
}

#[test]
fn mobile_send_receipts_dedupe_across_disconnect_and_restart() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut mobile = profile.client(device);
    native_hello(&mut mobile, u64::from(protocol::MINOR), "send-hello");
    let send_id = uuid::Uuid::new_v4();
    let send_args =
        json!({"threadId":1,"text":"history-stream-fixture","attachments":[],"mode":"prompt"});
    assert_golden(
        "send_command_request",
        &normalize(
            &json!({"jsonrpc":"2.0","id":send_id,"method":"thread.send",
            "params":{"commandId":send_id,"args":send_args}}),
            &profile.root,
        ),
    );
    let first = mobile.command("thread.send", send_args.clone(), send_id);
    assert_eq!(first["result"]["status"], "accepted", "{first}");
    assert_golden("send_command_receipt", &normalize(&first, &profile.root));
    mobile.until(|state| state["threads"][0]["status"] == "Running");
    // The prompt file is written by the harness process after the state goes
    // Running, so poll for it rather than racing the fixture script.
    let deadline = Instant::now() + Duration::from_secs(10);
    while prompts(&profile, 1).is_empty() {
        assert!(
            Instant::now() < deadline,
            "fake harness never saw the prompt"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    // A mutation the harness never sees still rides the same receipt machinery.
    let rename_id = uuid::Uuid::new_v4();
    let rename_args = json!({"threadId":1,"title":"mobile rename"});
    mobile
        .socket
        .send(Message::Text(
            json!({"jsonrpc":"2.0","id":rename_id,"method":"thread.rename",
            "params":{"commandId":rename_id,"args":rename_args}})
            .to_string()
            .into(),
        ))
        .unwrap();
    // The phone dies mid-run with the ack unread. The server may or may not have
    // read that frame before the socket closed; either way the resend must
    // resolve to exactly one durable receipt, not a second execution.
    drop(mobile);
    let mut mobile = profile.client(device);
    native_hello(&mut mobile, u64::from(protocol::MINOR), "reconnect-hello");
    let resent = mobile.command("thread.rename", rename_args, rename_id);
    assert_eq!(resent["result"]["status"], "accepted", "{resent}");
    // command.get resolves the retried command to that same durable receipt.
    let deadline = Instant::now() + Duration::from_secs(10);
    let stored = loop {
        let lookup = mobile.rpc(
            "command.get",
            json!({"commandId":rename_id}),
            json!("lookup"),
        );
        if lookup["error"].is_null() {
            break lookup["result"].clone();
        }
        assert!(
            Instant::now() < deadline,
            "retried command was not recorded: {lookup}"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(stored, resent["result"], "{stored}");
    // Replaying the send's UUID returns its stored receipt without a second prompt.
    let replayed = mobile.command("thread.send", send_args.clone(), send_id);
    assert_eq!(replayed["result"], first["result"], "{replayed}");
    assert_eq!(prompts(&profile, 1).len(), 1);
    // The same UUID with different args is an integrity error, not a retry.
    let tampered = mobile.command(
        "thread.send",
        json!({"threadId":1,"text":"tampered","attachments":[],"mode":"prompt"}),
        send_id,
    );
    assert!(
        tampered["error"]["message"]
            .as_str()
            .unwrap()
            .contains("different payload"),
        "{tampered}"
    );
    // Receipts are scoped to the device that sent them.
    let unknown = mobile.rpc(
        "command.get",
        json!({"commandId":uuid::Uuid::new_v4()}),
        json!("unknown"),
    );
    assert!(
        unknown["error"]["message"]
            .as_str()
            .unwrap()
            .contains("No receipt")
    );
    let mut stranger = profile.client(uuid::Uuid::new_v4());
    let scoped = stranger.rpc("command.get", json!({"commandId":send_id}), json!("scoped"));
    assert!(
        scoped["error"]["message"]
            .as_str()
            .unwrap()
            .contains("No receipt"),
        "{scoped}"
    );
    drop(stranger);
    // Let the turn settle before restart so no in-flight work re-prompts.
    mobile.until(|state| state["threads"][0]["status"] == "Completed");
    // Restart. Durable receipts resolve every lost acknowledgement.
    profile.stop();
    profile.launch(&["serve"]);
    let mut mobile = profile.client(device);
    native_hello(&mut mobile, u64::from(protocol::MINOR), "restarted-hello");
    for (id, expected) in [(send_id, &first["result"]), (rename_id, &stored)] {
        let lookup = mobile.rpc("command.get", json!({"commandId":id}), json!("relookup"));
        assert_eq!(lookup["result"], *expected, "{lookup}");
    }
    let replayed = mobile.command("thread.send", send_args, send_id);
    assert_eq!(replayed["result"], first["result"], "{replayed}");
    assert_eq!(
        prompts(&profile, 1).len(),
        1,
        "a durable receipt replay must not re-prompt"
    );
    profile.stop();
}

#[test]
fn mobile_scoped_history_pages_replays_and_gap_falls_back() {
    let mut template = Profile::new();
    template.launch(&["serve"]);
    let mut template_client = template.client(uuid::Uuid::new_v4());
    // Model discovery is asynchronous, and models are control rows in history.
    // Seed a settled catalog so the golden never depends on startup timing.
    template_client.until(|state| {
        state["threads"][0]["models"]
            .as_array()
            .is_some_and(|models| models.len() == 1 && models[0]["id"] == "fixture")
    });
    let base = template_client.snapshot.clone();
    drop(template_client);
    template.stop();

    let mut profile = Profile::new();
    let mut messages = Vec::new();
    let mut tools = Vec::new();
    for turn in 0..40usize {
        messages.push(json!({"id":format!("u-{turn}"),"role":"You",
            "text":format!("Mobile turn {turn}"),"images":[]}));
        let anchor = messages.len();
        messages.push(json!({"id":format!("a-{turn}"),"role":"Pi",
            "text":format!("Fixture reply {turn}"),"images":[]}));
        tools.push(
            json!({"id":format!("tool-{turn}"),"name":"read","detail":"main.rs",
            "done":true,"failed":false,"output":"ok","images":[],"structured":null,
            "parent_call":null,"exit_code":0,"subagents":[],
            "anchor":{"message":anchor,"offset":0},"block":0,"order":1}),
        );
    }
    let mut index = base["index"].clone();
    index["threads"][0]["session"] = Value::Null;
    let mut view = base["threads"][0].clone();
    view["id"] = json!(1);
    view["status"] = json!("Completed");
    for key in [
        "_history",
        "historySequence",
        "historyNextPosition",
        "historyRevision",
    ] {
        view.as_object_mut().unwrap().remove(key);
    }
    view["messages"] = json!(messages);
    view["tools"] = json!(tools);
    index["threadViews"] = json!([view]);
    fs::write(profile.root.join("Shina/index.json"), index.to_string()).unwrap();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut mobile = profile.client(device);
    native_hello(&mut mobile, u64::from(protocol::MINOR), "history-hello");
    mobile.rpc("workspace.subscribe", json!({}), json!("compact-subscribe"));
    // Workspace metadata stays bounded no matter how long the transcript is.
    let metadata = mobile.snapshot["threads"][0].to_string().len();
    assert!(
        metadata < 8 * 1024,
        "workspace thread metadata grew with the transcript: {metadata}"
    );
    assert!(
        mobile.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(mobile.snapshot["threads"][0]["historyRevision"].is_string());
    // The initial subscribe is a bounded snapshot page; its byte fragments
    // continue over thread.history with the returned fragmentCursor.
    let budget = 4096;
    let (page, envelope) = read_page(
        &mut mobile,
        "thread.subscribe",
        json!({"threadId":1,"byteBudget":budget}),
        budget,
    );
    assert_eq!(envelope["kind"], "snapshot", "{envelope}");
    assert!(envelope["streamEpoch"].is_string());
    let epoch = envelope["streamEpoch"].clone();
    assert_eq!(page["threadId"], 1);
    assert!(page["snapshotSequence"].is_u64());
    assert!(page["historyRevision"].is_string());
    assert!(page["nextCursor"].is_string(), "{page}");
    let snapshot_sequence = page["snapshotSequence"].clone();
    assert_golden("history_snapshot_page", &normalize(&page, &profile.root));
    // Older pages walk back to the first turn. Durable identities stay unique
    // and every seeded item is delivered exactly once across the paged walk.
    let mut ids: std::collections::BTreeSet<String> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["sourceItemId"].as_str().unwrap().to_owned())
        .collect();
    let mut covered: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mark = |items: &Value, covered: &mut std::collections::BTreeMap<String, usize>| {
        for row in items.as_array().unwrap() {
            *covered
                .entry(format!(
                    "{}:{}",
                    row["field"].as_str().unwrap_or("?"),
                    row["item"]["id"].as_str().unwrap_or("null")
                ))
                .or_default() += 1;
        }
    };
    mark(&page["items"], &mut covered);
    let mut cursor = page["nextCursor"].clone();
    let mut first_older = None;
    while let Some(at) = cursor.as_str().map(str::to_owned) {
        let (older, _) = read_page(
            &mut mobile,
            "thread.history",
            json!({"threadId":1,"historyCursor":at,"byteBudget":8192}),
            8192,
        );
        if first_older.is_none() {
            first_older = Some(older.clone());
        }
        assert!(
            older["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| ids.insert(row["sourceItemId"].as_str().unwrap().to_owned())),
            "a page repeated a durable item identity"
        );
        mark(&older["items"], &mut covered);
        cursor = older["nextCursor"].clone();
    }
    for turn in 0..40usize {
        for (field, id) in [
            ("messages", format!("u-{turn}")),
            ("messages", format!("a-{turn}")),
            ("tools", format!("tool-{turn}")),
        ] {
            assert_eq!(
                covered.get(&format!("{field}:{id}")).copied().unwrap_or(0),
                1,
                "paged coverage for {field}:{id}"
            );
        }
    }
    assert_golden(
        "history_older_page",
        &normalize(&first_older.unwrap(), &profile.root),
    );
    // With the subscription live, new work arrives as bounded thread.stream events.
    let sent = mobile.command(
        "thread.send",
        json!({"threadId":1,"text":"mobile-live-fixture","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(sent["result"]["status"], "accepted", "{sent}");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut live = None;
    while live.is_none() {
        assert!(Instant::now() < deadline, "no thread.stream event arrived");
        let value = mobile.read();
        // Catalog refresh events may legitimately arrive before the sent message.
        if value["method"] == "thread.stream"
            && value["params"]["kind"] == "event"
            && value["params"]["threadId"] == 1
            && value["params"]["events"].as_array().is_some_and(|events| {
                events.iter().any(|row| {
                    row["row"]["field"] == "messages"
                        && row["row"]["item"]["role"] == "You"
                        && row["row"]["item"]["text"] == "mobile-live-fixture"
                })
            })
        {
            live = Some(value["params"].clone());
        }
    }
    let live = live.unwrap();
    assert!(live["sequence"].as_u64().unwrap() > 0);
    assert_eq!(live["streamEpoch"], epoch);
    assert_golden("thread_stream_event", &normalize(&live, &profile.root));
    mobile.until(|state| state["threads"][0]["status"] == "Completed");
    // Suspend mid-conversation. While the phone is gone, new history grows past
    // the replay budget, so an at-cursor resume must fall back to a snapshot.
    drop(mobile);
    let mut writer = profile.client(uuid::Uuid::new_v4());
    for index in 0..4 {
        let big = writer.command(
            "thread.send",
            json!({"threadId":1,"text":"x".repeat(280_000 + index),"attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(big["result"]["status"], "accepted", "{big}");
        writer.until(|state| state["threads"][0]["status"] == "Completed");
    }
    // The fake harness appends its prompt file after the run completes, so wait
    // for all five sends to land before freezing the count.
    let deadline = Instant::now() + Duration::from_secs(10);
    while prompts(&profile, 1).len() < 5 {
        assert!(
            Instant::now() < deadline,
            "fake harness prompts never landed"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let prompt_count = prompts(&profile, 1).len();
    drop(writer);
    let mut mobile = profile.client(device);
    native_hello(&mut mobile, u64::from(protocol::MINOR), "suspended-hello");
    // A resume cursor whose retained events exceed the replay bounds falls back
    // to a fresh bounded snapshot instead of replaying or resending work.
    let (resumed_page, resumed) = read_page(
        &mut mobile,
        "thread.subscribe",
        json!({"threadId":1,"afterSequence":snapshot_sequence,"streamEpoch":epoch,
        "byteBudget":budget}),
        budget,
    );
    assert_eq!(
        resumed["kind"], "snapshot",
        "a replay gap must fall back to a snapshot: {resumed}"
    );
    assert!(
        resumed_page["snapshotSequence"].as_u64().unwrap() > snapshot_sequence.as_u64().unwrap(),
        "the fallback snapshot must advance past the stale cursor"
    );
    assert_eq!(prompts(&profile, 1).len(), prompt_count);
    // A wrong stream epoch takes the same fallback: never an error, never work.
    let (_, wrong_epoch) = read_page(
        &mut mobile,
        "thread.subscribe",
        json!({"threadId":1,"afterSequence":snapshot_sequence,
        "streamEpoch":"00000000-0000-0000-0000-000000000000","byteBudget":budget}),
        budget,
    );
    assert_eq!(wrong_epoch["kind"], "snapshot", "{wrong_epoch}");
    // Invalid targets and cursors are errors, not silent full transcripts.
    let unknown = mobile.rpc(
        "thread.subscribe",
        json!({"threadId":999,"byteBudget":budget}),
        json!("unknown-thread"),
    );
    assert!(
        unknown["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Unknown thread"),
        "{unknown}"
    );
    let garbage = mobile.rpc(
        "thread.history",
        json!({"threadId":1,"historyCursor":"not-a-cursor","byteBudget":budget}),
        json!("garbage-cursor"),
    );
    assert!(
        garbage["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Invalid history cursor"),
        "{garbage}"
    );
    profile.stop();
}

#[test]
fn mobile_approval_request_fences_stale_responses_after_reconnect() {
    let mut profile = Profile::new();
    fs::write(profile.root.join("use-native-approval-fixture"), "").unwrap();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut mobile = profile.client(device);
    native_hello(&mut mobile, u64::from(protocol::MINOR), "approval-hello");
    assert_eq!(
        mobile.command(
            "thread.configure",
            json!({"threadId":1,"harness":"claude"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    assert_eq!(
        mobile.command(
            "thread.send",
            json!({"threadId":1,"text":"approval-fixture","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    mobile.until(|state| state["threads"][0]["request"]["method"] == "confirm");
    let request = mobile.snapshot["threads"][0]["request"].clone();
    assert_golden("approval_request", &normalize(&request, &profile.root));
    // The phone disconnects with the approval still pending; reconnecting keeps it.
    drop(mobile);
    let mut mobile = profile.client(device);
    native_hello(
        &mut mobile,
        u64::from(protocol::MINOR),
        "approval-reconnect",
    );
    mobile.until(|state| state["threads"][0]["request"]["method"] == "confirm");
    let respond = |client: &mut Client, request_id: Value, generation: Value| {
        client.command(
            "request.respond",
            json!({"threadId":1,"requestId":request_id,"generation":generation,
            "requestContract":true,"response":{"decision":"accept"}}),
            uuid::Uuid::new_v4(),
        )
    };
    // A response from before the disconnect with a stale generation is refused:
    // the command lands as a rejected receipt, never applied to the request.
    let stale = respond(
        &mut mobile,
        request["id"].clone(),
        json!("a-superseded-runtime-generation"),
    );
    assert_eq!(stale["result"]["status"], "rejected", "{stale}");
    assert!(
        stale["result"]["result"]["error"]
            .as_str()
            .unwrap()
            .contains("generation"),
        "{stale}"
    );
    let expired = respond(
        &mut mobile,
        json!(uuid::Uuid::new_v4().to_string()),
        request["runtimeGeneration"].clone(),
    );
    assert_eq!(expired["result"]["status"], "rejected", "{expired}");
    assert!(
        expired["result"]["result"]["error"]
            .as_str()
            .unwrap()
            .contains("Request expired"),
        "{expired}"
    );
    let accepted = respond(
        &mut mobile,
        request["id"].clone(),
        request["runtimeGeneration"].clone(),
    );
    assert_eq!(accepted["result"]["status"], "accepted", "{accepted}");
    mobile.until(|state| state["threads"][0]["status"] == "Completed");
    profile.stop();
}

#[test]
fn mobile_paired_device_session_operates_without_maintain() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let host = profile.discovery();
    let port = host["port"].as_u64().unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let http = http();
    // A host client with the maintain grant mints the pairing code a phone
    // redeems. The pairing code itself never enters a golden fixture.
    let mut operator = profile.client(uuid::Uuid::new_v4());
    let issued = operator.rpc("devices.pair", json!({}), json!("mint"));
    assert!(issued["result"]["code"].is_string(), "{issued}");
    let code = issued["result"]["code"].as_str().unwrap().to_owned();
    let phone = uuid::Uuid::new_v4();
    // /v1/pair is unauthenticated but fenced: a malformed expectedServerId is a
    // bad request, a mismatched one is a conflict, and browsers get no Origin.
    for (body, status) in [
        (
            json!({"code":code,"deviceId":phone,"expectedServerId":"not-a-uuid"}),
            400,
        ),
        (
            json!({"code":code,"deviceId":phone,
            "expectedServerId":uuid::Uuid::new_v4()}),
            409,
        ),
    ] {
        let refused = http
            .post(format!("{base}/v1/pair"))
            .json(&body)
            .send()
            .unwrap();
        assert_eq!(refused.status().as_u16(), status, "{body}");
    }
    let origin = http
        .post(format!("{base}/v1/pair"))
        .header("Origin", "https://example.invalid")
        .json(&json!({"code":code,"deviceId":phone}))
        .send()
        .unwrap();
    assert_eq!(origin.status(), 403);
    let exchanged: Value = http
        .post(format!("{base}/v1/pair"))
        .json(&json!({"code":code,"deviceId":phone,
        "expectedServerId":host["serverId"]}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(exchanged["serverId"], host["serverId"]);
    assert_eq!(exchanged["peer"], false);
    assert_golden("pair_exchange", &normalize(&exchanged, &profile.root));
    // A redeemed code is consumed; it cannot mint a second credential.
    let reused = http
        .post(format!("{base}/v1/pair"))
        .json(&json!({"code":code,"deviceId":uuid::Uuid::new_v4()}))
        .send()
        .unwrap();
    assert_eq!(reused.status(), 403);
    // The phone credential connects over the bearer WebSocket with a pidless
    // hello, and the session is bound to the paired device id.
    let token = exchanged["token"].as_str().unwrap().to_owned();
    let mut mobile = Client::with_token(&host, token.clone());
    let hello = native_hello(&mut mobile, u64::from(protocol::MINOR), "paired-hello");
    assert_eq!(hello["result"]["deviceId"], phone.to_string());
    assert_eq!(hello["result"]["canOperate"], true);
    assert_eq!(hello["result"]["canApprove"], true);
    assert_eq!(hello["result"]["canUpdate"], true);
    // Pairing grants operate/approve/update but never host maintenance.
    for method in ["server.stop", "devices.pair", "server.prepareRestart"] {
        let refused = mobile.command(method, json!({}), uuid::Uuid::new_v4());
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("maintain"),
            "{method}: {refused}"
        );
    }
    // Work and receipts behave exactly like the bootstrap path.
    let send_id = uuid::Uuid::new_v4();
    let send_args =
        json!({"threadId":1,"text":"paired-device-fixture","attachments":[],"mode":"prompt"});
    let sent = mobile.command("thread.send", send_args.clone(), send_id);
    assert_eq!(sent["result"]["status"], "accepted", "{sent}");
    // The phone suspends; reconnecting with the same credential resolves the
    // send to its durable receipt and replays without a second prompt.
    drop(mobile);
    let mut mobile = Client::with_token(&host, token);
    native_hello(&mut mobile, u64::from(protocol::MINOR), "paired-reconnect");
    let lookup = mobile.rpc(
        "command.get",
        json!({"commandId":send_id}),
        json!("paired-lookup"),
    );
    assert_eq!(lookup["result"], sent["result"], "{lookup}");
    let replayed = mobile.command("thread.send", send_args, send_id);
    assert_eq!(replayed["result"], sent["result"], "{replayed}");
    mobile.until(|state| state["threads"][0]["status"] == "Completed");
    assert_eq!(
        prompts(&profile, 1).len(),
        1,
        "a paired-device replay must not re-prompt"
    );
    profile.stop();
}

#[test]
fn mobile_uploads_resume_by_offset_and_expire_on_restart() {
    use sha2::Digest;
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let client = profile.client(device);
    let base = format!("http://127.0.0.1:{}", profile.discovery()["port"]);
    let http = http();
    let bytes: Vec<u8> = (0..200_000usize).map(|i| (i % 239) as u8).collect();
    let hash = format!("{:x}", sha2::Sha256::digest(&bytes));
    let start = |http: &reqwest::blocking::Client, base: &str, token: &str| -> Value {
        let response = http
            .post(format!("{base}/v1/uploads"))
            .bearer_auth(token)
            .json(&json!({"type":"file","name":"mobile.bin",
            "mimeType":"application/octet-stream","length":bytes.len(),"sha256":hash}))
            .send()
            .unwrap();
        assert!(response.status().is_success(), "{response:?}");
        response.json().unwrap()
    };
    let started = start(&http, &base, &client.token);
    let upload = started["uploadId"].as_str().unwrap();
    assert_eq!(started["offset"], 0);
    let put = |http: &reqwest::blocking::Client,
               base: &str,
               token: &str,
               upload: &str,
               offset: usize,
               chunk: &[u8]| {
        http.put(format!("{base}/v1/uploads/{upload}?offset={offset}"))
            .bearer_auth(token)
            .body(chunk.to_vec())
            .send()
            .unwrap()
    };
    // Chunks land only at the acknowledged offset; a dropped write is retried there.
    let (head, tail) = bytes.split_at(90_000);
    assert!(
        put(&http, &base, &client.token, upload, 0, head)
            .status()
            .is_success()
    );
    let refused = put(&http, &base, &client.token, upload, 0, tail);
    assert_eq!(refused.status(), 400);
    assert_eq!(
        refused.json::<Value>().unwrap()["error"]["message"],
        "Invalid upload offset or size."
    );
    let resumed = put(&http, &base, &client.token, upload, head.len(), tail);
    assert!(resumed.status().is_success(), "{resumed:?}");
    assert_eq!(
        resumed.json::<Value>().unwrap()["offset"],
        bytes.len() as u64
    );
    // Committing early refuses while any byte is missing.
    let mut second = start(&http, &base, &client.token);
    let incomplete = http
        .post(format!(
            "{base}/v1/uploads/{}/commit",
            second["uploadId"].as_str().unwrap()
        ))
        .bearer_auth(&client.token)
        .send()
        .unwrap();
    assert_eq!(incomplete.status(), 400);
    assert_eq!(
        incomplete.json::<Value>().unwrap()["error"]["message"],
        "Incomplete upload."
    );
    let committed: Value = http
        .post(format!("{base}/v1/uploads/{upload}/commit"))
        .bearer_auth(&client.token)
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_golden("upload_attachment", &normalize(&committed, &profile.root));
    let id = committed["id"].as_str().unwrap();
    assert_eq!(id, format!("file-{hash}.bin"));
    // Stored bytes page back over the same bounded offset contract.
    let mut assembled = Vec::new();
    while assembled.len() < bytes.len() {
        let page = http
            .get(format!(
                "{base}/v1/blobs/{id}?offset={}&limit={}",
                assembled.len(),
                protocol::PAGE_BYTES
            ))
            .bearer_auth(&client.token)
            .send()
            .unwrap();
        assert!(page.status().is_success(), "{page:?}");
        assert_eq!(
            page.headers()["x-blob-length"].to_str().unwrap(),
            bytes.len().to_string()
        );
        let chunk = page.bytes().unwrap();
        assert!(!chunk.is_empty() && chunk.len() <= protocol::PAGE_BYTES);
        assembled.extend_from_slice(&chunk);
    }
    assert_eq!(assembled, bytes);
    // A server restart forgets in-flight uploads; the mobile client must restart
    // the upload, not resume it.
    profile.stop();
    profile.launch(&["serve"]);
    let client = profile.client(device);
    let base = format!("http://127.0.0.1:{}", profile.discovery()["port"]);
    let stale = put(&http, &base, &client.token, upload, 0, head);
    assert_eq!(stale.status(), 400);
    assert_eq!(
        stale.json::<Value>().unwrap()["error"]["message"],
        "Upload expired."
    );
    second = start(&http, &base, &client.token);
    let upload = second["uploadId"].as_str().unwrap();
    assert!(
        put(&http, &base, &client.token, upload, 0, &bytes)
            .status()
            .is_success()
    );
    let recommitted: Value = http
        .post(format!("{base}/v1/uploads/{upload}/commit"))
        .bearer_auth(&client.token)
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(recommitted["id"], id, "content addressing survives restart");
    profile.stop();
}
