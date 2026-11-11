//! KSG DesktopBackendManager lifecycle: attach first, supervise only our own child.
use crate::{
    protocol::{self, Command},
    server_auth::Discovery,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

type Worker = Mutex<Option<std::thread::JoinHandle<()>>>;
type Registration = (std::sync::Weak<AtomicBool>, std::sync::Weak<Worker>);
static SUPERVISORS: Mutex<Vec<Registration>> = Mutex::new(Vec::new());

/// Actual host exit, not a cancellable Windows session-end callback.
pub fn shutdown_all() {
    let supervisors = std::mem::take(&mut *SUPERVISORS.lock().expect("supervisors"));
    for (stop, _) in &supervisors {
        if let Some(stop) = stop.upgrade() {
            stop.store(true, Ordering::Release);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(7);
    for (_, worker) in supervisors {
        if let Some(worker) = worker.upgrade()
            && let Some(worker) = worker.lock().expect("client worker").take()
        {
            let _ = crate::process::join_for(
                worker,
                deadline.saturating_duration_since(Instant::now()),
            );
        }
    }
}

#[derive(Clone)]
pub struct Commands {
    sender: tokio::sync::mpsc::Sender<Command>,
    ready: Arc<AtomicBool>,
}
impl Commands {
    pub fn send(&self, method: &str, params: Value) -> Result<uuid::Uuid, String> {
        if !self.ready.load(Ordering::Acquire) {
            return Err("The local server is disconnected. Reconnect before sending commands. Your draft was kept.".into());
        }
        let command = Command::new(method, params);
        let id = command.command_id;
        self.sender.try_send(command).map_err(|_| {
            "Server command queue is full or disconnected. Your draft was kept.".to_owned()
        })?;
        Ok(id)
    }
}

pub enum Event {
    Receipt(Command, Value),
    Error(String),
}

pub struct Client {
    directory: PathBuf,
    device_directory: PathBuf,
    pub commands: Commands,
    input: Option<tokio::sync::mpsc::Receiver<Command>>,
    events: mpsc::Receiver<Event>,
    output: mpsc::SyncSender<Event>,
    snapshot: Arc<Mutex<Option<Value>>>,
    stop: Arc<AtomicBool>,
    worker: Arc<Worker>,
    history_thread: Arc<AtomicUsize>,
}

impl Client {
    pub fn new(directory: PathBuf) -> Self {
        let (sender, input) = tokio::sync::mpsc::channel(32);
        let (output, events) = mpsc::sync_channel(64);
        Self {
            device_directory: directory.clone(),
            directory,
            commands: Commands {
                sender,
                ready: Arc::default(),
            },
            input: Some(input),
            events,
            output,
            snapshot: Arc::default(),
            stop: Arc::default(),
            worker: Arc::default(),
            history_thread: Arc::default(),
        }
    }
    pub fn set_device_directory(&mut self, directory: PathBuf) {
        self.device_directory = directory;
    }
    pub fn start(&mut self, wake: rune::Waker) {
        let Some(input) = self.input.take() else {
            return;
        };
        let directory = self.directory.clone();
        let device_directory = self.device_directory.clone();
        let output = self.output.clone();
        let snapshot = self.snapshot.clone();
        let stop = self.stop.clone();
        let ready = self.commands.ready.clone();
        let history_thread = self.history_thread.clone();
        *self.worker.lock().expect("client worker") = Some(std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("client runtime");
            runtime.block_on(supervise(
                (directory, device_directory, history_thread),
                input,
                output,
                snapshot,
                stop,
                ready,
                wake,
            ));
        }));
        SUPERVISORS
            .lock()
            .expect("supervisors")
            .push((Arc::downgrade(&self.stop), Arc::downgrade(&self.worker)));
    }
    pub fn snapshot(&self) -> Option<Value> {
        self.snapshot.lock().expect("client snapshot").take()
    }
    pub fn show_history(&self, thread: usize) {
        self.history_thread.store(thread, Ordering::Release);
    }
    pub fn event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.lock().expect("client worker").take() {
            let _ = crate::process::join_for(worker, Duration::from_secs(7));
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn device_id(directory: &std::path::Path) -> Result<String, String> {
    // Each reserved draft slot has a stable device and its own receipt namespace.
    let path = directory.join("client-device");
    match std::fs::read_to_string(&path) {
        Ok(id) => Ok(uuid::Uuid::parse_str(id.trim())
            .map_err(|e| e.to_string())?
            .to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let id = uuid::Uuid::new_v4().to_string();
            crate::store::write_atomic(&path, id.as_bytes()).map_err(|e| e.to_string())?;
            Ok(id)
        }
        Err(e) => Err(e.to_string()),
    }
}

async fn authenticate(discovery: &Discovery, device: &str) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", discovery.port))
        .await
        .map_err(|e| e.to_string())?;
    let body = json!({"deviceId":device}).to_string();
    stream.write_all(format!("POST /v1/auth HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}", discovery.bootstrap_token,body.len(),body).as_bytes()).await.map_err(|e| e.to_string())?;
    let mut reply = String::new();
    stream
        .take(16384)
        .read_to_string(&mut reply)
        .await
        .map_err(|e| e.to_string())?;
    if reply.starts_with("HTTP/1.1 401") || reply.starts_with("HTTP/1.1 403") {
        return Err("Authentication denied. Restart the client after checking this profile's server.json permissions.".into());
    }
    if !reply.starts_with("HTTP/1.1 200") {
        return Err("Server authentication endpoint is unavailable.".into());
    }
    let value: Value = serde_json::from_str(
        reply
            .split_once("\r\n\r\n")
            .ok_or("Invalid authentication response.")?
            .1,
    )
    .map_err(|e| e.to_string())?;
    value["token"]
        .as_str()
        .map(str::to_owned)
        .ok_or("Missing device credential.".into())
}

pub fn restart_delay(attempt: u32) -> Duration {
    Duration::from_millis(500u64.saturating_mul(1u64 << attempt.min(5)).min(10_000))
}

fn spawn() -> Result<Child, String> {
    let mut command =
        std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Deliberately not Process::spawn: the UI's kill-on-close job must not own harnesses.
        command.creation_flags(0x08000000);
    }
    command.spawn().map_err(|e| e.to_string())
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Default)]
struct ProjectionCache {
    state: Value,
    seq: Option<u64>,
    epoch: Value,
    snapshot_pages: String,
    histories: std::collections::HashMap<u64, (String, Value, usize)>,
    history_order: VecDeque<u64>,
    desired_thread: Option<u64>,
    inflight: Option<History>,
    dirty: bool,
}

