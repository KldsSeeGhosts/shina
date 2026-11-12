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
        Self::with_token(discovery, auth["token"].as_str().unwrap().to_owned())
    }
    fn with_token(discovery: &Value, token: String) -> Self {
        let port = discovery["port"].as_u64().unwrap() as u16;
        let mut request = format!("ws://127.0.0.1:{port}/v1/ws")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
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
            token,
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
    assert_eq!(hello["result"]["protocolMinor"], 3);
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
