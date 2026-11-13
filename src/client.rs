//! KSG DesktopBackendManager lifecycle: attach first, supervise only our own child.
use crate::{
    client_history::ProjectionCache,
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
type AssetView = Option<(usize, Arc<Value>)>;
type Registration = (std::sync::Weak<AtomicBool>, std::sync::Weak<Worker>);
static SUPERVISORS: Mutex<Vec<Registration>> = Mutex::new(Vec::new());

enum HistoryRequest {
    Earlier(usize, Option<String>),
    Work(usize, String),
}
impl HistoryRequest {
    fn apply(self, cache: &mut ProjectionCache) {
        match self {
            Self::Earlier(thread, through) => cache.request_earlier(thread as u64, through),
            Self::Work(thread, anchor) => cache.request_work(thread as u64, anchor),
        }
    }
}

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
    Rpc {
        tag: String,
        result: Result<Value, String>,
    },
    Error(String),
    Maintenance(String, Value),
}

pub struct Client {
    pub remote: Option<crate::peer::Link>,
    pub linked: std::collections::BTreeMap<String, Client>,
    directory: PathBuf,
    device_directory: PathBuf,
    pub commands: Commands,
    pub info: Arc<Mutex<Value>>,
    input: Option<tokio::sync::mpsc::Receiver<Command>>,
    events: mpsc::Receiver<Event>,
    output: mpsc::SyncSender<Event>,
    snapshot: Arc<Mutex<Option<Value>>>,
    stop: Arc<AtomicBool>,
    worker: Arc<Worker>,
    history_thread: Arc<AtomicUsize>,
    history_requests: Arc<Mutex<VecDeque<HistoryRequest>>>,
    rpc_requests: Arc<Mutex<VecDeque<(String, String, Value)>>>,
}

impl Client {
    #[cfg(test)]
    pub(crate) fn test_commands(&mut self) -> tokio::sync::mpsc::Receiver<Command> {
        self.commands.ready.store(true, Ordering::Release);
        self.input.take().expect("unused test client")
    }
    #[cfg(test)]
    pub(crate) fn test_snapshot(&self, projection: Value) {
        *self.snapshot.lock().expect("client snapshot") = Some(projection);
    }
    #[cfg(test)]
    pub(crate) fn test_receipt(&self, command: Command, value: Value) {
        self.output.send(Event::Receipt(command, value)).unwrap();
    }

    #[cfg(test)]
    pub fn test_connected(directory: PathBuf) -> (Self, tokio::sync::mpsc::Receiver<Command>) {
        let mut client = Self::new(directory);
        let input = client.test_commands();
        (client, input)
    }

