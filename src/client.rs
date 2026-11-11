//! KSG DesktopBackendManager lifecycle: attach first, supervise only our own child.
use crate::{
    protocol::{Command, MAX_CONTROL},
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
        atomic::{AtomicBool, Ordering},
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
    pub fn connected(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn send(&self, method: &str, params: Value) -> Result<uuid::Uuid, String> {
        if !self.ready.load(Ordering::Acquire)
            && !matches!(method, "server.clientReady" | "server.cancelRestart")
        {
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
    Maintenance(String, Value),
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
        *self.worker.lock().expect("client worker") = Some(std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("client runtime");
            runtime.block_on(supervise(
                (directory, device_directory),
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
    if reply.starts_with("HTTP/1.1 401") {
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
        .env("SHINA_DESKTOP_OWNER_PID", std::process::id().to_string())
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

async fn connect(discovery: &Discovery, device: &str, desktop: bool) -> Result<Socket, String> {
    if discovery.protocol_major != crate::protocol::MAJOR {
        return Err("Protocol major mismatch. Update the server and client together.".into());
    }
    let token = tokio::time::timeout(Duration::from_secs(2), authenticate(discovery, device))
        .await
        .map_err(|_| "Server authentication timed out.")??;
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
        .max_message_size(Some(2 * MAX_CONTROL))
        .max_frame_size(Some(2 * MAX_CONTROL));
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::connect_async_with_config(request, Some(config), false),
    )
    .await
    .map_err(|_| "Server connection timed out.")?
    .map_err(|_| "Server listener is unavailable.")?;
    socket.send(Message::Text(json!({"jsonrpc":"2.0","id":"hello","method":"hello","params":{"protocolMajor":crate::protocol::MAJOR,"protocolMinor":crate::protocol::MINOR,"projectionSchema":1,"clientPid":desktop.then(std::process::id),"maintenanceClient":!desktop}}).to_string().into())).await.map_err(|e| e.to_string())?;
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
            json!({"jsonrpc":"2.0","id":"subscribe","method":"workspace.subscribe","params":{}})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(socket)
}

async fn supervise(
    profile: (PathBuf, PathBuf),
    mut input: tokio::sync::mpsc::Receiver<Command>,
    output: mpsc::SyncSender<Event>,
    snapshot: Arc<Mutex<Option<Value>>>,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    wake: rune::Waker,
) {
    let (directory, device_directory) = profile;
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
    let mut updating = false;
    let mut server_stopped = false;
    while !stop.load(Ordering::Acquire) {
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
                    result = connect(&discovery, &device, true) => result,
                    _ = stopped(&stop) => break,
                } {
                    Ok(mut connected) => {
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
            if !server_stopped && !updating && child.is_none() && Instant::now() >= next_spawn {
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
                            if let Some(method) = value["method"].as_str().filter(|m| matches!(*m, "server.draining" | "server.restarting" | "server.drainCancelled" | "server.stopping")) {
                                let draining = method != "server.drainCancelled";
                                server_stopped |= method == "server.stopping";
                                ready.store(!draining, Ordering::Release);
                                if output.try_send(Event::Maintenance(method.into(), value["params"].clone())).is_err() {
                                    socket = None;
                                }
                                if method == "server.restarting" { updating = true; }
                            } else if value["method"] == "subscription.snapshot" {
                                *snapshot.lock().expect("client snapshot") = Some(value["params"]["state"].clone());
                                let phase = value["params"]["maintenance"]["phase"].as_str().unwrap_or("ready");
                                let draining = phase != "ready";
                                server_stopped |= phase == "stopping";
                                updating |= phase == "restarting";
                                ready.store(!draining,Ordering::Release);
                            } else if let Some(id) = value["id"].as_str().and_then(|id| uuid::Uuid::parse_str(id).ok())
                                && let Some(command) = pending.iter().find(|command| command.command_id == id) {
                                    let transient = value["error"]["message"].as_str().is_some_and(|text| text.contains("queue is full") || text.contains("Runtime unavailable"));
                                    if transient { socket = None; }
                                    else if output.try_send(Event::Receipt(command.clone(), value)).is_ok() {
                                        if let Some(index) = pending.iter().position(|command| command.command_id == id) { pending.remove(index); }
                                    }
                                    else { socket = None; }
                            }
                            wake();
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => { if !send(connected, Message::Pong(payload)).await { socket = None; } }
                    Some(Ok(Message::Pong(_))) => {}
                    _ => { socket = None; report("Server disconnected. Reconnecting with the same command IDs.".into()); }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    ready.store(false, Ordering::Release);
    if !updating && let Some(mut process) = child {
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

/// Used by the service CLI and detached updater. Does not register a desktop PID.
pub async fn local_rpc(
    directory: &std::path::Path,
    method: &str,
    args: Value,
) -> Result<Value, String> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let discovery = Discovery::read(directory)?;
        static DEVICE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let mut socket = connect(
            &discovery,
            DEVICE.get_or_init(|| uuid::Uuid::new_v4().to_string()),
            false,
        )
        .await?;
        let command = Command::new(method, args);
        socket
            .send(wire_command(&command))
            .await
            .map_err(|e| e.to_string())?;
        while let Some(message) = socket.next().await {
            match message.map_err(|e| e.to_string())? {
                Message::Ping(payload) => socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|e| e.to_string())?,
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                    if value["id"] == command.command_id.to_string() {
                        if let Some(error) = value["error"]["message"].as_str() {
                            return Err(error.into());
                        }
                        return Ok(value["result"].clone());
                    }
                }
                _ => {}
            }
        }
        Err("Server disconnected before maintenance response.".into())
    })
    .await
    .map_err(|_| "Maintenance request timed out.".to_string())?
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
        assert!(client.commands.send("server.stop", json!({})).is_err());
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
}
