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
        // This path was created under temp_dir with a fresh UUID, never the real profile.
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Client {
    socket: WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>,
    snapshot: Value,
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
                    }
                    return value;
                }
                Message::Ping(payload) => self.socket.send(Message::Pong(payload)).unwrap(),
                _ => {}
            }
        }
    }
    fn until(&mut self, predicate: impl Fn(&Value) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !predicate(&self.snapshot) {
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
        let index: Value =
            serde_json::from_slice(&fs::read(profile.root.join("Shina/index.json")).unwrap())
                .unwrap();
        assert_eq!(
            index["commandReceipts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|receipt| receipt["commandId"] == id.to_string())
                .count(),
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
    assert_eq!(hello["result"]["protocolMinor"], 1);
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