    pub fn new(directory: PathBuf) -> Self {
        let (sender, input) = tokio::sync::mpsc::channel(32);
        let (output, events) = mpsc::sync_channel(64);
        Self {
            info: Arc::default(),
            remote: None,
            linked: Default::default(),
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
            history_requests: Arc::default(),
            rpc_requests: Arc::default(),
        }
    }
    pub fn set_device_directory(&mut self, directory: PathBuf) {
        self.device_directory = directory;
    }
    pub fn start(&mut self, wake: rune::Waker) {
        if self.remote.is_none() {
            match crate::peer::Link::load_all(&self.directory) {
                Ok(links) => {
                    for link in links {
                        self.add_link(link, wake.clone());
                    }
                }
                Err(error) => {
                    let _ = self.output.try_send(Event::Error(error));
                }
            }
        }
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
        let history_requests = self.history_requests.clone();
        let rpc_requests = self.rpc_requests.clone();
        let remote = self.remote.clone();
        let info = self.info.clone();
        *self.worker.lock().expect("client worker") = Some(std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("client runtime");
            runtime.block_on(supervise(
                (
                    directory,
                    device_directory,
                    history_thread,
                    history_requests,
                    rpc_requests,
                    remote,
                ),
                info,
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
    pub fn add_link(&mut self, link: crate::peer::Link, wake: rune::Waker) {
        let mut client = Self::new(self.directory.clone());
        client.device_directory = self
            .device_directory
            .join("linked-views")
            .join(&link.server_id);
        client.remote = Some(link.clone());
        client.start(wake);
        self.linked.insert(link.server_id, client);
    }
    pub fn commands_for(&self, server: &str) -> Option<&Commands> {
        if server == "local" {
            Some(&self.commands)
        } else {
            self.linked.get(server).map(|client| &client.commands)
        }
    }
    pub fn snapshots(&self) -> Vec<(String, Value)> {
        let mut views = Vec::new();
        if let Some(value) = self.snapshot() {
            views.push(("local".into(), value));
        }
        for (id, client) in &self.linked {
            if let Some(value) = client.snapshot() {
                views.push((id.clone(), value));
            }
        }
        views
    }
    pub fn machine_event(&self) -> Option<(String, Event)> {
        self.event()
            .map(|event| ("local".into(), event))
            .or_else(|| {
                self.linked
                    .iter()
                    .find_map(|(id, client)| client.event().map(|event| (id.clone(), event)))
            })
    }
    pub fn snapshot(&self) -> Option<Value> {
        self.snapshot.lock().expect("client snapshot").take()
    }
    pub fn version(&self) -> Option<String> {
        self.info.lock().expect("connection info")["binaryVersion"]
            .as_str()
            .map(str::to_owned)
    }
    pub fn supports_capability(&self, server: &str, capability: &str) -> bool {
        let info = if server == "local" {
            &self.info
        } else if let Some(remote) = self.linked.get(server) {
            &remote.info
        } else {
            return false;
        };
        info.lock().expect("connection info")["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|cap| cap == capability))
    }
    pub fn can_approve(&self, server: &str) -> bool {
        let info = if server == "local" {
            &self.info
        } else if let Some(remote) = self.linked.get(server) {
            &remote.info
        } else {
            return false;
        };
        info.lock().expect("connection info")["canApprove"] == true
    }
    pub fn can_operate(&self, server: &str) -> bool {
        let info = if server == "local" {
            &self.info
        } else if let Some(remote) = self.linked.get(server) {
            &remote.info
        } else {
            return false;
        };
        info.lock().expect("connection info")["canOperate"] == true
    }

    /// Sends attachments already prepared by the composer transaction, without a second upload.
    pub(crate) fn send_prepared(
        &self,
        command: Command,
        wake: rune::Waker,
    ) -> Result<uuid::Uuid, String> {
        if !self.commands.connected() {
            return Err("This machine is disconnected. Your draft was kept.".into());
        }
        let Some(link) = self.remote.clone() else {
            return self.commands.send(&command.method, command.params);
        };
        let id = command.command_id;
        let output = self.output.clone();
        let stop = self.stop.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
                .and_then(|rt| {
                    rt.block_on(async {
                        let mut attempt = 0;
                        loop {
                            if stop.load(Ordering::Acquire) {
                                break Err("The client stopped before the send completed.".into());
                            }
                            match remote_request_outcome(&link, command.clone(), false).await {
                                Ok(RemoteResponse::Receipt(result)) => break Ok(result),
                                Ok(RemoteResponse::Refused(error)) => break Err(error),
                                Err(error)
                                    if stop.load(Ordering::Acquire)
                                        || error.starts_with("Authentication denied.")
                                        || error.starts_with("Server handshake refused")
                                        || error.contains("Update required") =>
                                {
                                    break Err(error);
                                }
                                Err(_) => {
                                    // The same UUID reaches the durable receipt store on every retry.
                                    tokio::time::sleep(restart_delay(attempt)).await;
                                    attempt = attempt.saturating_add(1);
                                }
                            }
                        }
                    })
                });
            let value = match result {
                Ok(result) => json!({"result":result}),
                Err(error) => json!({"error":{"message":error}}),
            };
            let _ = output.send(Event::Receipt(command, value));
            wake();
        });
        Ok(id)
    }
    pub fn show_history(&self, thread: usize) {
        self.history_thread.store(thread, Ordering::Release);
    }
    pub fn load_earlier_history(&self, thread: usize, through_entry_id: Option<String>) {
        self.history_requests
            .lock()
            .expect("history requests")
            .push_back(HistoryRequest::Earlier(thread, through_entry_id));
    }
    pub fn load_work_history(&self, thread: usize, anchor: String) {
        self.history_requests
            .lock()
            .expect("history requests")
            .push_back(HistoryRequest::Work(thread, anchor));
    }
    pub fn call_rpc(&self, tag: impl Into<String>, method: impl Into<String>, params: Value) {
        self.rpc_requests.lock().expect("rpc requests").push_back((
            tag.into(),
            method.into(),
            params,
        ));
    }
    pub fn connected_ready(&self) -> bool {
        self.commands.connected()
    }
    pub fn has_capability(&self, capability: &str) -> bool {
        self.info.lock().expect("connection info")["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|cap| cap == capability))
    }
    pub fn event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
    pub fn shutdown(&mut self) {
        for client in self.linked.values_mut() {
            client.shutdown();
        }
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

pub(crate) type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(
    discovery: &Discovery,
    device: &str,
    device_directory: &std::path::Path,
    cache: &ProjectionCache,
    desktop: bool,
    remote: Option<&crate::peer::Link>,
) -> Result<(Socket, Value), String> {
    if discovery.protocol_major != crate::protocol::MAJOR {
        return Err("Protocol major mismatch. Update the server and client together.".into());
    }
    let credential_path = device_directory.join("client-credential.json");
    let saved = if remote.is_none() {
        crate::store::load(&credential_path)?
    } else {
        None
    };
    let token = if let Some(remote) = remote {
        remote.token.clone()
    } else if let Some(saved) = saved {
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
        crate::server_auth::save_secret(
            &credential_path,
            json!({"serverId":discovery.server_id,"deviceId":device,"token":token})
                .to_string()
                .as_bytes(),
        )?;
        token
    };
    let endpoint = match remote {
        Some(link) => format!("ws://{}/v1/ws", link.address()?),
        None => format!("ws://127.0.0.1:{}/v1/ws", discovery.port),
    };
    let mut request = endpoint.into_client_request().map_err(|e| e.to_string())?;
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
    socket.send(Message::Text(json!({"jsonrpc":"2.0","id":"hello","method":"hello","params":{"protocolMajor":protocol::MAJOR,"protocolMinor":protocol::MINOR,"projectionSchema":1,"clientPid":(desktop && remote.is_none()).then(std::process::id),"maintenanceClient":!desktop}}).to_string().into())).await.map_err(|e| e.to_string())?;
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
    if let Some(error) = reply["error"]["message"].as_str() {
        return Err(format!("Server handshake refused: {error}"));
    }
    if remote.is_none() && reply["result"]["instanceId"] != discovery.instance_id
        || reply["result"]["serverId"] != discovery.server_id
    {
        return Err(
            "Server handshake refused or instance changed. Check client/server versions.".into(),
        );
    }
    if desktop {
        socket
        .send(Message::Text(
            json!({"jsonrpc":"2.0","id":"subscribe","method":"workspace.subscribe","params":{"lastSeq":cache.seq,"epoch":cache.epoch}})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok((socket, reply["result"].clone()))
}

#[allow(clippy::too_many_arguments)]
async fn supervise(
    profile: (
        PathBuf,
        PathBuf,
        Arc<AtomicUsize>,
        Arc<Mutex<VecDeque<HistoryRequest>>>,
        Arc<Mutex<VecDeque<(String, String, Value)>>>,
        Option<crate::peer::Link>,
    ),
    info: Arc<Mutex<Value>>,
    mut input: tokio::sync::mpsc::Receiver<Command>,
    output: mpsc::SyncSender<Event>,
    snapshot: Arc<Mutex<Option<Value>>>,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    wake: rune::Waker,
) {
    let (directory, device_directory, history_thread, history_requests, rpc_requests, remote) =
        profile;
    let info_file = device_directory.join("connection-info.json");
    if let Ok(bytes) = std::fs::read(&info_file)
        && let Ok(value) = serde_json::from_slice(&bytes)
    {
        *info.lock().expect("connection info") = value;
    }
    let report = |error: String| {
        let _ = output.try_send(Event::Error(error));
        wake();
    };
    if let Err(error) = std::fs::create_dir_all(&directory) {
        report(error.to_string());
        return;
    }
    let device = match remote
        .as_ref()
        .map(|link| Ok(link.device_id.clone()))
        .unwrap_or_else(|| device_id(&device_directory))
    {
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
    let mut cache = ProjectionCache::default();
    let mut pending_rpcs = std::collections::HashMap::<String, String>::new();
    let mut rpc_seq = 0u64;
    let image_generation = Arc::new(AtomicUsize::new(0));
    let (asset_changed, asset_updates) = tokio::sync::watch::channel(None);
    let image_job = remote.as_ref().map(|remote| {
        let output = output.clone();
        let error_wake = wake.clone();
        tokio::spawn(fetch_assets(
            remote.clone(),
            directory.clone(),
            snapshot.clone(),
            image_generation.clone(),
            asset_updates,
            wake.clone(),
            Arc::new(move |error| {
                let _ = output.try_send(Event::Error(error));
                error_wake();
            }),
        ))
    });
    let view_writer = remote.as_ref().map(|_| {
        let output = output.clone();
        let wake = wake.clone();
        crate::cache_persistence::Writer::new(device_directory.join("view.json"), move |error| {
            let _ = output.try_send(Event::Error(error));
            wake();
        })
    });
    if remote.is_some() {
        match crate::store::load(&device_directory.join("view.json")) {
            Ok(Some(saved)) if saved["schemaVersion"] == 1 => {
                cache.state = saved["state"].clone();
                cache.seq = saved["seq"].as_u64();
                cache.epoch = saved["epoch"].clone();
                cache.dirty = true;
                if let Some(mut projected) = cache.projected() {
                    if let Some(remote) = &remote {
                        let _ = cache_images(remote, &directory, &mut projected, false).await;
                    }
                    *snapshot.lock().expect("client snapshot") = Some(projected);
                    wake();
                }
            }
            Ok(_) => {}
            Err(error) => report(error),
        }
    }
    'supervisor: while !stop.load(Ordering::Acquire) {
        cache.desired_thread = match history_thread.load(Ordering::Acquire) {
            0 => None,
            id => Some(id as u64),
        };
        while let Some(request) = history_requests
            .lock()
            .expect("history requests")
            .pop_front()
        {
            request.apply(&mut cache);
        }
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
            let discovery = remote
                .as_ref()
                .map(remote_discovery)
                .map(Ok)
                .unwrap_or_else(|| Discovery::read(&directory));
            if let Ok(discovery) = discovery {
                match tokio::select! {
                    result = connect(&discovery, &device, &device_directory, &cache, true, remote.as_ref()) => result,
                    _ = stopped(&stop) => break,
                } {
                    Ok((mut connected, hello)) => {
                        let metadata = json!({"serverId":hello["serverId"],"binaryVersion":hello["binaryVersion"],"protocolMinor":hello["protocolMinor"],"canUpdate":hello["canUpdate"],"canApprove":hello["canApprove"],"canOperate":hello["canOperate"],"capabilities":hello["capabilities"]});
                        *info.lock().expect("connection info") = metadata.clone();
                        let _ =
                            crate::store::write_atomic(&info_file, metadata.to_string().as_bytes());
                        wake();
                        if let Err(error) = cache.reconnect(&hello) {
                            report(error);
                            break;
                        }
                        if child
                            .as_ref()
                            .is_some_and(|child| child.id() == discovery.pid)
                        {
                            owned_instance = Some(discovery.instance_id);
                        }
                        let mut sent = true;
                        pending.retain(|command| {
                            if command.method == "thread.user-input.dismiss"
                                && !hello["capabilities"].as_array().is_some_and(|caps| {
                                    caps.iter().any(|cap| cap == protocol::QUESTION_DISMISS)
                                })
                            {
                                let _ = output.try_send(Event::Receipt(command.clone(), json!({"error":{"message":protocol::QUESTION_DISMISS_UPDATE_REQUIRED}})));
                                wake();
                                false
                            } else {
                                true
                            }
                        });
                        for command in &pending {
                            match uploaded_command(
                                command,
                                remote.as_ref(),
                                hello["capabilities"].as_array().is_some_and(|caps| {
                                    caps.iter().any(|cap| cap == protocol::FILE_ATTACHMENTS)
                                }),
                            )
                            .await
                            {
                                Ok(command) => {
                                    sent &= send(&mut connected, wire_command(&command)).await
                                }
                                Err(error) => {
                                    if error.starts_with("Authentication denied.") {
                                        report(error);
                                        break 'supervisor;
                                    }
                                    sent = false;
                                    break;
                                }
                            }
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
                    Err(_) => {
                        next_spawn = Instant::now() + restart_delay(attempt);
                        attempt = attempt.saturating_add(1);
                    }
                }
            }
            if remote.is_some() {
                tokio::select! {
                    _ = tokio::time::sleep(next_spawn.saturating_duration_since(Instant::now()).max(Duration::from_millis(100))) => {},
                    _ = stopped(&stop) => break,
                }
                continue;
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
                if command.method == "thread.user-input.dismiss"
                    && !info.lock().expect("connection info")["capabilities"].as_array()
                        .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::QUESTION_DISMISS))
                {
                    let _ = output.try_send(Event::Receipt(command, json!({"error":{"message":protocol::QUESTION_DISMISS_UPDATE_REQUIRED}})));
                    wake();
                    continue;
                }
                if command.params["questionAttachments"] == true
                    && !info.lock().expect("connection info")["capabilities"].as_array()
                        .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::QUESTION_ATTACHMENTS))
                {
                    let _ = output.try_send(Event::Receipt(command, json!({"error":{"message":protocol::QUESTION_ATTACHMENTS_UPDATE_REQUIRED}})));
                    wake();
                    continue;
                }
                if command.params["questionContract"] == true
                    && !info.lock().expect("connection info")["capabilities"].as_array()
                        .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::QUESTION_CONTRACT))
                {
                    let _ = output.try_send(Event::Receipt(command, json!({"error":{"message":protocol::QUESTIONS_UPDATE_REQUIRED}})));
                    wake();
                    continue;
                }
                if command.method == "request.respond" && command.params["response"].get("feedback").is_some()
                    && !info.lock().expect("connection info")["capabilities"].as_array()
                        .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::APPROVAL_FEEDBACK))
                {
                    let _ = output.try_send(Event::Receipt(command, json!({"error":{"message":protocol::APPROVAL_FEEDBACK_UPDATE_REQUIRED}})));
                    wake();
                    continue;
                }
                if command.params["requestContract"] == true
                    && !info.lock().expect("connection info")["capabilities"].as_array()
                        .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::COMPOSER_REQUESTS))
                {
                    let _ = output.try_send(Event::Receipt(command, json!({"error":{"message":protocol::REQUESTS_UPDATE_REQUIRED}})));
                    wake();
                    continue;
                }
                if matches!(command.method.as_str(), "queue.update" | "queue.returnDraft")
                    && !info.lock().expect("connection info")["capabilities"].as_array()
                        .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::COMPOSER_TRANSACTIONS))
                {
                    let _ = output.try_send(Event::Receipt(command, json!({"error":{"message":protocol::COMPOSER_UPDATE_REQUIRED}})));
                    wake();
                    continue;
                }
                if command.method == "thread.resume"
                    && !info.lock().expect("connection info")["capabilities"].as_array()
                        .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::RUN_CONTROL))
                {
                    let _ = output.try_send(Event::Receipt(command, json!({"error":{"message":protocol::RUN_CONTROL_UPDATE_REQUIRED}})));
                    wake();
                    continue;
                }
                let has_composer_context = info.lock().expect("connection info")["capabilities"]
                    .as_array()
                    .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::COMPOSER_CONTEXT));
                let command_to_send = downgrade_composer_context_command(command.clone(), has_composer_context);
                let has_file_attachments = info.lock().expect("connection info")["capabilities"]
                    .as_array().is_some_and(|caps| caps.iter().any(|cap| cap == protocol::FILE_ATTACHMENTS));
                let prepared = match uploaded_command(&command_to_send,remote.as_ref(),has_file_attachments).await {
                    Ok(command) => command,
                    Err(error) => {
                        let denied = error.starts_with("Authentication denied.");
                        let _ = output.try_send(Event::Receipt(command, json!({"error":{"message":error}})));
                        wake();
                        if denied { break 'supervisor; }
                        continue;
                    }
                };
                let message = wire_command(&prepared);
                pending.push_back(command);
                if !send(connected, message).await { socket = None; }
            }
            message = connected.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            let mut disconnect = false;
                            if let Some(method) = value["method"].as_str().filter(|m| matches!(*m, "server.draining" | "server.restarting" | "server.drainCancelled" | "server.stopping")) {
                                let draining = method != "server.drainCancelled";
                                server_stopped |= method == "server.stopping";
                                cache.maintenance = value["params"].clone();
                                ready.store(!draining, Ordering::Release);
                                if output.try_send(Event::Maintenance(method.into(), value["params"].clone())).is_err() {
                                    disconnect = true;
                                }
                                if method == "server.restarting" { updating = true; }
                            }
                            if let Err(error) = cache.receive(&value) {
                                cache = ProjectionCache::default();
                                socket = None;
                                report(error);
                                continue;
                            }
                            for notice in cache.notices.drain(..) {
                                report(notice);
                            }
                            if let Some(projected) = cache.projected() {
                                if let Some(writer) = &view_writer {
                                    writer.submit(json!({"schemaVersion":1,"state":projected,"seq":cache.seq,"epoch":cache.epoch}));
                                }
                                let asset_view=image_job.as_ref().filter(|job|!job.is_finished()).map(|_|Arc::new(projected.clone()));
                                let generation={
                                    let mut snapshot=snapshot.lock().expect("client snapshot");
                                    let generation=image_generation.fetch_add(1,Ordering::AcqRel)+1;
                                    *snapshot=Some(projected);
                                    generation
                                };
                                if let Some(view)=asset_view {
                                    asset_changed.send_replace(Some((generation,view)));
                                }
                            }
                            if value["method"] == "subscription.ready" || value["id"] == "subscribe" {
                                let phase = cache.maintenance["phase"].as_str().unwrap_or("ready");
                                server_stopped |= phase == "stopping";
                                updating |= phase == "restarting";
                                ready.store(cache.seq.is_some() && phase == "ready",Ordering::Release);
                            }
                            if let Some(id) = value["id"].as_str()
                                && let Some(tag) = pending_rpcs.remove(id)
                            {
                                let result = if let Some(msg) = value["error"]["message"].as_str() {
                                    Err(msg.to_owned())
                                } else {
                                    Ok(value["result"].clone())
                                };
                                let _ = output.try_send(Event::Rpc { tag, result });
                            } else if let Some(id) = value["id"].as_str().and_then(|id| uuid::Uuid::parse_str(id).ok())
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
            _ = tokio::time::sleep(Duration::from_millis(25)) => {
                while let Some(request) = history_requests
                    .lock()
                    .expect("history requests")
                    .pop_front()
                {
                    request.apply(&mut cache);
                }
                let mut disconnect = false;
                while let Some((tag, method, params)) =
                    rpc_requests.lock().expect("rpc requests").pop_front()
                {
                    rpc_seq += 1;
                    let rpc_id = format!("rpc-{rpc_seq}");
                    pending_rpcs.insert(rpc_id.clone(), tag);
                    let req = json!({"jsonrpc":"2.0","id":rpc_id,"method":method,"params":params});
                    if !send(connected, Message::Text(req.to_string().into())).await {
                        disconnect = true;
                        break;
                    }
                }
                if let Some(projected) = cache.projected() {
                    *snapshot.lock().expect("client snapshot") = Some(projected);
                    wake();
                }
                if !disconnect
                    && let Some(request) = cache.next_history()
                    && !send(connected, Message::Text(request.to_string().into())).await
                {
                    disconnect = true;
                }
                if disconnect {
                    socket = None;
                }
            }
        }
    }
    ready.store(false, Ordering::Release);
    if let Some(job) = image_job {
        job.abort();
    }
    drop(view_writer);
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

