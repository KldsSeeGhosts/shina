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
    time::{Duration, Instant},
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
fn peer_task_evidence_requires_its_contract_minor_not_an_identity_number() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let capability = |client: &mut Client, minor: u32| {
        client.rpc(
            "hello",
            json!({"protocolMajor":1,"protocolMinor":minor,"projectionSchema":1}),
            json!(format!("peer-evidence-{minor}")),
        )["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability == protocol::PEER_TASK_EVIDENCE)
    };
    assert!(!capability(&mut client, protocol::IDENTITY_MINOR));
    assert!(!capability(
        &mut client,
        protocol::PEER_TASK_EVIDENCE_MINOR - 1
    ));
    assert!(capability(&mut client, protocol::PEER_TASK_EVIDENCE_MINOR));
    profile.stop();
}

#[test]
fn serve_migrates_legacy_identity_and_reopens_with_the_same_logical_ids() {
    let mut profile = Profile::new();
    let legacy = json!({
        "version":4,
        "captains":[{
            "name":"Legacy Captain","role":"Fixture","instructions":"Preserve identity",
            "portrait":0
        }],
        "captainScopes":{"0":[]},
        "captainRemoteScopes":{},
        "captainHomes":{"0":1},
        "captainBooks":{},
        "captainLifecycles":{"0":{"version":1,"phase":"active"}},
        "captainReportPolicies":{},
        "captainReports":[],
        "captainQuestionAnswers":[],
        "projects":[],
        "threads":[{
            "id":1,"title":"Legacy home","captain":0,"captainHome":true,
            "project":null,"machine":"local","delegated":false,"parent":null,"session":null,
            "lifecycle":{"createdAt":1,"updatedAt":1,"lastVisitedAt":1}
        }],
        "tasks":[],
        "nextId":2,"nextProjectId":1,
        "active":1,"tabs":[1]
    });
    fs::write(profile.root.join("Shina/index.json"), legacy.to_string()).unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("identity-migration-hello"),
    );
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability == "identity-v1")
    );
    let identity = client.snapshot["index"]["identity"].clone();
    let captain_uid = identity["captainIds"]["0"].clone();
    let thread_uid = identity["threadIds"]["1"].clone();
    assert!(captain_uid.is_string());
    assert!(thread_uid.is_string());
    assert_eq!(identity["workspaceId"], hello["result"]["workspaceId"]);
    assert_eq!(identity["ownerId"], hello["result"]["ownerId"]);
    assert_eq!(identity["controllerId"], hello["result"]["controllerId"]);
    assert!(
        fs::read_dir(profile.root.join("Shina/backups"))
            .unwrap()
            .flatten()
            .any(|entry| entry.path().join("state.sqlite").is_file())
    );

    profile.stop();
    profile.launch(&["serve"]);
    let reopened = profile.client(uuid::Uuid::new_v4());
    let reopened_identity = reopened.snapshot["index"]["identity"].clone();
    assert_eq!(reopened_identity["workspaceId"], identity["workspaceId"]);
    assert_eq!(reopened_identity["ownerId"], identity["ownerId"]);
    assert_eq!(reopened_identity["captainIds"]["0"], captain_uid);
    assert_eq!(reopened_identity["threadIds"]["1"], thread_uid);
}

