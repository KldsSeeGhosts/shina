//! Real serve processes, real WebSockets and fake harness subprocesses. No provider calls.
#[allow(dead_code)]
#[path = "../src/client_history.rs"]
mod client_history;
#[allow(dead_code)]
#[path = "../src/history.rs"]
mod history;
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
    history_cache: Option<client_history::ProjectionCache>,
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
        let (mut socket, _) = tungstenite::connect(request).unwrap();
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
    let delegated = client.command(
        "task.delegate",
        json!({"threadId":parent,"args":args}),
        uuid::Uuid::new_v4(),
    );
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
        json!({"threadId":parent,"args":{"thread_id":child,"message":"remote-long-fixture-again"}}),
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
        json!({"threadId":parent,"args":{"thread_id":child,"message":"remote-confirm-fixture"}}),
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
            json!({"threadId":parent,"args":{"thread_id":child,"message":"A second remote turn"}}),
            uuid::Uuid::new_v4(),
        );
        assert_eq!(followup["result"]["status"], "accepted", "{followup}");
        let pause = PausedProcess::new(target.children.last().unwrap());
        let cancelled = client.command(
            "task.cancel",
            json!({"threadId":parent,"args":{"thread_id":child}}),
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
        std::thread::sleep(Duration::from_secs(3));
        let before_idle = fs::metadata(profile.root.join("Shina/state.sqlite-wal"))
            .unwrap()
            .len();
        std::thread::sleep(Duration::from_secs(3));
        let after_idle = fs::metadata(profile.root.join("Shina/state.sqlite-wal"))
            .unwrap()
            .len();
        assert_eq!(before_idle, after_idle, "idle workspace wrote to WAL");
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
            after_idle - before_idle
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
    let mut displayed = Value::Null;
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
            displayed = view;
        }
        let installed = cache.histories.get(&1);
        if let Some((revision, view, size)) = installed {
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
                .unwrap();
            if cache.state["threads"][0]["status"] == "Running" {
                assert_eq!(
                    displayed["threads"][0]["messages"]
                        .as_array()
                        .unwrap()
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
                assert_eq!(displayed["threads"][0]["messages"], view["messages"]);
                assert_eq!(
                    displayed["threads"][0]["historyRetry"]["fullRestarts"]
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
        } else if cache.histories.contains_key(&1) && !cache.complete(1) {
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
        client.history_cache.as_ref().unwrap().histories[&1]
            .1
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
        } else if cache.histories.contains_key(&1) {
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
        let loaded_msgs = view["threads"][0]["messages"].as_array().unwrap();
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
        cache.histories[&1].1["messages"].as_array().unwrap().len(),
        1800
    );
    assert_eq!(
        cache.histories[&1].1["tools"].as_array().unwrap().len(),
        900
    );
    assert_eq!(
        cache.histories[&1].1["messages"][0]["text"],
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
        } else if cache.histories.contains_key(&1) {
            break;
        }
    }
    // u-3 is NOT in initial window; requesting earlier throughEntryId="u-3" loads it!
    {
        let cache = linked_client.history_cache.as_mut().unwrap();
        assert!(
            !cache.histories[&1].1["messages"]
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
        linked_client.history_cache.as_ref().unwrap().histories[&1].1["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "u-3")
    );

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