struct History {
    thread: u64,
    revision: String,
    cursor: Value,
    text: String,
    waiting: bool,
}

impl ProjectionCache {
    fn receive(&mut self, value: &Value) -> Result<(), String> {
        let params = &value["params"];
        match value["method"].as_str() {
            Some("subscription.snapshotPage") => {
                self.snapshot_pages
                    .push_str(params["data"].as_str().ok_or("Invalid snapshot page.")?);
                if self.snapshot_pages.len() > protocol::MAX_SNAPSHOT_BYTES {
                    return Err("Workspace metadata exceeded the client memory budget.".into());
                }
                if params["nextCursor"].is_null() {
                    let snapshot: Value =
                        serde_json::from_str(&std::mem::take(&mut self.snapshot_pages))
                            .map_err(|e| e.to_string())?;
                    self.receive(&protocol::notification("subscription.snapshot", snapshot))?;
                }
            }
            Some("subscription.snapshot") => {
                self.state = params["state"].clone();
                self.seq = params["seq"].as_u64();
                self.epoch = params["epoch"].clone();
                self.dirty = true;
            }
            Some("subscription.patch") => {
                let seq = params["seq"].as_u64().ok_or("Invalid patch sequence.")?;
                if self.seq.is_some_and(|previous| seq <= previous) {
                    return Ok(());
                }
                if self.seq.and_then(|seq| seq.checked_add(1)) != Some(seq)
                    || self.epoch != params["epoch"]
                {
                    return Err("Projection replay gap. Reconnecting for a snapshot.".into());
                }
                protocol::apply(&mut self.state, params["patch"].clone())?;
                self.seq = Some(seq);
                self.dirty = true;
            }
            _ if value["id"] == "history" => {
                let Some(mut history) = self.inflight.take() else {
                    return Ok(());
                };
                if self.desired_thread.is_some_and(|id| id != history.thread) {
                    return Ok(());
                }
                if !value["error"].is_null() {
                    return Ok(());
                }
                let page = &value["result"];
                if history.text.is_empty()
                    && let Some(revision) = page["historyRevision"].as_str()
                {
                    history.revision = revision.to_owned();
                }
                history
                    .text
                    .push_str(page["data"].as_str().ok_or("Invalid transcript page.")?);
                if history.text.len() > protocol::MAX_SNAPSHOT_BYTES {
                    return Err("Thread history exceeded the client memory budget.".into());
                }
                history.cursor = page["nextCursor"].clone();
                if history.cursor.is_null() {
                    let view = serde_json::from_str(&history.text).map_err(|e| e.to_string())?;
                    self.histories
                        .insert(history.thread, (history.revision, view, history.text.len()));
                    self.history_order.retain(|id| *id != history.thread);
                    self.history_order.push_back(history.thread);
                    while self
                        .histories
                        .values()
                        .map(|(_, _, bytes)| *bytes)
                        .sum::<usize>()
                        > protocol::MAX_SNAPSHOT_BYTES
                    {
                        let Some(oldest) = self.history_order.pop_front() else {
                            break;
                        };
                        self.histories.remove(&oldest);
                    }
                    self.dirty = true;
                } else {
                    history.waiting = false;
                    self.inflight = Some(history);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn next_history(&mut self) -> Option<Value> {
        if self.inflight.is_none() {
            for thread in self.state["threads"].as_array()? {
                let id = thread["id"].as_u64()?;
                let desired = self
                    .desired_thread
                    .or_else(|| self.state["index"]["active"].as_u64());
                if desired.is_some_and(|desired| desired != id) {
                    continue;
                }
                let revision = thread["historyRevision"].as_str()?;
                if self
                    .histories
                    .get(&id)
                    .is_none_or(|(old, _, _)| old != revision)
                {
                    self.inflight = Some(History {
                        thread: id,
                        revision: revision.into(),
                        cursor: Value::Null,
                        text: String::new(),
                        waiting: false,
                    });
                    break;
                }
            }
        }
        let history = self.inflight.as_mut()?;
        if history.waiting {
            return None;
        }
        history.waiting = true;
        Some(
            json!({"jsonrpc":"2.0","id":"history","method":"thread.history",
            "params":{"threadId":history.thread,"cursor":history.cursor,"byteBudget":protocol::PAGE_BYTES}}),
        )
    }

    fn projected(&mut self) -> Option<Value> {
        if !self.dirty || self.seq.is_none() {
            return None;
        }
        self.dirty = false;
        if let Some(threads) = self.state["threads"].as_array() {
            self.histories
                .retain(|id, _| threads.iter().any(|thread| thread["id"] == *id));
            self.history_order
                .retain(|id| self.histories.contains_key(id));
        }
        let mut state = self.state.clone();
        if let Some(threads) = state["threads"].as_array_mut() {
            for thread in threads {
                if let Some((_, history, _)) =
                    thread["id"].as_u64().and_then(|id| self.histories.get(&id))
                {
                    for key in protocol::HISTORY_FIELDS {
                        thread[key] = history[key].clone();
                    }
                }
            }
        }
        Some(state)
    }
}

async fn connect(
    discovery: &Discovery,
    device: &str,
    device_directory: &std::path::Path,
    cache: &ProjectionCache,
) -> Result<Socket, String> {
    if discovery.protocol_major != crate::protocol::MAJOR {
        return Err("Protocol major mismatch. Update the server and client together.".into());
    }
    let credential_path = device_directory.join("client-credential.json");
    let saved = crate::store::load(&credential_path)?;
    let token = if let Some(saved) = saved {
        if saved["serverId"] != discovery.server_id || saved["deviceId"] != device {
            return Err(
                "Authentication denied. The saved credential belongs to another server or device."
                    .into(),
            );
        }
        saved["token"]
            .as_str()
            .ok_or("Invalid saved device credential.")?
            .to_owned()
    } else {
        let token = tokio::time::timeout(Duration::from_secs(2), authenticate(discovery, device))
            .await
            .map_err(|_| "Server authentication timed out.")??;
        crate::server_auth::protect(device_directory)?;
        crate::store::write_atomic(
            &credential_path,
            json!({"serverId":discovery.server_id,"deviceId":device,"token":token})
                .to_string()
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        token
    };
    let mut request = format!("ws://127.0.0.1:{}/v1/ws", discovery.port)
        .into_client_request()
        .map_err(|e| e.to_string())?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| "Invalid local credential.")?,
    );
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(protocol::MAX_SNAPSHOT_BYTES))
        .max_frame_size(Some(protocol::MAX_SNAPSHOT_BYTES));
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async_with_config(request, Some(config), false),
    )
    .await
    .map_err(|_| "Server connection timed out.")?
    .map_err(|error| match error {
        tokio_tungstenite::tungstenite::Error::Http(response) if matches!(response.status().as_u16(),401|403) =>
            "Authentication denied. This device may have been revoked. Re-enrollment requires user action.".to_owned(),
        _ => "Server listener is unavailable.".to_owned(),
    })?;
    socket.send(Message::Text(json!({"jsonrpc":"2.0","id":"hello","method":"hello","params":{"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}}).to_string().into())).await.map_err(|e| e.to_string())?;
    let reply: Value = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Ping(payload))) => socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|e| e.to_string())?,
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Text(text))) => {
                    let reply: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                    if reply["id"] == "hello" {
                        break Ok::<_, String>(reply);
                    }
                }
                _ => break Err("Server closed before hello.".into()),
            }
        }
    })
    .await
    .map_err(|_| "Server hello timed out.")??;
    if !reply["error"].is_null()
        || reply["result"]["instanceId"] != discovery.instance_id
        || reply["result"]["serverId"] != discovery.server_id
    {
        return Err(
            "Server handshake refused or instance changed. Check client/server versions.".into(),
        );
    }
    socket
        .send(Message::Text(
            json!({"jsonrpc":"2.0","id":"subscribe","method":"workspace.subscribe","params":{"lastSeq":cache.seq,"epoch":cache.epoch}})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(socket)
}

async fn supervise(
    profile: (PathBuf, PathBuf, Arc<AtomicUsize>),
    mut input: tokio::sync::mpsc::Receiver<Command>,
    output: mpsc::SyncSender<Event>,
    snapshot: Arc<Mutex<Option<Value>>>,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    wake: rune::Waker,
) {
    let (directory, device_directory, history_thread) = profile;
    let report = |error: String| {
        let _ = output.try_send(Event::Error(error));
        wake();
    };
    if let Err(error) = std::fs::create_dir_all(&directory) {
        report(error.to_string());
        return;
    }
    let device = match device_id(&device_directory) {
        Ok(id) => id,
        Err(error) => {
            report(error);
            return;
        }
    };
    let mut child: Option<Child> = None;
    let mut pending = VecDeque::<Command>::new();
    let mut attempt = 0;
    let mut next_spawn = Instant::now();
    let mut owned_instance = None;
    let mut socket: Option<Socket> = None;
    let mut cache = ProjectionCache::default();
    while !stop.load(Ordering::Acquire) {
        cache.desired_thread = match history_thread.load(Ordering::Acquire) {
            0 => None,
            id => Some(id as u64),
        };
        if socket.is_none() {
            ready.store(false, Ordering::Release);
            if let Some(process) = &mut child
                && process.try_wait().ok().flatten().is_some()
            {
                child = None;
                owned_instance = None;
                next_spawn = Instant::now() + restart_delay(attempt);
                attempt += 1;
                report("The local server exited. Retrying with backoff. If it stays unavailable, close this client and run shina serve to inspect startup errors.".into());
            }
            if let Ok(discovery) = Discovery::read(&directory) {
                match tokio::select! {
                    result = connect(&discovery, &device, &device_directory, &cache) => result,
                    _ = stopped(&stop) => break,
                } {
                    Ok(mut connected) => {
                        cache.inflight = None;
                        cache.snapshot_pages.clear();
                        if child
                            .as_ref()
                            .is_some_and(|child| child.id() == discovery.pid)
                        {
                            owned_instance = Some(discovery.instance_id);
                        }
                        let mut sent = true;
                        for command in &pending {
                            sent &= send(&mut connected, wire_command(command)).await;
                        }
                        if sent {
                            socket = Some(connected);
                            attempt = 0;
                            continue;
                        }
                    }
                    Err(error)
                        if error.contains("Authentication denied")
                            || error.contains("mismatch")
                            || error.contains("handshake refused") =>
                    {
                        report(error);
                        break;
                    }
                    Err(_) => {}
                }
            }
            if child.is_none() && Instant::now() >= next_spawn {
                match crate::server_lock::Owner::acquire(&directory) {
                    Ok(probe) => {
                        drop(probe);
                        match spawn() {
                            Ok(process) => child = Some(process),
                            Err(error) => {
                                report(error);
                                next_spawn = Instant::now() + restart_delay(attempt);
                                attempt += 1;
                            }
                        }
                    }
                    Err(_) => report(
                        "The profile is owned but its server is not ready. Waiting for recovery."
                            .into(),
                    ),
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        let connected = socket.as_mut().expect("connected");
        tokio::select! {
            command = input.recv(), if pending.len() < 32 => {
                let Some(command) = command else { break; };
                let message = wire_command(&command);
                pending.push_back(command);
                if !send(connected, message).await { socket = None; }
            }
            message = connected.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            let mut disconnect = false;
                            if let Err(error) = cache.receive(&value) {
                                cache = ProjectionCache::default();
                                socket = None;
                                report(error);
                                continue;
                            }
                            if let Some(projected) = cache.projected() {
                                *snapshot.lock().expect("client snapshot") = Some(projected);
                            }
                            if value["method"] == "subscription.ready" || value["id"] == "subscribe" {
                                ready.store(cache.seq.is_some(),Ordering::Release);
                            }
                            if let Some(id) = value["id"].as_str().and_then(|id| uuid::Uuid::parse_str(id).ok())
                                && let Some(command) = pending.iter().find(|command| command.command_id == id) {
                                    let transient = value["error"]["message"].as_str().is_some_and(|text| text.contains("queue is full") || text.contains("Runtime unavailable"));
                                    if transient { disconnect = true; }
                                    else if output.try_send(Event::Receipt(command.clone(), value)).is_ok() {
                                        if let Some(index) = pending.iter().position(|command| command.command_id == id) { pending.remove(index); }
                                    }
                                    else { disconnect = true; }
                            }
                            if !disconnect && let Some(request) = cache.next_history()
                                && !send(connected, Message::Text(request.to_string().into())).await {
                                    disconnect = true;
                                }
                            if disconnect { socket = None; }
                            wake();
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => { if !send(connected, Message::Pong(payload)).await { socket = None; } }
                    Some(Ok(Message::Pong(_))) => {}
                    _ => { socket = None; report("Server disconnected. Reconnecting with the same command IDs.".into()); }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if let Some(request) = cache.next_history()
                    && !send(connected, Message::Text(request.to_string().into())).await {
                        socket = None;
                    }
            }
        }
    }
    ready.store(false, Ordering::Release);
    if let Some(mut process) = child {
        // Only a process handle we spawned, with an authenticated matching instance, may be stopped.
        if owned_instance.is_some()
            && let Some(connected) = &mut socket
        {
            send(
                connected,
                wire_command(&Command::new("server.stop", json!({}))),
            )
            .await;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && process.try_wait().ok().flatten().is_none() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if process.try_wait().ok().flatten().is_none() {
            let _ = process.kill();
        }
        let _ = process.wait();
    }
}

fn wire_command(command: &Command) -> Message {
    Message::Text(
        json!({"jsonrpc":"2.0","id":command.command_id,"method":command.method,
        "params":{"commandId":command.command_id,"args":command.params}})
        .to_string()
        .into(),
    )
}

async fn send(socket: &mut Socket, message: Message) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(1), socket.send(message)).await,
        Ok(Ok(()))
    )
}

async fn stopped(stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_backoff_is_half_a_second_through_ten_seconds() {
        assert_eq!(
            (0..7)
                .map(|attempt| restart_delay(attempt).as_millis())
                .collect::<Vec<_>>(),
            [500, 1000, 2000, 4000, 8000, 10000, 10000]
        );
    }
    #[test]
    fn offline_commands_including_approvals_are_not_queued() {
        let client = Client::new(crate::store::directory().unwrap());
        assert!(
            client
                .commands
                .send(
                    "request.respond",
                    json!({"threadId":1,"response":{"confirmed":true}})
                )
                .is_err()
        );
        assert!(
            client
                .commands
                .send("thread.send", json!({"text":"kept locally"}))
                .is_err()
        );
    }

    #[test]
    fn reconnect_cache_applies_replay_without_losing_history_and_detects_gaps() {
        let mut cache = ProjectionCache::default();
        cache.receive(&protocol::notification("subscription.snapshot",
            json!({"epoch":"epoch","seq":10,"state":{"threads":[{"id":1,"historyRevision":"a","messages":[]}]}}))).unwrap();
        let request = cache.next_history().unwrap();
        assert_eq!(request["params"]["threadId"], 1);
        cache.receive(&json!({"id":"history","result":{"data":"{\"messages\":[{\"text\":\"retained\"}]}","nextCursor":null}})).unwrap();
        assert_eq!(
            cache.projected().unwrap()["threads"][0]["messages"][0]["text"],
            "retained"
        );
        cache.receive(&protocol::notification("subscription.patch",
            json!({"epoch":"epoch","seq":11,"patch":[{"op":"set","path":["threads","0","status"],"value":"completed"}]}))).unwrap();
        assert_eq!(
            cache.projected().unwrap()["threads"][0]["messages"][0]["text"],
            "retained"
        );
        assert!(
            cache
                .receive(&protocol::notification(
                    "subscription.patch",
                    json!({"epoch":"epoch","seq":13,"patch":[]})
                ))
                .is_err()
        );
    }

    #[test]
    fn native_history_fetch_is_lazy_and_follows_the_local_selection() {
        let mut cache = ProjectionCache {
            desired_thread: Some(2),
            state: json!({"index":{"active":1},"threads":[
                {"id":1,"historyRevision":"a"},{"id":2,"historyRevision":"b"}]}),
            ..ProjectionCache::default()
        };
        assert_eq!(cache.next_history().unwrap()["params"]["threadId"], 2);
        cache.desired_thread = Some(1);
        cache
            .receive(&json!({"id":"history","result":{"data":"{}","nextCursor":null}}))
            .unwrap();
        assert_eq!(cache.next_history().unwrap()["params"]["threadId"], 1);
        assert!(cache.histories.is_empty());
    }
}