#[test]
fn composer_request_capability_refuses_new_decisions_on_an_older_connection() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    for minor in [14, protocol::MINOR] {
        let hello = client.rpc(
            "hello",
            json!({"protocolMajor":1,"protocolMinor":minor,"projectionSchema":1}),
            json!(format!("request-capability-{minor}")),
        );
        assert_eq!(
            hello["result"]["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c == protocol::COMPOSER_REQUESTS),
            minor >= 15
        );
        let response = client.command(
            "request.respond",
            json!({"threadId":1,"requestId":"expired","generation":"expired",
                "requestContract":true,"response":{"decision":"cancel"}}),
            uuid::Uuid::new_v4(),
        );
        if minor < 15 {
            assert_eq!(
                response["error"]["message"],
                protocol::REQUESTS_UPDATE_REQUIRED
            );
        } else {
            assert!(
                response.to_string().contains("Request expired."),
                "{response}"
            );
        }
    }
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
fn choice_only_questions_use_native_option_ids_and_refuse_an_older_peer() {
    let mut profile = Profile::new();
    fs::write(profile.root.join("Shina/harnesses.json"), json!([{
        "id":"antigravity-fixture","name":"Antigravity fixture",
        "command":if cfg!(windows) {"python"} else {"python3"},
        "args":["-u",profile.root.join("fake_acp.py"),"--log",profile.root.join("choice-log.jsonl")]
    }]).to_string()).unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("questions-hello"),
    );
    assert_eq!(
        client.command(
            "thread.configure",
            json!({"threadId":1,"harness":"antigravity-fixture"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    assert_eq!(
        client.command(
            "thread.send",
            json!({"threadId":1,"text":"choice-only","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    client.until(|state| state["threads"][0]["request"]["method"] == "input");
    let request = client.snapshot["threads"][0]["request"].clone();
    assert_eq!(request["questions"][0]["allowCustomAnswer"], false);
    assert_eq!(request["questions"][0]["options"][1]["value"], "choice-2");
    let answer = json!({"threadId":1,"requestId":request["id"],"generation":request["runtimeGeneration"],
        "requestContract":true,"questionContract":true,
        "answers":[{"selected":["choice-2"],"other":"Ignore this unoffered custom answer."}],
        "response":{"value":"{}"}});
    let old = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":16,"projectionSchema":1}),
        json!("old-questions"),
    );
    assert!(
        !old["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == protocol::QUESTION_CONTRACT)
    );
    assert_eq!(
        client.command("request.respond", answer.clone(), uuid::Uuid::new_v4())["error"]["message"],
        protocol::QUESTIONS_UPDATE_REQUIRED
    );
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("new-questions"),
    );
    assert_eq!(
        client.command("request.respond", answer, uuid::Uuid::new_v4())["result"]["status"],
        "accepted"
    );
    client.until(|state| state["threads"][0]["status"] == "Completed");
    let records: Vec<Value> = fs::read_to_string(profile.root.join("choice-log.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let response = records
        .iter()
        .find(|r| r["id"] == 71 && r["result"].is_object())
        .unwrap();
    assert_eq!(
        response["result"]["outcome"],
        json!({"outcome":"selected","optionId":"choice-2"})
    );
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
fn remote_captain_documents_use_the_worker_mcp_endpoint_over_one_way_pairing() {
    let mut home = Profile::new();
    let mut target = Profile::new();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let mut owner = home.client(uuid::Uuid::new_v4());
    let mut executor = target.client(uuid::Uuid::new_v4());
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
    let put = owner.command(
        "captain.put",
        json!({"record":{"name":"Document keeper","role":"Keeper",
            "instructions":"PRIVATE-PROFILE","portrait":1}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(put["result"]["status"], "accepted", "{put}");
    let opened = owner.command("captain.open", json!({"captain":0}), uuid::Uuid::new_v4());
    assert_eq!(opened["result"]["status"], "accepted", "{opened}");
    let parent = opened["result"]["result"]["threadId"].clone();
    let scope = owner.command(
        "captain.scope",
        json!({"captain":0,"projects":[],
            "remoteProjects":[{"server":discovery["serverId"],"projectId":project}]}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(scope["result"]["status"], "accepted", "{scope}");
    let text = format!("{}SENTINEL-TAIL-RULE", "a😀".repeat(2_000));
    for (name, body) in [("CREW.md", text.as_str()), ("AGENTS.md", "PRIVATE-RULES")] {
        let write = owner.command(
            "captain.document.write",
            json!({"captain":0,"name":name,"text":body,"expectedRevision":0}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(write["result"]["status"], "accepted", "{write}");
    }
    let initialized = owner.command(
        "thread.send",
        json!({"threadId":parent,"text":"Initialize document coordinator",
            "attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(initialized["result"]["status"], "accepted", "{initialized}");
    owner.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == parent && thread["status"] == "Completed")
    });
    let coordinator_directory = PathBuf::from(
        owner.snapshot["directories"][parent.as_u64().unwrap().to_string()]
            .as_str()
            .expect("the server reports the coordinator directory"),
    );
    assert!(
        coordinator_directory.join("fixture-mcp.json").exists(),
        "no coordinator MCP fixture in {}",
        coordinator_directory.display()
    );
    let launch = call_fixture_tool(
        &coordinator_directory,
        "delegate_task",
        json!({"prompt":"peer-documents-hold-fixture","client_request_id":"documents",
            "target":{"environmentId":discovery["serverId"],"projectId":project,"model":"fixture"}}),
    );
    assert_ne!(launch["result"]["isError"], true, "{launch}");
    executor.until(|state| {
        workspace.join("fixture-mcp.json").exists()
            && fs::read_to_string(workspace.join("fixture-prompts.jsonl"))
                .ok()
                .is_some_and(|text| {
                    text.ends_with('\n') && text.contains("peer-documents-hold-fixture")
                })
            && state["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|thread| thread["status"] == "Working" || thread["status"] == "Running")
    });
    let brief = fs::read_to_string(workspace.join("fixture-prompts.jsonl")).unwrap();
    assert!(brief.contains("captain_document_read"));
    assert!(!brief.contains("SENTINEL-TAIL-RULE"));
    assert!(!brief.contains("PRIVATE-PROFILE"));
    assert!(!brief.contains("PRIVATE-RULES"));
    let read = |args| {
        let response = call_fixture_tool(&workspace, "captain_document_read", args);
        assert_ne!(response["result"]["isError"], true, "{response}");
        serde_json::from_str::<Value>(response["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap()
    };
    let mut collected = String::new();
    let mut offset = 0;
    for _ in 0..20 {
        let page = read(json!({"name":"CREW.md","offset":offset,
            "maxBytes":1024,"expectedRevision":1}));
        assert_eq!(page["revision"], 1);
        assert_eq!(page["bytes"], text.len());
        collected.push_str(page["text"].as_str().unwrap());
        if page["truncated"] == false {
            break;
        }
        let next = page["nextOffset"].as_u64().unwrap();
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(collected, text);
    for (args, error) in [
        (json!({"name":"AGENTS.md"}), "private"),
        (
            json!({"name":"CREW.md","expectedRevision":0}),
            "changed since revision",
        ),
    ] {
        let response = call_fixture_tool(&workspace, "captain_document_read", args);
        assert_eq!(response["result"]["isError"], true, "{response}");
        assert!(response.to_string().contains(error), "{response}");
    }
    executor.rpc(
        "workspace.subscribe",
        json!({}),
        json!("read-worker-origin"),
    );
    let origin = executor.snapshot["index"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["peerOrigin"].is_object())
        .unwrap()["peerOrigin"]
        .clone();
    let revoked = executor.command(
        "devices.revoke",
        json!({"deviceId":origin["deviceId"]}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(revoked["result"]["status"], "accepted", "{revoked}");
    let response = call_fixture_tool(
        &workspace,
        "captain_document_read",
        json!({"name":"CREW.md"}),
    );
    assert_eq!(response["result"]["isError"], true, "{response}");
    assert!(response.to_string().contains("authorized"), "{response}");
    drop(owner);
    drop(executor);
    home.stop();
    target.stop();
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
            fs::read_to_string(checkout.join("README.md")).unwrap(),
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
        fs::read_to_string(picked.join("README.md")).unwrap(),
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
        fs::read_to_string(PathBuf::from(dependent_checkout).join("upstream-result.txt")).unwrap(),
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

    fn crash_child(&mut self, index: usize) {
        let child = self.children.get_mut(index).expect("fixture server index");
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        let _ = child.wait();
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

fn durable_task_inputs(profile: &Profile) -> Vec<Value> {
    let connection = rusqlite::Connection::open(profile.root.join("Shina/state.sqlite")).unwrap();
    connection.busy_timeout(Duration::from_secs(2)).unwrap();
    let mut statement = connection
        .prepare("SELECT payload FROM task_input_intents ORDER BY updated_at, command_id")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect()
}

/// Pair live home and target serve processes over loopback, register matching
/// repositories, and return the target identity, both clients and a parent
/// home thread. The caller owns launching and stopping both profiles.
fn link_peer_servers(
    target: &mut Profile,
    home: &mut Profile,
    repository: &str,
) -> (Value, Client, Client, Value) {
    let target_discovery = target.discovery();
    let mut target_client = target.client(uuid::Uuid::new_v4());
    let target_workspace = target.root.join("workspace");
    fs::create_dir_all(&target_workspace).unwrap();
    let home_workspace = home.root.join("workspace");
    for path in [&target_workspace, &home_workspace] {
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
                .args(["remote", "add", "origin", repository])
                .status()
                .unwrap()
                .success()
        );
    }
    assert_eq!(
        target_client.command(
            "project.put",
            json!({"path":target_workspace}),
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
    let code = String::from_utf8(code.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let host = format!("127.0.0.1:{}", target_discovery["port"]);
    let linked = home.cli(&["peer", "link", &host, &code]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let mut client = home.client(uuid::Uuid::new_v4());
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!(format!("peer-home-{}", uuid::Uuid::new_v4())),
    );
    assert_eq!(
        client.command(
            "project.put",
            json!({"path":home_workspace}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let parent = client.command(
        "thread.create",
        json!({"project":1}),
        uuid::Uuid::new_v4(),
    )["result"]["result"]["threadId"]
        .clone();
    (target_discovery, target_client, client, parent)
}

fn delegate_fixture(
    client: &mut Client,
    parent: &Value,
    prompt: &str,
    key: &str,
    target: &Value,
) -> Value {
    let args = json!({"prompt":prompt,"client_request_id":key,
        "target":{"machine":target["serverId"],"model":"fixture"}});
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let response = client.command(
            "task.delegate",
            json!({"threadId":parent,"args":args}),
            uuid::Uuid::new_v4(),
        );
        let waiting = response["result"]["result"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("does not advertise this model"));
        if !waiting {
            assert_eq!(response["result"]["status"], "accepted", "{response}");
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "target did not advertise fixture model: {response}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
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
fn staged_files_linked_servers_failure_retry_claim_and_older_peer_negotiation() {
    use sha2::Digest;
    fn start(
        http: &reqwest::blocking::Client,
        base: &str,
        token: &str,
        name: &str,
        bytes: &[u8],
        staged: bool,
    ) -> String {
        let response = http
            .post(format!("{base}/v1/uploads"))
            .bearer_auth(token)
            .json(
                &json!({"staged":staged,"type":"file","name":name,"mimeType":"text/plain",
                "length":bytes.len(),"sha256":format!("{:x}", sha2::Sha256::digest(bytes))}),
            )
            .send()
            .unwrap();
        assert!(response.status().is_success(), "{response:?}");
        response.json::<Value>().unwrap()["uploadId"]
            .as_str()
            .unwrap()
            .into()
    }
    fn complete(
        http: &reqwest::blocking::Client,
        base: &str,
        token: &str,
        id: &str,
        bytes: &[u8],
    ) -> Value {
        let mut offset = 0;
        for chunk in bytes.chunks(protocol::MAX_PAGE_BYTES) {
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
    let mut a = Profile::new();
    let mut b = Profile::new();
    a.launch(&["serve"]);
    b.launch(&["serve"]);
    let discovery_a = a.discovery();
    let discovery_b = b.discovery();
    let base_a = format!("http://127.0.0.1:{}", discovery_a["port"]);
    let base_b = format!("http://127.0.0.1:{}", discovery_b["port"]);
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
    let linked = b.cli(&["link", &format!("127.0.0.1:{}", discovery_a["port"]), &code]);
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
    let mut client_a = Client::with_token(&discovery_a, token_a.into());
    let mut client_b = b.client(uuid::Uuid::new_v4());
    client_b.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("staged-b-hello"),
    );
    let hello = client_a.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("staged-hello"),
    );
    assert_eq!(
        hello["result"]["limits"]["fileAttachmentBytes"],
        protocol::FILE_ATTACHMENT_MAX_BYTES
    );
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == protocol::STAGED_ATTACHMENTS)
    );
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let bytes = "Staged text remains available until claim and release.\n"
        .repeat(12000)
        .into_bytes();
    let failed = start(&http, &base_a, token_a, "notes.txt", &bytes, true);
    assert_eq!(
        http.put(format!("{base_a}/v1/uploads/{failed}?offset=1"))
            .bearer_auth(token_a)
            .body("wrong offset")
            .send()
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        http.delete(format!("{base_a}/v1/staged/{failed}"))
            .bearer_auth(token_a)
            .send()
            .unwrap()
            .status(),
        204
    );
    let retry = start(&http, &base_a, token_a, "notes.txt", &bytes, true);
    let first = complete(&http, &base_a, token_a, &retry, &bytes);
    assert_eq!(
        http.get(format!(
            "{base_a}/v1/blobs/{}",
            first["id"].as_str().unwrap()
        ))
        .bearer_auth(token_a)
        .send()
        .unwrap()
        .status(),
        404
    );
    assert!(
        http.get(format!("{base_a}/v1/staged/{retry}"))
            .bearer_auth(token_a)
            .send()
            .unwrap()
            .status()
            .is_success()
    );
    let mut downloaded = Vec::new();
    while downloaded.len() < bytes.len() {
        let response = http
            .get(format!(
                "{base_a}/v1/staged/{retry}/blob?offset={}&limit={}",
                downloaded.len(),
                protocol::MAX_PAGE_BYTES
            ))
            .bearer_auth(token_a)
            .send()
            .unwrap();
        assert!(response.status().is_success());
        assert_eq!(
            response.headers()["x-blob-length"]
                .to_str()
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            bytes.len()
        );
        let page = response.bytes().unwrap();
        assert!(!page.is_empty() && page.len() <= protocol::MAX_PAGE_BYTES);
        downloaded.extend_from_slice(&page);
    }
    assert!(
        downloaded == bytes,
        "Pending upload bytes changed during transfer"
    );
    let second_id = start(
        &http,
        &base_b,
        &client_b.token,
        "notes.txt",
        &downloaded,
        true,
    );
    let second = complete(&http, &base_b, &client_b.token, &second_id, &downloaded);
    let renamed_id = start(
        &http,
        &base_b,
        &client_b.token,
        "renamed.txt",
        &downloaded,
        true,
    );
    let renamed = complete(&http, &base_b, &client_b.token, &renamed_id, &downloaded);
    for (client, attachments, leases) in [
        (&mut client_a, vec![first.clone()], vec![retry.clone()]),
        (
            &mut client_b,
            vec![second.clone(), renamed],
            vec![second_id.clone(), renamed_id.clone()],
        ),
    ] {
        let sent = client.command(
            "thread.send",
            json!({"threadId":1,"text":"Inspect staged files",
            "attachments":attachments,"stagedUploadIds":leases,"mode":"prompt"}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(sent["result"]["status"], "accepted", "{sent}");
        client.until(|state| state["threads"][0]["status"] == "Completed");
    }
    let history = client_b.rpc(
        "thread.history",
        json!({"threadId":1,"byteBudget":protocol::MAX_PAGE_BYTES}),
        json!("staged-history"),
    );
    assert!(history["error"].is_null(), "{history}");
    assert!(history["result"]["nextFragment"].is_null());
    let history: Value = serde_json::from_str(history["result"]["data"].as_str().unwrap()).unwrap();
    let message = &history["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["field"] == "messages" && row["item"]["role"] == "You")
        .expect("sent message in scoped history")["item"];
    assert_eq!(message["images"][0]["name"], "notes.txt");
    assert_eq!(message["images"][1]["name"], "renamed.txt");
    assert_eq!(
        http.delete(format!("{base_b}/v1/staged/{second_id}"))
            .bearer_auth(&client_b.token)
            .send()
            .unwrap()
            .status(),
        204
    );
    let claimed = http
        .get(format!(
            "{base_b}/v1/blobs/{}",
            second["id"].as_str().unwrap()
        ))
        .bearer_auth(&client_b.token)
        .send()
        .unwrap();
    assert!(claimed.status().is_success());
    assert_eq!(
        claimed.headers()["x-blob-length"]
            .to_str()
            .unwrap()
            .parse::<usize>()
            .unwrap(),
        bytes.len()
    );
    let page = claimed.bytes().unwrap();
    assert!(page.as_ref() == &bytes[..page.len()]);
    assert!(
        fs::read(
            b.root
                .join("Shina/attachments")
                .join(second["id"].as_str().unwrap())
        )
        .unwrap()
            == bytes
    );
    // Images use the same pending lease lifecycle, including failure and retry,
    // without invoking the fixture model, which deliberately does not declare vision.
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let png = encoded.into_inner();
    let start_image = |base: &str, token: &str, hash: &str, staged: bool| {
        let response = http.post(format!("{base}/v1/uploads")).bearer_auth(token)
            .json(&json!({"staged":staged,"type":"image","name":"pixel.png","length":png.len(),"sha256":hash}))
            .send().unwrap();
        assert!(response.status().is_success());
        response.json::<Value>().unwrap()["uploadId"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let bad_image = start_image(&base_a, token_a, &"0".repeat(64), true);
    assert!(
        http.put(format!("{base_a}/v1/uploads/{bad_image}?offset=0"))
            .bearer_auth(token_a)
            .body(png.clone())
            .send()
            .unwrap()
            .status()
            .is_success()
    );
    assert_eq!(
        http.post(format!("{base_a}/v1/uploads/{bad_image}/commit"))
            .bearer_auth(token_a)
            .send()
            .unwrap()
            .status(),
        400
    );
    let image_hash = format!("{:x}", sha2::Sha256::digest(&png));
    let mut image_identity = None;
    for (base, token) in [(&base_a, token_a), (&base_b, client_b.token.as_str())] {
        let upload = start_image(base, token, &image_hash, true);
        let image = complete(&http, base, token, &upload, &png);
        assert_eq!(image["mime"], "image/png");
        assert_eq!(image["width"], 4);
        assert_eq!(image["height"], 2);
        if let Some(id) = &image_identity {
            assert_eq!(&image["id"], id);
        }
        image_identity = Some(image["id"].clone());
        let page = http
            .get(format!("{base}/v1/staged/{upload}/blob?offset=0"))
            .bearer_auth(token)
            .send()
            .unwrap();
        assert!(page.status().is_success());
        assert_eq!(page.bytes().unwrap().as_ref(), png.as_slice());
        assert_eq!(
            http.delete(format!("{base}/v1/staged/{upload}"))
                .bearer_auth(token)
                .send()
                .unwrap()
                .status(),
            204
        );
    }
    let old = client_b.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::STAGED_ATTACHMENTS_MINOR - 1,"projectionSchema":1}),
        json!("legacy-staged-hello"),
    );
    assert_eq!(
        protocol::file_attachment_limit(&old["result"]),
        Some(protocol::FILE_ATTACHMENT_MAX_BYTES)
    );
    assert!(
        !old["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == protocol::STAGED_ATTACHMENTS)
    );
    let unsupported = client_b.command(
        "thread.send",
        json!({"threadId":1,"text":"Must not use staged fields",
        "attachments":[second.clone()],"stagedUploadIds":[second_id],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(
        unsupported["error"]["message"],
        protocol::STAGED_ATTACHMENTS_UPDATE_REQUIRED
    );
    let legacy_image = start_image(&base_b, &client_b.token, &image_hash, false);
    let legacy_image = complete(&http, &base_b, &client_b.token, &legacy_image, &png);
    assert!(legacy_image.get("stagedUploadId").is_none());
    assert_eq!(legacy_image["id"], image_identity.unwrap());
    let legacy_id = start(&http, &base_b, &client_b.token, "legacy.txt", &bytes, false);
    let legacy = complete(&http, &base_b, &client_b.token, &legacy_id, &bytes);
    assert!(legacy.get("stagedUploadId").is_none());
    let sent = client_b.command(
        "thread.send",
        json!({"threadId":1,"text":"Legacy deferred upload",
        "attachments":[legacy],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(sent["result"]["status"], "accepted", "{sent}");
    a.stop();
    b.stop();
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
fn linked_thread_reads_disambiguate_machine_ids_and_page_unrelated_threads() {
    let mut home = Profile::new();
    let mut target = Profile::new();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let target_discovery = target.discovery();
    let mut home_client = home.client(uuid::Uuid::new_v4());
    let mut target_client = target.client(uuid::Uuid::new_v4());
    for client in [&mut home_client, &mut target_client] {
        for id in 2..=7 {
            let created = client.command(
                "thread.create",
                json!({"occupiedDrafts":[1,2,3,4,5,6,7]}),
                uuid::Uuid::new_v4(),
            );
            assert_eq!(created["result"]["result"]["threadId"], id, "{created}");
        }
    }
    for (client, text) in [
        (&mut home_client, "HOME SEVENTH THREAD"),
        (&mut target_client, "remote-long-fixture-unrelated"),
    ] {
        let sent = client.command(
            "thread.send",
            json!({"threadId":7,"text":text,"attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(sent["result"]["status"], "accepted", "{sent}");
        client.until(|state| {
            state["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|thread| thread["id"] == 7 && thread["status"] == "Completed")
        });
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
    let host = format!("127.0.0.1:{}", target_discovery["port"]);
    let linked = home.cli(&["peer", "link", &host, code]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    // Refresh peer metadata before making the model's request.
    home_client.rpc(
        "workspace.subscribe",
        json!({}),
        json!("refresh-linked-reads"),
    );
    let home_directory = home.root.join("Shina/chats/7");
    let target_directory = target.root.join("Shina/chats/7");
    let home_prompts = fs::read(home_directory.join("fixture-prompts.jsonl")).unwrap();
    let target_prompts = fs::read(target_directory.join("fixture-prompts.jsonl")).unwrap();
    let local = read_fixture_thread(&home_directory, json!({"threadId":7}));
    assert!(
        local["items"][0]["text"]
            .as_str()
            .unwrap()
            .contains("HOME SEVENTH THREAD")
    );
    let remote = read_fixture_thread(
        &home_directory,
        json!({"machine":target_discovery["serverId"],
        "threadId":7,"limit":1,"afterPosition":0,"maxCharsPerItem":6000}),
    );
    let capabilities = call_fixture_tool(&home_directory, "orchestrator_capabilities", json!({}));
    let capabilities: Value = serde_json::from_str(
        capabilities["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(capabilities["peers"][0]["threadReads"], true);
    assert_eq!(capabilities["peers"][0]["threadLists"], true);
    assert_eq!(remote["thread"]["threadId"], 7);
    assert_eq!(remote["thread"]["machine"], target_discovery["serverId"]);
    let item = &remote["items"][0];
    assert_eq!(item["textTruncated"], true);
    assert_eq!(
        item["continuation"]["machine"],
        target_discovery["serverId"]
    );
    let rest = read_fixture_thread(&home_directory, item["continuation"].clone());
    let text = format!(
        "{}{}",
        item["text"].as_str().unwrap(),
        rest["items"][0]["text"].as_str().unwrap()
    );
    for fact in ["START FACT", "MIDDLE FACT", "END FACT"] {
        assert!(text.contains(fact));
    }
    assert!(!text.contains("HOME SEVENTH THREAD"));
    let refused = call_fixture_tool(
        &home_directory,
        "read_thread",
        json!({"machine":"not-linked-fixture","threadId":7}),
    );
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    assert!(
        refused["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not-linked-fixture")
    );
    let listing = call_fixture_tool(
        &home_directory,
        "list_threads",
        json!({"machine":target_discovery["serverId"],"allProjects":true,"titleContains":"remote-long","limit":1}),
    );
    assert_ne!(listing["result"]["isError"], true, "{listing}");
    let listing: Value =
        serde_json::from_str(listing["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(listing["threads"][0]["threadId"], 7);
    assert_eq!(
        listing["threads"][0]["machine"],
        target_discovery["serverId"]
    );
    assert_eq!(
        fs::read(home_directory.join("fixture-prompts.jsonl")).unwrap(),
        home_prompts
    );
    assert_eq!(
        fs::read(target_directory.join("fixture-prompts.jsonl")).unwrap(),
        target_prompts
    );
    drop(home_client);
    drop(target_client);
    home.stop();
    target.stop();
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
fn paired_cli_client_starts_views_steers_and_stops_only_the_remote_server() {
    let mut a = Profile::new();
    let mut b = Profile::new();
    a.launch(&["serve"]);
    b.launch(&["serve"]);
    let discovery_a = a.discovery();
    let discovery_b = b.discovery();
    let paired = b.cli(&["pair"]);
    assert!(
        paired.status.success(),
        "{}",
        String::from_utf8_lossy(&paired.stderr)
    );
    let text = String::from_utf8(paired.stdout).unwrap();
    let code = text
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap();
    let host = format!("127.0.0.1:{}", discovery_b["port"].as_u64().unwrap());
    assert!(a.cli(&["link", &host, code]).status.success());
    assert!(
        !a.cli(&["link", &host, code]).status.success(),
        "a used code must refuse"
    );
    let listing = a.cli(&["links"]);
    assert!(listing.status.success());
    let credential: Value = serde_json::from_slice(
        &fs::read(
            a.root
                .join("Shina/linked")
                .join(discovery_b["serverId"].as_str().unwrap())
                .join("credential.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        !String::from_utf8_lossy(&listing.stdout).contains(credential["token"].as_str().unwrap())
    );
    let mut remote = Client::with_token(&discovery_b, credential["token"].as_str().unwrap().into());
    let workspace = b.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    assert_eq!(
        remote.command(
            "project.put",
            json!({"path":workspace}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let created = remote.command(
        "thread.create",
        json!({"project":1,"occupiedDrafts":[1,2]}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(created["result"]["status"], "accepted", "{created}");
    let thread = created["result"]["result"]["threadId"].as_u64().unwrap();
    assert_eq!(
        remote.command(
            "thread.send",
            json!({"threadId":thread,"text":"disconnect-fixture","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    remote.until(|state| {
        state["threads"].as_array().unwrap().iter().any(|row| {
            row["id"] == thread
                && row["status"] == "Running"
                && row["configuring"] == false
                && row["operating"] == false
                && row["initializing"] == false
                && row["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|message| message["text"] == "disconnect-fixture")
        })
    });
    let steer = remote.command(
        "thread.send",
        json!({"threadId":thread,"text":"remote-steer","attachments":[],"mode":"steer"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(steer["result"]["status"], "accepted", "{steer}");
    assert_eq!(
        remote.command(
            "thread.stop",
            json!({"threadId":thread}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let local = Client::connect(&discovery_a, uuid::Uuid::new_v4());
    assert!(
        local.snapshot["index"]["projects"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        local.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        remote.command("server.stop", json!({}), uuid::Uuid::new_v4())["error"]["message"]
            .as_str()
            .unwrap()
            .contains("maintain")
    );
    drop(local);
    drop(remote);
    assert!(a.cli(&["unlink", &host]).status.success());
    assert!(a.cli(&["links"]).stdout.is_empty());
    a.stop();
    b.stop();
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

#[test]
fn two_serve_processes_converge_and_mcp_delivers_without_a_client() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.until(|state| {
        state["threads"][0]["models"]
            .as_array()
            .is_some_and(|models| !models.is_empty())
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let running = profile
            .children
            .iter_mut()
            .map(|child| child.try_wait().unwrap().is_none())
            .filter(|running| *running)
            .count();
        if running == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the losing serve process did not exit"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        client.command(
            "thread.send",
            json!({"threadId":1,"text":"delegate-fixture","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    drop(client);
    std::thread::sleep(Duration::from_secs(3));
    let mut second = profile.client(uuid::Uuid::new_v4());
    second.until(|state| {
        state["index"]["tasks"].as_array().is_some_and(|tasks| {
            tasks
                .iter()
                .any(|task| task["state"] == "completed" && task["delivered"] == true)
        })
    });
    assert!(
        second.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["text"]
                .as_str()
                .is_some_and(|text| text.contains("finished\nTask id:")))
    );
    profile.stop();
}

#[test]
fn keyed_task_controls_replay_on_the_wire_and_survive_restart() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.until(|state| {
        state["threads"][0]["models"]
            .as_array()
            .is_some_and(|models| !models.is_empty())
    });
    assert_eq!(
        client.command(
            "thread.send",
            json!({"threadId":1,"text":"delegate-fixture","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .is_some_and(|tasks| !tasks.is_empty())
    });
    let child = client.snapshot["index"]["tasks"][0]["child_thread_id"]
        .as_u64()
        .unwrap();
    client.until(|state| state["index"]["tasks"][0]["state"] == "completed");
    let task_id = client.snapshot["index"]["tasks"][0]["id"].clone();
    let first_attempt = client.snapshot["index"]["tasks"][0]["attempt_id"].clone();
    assert!(first_attempt.is_string(), "attempt identity is durable");

    // The compatibility policy is loud: unkeyed control is refused.
    let unkeyed = client.command(
        "task.send",
        json!({"threadId":1,"args":{"thread_id":child,"message":"no key"}}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        unkeyed.to_string().contains("client_request_id"),
        "{unkeyed}"
    );

    // A keyed send to the completed child starts a fresh attempt on the same task.
    let send = json!({"thread_id":child,"message":"wire-check","client_request_id":"send-wire-1"});
    let first = client.command(
        "task.send",
        json!({"threadId":1,"args":send.clone()}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(first["result"]["status"], "accepted", "{first}");
    assert_eq!(first["result"]["result"]["task_id"], task_id);
    assert_ne!(first["result"]["result"]["attempt_id"], first_attempt);
    assert_eq!(first["result"]["result"]["operation"]["saved"], true);
    assert_eq!(
        first["result"]["result"]["operation"]["requested_delivery"],
        "follow_up"
    );
    assert_eq!(
        first["result"]["result"]["operation"]["acceptance"],
        "not_observed"
    );
    let attempt_b = first["result"]["result"]["attempt_id"].clone();

    // An identical retry replays the committed receipt without another delivery.
    let replay = client.command(
        "task.send",
        json!({"threadId":1,"args":send}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(replay["result"]["result"]["replayed"], true, "{replay}");
    assert_eq!(replay["result"]["result"]["attempt_id"], attempt_b);

    // A used key with different arguments fails rather than running twice.
    let conflict = client.command(
        "task.send",
        json!({"threadId":1,"args":{"thread_id":child,"message":"changed","client_request_id":"send-wire-1"}}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        conflict
            .to_string()
            .contains("different send or cancel arguments"),
        "{conflict}"
    );

    // The single accepted input reaches the child harness exactly once.
    client.until(|state| state["index"]["tasks"][0]["state"] == "completed");
    let child_prompts = profile
        .root
        .join(format!("Shina/chats/{child}/fixture-prompts.jsonl"));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let count = fs::read_to_string(&child_prompts)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains("wire-check"))
            .count();
        if count == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "child prompt log at {child_prompts:?}: {count} deliveries"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // The receipt is durable across a server restart: replay, never resend.
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let replay = client.command(
        "task.send",
        json!({"threadId":1,"args":{"thread_id":child,"message":"wire-check","client_request_id":"send-wire-1"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(replay["result"]["result"]["replayed"], true, "{replay}");
    assert_eq!(
        replay["result"]["result"]["operation"]["completion"],
        "completed"
    );
    assert_eq!(
        replay["result"]["result"]["control"],
        first["result"]["result"]["control"]
    );
    let delivered = fs::read_to_string(&child_prompts).unwrap();
    assert_eq!(
        delivered
            .lines()
            .filter(|line| line.contains("wire-check"))
            .count(),
        1
    );

    // A second keyed send starts attempt C; the fixture raises a confirmation
    // and holds the run open, so the next cancel lands on live work.
    let send = client.command(
        "task.send",
        json!({"threadId":1,"args":{"thread_id":child,"message":"remote-confirm-fixture","client_request_id":"send-wire-2"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(send["result"]["status"], "accepted", "{send}");
    let attempt_c = send["result"]["result"]["attempt_id"].clone();
    assert_ne!(attempt_c, attempt_b);
    client.until(|state| state["index"]["tasks"][0]["state"] == "running");

    // A delayed cancel fenced to the older attempt is refused; attempt C's
    // live work is untouched.
    let stale = client.command(
        "task.cancel",
        json!({"threadId":1,"args":{"thread_id":child,"client_request_id":"cancel-stale","attempt_id":attempt_b}}),
        uuid::Uuid::new_v4(),
    );
    assert!(stale.to_string().contains("attempt"), "{stale}");
    assert_eq!(client.snapshot["index"]["tasks"][0]["state"], "running");

    let cancel = client.command(
        "task.cancel",
        json!({"threadId":1,"args":{"thread_id":child,"client_request_id":"cancel-wire-1","attempt_id":attempt_c}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(cancel["result"]["status"], "accepted", "{cancel}");
    let replay = client.command(
        "task.cancel",
        json!({"threadId":1,"args":{"thread_id":child,"client_request_id":"cancel-wire-1","attempt_id":attempt_c}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(replay["result"]["result"]["replayed"], true, "{replay}");
    client.until(|state| state["index"]["tasks"][0]["state"] == "cancelled");
    let replay = client.command(
        "task.cancel",
        json!({"threadId":1,"args":{"thread_id":child,"client_request_id":"cancel-wire-1","attempt_id":attempt_c}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(
        replay["result"]["result"]["operation"]["completion"],
        "cancelled"
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

/// Remote control carries the observed attempt through peer transport. A
/// delayed cancel naming an attempt the target already left is refused before
/// dispatch; a matching fence stops the remote worker.
#[test]
fn remote_task_cancel_fences_the_observed_attempt_over_peer_transport() {
    let mut target = Profile::new();
    let mut home = Profile::new();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let target_discovery = target.discovery();
    let mut target_client = target.client(uuid::Uuid::new_v4());
    let workspace = target.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let home_workspace = home.root.join("workspace");
    for path in [&workspace, &home_workspace] {
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
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://example.com/peer-fixture.git"
                ])
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
    let code = String::from_utf8(code.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let host = format!("127.0.0.1:{}", target_discovery["port"]);
    let linked = home.cli(&["peer", "link", &host, &code]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let mut client = home.client(uuid::Uuid::new_v4());
    assert_eq!(
        client.command(
            "project.put",
            json!({"path":home_workspace}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let parent = client.command(
        "thread.create",
        json!({"project":1}),
        uuid::Uuid::new_v4()
    )["result"]["result"]["threadId"]
        .clone();
    // The fixture child raises a confirmation and holds it open, keeping
    // attempt A alive while control commands travel.
    let args = json!({"prompt":"remote-confirm-fixture","client_request_id":"launch-1",
        "target":{"machine":target_discovery["serverId"],"model":"fixture"}});
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
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(delegated["result"]["status"], "accepted", "{delegated}");
    let child = delegated["result"]["result"]["thread_id"].clone();
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["remote"]["threadId"].is_number())
    });
    client.until(|state| state["index"]["tasks"][0]["remote"]["request"].is_object());
    let attempt_a = client.snapshot["index"]["tasks"][0]["attempt_id"].clone();
    assert!(attempt_a.is_string());

    // A cancel fenced to the observed attempt travels over peer transport and
    // stops the remote worker.
    let cancel = client.command(
        "task.cancel",
        json!({"threadId":parent,"args":{"thread_id":child,"client_request_id":"cancel-1","attempt_id":attempt_a}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(cancel["result"]["status"], "accepted", "{cancel}");
    client.until(|state| state["index"]["tasks"][0]["state"] == "cancelled");
    target_client.until(|state| {
        state["index"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["peerOrigin"]["taskId"].is_string())
    });
    let retry = client.command(
        "task.cancel",
        json!({"threadId":parent,"args":{"thread_id":child,"client_request_id":"cancel-1","attempt_id":attempt_a}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(retry["result"]["result"]["replayed"], true, "{retry}");

    // The owed stop must be acknowledged before further work is owed.
    client.until(|state| state["index"]["tasks"][0]["remote"]["stopOwed"] == false);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let replay = client.command(
            "task.cancel",
            json!({"threadId":parent,"args":{"thread_id":child,"client_request_id":"cancel-1","attempt_id":attempt_a}}),
            uuid::Uuid::new_v4(),
        );
        if replay["result"]["result"]["operation"]["completion"] == "cancelled" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "target cancellation remained unobserved: {replay}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // A new keyed send starts attempt B on the same remote thread; the target
    // records the attempt identity the home committed.
    let send = client.command(
        "task.send",
        json!({"threadId":parent,"args":{"thread_id":child,"message":"remote-confirm-fixture","client_request_id":"send-2"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(send["result"]["status"], "accepted", "{send}");
    let attempt_b = send["result"]["result"]["attempt_id"].clone();
    assert_ne!(attempt_a, attempt_b);
    target_client.until(|state| {
        state["index"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["peerOrigin"]["attemptId"] == attempt_b)
    });

    // The delayed cancel a caller fenced to attempt A is refused before
    // dispatch: attempt B keeps running on the target.
    let stale = client.command(
        "task.cancel",
        json!({"threadId":parent,"args":{"thread_id":child,"client_request_id":"cancel-2","attempt_id":attempt_a}}),
        uuid::Uuid::new_v4(),
    );
    assert!(stale.to_string().contains("attempt"), "{stale}");
    assert_ne!(client.snapshot["index"]["tasks"][0]["state"], "cancelled");
    assert_ne!(
        client.snapshot["index"]["tasks"][0]["remote"]["stopOwed"], true,
        "a refused fence owes no remote stop"
    );

    let cancel_b = client.command(
        "task.cancel",
        json!({"threadId":parent,"args":{"thread_id":child,"client_request_id":"cancel-3","attempt_id":attempt_b}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(cancel_b["result"]["status"], "accepted", "{cancel_b}");
    client.until(|state| state["index"]["tasks"][0]["state"] == "cancelled");
    home.stop();
    target.stop();
}

/// A delayed answer fenced to question A is rejected after the target moves to
/// question B. The home keeps the rejection, refreshes B, never answers B with
/// A, and retains the same outcome after a home restart.
#[test]
fn two_server_stale_question_rejection_survives_refresh_and_restart() {
    let mut target = Profile::new();
    let mut home = Profile::new();
    let gate = home.root.join("peer-send-release");
    fs::write(
        home.root.join("extra-env.json"),
        json!({"SHINA_TEST_PEER_SEND_BARRIER":gate}).to_string(),
    )
    .unwrap();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let target_discovery = target.discovery();
    let mut target_client = target.client(uuid::Uuid::new_v4());
    target_client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("stale-question-target"),
    );
    let workspace = target.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let home_workspace = home.root.join("workspace");
    for path in [&workspace, &home_workspace] {
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
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://example.com/stale-question.git"
                ])
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
    let code = String::from_utf8(code.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let host = format!("127.0.0.1:{}", target_discovery["port"]);
    let linked = home.cli(&["peer", "link", &host, &code]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let device = uuid::Uuid::new_v4();
    let mut client = home.client(device);
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("stale-question-home"),
    );
    assert_eq!(
        client.command(
            "project.put",
            json!({"path":home_workspace}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let parent = client.command(
        "thread.create",
        json!({"project":1}),
        uuid::Uuid::new_v4(),
    )["result"]["result"]["threadId"]
        .clone();
    let args = json!({"prompt":"stale-question-fixture","client_request_id":"stale-live-delegate",
        "target":{"machine":target_discovery["serverId"],"model":"fixture"}});
    let mut delegated = Value::Null;
    for _ in 0..100 {
        delegated = client.command(
            "task.delegate",
            json!({"threadId":parent,"args":args.clone()}),
            uuid::Uuid::new_v4(),
        );
        let waiting = delegated["result"]["result"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("does not advertise this model"));
        if !waiting {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(delegated["result"]["status"], "accepted", "{delegated}");
    let child = delegated["result"]["result"]["thread_id"].clone();
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["remote"]["request"]["requestId"] == "question-a")
    });
    let request_a = client.snapshot["index"]["tasks"][0]["remote"]["request"].clone();
    let answer = json!({"threadId":parent,"args":{
        "thread_id":child,"message":"stale home answer A","client_request_id":"stale-live-answer",
        "request_id":request_a["requestId"],"runtime_generation":request_a["generation"]}});
    let saved = client.command("task.send", answer.clone(), uuid::Uuid::new_v4());
    assert_eq!(saved["result"]["status"], "accepted", "{saved}");
    assert_eq!(
        saved["result"]["result"]["operation"]["transport"], "not_observed",
        "the barrier holds the peer command"
    );
    client.until(|state| {
        state["index"]["taskOperations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| {
                operation["request_key"] == "stale-live-answer"
                    && operation["dispatch"] == "dispatched"
            })
    });

    // The target's user answers A directly. That advances the provider to B
    // while the home's fenced answer A is still behind the barrier.
    target_client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["request"]["id"] == "question-a")
    });
    let worker = target_client.snapshot["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["request"]["id"] == "question-a")
        .unwrap()["id"]
        .clone();
    let target_request_a = target_client.snapshot["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == worker)
        .unwrap()["request"]
        .clone();
    let answered = target_client.command(
        "request.respond",
        json!({"threadId":worker,"requestId":target_request_a["id"],
            "generation":target_request_a["runtimeGeneration"],
            "requestContract":true,"response":{"value":"target user answer A"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(answered["result"]["status"], "accepted", "{answered}");
    target_client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == worker && thread["request"]["id"] == "question-b")
    });

    fs::write(&gate, b"release").unwrap();
    client.until(|state| {
        state["index"]["taskOperations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| {
                operation["request_key"] == "stale-live-answer"
                    && operation["transport"] == "rejected"
            })
            && state["index"]["tasks"][0]["remote"]["request"]["requestId"] == "question-b"
    });
    let operation = client.snapshot["index"]["taskOperations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["request_key"] == "stale-live-answer")
        .cloned()
        .unwrap();
    assert_eq!(operation["transport"], "rejected");
    assert_eq!(operation["rejection_stage"], "transport");
    assert_eq!(operation["acceptance"], "not_observed");
    assert_eq!(operation["completion"], "not_observed");
    let replay = client.command("task.send", answer.clone(), uuid::Uuid::new_v4());
    assert_eq!(replay["result"]["result"]["replayed"], true, "{replay}");
    assert_eq!(
        replay["result"]["result"]["operation"]["transport"],
        "rejected"
    );
    assert_eq!(
        replay["result"]["result"]["current"]["request"]["requestId"],
        "question-b"
    );
    let worker_dir = PathBuf::from(
        target_client.snapshot["directories"][worker.to_string()]
            .as_str()
            .unwrap(),
    );
    let replies = fs::read_to_string(worker_dir.join("fixture-question-replies.jsonl")).unwrap();
    assert_eq!(
        replies.lines().count(),
        1,
        "only the target user's A reached Pi: {replies}"
    );
    assert!(
        !replies.contains("stale home answer A"),
        "the stale answer never reached Pi: {replies}"
    );

    // A home restart reloads the same operation evidence from SQLite.
    home.stop();
    home.launch(&["serve"]);
    let mut client = home.client(device);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let replay = client.command("task.send", answer.clone(), uuid::Uuid::new_v4());
        let ok = replay["result"]["result"]["operation"]["transport"] == "rejected"
            && replay["result"]["result"]["current"]["request"]["requestId"] == "question-b";
        if ok {
            assert_eq!(
                replay["result"]["result"]["operation"]["rejection_stage"],
                "transport"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "replay did not retain rejection: {replay}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    home.stop();
    target.stop();
}

/// A peer without the input-evidence contract cannot prove provider
/// acceptance. The home keeps the target receipt as transport acceptance and
/// reports provider acceptance as unknown instead of promoting the receipt.
#[test]
fn old_peer_receipt_leaves_provider_acceptance_unknown() {
    let mut target = Profile::new();
    let mut home = Profile::new();
    fs::write(
        target.root.join("extra-env.json"),
        json!({"SHINA_TEST_OMIT_PEER_EVIDENCE":"1"}).to_string(),
    )
    .unwrap();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let target_discovery = target.discovery();
    let mut target_client = target.client(uuid::Uuid::new_v4());
    let workspace = target.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let home_workspace = home.root.join("workspace");
    for path in [&workspace, &home_workspace] {
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
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://example.com/old-peer.git"
                ])
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
    let code = String::from_utf8(code.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let host = format!("127.0.0.1:{}", target_discovery["port"]);
    let linked = home.cli(&["peer", "link", &host, &code]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let mut client = home.client(uuid::Uuid::new_v4());
    assert_eq!(
        client.command(
            "project.put",
            json!({"path":home_workspace}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let parent = client.command("thread.create", json!({"project":1}), uuid::Uuid::new_v4())["result"]["result"]["threadId"].clone();
    let args = json!({"prompt":"queue-wait-fixture","client_request_id":"old-peer-delegate",
        "target":{"machine":target_discovery["serverId"],"model":"fixture"}});
    let mut delegated = Value::Null;
    for _ in 0..100 {
        delegated = client.command(
            "task.delegate",
            json!({"threadId":parent,"args":args.clone()}),
            uuid::Uuid::new_v4(),
        );
        let waiting = delegated["result"]["result"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("does not advertise this model"));
        if !waiting {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(delegated["result"]["status"], "accepted", "{delegated}");
    let child = delegated["result"]["result"]["thread_id"].clone();
    client.until(|state| state["index"]["tasks"][0]["remote"]["threadId"].is_number());
    let send = client.command(
        "task.send",
        json!({"threadId":parent,"args":{
        "thread_id":child,"message":"legacy follow-up","client_request_id":"old-peer-send"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(send["result"]["status"], "accepted", "{send}");
    client.until(|state| {
        state["index"]["taskOperations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| {
                operation["request_key"] == "old-peer-send"
                    && operation["transport"] == "accepted"
                    && operation["acceptance"] == "unknown"
                    && operation["acceptance_basis"] == "legacy_peer"
            })
    });
    let operation = client.snapshot["index"]["taskOperations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["request_key"] == "old-peer-send")
        .cloned()
        .unwrap();
    assert_eq!(operation["transport"], "accepted");
    assert_eq!(operation["acceptance"], "unknown");
    assert_eq!(operation["completion"], "not_observed");
    home.stop();
    target.stop();
}

/// A target receipt for a queued follow-up is transport acceptance only. The
/// second follow-up remains not observed while the first prompt is held, then
/// becomes accepted only after the provider acknowledges its own prompt.
/// A target receipt for a queued follow-up is transport acceptance only. The
/// follow-up remains not observed while the provider slot is held, then
/// becomes accepted only after the provider acknowledges its own prompt.
#[test]
fn queued_target_response_is_not_provider_acceptance_until_flushed() {
    let mut target = Profile::new();
    let mut home = Profile::new();
    let release = target.root.join("release-injection");
    fs::write(
        target.root.join("extra-env.json"),
        json!({"SHINA_TEST_HOLD_INJECTION_FLUSH":release}).to_string(),
    )
    .unwrap();
    home.launch(&["serve"]);
    target.launch(&["serve"]);
    let target_discovery = target.discovery();
    let mut target_client = target.client(uuid::Uuid::new_v4());
    let workspace = target.root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let home_workspace = home.root.join("workspace");
    for path in [&workspace, &home_workspace] {
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
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://example.com/queued-target.git"
                ])
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
    let code = String::from_utf8(code.stdout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap()
        .to_owned();
    let host = format!("127.0.0.1:{}", target_discovery["port"]);
    let linked = home.cli(&["peer", "link", &host, &code]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let mut client = home.client(uuid::Uuid::new_v4());
    assert_eq!(
        client.command(
            "project.put",
            json!({"path":home_workspace}),
            uuid::Uuid::new_v4()
        )["result"]["status"],
        "accepted"
    );
    let parent = client.command("thread.create", json!({"project":1}), uuid::Uuid::new_v4())["result"]["result"]["threadId"].clone();
    let args = json!({"prompt":"peer-documents-hold-fixture","client_request_id":"queued-delegate",
        "target":{"machine":target_discovery["serverId"],"model":"fixture"}});
    let mut delegated = Value::Null;
    for _ in 0..100 {
        delegated = client.command(
            "task.delegate",
            json!({"threadId":parent,"args":args.clone()}),
            uuid::Uuid::new_v4(),
        );
        let waiting = delegated["result"]["result"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("does not advertise this model"));
        if !waiting {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(delegated["result"]["status"], "accepted", "{delegated}");
    let child = delegated["result"]["result"]["thread_id"].clone();
    client.until(|state| state["index"]["tasks"][0]["remote"]["threadId"].is_number());
    let second = client.command(
        "task.send",
        json!({"threadId":parent,"args":{
        "thread_id":child,"message":"queued-two-live","client_request_id":"queued-two"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(second["result"]["status"], "accepted", "{second}");
    client.until(|state| {
        state["index"]["taskOperations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| {
                operation["request_key"] == "queued-two"
                    && operation["transport"] == "accepted"
                    && operation["acceptance"] == "not_observed"
            })
    });
    let prompts = workspace.join("fixture-prompts.jsonl");
    assert_eq!(
        fs::read_to_string(&prompts)
            .unwrap_or_default()
            .lines()
            .count(),
        0,
        "the target stored both inputs but the provider slot is held"
    );
    fs::write(&release, b"release").unwrap();
    client.until(|state| {
        state["index"]["taskOperations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| {
                operation["request_key"] == "queued-two"
                    && operation["acceptance"] == "accepted"
                    && operation["acceptance_basis"] == "provider_ack"
            })
    });
    let delivered = fs::read_to_string(&prompts).unwrap();
    assert_eq!(
        delivered
            .lines()
            .filter(|line| line.contains("queued-two-live"))
            .count(),
        1,
        "the queued input reached the provider exactly once: {delivered}"
    );
    home.stop();
    target.stop();
}

/// The target stores peer input with its accepted command. A crash before the
/// provider flush, followed by a home crash and the same keyed task_send retry,
/// restores one logical input and delivers it once after stable history proves
/// the marker was absent.
#[test]
fn target_queued_input_survives_restart_and_keyed_retry_without_loss_or_duplication() {
    let mut target = Profile::new();
    let mut home = Profile::new();
    let port = free_port();
    let port_arg = port.to_string();
    let gate = target.root.join("peer-input-release");
    // The first delegated brief may flush normally. Removing the gate before
    // the follow-up holds that exact peer input before its provider write.
    fs::write(&gate, b"ready").unwrap();
    fs::write(
        target.root.join("extra-env.json"),
        json!({"SHINA_TEST_HOLD_PEER_INPUT":gate}).to_string(),
    )
    .unwrap();
    target.launch(&["serve", "--port", &port_arg]);
    home.launch(&["serve"]);
    let target_discovery = target.discovery();
    let (_, _target_client, mut client, parent) = link_peer_servers(
        &mut target,
        &mut home,
        "https://example.com/durable-target-input.git",
    );
    let delegated = delegate_fixture(
        &mut client,
        &parent,
        "durable-target-initial",
        "durable-target-input-launch",
        &target_discovery,
    );
    let child = delegated["result"]["result"]["thread_id"].clone();
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["state"] == "completed")
            && state["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|thread| thread["id"] == parent && thread["status"] == "Completed")
    });
    let prompts = target.root.join("workspace/fixture-prompts.jsonl");
    assert_eq!(fs::read_to_string(&prompts).unwrap().lines().count(), 1);

    fs::remove_file(&gate).unwrap();
    let send = json!({"thread_id":child,"message":"durable-target-input-followup",
        "client_request_id":"durable-target-input-send"});
    let queued = client.command(
        "task.send",
        json!({"threadId":parent,"args":send.clone()}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(queued["result"]["status"], "accepted", "{queued}");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let durable = durable_task_inputs(&target);
        if durable.iter().any(|input| {
            input["message"]
                .as_str()
                .is_some_and(|message| message.contains("durable-target-input-followup"))
                && input["queue_state"] == "queued"
                && input["provider_state"] == "not_observed"
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "target did not persist input: {durable:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        fs::read_to_string(&prompts).unwrap().lines().count(),
        1,
        "the held follow-up must not have reached the provider"
    );

    // Both sides lose their process state after the target command committed.
    target.crash_child(0);
    home.stop();
    // A fixed listener keeps the peer address stable. Remove the stale
    // discovery document so clients wait for the restarted server's new
    // bootstrap token instead of racing the previous process.
    let _ = fs::remove_file(target.root.join("Shina/server.json"));
    let _ = fs::remove_file(home.root.join("Shina/server.json"));
    fs::write(&gate, b"release").unwrap();
    target.launch(&["serve", "--port", &port_arg]);
    home.launch(&["serve"]);
    let _target_client = target.client(uuid::Uuid::new_v4());
    let mut client = home.client(uuid::Uuid::new_v4());
    let retried = client.command(
        "task.send",
        json!({"threadId":parent,"args":send.clone()}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(retried["result"]["status"], "accepted", "{retried}");
    assert_eq!(retried["result"]["result"]["replayed"], true, "{retried}");

    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let text = fs::read_to_string(&prompts).unwrap_or_default();
        let accepted = durable_task_inputs(&target).iter().any(|input| {
            input["message"]
                .as_str()
                .is_some_and(|message| message.contains("durable-target-input-followup"))
                && input["provider_state"] == "accepted"
                && input["recovery"] == "history_absent"
        });
        if accepted
            && text
                .lines()
                .filter(|line| line.contains("durable-target-input-followup"))
                .count()
                == 1
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "target input was neither accepted nor uniquely delivered: {text}\n{:?}",
            durable_task_inputs(&target)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["state"] == "completed")
    });
    let replay = client.command(
        "task.send",
        json!({"threadId":parent,"args":send}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(replay["result"]["result"]["replayed"], true, "{replay}");
    assert_eq!(
        replay["result"]["result"]["operation"]["acceptance"],
        "accepted"
    );
    assert_eq!(
        fs::read_to_string(&prompts)
            .unwrap()
            .lines()
            .filter(|line| line.contains("durable-target-input-followup"))
            .count(),
        1
    );
    home.stop();
    target.stop();
}

/// The adapter durably accepted and saved the prompt, but the target died
/// before observing the response. Stable provider history identifies that
/// exact input after restart, so recovery settles it without a second prompt.
#[test]
fn target_acceptance_before_receipt_observation_is_reconciled_without_replay() {
    let mut target = Profile::new();
    let mut home = Profile::new();
    let port = free_port();
    let port_arg = port.to_string();
    target.launch(&["serve", "--port", &port_arg]);
    home.launch(&["serve"]);
    let target_discovery = target.discovery();
    let (_, _target_client, mut client, parent) = link_peer_servers(
        &mut target,
        &mut home,
        "https://example.com/target-acceptance-history.git",
    );
    let delegated = delegate_fixture(
        &mut client,
        &parent,
        "durable-target-accept-initial",
        "target-acceptance-launch",
        &target_discovery,
    );
    let child = delegated["result"]["result"]["thread_id"].clone();
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["state"] == "completed")
            && state["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|thread| thread["id"] == parent && thread["status"] == "Completed")
    });

    let send = json!({"thread_id":child,"message":"peer-crash-after-accept-fixture",
        "client_request_id":"target-acceptance-send"});
    let accepted = client.command(
        "task.send",
        json!({"threadId":parent,"args":send.clone()}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(accepted["result"]["status"], "accepted", "{accepted}");
    let prompts = target.root.join("workspace/fixture-prompts.jsonl");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let text = fs::read_to_string(&prompts).unwrap_or_default();
        if text
            .lines()
            .filter(|line| line.contains("peer-crash-after-accept-fixture"))
            .count()
            == 1
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "provider never accepted input: {text}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    target.crash_child(0);
    let _ = fs::remove_file(target.root.join("Shina/server.json"));
    target.launch(&["serve", "--port", &port_arg]);

    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let reconciled = durable_task_inputs(&target).iter().any(|input| {
            input["message"]
                .as_str()
                .is_some_and(|message| message.contains("peer-crash-after-accept-fixture"))
                && input["provider_state"] == "accepted"
                && input["recovery"] == "accepted_from_history"
                && input["provider_basis"] == "history_replay"
        });
        if reconciled {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "provider acceptance was not reconciled from history: {:?}",
            durable_task_inputs(&target)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let replay = client.command(
        "task.send",
        json!({"threadId":parent,"args":send}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(replay["result"]["result"]["replayed"], true, "{replay}");
    let text = fs::read_to_string(&prompts).unwrap();
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("peer-crash-after-accept-fixture"))
            .count(),
        1,
        "the accepted prompt must not be replayed: {text}"
    );
    home.stop();
    target.stop();
}

/// Rejecting the target's durable intent insert rolls back the accepted peer
/// command before the provider can be touched. The replay row and prompt log
/// both remain unchanged.
#[test]
fn target_intent_transaction_failure_has_no_adapter_or_peer_effect() {
    let mut target = Profile::new();
    let mut home = Profile::new();
    target.launch(&["serve"]);
    home.launch(&["serve"]);
    let target_discovery = target.discovery();
    let (_, _target_client, mut client, parent) = link_peer_servers(
        &mut target,
        &mut home,
        "https://example.com/target-intent-rollback.git",
    );
    let delegated = delegate_fixture(
        &mut client,
        &parent,
        "target-rollback-initial",
        "target-rollback-launch",
        &target_discovery,
    );
    let child = delegated["result"]["result"]["thread_id"].clone();
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["state"] == "completed")
    });
    let prompts = target.root.join("workspace/fixture-prompts.jsonl");
    let before_rows = durable_task_inputs(&target);
    let before_prompts = fs::read_to_string(&prompts).unwrap();
    let connection = rusqlite::Connection::open(target.root.join("Shina/state.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_task_input
             BEFORE INSERT ON task_input_intents
             BEGIN SELECT RAISE(ABORT,'injected target transaction failure'); END;",
        )
        .unwrap();
    drop(connection);

    let sent = client.command(
        "task.send",
        json!({"threadId":parent,"args":{"thread_id":child,
            "message":"target-rollback-followup","client_request_id":"target-rollback-send"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(sent["result"]["status"], "accepted", "{sent}");
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        assert_eq!(durable_task_inputs(&target).len(), before_rows.len());
        assert_eq!(fs::read_to_string(&prompts).unwrap(), before_prompts);
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !fs::read_to_string(&prompts)
            .unwrap()
            .contains("target-rollback-followup")
    );
    target.crash_child(0);
    home.stop();
}

/// A steer accepted for run A but held until A settles is delivered to run B
/// as an ordinary follow-up. The target records that realized mode and both
/// run identities against the same observed task attempt.
#[test]
fn late_steer_records_follow_up_and_exact_run_attribution() {
    let mut target = Profile::new();
    let mut home = Profile::new();
    let gate = target.root.join("peer-input-release");
    let steer_armed = target.root.join("peer-steer-armed");
    let steer_end = target.root.join("peer-steer-end");
    let steer_ended = target.root.join("peer-steer-ended");
    fs::write(&gate, b"ready").unwrap();
    fs::write(
        target.root.join("extra-env.json"),
        json!({"SHINA_TEST_HOLD_PEER_INPUT":gate,
            "SHINA_TEST_STEER_ARMED":steer_armed,"SHINA_TEST_STEER_END":steer_end,
            "SHINA_TEST_STEER_ENDED":steer_ended})
        .to_string(),
    )
    .unwrap();
    target.launch(&["serve"]);
    home.launch(&["serve"]);
    let target_discovery = target.discovery();
    let (_, _target_client, mut client, parent) = link_peer_servers(
        &mut target,
        &mut home,
        "https://example.com/late-steer-attribution.git",
    );
    let delegated = delegate_fixture(
        &mut client,
        &parent,
        "peer-steer-race-fixture",
        "late-steer-launch",
        &target_discovery,
    );
    let child = delegated["result"]["result"]["thread_id"].clone();
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["state"] == "running")
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !steer_armed.exists() {
        assert!(
            Instant::now() < deadline,
            "fixture never armed steer release"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let attempt = client.snapshot["index"]["tasks"][0]["attempt_id"].clone();
    assert!(attempt.is_string(), "the observed attempt is required");

    fs::remove_file(&gate).unwrap();
    let steer = json!({"thread_id":child,"message":"late-steer-followup",
        "client_request_id":"late-steer-command","delivery":"steer","attempt_id":attempt});
    let accepted = client.command(
        "task.send",
        json!({"threadId":parent,"args":steer.clone()}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(accepted["result"]["status"], "accepted", "{accepted}");
    let target_attempt = accepted["result"]["result"]["attempt_id"].clone();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let durable = durable_task_inputs(&target);
        if durable.iter().any(|input| {
            input["message"]
                .as_str()
                .is_some_and(|message| message.contains("late-steer-followup"))
                && input["requested_delivery"] == "steer"
                && input["queue_state"] == "queued"
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "steer was not stored: {durable:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    let workspace = target.root.join("workspace");
    fs::write(&steer_end, b"end").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !steer_ended.exists() {
        assert!(Instant::now() < deadline, "fixture run A did not settle");
        std::thread::sleep(Duration::from_millis(25));
    }
    fs::write(&gate, b"release").unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let (target_run, realized_run) = loop {
        let durable = durable_task_inputs(&target);
        if let Some(input) = durable.iter().find(|input| {
            input["message"]
                .as_str()
                .is_some_and(|message| message.contains("late-steer-followup"))
                && input["provider_state"] == "accepted"
                && input["realized_delivery"] == "follow_up"
        }) {
            assert_eq!(input["attempt_id"], target_attempt, "{input}");
            assert!(input["target_run_id"].is_string(), "{input}");
            assert!(input["run_id"].is_string(), "{input}");
            assert_ne!(input["target_run_id"], input["run_id"], "{input}");
            break (input["target_run_id"].clone(), input["run_id"].clone());
        }
        assert!(
            Instant::now() < deadline,
            "late steer did not reach provider as follow-up: {durable:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_ne!(target_run, realized_run);
    let behavior = fs::read_to_string(workspace.join("fixture-behavior.jsonl")).unwrap();
    let record = behavior
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|record| {
            record["message"]
                .as_str()
                .is_some_and(|message| message.contains("late-steer-followup"))
        })
        .expect("late steer provider record");
    assert_eq!(record["streamingBehavior"], "followUp");
    home.stop();
    target.stop();
}

/// A cancel accepted while the fixed-link target is offline stays unobserved
/// and blocks a newer send. Restarting the target at the same listener and
/// identity lets the fenced stop settle without affecting later work.
#[test]
fn offline_cancel_stays_uncertain_until_target_settlement_and_never_stops_newer_attempt() {
    let mut target = Profile::new();
    let mut home = Profile::new();
    let port = free_port();
    let port_arg = port.to_string();
    let gate = home.root.join("peer-stop-release");
    fs::write(&gate, b"ready").unwrap();
    fs::write(
        home.root.join("extra-env.json"),
        json!({"SHINA_TEST_PEER_SEND_BARRIER":gate}).to_string(),
    )
    .unwrap();
    target.launch(&["serve", "--port", &port_arg]);
    home.launch(&["serve"]);
    let target_discovery = target.discovery();
    let (_, _target_client, mut client, parent) = link_peer_servers(
        &mut target,
        &mut home,
        "https://example.com/offline-cancel-newer-attempt.git",
    );
    let delegated = delegate_fixture(
        &mut client,
        &parent,
        "peer-documents-hold-fixture",
        "offline-cancel-launch",
        &target_discovery,
    );
    let child = delegated["result"]["result"]["thread_id"].clone();
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["state"] == "running")
    });
    let attempt_a = client.snapshot["index"]["tasks"][0]["attempt_id"].clone();
    fs::remove_file(&gate).unwrap();
    let cancel_args = json!({"thread_id":child,"client_request_id":"offline-cancel",
        "attempt_id":attempt_a});
    let cancelled = client.command(
        "task.cancel",
        json!({"threadId":parent,"args":cancel_args.clone()}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(cancelled["result"]["status"], "accepted", "{cancelled}");
    client.until(|state| {
        state["index"]["taskOperations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| operation["request_key"] == "offline-cancel")
    });
    let operation = client.snapshot["index"]["taskOperations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|operation| operation["request_key"] == "offline-cancel")
        .unwrap()
        .clone();
    assert_eq!(operation["completion"], "not_observed");
    assert_ne!(operation["transport"], "accepted");
    assert_eq!(client.snapshot["index"]["tasks"][0]["state"], "cancelled");

    let blocked = client.command(
        "task.send",
        json!({"threadId":parent,"args":{"thread_id":child,
            "message":"offline-cancel-newer-attempt","client_request_id":"offline-cancel-next"}}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        blocked.to_string().contains("owed stop") || blocked.to_string().contains("acknowledge"),
        "{blocked}"
    );

    fs::write(&gate, b"release").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !durable_task_inputs(&target)
        .iter()
        .any(|intent| intent["kind"] == "cancel")
    {
        assert!(Instant::now() < deadline, "target never received the stop");
        std::thread::sleep(Duration::from_millis(25));
    }
    // Reset only the controller's retry schedule after the target has the
    // durable stop. The same target listener and peer credential stay in use.
    home.stop();
    let _ = fs::remove_file(home.root.join("Shina/server.json"));
    home.launch(&["serve"]);
    let mut client = home.client(uuid::Uuid::new_v4());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let replay = client.command(
            "task.cancel",
            json!({"threadId":parent,"args":cancel_args.clone()}),
            uuid::Uuid::new_v4(),
        );
        if replay["result"]["result"]["replayed"] == true
            && replay["result"]["result"]["operation"]["completion"] == "cancelled"
        {
            break;
        }
        if Instant::now() >= deadline {
            panic!(
                "stop settlement was not observed: {replay}\nTarget inputs: {:?}",
                durable_task_inputs(&target),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let next = client.command(
        "task.send",
        json!({"threadId":parent,"args":{"thread_id":child,
            "message":"offline-cancel-newer-attempt","client_request_id":"offline-cancel-next"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(next["result"]["status"], "accepted", "{next}");
    assert_ne!(next["result"]["result"]["attempt_id"], attempt_a);
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["state"] == "running" || task["state"] == "completed")
    });
    let stale = client.command(
        "task.cancel",
        json!({"threadId":parent,"args":cancel_args}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(stale["result"]["result"]["replayed"], true, "{stale}");
    assert_ne!(client.snapshot["index"]["tasks"][0]["state"], "cancelled");
    client.until(|state| {
        state["index"]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["state"] == "completed")
    });
    target.stop();
    let _ = fs::remove_file(target.root.join("Shina/server.json"));
    target.launch(&["serve", "--port", &port_arg]);
    let replay = client.command(
        "task.send",
        json!({"threadId":parent,"args":{"thread_id":child,
            "message":"offline-cancel-newer-attempt","client_request_id":"offline-cancel-next"}}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(replay["result"]["result"]["replayed"], true, "{replay}");
    home.stop();
    target.stop();
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
fn acp_without_history_replay_keeps_the_server_transcript_on_resume() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let mut client = profile.client(device);
    client.command(
        "thread.configure",
        json!({"threadId":1,"harness":"fixture-acp"}),
        uuid::Uuid::new_v4(),
    );
    client.command(
        "thread.send",
        json!({"threadId":1,"text":"before restart","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    client.until(|state| state["threads"][0]["status"] == "Completed");
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(device);
    client.command(
        "thread.send",
        json!({"threadId":1,"text":"after restart","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    client.until(|state| {
        state["threads"][0]["status"] == "Completed"
            && state["threads"][0]["messages"]
                .as_array()
                .is_some_and(|messages| messages.iter().any(|m| m["text"] == "after restart"))
    });
    let messages = client.snapshot["threads"][0]["messages"]
        .as_array()
        .unwrap();
    assert!(messages.iter().any(|m| m["text"] == "before restart"));
    assert_eq!(
        messages
            .iter()
            .filter(|m| m["text"] == "after restart")
            .count(),
        1
    );
    profile.stop();
}

#[test]
fn a_lost_send_acknowledgement_retries_the_same_id_without_a_second_prompt() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let device = uuid::Uuid::new_v4();
    let id = uuid::Uuid::new_v4();
    let args = json!({"threadId":1,"text":"lost acknowledgement","attachments":[],"mode":"prompt"});
    let mut first = profile.client(device);
    first.socket.send(Message::Text(json!({"jsonrpc":"2.0","id":id,"method":"thread.send","params":{"commandId":id,"args":args}}).to_string().into())).unwrap();
    drop(first);
    let mut second = profile.client(device);
    assert_eq!(
        second.command("thread.send", args, id)["result"]["status"],
        "accepted"
    );
    second.until(|state| state["threads"][0]["status"] == "Completed");
    assert_eq!(
        second.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["text"] == "lost acknowledgement")
            .count(),
        1
    );
    profile.stop();
}

#[test]
fn restart_negotiation_fences_and_cancels_without_losing_the_server() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let old_client = client.command("server.prepareRestart", json!({}), uuid::Uuid::new_v4());
    assert!(
        old_client["error"]["message"]
            .as_str()
            .unwrap()
            .contains("older client")
    );
    let mismatch = client.rpc(
        "hello",
        json!({"protocolMajor":99,"protocolMinor":0,"projectionSchema":1}),
        json!("bad-version"),
    );
    assert!(
        mismatch["error"]["message"]
            .as_str()
            .unwrap()
            .contains("shina update")
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
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":999,"projectionSchema":1,"clientPid":4242}),
        json!("good-version"),
    );
    assert_eq!(hello["result"]["protocolMinor"], protocol::MINOR);
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability == "peer-delegation")
    );
    let prepared =
        client.command("server.prepareRestart", json!({}), uuid::Uuid::new_v4())["result"].clone();
    assert_eq!(prepared["phase"], "draining");
    assert_eq!(prepared["clientsReady"], false);
    let blocked = client.command(
        "thread.rename",
        json!({"threadId":1,"title":"fenced"}),
        uuid::Uuid::new_v4(),
    );
    assert!(
        blocked["error"]["message"]
            .as_str()
            .unwrap()
            .contains("draining")
    );
    let commit = client.command(
        "server.commitRestart",
        prepared.clone(),
        uuid::Uuid::new_v4(),
    );
    assert!(
        commit["error"]["message"]
            .as_str()
            .unwrap()
            .contains("clients")
    );
    let cancelled = client.command("server.cancelRestart", prepared, uuid::Uuid::new_v4());
    assert_eq!(cancelled["result"]["phase"], "ready");
    let renamed = client.command(
        "thread.rename",
        json!({"threadId":1,"title":"resumed"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(renamed["result"]["status"], "accepted");
    profile.stop();
}

#[test]
fn durable_subscriptions_replay_patches_and_fall_back_to_compact_snapshots() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut reader = profile.client(uuid::Uuid::new_v4());
    reader.rpc("subscription.cancel", json!({}), json!("cancel"));
    let after = reader.seq;
    let epoch = reader.epoch.clone();
    let mut writer = profile.client(uuid::Uuid::new_v4());
    writer.command(
        "thread.rename",
        json!({"threadId":1,"title":"Durable rename"}),
        uuid::Uuid::new_v4(),
    );
    writer.until(|state| state["index"]["threads"][0]["title"] == "Durable rename");
    reader.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":1,"projectionSchema":1}),
        json!("hello-v1"),
    );
    reader.notifications.clear();
    reader.rpc(
        "workspace.subscribe",
        json!({"lastSeq":after,"epoch":epoch}),
        json!("resume"),
    );
    assert!(
        reader
            .notifications
            .iter()
            .any(|value| value["method"] == "subscription.patch")
    );
    assert!(
        !reader
            .notifications
            .iter()
            .any(|value| value["method"] == "subscription.snapshot")
    );
    reader.notifications.clear();
    reader.rpc(
        "workspace.subscribe",
        json!({"lastSeq":u64::MAX,"epoch":epoch}),
        json!("future-cursor"),
    );
    assert!(
        reader
            .notifications
            .iter()
            .any(|value| value["method"] == "subscription.snapshot")
    );
    assert!(reader.snapshot["threads"][0]["historyRevision"].is_string());
    assert!(
        reader.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    profile.stop();
}

#[test]
fn protocol_one_zero_negotiates_snapshots_and_clear_restart_guidance() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":0,"projectionSchema":1}),
        json!("old-hello"),
    );
    assert_eq!(hello["result"]["protocolMinor"], 0);
    assert!(
        !hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "projection-patches")
    );
    client.notifications.clear();
    client.rpc(
        "workspace.subscribe",
        json!({"lastSeq":client.seq,"epoch":client.epoch}),
        json!("old-subscribe"),
    );
    assert!(
        client
            .notifications
            .iter()
            .any(|n| n["method"] == "subscription.snapshot")
    );
    assert!(
        !client
            .notifications
            .iter()
            .any(|n| n["method"] == "subscription.patch")
    );
    let prepared = client.command("server.prepareRestart", json!({}), uuid::Uuid::new_v4());
    assert!(
        prepared["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Close older clients")
    );
    profile.stop();
}

#[test]
fn unauthorized_socket_cannot_prepare_restart() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let discovery = profile.discovery();
    let mut request = format!(
        "ws://127.0.0.1:{}/v1/ws",
        discovery["port"].as_u64().unwrap()
    )
    .into_client_request()
    .unwrap();
    request.headers_mut().insert(
        "authorization",
        "Bearer not-a-device-credential".parse().unwrap(),
    );
    assert!(
        matches!(tungstenite::connect(request), Err(tungstenite::Error::Http(response)) if response.status().as_u16() == 401)
    );
    let mut client = profile.client(uuid::Uuid::new_v4());
    assert_eq!(
        client.rpc("server.status", json!({}), json!("status"))["result"]["phase"],
        "ready"
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

#[test]
fn service_launcher_logs_start_and_spawn_failure() {
    let profile = Profile::new();
    let directory = profile.root.join("Shina");
    let descriptor = directory.join("service.json");
    fs::write(&descriptor, json!({
        "platform":if cfg!(windows) { "Windows" } else { "Linux" },
        "name":"Shina-failed-launch-fixture",
        "executable":profile.root.join("missing-server"),
        "directory":directory,"unit":directory.join("unused"),"home":profile.root,"environment":{}
    }).to_string()).unwrap();
    let output = profile.cli(&["--service-run", descriptor.to_str().unwrap()]);
    assert!(!output.status.success());
    let log = fs::read_to_string(directory.join("service.log")).unwrap();
    assert!(log.contains("Service launcher") && log.contains("started."));
    assert!(log.contains("Service server failed:"));
}

#[cfg(windows)]
#[test]
#[ignore = "Creates a real temporary per-user scheduled task. Run explicitly on Windows, no elevation."]
fn scheduled_service_coordinated_update_waits_for_task_action_and_confirms_new_launcher() {
    use sha2::{Digest, Sha256};
    use std::os::windows::process::CommandExt;

    // Drop this before Profile removes the fresh UUID directory, even on assertion failure.
    struct TaskCleanup {
        name: String,
        root: PathBuf,
    }
    impl Drop for TaskCleanup {
        fn drop(&mut self) {
            let _ = Command::new(env!("CARGO_BIN_EXE_shina"))
                .args(["service", "uninstall"])
                .env("LOCALAPPDATA", &self.root)
                .creation_flags(0x08000000)
                .output();
            let _ = Command::new("schtasks.exe")
                .args(["/Delete", "/TN", &self.name, "/F"])
                .creation_flags(0x08000000)
                .output();
            // Only exact executable paths inside this UUID fixture may be terminated.
            let script = format!(
                "$r='{}\\'; Get-CimInstance Win32_Process | Where-Object {{ $_.ExecutablePath -and $_.ExecutablePath.StartsWith($r, [StringComparison]::OrdinalIgnoreCase) }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue; Wait-Process -Id $_.ProcessId -Timeout 5 -ErrorAction SilentlyContinue }}",
                self.root.to_string_lossy().replace('\'', "''")
            );
            let _ = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .creation_flags(0x08000000)
                .output();
            assert!(
                !Command::new("schtasks.exe")
                    .args(["/Query", "/TN", &self.name])
                    .creation_flags(0x08000000)
                    .output()
                    .unwrap()
                    .status
                    .success(),
                "temporary scheduled task was not removed"
            );
        }
    }

    let mut profile = Profile::new();
    let name = format!("Shina-update-test-{}", uuid::Uuid::new_v4().simple());
    let _cleanup = TaskCleanup {
        name: name.clone(),
        root: profile.root.clone(),
    };
    let directory = profile.root.join("Shina");
    let install = profile.root.join("Programs/Shina");
    fs::create_dir_all(&install).unwrap();
    let exe = install.join("shina.exe");
    fs::copy(env!("CARGO_BIN_EXE_shina"), &exe).unwrap();
    let installed_cli = |args: &[&str]| {
        Command::new(&exe)
            .args(args)
            .env("LOCALAPPDATA", &profile.root)
            .env("PI_CODING_AGENT_DIR", profile.root.join("enrollment"))
            .env("SHINA_PI_CLI", profile.root.join("fake_pi.js"))
            .stdin(Stdio::null())
            .creation_flags(0x08000000)
            .output()
            .unwrap()
    };
    let installed = installed_cli(&["service", "install", "--name", &name]);
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    let initial = profile.discovery();
    assert!(installed_cli(&["service", "stop"]).status.success());

    // Existing installations still have the tree-wide wait in Task Scheduler.
    // A normal start must upgrade that exact action without reinstalling the service.
    let legacy = format!(
        "$ErrorActionPreference='Stop'; $s=New-Object -ComObject Schedule.Service; $s.Connect(); $f=$s.GetFolder('\\'); $d=$f.GetTask('{name}').Definition; $d.Actions.Item(1).Arguments=$d.Actions.Item(1).Arguments.Replace('-PassThru; $p.WaitForExit();','-PassThru -Wait;'); $null=$f.RegisterTaskDefinition('{name}',$d,4,$null,$null,3,$null)"
    );
    let registered = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &legacy])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        registered.status.success(),
        "{}",
        String::from_utf8_lossy(&registered.stderr)
    );
    assert!(installed_cli(&["service", "start"]).status.success());
    let upgraded = format!(
        "$ErrorActionPreference='Stop'; $s=New-Object -ComObject Schedule.Service; $s.Connect(); $a=$s.GetFolder('\\').GetTask('{name}').Definition.Actions.Item(1).Arguments; if (-not $a.Contains('$p.WaitForExit();') -or $a.Contains('-PassThru -Wait;')) {{ throw 'Legacy task action was not upgraded' }}"
    );
    let upgraded = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &upgraded])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        upgraded.status.success(),
        "{}",
        String::from_utf8_lossy(&upgraded.stderr)
    );
    assert!(installed_cli(&["service", "stop"]).status.success());

    // Keep the actual task action alive after the launcher exits. This makes the
    // release-binary race deterministic without touching any system setting.
    let unit = directory.join("service-task.xml");
    let bytes = fs::read(&unit).unwrap();
    let words: Vec<_> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let xml = String::from_utf16(&words).unwrap().replace(
        "exit $p.ExitCode",
        "Start-Sleep -Seconds 3; exit $p.ExitCode",
    );
    fs::write(
        &unit,
        xml.encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let registered = Command::new("schtasks.exe")
        .args([
            "/Create",
            "/TN",
            &name,
            "/XML",
            unit.to_str().unwrap(),
            "/F",
        ])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        registered.status.success(),
        "{}",
        String::from_utf8_lossy(&registered.stderr)
    );
    assert!(installed_cli(&["service", "start"]).status.success());
    let running = profile.discovery();
    assert_ne!(initial["instanceId"], running["instanceId"]);
    assert!(installed_cli(&["service", "stop"]).status.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while directory.join("service-launcher.pid").exists() {
        assert!(Instant::now() < deadline, "old launcher did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
    let probe = format!(
        "$ErrorActionPreference='Stop'; $s=New-Object -ComObject Schedule.Service; $s.Connect(); $t=$s.GetFolder('\\').GetTask('{}'); if ($t.State -ne 4) {{ throw 'Expected the old wrapper to remain Running' }}; schtasks.exe /Run /TN '{}'; if ($LASTEXITCODE -ne 0) {{ throw 'Expected /Run to return success' }}; Write-Output 'Old launcher absent, task Running, /Run exit 0'",
        name, name
    );
    let dropped = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &probe])
        .creation_flags(0x08000000)
        .output()
        .unwrap();
    assert!(
        dropped.status.success(),
        "{}",
        String::from_utf8_lossy(&dropped.stderr)
    );
    eprintln!("{}", String::from_utf8_lossy(&dropped.stdout));
    assert!(
        !directory.join("service-launcher.pid").exists(),
        "IgnoreNew should drop the request"
    );
    assert!(installed_cli(&["service", "start"]).status.success());
    let running = profile.discovery();
    let launcher = fs::read_to_string(directory.join("service-launcher.pid")).unwrap();

    // Exercise a pre-open version-change backup and the validation fence as well.
    fs::write(
        directory.join("server-binary-version.json"),
        "previous-fixture-version",
    )
    .unwrap();
    let updates = directory.join("updates");
    fs::create_dir_all(&updates).unwrap();
    let prepared = install.join("shina.new.exe");
    let download = updates.join("download.exe");
    fs::copy(&exe, &download).unwrap();
    let helper = updates.join("coordinated-helper.exe");
    fs::copy(&exe, &helper).unwrap();
    let hash = format!("{:x}", Sha256::digest(fs::read(&download).unwrap()));
    for tag in [format!("v{}", env!("CARGO_PKG_VERSION")), "v99.0.0".into()] {
        fs::copy(&exe, &prepared).unwrap();
        fs::write(
            updates.join("ready.json"),
            json!({
                "version":1,"tag":tag,"asset":"shina.exe",
                "size":fs::metadata(&download).unwrap().len(),"sha256":hash,
                "download":download,"prepared":prepared,"target":{"windows":exe}
            })
            .to_string(),
        )
        .unwrap();
        profile.children.push(
            Command::new(&helper)
                .arg("--coordinated-update")
                .env("LOCALAPPDATA", &profile.root)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x08000000)
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        while profile
            .children
            .last_mut()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_none()
        {
            assert!(
                Instant::now() < deadline,
                "coordinated service update timed out"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        let result = fs::read_to_string(updates.join("coordinated-result.txt")).unwrap();
        if tag == "v99.0.0" {
            assert!(result.contains("Readiness validation failed:"));
            assert!(result.contains(&format!(
                "version {}, expected v99.0.0",
                env!("CARGO_PKG_VERSION")
            )));
            assert!(result.contains("Previous binary and state restored."));
            let failure: Value =
                serde_json::from_slice(&fs::read(updates.join("failed-release.json")).unwrap())
                    .unwrap();
            assert_eq!(failure["tag"], "v99.0.0");
        } else {
            assert_eq!(result, "Update completed.");
        }
        assert_ne!(
            launcher,
            fs::read_to_string(directory.join("service-launcher.pid")).unwrap()
        );
        let resumed = profile.discovery();
        assert_ne!(running["instanceId"], resumed["instanceId"]);
        assert_eq!(resumed["binaryVersion"], env!("CARGO_PKG_VERSION"));
        assert!(!directory.join("update-validation").exists());
        assert!(fs::read_dir(directory.join("backups")).unwrap().count() >= 2);
        let mut client = profile.client(uuid::Uuid::new_v4());
        assert!(
            client.command("server.status", json!({}), uuid::Uuid::new_v4())["error"].is_null()
        );
    }
    assert!(installed_cli(&["service", "stop"]).status.success());
}

#[test]
fn committed_restart_waits_for_client_acknowledgement_then_exits_cleanly() {
    let mut profile = Profile::new();
    let process = profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":1,"projectionSchema":1,"clientPid":4242}),
        json!("register"),
    );
    let mut prepared =
        client.command("server.prepareRestart", json!({}), uuid::Uuid::new_v4())["result"].clone();
    prepared["pid"] = json!(4242);
    let ack = client.command("server.clientReady", prepared.clone(), uuid::Uuid::new_v4());
    assert_eq!(ack["result"]["clientsReady"], true);
    let commit = client.command("server.commitRestart", prepared, uuid::Uuid::new_v4());
    assert_eq!(commit["result"]["phase"], "restarting");
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = profile.children[process].try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "committed server did not exit");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(!profile.root.join("Shina/server.json").exists());
    assert_eq!(
        fs::read_to_string(profile.root.join("Shina/restart-flushed")).unwrap(),
        commit["result"]["updateId"].as_str().unwrap()
    );
}

#[test]
fn history_version_skew_is_negotiated_or_reports_update_required() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":999,"projectionSchema":1,"futureField":true}),
        json!("future-hello"),
    );
    assert_eq!(hello["result"]["protocolMinor"], protocol::MINOR);
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == protocol::HISTORY_ITEMS)
    );
    let history = client.rpc(
        "thread.subscribe",
        json!({"threadId":1,"futureField":"ignored"}),
        json!("future-history"),
    );
    assert!(history["error"].is_null(), "{history}");
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":6,"projectionSchema":1}),
        json!("old-hello"),
    );
    assert!(
        !hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == protocol::HISTORY_ITEMS)
    );
    let history = client.rpc(
        "thread.subscribe",
        json!({"threadId":1}),
        json!("unsupported-history"),
    );
    assert_eq!(
        history["error"]["code"],
        protocol::HISTORY_UPDATE_REQUIRED_CODE
    );
    assert!(
        history["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Update required.")
    );
    profile.stop();
}

#[test]
fn streaming_history_walk_finishes_before_settlement() {
    streaming_history_case(200_000, false);
}

#[test]
fn streaming_storage_scaling() {
    let mut template = Profile::new();
    template.launch(&["serve"]);
    let client = template.client(uuid::Uuid::new_v4());
    let base = client.snapshot.clone();
    drop(client);
    template.stop();
    let mut costs = Vec::new();
    for count in [10, 100, 500] {
        let mut profile = Profile::new();
        let mut index = base["index"].clone();
        let mut records = Vec::new();
        let mut views = Vec::new();
        for id in 1..=count {
            let mut record = index["threads"][0].clone();
            record["id"] = json!(id);
            record["session"] = Value::Null;
            records.push(record);
            let mut view = base["threads"][0].clone();
            view["id"] = json!(id);
            view["status"] = json!("Completed");
            view["messages"] = json!([{"role":"You","text":"x".repeat(if id == 1 {190_000} else {4096}),"images":[]}]);
            views.push(view);
        }
        index["threads"] = json!(records);
        index["threadViews"] = json!(views);
        index["nextId"] = json!(count + 1);
        reset_test_thread_identities(&mut index, 1..=count);
        fs::write(profile.root.join("Shina/index.json"), index.to_string()).unwrap();
        fs::write(profile.root.join("measure-storage"), "").unwrap();
        profile.launch(&["serve"]);
        let mut client = profile.client(uuid::Uuid::new_v4());
        client.rpc(
            "hello",
            json!({"protocolMajor":1,"protocolMinor":7,"projectionSchema":1}),
            json!("measure-hello"),
        );
        client.rpc("workspace.subscribe", json!({}), json!("measure-subscribe"));
        let db = rusqlite::Connection::open(profile.root.join("Shina/state.sqlite")).unwrap();
        // Startup work (catalog loads, migrations) finishes at a time that depends on
        // the machine, so a fixed pause is not a reliable start for the idle window.
        // An idle server must reach five consecutive seconds without a WAL write.
        let wal = profile.root.join("Shina/state.sqlite-wal");
        let wal_len = || fs::metadata(&wal).unwrap().len();
        let mut last = wal_len();
        let mut quiet = 0;
        for _ in 0..90 {
            std::thread::sleep(Duration::from_secs(1));
            let now = wal_len();
            quiet = if now == last { quiet + 1 } else { 0 };
            last = now;
            if quiet == 5 {
                break;
            }
        }
        assert_eq!(quiet, 5, "idle workspace kept writing to the WAL");
        let receipt = client.command(
            "thread.send",
            json!({"threadId":1,"text":"history-stream-fixture","attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(receipt["result"]["status"], "accepted", "{receipt}");
        std::thread::sleep(Duration::from_secs(3));
        let observed = fs::read_to_string(profile.root.join("storage.jsonl")).unwrap();
        // A writer may be in the middle of its last JSONL record.
        let complete = observed
            .rsplit_once('\n')
            .map(|(complete, _)| complete)
            .unwrap_or("");
        let samples: Vec<Value> = complete
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let start = receipt["result"]["seq"].as_u64().unwrap();
        let samples: Vec<_> = samples
            .iter()
            .filter(|s| s["seq"].as_u64().unwrap() > start)
            .collect();
        let serialized: u64 = samples
            .iter()
            .map(|s| s["serialized"].as_u64().unwrap())
            .sum();
        let storage: u64 = samples.iter().map(|s| s["storage"].as_u64().unwrap()).sum();
        println!(
            "STORAGE threads={count} updates={} serialized/update={} WAL/update={} sequence_advances={} idle_WAL={}",
            samples.len(),
            serialized / samples.len() as u64,
            storage / samples.len() as u64,
            samples.len(),
            0
        );
        costs.push((
            serialized / samples.len() as u64,
            storage / samples.len() as u64,
        ));
        drop(db);
        profile.stop();
    }
    assert!(costs[2].0 <= costs[0].0 * 2, "{costs:?}");
    assert!(costs[2].1 <= costs[0].1 * 2, "{costs:?}");
}

#[test]
fn scoped_history_resumes_after_server_restart() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":7,"projectionSchema":1}),
        json!("resume-hello"),
    );
    let receipt = client.command(
        "thread.send",
        json!({"threadId":1,"text":"first","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    client.until_sequence(receipt["result"]["seq"].as_u64().unwrap(), |s| {
        s["threads"][0]["status"] == "Completed"
    });
    let initial = client.rpc(
        "thread.subscribe",
        json!({"threadId":1}),
        json!("initial-history"),
    );
    let page: Value = serde_json::from_str(initial["result"]["data"].as_str().unwrap()).unwrap();
    let sequence = page["snapshotSequence"].clone();
    let identity = page["items"][0]["sourceItemId"].clone();
    let epoch = initial["result"]["streamEpoch"].clone();
    let receipt = client.command(
        "thread.send",
        json!({"threadId":1,"text":"second","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    client.until_sequence(receipt["result"]["seq"].as_u64().unwrap(), |s| {
        s["threads"][0]["status"] == "Completed"
    });
    drop(client);
    profile.stop();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":7,"projectionSchema":1}),
        json!("resume-hello"),
    );
    let resumed = client.rpc(
        "thread.subscribe",
        json!({"threadId":1,"afterSequence":sequence,"streamEpoch":epoch}),
        json!("resumed-history"),
    );
    assert_eq!(resumed["result"]["kind"], "synchronized", "{resumed}");
    assert_eq!(resumed["result"]["streamEpoch"], epoch);
    let current = client.rpc(
        "thread.subscribe",
        json!({"threadId":1}),
        json!("current-history"),
    );
    let page: Value = serde_json::from_str(current["result"]["data"].as_str().unwrap()).unwrap();
    assert_eq!(page["items"][0]["sourceItemId"], identity);
    profile.stop();
}

#[test]
fn streaming_history_two_mib_walk_finishes_before_settlement() {
    streaming_history_case(2 * 1024 * 1024, false);
}

#[test]
fn streaming_history_resumes_after_disconnect() {
    streaming_history_case(200_000, true);
}

#[test]
fn reconnect_attaches_the_retained_cache_before_any_workspace_subscription() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut actor = profile.client(uuid::Uuid::new_v4());
    std::thread::sleep(Duration::from_secs(1));
    actor.rpc("workspace.subscribe", json!({}), json!("base"));
    let mut retained = client_history::ProjectionCache::default();
    retained
        .receive(&protocol::notification(
            "subscription.snapshot",
            json!({
                "state":protocol::compact(&actor.snapshot),"seq":actor.seq,"epoch":actor.epoch
            }),
        ))
        .unwrap();
    let retained_sequence = retained.seq.unwrap();
    let missed = actor.command(
        "thread.rename",
        json!({"threadId":1,"title":"missed base"}),
        uuid::Uuid::new_v4(),
    );
    actor.until_sequence(missed["result"]["seq"].as_u64().unwrap(), |_| true);

    // Reproduce the old fixture. Its convenience constructor consumes a newer
    // snapshot without giving it to the retained cache.
    let mut discarded = Client::with_token(&profile.discovery(), actor.token.clone());
    let hello = discarded.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":7,"projectionSchema":1}),
        json!("old-hello"),
    );
    let installed_sequence = discarded.seq;
    assert!(installed_sequence > retained_sequence);
    let receipt = actor.command(
        "thread.rename",
        json!({"threadId":1,"title":"contiguous server patch"}),
        uuid::Uuid::new_v4(),
    );
    actor.until_sequence(receipt["result"]["seq"].as_u64().unwrap(), |_| true);
    let wire = loop {
        let value = discarded.read();
        if value["method"] == "subscription.patch" {
            break value;
        }
    };
    assert_eq!(
        wire["params"]["seq"],
        installed_sequence + 1,
        "the server must send the next sequence for the snapshot actually consumed"
    );
    assert!(
        retained
            .receive(&wire)
            .unwrap_err()
            .contains("Projection replay gap")
    );
    drop(discarded);

    // Native reconnect installs the retained base before its one subscription,
    // and requests replay from that base rather than discarding another snapshot.
    let mut resumed = Client::unsubscribed(&profile.discovery(), actor.token.clone());
    resumed.snapshot = retained.state.clone();
    resumed.seq = retained_sequence;
    resumed.epoch = retained.epoch.clone();
    retained.reconnect(&hello["result"]).unwrap();
    let cursor = json!({"lastSeq":retained.seq,"epoch":retained.epoch});
    resumed.history_cache = Some(retained);
    resumed.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":7,"projectionSchema":1}),
        json!("native-hello"),
    );
    resumed.rpc("workspace.subscribe", cursor, json!("native-subscribe"));
    assert!(
        resumed.history_cache.as_ref().unwrap().seq.unwrap()
            >= receipt["result"]["seq"].as_u64().unwrap()
    );
    assert_eq!(
        resumed.history_cache.as_ref().unwrap().state["index"]["threads"][0]["title"],
        "contiguous server patch"
    );
    assert!(
        !resumed
            .notifications
            .iter()
            .any(|value| value["method"] == "subscription.snapshot")
    );
    profile.stop();
}

fn streaming_history_case(bytes: usize, disconnect: bool) {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":7,"projectionSchema":1}),
        json!("paging-hello"),
    );
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == protocol::HISTORY_ITEMS)
    );
    client.rpc("workspace.subscribe", json!({}), json!("paging-subscribe"));
    for chunk in (0..bytes).step_by(750_000) {
        let receipt = client.command("thread.send",
            json!({"threadId":1,"text":"x".repeat((bytes - chunk).min(750_000)),"attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4());
        assert_eq!(receipt["result"]["status"], "accepted");
        client.until_sequence(receipt["result"]["seq"].as_u64().unwrap(), |state| {
            state["threads"][0]["status"] == "Completed"
        });
    }
    let receipt = client.command(
        "thread.send",
        json!({"threadId":1,"text":"history-stream-fixture","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(receipt["result"]["status"], "accepted");
    client.until(|state| state["threads"][0]["status"] == "Running");
    let mut cache = client_history::ProjectionCache::default();
    cache.reconnect(&hello["result"]).unwrap();
    // This fixture already subscribed before attaching its scoped-history cache.
    // Seed that installed workspace so a queued live patch has a valid base.
    cache
        .receive(&protocol::notification(
            "subscription.snapshot",
            json!({
                "state":protocol::compact(&client.snapshot),"seq":client.seq,"epoch":client.epoch
            }),
        ))
        .unwrap();
    client.history_cache = Some(cache);
    client.rpc("workspace.subscribe", json!({}), json!("history-subscribe"));
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut displayed: Option<client_history::Projection> = None;
    let mut first_live_len = None;
    let mut live_advanced = false;
    let mut reconnected = false;
    let mut full_walks = 0;
    let mut replays = 0;
    let mut page_requests = 0;
    let mut requests_at_completion = None;
    let mut first_walk_complete = false;
    loop {
        assert!(
            Instant::now() < deadline,
            "streaming history did not converge"
        );
        let cache = client.history_cache.as_mut().unwrap();
        if let Some(view) = cache.projected() {
            displayed = Some(view);
        }
        let installed = cache.installed_info(1);
        if let Some((revision, view, size)) = installed.as_ref() {
            if !first_walk_complete && cache.complete(1) {
                assert!(*size > bytes);
                assert_eq!(
                    cache.state["threads"][0]["status"], "Running",
                    "walk must complete during streaming"
                );
                first_walk_complete = true;
                requests_at_completion = Some(page_requests);
            }
            let assistant = view["messages"].as_array().unwrap().last().unwrap()["text"]
                .as_str()
                .unwrap()
                .to_string();
            if cache.state["threads"][0]["status"] == "Running" {
                assert_eq!(
                    displayed.as_ref().unwrap().views[&1].fields["messages"]
                        .items
                        .last()
                        .unwrap()["text"],
                    assistant
                );
                if let Some(first) = first_live_len {
                    live_advanced |= assistant.len() > first;
                } else {
                    first_live_len = Some(assistant.len());
                }
            }
            if cache.state["threads"][0]["status"] == "Completed"
                && cache.state["threads"][0]["historyRevision"] == *revision
                && cache.complete(1)
            {
                assert!(
                    live_advanced,
                    "displayed tail must advance before settlement"
                );
                let expected: String = (0..250)
                    .map(|tick| format!("stream-{tick:03} 😀\n"))
                    .collect();
                assert_eq!(assistant, expected);
                assert_eq!(
                    displayed.as_ref().unwrap().views[&1].fields["messages"]
                        .items
                        .iter()
                        .map(|item| &**item)
                        .collect::<Vec<_>>(),
                    view["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    displayed.as_ref().unwrap()["threads"][0]["historyRetry"]["fullRestarts"]
                        .as_u64()
                        .unwrap_or(0),
                    0
                );
                break;
            }
        }
        if disconnect && live_advanced && page_requests >= 3 && !reconnected {
            let cache = client.history_cache.take().unwrap();
            let token = client.token.clone();
            drop(client);
            client = Client::unsubscribed(&profile.discovery(), token);
            client.snapshot = cache.state.clone();
            client.seq = cache.seq.unwrap();
            client.epoch = cache.epoch.clone();
            let hello = client.rpc(
                "hello",
                json!({"protocolMajor":1,"protocolMinor":7,"projectionSchema":1}),
                json!("reconnect-hello"),
            );
            let mut cache = cache;
            cache.reconnect(&hello["result"]).unwrap();
            let cursor = json!({"lastSeq":cache.seq,"epoch":cache.epoch});
            client.history_cache = Some(cache);
            client.rpc("workspace.subscribe", cursor, json!("reconnect-subscribe"));
            reconnected = true;
            continue;
        }
        let cache = client.history_cache.as_mut().unwrap();
        if let Some(request) = cache.next_history() {
            assert!(
                cache.next_history().is_none(),
                "only one history request may be in flight"
            );
            page_requests += 1;
            let response = client.rpc(
                request["method"].as_str().unwrap(),
                request["params"].clone(),
                json!("history"),
            );
            assert!(
                response["error"].is_null(),
                "streaming walk must not restart: {response}"
            );
            full_walks += usize::from(response["result"]["kind"] == "snapshot");
            replays += usize::from(response["result"]["kind"] == "synchronized");
            assert!(
                response["result"]["data"]
                    .as_str()
                    .is_none_or(|text| text.len() <= 65536)
            );
            // Match the audit's page-request delay, not a delay on reading live events.
            std::thread::sleep(Duration::from_millis(120));
        } else if cache.assembled(1).is_some() && !cache.complete(1) {
            cache.request_earlier(1, None);
        } else {
            client.read();
        }
    }
    assert_eq!(full_walks, 1);
    assert_eq!(replays, usize::from(disconnect));
    assert_eq!(
        requests_at_completion,
        Some(page_requests),
        "streaming must not trigger history downloads after loading"
    );
    assert_eq!(reconnected, disconnect);
    // Compare all transcript/activity fields against a full walk of the final revision.
    let final_history = read_full_history(&mut client, 65536);
    assert_eq!(
        client
            .history_cache
            .as_ref()
            .unwrap()
            .assembled(1)
            .unwrap()
            .to_string()
            .as_bytes(),
        final_history.to_string().as_bytes()
    );
    profile.stop();
}

fn read_full_history(client: &mut Client, budget: usize) -> Value {
    let mut history_cursor = Value::Null;
    let mut rows: Vec<history::Row> = Vec::new();
    let mut fields;
    loop {
        let mut text = String::new();
        let mut fragment = Value::Null;
        loop {
            let reply = client.rpc(
                "thread.history",
                json!({"threadId":1,"historyCursor":history_cursor,
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
fn history_over_two_mib_and_blobs_stream_in_bounded_pages() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":7,"projectionSchema":1}),
        json!("paging-hello"),
    );
    client.rpc("workspace.subscribe", json!({}), json!("paging-subscribe"));
    let mut expected = Vec::new();
    for index in 0..3 {
        let text = format!("{} {index}", "x".repeat(800_000));
        let receipt = client.command(
            "thread.send",
            json!({"threadId":1,"text":text,"attachments":[],"mode":"prompt"}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(receipt["result"]["status"], "accepted");
        client.until_sequence(receipt["result"]["seq"].as_u64().unwrap(), |state| {
            state["threads"][0]["status"] == "Completed"
        });
        expected.push(text);
    }
    let history = read_full_history(&mut client, 262144);
    assert!(history.to_string().len() > 2 * 1024 * 1024);
    for text in expected {
        assert_eq!(
            history["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|message| message["text"] == text)
                .count(),
            1
        );
    }
    assert!(
        client.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let bytes = vec![42u8; 600_000];
    fs::create_dir_all(profile.root.join("Shina/attachments")).unwrap();
    fs::write(profile.root.join("Shina/attachments/fixture.png"), &bytes).unwrap();
    let port = profile.discovery()["port"].as_u64().unwrap() as u16;
    let mut assembled = Vec::new();
    while assembled.len() < bytes.len() {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream,"GET /v1/blobs/fixture.png?offset={}&limit=65536 HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",assembled.len(),client.token).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        assert!(response.starts_with(b"HTTP/1.1 200"));
        assert!(response.len() - end <= 65536);
        assembled.extend_from_slice(&response[end..]);
    }
    assert_eq!(assembled, bytes);
    profile.stop();
}

#[test]
fn slow_subscribers_do_not_block_the_runtime_or_other_clients() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let _slow = profile.client(uuid::Uuid::new_v4());
    let mut fast = profile.client(uuid::Uuid::new_v4());
    let started = Instant::now();
    for index in 0..30 {
        let receipt = fast.command(
            "thread.rename",
            json!({"threadId":1,"title":format!("{} {index}","x".repeat(128_000))}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(receipt["result"]["status"], "accepted");
    }
    assert!(started.elapsed() < Duration::from_secs(20));
    let receipt = fast.command(
        "thread.rename",
        json!({"threadId":1,"title":"Still responsive"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(receipt["result"]["status"], "accepted");
    profile.stop();
}

#[test]
fn process_crashes_at_dispatch_boundaries_never_repeat_the_prompt() {
    for phase in ["pending", "dispatching", "sent"] {
        let mut profile = Profile::new();
        let child = profile.launch_with_fault(&["serve"], Some(phase));
        let device = uuid::Uuid::new_v4();
        let mut client = profile.client(device);
        let id = uuid::Uuid::new_v4();
        let args =
            json!({"threadId":1,"text":"crash-boundary-fixture","attachments":[],"mode":"prompt"});
        client.socket.send(Message::Text(json!({
            "jsonrpc":"2.0","id":id,"method":"thread.send","params":{"commandId":id,"args":args}
        }).to_string().into())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while profile.children[child].try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "did not reach {phase} boundary");
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(
            profile.children[child].try_wait().unwrap().unwrap().code(),
            Some(86)
        );
        drop(client);
        profile.launch(&["serve"]);
        let mut resumed = profile.client(device);
        let receipt = resumed.command("thread.send", args, id);
        assert_ne!(receipt["result"]["status"], "rejected");
        assert!(
            !resumed.snapshot["threads"][0]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["text"] == "Fixture complete")
        );
        let database = rusqlite::Connection::open(profile.root.join("Shina/state.sqlite")).unwrap();
        let interrupted: i64 = database
            .query_row(
                "SELECT count(*) FROM effect_outbox WHERE kind='harness' AND status='interrupted'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(interrupted > 0);
        let prompts = fs::read_to_string(profile.root.join("Shina/chats/1/fixture-prompts.jsonl"))
            .unwrap_or_default();
        // A lost response after the prompt reached the harness is retried by
        // receipt replay, so exactly one prompt exists. Earlier boundaries
        // never sent one.
        assert_eq!(
            prompts.lines().count(),
            usize::from(phase == "sent"),
            "{phase}: {prompts}"
        );
        profile.stop();
    }
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
fn linked_machine_thread_pill_sends_and_agent_reads_remote_thread_via_read_thread() {
    let mut machine_a = Profile::new();
    let mut machine_b = Profile::new();
    machine_a.launch(&["serve"]);
    machine_b.launch(&["serve"]);
    let a_discovery = machine_a.discovery();
    let b_discovery = machine_b.discovery();
    let mut a_client = machine_a.client(uuid::Uuid::new_v4());
    let mut b_client = machine_b.client(uuid::Uuid::new_v4());

    // Create thread 7 on machine B with a distinct transcript fact.
    for id in 2..=7 {
        let created = b_client.command(
            "thread.create",
            json!({"occupiedDrafts":[1,2,3,4,5,6,7]}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(created["result"]["result"]["threadId"], id, "{created}");
    }
    let sent_b = b_client.command(
        "thread.send",
        json!({"threadId":7,"text":"SECRET_FROM_MACHINE_B","attachments":[],"mode":"prompt"}),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(sent_b["result"]["status"], "accepted", "{sent_b}");
    b_client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == 7 && thread["status"] == "Completed")
    });

    // Link machine B to machine A.
    let code = machine_b.cli(&["pair", "--peer"]);
    assert!(code.status.success());
    let output = String::from_utf8(code.stdout).unwrap();
    let code = output
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Pairing code: ")
        .unwrap();
    let host_b = format!("127.0.0.1:{}", b_discovery["port"]);
    let linked = machine_a.cli(&["peer", "link", &host_b, code]);
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    a_client.rpc("workspace.subscribe", json!({}), json!("refresh-peers"));
    a_client.until(|state| {
        state["peers"]
            .as_array()
            .is_some_and(|peers| !peers.is_empty())
    });

    // Attach thread 7 from machine B in a composer targeting machine A and send.
    let b_server_id = b_discovery["serverId"].as_str().unwrap();
    let send_a = a_client.command(
        "thread.send",
        json!({
            "threadId": 1,
            "text": "Please inspect [Remote seven](t3-context://v1/thread/thread_7)",
            "attachments": [],
            "context": {
                "version": 1,
                "records": [{
                    "version": 1,
                    "contextId": "thread_7",
                    "kind": "thread",
                    "label": "Remote seven",
                    "environmentId": b_server_id,
                    "threadId": "7",
                    "title": "Remote seven"
                }]
            },
            "mode": "prompt"
        }),
        uuid::Uuid::new_v4(),
    );
    assert_eq!(send_a["result"]["status"], "accepted", "{send_a}");
    a_client.until(|state| {
        state["threads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|thread| thread["id"] == 1 && thread["status"] == "Completed")
    });

    // Verify the prompt delivered to the agent on A contains the read_thread reference with
    // machine B's server id and thread 7, and that calling read_thread with those exact
    // arguments through A's MCP reads B's thread.
    let a_thread_dir = machine_a.root.join("Shina/chats/1");
    let prompts_raw = fs::read_to_string(a_thread_dir.join("fixture-prompts.jsonl")).unwrap();
    let prompt_text: String = serde_json::from_str(prompts_raw.lines().next().unwrap()).unwrap();
    assert!(
        prompt_text.contains("[Thread: Remote seven; ref=thread_7]")
            && prompt_text.contains("<t3_context version=\"1\">")
            && prompt_text.contains(b_server_id),
        "{prompt_text}"
    );
    let (_, after_call) = prompt_text
        .split_once("read_thread(")
        .expect("read_thread call in prompt");
    let (args_json, _) = after_call.split_once(')').expect("closing paren");
    let read_args: Value = serde_json::from_str(args_json).unwrap();
    assert_eq!(read_args["machine"], b_server_id);
    assert_eq!(read_args["threadId"], 7);

    let read_result = read_fixture_thread(&a_thread_dir, read_args);
    assert_eq!(read_result["thread"]["threadId"], 7);
    assert_eq!(read_result["thread"]["machine"], b_server_id);
    assert!(
        read_result["items"][0]["text"]
            .as_str()
            .unwrap()
            .contains("SECRET_FROM_MACHINE_B"),
        "{read_result}"
    );
    let _ = a_discovery;

    drop(a_client);
    drop(b_client);
    machine_a.stop();
    machine_b.stop();
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
fn completed_turn_defers_500_work_records_until_the_fold_loads_them() {
    let mut template = Profile::new();
    template.launch(&["serve"]);
    let base = template.client(uuid::Uuid::new_v4()).snapshot.clone();
    template.stop();
    let mut profile = Profile::new();
    let mut index = base["index"].clone();
    index["threads"][0]["session"] = Value::Null;
    let mut view = base["threads"][0].clone();
    view["status"] = json!("Completed");
    view["messages"] = json!([
        {"id":"u","role":"You","text":"Do the work","images":[]},
        {"id":"a","role":"Pi","text":"Finished","images":[]}
    ]);
    view["work"] = json!([{"start":1,"stopped":false,"elapsed":10,"outcome":"Completed"}]);
    view["tools"] = json!(
        (0..500)
            .map(|i| json!({
                "id":format!("work-{i}"),"name":"read","kind":"read","detail":"file.rs",
                "done":true,"failed":false,"output":"x".repeat(2048),"images":[],
                "structured":null,"parent_call":null,"exit_code":0,"subagents":[],
                "anchor":{"message":1,"offset":0},"block":0,"order":i+1
            }))
            .collect::<Vec<_>>()
    );
    index["threadViews"] = json!([view]);
    fs::write(profile.root.join("Shina/index.json"), index.to_string()).unwrap();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("hello-work"),
    );
    assert!(
        hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == "deferred-work"),
        "{hello}"
    );
    let mut cache = client_history::ProjectionCache::default();
    cache.reconnect(&hello["result"]).unwrap();
    cache
        .receive(&protocol::notification(
            "subscription.snapshot",
            json!({
                "state":protocol::compact(&client.snapshot),"seq":client.seq,"epoch":client.epoch
            }),
        ))
        .unwrap();
    client.history_cache = Some(cache);
    let mut initial_page_bytes = 0;
    while let Some(request) = client.history_cache.as_mut().unwrap().next_history() {
        let reply = client.rpc(
            request["method"].as_str().unwrap(),
            request["params"].clone(),
            json!("history"),
        );
        initial_page_bytes = reply["result"]["totalBytes"].as_u64().unwrap_or(0) as usize;
    }
    let cache = client.history_cache.as_mut().unwrap();
    let history = cache.assembled(1).unwrap();
    let bytes = history.to_string().len();
    assert_eq!(history["tools"].as_array().unwrap().len(), 1, "{history}");
    assert_eq!(history["deferredWork"][0]["itemCount"], 500);
    assert!(
        bytes < 16 * 1024,
        "initial scoped history encoded {bytes} bytes"
    );
    assert!(initial_page_bytes > 0 && initial_page_bytes < 16 * 1024);
    eprintln!(
        "500 folded work records: initial page {initial_page_bytes} bytes, cached history {bytes} bytes, 1 work record"
    );
    let anchor = history["deferredWork"][0]["sourceItemId"]
        .as_str()
        .unwrap()
        .to_owned();
    cache.request_work(1, anchor.clone());
    let _failed = cache.next_history().unwrap();
    cache
        .receive(&json!({"id":"history","error":{"code":-32603,"message":"Fixture work failure"}}))
        .unwrap();
    assert_eq!(
        cache.assembled(1).unwrap()["deferredWork"][0]["loading"],
        false
    );
    assert_eq!(
        cache.assembled(1).unwrap()["deferredWork"][0]["error"],
        "Fixture work failure"
    );
    cache.request_work(1, anchor);
    let first = cache.next_history().unwrap();
    assert!(first["params"]["workAnchorId"].is_string());
    client.rpc(
        first["method"].as_str().unwrap(),
        first["params"].clone(),
        json!("history"),
    );
    // Reconnect in the middle of the work cursor walk, retaining already merged records.
    let mut saved = client.history_cache.take().unwrap();
    let token = client.token.clone();
    drop(client);
    client = Client::unsubscribed(&profile.discovery(), token);
    client.snapshot = saved.state.clone();
    client.seq = saved.seq.unwrap();
    client.epoch = saved.epoch.clone();
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}),
        json!("work-reconnect"),
    );
    saved.reconnect(&hello["result"]).unwrap();
    client.history_cache = Some(saved);
    let mut requests = 0;
    while let Some(request) = client.history_cache.as_mut().unwrap().next_history() {
        client.rpc(
            request["method"].as_str().unwrap(),
            request["params"].clone(),
            json!("history"),
        );
        requests += 1;
        assert!(requests < 100, "work history did not complete");
    }
    let cache = client.history_cache.as_ref().unwrap();
    assert_eq!(
        cache.assembled(1).unwrap()["tools"]
            .as_array()
            .unwrap()
            .len(),
        500
    );
    assert!(
        cache.assembled(1).unwrap()["deferredWork"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // A peer without the capability sends ordinary inline pages and is never asked for work.
    let hello = client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":10,"projectionSchema":1}),
        json!("old-work-peer"),
    );
    assert!(
        !hello["result"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|cap| cap == "deferred-work")
    );
    client
        .history_cache
        .as_mut()
        .unwrap()
        .reconnect(&hello["result"])
        .unwrap();
    while let Some(request) = client.history_cache.as_mut().unwrap().next_history() {
        assert!(request["params"]["workAnchorId"].is_null());
        client.rpc(
            request["method"].as_str().unwrap(),
            request["params"].clone(),
            json!("history"),
        );
    }
    let cache = client.history_cache.as_mut().unwrap();
    let projected = cache.projected().unwrap();
    assert_eq!(
        projected["threads"][0]["historyMeta"]["workLoading"],
        "inline-legacy"
    );
    assert!(cache.assembled(1).unwrap()["deferredWork"].is_null());
    profile.stop();
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

#[path = "support/admission_contract.rs"]
mod admission_contract;

#[path = "support/update_recovery.rs"]
mod update_recovery;

#[path = "support/capability_parity.rs"]
mod capability_parity;
