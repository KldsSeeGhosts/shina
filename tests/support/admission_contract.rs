//! Admission scheduler contract over a real authenticated serve process.
//! Every mutation goes through the command envelope and the only provider
//! is the fake Pi fixture. These tests pin the #23 contract end to end:
//! one scheduler for crew roots and delegated work, a durable reservation
//! committed before provider startup, an observable queue, atomic
//! configure receipts, restart behaviour and remote uncertainty.
use super::captain_contract::{accepted, captain_open, hello, prompts, until_prompts};
use super::*;

fn scheduling_status(client: &mut Client) -> Value {
    let reply = client.command("scheduling.status", json!({}), uuid::Uuid::new_v4());
    accepted(&reply)
}

fn scheduling_configure(client: &mut Client, args: Value) -> Value {
    let reply = client.command("scheduling.configure", args, uuid::Uuid::new_v4());
    accepted(&reply)
}

fn captain_open_id(client: &mut Client, name: &str, captain: usize) -> u64 {
    let put = client.command(
        "captain.put",
        json!({"record":{"name":name,"role":"Keeper","instructions":"Guard the harbor.","portrait":1}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(put["result"]["status"], "accepted", "{put}");
    accepted(&client.command(
        "captain.open",
        json!({"captain":captain,"project":null}),
        uuid::Uuid::new_v4(),
    ))["threadId"]
        .as_u64()
        .unwrap()
}

fn delegate(client: &mut Client, home: u64, prompt: &str, key: &str) -> Value {
    let reply = client.command(
        "task.delegate",
        json!({"threadId":home,"args":{"prompt":prompt,"client_request_id":key}}),
        uuid::Uuid::new_v4(),
    );
    accepted(&reply)
}

fn task_record(client: &Client, task_id: &str) -> Value {
    client.snapshot["index"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == task_id)
        .cloned()
        .unwrap_or_else(|| panic!("task {task_id} missing: {}", client.snapshot))
}

fn assert_proxy_send_refused(client: &mut Client, child: &Value, task_id: &str) {
    let task = task_record(client, task_id);
    let before = scheduling_status(client)["usage"]["account"].clone();
    let previous = client.snapshot["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == *child)
        .unwrap()
        .clone();
    let sent = client.command(
        "thread.send",
        json!({"threadId":child,"text":"proxy input must not run locally"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(sent["result"]["status"], "rejected", "{sent}");
    assert!(
        sent["result"]["result"]["error"]
            .as_str()
            .unwrap()
            .contains("remote task"),
        "{sent}"
    );
    let after = scheduling_status(client)["usage"]["account"].clone();
    assert_eq!(after["held"], before["held"]);
    assert_eq!(after["queued"], before["queued"]);
    assert_eq!(task_record(client, task_id)["state"], task["state"]);
    let proxy = client.snapshot["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == *child)
        .unwrap();
    assert_eq!(proxy["status"], previous["status"], "{proxy}");
    assert_eq!(proxy["queued"], previous["queued"], "{proxy}");
}

#[test]
fn crew_roots_share_the_account_limit_and_the_queue_survives_restart() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    hello(&mut client, "admission-crew");
    captain_open(&mut client, "Quartermaster");

    // The configure contract: a stale compare-and-set refuses, an invalid
    // field rejects the whole call, and one accepted write lands atomically.
    let base = scheduling_status(&mut client);
    let stale = scheduling_configure(
        &mut client,
        json!({"expectRevision":base["revision"].as_u64().unwrap() + 7,
            "limits":{"accountRunning":1}}),
    );
    assert_eq!(stale["outcome"], "stale", "{stale}");
    let invalid = scheduling_configure(
        &mut client,
        json!({"limits":{"accountRunning":1,"executorRunning":0}}),
    );
    assert_eq!(invalid["outcome"], "rejected", "{invalid}");
    assert_eq!(
        scheduling_status(&mut client)["limits"]["accountRunning"],
        base["limits"]["accountRunning"],
        "a rejected configure must not partially apply earlier fields"
    );
    let receipt = scheduling_configure(&mut client, json!({"limits":{"accountRunning":1}}));
    assert_eq!(receipt["outcome"], "accepted", "{receipt}");

    // Root A starts immediately and holds its provider turn.
    let root_a = accepted(&client.command(
        "captain.crew.create",
        json!({"captain":0,"title":"A","input":{"text":"queue-wait-fixture"}}),
        uuid::Uuid::new_v4(),
    ));
    assert_eq!(root_a["admission"]["state"], "held", "{root_a}");
    let crew_a = root_a["threadId"].as_u64().unwrap();
    until_prompts(&profile, crew_a, 1);

    // Root B is accepted but queued against the shared account cap.
    let root_b = accepted(&client.command(
        "captain.crew.create",
        json!({"captain":0,"title":"B","input":{"text":"crew-b-fixture"}}),
        uuid::Uuid::new_v4(),
    ));
    assert_eq!(root_b["admission"]["state"], "queued", "{root_b}");
    assert_eq!(root_b["admission"]["blockedBy"], "account", "{root_b}");
    let crew_b = root_b["threadId"].as_u64().unwrap();
    client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == crew_b)
    });
    let status = scheduling_status(&mut client);
    assert_eq!(status["usage"]["account"]["held"], 1, "{status}");
    assert_eq!(status["usage"]["account"]["queued"], 1, "{status}");
    assert_eq!(status["queue"][0]["position"], 1, "{status}");
    assert_eq!(status["queue"][0]["blockedBy"], "account", "{status}");
    assert_eq!(
        status["queue"][0]["subject"]["threadId"], crew_b,
        "{status}"
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        prompts(&profile, crew_b).is_empty(),
        "a queued root must never reach the provider: {:?}",
        prompts(&profile, crew_b)
    );

    // The queued reservation is durable: A's live hold died with the
    // process, so the surviving line grants B exactly once after restart.
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(device);
    hello(&mut client, "admission-crew-restart");
    client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == crew_b)
    });
    let status = scheduling_status(&mut client);
    assert!(
        status["queue"]
            .as_array()
            .unwrap()
            .iter()
            .chain(status["held"].as_array().unwrap())
            .any(|row| row["subject"]["threadId"] == crew_b),
        "crew B's reservation must survive restart: {status}"
    );
    until_prompts(&profile, crew_b, 1);
    assert!(
        prompts(&profile, crew_b)[0].contains("crew-b-fixture"),
        "{:?}",
        prompts(&profile, crew_b)
    );
    client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == crew_b && thread["status"] == "Completed")
    });
    let status = scheduling_status(&mut client);
    assert_eq!(status["queue"], json!([]), "{status}");
    assert_eq!(status["usage"]["account"]["held"], 0, "{status}");
    profile.stop();
}

