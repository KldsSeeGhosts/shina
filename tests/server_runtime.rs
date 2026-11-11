//! Real serve processes, real WebSockets and fake harness subprocesses. No provider calls.
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::{self, Message, WebSocket, client::IntoClientRequest};

struct Profile {
    root: PathBuf,
    children: Vec<Child>,
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
                fs::read_to_string(
                    self.root
                        .join(format!("server-{}.log", self.children.len() - 1))
                )
                .unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn client(&self, device: uuid::Uuid) -> Client {
        Client::connect(&self.discovery(), device)
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

struct Client {
    socket: WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    snapshot: Value,
    seq: u64,
    epoch: Value,
    notifications: Vec<Value>,
    token: String,
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
        let mut request = format!("ws://127.0.0.1:{port}/v1/ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", auth["token"].as_str().unwrap())
                .parse()
                .unwrap(),
        );
        let (mut socket, _) = tungstenite::connect(request).unwrap();
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
        }
        let mut client = Self {
            socket,
            snapshot: Value::Null,
            seq: 0,
            epoch: Value::Null,
            notifications: Vec::new(),
            token: auth["token"].as_str().unwrap().to_owned(),
        };
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
            match self.socket.read().unwrap() {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if value["method"] == "subscription.snapshot" {
                        self.snapshot = value["params"]["state"].clone();
                        self.seq = value["params"]["seq"].as_u64().unwrap();
                        self.epoch = value["params"]["epoch"].clone();
                    }
                    if value["method"] == "subscription.patch" {
                        for patch in value["params"]["patch"].as_array().unwrap() {
                            let path = patch["path"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|part| {
                                    part.as_str().unwrap().replace('~', "~0").replace('/', "~1")
                                })
                                .collect::<Vec<_>>();
                            if path.is_empty() {
                                self.snapshot = patch["value"].clone();
                                continue;
                            }
                            let parent = if path.len() == 1 {
                                String::new()
                            } else {
                                format!("/{}", path[..path.len() - 1].join("/"))
                            };
                            let key = patch["path"]
                                .as_array()
                                .unwrap()
                                .last()
                                .unwrap()
                                .as_str()
                                .unwrap();
                            let target = self.snapshot.pointer_mut(&parent).unwrap();
                            if let Some(object) = target.as_object_mut() {
                                if patch["op"] == "remove" {
                                    object.remove(key);
                                } else {
                                    object.insert(key.into(), patch["value"].clone());
                                }
                            } else {
                                target.as_array_mut().unwrap()[key.parse::<usize>().unwrap()] =
                                    patch["value"].clone();
                            }
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
fn history_over_two_mib_and_blobs_stream_in_bounded_pages() {
    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let mut client = profile.client(uuid::Uuid::new_v4());
    client.rpc(
        "hello",
        json!({"protocolMajor":1,"protocolMinor":1,"projectionSchema":1}),
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
    let mut text = String::new();
    let mut cursor = Value::Null;
    loop {
        let response = client.rpc(
            "thread.history",
            json!({"threadId":1,"cursor":cursor,"byteBudget":262144}),
            json!("history-page"),
        );
        assert!(response["error"].is_null(), "{response}");
        let page = &response["result"];
        assert!(page["data"].as_str().unwrap().len() <= 262144);
        text.push_str(page["data"].as_str().unwrap());
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert!(text.len() > 2 * 1024 * 1024);
    let history: Value = serde_json::from_str(&text).unwrap();
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
