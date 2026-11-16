//! Captain lifecycle and independent crew commands over a real serve process.
//! Every mutation goes through the command envelope so receipts, version
//! fencing and restart dedup are exercised end to end. No provider calls: the
//! harness is the fake Pi fixture.
use super::*;

/// Captain commands are negotiated: hello at the current minor unlocks the
/// archive, roles, delete and crew-control capability gates.
fn hello(client: &mut Client, id: &str) {
    let reply = client.rpc(
        "hello",
        json!({"protocolMajor":protocol::MAJOR,"protocolMinor":protocol::MINOR,
        "projectionSchema":1}),
        json!(id),
    );
    assert!(reply["error"].is_null(), "{reply}");
}

fn accepted(reply: &Value) -> Value {
    assert_eq!(reply["result"]["status"], "accepted", "{reply}");
    reply["result"]["result"].clone()
}

fn rejected(reply: &Value) -> Value {
    assert_eq!(reply["result"]["status"], "rejected", "{reply}");
    reply["result"]["result"]["error"].clone()
}

fn lifecycle_preview(client: &mut Client, captain: usize, action: &str) -> Value {
    let reply = client.command(
        "captain.lifecycle.preview",
        json!({"captain":captain,"action":action}),
        uuid::Uuid::new_v4(),
    );
    let preview = accepted(&reply);
    assert_eq!(preview["captain"], captain);
    assert_eq!(preview["action"], action);
    assert!(preview["captainVersion"].is_string(), "{preview}");
    assert!(preview["crewSeen"].is_array(), "{preview}");
    preview
}

fn index_thread(client: &Client, id: u64) -> Value {
    client.snapshot["index"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("thread {id} missing: {}", client.snapshot))
}

