//! Real serve processes, real WebSockets and fake harness subprocesses. No provider calls.
#[allow(dead_code)]
#[path = "../src/client_history.rs"]
mod client_history;
#[allow(dead_code)]
#[path = "../src/history.rs"]
mod history;
#[path = "../src/history_work.rs"]
mod history_work;
#[allow(dead_code)]
#[path = "../src/protocol.rs"]
mod protocol;
#[allow(dead_code)]
#[path = "../src/thread_find.rs"]
mod thread_find;
#[allow(dead_code)]
#[path = "../src/thread_search.rs"]
mod thread_search;

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio_tungstenite::tungstenite::{self, Message, WebSocket, client::IntoClientRequest};

struct Profile {
    root: PathBuf,
    children: Vec<Child>,
}

fn reset_test_thread_identities(index: &mut Value, ids: impl IntoIterator<Item = usize>) {
    let identities = ids
        .into_iter()
        .map(|id| (id.to_string(), json!(uuid::Uuid::new_v4().to_string())))
        .collect::<serde_json::Map<_, _>>();
    index["identity"]["threadIds"] = Value::Object(identities);
}

#[test]
fn question_dismissal_is_negotiated_idempotent_and_starts_no_provider_turn() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.command(
        "thread.send",
        json!({"threadId":1,"text":"dismiss-question-fixture",
        "attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    client.until(|state| {
        state["threads"][0]["request"]["id"] == "dismiss-question"
            && state["threads"][0]["status"] == "Completed"
    });
    let request = client.snapshot["threads"][0]["request"].clone();
    let args = json!({"threadId":1,"requestId":request["id"],
        "generation":request["runtimeGeneration"]});
    for minor in [18, protocol::MINOR] {
        let hello = client.rpc(
            "hello",
            json!({"protocolMajor":1,
            "protocolMinor":minor,"projectionSchema":1}),
            json!(format!("dismiss-{minor}")),
        );
        assert_eq!(
            hello["result"]["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cap| cap == "question-dismiss-v1"),
            minor >= 19
        );
        let id = uuid::Uuid::new_v4();
        let result = client.command("thread.user-input.dismiss", args.clone(), id);
        if minor < 19 {
            assert_eq!(
                result["error"]["message"],
                "Update required. Update Shina on the client and server to dismiss questions."
            );
            assert_eq!(
                client.snapshot["threads"][0]["request"]["id"],
                "dismiss-question"
            );
        } else {
            assert_eq!(result["result"]["status"], "accepted", "{result}");
            client.until_sequence(result["result"]["seq"].as_u64().unwrap(), |state| {
                state["threads"][0]["request"].is_null()
            });
            assert_eq!(
                client.command("thread.user-input.dismiss", args.clone(), id)["result"],
                result["result"],
                "receipt replay"
            );
        }
    }
    let repeated = client.command("thread.user-input.dismiss", args, uuid::Uuid::new_v4());
    assert!(
        repeated.to_string().contains("already been answered"),
        "{repeated}"
    );
    assert_eq!(
        fs::read_to_string(profile.root.join("Shina/chats/1/fixture-prompts.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(
        !profile
            .root
            .join("Shina/chats/1/fixture-question-replies.jsonl")
            .exists()
    );
    assert_eq!(client.snapshot["threads"][0]["status"], "Completed");
    profile.stop();
}

#[test]
fn question_files_use_server_paths_and_document_text_in_the_native_answer() {
    use sha2::Digest;
    let mut profile = Profile::new();
    fs::write(profile.root.join("use-native-approval-fixture"), "").unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("file-answer-hello"),
    );
    assert_eq!(
        client.command(
            "thread.configure",
            json!({"threadId":1,"harness":"claude"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    assert_eq!(
        client.command(
            "thread.send",
            json!({"threadId":1,"text":"question-file-fixture","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    client.until(|state| state["threads"][0]["request"]["method"] == "input");
    let request = client.snapshot["threads"][0]["request"].clone();
    let bytes = br##"{"cells":[{"cell_type":"markdown","source":["# File answer"]}]}"##;
    let base = format!("http://127.0.0.1:{}", profile.discovery()["port"]);
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let started: Value = http
        .post(format!("{base}/v1/uploads"))
        .bearer_auth(&client.token)
        .json(
            &json!({"type":"file","name":"answer.ipynb","mimeType":"application/octet-stream",
            "length":bytes.len(),"sha256":format!("{:x}",sha2::Sha256::digest(bytes))}),
        )
        .send()
        .unwrap()
        .json()
        .unwrap();
    let upload = started["uploadId"].as_str().unwrap();
    assert!(
        http.put(format!("{base}/v1/uploads/{upload}?offset=0"))
            .bearer_auth(&client.token)
            .body(bytes.to_vec())
            .send()
            .unwrap()
            .status()
            .is_success()
    );
    let file: Value = http
        .post(format!("{base}/v1/uploads/{upload}/commit"))
        .bearer_auth(&client.token)
        .send()
        .unwrap()
        .json()
        .unwrap();
    let args = json!({"threadId":1,"requestId":request["id"],"generation":request["runtimeGeneration"],
        "requestContract":true,"questionContract":true,"questionAttachments":true,
        "answers":[{"selected":[],"other":"Review the attached notebook."}],
        "attachmentsByQuestionId":{"Attach a file.":[file]},"questionTextById":{"Attach a file.":"Attach a file."},
        "response":{"value":""}});
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":17,"projectionSchema":1}),
        json!("file-answer-old"),
    );
    assert_eq!(
        client.command("request.respond", args.clone(), uuid::Uuid::new_v4())["error"]["message"],
        protocol::QUESTION_ATTACHMENTS_UPDATE_REQUIRED
    );
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("file-answer-new"),
    );
    let response = client.command("request.respond", args, uuid::Uuid::new_v4());
    assert_eq!(response["result"]["status"], "accepted", "{response}");
    client.until(|state| state["threads"][0]["status"] == "Completed");
    let records: Vec<Value> =
        fs::read_to_string(profile.root.join("Shina/chats/1/claude-requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    let native = records
        .iter()
        .find(|r| r["response"]["request_id"] == request["id"])
        .unwrap();
    let answer = native["response"]["response"]["updatedInput"]["answers"]["Attach a file."]
        .as_str()
        .unwrap();
    assert!(
        answer.contains("Review the attached notebook.\n\nAttached file \"answer.ipynb\": "),
        "{answer}"
    );
    let references = answer
        .strip_prefix("Review the attached notebook.\n\nAttached file \"answer.ipynb\": ")
        .unwrap();
    let (attachment, extracted) = references
        .split_once(" (text extracted to: ")
        .unwrap_or_else(|| panic!("Missing extracted document reference: {answer}"));
    // Paths are JSON strings in the provider text. Decode before checking
    // ownership so Windows backslash escaping cannot alter the assertion.
    let attachment: PathBuf = serde_json::from_str(attachment).unwrap();
    let extracted: PathBuf = serde_json::from_str(extracted.strip_suffix(')').unwrap()).unwrap();
    let root = profile.root.canonicalize().unwrap();
    assert!(
        attachment.starts_with(&root) && attachment.is_file(),
        "{answer}"
    );
    assert!(
        extracted.starts_with(&root) && extracted.is_file(),
        "{answer}"
    );
    assert!(
        fs::read_to_string(extracted)
            .unwrap()
            .contains("# File answer"),
        "{answer}"
    );
    profile.stop();
}

/// The whole serve path: the fixture answers `mcp_status` with the endpoint it was
/// launched with, read-only admits exact authentic names on both permission paths, and
/// the fixture's real `tools/call` reaches `State::mcp_call` for this thread. Spoofed
/// and unadvertised names stay denied and never dispatch.
#[test]
fn read_only_claude_dispatches_verified_shina_tools_to_the_calling_thread() {
    let mut profile = Profile::new();
    fs::write(profile.root.join("use-native-approval-fixture"), "").unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("dispatch-hello"),
    );
    assert_eq!(
        client.command(
            "thread.configure",
            json!({"threadId":1,"harness":"claude","access":"read-only"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    assert_eq!(
        client.command(
            "thread.send",
            json!({"threadId":1,"text":"shina-dispatch-fixture","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    client.until(|state| state["threads"][0]["status"] == "Completed");
    let records: Vec<Value> =
        fs::read_to_string(profile.root.join("Shina/chats/1/claude-requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    let decision = |name: &str| {
        let record = records
            .iter()
            .filter(|record| record["type"] == "control_response")
            .find(|record| record["response"]["request_id"] == format!("callback-{name}"))
            .unwrap_or_else(|| panic!("no response for {name}"));
        record["response"]["response"]["behavior"]
            .as_str()
            .or_else(|| {
                record["response"]["response"]["hookSpecificOutput"]["permissionDecision"].as_str()
            })
            .unwrap()
            .to_owned()
    };
    // The session verified its resolved shina server before admitting any name.
    assert!(
        records
            .iter()
            .any(|record| record["request"]["subtype"] == "mcp_status"),
        "no mcp_status verification request"
    );
    assert_eq!(decision("mcp__shina__orchestrator_capabilities"), "allow");
    assert_eq!(decision("mcp__shina__list_threads"), "allow");
    assert_eq!(decision("mcp__shina__exec_shell"), "deny");
    assert_eq!(decision("mcp__shina_evil__list_threads"), "deny");
    let dispatched = |name: &str| {
        records
            .iter()
            .find(|record| record["dispatch"] == name)
            .unwrap_or_else(|| panic!("no dispatch record for {name}"))
    };
    let capabilities = dispatched("mcp__shina__orchestrator_capabilities");
    assert!(
        capabilities["result"]
            .as_str()
            .unwrap()
            .contains("\"caller\""),
        "authenticated dispatch returned caller capabilities: {capabilities}"
    );
    let threads = dispatched("mcp__shina__list_threads");
    assert!(
        threads["result"].as_str().unwrap().contains("\"threads\""),
        "authenticated dispatch returned the thread list: {threads}"
    );
    assert_eq!(dispatched("mcp__shina__exec_shell")["denied"], true);
    assert_eq!(dispatched("mcp__shina_evil__list_threads")["denied"], true);
    profile.stop();
}

#[test]
fn approval_feedback_and_decline_redirect_use_native_decisions_and_refuse_an_older_peer() {
    let mut profile = Profile::new();
    fs::write(profile.root.join("use-native-approval-fixture"), "").unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let negotiate = |client: &mut Client, minor| {
        client.rpc(
            "hello",
            json!({"protocolMajor":1,"protocolMinor":minor,"projectionSchema":1}),
            json!(format!("approval-hello-{minor}")),
        )
    };
    let hello = negotiate(&mut client, protocol::MINOR);
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == protocol::APPROVAL_FEEDBACK)
    );
    assert_eq!(
        client.command(
            "thread.configure",
            json!({"threadId":1,"harness":"claude"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    assert_eq!(
        client.command(
            "thread.send",
            json!({"threadId":1,"text":"approval-fixture","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    client.until(|state| state["threads"][0]["request"]["method"] == "confirm");
    let request = client.snapshot["threads"][0]["request"].clone();
    assert_eq!(request["responseCapability"]["supportsFeedback"], true);
    let args = json!({"threadId":1,"requestId":request["id"],
        "generation":request["runtimeGeneration"],"requestContract":true,
        "response":{"decision":"decline","feedback":"Use a safer command."}});
    let old = negotiate(&mut client, 15);
    assert!(
        !old["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == protocol::APPROVAL_FEEDBACK)
    );
    let refused = client.command("request.respond", args.clone(), uuid::Uuid::new_v4());
    assert_eq!(
        refused["error"]["message"],
        protocol::APPROVAL_FEEDBACK_UPDATE_REQUIRED
    );
    negotiate(&mut client, protocol::MINOR);
    assert_eq!(
        client.command("request.respond", args, uuid::Uuid::new_v4())["result"]["status"],
        "accepted"
    );
    client.until(|state| state["threads"][0]["status"] == "Completed");
    assert!(
        read_full_history(&mut client, 48_000)["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == "Use a safer command.")
    );

    assert_eq!(
        client.command(
            "thread.send",
            json!({"threadId":1,"text":"approval-fixture again","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    client.until(|state| state["threads"][0]["request"]["method"] == "confirm");
    let next = client.snapshot["threads"][0]["request"].clone();
    assert_eq!(
        client.command(
            "request.respond",
            json!({"threadId":1,"requestId":next["id"],
        "generation":next["runtimeGeneration"],"requestContract":true,
        "response":{"decision":"decline"}}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    assert_eq!(client.command("thread.send",
        json!({"threadId":1,"text":"Redirect to another task.","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4())["result"]["status"], "accepted");
    client.until(|state| state["threads"][0]["status"] == "Completed");
    let redirected_history = read_full_history(&mut client, 48_000);
    assert!(
        redirected_history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == "Redirect to another task."),
        "{redirected_history}"
    );
    let records: Vec<Value> =
        fs::read_to_string(profile.root.join("Shina/chats/1/claude-requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    let feedback = records
        .iter()
        .find(|r| r["response"]["request_id"] == request["id"])
        .unwrap();
    assert_eq!(
        feedback["response"]["response"]["message"],
        "Use a safer command."
    );
    assert_ne!(feedback["response"]["response"]["interrupt"], true);
    let decline = records
        .iter()
        .position(|r| r["response"]["request_id"] == next["id"])
        .unwrap();
    let redirect = records
        .iter()
        .position(|r| {
            r["type"] == "user"
                && r["message"]["content"]
                    .to_string()
                    .contains("Redirect to another task.")
        })
        .unwrap();
    assert!(decline < redirect);
    assert_eq!(
        records[decline]["response"]["response"]["message"],
        "User declined tool execution."
    );
    assert!(!records.iter().any(|r| {
        r["type"] == "user"
            && r["message"]["content"]
                .to_string()
                .contains("Use a safer command.")
    }));
    profile.stop();
    profile.launch(&["serve"]);
    let client = profile.client(uuid::Uuid::new_v4());
    assert!(
        client.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == "Use a safer command.")
    );
}

fn read_fixture_thread(workspace: &std::path::Path, args: Value) -> Value {
    let response = call_fixture_tool(workspace, "read_thread", args);
    assert_ne!(response["result"]["isError"], true, "{response}");
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn call_fixture_tool(workspace: &std::path::Path, tool: &str, args: Value) -> Value {
    let host: Value =
        serde_json::from_slice(&fs::read(workspace.join("fixture-mcp.json")).unwrap()).unwrap();
    let url = reqwest::Url::parse(host["url"].as_str().unwrap()).unwrap();
    let mut stream = TcpStream::connect((url.host_str().unwrap(), url.port().unwrap())).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":args}}).to_string();
    write!(stream, "POST {} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", url.path(), host["token"].as_str().unwrap(), body.len(), body).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let response: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    response
}

#[test]
fn local_delegate_mcp_retries_keep_one_worker_after_restart_and_deletion() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let workspace = profile.root.join("Shina/chats/1");
    let connect_parent = |client: &mut Client| {
        let sent = client.command(
            "thread.send",
            json!({"threadId":1,"text":"Initialize the coordinator fixture",
                "attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(sent["result"]["status"], "accepted", "{sent}");
        client.until(|state| state["threads"][0]["status"] == "Completed");
    };
    connect_parent(&mut client);
    let args = json!({"prompt":"One keyed worker","title":"Keyed worker",
        "client_request_id":"local-mcp-retry"});
    let accepted = |args: Value| {
        let response = call_fixture_tool(&workspace, "delegate_task", args);
        assert_ne!(response["result"]["isError"], true, "{response}");
        serde_json::from_str::<Value>(response["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap()
    };
    let first = accepted(args.clone());
    let child = first["thread_id"].as_u64().unwrap();
    let worker_directory = profile.root.join(format!("Shina/chats/{child}"));
    // Discarding the first response's contents does not require another launch.
    let retry = accepted(args.clone());
    assert_eq!(retry["thread_id"], child);
    assert_eq!(retry["task_id"], first["task_id"]);
    assert_eq!(retry["replayed"], true);
    let mut changed = args.clone();
    changed["prompt"] = json!("Different work under the same key");
    let rejected = call_fixture_tool(&workspace, "delegate_task", changed);
    assert_eq!(rejected["result"]["isError"], true, "{rejected}");
    assert!(
        rejected.to_string().contains("different brief or target"),
        "{rejected}"
    );
    client.until(|state| {
        state["index"]["tasks"].as_array().is_some_and(|tasks| {
            tasks.len() == 1 && tasks[0]["state"] == "completed" && tasks[0]["delivered"] == true
        })
    });
    let prompts_before = fs::read(worker_directory.join("fixture-prompts.jsonl")).unwrap();
    assert_eq!(String::from_utf8_lossy(&prompts_before).lines().count(), 1);
    drop(client);
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    connect_parent(&mut client);
    let mut waited = args.clone();
    waited["wait_seconds"] = json!(1);
    let restarted = accepted(waited);
    assert_eq!(restarted["thread_id"], child);
    assert_eq!(restarted["task_id"], first["task_id"]);
    assert_eq!(
        fs::read(worker_directory.join("fixture-prompts.jsonl")).unwrap(),
        prompts_before,
        "a retry must not submit another provider prompt"
    );
    let deleted = client.command(
        "thread.delete",
        json!({"threadId":child}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(deleted["result"]["status"], "accepted", "{deleted}");
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .is_some_and(Vec::is_empty)
    });
    drop(client);
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    connect_parent(&mut client);
    let gone = accepted(args);
    assert_eq!(gone["thread_id"], child);
    assert_eq!(gone["task_id"], first["task_id"]);
    assert_eq!(gone["exists"], false);
    assert_eq!(gone["replayed"], true);
    client.rpc(
        "workspace.subscribe",
        json!({}),
        json!("check-no-recreation"),
    );
    assert!(
        client.snapshot["index"]["tasks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn delegated_peer_writers_use_distinct_git_worktrees_over_the_live_transport() {
    let mut home = Profile::new();
    let mut target = Profile::new();
    let repository = target.root.join("repository");
    fs::create_dir_all(&repository).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-b", "main", "--quiet"]);
    fs::write(repository.join("README.md"), "committed base\n").unwrap();
    git(&["add", "README.md"]);
    git(&["commit", "-m", "fixture base", "--quiet"]);
    let base = git(&["rev-parse", "HEAD"]).trim().to_owned();
    fs::write(repository.join("README.md"), "staged parent changes\n").unwrap();
    git(&["add", "README.md"]);
    fs::write(
        repository.join("parent-only.txt"),
        "untracked parent changes\n",
    )
    .unwrap();
    let original_status = git(&["status", "--porcelain"]);
    let picked = target.root.join("picked");
    git(&[
        "worktree",
        "add",
        "-b",
        "picked-branch",
        picked.to_str().unwrap(),
        "main",
    ]);
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let mut executor = target.client(uuid::Uuid::new_v4());
    let project = executor.command(
        "project.put",
        json!({"path":repository}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(project["result"]["status"], "accepted", "{project}");
    let project = project["result"]["result"]["projectId"].clone();
    // Force different local id spaces so a missing upstream translation
    // cannot pass by accidentally addressing the same numeric thread id.
    for _ in 0..2 {
        let created = executor.command(
            "thread.create",
            json!({"project":project,"occupiedDrafts":[1,2,3]}),
            uuid::Uuid::new_v4(),
        );
        assert!(
            created["result"]["result"]["threadId"].is_number(),
            "{created}"
        );
    }
    let discovery = target.discovery();
    let pairing = target.cli(&["pair", "--peer"]);
    assert!(pairing.status.success());
    let pairing = String::from_utf8(pairing.stdout).unwrap();
    let code = pairing
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
    let mut owner = home.client(uuid::Uuid::new_v4());
    owner.until(|state| {
        state["peers"].as_array().is_some_and(|peers| {
            peers.iter().any(|peer| {
                peer["serverId"] == discovery["serverId"]
                    && peer["capabilities"]["contracts"]
                        .as_array()
                        .is_some_and(|contracts| {
                            contracts
                                .iter()
                                .any(|contract| contract == protocol::PLACEMENT)
                        })
                    && peer["capabilities"]["models"]
                        .as_array()
                        .is_some_and(|models| models.iter().any(|model| model["id"] == "fixture"))
            })
        })
    });
    let captain = owner.command(
        "captain.put",
        json!({"record":{"name":"Placement captain","role":"","instructions":"","portrait":0}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(captain["result"]["status"], "accepted", "{captain}");
    let opened = owner.command("captain.open", json!({"captain":0}), uuid::Uuid::new_v4());
    assert_eq!(opened["result"]["status"], "accepted", "{opened}");
    let parent = opened["result"]["result"]["threadId"].clone();
    let scope = owner.command(
        "captain.scope",
        json!({
            "captain":0, "projects":[],
            "remoteProjects":[{"server":discovery["serverId"],"projectId":project}]
        }),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(scope["result"]["status"], "accepted", "{scope}");
    let mut homes = Vec::new();
    for brief in ["placement-writer-one", "placement-writer-two"] {
        let args = json!({
            "prompt":brief, "client_request_id":brief,
            "target":{"environmentId":discovery["serverId"],"projectId":project,"model":"fixture"},
            "placement":{"mode":"isolated","baseCommit":base}
        });
        let accepted = owner.command(
            "task.delegate",
            json!({"threadId":parent,"args":args}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(accepted["result"]["status"], "accepted", "{accepted}");
        let child = accepted["result"]["result"]["thread_id"].clone();
        assert!(child.is_number(), "{accepted}");
        let replay = owner.command(
            "task.delegate",
            json!({"threadId":parent,"args":args}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(replay["result"]["result"]["thread_id"], child);
        homes.push(child);
    }
    executor.until(|state| {
        let Some(threads) = state["index"]["threads"].as_array() else {
            return false;
        };
        let workers: Vec<_> = threads
            .iter()
            .filter(|thread| thread["peerOrigin"].is_object())
            .collect();
        workers.len() == 2
            && workers.iter().all(|thread| {
                thread["placement"]["phase"] == "ready"
                    && thread["placement"]["checkout"]
                        .as_str()
                        .is_some_and(|checkout| {
                            fs::read_to_string(
                                PathBuf::from(checkout).join("fixture-prompts.jsonl"),
                            )
                            .ok()
                            .is_some_and(|text| {
                                text.ends_with('\n') && text.contains("placement-writer-")
                            })
                        })
            })
    });
    let workers: Vec<_> = executor.snapshot["index"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|thread| thread["peerOrigin"].is_object())
        .collect();
    let first = workers[0]["placement"].clone();
    let second = workers[1]["placement"].clone();
    let upstream_target = workers[0]["id"].clone();
    assert_ne!(first["checkout"], second["checkout"]);
    assert_ne!(first["branch"], second["branch"]);
    for worker in &workers {
        let placement = &worker["placement"];
        assert_eq!(placement["mode"], "isolated");
        assert_eq!(placement["baseCommit"], base);
        let checkout = PathBuf::from(placement["checkout"].as_str().unwrap());
        assert_ne!(checkout, repository);
        assert_eq!(
            read_checkout_text(checkout.join("README.md")),
            "committed base\n"
        );
        assert!(!checkout.join("parent-only.txt").exists());
        assert_eq!(
            executor.snapshot["directories"][worker["id"].as_u64().unwrap().to_string()],
            placement["checkout"]
        );
    }
    owner.until(|state| {
        state["index"]["tasks"].as_array().is_some_and(|tasks| {
            tasks.len() == 2
                && tasks
                    .iter()
                    .all(|task| task["remote"]["placement"]["phase"] == "ready")
        })
    });
    for task in owner.snapshot["index"]["tasks"].as_array().unwrap() {
        let placement = &task["remote"]["placement"];
        assert!(placement == &first || placement == &second);
        assert!(homes.contains(&task["child_thread_id"]));
    }
    let accepted = owner.command(
        "task.delegate",
        json!({"threadId":parent,"args":{
            "prompt":"placement-writer-existing", "client_request_id":"placement-writer-existing",
            "target":{"environmentId":discovery["serverId"],"projectId":project,"model":"fixture"},
            "placement":{"mode":"existing","worktree":"picked-branch"}
        }}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(accepted["result"]["status"], "accepted", "{accepted}");
    let existing_home = accepted["result"]["result"]["thread_id"].clone();
    assert!(existing_home.is_number(), "{accepted}");
    executor.until(|state| {
        let Some(threads) = state["index"]["threads"].as_array() else {
            return false;
        };
        let workers: Vec<_> = threads
            .iter()
            .filter(|thread| thread["peerOrigin"].is_object())
            .collect();
        workers.len() == 3
            && workers.iter().any(|thread| {
                thread["placement"]["mode"] == "existing"
                    && thread["placement"]["phase"] == "ready"
                    && thread["placement"]["checkout"]
                        .as_str()
                        .is_some_and(|checkout| {
                            fs::read_to_string(
                                PathBuf::from(checkout).join("fixture-prompts.jsonl"),
                            )
                            .ok()
                            .is_some_and(|text| {
                                text.ends_with('\n') && text.contains("placement-writer-existing")
                            })
                        })
            })
    });
    let workers: Vec<_> = executor.snapshot["index"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|thread| thread["peerOrigin"].is_object())
        .collect();
    assert_eq!(workers.len(), 3);
    let existing = workers
        .iter()
        .find(|thread| thread["placement"]["mode"] == "existing")
        .expect("existing worktree worker");
    let placement = &existing["placement"];
    assert_eq!(placement["branch"], "picked-branch");
    assert_eq!(placement["newWorktree"], false);
    assert_eq!(placement["primaryCheckout"], false);
    assert_eq!(
        fs::canonicalize(placement["checkout"].as_str().unwrap()).unwrap(),
        fs::canonicalize(&picked).unwrap()
    );
    assert_eq!(
        executor.snapshot["directories"][existing["id"].as_u64().unwrap().to_string()],
        placement["checkout"]
    );
    assert_eq!(
        read_checkout_text(picked.join("README.md")),
        "committed base\n"
    );
    assert!(!picked.join("parent-only.txt").exists());
    owner.until(|state| {
        state["index"]["tasks"].as_array().is_some_and(|tasks| {
            tasks.len() == 3
                && tasks
                    .iter()
                    .all(|task| task["remote"]["placement"]["phase"] == "ready")
        })
    });
    let existing_task = owner.snapshot["index"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["child_thread_id"] == existing_home)
        .expect("existing worktree task");
    assert!(existing_task["remote"]["placement"] == *placement);

    // A dependent launch uses the home-local worker address on the wire,
    // maps it to the executor's worker, and starts at its output commit.
    let upstream_home = owner.snapshot["index"]["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["remote"]["threadId"] == upstream_target)
        .expect("upstream task mapping")["child_thread_id"]
        .clone();
    assert_ne!(upstream_home, upstream_target);
    let upstream_checkout = first["checkout"].as_str().unwrap();
    fs::write(
        PathBuf::from(upstream_checkout).join("upstream-result.txt"),
        "upstream-only result\n",
    )
    .unwrap();
    git(&["-C", upstream_checkout, "add", "upstream-result.txt"]);
    git(&[
        "-C",
        upstream_checkout,
        "commit",
        "-m",
        "upstream output",
        "--quiet",
    ]);
    let upstream_head = git(&["-C", upstream_checkout, "rev-parse", "HEAD"])
        .trim()
        .to_owned();
    assert_ne!(upstream_head, base);
    let dependent = owner.command(
        "task.delegate",
        json!({"threadId":parent,"args":{
            "prompt":"placement-writer-dependent", "client_request_id":"placement-writer-dependent",
            "target":{"environmentId":discovery["serverId"],"projectId":project,"model":"fixture"},
            "placement":{"mode":"isolated","upstreamThreadId":upstream_home}
        }}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(dependent["result"]["status"], "accepted", "{dependent}");
    let dependent_home = dependent["result"]["result"]["thread_id"].clone();
    executor.until(|state| {
        state["index"]["threads"].as_array().is_some_and(|threads| {
            threads.iter().any(|thread| {
                thread["placement"]["upstreamThreadId"] == upstream_target
                    && thread["placement"]["phase"] == "ready"
                    && thread["placement"]["baseCommit"] == upstream_head
                    && thread["placement"]["checkout"]
                        .as_str()
                        .is_some_and(|checkout| {
                            fs::read_to_string(
                                PathBuf::from(checkout).join("fixture-prompts.jsonl"),
                            )
                            .ok()
                            .is_some_and(|text| {
                                text.ends_with('\n') && text.contains("placement-writer-dependent")
                            })
                        })
            })
        })
    });
    let dependent_placement = executor.snapshot["index"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["placement"]["upstreamThreadId"] == upstream_target)
        .expect("dependent worker")["placement"]
        .clone();
    assert_eq!(dependent_placement["baseBranch"], first["branch"]);
    assert_eq!(dependent_placement["baseCommit"], upstream_head);
    assert_ne!(dependent_placement["branch"], first["branch"]);
    let dependent_checkout = dependent_placement["checkout"].as_str().unwrap();
    assert_eq!(
        git(&["-C", dependent_checkout, "rev-parse", "HEAD"]).trim(),
        upstream_head
    );
    assert_eq!(
        read_checkout_text(PathBuf::from(dependent_checkout).join("upstream-result.txt")),
        "upstream-only result\n"
    );
    owner.until(|state| {
        state["index"]["tasks"].as_array().is_some_and(|tasks| {
            tasks.iter().any(|task| {
                task["child_thread_id"] == dependent_home
                    && task["remote"]["placement"] == dependent_placement
            })
        })
    });
    assert_eq!(git(&["status", "--porcelain"]), original_status);
    assert_eq!(git(&["symbolic-ref", "--short", "HEAD"]).trim(), "main");
    assert_eq!(
        git(&["worktree", "list", "--porcelain"])
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count(),
        5
    );
    assert!(!repository.join("fixture-prompts.jsonl").exists());
    drop(owner);
    drop(executor);
    home.stop();
    target.stop();
}

// Git may check out ordinary text files with CRLF under the host's configuration.
fn read_checkout_text(path: impl AsRef<std::path::Path>) -> String {
    fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

#[cfg(unix)]
struct PausedProcess(u32);
#[cfg(unix)]
impl PausedProcess {
    fn new(child: &Child) -> Self {
        // Drop resumes this fixture-owned serve process even when an assertion fails.
        assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGSTOP) }, 0);
        Self(child.id())
    }
}
#[cfg(unix)]
impl Drop for PausedProcess {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.0 as i32, libc::SIGCONT);
        }
    }
}
impl Profile {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("shina-server-integration-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("Shina")).unwrap();
        fs::create_dir_all(root.join("enrollment")).unwrap();
        fs::write(root.join("enrollment/models.json"), json!({"providers":{"cpa":{"baseUrl":"http://127.0.0.1:1/v1","models":[{"id":"fixture"}]}}}).to_string()).unwrap();
        fs::write(
            root.join("fake_pi.js"),
            include_str!("../assets/tests/fake_pi.cjs"),
        )
        .unwrap();
        fs::write(
            root.join("fake_acp.py"),
            include_str!("../assets/tests/fake_acp.py"),
        )
        .unwrap();
        fs::write(
            root.join("fake_claude_requests.js"),
            include_str!("../assets/tests/fake_claude_requests.cjs"),
        )
        .unwrap();
        fs::write(
            root.join("fake_codex.js"),
            include_str!("../assets/tests/fake_codex.js"),
        )
        .unwrap();
        fs::write(root.join("Shina/harnesses.json"), json!([{"id":"fixture-acp","name":"Fixture ACP",
            "command":if cfg!(windows) { "python" } else { "python3" },
            "args":["-u",root.join("fake_acp.py"),"--log",root.join("acp-log.jsonl"),"--delay-seconds","0.6","--resume","--no-history"]}]).to_string()).unwrap();
        // Pin both the initial draft and newly allocated threads to the fake Pi backend.
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        fs::write(
            root.join("Shina/index.json"),
            json!({
                "version": 4,
                "defaultHarness": "pi",
                "captains": [],
                "projects": [],
                "threads": [{
                    "id": 1,
                    "title": "New thread",
                    "machine": "local",
                    "harness": "pi",
                    "neverSent": true,
                    "updated": now.as_secs(),
                    "lifecycle": {"createdAt": now.as_millis() as u64, "updatedAt": now.as_millis() as u64},
                }],
            })
            .to_string(),
        )
        .unwrap();
        Self {
            root,
            children: Vec::new(),
        }
    }
    fn launch(&mut self, args: &[&str]) -> usize {
        self.launch_with_fault(args, None)
    }
    fn launch_with_fault(&mut self, args: &[&str], fault: Option<&str>) -> usize {
        let mut command = Command::new(env!("CARGO_BIN_EXE_shina"));
        command
            .args(args)
            .env("LOCALAPPDATA", &self.root)
            .env("XDG_DATA_HOME", &self.root)
            .env("T3CODE_HOME", self.root.join("t3-fixture"))
            .env("PI_CODING_AGENT_DIR", self.root.join("enrollment"))
            .env("SHINA_PI_CLI", self.root.join("fake_pi.js"))
            .env("SHINA_CODEX_CLI", self.root.join("no-real-codex"))
            .env("SHINA_CLAUDE_CLI", self.root.join("no-real-claude"))
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(
                fs::File::create(
                    self.root
                        .join(format!("server-{}.log", self.children.len())),
                )
                .unwrap(),
            );
        if let Some(fault) = fault {
            command.env("SHINA_TEST_CRASH_EFFECT", fault);
        }
        if let Ok(extra) = fs::read(self.root.join("extra-env.json"))
            && let Ok(Value::Object(extra)) = serde_json::from_slice::<Value>(&extra)
        {
            for (name, value) in extra {
                if let Some(value) = value.as_str() {
                    command.env(name, value);
                }
            }
        }
        if self.root.join("use-native-approval-fixture").exists() {
            command
                .env(
                    "SHINA_CLAUDE_CLI",
                    self.root.join("fake_claude_requests.js"),
                )
                .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"));
        }
        if self.root.join("use-fake-codex-fixture").exists() {
            command.env("SHINA_CODEX_CLI", self.root.join("fake_codex.js"));
        }
        if self.root.join("measure-storage").exists() {
            command.env(
                "SHINA_TEST_STORAGE_METRICS",
                self.root.join("storage.jsonl"),
            );
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        self.children.push(command.spawn().unwrap());
        self.children.len() - 1
    }
    fn discovery(&self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            if let Ok(bytes) = fs::read(self.root.join("Shina/server.json"))
                && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
                && let Some(port) = value["port"].as_u64()
                && TcpStream::connect_timeout(
                    &([127, 0, 0, 1], port as u16).into(),
                    Duration::from_millis(50),
                )
                .is_ok()
            {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "serve did not publish readiness: {}",
                fs::read_to_string(self.root.join(format!(
                    "server-{}.log",
                    self.children.len().saturating_sub(1)
                )))
                .unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn client(&self, device: uuid::Uuid) -> Client {
        Client::connect(&self.discovery(), device)
    }
    fn cli(&self, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_shina"));
        command
            .args(args)
            .env("LOCALAPPDATA", &self.root)
            .env("XDG_DATA_HOME", &self.root)
            .env("PI_CODING_AGENT_DIR", self.root.join("enrollment"))
            .env("SHINA_PI_CLI", self.root.join("fake_pi.js"))
            .env("SHINA_CODEX_CLI", self.root.join("no-real-codex"))
            .env("SHINA_CLAUDE_CLI", self.root.join("no-real-claude"))
            .current_dir(&self.root)
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command.output().unwrap()
    }
    fn stop(&mut self) {
        if self
            .children
            .iter_mut()
            .any(|child| child.try_wait().unwrap().is_none())
        {
            let mut client = self.client(uuid::Uuid::new_v4());
            let _ = client.command("server.stop", json!({}), uuid::Uuid::new_v4());
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        for child in &mut self.children {
            while child.try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline, "serve did not stop");
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        for child in &mut self.children {
            #[cfg(windows)]
            if child.try_wait().ok().flatten().is_none() {
                use std::os::windows::process::CommandExt;
                // A failing service-launcher test must also reap its owned serve child.
                let _ = Command::new("taskkill.exe")
                    .args(["/F", "/T", "/PID", &child.id().to_string()])
                    .creation_flags(0x08000000)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        if std::thread::panicking() {
            for index in 0..self.children.len() {
                eprintln!(
                    "fixture server {index}: {}",
                    fs::read_to_string(self.root.join(format!("server-{index}.log")))
                        .unwrap_or_default()
                );
            }
        }
        // This path was created under temp_dir with a fresh UUID, never the real profile.
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Client {
    socket: WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    snapshot: Value,
    seq: u64,
    epoch: Value,
    notifications: Vec<Value>,
    token: String,
    history_cache: Option<client_history::ProjectionCache>,
    snapshot_pages: String,
}
impl Client {
    fn connect(discovery: &Value, device: uuid::Uuid) -> Self {
        let port = discovery["port"].as_u64().unwrap() as u16;
        let mut http = TcpStream::connect(("127.0.0.1", port)).unwrap();
        http.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let body = json!({"deviceId":device}).to_string();
        write!(http,"POST /v1/auth HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",discovery["bootstrapToken"].as_str().unwrap(),body.len(),body).unwrap();
        let mut reply = String::new();
        http.read_to_string(&mut reply).unwrap();
        assert!(
            reply.starts_with("HTTP/1.1 200"),
            "bootstrap exchange failed"
        );
        let auth: Value = serde_json::from_str(reply.split_once("\r\n\r\n").unwrap().1).unwrap();
        Self::with_token(discovery, auth["token"].as_str().unwrap().to_owned())
    }
    fn with_token(discovery: &Value, token: String) -> Self {
        let mut client = Self::unsubscribed(discovery, token);
        let hello = client.rpc(
            "hello",
            json!({"protocolMajor":1,"protocolMinor":0,"projectionSchema":1}),
            json!("hello"),
        );
        assert!(hello["error"].is_null(), "{hello}");
        client.rpc("workspace.subscribe", json!({}), json!("subscribe"));
        assert!(!client.snapshot.is_null());
        client
    }
    fn unsubscribed(discovery: &Value, token: String) -> Self {
        let port = discovery["port"].as_u64().unwrap() as u16;
        let mut request = format!("ws://127.0.0.1:{port}/v1/ws")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut socket, _) = loop {
            match tungstenite::connect(request.clone()) {
                Ok(connected) => break connected,
                Err(error) => {
                    assert!(
                        Instant::now() < deadline,
                        "WebSocket listener did not become ready: {error}"
                    );
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        };
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
        }
        Self {
            socket,
            snapshot: Value::Null,
            seq: 0,
            epoch: Value::Null,
            notifications: Vec::new(),
            token,
            history_cache: None,
            snapshot_pages: String::new(),
        }
    }
    fn rpc(&mut self, method: &str, params: Value, id: Value) -> Value {
        self.socket
            .send(Message::Text(
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
                    .to_string()
                    .into(),
            ))
            .unwrap();
        loop {
            let value = self.read();
            if value["id"] == id {
                return value;
            }
        }
    }
    fn command(&mut self, method: &str, args: Value, id: uuid::Uuid) -> Value {
        self.rpc(method, json!({"commandId":id,"args":args}), json!(id))
    }
    fn read(&mut self) -> Value {
        loop {
            match self.socket.read().unwrap_or_else(|error| {
                panic!(
                    "Socket read failed: {error}. Last snapshot: {}",
                    self.snapshot
                )
            }) {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if let Some(cache) = &mut self.history_cache {
                        cache.receive(&value).unwrap();
                    }
                    if value["method"] == "subscription.snapshotPage" {
                        self.snapshot_pages.push_str(
                            value["params"]["data"]
                                .as_str()
                                .expect("snapshot page data"),
                        );
                        if value["params"]["nextCursor"].is_null() {
                            let page: Value =
                                serde_json::from_str(&std::mem::take(&mut self.snapshot_pages))
                                    .expect("complete snapshot pages");
                            self.snapshot = page["state"].clone();
                            self.seq = page["seq"].as_u64().unwrap();
                            self.epoch = page["epoch"].clone();
                        }
                    }
                    if value["method"] == "subscription.snapshot" {
                        self.snapshot = value["params"]["state"].clone();
                        self.seq = value["params"]["seq"].as_u64().unwrap();
                        self.epoch = value["params"]["epoch"].clone();
                    }
                    if value["method"] == "subscription.ready" {
                        assert!(
                            value["params"]["seq"].as_u64().unwrap() >= self.seq,
                            "synchronization cursor precedes its installed snapshot"
                        );
                    }
                    if value["method"] == "subscription.patch" {
                        if let Err(error) =
                            protocol::apply(&mut self.snapshot, value["params"]["patch"].clone())
                        {
                            let ids: Vec<Value> = self.snapshot["index"]["threads"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|thread| thread["id"].clone())
                                .collect();
                            panic!(
                                "subscription patch at seq {} failed: {error}; index thread ids {ids:?}; batch {}",
                                self.seq + 1,
                                value["params"]["patch"]
                            );
                        }
                        self.seq = value["params"]["seq"].as_u64().unwrap();
                    }
                    if value["method"]
                        .as_str()
                        .is_some_and(|method| method.starts_with("subscription."))
                    {
                        self.notifications.push(value.clone());
                        if self.notifications.len() > 512 {
                            self.notifications.remove(0);
                        }
                    }
                    return value;
                }
                Message::Ping(payload) => self.socket.send(Message::Pong(payload)).unwrap(),
                _ => {}
            }
        }
    }
    fn until(&mut self, predicate: impl Fn(&Value) -> bool) {
        self.until_sequence(0, predicate);
    }
    fn until_sequence(&mut self, seq: u64, predicate: impl Fn(&Value) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while self.seq < seq || !predicate(&self.snapshot) {
            assert!(
                Instant::now() < deadline,
                "snapshot condition timed out: {}",
                self.snapshot
            );
            self.read();
        }
    }
}

#[test]
fn ordinary_files_transfer_between_linked_servers_and_survive_restart() {
    use sha2::Digest;
    fn upload(http: &reqwest::blocking::Client, base: &str, token: &str, bytes: &[u8]) -> Value {
        let start = http
            .post(format!("{base}/v1/uploads"))
            .bearer_auth(token)
            .json(
                &json!({"type":"file","name":"ordinary.bin","mimeType":"application/octet-stream",
                "length":bytes.len(),"sha256":format!("{:x}",sha2::Sha256::digest(bytes))}),
            )
            .send()
            .unwrap();
        assert!(start.status().is_success(), "{start:?}");
        let started: Value = start.json().unwrap();
        let id = started["uploadId"].as_str().unwrap();
        let mut offset = 0;
        for chunk in bytes.chunks(protocol::PAGE_BYTES) {
            let response = http
                .put(format!("{base}/v1/uploads/{id}?offset={offset}"))
                .bearer_auth(token)
                .body(chunk.to_vec())
                .send()
                .unwrap();
            assert!(response.status().is_success(), "{response:?}");
            offset += chunk.len();
        }
        let response = http
            .post(format!("{base}/v1/uploads/{id}/commit"))
            .bearer_auth(token)
            .send()
            .unwrap();
        assert!(response.status().is_success(), "{response:?}");
        response.json().unwrap()
    }
    fn download(
        http: &reqwest::blocking::Client,
        base: &str,
        token: &str,
        id: &str,
        length: usize,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        while bytes.len() < length {
            let response = http
                .get(format!(
                    "{base}/v1/blobs/{id}?offset={}&limit={}",
                    bytes.len(),
                    protocol::PAGE_BYTES
                ))
                .bearer_auth(token)
                .send()
                .unwrap();
            assert!(response.status().is_success(), "{response:?}");
            assert_eq!(
                response.headers()["x-blob-length"].to_str().unwrap(),
                length.to_string()
            );
            let chunk = response.bytes().unwrap();
            assert!(!chunk.is_empty() && chunk.len() <= protocol::PAGE_BYTES);
            bytes.extend_from_slice(&chunk);
        }
        assert_eq!(bytes.len(), length);
        bytes
    }
    let mut a = Profile::new();
    let mut b = Profile::new();
    a.launch(&["serve"]);
    b.launch(&["serve"]);
    let discovery_a = a.discovery();
    let discovery_b = b.discovery();
    let base_a = format!("http://127.0.0.1:{}", discovery_a["port"]);
    let base_b = format!("http://127.0.0.1:{}", discovery_b["port"]);
    let mut local_a = a.client(uuid::Uuid::new_v4());
    let mut local_b = b.client(uuid::Uuid::new_v4());
    let paired = a.cli(&["pair"]);
    assert!(paired.status.success());
    let code = String::from_utf8(paired.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let host_a = format!("127.0.0.1:{}", discovery_a["port"]);
    let linked = b.cli(&["link", &host_a, &code]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let credential: Value = serde_json::from_slice(
        &fs::read(
            b.root
                .join("Shina/linked")
                .join(discovery_a["serverId"].as_str().unwrap())
                .join("credential.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let token_a = credential["token"].as_str().unwrap();
    let mut linked_a = Client::with_token(&discovery_a, token_a.into());
    let hello = linked_a.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":10,"projectionSchema":1}),
        json!("files-hello"),
    );
    for capability in [
        protocol::MODEL_FAVORITES,
        protocol::MODEL_CATALOG_REFRESH,
        protocol::MODEL_OPTIONS,
        protocol::THREAD_FIND,
        protocol::THREAD_SEARCH,
        protocol::COMPOSER_CONTEXT,
        protocol::FILE_ATTACHMENTS,
    ] {
        assert!(
            hello["result"]["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cap| cap == capability),
            "{capability}: {hello}"
        );
    }
    let old = linked_a.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":9,"projectionSchema":1}),
        json!("files-old-hello"),
    );
    assert!(
        !old["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == protocol::FILE_ATTACHMENTS)
    );
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let bytes: Vec<_> = (0..protocol::PAGE_BYTES * 2 + 71)
        .map(|i| (i % 251) as u8)
        .collect();
    let first = upload(&http, &base_a, &local_a.token, &bytes);
    let id = first["id"].as_str().unwrap();
    assert_eq!(id, format!("file-{:x}.bin", sha2::Sha256::digest(&bytes)));
    let fetched = download(&http, &base_a, token_a, id, bytes.len());
    assert_eq!(fetched, bytes);
    let second = upload(&http, &base_b, &local_b.token, &fetched);
    assert_eq!(
        second["id"], id,
        "content addressing is independent of the owning machine"
    );
    assert_eq!(
        upload(&http, &base_b, &local_b.token, &bytes)["id"],
        id,
        "identical uploads are idempotent"
    );
    assert_eq!(
        download(&http, &base_b, &local_b.token, id, bytes.len()),
        bytes
    );
    assert_eq!(
        http.get(format!("{base_a}/v1/blobs/{id}"))
            .send()
            .unwrap()
            .status(),
        401
    );
    for (length, error) in [
        (0, "'ordinary.bin' is empty or could not be read."),
        (
            50 * 1024 * 1024 + 1,
            "'ordinary.bin' exceeds the 50 MB attachment limit.",
        ),
    ] {
        let response = http
            .post(format!("{base_b}/v1/uploads"))
            .bearer_auth(&local_b.token)
            .json(
                &json!({"type":"file","name":"ordinary.bin","mimeType":"application/octet-stream",
                "length":length,"sha256":"0".repeat(64)}),
            )
            .send()
            .unwrap();
        assert_eq!(response.status(), 400);
        assert_eq!(response.json::<Value>().unwrap()["error"]["message"], error);
    }
    let record = json!({"version":1,"contextId":"file_reference","kind":"file","label":"ordinary.bin",
        "attachmentId":id,"name":"ordinary.bin","mimeType":"application/octet-stream","sizeBytes":bytes.len()});
    for (client, attachment) in [(&mut local_a, &first), (&mut local_b, &second)] {
        let sent = client.command(
            "thread.send",
            json!({"threadId":1,
            "text":"Inspect [ordinary.bin](t3-context://v1/file/file_reference)",
            "attachments":[attachment],"context":{"version":1,"records":[record]},"mode":"prompt"}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(sent["result"]["status"], "accepted", "{sent}");
        client.until(|state| state["threads"][0]["status"] == "Completed");
        assert_eq!(
            client.snapshot["threads"][0]["messages"][0]["context"]["records"][0]["attachmentId"],
            id
        );
        assert_eq!(
            client.snapshot["threads"][0]["messages"][0]["images"][0]["id"],
            id
        );
    }
    drop(local_b);
    b.stop();
    b.launch(&["serve"]);
    let mut restored = b.client(uuid::Uuid::new_v4());
    restored.until(|state| {
        !state["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    });
    assert_eq!(
        restored.snapshot["threads"][0]["messages"][0]["context"]["records"][0],
        record
    );
    let restarted_base = format!("http://127.0.0.1:{}", b.discovery()["port"]);
    assert_eq!(
        download(&http, &restarted_base, &restored.token, id, bytes.len()),
        bytes
    );
    drop(restored);
    drop(local_a);
    drop(linked_a);
    a.stop();
    b.stop();
}

#[test]
fn peer_cli_delegation_recovers_home_and_delivers_one_acknowledged_remote_result() {
    let mut home = Profile::new();
    let mut target = Profile::new();
    home.launch(&["serve"]);
    target.launch_with_fault(&["serve"], Some("peer-ack"));
    let target_discovery = target.discovery();
    let mut target_client = target.client(uuid::Uuid::new_v4());
    let workspace = target.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let home_workspace = home.root.join("workspace");
    // The same repository is addressed through HTTPS and SCP-style SSH.
    // Both @ inside the path and a repository name ending in .git survive
    // normalization; stripping credentials or suffixes must not change it.
    for (path, remote) in [
        (&workspace, "https://example.com/team/peer@fixture.git.git"),
        (&home_workspace, "git@EXAMPLE.com:team/peer@fixture.git.git"),
    ] {
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(path)
                .args(["remote", "add", "origin", remote])
                .status()
                .unwrap()
                .success()
        );
    }
    assert_eq!(
        target_client.command(
            "project.put",
            json!({"path":workspace}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let code = target.cli(&["pair", "--peer"]);
    assert!(
        code.status.success(),
        "{}",
        String::from_utf8_lossy(&code.stderr)
    );
    let output = String::from_utf8(code.stdout).unwrap();
    let code = output
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap();
    let host = format!("127.0.0.1:{}", target_discovery["port"]);
    let linked = home.cli(&["peer", "link", &host, code]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let listing = home.cli(&["peer", "list"]);
    assert!(listing.status.success());
    let secret: Value = serde_json::from_slice(
        &fs::read(
            home.root
                .join("Shina/peers")
                .join(target_discovery["serverId"].as_str().unwrap())
                .join("credential.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!String::from_utf8_lossy(&listing.stdout).contains(secret["token"].as_str().unwrap()));
    let mut client = home.client(uuid::Uuid::new_v4());
    assert_eq!(
        client.command(
            "project.put",
            json!({"path":home_workspace}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let parent=client.command("thread.create",json!({"project":1}),uuid::Uuid::new_v4())["result"]["result"]["threadId"].as_u64().unwrap();
    let args = json!({"prompt":"remote-long-fixture","client_request_id":"one-launch",
        "target":{"machine":target_discovery["serverId"],"model":"fixture"}});
    // The home server learns the peer's model catalog asynchronously after linking. Until it
    // has, delegation is refused with "does not advertise this model"; wait for the catalog.
    let mut delegated = Value::Null;
    for _ in 0..100 {
        delegated = client.command(
            "task.delegate",
            json!({"threadId":parent,"args":args}),
            uuid::Uuid::new_v4(),
        );
        let waiting = delegated["result"]["result"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("does not advertise this model"));
        if !waiting {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(delegated["result"]["status"], "accepted", "{delegated}");
    let child = delegated["result"]["result"]["thread_id"].clone();
    let retry = client.command(
        "task.delegate",
        json!({"threadId":parent,"args":args}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(retry["result"]["result"]["thread_id"], child);
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["remote"]["threadId"].is_number())
    });
    let launch =
        client.snapshot["index"]["tasks"][0]["remote"]["launchCommand"]["commandId"].clone();
    #[cfg(unix)]
    let pause = PausedProcess::new(&target.children[0]);
    // Stop only the home server. The accepted target thread remains owned by the target.
    drop(client);
    home.stop();
    home.launch(&["serve"]);
    let mut client = home.client(uuid::Uuid::new_v4());
    #[cfg(unix)]
    {
        client.until(|state| state["index"]["tasks"][0]["remote"]["state"] == "target_offline");
        assert_ne!(client.snapshot["index"]["tasks"][0]["state"], "failed");
        drop(pause);
    }
    let deadline = Instant::now() + Duration::from_secs(25);
    while target.children[0].try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            client.rpc("workspace.subscribe", json!({}), json!("diagnose-follower"));
            target_client.rpc("workspace.subscribe", json!({}), json!("diagnose-target"));
            panic!(
                "target did not reach the acknowledgement crash boundary. Home tasks {}, target threads {}",
                client.snapshot["index"]["tasks"], target_client.snapshot["index"]["threads"]
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        target.children[0].try_wait().unwrap().unwrap().code(),
        Some(86)
    );
    drop(target_client);
    // The injected crash leaves discovery behind. Do not authenticate with its
    // old bootstrap token while the restarted listener reuses the same port.
    fs::remove_file(target.root.join("Shina/server.json")).unwrap();
    target.launch(&["serve", "--port", &target_discovery["port"].to_string()]);
    target_client = target.client(uuid::Uuid::new_v4());
    client.until(|state| state["index"]["tasks"][0]["state"] == "completed");
    assert_eq!(
        client.snapshot["index"]["tasks"][0]["remote"]["launchCommand"]["commandId"],
        launch
    );
    assert_eq!(
        client.snapshot["index"]["tasks"].as_array().unwrap().len(),
        1
    );
    assert_eq!(client.snapshot["index"]["tasks"][0]["delivered"], false);
    let release = client.command(
        "task.resumeResults",
        json!({"threadId":parent}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(release["result"]["status"], "accepted", "{release}");
    client.until(|state| state["index"]["tasks"][0]["delivered"] == true);
    client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == parent)
            .is_some_and(|t| t["status"] == "Completed")
    });
    let messages = client.snapshot["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == parent)
        .unwrap()["messages"]
        .as_array()
        .unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m["text"]
                .as_str()
                .is_some_and(|s| s.contains("Shina: task #")))
            .count(),
        1
    );
    let excerpt = client.snapshot["index"]["tasks"][0]["result_excerpt"]
        .as_str()
        .unwrap();
    assert!(excerpt.contains("START FACT"));
    assert!(!excerpt.contains("END FACT"));
    let completion = messages
        .iter()
        .find(|m| {
            m["text"]
                .as_str()
                .is_some_and(|s| s.contains("Shina: task #"))
        })
        .unwrap()["text"]
        .as_str()
        .unwrap();
    assert!(completion.lines().last().unwrap().contains("read_thread"));
    for restarted in [false, true] {
        if restarted {
            drop(target_client);
            target.stop();
            target.launch(&["serve", "--port", &target_discovery["port"].to_string()]);
            target_client = target.client(uuid::Uuid::new_v4());
        }
        let first = read_fixture_thread(
            &home_workspace,
            json!({"threadId":client.snapshot["index"]["tasks"][0]["remote"]["threadId"],
                "machine":target_discovery["serverId"],"maxCharsPerItem":6000}),
        );
        let item = first["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "assistant")
            .unwrap();
        assert_eq!(item["textTruncated"], true);
        let second = read_fixture_thread(&home_workspace, item["continuation"].clone());
        let text = format!(
            "{}{}",
            item["text"].as_str().unwrap(),
            second["items"][0]["text"].as_str().unwrap()
        );
        for fact in ["START FACT", "MIDDLE FACT", "END FACT"] {
            assert!(
                text.contains(fact),
                "{fact} missing after restart={restarted}"
            );
        }
        assert_eq!(second["items"][0]["textTruncated"], false);
    }
    let result_thread = client.snapshot["index"]["tasks"][0]["remote"]["threadId"].clone();
    target_client.rpc(
        "workspace.subscribe",
        json!({}),
        json!("resubscribe-peer-result"),
    );
    let rows = target_client.snapshot["index"]["threads"]
        .as_array()
        .unwrap();
    assert_eq!(
        rows.iter()
            .filter(|t| t["peerOrigin"]["taskId"].is_string())
            .count(),
        1
    );
    assert!(
        rows.iter().find(|t| t["id"] == result_thread).unwrap()["peerOrigin"]["acknowledged"]
            == true
    );
    let followup = client.command(
        "task.send",
        json!({"threadId":parent,"args":{"thread_id":child,"message":"remote-long-fixture-again","client_request_id":"followup-again"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(followup["result"]["status"], "accepted", "{followup}");
    client.until(|state| state["index"]["tasks"][0]["state"] == "running");
    client.until(|state| state["index"]["tasks"][0]["state"] == "completed");
    let target_finished: u64 = fs::read_to_string(workspace.join("fixture-finished-at"))
        .unwrap()
        .parse()
        .unwrap();
    let home_finished = client.snapshot["index"]["tasks"][0]["finished_at"]
        .as_u64()
        .unwrap();
    assert!(
        home_finished.saturating_sub(target_finished) < 1500,
        "completion took {}ms, an extra polling interval",
        home_finished.saturating_sub(target_finished)
    );
    assert!(
        client.snapshot["index"]["tasks"][0]["result"]
            .as_str()
            .unwrap()
            .contains("END FACT")
    );
    let confirmation = client.command(
        "task.send",
        json!({"threadId":parent,"args":{"thread_id":child,"message":"remote-confirm-fixture","client_request_id":"confirm-turn"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(
        confirmation["result"]["status"], "accepted",
        "{confirmation}"
    );
    client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == child && t["request"]["id"] == "remote-confirm")
    });
    let pending = client.snapshot["index"]["tasks"][0]["remote"]["request"].clone();
    assert_eq!(pending["threadId"], result_thread);
    assert_eq!(pending["kind"], "confirm");
    assert_eq!(pending["payload"]["title"], "Run fixture command?");
    let stale = target_client.command(
        "request.respond",
        json!({"threadId":result_thread,"requestId":pending["requestId"],
        "generation":"stale-generation","response":{"confirmed":true}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(stale["result"]["status"], "rejected", "{stale}");
    target_client.rpc("workspace.subscribe", json!({}), json!("check-stale"));
    assert!(
        target_client.snapshot["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == result_thread && t["request"]["id"] == "remote-confirm")
    );
    let approved = target_client.command(
        "request.respond",
        json!({"threadId":result_thread,"requestId":pending["requestId"],
            "generation":pending["generation"],"response":{"confirmed":true}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(approved["result"]["status"], "accepted", "{approved}");
    client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == child && t["request"].is_null())
    });
    #[cfg(unix)]
    {
        let followup = client.command(
            "task.send",
            json!({"threadId":parent,"args":{"thread_id":child,"message":"A second remote turn","client_request_id":"second-turn"}}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(followup["result"]["status"], "accepted", "{followup}");
        let attempt = followup["result"]["result"]["attempt_id"].clone();
        let pause = PausedProcess::new(target.children.last().unwrap());
        let cancelled = client.command(
            "task.cancel",
            json!({"threadId":parent,"args":{"thread_id":child,"client_request_id":"stop-second-turn","attempt_id":attempt}}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(cancelled["result"]["status"], "accepted", "{cancelled}");
        client.until(|state| {
            state["index"]["tasks"][0]["state"] == "cancelled"
                && state["index"]["tasks"][0]["remote"]["stopOwed"] == true
        });
        drop(client);
        home.stop();
        home.launch(&["serve"]);
        client = home.client(uuid::Uuid::new_v4());
        assert_eq!(
            client.snapshot["index"]["tasks"][0]["remote"]["stopOwed"],
            true
        );
        drop(pause);
        client.until(|state| state["index"]["tasks"][0]["remote"]["stopOwed"] == false);
        assert_eq!(client.snapshot["index"]["tasks"][0]["state"], "cancelled");
    }
    let unlink = home.cli(&[
        "peer",
        "unlink",
        target_discovery["serverId"].as_str().unwrap(),
    ]);
    assert!(
        unlink.status.success(),
        "{}",
        String::from_utf8_lossy(&unlink.stderr)
    );
    drop(client);
    drop(target_client);
    home.stop();
    target.stop();
}

#[test]
fn remote_project_scope_selects_remote_only_git_plain_and_scratch_and_survives_reconnect() {
    let mut home = Profile::new();
    let mut target = Profile::new();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let discovery = target.discovery();
    let mut there = target.client(uuid::Uuid::new_v4());
    for name in ["remote-only", "plain", "scratch", "duplicate", "outside"] {
        let path = target.root.join(name);
        fs::create_dir_all(&path).unwrap();
        if matches!(name, "remote-only" | "duplicate") {
            assert!(
                Command::new("git")
                    .args(["init", "--quiet"])
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args([
                        "remote",
                        "add",
                        "origin",
                        "https://example.com/only-there.git"
                    ])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        assert_eq!(
            there.command("project.put", json!({"path":path}), uuid::Uuid::new_v4())["result"]["status"],
            "accepted"
        );
    }
    let code = target.cli(&["pair", "--peer"]);
    assert!(code.status.success());
    let output = String::from_utf8(code.stdout).unwrap();
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
    let mut here = home.client(uuid::Uuid::new_v4());
    here.until(|state| {
        state["peers"]
            .as_array()
            .is_some_and(|ps| ps.iter().any(|p| p["serverId"] == discovery["serverId"]))
    });
    assert_eq!(
        here.command(
            "captain.put",
            json!({"record":{"name":"Remote captain","role":"","instructions":"","portrait":0}}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let opened = here.command(
        "captain.open",
        json!({"captain":0,"project":null}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(opened["result"]["status"], "accepted", "{opened}");
    let parent = opened["result"]["result"]["threadId"].clone();
    let scoped = json!(
        [1, 2, 3, 4]
            .into_iter()
            .map(|id| json!({"server":discovery["serverId"],"projectId":id}))
            .collect::<Vec<_>>()
    );
    let scope = here.command(
        "captain.scope",
        json!({"captain":0,"projects":[],"remoteProjects":scoped}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(scope["result"]["status"], "accepted", "{scope}");
    for project in [1, 2, 3, 4] {
        let args = json!({"prompt":"remote-long-fixture","client_request_id":format!("project-{project}"),
            "target":{"environmentId":discovery["serverId"],"projectId":project,"model":"fixture"}});
        let response = here.command(
            "task.delegate",
            json!({"threadId":parent,"args":args}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(response["result"]["status"], "accepted", "{response}");
        assert_eq!(response["result"]["result"]["project"]["id"], project);
        assert_eq!(
            response["result"]["result"]["machine"],
            discovery["serverId"]
        );
        here.until(|s| {
            s["index"]["tasks"].as_array().unwrap().iter().any(|t| {
                t["remote"]["project"]["id"] == project && t["remote"]["threadId"].is_number()
            })
        });
        there.until(|s| {
            s["index"]["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["project"] == project && t["peerOrigin"]["taskId"].is_string())
        });
    }
    let denied = here.command("task.delegate", json!({"threadId":parent,"args":{
        "prompt":"Denied","target":{"environmentId":discovery["serverId"],"projectId":5,"model":"fixture"}}}), uuid::Uuid::new_v4());
    assert_eq!(denied["result"]["status"], "rejected", "{denied}");
    assert!(denied.to_string().contains("outside"));
    drop(here);
    home.stop();
    home.launch(&["serve"]);
    let mut here = home.client(uuid::Uuid::new_v4());
    let retry = here.command(
        "task.delegate",
        json!({"threadId":parent,"args":{
        "prompt":"remote-long-fixture","client_request_id":"project-4",
        "target":{"environmentId":discovery["serverId"],"projectId":4,"model":"fixture"}}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(retry["result"]["status"], "accepted", "{retry}");
    assert_eq!(
        here.snapshot["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["remote"]["project"]["id"] == 4)
            .count(),
        1
    );
    let local = home.root.join("local-counterpart");
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(&local)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&local)
            .args([
                "remote",
                "add",
                "origin",
                "https://example.com/only-there.git"
            ])
            .status()
            .unwrap()
            .success()
    );
    here.command("project.put", json!({"path":local}), uuid::Uuid::new_v4());
    let ordinary = here.command("thread.create", json!({"project":1}), uuid::Uuid::new_v4())["result"]["result"]["threadId"].clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let ambiguous = here.command("task.delegate", json!({"threadId":ordinary,"args":{
            "prompt":"Must not guess a checkout","target":{"environmentId":discovery["serverId"],"model":"fixture"}}}), uuid::Uuid::new_v4());
        assert_eq!(ambiguous["result"]["status"], "rejected", "{ambiguous}");
        let text = ambiguous.to_string();
        if text.contains("Several projects") {
            assert!(
                text.contains("remote-only") && text.contains("duplicate"),
                "{text}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "{ambiguous}");
    }
}

#[test]
fn registered_acp_catalog_reaches_the_picker_before_the_first_send() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("acp-catalog-before-send"),
    );
    assert!(hello["error"].is_null(), "{hello}");
    client.until(|state| {
        let catalog = &state["nativeCatalogs"]["fixture-acp"];
        catalog["ready"] == true && catalog["models"][0]["id"] == "fixture-model"
    });
    let catalog = &client.snapshot["nativeCatalogs"]["fixture-acp"];
    assert_eq!(catalog["name"], "Fixture ACP");
    assert_eq!(catalog["models"][0]["is_default"], true);
    let records: Vec<Value> = fs::read_to_string(profile.root.join("acp-log.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let setup = records
        .iter()
        .find(|record| record["method"] == "session/new")
        .unwrap();
    assert_eq!(setup["params"]["mcpServers"], json!([]));
    assert_eq!(setup["params"]["cwd"], json!(profile.root.join("Shina")));
    assert!(
        !records
            .iter()
            .any(|record| record["method"] == "session/prompt")
    );
    profile.stop();
}

#[test]
fn pi_and_acp_survive_disconnect_and_a_second_client_sees_the_same_state() {
    for harness in ["pi", "fixture-acp"] {
        let mut profile = Profile::new();
        profile.launch(&["serve"]);
        let device = uuid::Uuid::new_v4();
        let mut first = profile.client(device);
        if harness != "pi" {
            assert_eq!(
                first.command(
                    "thread.configure",
                    json!({"threadId":1,"harness":harness}),
                    uuid::Uuid::new_v4()
                )["result"]["status"],
                "accepted"
            );
        }
        let id = uuid::Uuid::new_v4();
        let args =
            json!({"threadId":1,"text":"disconnect-fixture","attachments":[],"mode":"prompt"});
        assert_eq!(
            first.command("thread.send", args.clone(), id)["result"]["status"],
            "accepted"
        );
        let mut second = profile.client(uuid::Uuid::new_v4());
        second.until(|state| {
            state["threads"][0]["messages"]
                .as_array()
                .is_some_and(|messages| messages.iter().any(|m| m["text"] == "disconnect-fixture"))
        });
        drop(first);
        drop(second);
        // No client exists during provider settlement.
        std::thread::sleep(Duration::from_millis(900));
        let mut reconnected = profile.client(device);
        reconnected.until(|state| state["threads"][0]["status"] == "Completed");
        assert!(
            reconnected.snapshot["threads"][0]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["text"]
                    == if harness == "pi" {
                        "Fixture complete"
                    } else {
                        "Hello ACP"
                    })
        );
        assert_eq!(
            reconnected.command("thread.send", args, id)["result"]["status"],
            "accepted"
        );
        let changed = reconnected.command(
            "thread.send",
            json!({"threadId":1,"text":"changed","attachments":[],"mode":"prompt"}),
            id,
        );
        assert!(
            changed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("different payload")
        );
        let database = rusqlite::Connection::open(profile.root.join("Shina/state.sqlite")).unwrap();
        assert_eq!(
            database
                .query_row(
                    "SELECT count(*) FROM command_receipts WHERE command_id=?1",
                    [id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        let user_count = reconnected.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["text"] == "disconnect-fixture")
            .count();
        assert_eq!(user_count, 1);
        profile.stop();
    }
}

#[test]
fn queued_edit_commits_in_place_and_rejects_stale_removed_and_started_items() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut first = profile.client(uuid::Uuid::new_v4());
    first.command(
        "thread.send",
        json!({"threadId":1,"text":"queue-wait-fixture","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    first.until(|state| {
        state["threads"][0]["initializing"] == false && state["threads"][0]["status"] == "Running"
    });
    first.command(
        "thread.send",
        json!({"threadId":1,"text":"Queued original","attachments":[],"mode":"queue"}),
        uuid::Uuid::new_v4(),
    );
    first.until(|state| {
        state["threads"][0]["queue"]
            .as_array()
            .is_some_and(|queue| queue.len() == 1)
    });
    let thread = &first.snapshot["threads"][0];
    let id = thread["queue"][0][0].clone();
    let generation = thread["generation"].clone();
    let draft = thread["queue_drafts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry[0] == id)
        .unwrap()[1]
        .clone();
    let revision =
        protocol::revision(&json!({"text":"Queued original","attachments":[],"draft":draft}));
    let edit = json!({"threadId":1,"inputId":id,"generation":generation,"expectedRevision":revision,
        "text":"Queued edited","attachments":[]});
    let command = uuid::Uuid::new_v4();
    assert_eq!(
        first.command("queue.update", edit.clone(), command)["result"]["status"],
        "accepted"
    );
    assert_eq!(
        first.command("queue.update", edit.clone(), command)["result"]["status"],
        "accepted",
        "a lost acknowledgement repeats the same receipt, not the mutation"
    );
    first.until(|state| state["threads"][0]["queue"][0][1] == "Queued edited");
    assert_eq!(first.snapshot["threads"][0]["queue"][0][0], id);
    assert_eq!(
        first.command("queue.update", edit.clone(), uuid::Uuid::new_v4())["result"]["status"],
        "rejected"
    );
    let mut second = profile.client(uuid::Uuid::new_v4());
    assert_eq!(
        second.command(
            "queue.remove",
            json!({"threadId":1,"inputId":id,"generation":generation}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    assert_eq!(
        first.command("queue.update", edit, uuid::Uuid::new_v4())["result"]["status"],
        "rejected"
    );
    let queued = first.command(
        "thread.send",
        json!({"threadId":1,"text":"Next queued","attachments":[],"mode":"queue"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(queued["result"]["status"], "accepted", "{queued}");
    // The prior edited item also made this queue length one. Command replies can
    // arrive before the subscription catches up, so wait for this commit too.
    first.until_sequence(queued["result"]["seq"].as_u64().unwrap(), |state| {
        state["threads"][0]["queue"]
            .as_array()
            .is_some_and(|queue| queue.len() == 1 && queue[0][1] == "Next queued")
    });
    let thread = &first.snapshot["threads"][0];
    let id = thread["queue"][0][0].clone();
    let draft = thread["queue_drafts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry[0] == id)
        .unwrap()[1]
        .clone();
    let revision =
        protocol::revision(&json!({"text":"Next queued","attachments":[],"draft":draft}));
    first.command(
        "thread.send",
        json!({"threadId":1,"text":"queue-release-fixture","attachments":[],"mode":"steer"}),
        uuid::Uuid::new_v4(),
    );
    first.until(|state| {
        state["threads"][0]["messages"]
            .as_array()
            .is_some_and(|messages| {
                messages
                    .iter()
                    .any(|message| message["text"] == "Next queued")
            })
    });
    assert_eq!(
        first.command(
            "queue.update",
            json!({"threadId":1,"inputId":id,"generation":generation,
        "expectedRevision":revision,"text":"Too late","attachments":[]}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "rejected"
    );
    profile.stop();
}

#[test]
fn stop_holds_the_server_queue_and_resume_continues_only_the_selected_interrupted_run() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.command(
        "thread.send",
        json!({"threadId":1,"text":"queue-wait-fixture","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    client.until(|state| {
        state["threads"][0]["initializing"] == false && state["threads"][0]["status"] == "Running"
    });
    let run = client.snapshot["threads"][0]["run_id"].clone();
    let generation = client.snapshot["threads"][0]["generation"].clone();
    assert!(run.is_string());
    for text in ["Queued A", "Queued B"] {
        assert_eq!(
            client.command(
                "thread.send",
                json!({"threadId":1,"text":text,"attachments":[],"mode":"queue"}),
                uuid::Uuid::new_v4()
            )["result"]["status"],
            "accepted"
        );
    }
    client.until(|state| {
        state["threads"][0]["queue"]
            .as_array()
            .is_some_and(|queue| queue.len() == 2)
    });
    assert_eq!(
        client.command(
            "thread.stop",
            json!({"threadId":1,"runId":run}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    client.until(|state| state["threads"][0]["status"] == "Ready");
    assert_eq!(client.snapshot["threads"][0]["queue_held"], true);
    assert_eq!(
        client.snapshot["threads"][0]["queue"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    drop(client);
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    assert_eq!(client.snapshot["threads"][0]["queue_held"], true);
    assert_eq!(client.snapshot["threads"][0]["resumable_run_id"], run);
    assert_eq!(
        client.snapshot["threads"][0]["queue"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        client.command(
            "thread.resume",
            json!({"threadId":1,"generation":generation,
        "continuationOfRunId":run,"releaseQueue":true}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "rejected"
    );
    let generation = client.snapshot["threads"][0]["generation"].clone();
    assert_eq!(
        client.command(
            "thread.resume",
            json!({"threadId":1,"generation":generation,
        "continuationOfRunId":"wrong-run","releaseQueue":true}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "rejected"
    );
    let resume = uuid::Uuid::new_v4();
    let args =
        json!({"threadId":1,"generation":generation,"continuationOfRunId":run,"releaseQueue":true});
    assert_eq!(
        client.command("thread.resume", args.clone(), resume)["result"]["status"],
        "accepted"
    );
    assert_eq!(
        client.command(
            "thread.stop",
            json!({"threadId":1,"runId":run}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "rejected",
        "a stop captured before resume must not interrupt its new run"
    );
    client.until(|state| {
        state["threads"][0]["status"] == "Completed"
            && state["threads"][0]["messages"]
                .as_array()
                .is_some_and(|messages| {
                    messages.iter().any(|message| message["text"] == "Queued B")
                })
    });
    assert_eq!(
        client.command("thread.resume", args, resume)["result"]["status"],
        "accepted"
    );
    let workspace = profile.root.join("Shina/chats/1");
    let prompts: Vec<String> = fs::read_to_string(workspace.join("fixture-prompts.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        prompts,
        [
            "queue-wait-fixture",
            "Continue where you left off.",
            "Queued A",
            "Queued B"
        ]
    );
    profile.stop();
}

/// A declared steer reaches the child harness as streamingBehavior:"steer"
/// over the live MCP wire, while an ordinary send goes as followUp. Steering
/// without an observed attempt, or against a finished attempt, is refused.
#[test]
fn steered_task_send_reaches_the_child_harness_as_steer_on_the_wire() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let workspace = profile.root.join("Shina/chats/1");
    assert_eq!(
        client.command(
            "thread.send",
            json!({"threadId":1,"text":"Initialize the coordinator fixture","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    client.until(|state| state["threads"][0]["status"] == "Completed");

    // The hold marker keeps the child's turn active with no pending request,
    // so a declared steer has a live run to pin.
    let delegated = call_fixture_tool(
        &workspace,
        "delegate_task",
        json!({"prompt":"peer-documents-hold-fixture","title":"Steerable child",
            "client_request_id":"steer-delegate"}),
    );
    assert_ne!(delegated["result"]["isError"], true, "{delegated}");
    let child =
        serde_json::from_str::<Value>(delegated["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap()["thread_id"]
            .as_u64()
            .unwrap();
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .is_some_and(|tasks| !tasks.is_empty() && tasks[0]["state"] == "running")
    });
    let attempt = client.snapshot["index"]["tasks"][0]["attempt_id"].clone();
    let child_directory = profile
        .root
        .join(format!("Shina/chats/{child}/fixture-behavior.jsonl"));

    // Steering without the observed attempt is refused before any dispatch.
    let unfenced = client.command(
        "task.send",
        json!({"threadId":1,"args":{"thread_id":child,"message":"steer-this-turn","client_request_id":"steer-0","delivery":"steer"}}),
        uuid::Uuid::new_v4(),
    );
    assert!(unfenced.to_string().contains("attempt_id"), "{unfenced}");

    // The fenced steer interrupts the live run on the wire.
    let steered = client.command(
        "task.send",
        json!({"threadId":1,"args":{"thread_id":child,"message":"steer-this-turn","client_request_id":"steer-1","delivery":"steer","attempt_id":attempt}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(steered["result"]["status"], "accepted", "{steered}");

    // An ordinary keyed send stays follow-up input while the run is live.
    let follow_up = client.command(
        "task.send",
        json!({"threadId":1,"args":{"thread_id":child,"message":"queue-behind","client_request_id":"send-1"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(follow_up["result"]["status"], "accepted", "{follow_up}");

    let deadline = Instant::now() + Duration::from_secs(15);
    let records = loop {
        if let Ok(text) = fs::read_to_string(&child_directory) {
            let records: Vec<Value> = text
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            if records.iter().any(|record| {
                record["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("steer-this-turn"))
            }) && records.iter().any(|record| {
                record["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("queue-behind"))
            }) {
                break records;
            }
        }
        assert!(
            Instant::now() < deadline,
            "child wire log at {child_directory:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let steered_prompt = records
        .iter()
        .find(|record| {
            record["message"]
                .as_str()
                .is_some_and(|m| m.contains("steer-this-turn"))
        })
        .unwrap();
    assert_eq!(steered_prompt["streamingBehavior"], "steer");
    let queued_prompt = records
        .iter()
        .find(|record| {
            record["message"]
                .as_str()
                .is_some_and(|m| m.contains("queue-behind"))
        })
        .unwrap();
    assert_eq!(queued_prompt["streamingBehavior"], "followUp");

    // Once the attempt finishes, steering is refused outright instead of
    // silently queueing as input the caller never marked as follow-up.
    client.until(|state| state["index"]["tasks"][0]["state"] == "completed");
    let finished = client.command(
        "task.send",
        json!({"threadId":1,"args":{"thread_id":child,"message":"steer-after-end","client_request_id":"steer-2","delivery":"steer","attempt_id":attempt}}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        finished.to_string().contains("cannot accept steering"),
        "{finished}"
    );
    profile.stop();
}

#[test]
fn receipts_and_server_transcripts_survive_restart_without_replaying_a_send() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let id = uuid::Uuid::new_v4();
    let args = json!({"threadId":1,"text":"persisted","attachments":[],"mode":"prompt"});
    let mut client = profile.client(device);
    let first = client.command("thread.send", args.clone(), id);
    client.until(|state| state["threads"][0]["status"] == "Completed");
    let epoch = client.snapshot["index"]["serverEpoch"].clone();
    profile.stop();
    profile.launch(&["serve"]);
    let mut resumed = profile.client(device);
    assert_eq!(resumed.snapshot["index"]["serverEpoch"], epoch);
    assert_eq!(
        resumed.command("thread.send", args, id)["result"],
        first["result"]
    );
    assert!(
        resumed.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["text"] == "persisted")
    );
    profile.stop();
}

#[test]
fn authentication_origins_and_major_versions_refuse_commands() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let discovery = profile.discovery();
    for (origin, expected) in [(false, "401"), (true, "403")] {
        let mut stream =
            TcpStream::connect(("127.0.0.1", discovery["port"].as_u64().unwrap() as u16)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream,"GET /v1/ws HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{}Authorization: Bearer invalid\r\n\r\n",if origin {"Origin: https://example.invalid\r\n"} else {""}).unwrap();
        let mut reply = [0u8; 256];
        let count = stream.read(&mut reply).unwrap();
        assert!(
            String::from_utf8_lossy(&reply[..count]).starts_with(&format!("HTTP/1.1 {expected}"))
        );
    }
    let mut client = profile.client(uuid::Uuid::new_v4());
    let mismatch = client.rpc(
        "hello",
        json!({"protocolMajor":99,"projectionSchema":1}),
        json!("bad-version"),
    );
    assert!(
        mismatch["error"]["message"]
            .as_str()
            .unwrap()
            .contains("mismatch")
    );
    let refused = client.command(
        "thread.rename",
        json!({"threadId":1,"title":"refused"}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("hello")
    );
    profile.stop();
}

#[test]
fn service_restart_keeps_sqlite_open_and_retains_atomic_receipts_and_backups() {
    let mut profile = Profile::new();
    let directory = profile.root.join("Shina");
    let descriptor = directory.join("service.json");
    fs::write(&descriptor, json!({
        "platform":if cfg!(windows) { "Windows" } else if cfg!(target_os="macos") { "Mac" } else { "Linux" },
        "name":"Shina-integration-only","executable":env!("CARGO_BIN_EXE_shina"),
        "directory":directory,"unit":directory.join("unused-unit"),"home":profile.root,"environment":{}
    }).to_string()).unwrap();
    // Exercise the real launcher without registering any operating-system service.
    let launcher = profile.launch(&["--service-run", descriptor.to_str().unwrap()]);
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":2,"projectionSchema":1,"maintenanceClient":true}),
        json!("maintenance"),
    );
    let id = uuid::Uuid::new_v4();
    let args = json!({"threadId":1,"title":"saved across service restart"});
    let receipt = client.command("thread.rename", args.clone(), id);
    assert_eq!(receipt["result"]["status"], "accepted");
    let database = rusqlite::Connection::open_with_flags(
        directory.join("state.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let prepared =
        client.command("server.prepareRestart", json!({}), uuid::Uuid::new_v4())["result"].clone();
    assert_eq!(prepared["phase"], "draining");
    assert_eq!(
        client.command("server.commitRestart", prepared, uuid::Uuid::new_v4())["result"]["phase"],
        "restarting"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while profile.children[launcher].try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "service launcher did not exit");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        profile.children[launcher]
            .try_wait()
            .unwrap()
            .unwrap()
            .success()
    );
    drop(client);
    assert_eq!(
        database
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let stored: String = database
        .query_row(
            "SELECT payload FROM command_receipts WHERE command_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap(),
        receipt["result"]
    );
    let consistent: bool = database
        .query_row(
            "SELECT p.seq=(SELECT max(seq) FROM events) FROM projection p WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(consistent);
    // Force the same pre-open backup hook used when an update changes binary version.
    fs::write(
        directory.join("server-binary-version.json"),
        "previous-version",
    )
    .unwrap();
    profile.launch(&["--service-run", descriptor.to_str().unwrap()]);
    let mut resumed = profile.client(device);
    assert_eq!(
        resumed.command("thread.rename", args, id)["result"],
        receipt["result"]
    );
    assert_eq!(
        resumed.snapshot["index"]["threads"][0]["title"],
        "saved across service restart"
    );
    let saved = fs::read_dir(directory.join("backups"))
        .unwrap()
        .flatten()
        .any(|entry| {
            let path = entry.path().join("state.sqlite");
            if !path.is_file() {
                return false;
            }
            let backup = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            backup
                .query_row(
                    "SELECT count(*) FROM command_receipts WHERE command_id=?1",
                    [id.to_string()],
                    |r| r.get::<_, i64>(0),
                )
                .ok()
                == Some(1)
        });
    assert!(saved, "pre-open backup did not retain the durable receipt");
    drop(database);
    #[cfg(windows)]
    {
        // Uses authenticated IPC without installing or ending any scheduled task.
        let stopping = profile.launch(&["service", "stop"]);
        let deadline = Instant::now() + Duration::from_secs(10);
        while profile.children[stopping].try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "service stop did not finish");
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            profile.children[stopping]
                .try_wait()
                .unwrap()
                .unwrap()
                .success()
        );
    }
    profile.stop();
}

fn read_full_history(client: &mut Client, budget: usize) -> Value {
    read_thread_history(client, 1, budget)
}

fn read_thread_history(client: &mut Client, thread: u64, budget: usize) -> Value {
    let mut history_cursor = Value::Null;
    let mut rows: Vec<history::Row> = Vec::new();
    let mut fields;
    loop {
        let mut text = String::new();
        let mut fragment = Value::Null;
        loop {
            let reply = client.rpc(
                "thread.history",
                json!({"threadId":thread,"historyCursor":history_cursor,
                "fragmentCursor":fragment,"byteBudget":budget}),
                json!("verify-full"),
            );
            assert!(reply["error"].is_null(), "{reply}");
            assert!(reply["result"]["data"].as_str().unwrap().len() <= budget);
            text.push_str(reply["result"]["data"].as_str().unwrap());
            fragment = reply["result"]["nextFragment"].clone();
            if fragment.is_null() {
                break;
            }
        }
        let page: Value = serde_json::from_str(&text).unwrap();
        rows.extend(serde_json::from_value::<Vec<history::Row>>(page["items"].clone()).unwrap());
        fields = page["fields"].clone();
        history_cursor = page["nextCursor"].clone();
        if history_cursor.is_null() {
            break;
        }
    }
    for field in protocol::HISTORY_FIELDS {
        if fields.get(field).is_none() {
            fields[field] = json!([]);
        }
    }
    rows.sort_by(|a, b| a.field.cmp(&b.field).then(a.ordinal.cmp(&b.ordinal)));
    for row in rows {
        fields[&row.field].as_array_mut().unwrap().push(row.item);
    }
    fields
}

#[test]
fn durable_credentials_survive_restart_and_revoked_sockets_cannot_mutate() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let victim_id = uuid::Uuid::new_v4();
    let mut victim = profile.client(victim_id);
    let token = victim.token.clone();
    profile.stop();
    profile.launch(&["serve"]);
    let discovery = profile.discovery();
    let mut request = format!(
        "ws://127.0.0.1:{}/v1/ws",
        discovery["port"].as_u64().unwrap()
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (socket, _) = tungstenite::connect(request.clone()).unwrap();
    victim.socket = socket;
    victim.rpc(
        "hello",
        json!({"protocolMajor":1,"projectionSchema":1}),
        json!("restarted-hello"),
    );
    let mut admin = profile.client(uuid::Uuid::new_v4());
    assert!(
        admin.rpc("devices.list", json!({}), json!("devices"))["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["deviceId"] == victim_id.to_string())
    );
    assert_eq!(
        admin.command(
            "devices.revoke",
            json!({"deviceId":victim_id}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    assert!(tungstenite::connect(request).is_err());
    victim
        .socket
        .send(Message::Text(
            json!({"jsonrpc":"2.0","id":"revoked","method":"server.status","params":{}})
                .to_string()
                .into(),
        ))
        .ok();
    assert!(!matches!(victim.socket.read(), Ok(Message::Text(_))));
    profile.stop();
}

#[test]
fn older_history_paging_loads_thousands_of_items_on_demand_and_across_reconnect() {
    let mut template = Profile::new();
    template.launch(&["serve"]);
    let base = template.client(uuid::Uuid::new_v4()).snapshot.clone();
    template.stop();

    let mut profile = Profile::new();
    // Seed a realistic large thread (> 2500 items: 900 turns * 3 items = 2700 items).
    let mut messages = Vec::new();
    let mut tools = Vec::new();
    for turn in 0..900usize {
        messages.push(json!({"id":format!("u-{turn}"),"role":"You","text":format!("Turn prompt {turn}"),"images":[]}));
        let a_idx = messages.len();
        messages.push(json!({"id":format!("a-{turn}"),"role":"Pi","text":format!("Turn reply {turn}"),"images":[]}));
        tools.push(json!({
            "id":format!("tool-{turn}"),"name":"read","detail":"file.rs","done":true,"failed":false,
            "output":"ok","images":[],"structured":null,"parent_call":null,"exit_code":0,"subagents":[],
            "anchor":{"message":a_idx,"offset":0},"block":0,"order":1
        }));
    }
    let mut index = base["index"].clone();
    index["threads"][0]["title"] = json!("Huge thread");
    index["threads"][0]["renamed"] = json!(true);
    index["threads"][0]["session"] = Value::Null;
    let mut view = base["threads"][0].clone();
    view["status"] = json!("Completed");
    view["messages"] = json!(messages);
    view["tools"] = json!(tools);
    index["threadViews"] = json!([view]);
    fs::write(profile.root.join("Shina/index.json"), index.to_string()).unwrap();

    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("hello"),
    );
    let mut cache = client_history::ProjectionCache::default();
    cache.reconnect(&hello["result"]).unwrap();
    client.rpc("workspace.subscribe", json!({}), json!("sub"));
    cache
        .receive(&protocol::notification(
            "subscription.snapshot",
            json!({"state":protocol::compact(&client.snapshot),"seq":client.seq,"epoch":client.epoch}),
        ))
        .unwrap();
    client.history_cache = Some(cache);

    // 1. Initial thread.subscribe loads only the recent window, not all 2700 items.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            Instant::now() < deadline,
            "initial subscribe did not finish"
        );
        let cache = client.history_cache.as_mut().unwrap();
        if let Some(req) = cache.next_history() {
            client.rpc(
                req["method"].as_str().unwrap(),
                req["params"].clone(),
                json!("history"),
            );
        } else if cache.assembled(1).is_some() {
            break;
        } else {
            client.read();
        }
    }
    {
        let cache = client.history_cache.as_mut().unwrap();
        assert!(
            cache.next_history().is_none(),
            "must stop after initial window"
        );
        let view = cache.projected().unwrap();
        assert_eq!(view["threads"][0]["historyMeta"]["hasMoreHistory"], true);
        let loaded_msgs = &view.views[&1].fields["messages"].items;
        assert!(
            loaded_msgs.len() <= 20,
            "recent budget only, got {}",
            loaded_msgs.len()
        );
        assert_eq!(loaded_msgs.last().unwrap()["text"], "Turn reply 899");
    }

    // 2. Request earlier pages on demand, including across a reconnect mid-walk.
    let mut pages = 0usize;
    let mut reconnected = false;
    loop {
        assert!(Instant::now() < deadline, "older paging did not finish");
        let cache = client.history_cache.as_mut().unwrap();
        if let Some(req) = cache.next_history() {
            client.rpc(
                req["method"].as_str().unwrap(),
                req["params"].clone(),
                json!("history"),
            );
            continue;
        }
        let meta = cache
            .projected()
            .map(|v| v["threads"][0]["historyMeta"].clone());
        if meta.as_ref().is_some_and(|m| m["hasMoreHistory"] == false) || cache.complete(1) {
            break;
        }
        if pages == 2 && !reconnected {
            let saved = client.history_cache.take().unwrap();
            let token = client.token.clone();
            drop(client);
            client = Client::unsubscribed(&profile.discovery(), token);
            client.snapshot = saved.state.clone();
            client.seq = saved.seq.unwrap();
            client.epoch = saved.epoch.clone();
            let hello = client.rpc(
                "hello",
                json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
                json!("reconnect-hello"),
            );
            let mut saved = saved;
            saved.reconnect(&hello["result"]).unwrap();
            let cursor = json!({"lastSeq":saved.seq,"epoch":saved.epoch});
            client.history_cache = Some(saved);
            client.rpc("workspace.subscribe", cursor, json!("reconnect-sub"));
            reconnected = true;
            // After reconnect, synchronize thread.subscribe first before requesting next page.
            let sub = client
                .history_cache
                .as_mut()
                .unwrap()
                .next_history()
                .unwrap();
            assert_eq!(sub["method"], "thread.subscribe");
            client.rpc("thread.subscribe", sub["params"].clone(), json!("history"));
        }
        client
            .history_cache
            .as_mut()
            .unwrap()
            .request_earlier(1, None);
        pages += 1;
    }
    assert!(
        pages > 5,
        "should require multiple older pages, took {pages}"
    );
    let cache = client.history_cache.as_ref().unwrap();
    assert_eq!(
        cache.assembled(1).unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        1800
    );
    assert_eq!(
        cache.assembled(1).unwrap()["tools"]
            .as_array()
            .unwrap()
            .len(),
        900
    );
    assert_eq!(
        cache.assembled(1).unwrap()["messages"][0]["text"],
        "Turn prompt 0"
    );
    assert!(cache.complete(1));

    // 3. Peer lacking history-items (protocolMinor 6) yields explicit update-required state.
    let mut legacy = client_history::ProjectionCache::default();
    assert!(
        legacy
            .reconnect(&json!({"capabilities":["snapshot-subscriptions"]}))
            .is_err()
    );
    profile.stop();
}

#[test]
fn server_backed_find_searches_thousands_of_items_pages_to_match_works_on_linked_machine_and_handles_old_peer()
 {
    let mut template = Profile::new();
    template.launch(&["serve"]);
    let base = template.client(uuid::Uuid::new_v4()).snapshot.clone();
    template.stop();

    let mut host_a = Profile::new();
    let mut host_b = Profile::new();

    // Seed 900 turns (2700 items) on host_b with an early unique needle at turn 3.
    let mut messages = Vec::new();
    let mut tools = Vec::new();
    for turn in 0..900usize {
        let prompt = if turn == 3 {
            "Turn prompt 3 contains secret-albatross-needle inside".to_owned()
        } else if turn == 895 {
            "Turn prompt 895 also has secret-albatross-needle".to_owned()
        } else {
            format!("Turn prompt {turn}")
        };
        messages.push(json!({"id":format!("u-{turn}"),"role":"You","text":prompt,"images":[]}));
        let a_idx = messages.len();
        messages.push(json!({"id":format!("a-{turn}"),"role":"Pi","text":format!("Turn reply {turn}"),"images":[]}));
        tools.push(json!({
            "id":format!("tool-{turn}"),"name":"read","detail":"file.rs","done":true,"failed":false,
            "output":"ok","images":[],"structured":null,"parent_call":null,"exit_code":0,"subagents":[],
            "anchor":{"message":a_idx,"offset":0},"block":0,"order":1
        }));
        if turn == 3 {
            let mut plan = tools.last().unwrap().clone();
            plan["id"] = json!("old-plan");
            plan["name"] = json!("plan");
            plan["kind"] = json!("plan");
            plan["input"] = json!({"markdown":"# Stored proposal\n\n## Summary\n\nA deepplanneedle in a stored plan."});
            plan["order"] = json!(2);
            tools.push(plan);
        }
    }
    let mut index = base["index"].clone();
    index["threads"][0]["title"] = json!("Long searchable thread");
    index["threads"][0]["renamed"] = json!(true);
    index["threads"][0]["session"] = Value::Null;
    let mut view = base["threads"][0].clone();
    view["status"] = json!("Completed");
    view["messages"] = json!(messages);
    view["tools"] = json!(tools);
    index["threadViews"] = json!([view]);
    fs::write(host_b.root.join("Shina/index.json"), index.to_string()).unwrap();

    host_a.launch(&["serve"]);
    host_b.launch(&["serve"]);
    let discovery_b = host_b.discovery();
    let paired = host_b.cli(&["pair"]);
    assert!(paired.status.success());
    let code = String::from_utf8(paired.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let host = format!("127.0.0.1:{}", discovery_b["port"].as_u64().unwrap());
    assert!(host_a.cli(&["link", &host, &code]).status.success());

    let credential: Value = serde_json::from_slice(
        &fs::read(
            host_a
                .root
                .join("Shina/linked")
                .join(discovery_b["serverId"].as_str().unwrap())
                .join("credential.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut linked_client =
        Client::with_token(&discovery_b, credential["token"].as_str().unwrap().into());
    let hello = linked_client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("linked-hello"),
    );
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == "thread-find")
    );

    // Search the complete stored conversation on linked machine host_b.
    let found = linked_client.rpc(
        "thread.find",
        json!({"threadId":1,"query":"secret-albatross-needle"}),
        json!("find-1"),
    );
    assert!(found["error"].is_null(), "{found}");
    assert_eq!(found["result"]["totalMatches"], 2);
    assert_eq!(found["result"]["activeIndex"], 0);
    assert_eq!(found["result"]["match"]["entryId"], "u-3");

    // Step Next -> advances to u-895.
    let next = linked_client.rpc(
        "thread.find",
        json!({"threadId":1,"query":"secret-albatross-needle","start":{"entryId":"u-3","occurrence":0},"offset":1}),
        json!("find-2"),
    );
    assert_eq!(next["result"]["activeIndex"], 1);
    assert_eq!(next["result"]["match"]["entryId"], "u-895");
    let plan = linked_client.rpc(
        "thread.find",
        json!({"threadId":1,"query":"deepplanneedle"}),
        json!("find-plan"),
    );
    assert_eq!(plan["result"]["totalMatches"], 1, "{plan}");
    assert_eq!(plan["result"]["match"]["entryId"], "old-plan");

    // Load initial scoped window via ProjectionCache, then jump to earlier match u-3!
    let mut cache = client_history::ProjectionCache::default();
    cache.reconnect(&hello["result"]).unwrap();
    linked_client.rpc("workspace.subscribe", json!({}), json!("sub"));
    cache
        .receive(&protocol::notification(
            "subscription.snapshot",
            json!({"state":protocol::compact(&linked_client.snapshot),"seq":linked_client.seq,"epoch":linked_client.epoch}),
        ))
        .unwrap();
    linked_client.history_cache = Some(cache);
    loop {
        let cache = linked_client.history_cache.as_mut().unwrap();
        if let Some(req) = cache.next_history() {
            linked_client.rpc(
                req["method"].as_str().unwrap(),
                req["params"].clone(),
                json!("history"),
            );
        } else if cache.assembled(1).is_some() {
            break;
        }
    }
    // u-3 is NOT in initial window; requesting earlier throughEntryId="u-3" loads it!
    {
        let cache = linked_client.history_cache.as_mut().unwrap();
        assert!(
            !cache.assembled(1).unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["id"] == "u-3")
        );
        cache.request_earlier(1, Some("u-3".into()));
    }
    loop {
        let cache = linked_client.history_cache.as_mut().unwrap();
        if let Some(req) = cache.next_history() {
            linked_client.rpc(
                req["method"].as_str().unwrap(),
                req["params"].clone(),
                json!("history"),
            );
        } else {
            break;
        }
    }
    assert!(
        linked_client
            .history_cache
            .as_ref()
            .unwrap()
            .assembled(1)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "u-3")
    );
    assert!(
        linked_client
            .history_cache
            .as_ref()
            .unwrap()
            .assembled(1)
            .unwrap()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["id"] == "old-plan")
    );
    let mut reconnected =
        Client::with_token(&discovery_b, credential["token"].as_str().unwrap().into());
    reconnected.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("reconnect"),
    );
    let plan = reconnected.rpc(
        "thread.find",
        json!({"threadId":1,"query":"Stored proposal"}),
        json!("plan-title"),
    );
    assert_eq!(plan["result"]["match"]["entryId"], "old-plan", "{plan}");

    // Peer without thread-find (protocolMinor 9) does not advertise "thread-find".
    let mut old_client =
        Client::with_token(&discovery_b, credential["token"].as_str().unwrap().into());
    let old_hello = old_client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":9,"projectionSchema":1}),
        json!("old-hello"),
    );
    assert!(
        !old_hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == "thread-find")
    );

    host_a.stop();
    host_b.stop();
}

#[test]
fn sidebar_search_queries_full_transcripts_across_local_and_linked_servers_and_notes_old_peer() {
    let mut template = Profile::new();
    template.launch(&["serve"]);
    let base = template.client(uuid::Uuid::new_v4()).snapshot.clone();
    template.stop();

    let mut host_a = Profile::new();
    let mut host_b = Profile::new();

    // Seed unvisited thread 2 on host_a with hidden early message, and unvisited thread 2 on host_b.
    let make_index = |phrase: &str| {
        let mut messages = Vec::new();
        for turn in 0..80usize {
            let prompt = if turn == 4 {
                format!("Early turn with {phrase} inside")
            } else {
                format!("Normal turn {turn}")
            };
            messages.push(json!({"id":format!("u-{turn}"),"role":"You","text":prompt,"images":[]}));
            messages.push(json!({"id":format!("a-{turn}"),"role":"Pi","text":format!("Reply {turn}"),"images":[]}));
        }
        let mut index = base["index"].clone();
        let mut t2 = index["threads"][0].clone();
        t2["id"] = json!(2);
        t2["title"] = json!("Unrelated title");
        t2["renamed"] = json!(true);
        t2["session"] = Value::Null;
        index["threads"] = json!([index["threads"][0], t2]);
        index["nextId"] = json!(3);
        let mut v1 = base["threads"][0].clone();
        v1["status"] = json!("Completed");
        let mut v2 = base["threads"][0].clone();
        v2["id"] = json!(2);
        v2["status"] = json!("Completed");
        v2["messages"] = json!(messages);
        index["threadViews"] = json!([v1, v2]);
        reset_test_thread_identities(&mut index, [1, 2]);
        index
    };
    fs::write(
        host_a.root.join("Shina/index.json"),
        make_index("periwinkle-compass").to_string(),
    )
    .unwrap();
    fs::write(
        host_b.root.join("Shina/index.json"),
        make_index("cobalt-astrolabe").to_string(),
    )
    .unwrap();

    host_a.launch(&["serve"]);
    host_b.launch(&["serve"]);
    let discovery_b = host_b.discovery();
    let paired = host_b.cli(&["pair"]);
    assert!(paired.status.success());
    let code = String::from_utf8(paired.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let host = format!("127.0.0.1:{}", discovery_b["port"].as_u64().unwrap());
    assert!(host_a.cli(&["link", &host, &code]).status.success());

    let mut client_a = host_a.client(uuid::Uuid::new_v4());
    let hello_a = client_a.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("hello-a"),
    );
    assert!(
        hello_a["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == "thread-search")
    );
    client_a.rpc("workspace.subscribe", json!({}), json!("sub-a"));
    // Unvisited thread 2 has empty messages in compact workspace snapshot.
    assert!(
        client_a.snapshot["threads"][1]["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let hits_a = client_a.rpc(
        "threads.search",
        json!({"query":"periwinkle-compass"}),
        json!("search-a"),
    );
    assert!(hits_a["error"].is_null(), "{hits_a}");
    let matches_a = hits_a["result"]["matches"].as_array().unwrap();
    assert_eq!(matches_a.len(), 1);
    assert_eq!(matches_a[0]["threadId"], 2);
    assert_eq!(matches_a[0]["entryId"], "u-4");
    assert!(
        matches_a[0]["snippet"]
            .as_str()
            .unwrap()
            .contains("periwinkle-compass")
    );

    let credential: Value = serde_json::from_slice(
        &fs::read(
            host_a
                .root
                .join("Shina/linked")
                .join(discovery_b["serverId"].as_str().unwrap())
                .join("credential.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut client_b =
        Client::with_token(&discovery_b, credential["token"].as_str().unwrap().into());
    client_b.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("hello-b"),
    );
    let hits_b = client_b.rpc(
        "threads.search",
        json!({"query":"cobalt-astrolabe"}),
        json!("search-b"),
    );
    assert!(hits_b["error"].is_null(), "{hits_b}");
    let matches_b = hits_b["result"]["matches"].as_array().unwrap();
    assert_eq!(matches_b.len(), 1);
    assert_eq!(matches_b[0]["threadId"], 2);

    // Peer lacking thread-search (protocolMinor 9) does not advertise capability.
    let mut old_client =
        Client::with_token(&discovery_b, credential["token"].as_str().unwrap().into());
    let old_hello = old_client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":9,"projectionSchema":1}),
        json!("old-hello"),
    );
    assert!(
        !old_hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == "thread-search")
    );

    host_a.stop();
    host_b.stop();
}

#[path = "support/mobile_contract.rs"]
mod mobile_contract;

#[path = "support/mobile_sdk_runtime.rs"]
mod mobile_sdk_runtime;

#[path = "support/captain_contract.rs"]
mod captain_contract;

#[path = "support/repository_contract.rs"]
mod repository_contract;

#[path = "support/admission_contract.rs"]
mod admission_contract;

#[path = "support/update_recovery.rs"]
mod update_recovery;

#[path = "support/capability_parity.rs"]
mod capability_parity;

#[test]
fn task_plans_are_negotiated_and_drive_the_mcp_plan_reducer() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let prepare = json!({"threadId":1,"args":{"plan":"p","node":"a","prompt":"Map the code."}});
    for minor in [protocol::TASK_PLANS_MINOR - 1, protocol::MINOR] {
        let hello = client.rpc(
            "hello",
            json!({"protocolMajor":1,"protocolMinor":minor,"projectionSchema":1}),
            json!(format!("task-plans-{minor}")),
        );
        let negotiated = hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == protocol::TASK_PLANS);
        assert_eq!(negotiated, minor >= protocol::TASK_PLANS_MINOR);
        let result = client.command("task.plan.prepare", prepare.clone(), uuid::Uuid::new_v4());
        if !negotiated {
            assert_eq!(
                result["error"]["message"],
                protocol::TASK_PLANS_UPDATE_REQUIRED,
                "{result}"
            );
            continue;
        }
        assert_eq!(result["result"]["status"], "accepted", "{result}");
        assert_eq!(result["result"]["result"]["revision"], 1);
        // The read returns the same plan value the prepare reply carried.
        let read = client.command(
            "task.plan",
            json!({"threadId":1,"args":{"plan":"p"}}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(read["result"]["status"], "accepted", "{read}");
        assert_eq!(
            read["result"]["result"]["plan"]["nodes"][0],
            result["result"]["result"]["plan"]["nodes"][0]
        );
        let stale = client.command(
            "task.plan.update",
            json!({"threadId":1,"args":{"plan":"p","node":"a","expected_revision":9,"title":"x"}}),
            uuid::Uuid::new_v4(),
        );
        assert!(
            stale.to_string().contains("changed since revision 9"),
            "{stale}"
        );
    }
    profile.stop();
}