#[test]
fn delegated_tasks_obey_coordinator_scopes_and_fairness() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    hello(&mut client, "admission-tasks");
    let home_a = captain_open_id(&mut client, "First", 0);
    let home_b = captain_open_id(&mut client, "Second", 1);
    let receipt = scheduling_configure(&mut client, json!({"limits":{"coordinatorRunning":1}}));
    assert_eq!(receipt["outcome"], "accepted", "{receipt}");

    // Each Captain home is a stable coordinator scope: captain:0 and
    // captain:1 each run one, never double.
    let task_a = delegate(&mut client, home_a, "peer-documents-hold-fixture", "cap-a");
    let task_a_id = task_a["task_id"].as_str().unwrap().to_owned();
    let task_b = delegate(&mut client, home_b, "peer-documents-hold-fixture", "cap-b");
    let task_b_id = task_b["task_id"].as_str().unwrap().to_owned();
    client.until(|state| {
        let tasks = state["index"]["tasks"].as_array().unwrap();
        tasks
            .iter()
            .any(|task| task["id"] == task_a_id && task["state"] == "running")
            && tasks
                .iter()
                .any(|task| task["id"] == task_b_id && task["state"] == "running")
    });
    let status = scheduling_status(&mut client);
    let coordinator = |id: &str| {
        status["usage"]["coordinators"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("coordinator {id} missing: {status}"))
    };
    assert_eq!(coordinator("captain:0")["held"], 1, "{status}");
    assert_eq!(coordinator("captain:1")["held"], 1, "{status}");

    // A second task under captain:0 queues with a visible reason while
    // captain:1 keeps its own slot: the scopes are independent.
    let task_c = delegate(&mut client, home_a, "peer-documents-hold-fixture", "cap-c");
    let task_c_id = task_c["task_id"].as_str().unwrap().to_owned();
    let status = scheduling_status(&mut client);
    assert_eq!(status["queue"].as_array().unwrap().len(), 1, "{status}");
    assert_eq!(status["queue"][0]["blockedBy"], "coordinator", "{status}");
    assert_eq!(
        status["queue"][0]["coordinatorKey"], "captain:0",
        "{status}"
    );
    assert_eq!(status["queue"][0]["position"], 1, "{status}");

    // Cancelling the running task frees exactly its coordinator's slot and
    // the queued attempt grants.
    let attempt_a = task_record(&client, &task_a_id)["attempt_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let child_a = task_record(&client, &task_a_id)["child_thread_id"]
        .as_u64()
        .unwrap();
    let cancel = client.command(
        "task.cancel",
        json!({"threadId":home_a,"args":{"thread_id":child_a,
            "client_request_id":"cancel-a","attempt_id":attempt_a}}),
        uuid::Uuid::new_v4(),
    );
    assert_ne!(cancel["result"]["status"], "rejected", "{cancel}");
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == task_c_id && task["state"] == "running")
    });
    let status = scheduling_status(&mut client);
    assert_eq!(status["queue"], json!([]), "{status}");
    assert_eq!(coordinator("captain:0")["held"], 1, "{status}");
    profile.stop();
}