fn thread_view(client: &Client, id: u64) -> Value {
    client.snapshot["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("thread view {id} missing: {}", client.snapshot))
}

fn captain_open(client: &mut Client, name: &str) -> u64 {
    let put = client.command(
        "captain.put",
        json!({"record":{"name":name,"role":"Keeper","instructions":"Guard the harbor.","portrait":1}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(put["result"]["status"], "accepted", "{put}");
    let opened = client.command(
        "captain.open",
        json!({"captain":0,"project":null}),
        uuid::Uuid::new_v4(),
    );
    accepted(&opened)["threadId"].as_u64().unwrap()
}

fn prompts(profile: &Profile, thread: u64) -> Vec<String> {
    fs::read_to_string(
        profile
            .root
            .join(format!("Shina/chats/{thread}/fixture-prompts.jsonl")),
    )
    .map(|text| text.lines().map(str::to_owned).collect())
    .unwrap_or_default()
}

fn until_prompts(profile: &Profile, thread: u64, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while prompts(profile, thread).len() < count {
        assert!(
            Instant::now() < deadline,
            "fake harness saw too few prompts for thread {thread}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn report_policy_and_activity_are_durable_without_overwriting_the_saved_summary() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    hello(&mut client, "policy-hello");
    let home = captain_open(&mut client, "Harbor");
    let summary = "A hand-written summary stays here.";
    accepted(&client.command(
        "captain.document.write",
        json!({
            "captain":0,"name":"DIGEST.md","text":summary,"expectedRevision":0
        }),
        uuid::Uuid::new_v4(),
    ));
    let command = uuid::Uuid::new_v4();
    let args = json!({"captain":0,"policy":"ignore","expectedPolicy":"react"});
    let receipt = client.command("captain.reportPolicy", args.clone(), command);
    assert_eq!(accepted(&receipt)["reportPolicy"], "ignore");
    assert_eq!(
        client.command("captain.reportPolicy", args.clone(), command)["result"],
        receipt["result"]
    );
    let stale = client.command(
        "captain.reportPolicy",
        json!({
            "captain":0,"policy":"context","expectedPolicy":"react"
        }),
        uuid::Uuid::new_v4(),
    );
    assert!(rejected(&stale).as_str().unwrap().contains("changed since"));
    let created = accepted(&client.command(
        "captain.crew.create",
        json!({
            "captain":0,"input":{"text":"Report a bounded fixture outcome","attachments":[]}
        }),
        uuid::Uuid::new_v4(),
    ));
    let worker = created["threadId"].as_u64().unwrap();
    client.until(|state| {
        state["index"]["captainReports"]
            .as_array()
            .is_some_and(|reports| reports.iter().any(|report| report["threadId"] == worker))
    });
    let read =
        accepted(&client.command("captain.read", json!({"captain":0}), uuid::Uuid::new_v4()));
    assert_eq!(read["reportPolicy"], "ignore");
    assert_eq!(read["digest"]["text"], summary);
    let reports = read["activityDigest"]["whatsNew"].as_array().unwrap();
    assert!(reports.iter().any(|report| report["threadId"] == worker
        && report["reportDelivered"] == false
        && report["reportPolicy"] == "ignore"));
    assert!(
        prompts(&profile, home).is_empty(),
        "ignored report must not wake its Captain"
    );
    let report = client.snapshot["index"]["captainReports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|report| report["threadId"] == worker)
        .unwrap()
        .clone();
    assert_eq!(report["policy"], "ignore");
    assert_eq!(report["delivered"], false);
    // A suppressed report stays in its owning Captain's digest after release.
    accepted(&client.command(
        "captain.crew.release",
        json!({
            "captain":0,"threadId":worker
        }),
        uuid::Uuid::new_v4(),
    ));
    profile.stop();
    drop(client);
    profile.launch(&["serve"]);
    let mut client = profile.client(device);
    hello(&mut client, "policy-restarted");
    assert_eq!(
        client.command("captain.reportPolicy", args, command)["result"],
        receipt["result"]
    );
    let read =
        accepted(&client.command("captain.read", json!({"captain":0}), uuid::Uuid::new_v4()));
    assert_eq!(read["reportPolicy"], "ignore");
    assert_eq!(read["digest"]["text"], summary);
    assert!(
        read["activityDigest"]["whatsNew"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["reportId"] == report["id"] && entry["reportDelivered"] == false
            })
    );
    assert!(prompts(&profile, home).is_empty());
    profile.stop();
}

#[test]
fn captain_archive_unarchive_receipts_fence_stale_previews_across_restart() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    hello(&mut client, "captain-hello");
    let home = captain_open(&mut client, "Harbor");
    // The preview rides the command envelope: an accepted receipt carries the
    // version string and the exact crew list a confirm must echo back.
    let preview = lifecycle_preview(&mut client, 0, "archive");
    assert_eq!(preview["crewSeen"], json!([]));
    assert_eq!(preview["affectedThreads"], json!([home]));
    assert_eq!(preview["blockers"], json!([]));
    // A confirmation that does not echo the preview's exact crewSeen is stale.
    let wrong_crew = client.command(
        "captain.archive",
        json!({"captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":[home + 100]}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        rejected(&wrong_crew)
            .as_str()
            .unwrap()
            .contains("Captain changed since version"),
        "{wrong_crew}"
    );
    let archive_args = json!({"captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"]});
    let archive_id = uuid::Uuid::new_v4();
    let archived = client.command("captain.archive", archive_args.clone(), archive_id);
    assert_eq!(archived["result"]["result"]["lifecycle"], "archived");
    client.until(|state| state["index"]["captainLifecycles"]["0"]["phase"] == "archived");
    // Replaying the same command UUID returns the stored receipt, no re-run.
    let replay = client.command("captain.archive", archive_args, archive_id);
    assert_eq!(replay["result"], archived["result"], "{replay}");
    // The group record and the archived marker survive a server restart.
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(device);
    hello(&mut client, "captain-restarted");
    assert_eq!(
        client.snapshot["index"]["captainLifecycles"]["0"]["phase"],
        "archived"
    );
    assert!(index_thread(&client, home)["lifecycle"]["archivedAt"].is_number());
    // Durable dedup resolves the same UUID after restart, and the receipt
    // stays byte-identical to the pre-restart acknowledgement.
    let replay = client.command(
        "captain.archive",
        json!({"captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"]}),
        archive_id,
    );
    assert_eq!(replay["result"], archived["result"], "{replay}");
    // The pre-archive preview version is now stale: the mutation is refused
    // and nothing changes.
    let stale = client.command(
        "captain.unarchive",
        json!({"captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":[]}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        rejected(&stale)
            .as_str()
            .unwrap()
            .contains("Captain changed since version"),
        "{stale}"
    );
    assert_eq!(
        client.snapshot["index"]["captainLifecycles"]["0"]["phase"], "archived",
        "a stale confirmation must not mutate: {}",
        client.snapshot
    );
    // A fresh preview re-arms the confirmation exactly.
    let preview = lifecycle_preview(&mut client, 0, "unarchive");
    assert_eq!(preview["blockers"], json!([]));
    let restored = client.command(
        "captain.unarchive",
        json!({"captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"]}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(restored["result"]["result"]["lifecycle"], "active");
    client.until(|state| state["index"]["captainLifecycles"]["0"].is_null());
    client.until(|state| {
        state["index"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == home && thread["lifecycle"]["archivedAt"].is_null())
    });
    profile.stop();
}

#[test]
fn captain_promote_demote_and_delete_keep_thread_identity_and_conversations() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    hello(&mut client, "roles-hello");
    let created = client.command(
        "thread.create",
        json!({"title":"Promotable thread"}),
        uuid::Uuid::new_v4(),
    );
    let tid = accepted(&created)["threadId"].as_u64().unwrap();
    client.until(|state| {
        state["index"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == tid)
    });
    let before = index_thread(&client, tid);
    let preview = accepted(&client.command(
        "captain.promote.preview",
        json!({"threadId":tid}),
        uuid::Uuid::new_v4(),
    ));
    assert!(preview["threadVersion"].is_string(), "{preview}");
    assert_eq!(preview["blockers"], json!([]), "{preview}");
    assert_eq!(preview["permissionsChanged"], false);
    let promoted = client.command(
        "captain.promote",
        json!({"threadId":tid,"record":{"name":"Ada","role":"Lead","instructions":"Keep the log.","portrait":2},
        "expectedVersion":preview["threadVersion"]}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(promoted["result"]["status"], "accepted", "{promoted}");
    assert_eq!(promoted["result"]["result"]["captain"], 0);
    assert_eq!(promoted["result"]["result"]["threadId"], tid);
    client.until(|state| {
        state["index"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == tid && thread["captainHome"] == true)
    });
    // Promotion changes only the role: same thread id, project and access.
    let home = index_thread(&client, tid);
    assert_eq!(home["captain"], 0);
    assert_eq!(home["project"], before["project"]);
    assert_eq!(home["accessMode"], before["accessMode"]);
    assert_eq!(home["session"], before["session"]);
    // Demotion through the shared preview/confirm pair retires the profile
    // and returns the same thread to an ordinary conversation.
    let preview = lifecycle_preview(&mut client, 0, "demote");
    assert_eq!(preview["blockers"], json!([]), "{preview}");
    let demoted = client.command(
        "captain.demote",
        json!({"captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"]}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(demoted["result"]["result"]["lifecycle"], "retired");
    assert_eq!(demoted["result"]["result"]["threadId"], tid);
    client.until(|state| {
        state["index"]["captainLifecycles"]["0"]["phase"] == "retired"
            && state["index"]["captainLifecycles"]["0"]["formerHome"] == tid
    });
    let ordinary = index_thread(&client, tid);
    assert_eq!(ordinary["captainHome"], false);
    assert_eq!(ordinary["captain"], Value::Null);
    assert_eq!(ordinary["project"], before["project"]);
    assert_eq!(ordinary["accessMode"], before["accessMode"]);
    // Deletion requires the typed name and a fresh preview. A wrong name is
    // refused before the version is even checked for mutation.
    let preview = lifecycle_preview(&mut client, 0, "delete");
    let mistyped = client.command(
        "captain.delete",
        json!({"captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"],
        "confirmName":"Not Ada","keepConversations":true}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        rejected(&mistyped)
            .as_str()
            .unwrap()
            .contains("exact current name"),
        "{mistyped}"
    );
    assert_eq!(
        client.snapshot["index"]["captainLifecycles"]["0"]["phase"],
        "retired"
    );
    let deleted = client.command(
        "captain.delete",
        json!({"captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"],
        "confirmName":"Ada","keepConversations":true}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(deleted["result"]["status"], "accepted", "{deleted}");
    assert_eq!(deleted["result"]["result"]["keptConversations"], true);
    client.until(|state| state["index"]["captainLifecycles"]["0"]["phase"] == "deleted");
    // The conversation survives with its id; only the profile record is gone.
    assert_eq!(
        client.snapshot["index"]["captains"][0]["name"],
        "Deleted Captain 0"
    );
    let kept = index_thread(&client, tid);
    assert_eq!(kept["captain"], Value::Null);
    assert_eq!(kept["captainHome"], false);
    profile.stop();
}

#[test]
fn independent_crew_create_send_stop_and_uuid_replay_across_restart() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    hello(&mut client, "crew-hello");
    let home = captain_open(&mut client, "Quartermaster");
    let create_args = json!({"captain":0,"title":"Scout","input":{"text":"crew-scout-fixture"}});
    let create_id = uuid::Uuid::new_v4();
    let created = client.command("captain.crew.create", create_args.clone(), create_id);
    let result = accepted(&created);
    assert_eq!(result["saved"], true);
    assert_eq!(result["captain"], 0);
    let crew = result["threadId"].as_u64().unwrap();
    assert_ne!(crew, home);
    assert!(result["runId"].is_string(), "{result}");
    assert!(result["runtimeGeneration"].is_string(), "{result}");
    client.until(|state| {
        state["index"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == crew)
    });
    // An independent root: in the Captain's crew, with no Task record, no
    // parent link and no delegated flag.
    let meta = index_thread(&client, crew);
    assert_eq!(meta["crewCaptain"], 0, "{meta}");
    assert_ne!(meta["captainHome"], true, "{meta}");
    assert_eq!(meta["parent"], Value::Null, "{meta}");
    assert_ne!(meta["delegated"], true, "{meta}");
    assert_eq!(client.snapshot["index"]["tasks"], json!([]));
    until_prompts(&profile, crew, 1);
    assert!(
        prompts(&profile, crew)[0].contains("crew-scout-fixture"),
        "{:?}",
        prompts(&profile, crew)
    );
    // The same command UUID replays its durable receipt; the fake Pi sees
    // the root input exactly once.
    let replay = client.command("captain.crew.create", create_args, create_id);
    assert_eq!(replay["result"], created["result"], "{replay}");
    until_prompts(&profile, crew, 1);
    assert_eq!(prompts(&profile, crew).len(), 1);
    // crew.send pushes one saved input through the ordinary queue.
    let sent = client.command(
        "captain.crew.send",
        json!({"captain":0,"threadId":crew,"input":{"text":"crew-followup-fixture"}}),
        uuid::Uuid::new_v4(),
    );
    let result = accepted(&sent);
    assert_eq!(result["saved"], true);
    assert_eq!(result["threadId"], crew);
    assert!(result["inputId"].is_number(), "{result}");
    until_prompts(&profile, crew, 2);
    // Let the run settle so the stop fence below is deterministic.
    client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == crew && thread["status"] == "Completed")
    });
    let view = thread_view(&client, crew);
    // A stop naming another generation is fenced, with no state change.
    let stale = client.command(
        "captain.crew.stop",
        json!({"captain":0,"threadId":crew,"runId":view["run_id"],"runtimeGeneration":"stale-generation"}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        rejected(&stale)
            .as_str()
            .unwrap()
            .contains("earlier crew run or runtime generation"),
        "{stale}"
    );
    let stopped = client.command(
        "captain.crew.stop",
        json!({"captain":0,"threadId":crew,"runId":view["run_id"],"runtimeGeneration":view["generation"]}),
        uuid::Uuid::new_v4(),
    );
    let result = accepted(&stopped);
    assert_eq!(result["saved"], true);
    assert_eq!(result["threadId"], crew);
    // A thread outside the crew refuses the same control.
    let stranger = client.command(
        "captain.crew.send",
        json!({"captain":0,"threadId":home,"input":{"text":"not-independent"}}),
        uuid::Uuid::new_v4(),
    );
    assert!(rejected(&stranger).is_string(), "{stranger}");
    // Restart. The durable create receipt still dedupes and never re-prompts.
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(device);
    hello(&mut client, "crew-restarted");
    let replay = client.command(
        "captain.crew.create",
        json!({"captain":0,"title":"Scout","input":{"text":"crew-scout-fixture"}}),
        create_id,
    );
    assert_eq!(replay["result"], created["result"], "{replay}");
    assert_eq!(
        prompts(&profile, crew).len(),
        2,
        "a durable receipt replay must not re-prompt"
    );
    let meta = index_thread(&client, crew);
    assert_eq!(meta["crewCaptain"], 0, "{meta}");
    assert_eq!(meta["parent"], Value::Null, "{meta}");
    profile.stop();
}