/// KSG assets.ts keys in-flight work by environment/resource, not projection.
/// One consumer retains a fetch across projection updates. Completed blob IDs
/// enter the existing content-addressed cache and hydrate the newest view only.
async fn fetch_assets(
    remote: crate::peer::Link,
    profile: PathBuf,
    snapshot: Arc<Mutex<Option<Value>>>,
    generation: Arc<AtomicUsize>,
    mut updates: tokio::sync::watch::Receiver<AssetView>,
    wake: rune::Waker,
    report: Arc<dyn Fn(String) + Send + Sync>,
) {
    while updates.changed().await.is_ok() {
        let view = updates.borrow_and_update().clone();
        let Some((_, view)) = view else {
            continue;
        };
        let mut projected = (*view).clone();
        if let Err(error) = cache_images(&remote, &profile, &mut projected, true).await {
            if error.starts_with("Authentication denied.") {
                report(error);
                return;
            }
            // Asset failure does not fence state reception or create a tight retry loop.
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        // The UI consumes pending snapshots with take(). Keep the latest asset
        // source independently so a completed fetch can still wake an idle UI.
        let latest = updates.borrow().clone();
        let Some((version, current)) = latest else {
            continue;
        };
        let mut current = (*current).clone();
        if cache_images(&remote, &profile, &mut current, false)
            .await
            .is_ok()
        {
            let mut snapshot = snapshot.lock().expect("client snapshot");
            if generation.load(Ordering::Acquire) == version {
                *snapshot = Some(current);
                wake();
            }
        }
    }
}

async fn cache_images(
    remote: &crate::peer::Link,
    profile: &std::path::Path,
    projected: &mut Value,
    fetch: bool,
) -> Result<(), String> {
    let file_error = cache_files(remote, profile, projected, fetch).await.err();
    if file_error
        .as_ref()
        .is_some_and(|error| error.starts_with("Authentication denied."))
    {
        return Err(file_error.unwrap());
    }
    cache_html(remote, profile, projected, fetch).await?;
    fn paths(value: &Value, prefix: String, out: &mut Vec<String>) {
        match value {
            Value::Object(object)
                if object
                    .get("mime")
                    .and_then(Value::as_str)
                    .is_some_and(|mime| mime.starts_with("image/"))
                    && object
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(crate::attachments::safe_id)
                    && object.get("path").is_none_or(Value::is_null) =>
            {
                out.push(prefix)
            }
            Value::Object(object) => {
                for (key, value) in object {
                    paths(
                        value,
                        format!("{prefix}/{}", key.replace('~', "~0").replace('/', "~1")),
                        out,
                    );
                }
            }
            Value::Array(array) => {
                for (index, value) in array.iter().enumerate() {
                    paths(value, format!("{prefix}/{index}"), out);
                }
            }
            _ => {}
        }
    }
    let mut references = Vec::new();
    paths(projected, String::new(), &mut references);
    if references.is_empty() {
        return Ok(());
    }
    let http = crate::peer::http()?;
    for path in references {
        let reference = &projected.pointer(&path).ok_or("Missing image reference.")?;
        if cached_image_valid(profile, reference) {
            continue;
        }
        let id = reference["id"].as_str().ok_or("Missing image id.")?;
        let length = reference["bytes"]
            .as_u64()
            .filter(|length| *length > 0 && *length <= crate::attachments::MAX_IMAGE_BYTES)
            .ok_or("Invalid image length.")?;
        let mapping = remote
            .directory(profile)
            .join("blobs")
            .join(format!("{id}.json"));
        let cached = crate::store::load(&mapping)
            .ok()
            .flatten()
            .filter(|value| cached_image_valid(profile, value));
        let attachment = if let Some(cached) = cached {
            cached
        } else {
            if !fetch {
                continue;
            }
            let address = remote.address()?;
            let mut bytes = Vec::new();
            while (bytes.len() as u64) < length {
                let response = http
                    .get(format!(
                        "http://{address}/v1/blobs/{id}?offset={}&limit={}",
                        bytes.len(),
                        protocol::PAGE_BYTES
                    ))
                    .bearer_auth(&remote.token)
                    .send()
                    .await
                    .map_err(|_| "Remote image unavailable.")?;
                if matches!(response.status().as_u16(), 401 | 403) {
                    return Err("Authentication denied. Reconnect this linked machine manually before fetching assets.".into());
                }
                if !response.status().is_success()
                    || response
                        .content_length()
                        .is_none_or(|count| count > protocol::MAX_PAGE_BYTES as u64)
                {
                    return Err("Invalid remote blob response.".into());
                }
                let chunk = response
                    .bytes()
                    .await
                    .map_err(|_| "Remote image interrupted.")?;
                if chunk.is_empty() || bytes.len() + chunk.len() > length as usize {
                    return Err("Invalid remote blob length.".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            let image = crate::attachments::store_uploaded(
                &profile.join("attachments"),
                reference["name"].as_str().unwrap_or("Image"),
                &bytes,
            )?;
            let value = image.to_value();
            crate::store::write_atomic(&mapping, value.to_string().as_bytes())
                .map_err(|e| e.to_string())?;
            value
        };
        *projected
            .pointer_mut(&path)
            .ok_or("Missing image reference.")? = attachment;
    }
    file_error.map_or(Ok(()), Err)
}

/// KSG stages uploaded file resources per environment. Keep ordinary-file bytes in the
/// same immutable content store as uploads, never trust the source machine's path.
pub(crate) async fn cache_files(
    remote: &crate::peer::Link,
    profile: &std::path::Path,
    projected: &mut Value,
    fetch: bool,
) -> Result<(), String> {
    fn references(value: &Value, prefix: String, out: &mut Vec<String>) {
        match value {
            Value::Object(object)
                if object.get("mime").and_then(Value::as_str).is_some()
                    && object
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(crate::attachments::safe_file_id) =>
            {
                out.push(prefix)
            }
            Value::Object(object) => {
                for (key, value) in object {
                    references(
                        value,
                        format!("{prefix}/{}", key.replace('~', "~0").replace('/', "~1")),
                        out,
                    );
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    references(value, format!("{prefix}/{index}"), out);
                }
            }
            _ => {}
        }
    }
    let mut paths = Vec::new();
    references(projected, String::new(), &mut paths);
    let http = crate::peer::http()?;
    let mut failure = None;
    for path in paths {
        let result: Result<(),String> = async {
        let reference = projected.pointer(&path).ok_or("Missing file reference.")?;
        let id = reference["id"]
            .as_str()
            .ok_or("Missing file id.")?
            .to_owned();
        let length = reference["bytes"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= crate::attachments::MAX_FILE_BYTES)
            .ok_or("Invalid file length.")?;
        let local_path = profile.join("attachments").join(&id);
        let mut local = reference.clone();
        local["path"] = json!(local_path);
        let valid =
            std::fs::symlink_metadata(&local_path).is_ok_and(|m| m.is_file() && m.len() == length);
        if !valid && !fetch {
            // A foreign path may happen to exist locally. It is never a cached file.
            *projected.pointer_mut(&path).unwrap() = local;
            return Ok(());
        }
        if !valid {
            let address = remote.address()?;
            let mut bytes = Vec::new();
            while (bytes.len() as u64) < length {
                let response = http
                    .get(format!(
                        "http://{address}/v1/blobs/{id}?offset={}&limit={}",
                        bytes.len(),
                        protocol::PAGE_BYTES
                    ))
                    .bearer_auth(&remote.token)
                    .send()
                    .await
                    .map_err(|_| "The environment it came from is not reachable right now.")?;
                if matches!(response.status().as_u16(), 401 | 403) {
                    return Err("Authentication denied. Reconnect this linked machine manually before fetching assets.".into());
                }
                if !response.status().is_success()
                    || response
                        .content_length()
                        .is_none_or(|n| n > protocol::MAX_PAGE_BYTES as u64)
                {
                    return Err("The original attachment is no longer available.".into());
                }
                let chunk = response
                    .bytes()
                    .await
                    .map_err(|_| "Downloading it from the source failed.")?;
                if chunk.is_empty() || bytes.len() + chunk.len() > length as usize {
                    return Err("Invalid remote file length.".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            use sha2::Digest;
            if id != format!("file-{:x}.bin", sha2::Sha256::digest(&bytes)) {
                return Err("File hash mismatch.".into());
            }
            local = crate::attachments::store_uploaded_file(
                &profile.join("attachments"),
                reference["name"].as_str().ok_or("Missing file name.")?,
                reference["mime"]
                    .as_str()
                    .ok_or("Missing file MIME type.")?,
                &bytes,
            )?
            .to_value();
        }
        *projected.pointer_mut(&path).unwrap() = local;
        Ok(())
        }.await;
        if let Err(error) = result {
            if error.starts_with("Authentication denied.") {
                return Err(error);
            }
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn cached_image_valid(profile: &std::path::Path, value: &Value) -> bool {
    let Ok(attachment) = serde_json::from_value::<crate::attachments::Attachment>(value.clone())
    else {
        return false;
    };
    attachment.path.is_none()
        && crate::attachments::safe_id(&attachment.id)
        && matches!(
            attachment.mime.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        )
        && attachment.width > 0
        && attachment.height > 0
        && u64::from(attachment.width) * u64::from(attachment.height)
            <= crate::attachments::MAX_IMAGE_PIXELS
        && attachment.bytes > 0
        && attachment.bytes <= crate::attachments::MAX_IMAGE_BYTES
        && std::fs::metadata(profile.join("attachments").join(&attachment.id))
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == attachment.bytes)
}

pub(crate) async fn cache_html(
    remote: &crate::peer::Link,
    profile: &std::path::Path,
    projected: &Value,
    fetch: bool,
) -> Result<(), String> {
    fn references(value: &Value, out: &mut Vec<crate::html_render::Reference>, depth: usize) {
        if depth == 0 || out.len() >= 32 {
            return;
        }
        if let Some(reference) = crate::html_render::Reference::read(&value["htmlRender"]) {
            if !out
                .iter()
                .any(|r| r.attachment_id == reference.attachment_id)
            {
                out.push(reference);
            }
            return;
        }
        match value {
            Value::Object(object) => {
                for value in object.values() {
                    references(value, out, depth - 1);
                }
            }
            Value::Array(values) => {
                for value in values {
                    references(value, out, depth - 1);
                }
            }
            _ => {}
        }
    }
    if !fetch {
        return Ok(());
    }
    let mut records = vec![];
    references(projected, &mut records, 24);
    if records.is_empty() {
        return Ok(());
    }
    let http = crate::peer::http()?;
    let address = remote.address()?;
    let directory = profile.join("attachments");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    for reference in records {
        for blob in reference.blobs() {
            let path = directory.join(&blob.id);
            if std::fs::metadata(&path).is_ok_and(|m| m.len() == blob.bytes) {
                continue;
            }
            let mut bytes = Vec::new();
            while bytes.len() < blob.bytes as usize {
                let response = http
                    .get(format!(
                        "http://{address}/v1/blobs/{}?offset={}&limit={}",
                        blob.id,
                        bytes.len(),
                        protocol::PAGE_BYTES
                    ))
                    .bearer_auth(&remote.token)
                    .send()
                    .await
                    .map_err(|_| "Remote HTML render unavailable.")?;
                if matches!(response.status().as_u16(), 401 | 403) {
                    return Err("Authentication denied. Reconnect this linked machine manually before fetching assets.".into());
                }
                if !response.status().is_success()
                    || response
                        .content_length()
                        .is_none_or(|n| n > protocol::MAX_PAGE_BYTES as u64)
                {
                    return Err("Invalid remote HTML blob response.".into());
                }
                let chunk = response
                    .bytes()
                    .await
                    .map_err(|_| "Remote HTML render interrupted.")?;
                if chunk.is_empty() || bytes.len() + chunk.len() > blob.bytes as usize {
                    return Err("Invalid remote HTML blob length.".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            if blob.id.ends_with(".png") {
                let image = image::ImageReader::with_format(
                    std::io::Cursor::new(&bytes),
                    image::ImageFormat::Png,
                )
                .into_dimensions()
                .map_err(|_| "Invalid remote HTML snapshot.")?;
                let snapshot = [&reference.light, &reference.dark]
                    .into_iter()
                    .flatten()
                    .find(|s| s.blob.id == blob.id)
                    .ok_or("Unbound snapshot.")?;
                if image != (snapshot.width, snapshot.height) {
                    return Err("Remote snapshot dimensions mismatch.".into());
                }
            } else if std::str::from_utf8(&bytes).is_err() {
                return Err("Invalid remote HTML encoding.".into());
            }
            crate::store::write_atomic(&path, &bytes).map_err(|e| e.to_string())?;
        }
        crate::html_render::import(&directory, &reference)?;
    }
    Ok(())
}

/// Strip the additive `context` field and pre-project `t3-context://` references into
/// `text` when talking to an older server without `composer-context-v1` (whose `SendInput`
/// has `deny_unknown_fields`).
pub(crate) fn downgrade_composer_context_command(
    mut command: Command,
    has_composer_context: bool,
) -> Command {
    if has_composer_context || !matches!(command.method.as_str(), "thread.send" | "queue.update") {
        return command;
    }
    if let Some(obj) = command.params.as_object_mut() {
        let context = obj.remove("context").filter(|v| !v.is_null());
        if let Some(text) = obj.get("text").and_then(Value::as_str).map(str::to_owned)
            && !crate::composer_context::collect_composer_context_references(&text).is_empty()
        {
            let records = context
                .and_then(|v| crate::composer_context::decode_message_context(&v).ok())
                .map(|c| c.records)
                .unwrap_or_default();
            obj.insert(
                "text".into(),
                json!(
                    crate::composer_context::project_composer_context_for_provider(&text, &records)
                ),
            );
            // Older server validates `markdown(&editor) == prompt.text`, which would fail
            // once `text` is projected; omit `draft` when links were pre-projected.
            obj.remove("draft");
        }
    }
    command
}

pub(crate) async fn uploaded_command(
    command: &Command,
    remote: Option<&crate::peer::Link>,
    files_supported: bool,
) -> Result<Command, String> {
    let mut prepared = command.clone();
    let Some(remote) =
        remote.filter(|_| matches!(command.method.as_str(), "thread.send" | "queue.update"))
    else {
        return Ok(prepared);
    };
    let Some(attachments) = prepared.params["attachments"].as_array_mut() else {
        return Ok(prepared);
    };
    let http = crate::peer::http()?;
    let address = remote.address()?;
    let base = format!("http://{address}");
    let mut replacements = std::collections::HashMap::new();
    for attachment in attachments {
        if command.method == "queue.update"
            && command.params["existingAttachmentIds"]
                .as_array()
                .is_some_and(|ids| ids.contains(&attachment["id"]))
        {
            continue;
        }
        let ordinary = !attachment["path"].is_null()
            || attachment["id"]
                .as_str()
                .is_some_and(crate::attachments::safe_file_id);
        if ordinary && !files_supported {
            return Err(protocol::FILE_ATTACHMENTS_UPDATE_REQUIRED.into());
        }
        if ordinary
            && command.params["draft"]["attachments"]
                .as_array()
                .is_some_and(|files| !files.iter().any(|file| file["id"] == attachment["id"]))
            && !command.params["context"]["records"]
                .as_array()
                .is_some_and(|records| {
                    records.iter().any(|record| {
                        record["kind"] == "file" && record["attachmentId"] == attachment["id"]
                    })
                })
        {
            return Err("Local file paths cannot be sent to another machine. Attach the file or reference a path on that server in your message.".into());
        }
        let image = crate::attachments::Attachment::from_value(attachment)
            .ok_or("The attachment is missing from this device.")?;
        let bytes = if ordinary {
            crate::attachments::transfer_file_bytes(&image)?
        } else {
            crate::attachments::read(&image)?
        };
        let original_id = image.id.clone();
        use sha2::Digest;
        let response = http.post(format!("{base}/v1/uploads")).bearer_auth(&remote.token)
            .json(&json!({"type":if ordinary {"file"} else {"image"},"mimeType":image.mime,
                "name":image.name,"length":bytes.len(),"sha256":format!("{:x}",sha2::Sha256::digest(&bytes))}))
            .send().await.map_err(|_| "Attachment upload could not start. Your draft was kept.")?;
        if !response.status().is_success() {
            return Err(
                upload_error(response, "Attachment upload refused. Your draft was kept.").await,
            );
        }
        let value: Value = response
            .json()
            .await
            .map_err(|_| "Invalid upload response.")?;
        let id = value["uploadId"]
            .as_str()
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            .ok_or("Invalid upload identity.")?;
        let mut offset = 0;
        for chunk in bytes.chunks(protocol::MAX_PAGE_BYTES) {
            let response = http
                .put(format!("{base}/v1/uploads/{id}?offset={offset}"))
                .bearer_auth(&remote.token)
                .body(chunk.to_vec())
                .send()
                .await
                .map_err(|_| "Attachment upload interrupted. Your draft was kept.")?;
            if !response.status().is_success() {
                return Err(upload_error(
                    response,
                    "Attachment upload offset refused. Your draft was kept.",
                )
                .await);
            }
            offset += chunk.len();
        }
        let response = http
            .post(format!("{base}/v1/uploads/{id}/commit"))
            .bearer_auth(&remote.token)
            .send()
            .await
            .map_err(|_| "Attachment commit interrupted. Your draft was kept.")?;
        if !response.status().is_success() {
            return Err(upload_error(
                response,
                "Attachment validation failed. Your draft was kept.",
            )
            .await);
        }
        *attachment = response
            .json()
            .await
            .map_err(|_| "Invalid committed attachment.")?;
        if ordinary && attachment["id"] != format!("file-{:x}.bin", sha2::Sha256::digest(&bytes)) {
            return Err("File hash mismatch.".into());
        }
        replacements.insert(original_id, attachment.clone());
    }
    rewrite_uploaded_attachments(&mut prepared.params, &replacements);
    Ok(prepared)
}

async fn upload_error(response: reqwest::Response, fallback: &str) -> String {
    if matches!(response.status().as_u16(), 401 | 403) {
        return "Authentication denied. Reconnect this linked machine manually before transferring files.".into();
    }
    response
        .json::<Value>()
        .await
        .ok()
        .and_then(|value| value["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| fallback.into())
}

fn rewrite_uploaded_attachments(
    value: &mut Value,
    replacements: &std::collections::HashMap<String, Value>,
) {
    if let Some(replacement) = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|_| value.get("mime").is_some())
        .and_then(|id| replacements.get(id))
    {
        *value = replacement.clone();
        return;
    }
    match value {
        Value::Object(object) => {
            if let Some(id) = object.get("attachmentId").and_then(Value::as_str)
                && let Some(replacement) = replacements.get(id)
            {
                object.insert("attachmentId".into(), replacement["id"].clone());
                object.insert("sizeBytes".into(), replacement["bytes"].clone());
            }
            for value in object.values_mut() {
                rewrite_uploaded_attachments(value, replacements);
            }
        }
        Value::Array(values) => {
            for value in values {
                rewrite_uploaded_attachments(value, replacements);
            }
        }
        _ => {}
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
        let device_directory = directory.join("clients").join("maintenance");
        std::fs::create_dir_all(&device_directory).map_err(|e| e.to_string())?;
        let _owner = crate::server_lock::Owner::acquire(&device_directory)?;
        let device = device_id(&device_directory)?;
        let (mut socket, _) = connect(
            &discovery,
            &device,
            &device_directory,
            &ProjectionCache::default(),
            false,
            None,
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

fn remote_discovery(link: &crate::peer::Link) -> Discovery {
    Discovery {
        server_id: link.server_id.clone(),
        instance_id: String::new(),
        epoch: String::new(),
        pid: 0,
        started_at: 0,
        port: 0,
        protocol_major: protocol::MAJOR,
        protocol_minor: protocol::MINOR,
        binary_version: String::new(),
        bootstrap_token: String::new(),
    }
}

pub async fn remote_rpc(
    link: &crate::peer::Link,
    method: &str,
    args: Value,
) -> Result<Value, String> {
    remote_request(
        link,
        Command::new(method, args),
        method.starts_with("peer."),
    )
    .await
}

pub async fn remote_command(link: &crate::peer::Link, command: Command) -> Result<Value, String> {
    remote_request(link, command, true).await
}

async fn remote_request(
    link: &crate::peer::Link,
    command: Command,
    peer: bool,
) -> Result<Value, String> {
    match remote_request_outcome(link, command, peer).await? {
        RemoteResponse::Receipt(value) => Ok(value),
        RemoteResponse::Refused(error) => Err(error),
    }
}

enum RemoteResponse {
    Receipt(Value),
    Refused(String),
}

async fn remote_request_outcome(
    link: &crate::peer::Link,
    command: Command,
    peer: bool,
) -> Result<RemoteResponse, String> {
    let timeout = if command.method == "peer.thread.wait" {
        58
    } else if command.method == "request.respond" && command.params["questionAttachments"] == true {
        let files = command.params["attachmentsByQuestionId"]
            .as_object()
            .into_iter()
            .flat_map(|map| map.values())
            .filter_map(Value::as_array)
            .map(Vec::len)
            .sum::<usize>();
        8 + 16 * files as u64
    } else {
        8
    };
    tokio::time::timeout(Duration::from_secs(timeout), async {
        // Pin identity before sending a peer credential, then negotiate the
        // required contracts. Later additive protocol minors remain compatible.
        let pinned = if peer {
            Some(crate::peer::pin(link).await?)
        } else {
            None
        };
        let link = pinned.as_ref().unwrap_or(link);
        let (mut socket, hello) = connect(
            &remote_discovery(link),
            &link.device_id,
            std::path::Path::new(""),
            &ProjectionCache::default(),
            false,
            Some(link),
        )
        .await?;
        if command.method == "hello" {
            return Ok(RemoteResponse::Receipt(hello));
        }
        if command.method == "thread.user-input.dismiss"
            && !hello["capabilities"]
                .as_array()
                .is_some_and(|caps| caps.iter().any(|cap| cap == protocol::QUESTION_DISMISS))
        {
            return Ok(RemoteResponse::Refused(
                protocol::QUESTION_DISMISS_UPDATE_REQUIRED.into(),
            ));
        }
        for (marker, capability, error) in [
            (
                "questionContract",
                protocol::QUESTION_CONTRACT,
                protocol::QUESTIONS_UPDATE_REQUIRED,
            ),
            (
                "questionAttachments",
                protocol::QUESTION_ATTACHMENTS,
                protocol::QUESTION_ATTACHMENTS_UPDATE_REQUIRED,
            ),
        ] {
            if command.params[marker] == true
                && !hello["capabilities"]
                    .as_array()
                    .is_some_and(|caps| caps.iter().any(|c| c == capability))
            {
                return Ok(RemoteResponse::Refused(error.into()));
            }
        }
        if peer
            && command.method != "peer.capabilities"
            && !hello["capabilities"].as_array().is_some_and(|caps| {
                caps.iter().any(|cap| cap == protocol::THREAD_READ)
                    && (matches!(
                        command.method.as_str(),
                        "peer.thread.read" | "peer.thread.list"
                    ) || caps.iter().any(|cap| cap == protocol::PEER_TASKS))
            })
        {
            return Err(format!(
                "Update required on {}. Run shina update --yes on that machine.",
                link.host
            ));
        }
        if command.method == "filesystem.browse" && !supports_browse(&hello) {
            return Err(
                "This server needs an update to browse folders. Type a path instead.".into(),
            );
        }
        if matches!(
            command.method.as_str(),
            "server.updateStatus" | "server.requestUpdate"
        ) && hello["protocolMinor"].as_u64().unwrap_or(0) < 6
        {
            return Err("older protocol".into());
        }
        socket
            .send(wire_command(&command))
            .await
            .map_err(|_| "Remote command could not be sent.")?;
        while let Some(message) = socket.next().await {
            match message.map_err(|_| "Remote server disconnected.")? {
                Message::Ping(payload) => {
                    socket
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|_| "Remote server disconnected.")?;
                }
                Message::Text(text) => {
                    let value: Value =
                        serde_json::from_str(&text).map_err(|_| "Invalid remote response.")?;
                    if value["id"] == command.command_id.to_string() {
                        if let Some(error) = value["error"]["message"].as_str() {
                            return Ok(RemoteResponse::Refused(error.into()));
                        }
                        return Ok(RemoteResponse::Receipt(value["result"].clone()));
                    }
                }
                _ => {}
            }
        }
        Err("Remote server disconnected before its receipt.".into())
    })
    .await
    .map_err(|_| "Remote request timed out.".to_string())?
}

pub fn supports_browse(hello: &Value) -> bool {
    hello["capabilities"]
        .as_array()
        .is_some_and(|capabilities| {
            capabilities
                .iter()
                .any(|capability| capability == protocol::FILESYSTEM_BROWSE)
        })
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
    fn streaming_projections_do_not_cancel_or_repeat_an_inflight_blob() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut entropy=17_u32;
            let image=image::RgbaImage::from_fn(256,256,|_,_| {
                let mut pixel=[0;4];
                for byte in &mut pixel {
                    entropy ^= entropy << 13;
                    entropy ^= entropy >> 17;
                    entropy ^= entropy << 5;
                    *byte=entropy as u8;
                }
                image::Rgba(pixel)
            });
            let mut png=std::io::Cursor::new(Vec::new());
            image.write_to(&mut png,image::ImageFormat::Png).unwrap();
            let bytes=Arc::new(png.into_inner());
            assert!(bytes.len()>protocol::PAGE_BYTES);
            let starts=Arc::new(AtomicUsize::new(0));
            let requests=Arc::new(AtomicUsize::new(0));
            let started=Arc::new(tokio::sync::Notify::new());
            let app=axum::Router::new().fallback(axum::routing::get({
                let bytes=bytes.clone();
                let starts=starts.clone();
                let requests=requests.clone();
                let started=started.clone();
                move |axum::extract::Query(query):axum::extract::Query<std::collections::HashMap<String,usize>>| {
                    let bytes=bytes.clone(); let starts=starts.clone(); let requests=requests.clone(); let started=started.clone();
                    async move {
                        requests.fetch_add(1,Ordering::Relaxed);
                        let offset=query["offset"];
                        if offset==0 {
                            starts.fetch_add(1,Ordering::Relaxed);
                            started.notify_one();
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                        bytes[offset..(offset+protocol::PAGE_BYTES).min(bytes.len())].to_vec()
                    }
                }
            }));
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let host=listener.local_addr().unwrap();
            let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
            let profile=crate::store::directory().unwrap().join("stable-blob-test");
            let remote=crate::peer::Link{server_id:uuid::Uuid::new_v4().to_string(),host:host.to_string(),device_id:uuid::Uuid::new_v4().to_string(),token:"fixture".into()};
            let view=|seq|json!({"seq":seq,"attachment":{"id":"stable-fixture.png","name":"Image","mime":"image/png","bytes":bytes.len(),"path":null}});
            let snapshot=Arc::new(Mutex::new(Some(view(1))));
            let generation=Arc::new(AtomicUsize::new(1));
            let (changed,updates)=tokio::sync::watch::channel(None);
            let worker=tokio::spawn(fetch_assets(remote,profile,snapshot.clone(),generation.clone(),updates,Arc::new(||{}),Arc::new(|_|{})));
            changed.send_replace(Some((1,Arc::new(view(1)))));
            tokio::time::timeout(Duration::from_secs(2),started.notified()).await.unwrap();
            let mut consumed=Value::Null;
            for seq in 2..=30 {
                {
                    let mut snapshot=snapshot.lock().unwrap();
                    generation.store(seq,Ordering::Release);
                    *snapshot=Some(view(seq));
                }
                changed.send_replace(Some((seq,Arc::new(view(seq)))));
                // Match Client::snapshot, which drains the UI's pending value.
                if let Some(view)=snapshot.lock().unwrap().take() {consumed=view;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let deadline=Instant::now()+Duration::from_secs(5);
            loop {
                if consumed["seq"]==30 && consumed["attachment"]["width"]==256
                    || snapshot.lock().unwrap().as_ref().is_some_and(|s|s["seq"]==30 && s["attachment"]["width"]==256) {break}
                assert!(Instant::now()<deadline,"asset did not hydrate the newest projection");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(starts.load(Ordering::Relaxed),1,"projection updates restarted the blob");
            assert_eq!(requests.load(Ordering::Relaxed),bytes.len().div_ceil(protocol::PAGE_BYTES));
            worker.abort();
            server.abort();
        });
    }

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

    #[test]
    fn reconnect_cache_applies_replay_without_losing_history_and_detects_gaps() {
        let mut cache = ProjectionCache::default();
        cache.receive(&protocol::notification("subscription.snapshot",
            json!({"epoch":"epoch","seq":10,"state":{"threads":[{"id":1,"historyRevision":"a","messages":[]}]}}))).unwrap();
        let request = cache.next_history().unwrap();
        assert_eq!(request["params"]["threadId"], 1);
        cache.receive(&json!({"id":"history","result":{"kind":"snapshot","streamEpoch":"stream",
            "data":json!({"threadId":1,"snapshotSequence":10,"historyRevision":"a","items":[{
                "sourceThreadId":1,"sourceItemId":"message-1","field":"messages","ordinal":0,"position":0,"item":{"text":"retained"}
            }],"fields":{"id":1},"nextCursor":null}).to_string(),"nextFragment":null}})).unwrap();
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
    fn replay_ready_refreshes_maintenance_even_without_projection_changes() {
        let mut cache = ProjectionCache::default();
        cache.maintenance = json!({"phase":"draining"});
        cache.seq = Some(42);
        cache
            .receive(&protocol::notification(
                "subscription.ready",
                json!({"seq":42,"maintenance":{"phase":"ready"}}),
            ))
            .unwrap();
        assert_eq!(cache.maintenance["phase"], "ready");
        assert_eq!(cache.seq, Some(42));
    }

    #[test]
    fn native_history_fetch_is_lazy_and_follows_the_local_selection() {
        let mut cache = ProjectionCache::default();
        cache.desired_thread = Some(2);
        cache.state = json!({"index":{"active":1},"threads":[
            {"id":1,"historyRevision":"a"},{"id":2,"historyRevision":"b"}]});
        assert_eq!(cache.next_history().unwrap()["params"]["threadId"], 2);
        cache.desired_thread = Some(1);
        cache
            .receive(&json!({"id":"history","result":{"data":"{}","nextCursor":null}}))
            .unwrap();
        assert_eq!(cache.next_history().unwrap()["params"]["threadId"], 1);
        assert!(cache.histories.is_empty());
    }
}