#[test]
fn a_failed_admission_commit_interrupts_before_any_provider_effect() {
    let mut profile = Profile::new();
    profile.launch_with_fault(&["serve"], Some("admission-commit"));
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    hello(&mut client, "admission-commit");
    let home = captain_open(&mut client, "Quartermaster");
    let task = delegate(&mut client, home, "commit-fault-fixture", "commit-1");
    let task_id = task["task_id"].as_str().unwrap().to_owned();
    let child = task["thread_id"].as_u64().unwrap();

    // The reservation commits before the prompt can reach the provider; a
    // commit failure interrupts the task instead of starting it, and no
    // durable occupancy leaks.
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == task_id && task["state"] == "interrupted")
    });
    let status = scheduling_status(&mut client);
    assert_eq!(status["queue"], json!([]), "{status}");
    assert_eq!(status["held"], json!([]), "{status}");
    assert!(
        prompts(&profile, child).is_empty(),
        "the provider must never see a prompt without a committed reservation"
    );

    // Without the fault the same task resumes cleanly and runs once.
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(device);
    hello(&mut client, "admission-commit-resume");
    accepted(&client.command(
        "task.resume",
        json!({"taskId":task_id}),
        uuid::Uuid::new_v4(),
    ));
    until_prompts(&profile, child, 1);
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == task_id && task["state"] == "completed")
    });
    assert_eq!(prompts(&profile, child).len(), 1);
    profile.stop();
}

