//! Effective capability contracts over real serve processes. Each fake
//! harness (Pi, native Claude, native Codex, ACP) records its thread's
//! token-scoped MCP endpoint; the tests then drive initialize, tools/list and
//! tools/call through the same dispatch and permission paths a real agent
//! would. No provider calls.
use super::*;

fn hello(client: &mut Client, id: &str) {
    let reply = client.rpc(
        "hello",
        json!({"protocolMajor":protocol::MAJOR,"protocolMinor":protocol::MINOR,
        "projectionSchema":1}),
        json!(id),
    );
    assert!(reply["error"].is_null(), "{reply}");
}

fn captain_open(client: &mut Client, name: &str) -> u64 {
    let put = client.command(
        "captain.put",
        json!({"record":{"name":name,"role":"Keeper","instructions":"Guard.","portrait":1}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(put["result"]["status"], "accepted", "{put}");
    let opened = client.command("captain.open", json!({"captain":0}), uuid::Uuid::new_v4());
    assert_eq!(opened["result"]["status"], "accepted", "{opened}");
    opened["result"]["result"]["threadId"].as_u64().unwrap()
}

/// Any JSON-RPC request against a thread's recorded MCP endpoint.
fn fixture_rpc(directory: &std::path::Path, method: &str, params: Value) -> Value {
    let host: Value =
        serde_json::from_slice(&fs::read(directory.join("fixture-mcp.json")).unwrap()).unwrap();
    fixture_rpc_as(
        &host["url"].as_str().unwrap(),
        host["token"].as_str().unwrap(),
        method,
        params,
    )
}

fn fixture_rpc_as(url: &str, token: &str, method: &str, params: Value) -> Value {
    let url = reqwest::Url::parse(url).unwrap();
    let mut stream = TcpStream::connect((url.host_str().unwrap(), url.port().unwrap())).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string();
    write!(stream, "POST {} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", url.path(), body.len(), body).unwrap();
    let mut response = String::new();
    if let Err(error) = stream.read_to_string(&mut response)
        && (error.kind() != std::io::ErrorKind::ConnectionReset || response.is_empty())
    {
        panic!("{method} response read failed: {error}; received {response:?}");
    }
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

fn tool_names(directory: &std::path::Path) -> Vec<String> {
    fixture_rpc(directory, "tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| {
            assert!(tool["description"].is_string(), "{tool}");
            assert!(tool["inputSchema"]["type"] == "object", "{tool}");
            tool["name"].as_str().unwrap().to_owned()
        })
        .collect()
}

/// A successful tools/call, returning the parsed result payload.
fn fixture_call(directory: &std::path::Path, tool: &str, args: Value) -> Value {
    let response = fixture_rpc(
        directory,
        "tools/call",
        json!({"name":tool,"arguments":args}),
    );
    assert_ne!(response["result"]["isError"], true, "{tool}: {response}");
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// A refused tools/call, returning the refusal text.
fn fixture_refusal(directory: &std::path::Path, tool: &str, args: Value) -> String {
    let response = fixture_rpc(
        directory,
        "tools/call",
        json!({"name":tool,"arguments":args}),
    );
    assert_eq!(response["result"]["isError"], true, "{tool}: {response}");
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn mcp_initialize(directory: &std::path::Path) -> Value {
    let response = fixture_rpc(
        directory,
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"parity-test","version":"0"}}),
    );
    assert_eq!(
        response["result"]["protocolVersion"], "2025-06-18",
        "{response}"
    );
    assert_eq!(
        response["result"]["capabilities"]["experimental"]["shina"]["effectiveCapabilities"],
        "shina-effective-capabilities/v1",
        "{response}"
    );
    assert_eq!(
        response["result"]["serverInfo"]["name"], "Shina",
        "{response}"
    );
    response["result"].clone()
}

/// Waits for the thread's session directory to contain the fake harness's
/// token-scoped endpoint record. Captain homes and crew run outside chats/,
/// so the server's reported directory wins over the default.
fn jsonl(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn directory_of(client: &mut Client, profile: &Profile, thread: u64) -> PathBuf {
    let chats = profile.root.join(format!("Shina/chats/{thread}"));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        client.rpc("workspace.subscribe", json!({}), json!("directories"));
        let mut candidates = vec![chats.clone()];
        if let Some(reported) = client.snapshot["directories"][thread.to_string()].as_str() {
            candidates.push(PathBuf::from(reported));
        }
        if let Some(found) = candidates
            .iter()
            .find(|directory| directory.join("fixture-mcp.json").exists())
        {
            return found.clone();
        }
        if Instant::now() >= deadline {
            let artifacts: Vec<String> = walk(&profile.root)
                .into_iter()
                .filter(|path| {
                    path.ends_with("fixture-mcp.json")
                        || path.ends_with("codex-requests.jsonl")
                        || path.ends_with("acp-log.jsonl")
                        || path.ends_with("requests.jsonl")
                })
                .collect();
            let mut logs = String::new();
            for artifact in &artifacts {
                logs.push_str(&format!(
                    "\n== {artifact} ==\n{}",
                    fs::read_to_string(artifact).unwrap_or_default()
                ));
            }
            let view = client.snapshot["threads"]
                .as_array()
                .and_then(|threads| threads.iter().find(|t| t["id"] == thread).cloned());
            panic!(
                "no MCP fixture for thread {thread}; candidates {candidates:?}; \
                 directories {}; artifacts {artifacts:?}{logs}\nthread view {view:?}",
                client.snapshot["directories"]
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn walk(root: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        if let Ok(entries) = fs::read_dir(&directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    found.push(path.display().to_string());
                }
            }
        }
    }
    found
}

fn capability_record(directory: &std::path::Path) -> Value {
    fixture_call(directory, "orchestrator_capabilities", json!({}))
}

fn endpoint(directory: &std::path::Path) -> (String, String) {
    let host: Value =
        serde_json::from_slice(&fs::read(directory.join("fixture-mcp.json")).unwrap()).unwrap();
    (
        host["url"].as_str().unwrap().to_owned(),
        host["token"].as_str().unwrap().to_owned(),
    )
}

fn tool_names_as(url: &str, token: &str) -> Vec<String> {
    fixture_rpc_as(url, token, "tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn capability_contract_binds_advertisement_and_dispatch_to_the_calling_role() {
    let mut profile = Profile::new();
    fs::write(profile.root.join("use-native-approval-fixture"), "").unwrap();
    fs::write(profile.root.join("use-fake-codex-fixture"), "").unwrap();
    fs::write(
        profile.root.join("Shina/harnesses.json"),
        json!([
            {"id":"fixture-acp","name":"Fixture ACP",
            "command":if cfg!(windows) { "python" } else { "python3" },
            "args":["-u",profile.root.join("fake_acp.py"),"--log",
                profile.root.join("acp-log.jsonl"),"--resume","--no-history"]},
            {"id":"fixture-acp-nomcp","name":"Fixture ACP without MCP",
            "command":if cfg!(windows) { "python" } else { "python3" },
            "args":["-u",profile.root.join("fake_acp.py"),"--log",
                profile.root.join("acp-nomcp-log.jsonl"),"--no-mcp"]}
        ])
        .to_string(),
    )
    .unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    hello(&mut client, "parity-hello");

    // The primary read-only Captain home advertises the coordinator contract.
    let home = captain_open(&mut client, "Parity");
    let send = client.command(
        "thread.send",
        json!({"threadId":home,"text":"Initialize the coordinator fixture",
            "attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(send["result"]["status"], "accepted", "{send}");
    let home_directory = directory_of(&mut client, &profile, home);
    let initialize = mcp_initialize(&home_directory);
    assert!(
        initialize["instructions"]
            .as_str()
            .unwrap()
            .contains("delegate_task"),
        "coordinator instructions must describe delegation: {initialize}"
    );
    let tools = tool_names(&home_directory);
    for listed in [
        "delegate_task",
        "captain_crew_create",
        "captain_document_write",
        "create_routine",
        "remember",
    ] {
        assert!(
            tools.iter().any(|tool| tool == listed),
            "missing {listed}: {tools:?}"
        );
    }
    // ask_memory stays unlisted while Honcho learned memory is disabled.
    assert!(!tools.iter().any(|tool| tool == "ask_memory"), "{tools:?}");
    let record = capability_record(&home_directory);
    assert_eq!(record["contract"], "shina-effective-capabilities/v1");
    assert_eq!(record["caller"]["role"], "captain_home", "{record}");
    assert_eq!(record["caller"]["captain"], 0);
    assert_eq!(record["caller"]["access"], "read-only", "{record}");
    assert_eq!(record["tools"]["delegate_task"]["listed"], true, "{record}");
    assert_eq!(
        record["effective"]["execution"]["remote_independent_roots"]["available"], false,
        "independent crew roots on a linked machine are not implemented: {record}"
    );
    assert_eq!(
        record["effective"]["capacity"]["scheduler"]["implemented"], true,
        "capabilities report the integrated durable scheduler: {record}"
    );
    // A shared context document at a realistic size reaches workers verbatim.
    let large = format!("{}SHARED-CONTEXT-TAIL", "large note body; ".repeat(2_000));
    let write = client.command(
        "captain.document.write",
        json!({"captain":0,"name":"NOTES.md","text":large,"expectedRevision":0}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(write["result"]["status"], "accepted", "{write}");

    // A delegated leaf: worker instructions, a leaf tool list, and dispatch
    // that refuses what the list already omitted. A retry under the same
    // client_request_id replays the stored response instead of double-launching.
    let brief = json!({"prompt":"Parity leaf","title":"leaf","client_request_id":"parity-leaf"});
    let first = fixture_call(&home_directory, "delegate_task", brief.clone());
    let leaf = first["thread_id"].as_u64().unwrap();
    let retry = fixture_call(&home_directory, "delegate_task", brief);
    assert_eq!(retry["thread_id"], leaf);
    assert_eq!(retry["replayed"], true, "{retry}");
    let leaf_directory = directory_of(&mut client, &profile, leaf);
    let prompt_deadline = Instant::now() + Duration::from_secs(20);
    let leaf_prompt = loop {
        if let Ok(prompts) = fs::read_to_string(leaf_directory.join("fixture-prompts.jsonl")) {
            if let Some(prompt) = prompts
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|prompt| {
                    prompt
                        .as_str()
                        .is_some_and(|text| text.contains("Parity leaf"))
                })
            {
                break prompt.as_str().unwrap().to_owned();
            }
        }
        assert!(
            Instant::now() < prompt_deadline,
            "worker prompt with the shared-document manifest never appeared"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(
        !leaf_prompt.contains("SHARED-CONTEXT-TAIL"),
        "a delegated task must not copy the shared document whole: {leaf_prompt}"
    );
    let leaf_initialize = mcp_initialize(&leaf_directory);
    assert!(
        leaf_initialize["instructions"]
            .as_str()
            .unwrap()
            .contains("cannot delegate further"),
        "worker instructions must match the leaf contract: {leaf_initialize}"
    );
    let leaf_tools = tool_names(&leaf_directory);
    for omitted in [
        "delegate_task",
        "captain_crew_create",
        "remember",
        "ask_memory",
    ] {
        assert!(
            !leaf_tools.iter().any(|tool| tool == omitted),
            "{omitted} listed for a leaf"
        );
    }
    for listed in [
        "task_status",
        "list_threads",
        "propose_memory",
        "captain_document_read",
    ] {
        assert!(
            leaf_tools.iter().any(|tool| tool == listed),
            "missing {listed}"
        );
    }
    assert!(
        fixture_refusal(&leaf_directory, "delegate_task", json!({"prompt":"nope"}))
            .contains("cannot delegate further")
    );
    // Large shared context reads through the leaf's authorized document path.
    let notes = fixture_call(
        &leaf_directory,
        "captain_document_read",
        json!({"captain":0,"name":"NOTES.md"}),
    );
    assert_eq!(notes["text"], large, "{notes}");
    // The private captain profile is not a shared document.
    assert!(
        fixture_refusal(
            &leaf_directory,
            "captain_document_read",
            json!({"captain":0,"name":"AGENTS.md"})
        )
        .contains("private")
    );

    // The same worker contract binds through each harness adapter's real MCP
    // wiring: native Claude --mcp-config, Codex config overrides, ACP session
    // mcpServers.
    for harness in ["pi", "claude", "codex", "fixture-acp"] {
        let prompt = if harness == "claude" {
            "Parity claude leaf"
        } else {
            "shina-dispatch-fixture"
        };
        let launched = fixture_call(
            &home_directory,
            "delegate_task",
            json!({"prompt":prompt,"title":harness,
                "client_request_id":format!("parity-{harness}"),
                "target":{"harness":harness}}),
        );
        let worker = launched["thread_id"].as_u64().unwrap();
        let worker_directory = directory_of(&mut client, &profile, worker);
        let worker_initialize = mcp_initialize(&worker_directory);
        assert!(
            worker_initialize["instructions"]
                .as_str()
                .unwrap()
                .contains("cannot delegate further"),
            "{harness} worker instructions: {worker_initialize}"
        );
        let worker_tools = tool_names(&worker_directory);
        assert!(
            !worker_tools.iter().any(|tool| tool == "delegate_task"),
            "{harness} leaf must not list delegate_task: {worker_tools:?}"
        );
        client.until(|state| {
            state["threads"].as_array().is_some_and(|threads| {
                threads.iter().any(|thread| {
                    thread["id"] == worker
                        && matches!(
                            thread["status"].as_str(),
                            Some("Completed" | "Failed" | "Interrupted")
                        )
                })
            })
        });
        let worker_thread = client.snapshot["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|thread| thread["id"] == worker)
            .unwrap()
            .clone();
        assert_eq!(
            worker_thread["status"], "Completed",
            "{harness} workflow did not complete: {worker_thread}"
        );
        let (url, token) = endpoint(&worker_directory);
        let auth = format!("Bearer {token}");
        match harness {
            "claude" => {
                let records = jsonl(&worker_directory.join("claude-requests.jsonl"));
                let status = records
                    .iter()
                    .find(|record| record["mcp_status"].is_object())
                    .expect("Claude mcp_status resolution");
                assert_eq!(status["mcp_status"]["config"]["url"], url);
                assert_eq!(
                    status["mcp_status"]["config"]["headers"]["Authorization"],
                    auth
                );
            }
            "pi" | "codex" | "fixture-acp" => {
                let path = match harness {
                    "pi" => worker_directory.join("pi-mcp-dispatch.jsonl"),
                    "codex" => worker_directory.join("codex-mcp-dispatch.jsonl"),
                    _ => worker_directory.join("acp-mcp-dispatch.jsonl"),
                };
                let records = jsonl(&path);
                for name in ["orchestrator_capabilities", "list_threads"] {
                    let call = records
                        .iter()
                        .find(|record| record["name"] == name)
                        .unwrap_or_else(|| panic!("{harness} did not dispatch {name}"));
                    assert_eq!(call["url"], url, "{harness} {name} endpoint");
                    assert_eq!(call["authorization"], auth, "{harness} {name} bearer");
                    assert!(
                        call["result"]
                            .as_str()
                            .unwrap()
                            .contains(if name == "list_threads" {
                                "\"threads\""
                            } else {
                                "\"caller\""
                            }),
                        "{harness} {name}: {call}"
                    );
                }
            }
            _ => unreachable!(),
        }
    }

    // Crew threads are coordinators of their own work but not the Captain's:
    // they may delegate, report and read shared documents, and nothing else
    // captain-owned is advertised or accepted.
    let created = client.command(
        "captain.crew.create",
        json!({"captain":0,"input":{"text":"Parity crew","attachments":[]}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(created["result"]["status"], "accepted", "{created}");
    let crew = created["result"]["result"]["threadId"].as_u64().unwrap();
    let crew_directory = directory_of(&mut client, &profile, crew);
    let crew_prompt_deadline = Instant::now() + Duration::from_secs(20);
    let crew_prompt = loop {
        if let Ok(prompts) = fs::read_to_string(crew_directory.join("fixture-prompts.jsonl")) {
            if let Some(prompt) = prompts
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|prompt| {
                    prompt
                        .as_str()
                        .is_some_and(|text| text.contains("Parity crew"))
                })
            {
                break prompt.as_str().unwrap().to_owned();
            }
        }
        assert!(
            Instant::now() < crew_prompt_deadline,
            "crew prompt with the shared-document manifest never appeared"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    let notes_index = format!("NOTES.md (revision 1, {} bytes", large.len());
    assert!(
        crew_prompt.contains(&notes_index)
            && crew_prompt.contains("not in this context; readable with captain_document_read"),
        "the injected crew context must point to the full document instead of overfilling the prompt: {crew_prompt}"
    );
    assert!(
        !crew_prompt.contains("SHARED-CONTEXT-TAIL"),
        "a document beyond the injection limit must not be copied whole: {crew_prompt}"
    );
    let crew_tools = tool_names(&crew_directory);
    assert!(
        crew_tools.iter().any(|tool| tool == "delegate_task"),
        "{crew_tools:?}"
    );
    assert!(
        crew_tools.iter().any(|tool| tool == "captain_crew_report"),
        "{crew_tools:?}"
    );
    for omitted in ["captain_crew_create", "captain_document_write", "remember"] {
        assert!(
            !crew_tools.iter().any(|tool| tool == omitted),
            "{omitted} listed for crew"
        );
    }
    let crew_record = capability_record(&crew_directory);
    assert_eq!(crew_record["caller"]["role"], "crew", "{crew_record}");
    let crew_refusal = fixture_refusal(
        &crew_directory,
        "captain_crew_create",
        json!({"input":{"text":"x","attachments":[]}}),
    );
    assert!(crew_refusal.contains("Captain"), "{crew_refusal}");

    // Stale discovery: the coordinator's earlier tools/list advertised
    // delegate_task, but dispatch revalidates after captain.reset demotes the
    // home. Advertisement is discovery, not authority.
    client.until(|state| {
        state["threads"].as_array().is_some_and(|threads| {
            threads
                .iter()
                .any(|thread| thread["id"] == home && thread["status"] == "Completed")
        })
    });
    // The old endpoint keeps its own bearer token even after the reset writes
    // a new endpoint record into the same scratch directory.
    let (old_url, old_token) = endpoint(&home_directory);
    let reset = client.command("captain.reset", json!({"captain":0}), uuid::Uuid::new_v4());
    assert_eq!(reset["result"]["status"], "accepted", "{reset}");
    let new_home = reset["result"]["result"]["threadId"].as_u64().unwrap();
    assert_ne!(new_home, home);
    let refusal = fixture_rpc_as(
        &old_url,
        &old_token,
        "tools/call",
        json!({"name":"delegate_task","arguments":{"prompt":"stale","client_request_id":"stale"}}),
    );
    assert_eq!(refusal["result"]["isError"], true, "{refusal}");
    assert!(
        refusal["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("primary home"),
        "{refusal}"
    );
    let earlier = fixture_rpc_as(
        &old_url,
        &old_token,
        "tools/call",
        json!({"name":"orchestrator_capabilities","arguments":{}}),
    );
    let earlier: Value =
        serde_json::from_str(earlier["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(earlier["caller"]["role"], "earlier_home", "{earlier}");
    assert!(
        !tool_names_as(&old_url, &old_token)
            .iter()
            .any(|tool| tool == "delegate_task")
    );
    let continued = client.command(
        "thread.send",
        json!({"threadId":home,"text":"earlier-home-role-update",
            "attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(continued["result"]["status"], "accepted", "{continued}");
    let instruction_deadline = Instant::now() + Duration::from_secs(20);
    let instructions = loop {
        if let Ok(prompts) = fs::read_to_string(home_directory.join("fixture-system-prompt.jsonl"))
        {
            if let Some(prompt) = prompts
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|prompt| {
                    prompt.as_str().is_some_and(|text| {
                        text.contains("This conversation is an earlier Captain home")
                    })
                })
            {
                break prompt.as_str().unwrap().to_owned();
            }
        }
        let view = client.snapshot["threads"]
            .as_array()
            .and_then(|threads| threads.iter().find(|thread| thread["id"] == home))
            .cloned();
        assert!(
            Instant::now() < instruction_deadline,
            "the earlier home session never received role-matched Pi instructions; thread {view:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(
        !instructions.contains("You may delegate independent work"),
        "{instructions}"
    );
    // The new home's session rewrites the endpoint record with its own token.
    let new_deadline = Instant::now() + Duration::from_secs(60);
    let (new_url, new_token) = loop {
        if let Ok(bytes) = fs::read(home_directory.join("fixture-mcp.json"))
            && let Ok(host) = serde_json::from_slice::<Value>(&bytes)
            && host["token"].as_str() != Some(old_token.as_str())
        {
            break (
                host["url"].as_str().unwrap().to_owned(),
                host["token"].as_str().unwrap().to_owned(),
            );
        }
        assert!(
            Instant::now() < new_deadline,
            "new home endpoint never appeared"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        tool_names_as(&new_url, &new_token)
            .iter()
            .any(|tool| tool == "delegate_task")
    );

    // Archived captains lose all coordination until restored. Delivered worker
    // results keep waking the new home, so keep stopping it until every thread
    // settles; its endpoint record is already captured and keeps answering as
    // the archived thread.
    let settle_deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let busy: Vec<Value> = client.snapshot["threads"]
            .as_array()
            .map(|threads| {
                threads
                    .iter()
                    .filter(|thread| {
                        !matches!(
                            thread["status"].as_str().unwrap_or_default(),
                            "Completed" | "Failed" | "Interrupted" | "Idle" | "Ready"
                        ) || thread["request"].is_object()
                    })
                    .map(|thread| {
                        json!({"id":thread["id"],"status":thread["status"],
                            "request":thread["request"]})
                    })
                    .collect()
            })
            .unwrap_or_default();
        if busy.is_empty() {
            break;
        }
        if busy.iter().any(|thread| thread["id"] == new_home) {
            client.command(
                "thread.stop",
                json!({"threadId":new_home}),
                uuid::Uuid::new_v4(),
            );
        }
        client.rpc("workspace.subscribe", json!({}), json!("settle"));
        assert!(
            Instant::now() < settle_deadline,
            "threads never settled: {busy:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let preview = client.command(
        "captain.lifecycle.preview",
        json!({"captain":0,"action":"archive"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(preview["result"]["status"], "accepted", "{preview}");
    let preview = preview["result"]["result"].clone();
    let archived = client.command(
        "captain.archive",
        json!({"captain":0,"expectedVersion":preview["captainVersion"],
            "crewSeen":preview["crewSeen"]}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(archived["result"]["status"], "accepted", "{archived}");
    let archived_tools = tool_names_as(&new_url, &new_token);
    for omitted in [
        "delegate_task",
        "captain_crew_create",
        "captain_document_write",
    ] {
        assert!(
            !archived_tools.iter().any(|tool| tool == omitted),
            "{omitted} while archived"
        );
    }
    let archived_refusal = fixture_rpc_as(
        &new_url,
        &new_token,
        "tools/call",
        json!({"name":"delegate_task","arguments":{"prompt":"x"}}),
    );
    assert_eq!(
        archived_refusal["result"]["isError"], true,
        "{archived_refusal}"
    );
    let archived_record = fixture_rpc_as(
        &new_url,
        &new_token,
        "tools/call",
        json!({"name":"orchestrator_capabilities","arguments":{}}),
    );
    let archived_record: Value = serde_json::from_str(
        archived_record["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        archived_record["caller"]["owner_status"], "archived",
        "{archived_record}"
    );

    // Revoked scope: a bearer token that is not the thread's gets no tools.
    let revoked = fixture_rpc_as(&new_url, "revoked-token", "tools/list", json!({}));
    assert!(
        revoked.is_null() || revoked["error"].is_object() || revoked.get("result").is_none(),
        "{revoked}"
    );

    // Remote independent roots are reported unsupported, not faked: a delegate
    // targeting a linked machine is denied until the Captain's scope names a
    // remote project there.
    let remote = fixture_rpc_as(
        &new_url,
        &new_token,
        "tools/call",
        json!({"name":"delegate_task","arguments":
            {"prompt":"remote root","target":{"environmentId":"unlinked-peer"}}}),
    );
    assert_eq!(remote["result"]["isError"], true, "{remote}");

    // Startup refusal is surfaced, not hidden: an ACP agent that supports
    // neither MCP transport cannot host Shina tools, and the thread fails
    // with the transport reason.
    let configure = client.command(
        "thread.configure",
        json!({"threadId":1,"harness":"fixture-acp-nomcp"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(configure["result"]["status"], "accepted", "{configure}");
    let refused = client.command(
        "thread.send",
        json!({"threadId":1,"text":"start","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(refused["result"]["status"], "accepted", "{refused}");
    client.until(|state| {
        state["threads"].as_array().is_some_and(|threads| {
            threads
                .iter()
                .any(|thread| thread["id"] == 1 && thread["status"] == "Failed")
        })
    });
    let view = client.snapshot["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == 1)
        .unwrap()
        .clone();
    assert!(
        view.to_string().contains("MCP"),
        "startup refusal must name the missing MCP transport: {view}"
    );
}

#[test]
fn remote_worker_capability_contract_over_a_real_pairing() {
    let mut home = Profile::new();
    let mut target = Profile::new();
    fs::write(
        target.root.join("Shina/harnesses.json"),
        json!([
            {"id":"fixture-acp-nomcp","name":"Fixture ACP without MCP",
            "command":if cfg!(windows) { "python" } else { "python3" },
            "args":["-u",target.root.join("fake_acp.py"),"--log",
                target.root.join("acp-nomcp-log.jsonl"),"--no-mcp"]}
        ])
        .to_string(),
    )
    .unwrap();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let mut owner = home.client(uuid::Uuid::new_v4());
    let mut executor = target.client(uuid::Uuid::new_v4());
    hello(&mut owner, "remote-parity-owner");
    hello(&mut executor, "remote-parity-executor");
    let workspace = target.root.join("workspace");
    let project = executor.command(
        "project.put",
        json!({"path":workspace,"createFolder":true}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(project["result"]["status"], "accepted", "{project}");
    let project = project["result"]["result"]["projectId"].clone();
    let discovery = target.discovery();
    let paired = target.cli(&["pair", "--peer"]);
    assert!(paired.status.success());
    let output = String::from_utf8(paired.stdout).unwrap();
    let code = output
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap();
    let linked = home.cli(&[
        "peer",
        "link",
        &format!("127.0.0.1:{}", discovery["port"]),
        code,
    ]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let home_thread = captain_open(&mut owner, "Remote parity");
    let scope = owner.command(
        "captain.scope",
        json!({"captain":0,"projects":[],
            "remoteProjects":[{"server":discovery["serverId"],"projectId":project}]}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(scope["result"]["status"], "accepted", "{scope}");
    let initialized = owner.command(
        "thread.send",
        json!({"threadId":home_thread,"text":"Initialize remote coordinator",
            "attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(initialized["result"]["status"], "accepted", "{initialized}");
    let coordinator_directory = directory_of(&mut owner, &home, home_thread);

    // The record reports the negotiated peer feature set truthfully.
    let record = capability_record(&coordinator_directory);
    assert_eq!(record["caller"]["role"], "captain_home", "{record}");
    let peers = record["effective"]["execution"]["peers"]
        .as_array()
        .unwrap();
    let peer = peers
        .iter()
        .find(|peer| peer["serverId"] == discovery["serverId"])
        .unwrap_or_else(|| panic!("peer missing from record: {record}"));
    assert_eq!(peer["reachable"], true, "{peer}");
    assert!(
        peer["contracts"].as_array().is_some_and(|contracts| {
            contracts.iter().any(|contract| contract == "peer-tasks-v2")
        }),
        "{peer}"
    );
    assert_eq!(peer["delegation"]["available"], true, "{peer}");
    assert_eq!(peer["documentReads"]["available"], true, "{peer}");

    fixture_call(
        &coordinator_directory,
        "delegate_task",
        json!({"prompt":"remote-parity-leaf","client_request_id":"remote-parity",
            "target":{"environmentId":discovery["serverId"],"projectId":project,
                "model":"fixture"}}),
    );
    // The remote leaf binds the remote-worker contract on its own machine.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !workspace.join("fixture-mcp.json").exists() {
        assert!(
            Instant::now() < deadline,
            "remote fixture endpoint never appeared"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let remote_initialize = mcp_initialize(&workspace);
    let instructions = remote_initialize["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("linked machine"),
        "{remote_initialize}"
    );
    assert!(
        instructions.contains("captain_document_read"),
        "{remote_initialize}"
    );
    let remote_tools = tool_names(&workspace);
    for listed in [
        "task_status",
        "list_threads",
        "captain_document_read",
        "propose_memory",
    ] {
        assert!(
            remote_tools.iter().any(|tool| tool == listed),
            "missing {listed}"
        );
    }
    for omitted in [
        "delegate_task",
        "captain_crew_create",
        "captain_document_write",
        "remember",
        "create_routine",
    ] {
        assert!(
            !remote_tools.iter().any(|tool| tool == omitted),
            "{omitted} listed remotely"
        );
    }
    let remote_record = capability_record(&workspace);
    assert_eq!(
        remote_record["caller"]["role"], "remote_worker",
        "{remote_record}"
    );
    assert!(
        fixture_refusal(&workspace, "delegate_task", json!({"prompt":"x"}))
            .contains("remote worker")
    );
    executor.rpc(
        "workspace.subscribe",
        json!({}),
        json!("target-before-refusal"),
    );
    let target_threads = executor.snapshot["threads"].as_array().map_or(0, Vec::len);
    owner.rpc(
        "workspace.subscribe",
        json!({}),
        json!("home-before-refusal"),
    );
    let home_threads = owner.snapshot["threads"].as_array().map_or(0, Vec::len);
    let error = fixture_refusal(
        &coordinator_directory,
        "delegate_task",
        json!({"prompt":"target must preflight ACP","client_request_id":"remote-no-mcp",
            "target":{"environmentId":discovery["serverId"],"projectId":project,
                "model":"fixture","harness":"fixture-acp-nomcp"}}),
    );
    assert!(
        error.contains("cannot run fixture-acp-nomcp") && error.contains("neither HTTP nor stdio"),
        "{error}"
    );
    owner.rpc(
        "workspace.subscribe",
        json!({}),
        json!("home-after-refusal"),
    );
    assert_eq!(
        owner.snapshot["threads"].as_array().map_or(0, Vec::len),
        home_threads,
        "the origin must refuse a peer-advertised non-executable harness before allocating"
    );
    executor.rpc(
        "workspace.subscribe",
        json!({}),
        json!("target-after-refusal"),
    );
    assert_eq!(
        executor.snapshot["threads"].as_array().map_or(0, Vec::len),
        target_threads,
        "the target must refuse an MCP-less ACP harness before allocating a thread"
    );
    target.stop();
}

#[test]
fn acp_without_mcp_is_refused_before_thread_allocation() {
    let mut profile = Profile::new();
    fs::write(
        profile.root.join("Shina/harnesses.json"),
        json!([
            {"id":"fixture-acp","name":"Fixture ACP",
            "command":if cfg!(windows) { "python" } else { "python3" },
            "args":["-u",profile.root.join("fake_acp.py"),"--log",
                profile.root.join("acp-log.jsonl"),"--resume","--no-history"]},
            {"id":"fixture-acp-nomcp","name":"Fixture ACP without MCP",
            "command":if cfg!(windows) { "python" } else { "python3" },
            "args":["-u",profile.root.join("fake_acp.py"),"--log",
                profile.root.join("acp-nomcp-log.jsonl"),"--no-mcp"]},
            {"id":"fixture-acp-exits","name":"Fixture ACP startup failure",
            "command":if cfg!(windows) { "python" } else { "python3" },
            "args":["-c","import sys; sys.exit(7)"]}
        ])
        .to_string(),
    )
    .unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    hello(&mut client, "acp-preflight");
    let home = captain_open(&mut client, "Preflight");
    let send = client.command(
        "thread.send",
        json!({"threadId":home,"text":"Initialize preflight","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(send["result"]["status"], "accepted", "{send}");
    let directory = directory_of(&mut client, &profile, home);
    client.rpc("workspace.subscribe", json!({}), json!("before"));
    let before = client.snapshot["threads"].as_array().unwrap().len();
    let error = fixture_refusal(
        &directory,
        "delegate_task",
        json!({"prompt":"must not allocate","client_request_id":"no-mcp",
            "target":{"harness":"fixture-acp-nomcp"}}),
    );
    assert!(error.contains("MCP"), "{error}");
    assert!(
        error.contains("no thread was allocated") || error.contains("neither HTTP nor stdio"),
        "{error}"
    );
    client.rpc("workspace.subscribe", json!({}), json!("after"));
    assert_eq!(
        client.snapshot["threads"].as_array().unwrap().len(),
        before,
        "an unsupported ACP harness must be refused before a thread is allocated"
    );
    let error = fixture_refusal(
        &directory,
        "delegate_task",
        json!({"prompt":"startup must not allocate","client_request_id":"exit-before-init",
            "target":{"harness":"fixture-acp-exits"}}),
    );
    assert!(
        error.contains("exited before answering initialize"),
        "{error}"
    );
    client.rpc("workspace.subscribe", json!({}), json!("after-exit"));
    assert_eq!(
        client.snapshot["threads"].as_array().unwrap().len(),
        before,
        "an ACP startup failure must be refused before a thread is allocated"
    );
    let record = capability_record(&directory);
    let harnesses = record["effective"]["execution"]["local"]["harnesses"]
        .as_array()
        .unwrap();
    let unsupported = harnesses
        .iter()
        .find(|harness| harness["id"] == "fixture-acp-nomcp")
        .unwrap();
    assert_eq!(unsupported["executable"], false, "{unsupported}");
    assert_eq!(unsupported["discovery"], "probed", "{unsupported}");
    assert_eq!(unsupported["mcp"]["available"], false, "{unsupported}");
    let supported = harnesses
        .iter()
        .find(|harness| harness["id"] == "fixture-acp")
        .unwrap();
    assert_eq!(supported["executable"], true, "{supported}");
    assert_eq!(supported["discovery"], "probed", "{supported}");
    assert_eq!(supported["mcp"]["available"], true, "{supported}");
    let failed = harnesses
        .iter()
        .find(|harness| harness["id"] == "fixture-acp-exits")
        .unwrap();
    assert_eq!(failed["executable"], false, "{failed}");
    assert_eq!(failed["discovery"], "failed", "{failed}");
    assert!(
        failed["reason"]
            .as_str()
            .unwrap()
            .contains("exited before answering initialize"),
        "{failed}"
    );
}

#[test]
fn read_only_claude_crew_dispatches_through_can_use_tool_and_hook_binding() {
    let mut profile = Profile::new();
    fs::write(profile.root.join("use-native-approval-fixture"), "").unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    hello(&mut client, "claude-crew-dispatch");
    let home = captain_open(&mut client, "Claude binding");
    let send = client.command(
        "thread.send",
        json!({"threadId":home,"text":"Initialize Claude binding",
            "attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(send["result"]["status"], "accepted", "{send}");
    let _home_directory = directory_of(&mut client, &profile, home);
    let created = client.command(
        "captain.crew.create",
        json!({"captain":0,"access":"read-only","target":{"harness":"claude"},
            "input":{"text":"shina-dispatch-fixture","attachments":[]}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(created["result"]["status"], "accepted", "{created}");
    let worker = created["result"]["result"]["threadId"].as_u64().unwrap();
    let directory = directory_of(&mut client, &profile, worker);
    client.until(|state| {
        state["threads"].as_array().is_some_and(|threads| {
            threads.iter().any(|thread| {
                thread["id"] == worker
                    && matches!(
                        thread["status"].as_str(),
                        Some("Completed" | "Failed" | "Interrupted")
                    )
            })
        })
    });
    assert_eq!(
        client.snapshot["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|thread| thread["id"] == worker)
            .unwrap()["status"],
        "Completed"
    );
    let (url, token) = endpoint(&directory);
    let auth = format!("Bearer {token}");
    let records = jsonl(&directory.join("claude-requests.jsonl"));
    let status = records
        .iter()
        .find(|record| record["mcp_status"].is_object())
        .expect("Claude mcp_status resolution");
    assert_eq!(status["mcp_status"]["config"]["url"], url);
    assert_eq!(
        status["mcp_status"]["config"]["headers"]["Authorization"],
        auth
    );
    let decision = |name: &str| {
        let response = records
            .iter()
            .find(|record| record["response"]["request_id"] == format!("callback-{name}"))
            .unwrap_or_else(|| panic!("no Claude permission response for {name}"));
        response["response"]["response"].clone()
    };
    assert_eq!(
        decision("mcp__shina__orchestrator_capabilities")["behavior"],
        "allow"
    );
    assert_eq!(
        decision("mcp__shina__list_threads")["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    assert_eq!(decision("mcp__shina__exec_shell")["behavior"], "deny");
    assert_eq!(
        decision("mcp__shina_evil__list_threads")["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    for (name, expected) in [
        ("mcp__shina__orchestrator_capabilities", "\"caller\""),
        ("mcp__shina__list_threads", "\"threads\""),
    ] {
        let dispatch = records
            .iter()
            .find(|record| record["dispatch"] == name)
            .unwrap_or_else(|| panic!("Claude did not dispatch {name}"));
        assert!(dispatch["result"].as_str().unwrap().contains(expected));
    }
    for name in ["mcp__shina__exec_shell", "mcp__shina_evil__list_threads"] {
        let dispatch = records
            .iter()
            .find(|record| record["dispatch"] == name)
            .expect("denied dispatch record");
        assert_eq!(dispatch["denied"], true, "{name}");
    }
}
