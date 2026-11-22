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