#[test]
fn provider_rate_limit_holds_queued_work_and_persists_backoff() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    hello(&mut client, "admission-provider-backoff");
    let home = captain_open(&mut client, "Quartermaster");
    let first = delegate(
        &mut client,
        home,
        "admission-rate-limit-fixture",
        "provider-refusal",
    );
    let first_id = first["task_id"].as_str().unwrap().to_owned();
    let first_child = first["thread_id"].as_u64().unwrap();
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == first_id && task["state"] == "failed")
    });
    let failed = task_record(&client, &first_id);
    assert!(
        failed["error_excerpt"].as_str().unwrap().contains("429"),
        "{failed}"
    );
    let status = scheduling_status(&mut client);
    let deadline = |status: &Value| {
        status["providerBackoff"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["provider"] == "cpa"))
            .and_then(|row| row["until"].as_u64())
            .unwrap_or_else(|| panic!("provider backoff missing: {status}"))
    };
    let backoff = deadline(&status);
    assert!(backoff > 0, "{status}");
    assert_eq!(status["usage"]["account"]["held"], 0, "{status}");
    assert_eq!(prompts(&profile, first_child).len(), 1);

    let second = delegate(
        &mut client,
        home,
        "after-rate-limit-fixture",
        "after-refusal",
    );
    let second_id = second["task_id"].as_str().unwrap().to_owned();
    let second_child = second["thread_id"].as_u64().unwrap();
    let status = scheduling_status(&mut client);
    assert_eq!(
        status["queue"][0]["subject"]["taskId"], second_id,
        "{status}"
    );
    assert_eq!(
        status["queue"][0]["blockedBy"], "provider_backoff",
        "{status}"
    );
    std::thread::sleep(Duration::from_millis(250));
    assert!(prompts(&profile, second_child).is_empty());
    assert_eq!(prompts(&profile, first_child).len(), 1);

    // Restart must preserve the same deadline rather than granting early or
    // retrying the refused input. Existing unit coverage advances past expiry.
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(device);
    hello(&mut client, "admission-provider-backoff-restart");
    let status = scheduling_status(&mut client);
    assert_eq!(deadline(&status), backoff, "{status}");
    assert_eq!(
        status["queue"][0]["subject"]["taskId"], second_id,
        "{status}"
    );
    assert_eq!(
        status["queue"][0]["blockedBy"], "provider_backoff",
        "{status}"
    );
    assert!(prompts(&profile, second_child).is_empty());
    assert_eq!(prompts(&profile, first_child).len(), 1);
    profile.stop();
}

#[test]
fn a_crash_after_the_reservation_never_starts_the_prompt_twice() {
    let mut profile = Profile::new();
    let process = profile.launch_with_fault(&["serve"], Some("pending"));
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    hello(&mut client, "admission-crash");
    let home = captain_open(&mut client, "Quartermaster");
    let command_id = uuid::Uuid::new_v4();
    let args =
        json!({"threadId":home,"args":{"prompt":"crash-fixture","client_request_id":"crash-1"}});
    // The process may exit before the socket can deliver the RPC response.
    client
        .socket
        .send(Message::Text(
            json!({"jsonrpc":"2.0","id":command_id,
        "method":"task.delegate","params":{"commandId":command_id,"args":args}})
            .to_string()
            .into(),
        ))
        .unwrap();

    // The process dies at the pending effect boundary, after the
    // reservation committed but before the prompt reached the harness.
    let deadline = Instant::now() + Duration::from_secs(15);
    let exit = loop {
        if let Some(exit) = profile.children[process].try_wait().unwrap() {
            break exit;
        }
        assert!(Instant::now() < deadline, "serve did not reach the crash");
        std::thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(exit.code(), Some(86), "expected the pending prompt crash");
    drop(client);
    fs::remove_file(profile.root.join("Shina/server.json")).unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(device);
    hello(&mut client, "admission-crash-restart");
    let replay = client.command("task.delegate", args, command_id);
    let receipt = &replay["result"];
    let task = match receipt["status"].as_str() {
        Some("accepted") => &receipt["result"],
        Some("interrupted") => &receipt["result"]["previousResult"],
        _ => panic!("Expected the committed delegation receipt: {replay}"),
    };
    let task_id = task["task_id"].as_str().unwrap().to_owned();
    let child = task["thread_id"].as_u64().unwrap();

    // Recovery never counts a crashed prompt as delivered work: the task
    // settles as interrupted with no provider side effect.
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == task_id && task["state"] == "interrupted")
    });
    assert!(
        prompts(&profile, child).is_empty(),
        "a prompt that never dispatched must not replay during recovery"
    );
    let status = scheduling_status(&mut client);
    assert_eq!(status["held"], json!([]), "{status}");
    accepted(&client.command(
        "task.resume",
        json!({"taskId":task_id}),
        uuid::Uuid::new_v4(),
    ));
    until_prompts(&profile, child, 1);
    assert_eq!(prompts(&profile, child).len(), 1);
    profile.stop();
}

#[test]
fn mcp_delegate_calls_share_the_one_account_limit() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    hello(&mut client, "admission-mcp");
    let workspace = profile.root.join("Shina/chats/1");
    // The parent run publishes the authenticated MCP endpoint the fixture
    // then calls back through.
    accepted(&client.command(
        "thread.send",
        json!({"threadId":1,"text":"Initialize the coordinator fixture",
            "attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    ));
    client.until(|state| state["threads"][0]["status"] == "Completed");
    let receipt = scheduling_configure(&mut client, json!({"limits":{"accountRunning":1}}));
    assert_eq!(receipt["outcome"], "accepted", "{receipt}");

    let call = |args: Value| {
        let response = call_fixture_tool(&workspace, "delegate_task", args);
        assert_ne!(response["result"]["isError"], true, "{response}");
        serde_json::from_str::<Value>(response["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap()
    };
    let first = call(
        json!({"prompt":"queue-wait-fixture","title":"MCP worker one",
            "client_request_id":"mcp-admission-1"}),
    );
    let held_child = first["thread_id"].as_u64().unwrap();
    until_prompts(&profile, held_child, 1);
    let second = call(
        json!({"prompt":"mcp-second-fixture","title":"MCP worker two",
            "client_request_id":"mcp-admission-2"}),
    );
    let queued_child = second["thread_id"].as_u64().unwrap();
    let queued_task = second["task_id"].as_str().unwrap().to_owned();

    // The second authenticated MCP call must land in the same queue a
    // websocket command would, never in a second provider slot.
    let status = scheduling_status(&mut client);
    assert_eq!(status["usage"]["account"]["held"], 1, "{status}");
    assert_eq!(status["usage"]["account"]["queued"], 1, "{status}");
    assert_eq!(
        status["queue"][0]["subject"]["taskId"], queued_task,
        "{status}"
    );
    assert_eq!(status["queue"][0]["blockedBy"], "account", "{status}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        prompts(&profile, queued_child).is_empty(),
        "a queued MCP task must never reach the provider: {:?}",
        prompts(&profile, queued_child)
    );

    // Cancelling the held attempt frees the slot; the queued MCP task
    // grants and its prompt reaches the provider exactly once.
    let held_task = first["task_id"].as_str().unwrap().to_owned();
    let held_attempt = task_record(&client, &held_task)["attempt_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let cancel = client.command(
        "task.cancel",
        json!({"threadId":1,"args":{"thread_id":held_child,
            "client_request_id":"mcp-cancel","attempt_id":held_attempt}}),
        uuid::Uuid::new_v4(),
    );
    assert_ne!(cancel["result"]["status"], "rejected", "{cancel}");
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == held_task && task["state"] == "cancelled")
    });
    until_prompts(&profile, queued_child, 1);
    assert!(
        prompts(&profile, queued_child)[0].contains("mcp-second-fixture"),
        "{:?}",
        prompts(&profile, queued_child)
    );
    profile.stop();
}

#[test]
fn rejected_remote_launch_releases_capacity_and_retries_once_after_restart() {
    let mut home = Profile::new();
    let mut target = Profile::new();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let (discovery, mut there, mut here, parent) = link_peer_servers(
        &mut target,
        &mut home,
        "https://example.com/admission-launch-refusal.git",
    );
    hello(&mut there, "refused-launch-target");
    hello(&mut here, "refused-launch-home");
    captain_open(&mut there, "Target capacity");
    scheduling_configure(
        &mut there,
        json!({"limits":{
            "accountRunning":1,"accountPending":1,
        }}),
    );
    let first = accepted(&there.command(
        "captain.crew.create",
        json!({"captain":0,"title":"Holding target capacity",
            "input":{"text":"queue-wait-fixture"}}),
        uuid::Uuid::new_v4(),
    ));
    let first_id = first["threadId"].as_u64().unwrap().to_string();
    there.until(|state| state["directories"][&first_id].is_string());
    let first_prompts = PathBuf::from(there.snapshot["directories"][&first_id].as_str().unwrap())
        .join("fixture-prompts.jsonl");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fs::read_to_string(&first_prompts)
        .unwrap_or_default()
        .contains("queue-wait-fixture")
    {
        assert!(
            Instant::now() < deadline,
            "the target's held provider did not start"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let queued = accepted(&there.command(
        "captain.crew.create",
        json!({"captain":0,"title":"Filling target queue",
            "input":{"text":"target-queued-capacity-fixture"}}),
        uuid::Uuid::new_v4(),
    ));
    assert_eq!(queued["admission"]["state"], "queued", "{queued}");
    scheduling_configure(&mut here, json!({"limits":{"accountRunning":1}}));
    let delegated = delegate_fixture(
        &mut here,
        &parent,
        "must be refused before target allocation",
        "refused-launch",
        &discovery,
    );
    let task_id = delegated["result"]["result"]["task_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let child = delegated["result"]["result"]["thread_id"].clone();
    here.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == task_id && task["state"] == "failed")
    });
    let failed = task_record(&here, &task_id);
    assert!(failed["remote"]["threadId"].is_null(), "{failed}");
    let original = failed["remote"]["launchCommand"]["commandId"].clone();
    assert_eq!(failed["remote"]["rejectedLaunch"], original, "{failed}");
    let status = scheduling_status(&mut here);
    assert_eq!(status["usage"]["account"]["held"], 0, "{status}");
    assert_eq!(status["usage"]["account"]["queued"], 0, "{status}");
    assert_proxy_send_refused(&mut here, &child, &task_id);
    there.rpc("workspace.subscribe", json!({}), json!("refused-target"));
    assert!(
        there.snapshot["index"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .all(|thread| thread["peerOrigin"]["taskId"] != task_id)
    );
    drop(here);
    home.stop();
    home.launch(&["serve"]);
    let mut here = home.client(uuid::Uuid::new_v4());
    hello(&mut here, "refused-launch-home-restart");
    let status = scheduling_status(&mut here);
    assert_eq!(status["usage"]["account"]["held"], 0, "{status}");
    assert_eq!(
        task_record(&here, &task_id)["remote"]["rejectedLaunch"],
        original
    );

    // Free a target slot. A new deliberate attempt gets a new launch command,
    // while retries of that attempt retain its single saved command.
    scheduling_configure(
        &mut there,
        json!({"limits":{
            "accountRunning":2,"accountPending":4,
        }}),
    );
    let retry = json!({"threadId":parent,"args":{
        "thread_id":child,"message":"retry-after-target-refusal",
        "client_request_id":"retry-refused-launch",
    }});
    let accepted_retry = here.command("task.send", retry.clone(), uuid::Uuid::new_v4());
    assert_eq!(
        accepted_retry["result"]["status"], "accepted",
        "{accepted_retry}"
    );
    let fresh = accepted_retry["result"]["result"]["remote"]["launchCommand"]["commandId"].clone();
    assert!(fresh.is_string());
    assert_ne!(fresh, original);
    here.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == task_id && task["state"] == "completed")
    });
    let operation = task_record(&here, &task_id);
    assert_eq!(operation["remote"]["launchCommand"]["commandId"], fresh);
    let status = scheduling_status(&mut here);
    assert_eq!(status["usage"]["account"]["held"], 0, "{status}");
    assert_proxy_send_refused(&mut here, &child, &task_id);
    let replay = here.command("task.send", retry, uuid::Uuid::new_v4());
    assert_eq!(replay["result"]["result"]["replayed"], true, "{replay}");
    assert_eq!(
        task_record(&here, &task_id)["remote"]["launchCommand"]["commandId"],
        fresh
    );
    let target_id = task_record(&here, &task_id)["remote"]["threadId"]
        .as_u64()
        .unwrap()
        .to_string();
    there.rpc("workspace.subscribe", json!({}), json!("retry-target"));
    let target_prompts = fs::read_to_string(
        PathBuf::from(there.snapshot["directories"][&target_id].as_str().unwrap())
            .join("fixture-prompts.jsonl"),
    )
    .unwrap();
    assert_eq!(
        target_prompts
            .lines()
            .filter(|line| line.contains("retry-after-target-refusal"))
            .count(),
        1
    );
    home.stop();
    target.stop();
}

#[test]
fn a_cancelled_remote_task_stays_uncertain_until_the_stop_lands() {
    let mut home = Profile::new();
    let mut target = Profile::new();
    // Stop the executor after launch acceptance but before the first adapter
    // write. Provider completion cannot settle this generation.
    let release = target.root.join("release-first-peer-input");
    fs::write(
        target.root.join("extra-env.json"),
        json!({"SHINA_TEST_HOLD_PEER_INPUT":release}).to_string(),
    )
    .unwrap();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let mut there = target.client(uuid::Uuid::new_v4());
    let workspace = target.root.join("workspace");
    let project = there.command(
        "project.put",
        json!({"path":workspace,"createFolder":true}),
        uuid::Uuid::new_v4(),
    );
    let project_id = project["result"]["result"]["projectId"].clone();
    let discovery = target.discovery();
    let server_id = discovery["serverId"].as_str().unwrap().to_owned();
    let paired = target.cli(&["pair", "--peer"]);
    assert!(paired.status.success());
    let output = String::from_utf8(paired.stdout).unwrap();
    let code = output
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let linked = home.cli(&[
        "peer",
        "link",
        &format!("127.0.0.1:{}", discovery["port"]),
        &code,
    ]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let mut here = home.client(uuid::Uuid::new_v4());
    hello(&mut here, "admission-remote");
    let home_thread = captain_open(&mut here, "Quartermaster");
    here.until(|state| {
        state["peers"]
            .as_array()
            .is_some_and(|peers| peers.iter().any(|peer| peer["serverId"] == server_id))
    });
    let scoped = here.command(
        "captain.scope",
        json!({"captain":0,"projects":[],"remoteProjects":[{"server":server_id,"projectId":project_id}]}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(scoped["result"]["status"], "accepted", "{scoped}");

    // An accepted remote launch occupies one home reservation even before its
    // provider sees the prompt.
    let task = {
        let reply = here.command(
            "task.delegate",
            json!({"threadId":home_thread,"args":{"prompt":"peer-documents-hold-fixture",
                "client_request_id":"remote-1",
                "target":{"environmentId":server_id,"projectId":project_id,"model":"fixture"}}}),
            uuid::Uuid::new_v4(),
        );
        accepted(&reply)
    };
    let task_id = task["task_id"].as_str().unwrap().to_owned();
    here.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == task_id && task["remote"]["threadId"].is_number())
    });
    there.until(|state| {
        state["index"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["peerOrigin"]["taskId"].is_string())
    });

    let executor_thread = task_record(&here, &task_id)["remote"]["threadId"]
        .as_u64()
        .unwrap();
    let directory = there.snapshot["directories"][executor_thread.to_string()]
        .as_str()
        .map(PathBuf::from)
        .expect("executor working directory");
    let provider_prompts = directory.join("fixture-prompts.jsonl");
    assert!(!provider_prompts.exists());

    // While the executor is offline, the controller cannot know whether the
    // first prompt was sent. Keep its reservation until the stop settles.
    target.stop();
    let attempt = task_record(&here, &task_id)["attempt_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let child = task_record(&here, &task_id)["child_thread_id"]
        .as_u64()
        .unwrap();
    let cancel = here.command(
        "task.cancel",
        json!({"threadId":home_thread,"args":{"thread_id":child,
            "client_request_id":"remote-cancel","attempt_id":attempt}}),
        uuid::Uuid::new_v4(),
    );
    assert_ne!(cancel["result"]["status"], "rejected", "{cancel}");
    here.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == task_id && task["state"] == "cancelled")
    });
    let status = scheduling_status(&mut here);
    let held = status["held"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["subject"]["taskId"] == task_id)
        .cloned()
        .unwrap_or_else(|| panic!("cancelled remote reservation missing: {status}"));
    assert_eq!(held["uncertain"], true, "{status}");

    // Once the peer returns, the owed stop lands and the uncertain slot
    // finally releases. The link stores the peer's port, so the relaunch
    // must reclaim it.
    target.launch(&["serve", "--port", &discovery["port"].to_string()]);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = scheduling_status(&mut here);
        let still = status["held"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["subject"]["taskId"] == task_id);
        if !still {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "owed stop never landed: {status}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let input = durable_task_inputs(&target)
        .into_iter()
        .find(|input| input["kind"] == "input" && input["attempt_id"] == attempt)
        .expect("the cancelled launch's input");
    assert_eq!(input["provider_state"], "rejected", "{input}");
    assert_eq!(
        input["provider_basis"], "target_cancelled_before_dispatch",
        "{input}"
    );
    assert!(!provider_prompts.exists());
    home.stop();
    target.stop();
}
