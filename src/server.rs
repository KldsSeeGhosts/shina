//! Headless server and bounded native-client WebSocket endpoint.
use crate::{
    protocol::{self, Command},
    runtime::{self, Call, RuntimeState},
    server_auth::{self, Discovery},
    server_lock::Owner,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone)]
struct Shared {
    remote: bool,
    pairing: Arc<Mutex<server_auth::Pairing>>,
    uploads: Arc<Mutex<HashMap<String, Upload>>>,
    discovery: Discovery,
    directory: PathBuf,
    database: crate::database::Database,
    connections: Arc<tokio::sync::Semaphore>,
    calls: mpsc::Sender<Call>,
    snapshots: watch::Receiver<Arc<Value>>,
    client_pids: Arc<Mutex<HashMap<String, Option<u32>>>>,
    maintenance_gate: Arc<AtomicBool>,
    shutdown: watch::Receiver<bool>,
}

struct Upload {
    device: String,
    path: PathBuf,
    length: u64,
    offset: u64,
    hash: String,
    name: String,
    expires: Instant,
}
impl Drop for Upload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn authorized(shared: &Shared, headers: &HeaderMap, method: &str) -> Option<String> {
    if headers.contains_key("origin") {
        return None;
    }
    let device = shared.database.device(bearer(headers)?).ok()??;
    if shared.remote && !shared.database.paired(&device) {
        return None;
    }
    shared
        .database
        .authorize(&device, method, &json!({}))
        .ok()?;
    Some(device)
}

async fn upload_start(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(device) = authorized(&shared, &headers, "uploads.write") else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let length = body["length"]
            .as_u64()
            .filter(|length| *length > 0 && *length <= crate::attachments::MAX_IMAGE_BYTES)
            .ok_or("Invalid image length.")?;
        let hash = body["sha256"]
            .as_str()
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("Invalid hash.")?;
        let mut uploads = shared.uploads.lock().expect("uploads");
        uploads.retain(|_, upload| upload.expires > Instant::now());
        if uploads.len() >= 32
            || uploads
                .values()
                .filter(|upload| upload.device == device)
                .count()
                >= 8
        {
            return Err("Upload limit reached.".into());
        }
        let directory = shared.directory.join("uploads");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = directory.join(&id);
        std::fs::File::create_new(&path).map_err(|e| e.to_string())?;
        uploads.insert(
            id.clone(),
            Upload {
                device,
                path,
                length,
                offset: 0,
                hash: hash.to_ascii_lowercase(),
                name: body["name"]
                    .as_str()
                    .unwrap_or("Image")
                    .chars()
                    .take(120)
                    .collect(),
                expires: Instant::now() + Duration::from_secs(300),
            },
        );
        Ok(json!({"uploadId":id,"offset":0}))
    })
    .await;
    upload_response(result)
}
async fn upload_chunk(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Query(page): Query<BlobPage>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let Some(device) = authorized(&shared, &headers, "uploads.write") else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        use std::io::Write;
        let mut uploads = shared.uploads.lock().expect("uploads");
        let upload = uploads
            .get_mut(&id)
            .filter(|upload| upload.device == device && upload.expires > Instant::now())
            .ok_or("Upload expired.")?;
        if bytes.is_empty()
            || bytes.len() > protocol::MAX_PAGE_BYTES
            || page.offset != upload.offset
            || upload.offset + bytes.len() as u64 > upload.length
        {
            return Err("Invalid upload offset or size.".into());
        }
        std::fs::OpenOptions::new()
            .append(true)
            .open(&upload.path)
            .and_then(|mut file| file.write_all(&bytes))
            .map_err(|e| e.to_string())?;
        upload.offset += bytes.len() as u64;
        Ok(json!({"offset":upload.offset}))
    })
    .await;
    upload_response(result)
}
async fn upload_commit(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(device) = authorized(&shared, &headers, "uploads.write") else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        use sha2::Digest;
        let upload = {
            let mut uploads = shared.uploads.lock().expect("uploads");
            if !uploads
                .get(&id)
                .is_some_and(|upload| upload.device == device && upload.expires > Instant::now())
            {
                return Err("Upload expired.".into());
            }
            uploads.remove(&id).expect("checked")
        };
        if upload.offset != upload.length {
            return Err("Incomplete upload.".into());
        }
        let bytes = std::fs::read(&upload.path).map_err(|e| e.to_string())?;
        if format!("{:x}", sha2::Sha256::digest(&bytes)) != upload.hash {
            return Err("Image hash mismatch.".into());
        }
        shared
            .database
            .authorize(&device, "uploads.write", &json!({}))?;
        let attachment = crate::attachments::store_uploaded(
            &shared.directory.join("attachments"),
            &upload.name,
            &bytes,
        )?;
        Ok(attachment.to_value())
    })
    .await;
    upload_response(result)
}
fn upload_response(result: Result<Result<Value, String>, tokio::task::JoinError>) -> Response {
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

#[derive(serde::Deserialize)]
struct BlobPage {
    #[serde(default)]
    offset: u64,
    #[serde(default = "default_blob_limit")]
    limit: usize,
}

fn default_blob_limit() -> usize {
    protocol::PAGE_BYTES
}

async fn blob(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Query(page): Query<BlobPage>,
    headers: HeaderMap,
) -> Response {
    if authorized(&shared, &headers, "blobs.read").is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !crate::attachments::safe_id(&id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let result = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Seek};
        let directory = shared.directory.join("attachments").canonicalize()?;
        let path = directory.join(id).canonicalize()?;
        if !path.starts_with(&directory) {
            return Err(std::io::Error::other("Invalid blob path."));
        }
        let mut file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        if page.offset > length || length > crate::attachments::MAX_IMAGE_BYTES {
            return Err(std::io::Error::other("Invalid blob offset or length."));
        }
        file.seek(std::io::SeekFrom::Start(page.offset))?;
        let mut bytes = Vec::new();
        file.take(page.limit.clamp(1, protocol::MAX_PAGE_BYTES) as u64)
            .read_to_end(&mut bytes)?;
        Ok((bytes, length))
    })
    .await;
    match result {
        Ok(Ok((bytes, length))) => {
            let next = page.offset + bytes.len() as u64;
            (
                [
                    ("content-type", "application/octet-stream".to_owned()),
                    ("x-blob-length", length.to_string()),
                    ("x-next-offset", next.to_string()),
                    ("cache-control", "no-store".to_owned()),
                ],
                bytes,
            )
                .into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn authenticate(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if shared.remote
        || headers.contains_key("origin")
        || bearer(&headers) != Some(shared.discovery.bootstrap_token.as_str())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(device) = body["deviceId"]
        .as_str()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let token = server_auth::secret();
    if shared.database.issue_device(device, &token).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    Json(json!({"deviceId":device,"token":token})).into_response()
}

async fn pair(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if headers.contains_key("origin") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let result = shared.pairing.lock().expect("pairing").exchange(
        &shared.database,
        body["code"].as_str().unwrap_or(""),
        body["deviceId"].as_str().unwrap_or(""),
        crate::lifecycle::now(),
    );
    match result {
        Ok(token) => {
            Json(json!({"serverId":shared.discovery.server_id,"token":token})).into_response()
        }
        Err(_) => StatusCode::FORBIDDEN.into_response(),
    }
}

async fn upgrade(
    State(shared): State<Shared>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if headers.contains_key("origin") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let token = bearer(&headers).unwrap_or_default().to_owned();
    let device = shared.database.device(&token).ok().flatten();
    let Some(device) = device else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if shared.remote && !shared.database.paired(&device) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(permit) = shared.connections.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    ws.max_message_size(protocol::MAX_CONTROL)
        .max_frame_size(protocol::MAX_CONTROL)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            connection(socket, shared, device, token).await
        })
}

async fn send(socket: &mut WebSocket, value: Value) -> bool {
    let text = value.to_string();
    if text.len() > protocol::MAX_SNAPSHOT_BYTES {
        return false;
    }
    matches!(
        tokio::time::timeout(
            Duration::from_secs(2),
            socket.send(Message::Text(text.into()))
        )
        .await,
        Ok(Ok(()))
    )
}

async fn snapshot(socket: &mut WebSocket, value: &Value, paged: bool) -> bool {
    let mut snapshot = value.clone();
    snapshot["subscriptionId"] = json!("workspace");
    if paged {
        snapshot["state"] = protocol::compact(&snapshot["state"]);
        // Even workspace metadata can exceed one frame. Bound every wire fragment.
        let text = snapshot.to_string();
        if text.len() > protocol::PAGE_BYTES {
            let mut cursor = Value::Null;
            loop {
                let page = protocol::text_page(&text, "workspace", &json!({"cursor":cursor}))
                    .expect("generated cursor");
                cursor = page["nextCursor"].clone();
                if !send(
                    socket,
                    protocol::notification("subscription.snapshotPage", page),
                )
                .await
                {
                    return false;
                }
                if cursor.is_null() {
                    return true;
                }
            }
        }
    }
    send(
        socket,
        protocol::notification("subscription.snapshot", snapshot),
    )
    .await
}

async fn synchronize(
    socket: &mut WebSocket,
    shared: &Shared,
    current: &Value,
    after: Option<u64>,
    paged: bool,
) -> bool {
    if paged
        && let Some(after) = after
        && let Ok(Some(events)) = shared
            .database
            .replay(after, current["seq"].as_u64().unwrap_or(0))
    {
        for event in events {
            if !send(
                socket,
                protocol::notification(
                    "subscription.patch",
                    json!({"epoch":current["epoch"],"subscriptionId":"workspace",
                    "seq":event["seq"],"patch":event["patch"]}),
                ),
            )
            .await
            {
                return false;
            }
        }
        return true;
    }
    snapshot(socket, current, paged).await
}

async fn connection(mut socket: WebSocket, shared: Shared, device: String, token: String) {
    let connection_id = uuid::Uuid::new_v4().to_string();
    let mut snapshots = shared.snapshots.clone();
    let mut hello = false;
    let mut subscribed = false;
    let mut paged = false;
    let mut cursor = None;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut last_seen = Instant::now();
    let mut last_maintenance = Value::Null;
    let mut shutdown = shared.shutdown.clone();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break };
                if shared.database.device(&token).ok().flatten().as_deref() != Some(device.as_str()) { break; }
                last_seen = Instant::now();
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) { break; }
                    continue;
                };
                let request: Value = match serde_json::from_str(&text) {
                    Ok(value) => value,
                    Err(_) => { break; }
                };
                let id = request["id"].clone();
                let method = request["method"].as_str().unwrap_or("");
                let params = &request["params"];
                let mut result = if request["jsonrpc"] != "2.0" || id.is_null() {
                    Err("Expected a JSON-RPC 2.0 request with an id.".into())
                } else if let Err(error) = shared.database.authorize(&device, method,
                    params.get("args").unwrap_or(params)) {
                    Err(error)
                } else if method == "hello" {
                    if let Err(error) = crate::maintenance::negotiate(params["protocolMajor"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0), params["protocolMinor"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0), params["projectionSchema"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0)) {
                        hello = false;
                        Err(error)
                    } else if params["maintenanceClient"] != true && shared.maintenance_gate.load(Ordering::Acquire) {
                        Err("Server is draining. Reopen this client after the update completes.".into())
                    } else {
                        hello = true;
                        if !shared.remote && !shared.database.paired(&device) && params["maintenanceClient"] != true {
                            let pid = params["clientPid"].as_u64().and_then(|n| u32::try_from(n).ok()).filter(|pid| *pid > 0);
                            let mut clients = shared.client_pids.lock().expect("clients");
                            if shared.maintenance_gate.load(Ordering::Acquire) { break; }
                            clients.insert(connection_id.clone(), pid);
                        } else {
                            shared.client_pids.lock().expect("clients").remove(&connection_id);
                        }
                        paged = params["protocolMinor"].as_u64().unwrap_or(0) >= 1;
                        let minor = params["protocolMinor"].as_u64().unwrap_or(0).min(u64::from(protocol::MINOR));
                        let mut capabilities = vec!["local-commands", "snapshot-subscriptions"];
                        if minor >= 1 { capabilities.extend(["projection-patches","history-paging","blob-paging","durable-devices"]); }
                        if minor >= 2 { capabilities.push("coordinated-restart"); }
                        if minor >= 3 { capabilities.extend(["scoped-pairing","multi-server-clients","image-uploads"]); }
                        if minor >= 4 { capabilities.push(protocol::FILESYSTEM_BROWSE); }
                        Ok(json!({"protocolMajor":protocol::MAJOR,"protocolMinor":minor,"projectionSchema":1,
                            "serverId":shared.discovery.server_id,"instanceId":shared.discovery.instance_id,"epoch":shared.discovery.epoch,
                            "binaryVersion":shared.discovery.binary_version,"deviceId":device,"serverTime":crate::lifecycle::now(),
                            "capabilities":capabilities,"limits":{"controlBytes":protocol::MAX_CONTROL,"outstandingCommands":32,"pageBytes":protocol::MAX_PAGE_BYTES}}))
                    }
                } else if !hello {
                    Err("Send hello before other requests.".into())
                } else if matches!(method, "workspace.subscribe" | "thread.subscribe") {
                    subscribed = true;
                    let current = snapshots.borrow_and_update().clone();
                    let after = params["lastSeq"].as_u64().filter(|_| params["epoch"] == current["epoch"]);
                    if !synchronize(&mut socket, &shared, &current, after, paged).await { break; }
                    cursor = current["seq"].as_u64();
                    if !send(&mut socket, protocol::notification("subscription.ready", json!({"subscriptionId":"workspace","seq":current["seq"],"maintenance":current["maintenance"]}))).await { break; }
                    Ok(json!({"subscriptionId":"workspace"}))
                } else if method == "subscription.cancel" {
                    subscribed = false;
                    Ok(json!({}))
                } else if method == "devices.list" {
                    shared.database.devices()
                } else if method == "devices.pair" {
                    if shared.remote { Err("Create pairing codes on the host.".into()) }
                    else { shared.pairing.lock().expect("pairing").create(crate::lifecycle::now()) }
                } else if method == "filesystem.browse" {
                    let args = params.get("args").unwrap_or(params);
                    let path = args["path"].as_str().unwrap_or("").to_string();
                    let partial = args["partial"] == true;
                    let database = shared.database.clone();
                    let device = device.clone();
                    tokio::task::spawn_blocking(move || {
                        database.authorize(&device, "filesystem.browse", &json!({}))?;
                        let listing = crate::projects::browse_folder(&path, partial)?;
                        // Revocation also fences an in-flight listing.
                        database.authorize(&device, "filesystem.browse", &json!({}))?;
                        serde_json::to_value(listing).map_err(|_| "Could not encode the folder listing.".into())
                    }).await.unwrap_or_else(|_| Err("Folder listing failed.".into()))
                } else if method == "thread.history" {
                    let current = snapshots.borrow().clone();
                    current["state"]["threads"].as_array()
                        .and_then(|threads| threads.iter().find(|thread| thread["id"] == params["threadId"]))
                        .ok_or_else(|| "Unknown thread.".to_owned())
                        .and_then(|thread| {
                            let history = protocol::history_payload(thread);
                            let mut page = protocol::text_page(&history.to_string(), &params["threadId"].to_string(), params)?;
                            page["historyRevision"] = json!(protocol::revision(&history));
                            Ok(page)
                        })
                } else {
                    let mut command = if method == "command.get" || method == "server.status" {
                        Command::new(method, params.clone())
                    } else {
                        match serde_json::from_value::<Command>(json!({"commandId":params["commandId"],"method":method,"params":params["args"]})) {
                            Ok(command) => command,
                            Err(_) => {
                                if !send(&mut socket, protocol::response(id, Err("A UUID commandId and args are required.".into()))).await { break; }
                                continue;
                            }
                        }
                    };
                    if method == "server.prepareRestart" {
                        let clients = {
                            let clients = shared.client_pids.lock().expect("clients");
                            if clients.values().any(Option::is_none) { None } else {
                                shared.maintenance_gate.store(true, Ordering::Release);
                                Some(clients.values().flatten().copied().collect::<Vec<_>>())
                            }
                        };
                        let Some(clients) = clients else {
                            if !send(&mut socket, protocol::response(id, Err("An older client cannot coordinate restart. Close older clients, then run shina update --yes on this host and reopen them.".into()))).await { break; }
                            continue;
                        };
                        command.params["clientPids"] = json!(clients);
                    }
                    if method == "server.clientReady" {
                        let registered = shared.client_pids.lock().expect("clients")
                            .get(&connection_id).copied().flatten();
                        if registered.is_none() || command.params["pid"].as_u64() != registered.map(u64::from) {
                            if !send(&mut socket, protocol::response(id, Err("Only a registered local client may acknowledge its own PID.".into()))).await { break; }
                            continue;
                        }
                    }
                    let (reply, response) = oneshot::channel();
                    if shared.calls.try_send(Call { device:device.clone(),command,reply }).is_err() {
                        Err("Runtime command queue is full. Retry the same commandId.".into())
                    } else {
                        match tokio::time::timeout(Duration::from_secs(30), response).await {
                            Ok(Ok(result)) => result,
                            _ => Err("Runtime unavailable. Resolve the receipt before retrying.".into())
                        }
                    }
                };
                if method == "server.prepareRestart" && result.is_err()
                    && shared.snapshots.borrow()["maintenance"]["phase"] == "ready" {
                    shared.maintenance_gate.store(false, Ordering::Release);
                }
                if method == "server.prepareRestart" && let Ok(value) = &mut result {
                    value["serverPid"] = json!(shared.discovery.pid);
                    value["desktopOwned"] = json!(std::env::var("SHINA_DESKTOP_OWNER_PID").is_ok());
                }
                if method == "server.status" && let Ok(value) = &mut result {
                    value["serverId"] = json!(shared.discovery.server_id);
                    value["instanceId"] = json!(shared.discovery.instance_id);
                }
                if !send(&mut socket, protocol::response(id, result)).await { break; }
            }
            changed = snapshots.changed() => {
                if changed.is_err() { break; }
                if shared.database.device(&token).ok().flatten().as_deref() != Some(device.as_str()) { break; }
                let current = snapshots.borrow_and_update().clone();
                let phase = current["maintenance"]["phase"].as_str().unwrap_or("ready");
                let method = match phase { "draining" => "server.draining", "restarting" => "server.restarting", "stopping" => "server.stopping", _ => "server.drainCancelled" };
                if current["maintenance"] != last_maintenance {
                    let notify = phase != "ready" || last_maintenance["phase"].as_str().is_some_and(|phase| phase != "ready");
                    last_maintenance = current["maintenance"].clone();
                    if notify && !send(&mut socket, protocol::notification(method, last_maintenance.clone())).await { break; }
                }
                if subscribed {
                    if !synchronize(&mut socket, &shared, &current, cursor, paged).await { break; }
                    cursor = current["seq"].as_u64();
                }
            }
            _ = heartbeat.tick() => {
                shared.uploads.lock().expect("uploads").retain(|_,upload| upload.expires > Instant::now());
                if shared.database.device(&token).ok().flatten().as_deref() != Some(device.as_str()) { break; }
                if last_seen.elapsed() > Duration::from_secs(45) { break; }
                if !matches!(tokio::time::timeout(Duration::from_secs(2),socket.send(Message::Ping(Vec::new().into()))).await,Ok(Ok(()))) { break; }
            }
        }
    }
    shared
        .client_pids
        .lock()
        .expect("clients")
        .remove(&connection_id);
}

pub fn serve() -> Result<(), String> {
    let mut tailnet = false;
    let mut port = None;
    let args: Vec<_> = std::env::args().skip(2).collect();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tailnet" => tailnet = true,
            "--port" => {
                port = Some(
                    args.next()
                        .ok_or("--port requires a number.")?
                        .parse::<u16>()
                        .map_err(|_| "Invalid port.")?,
                )
            }
            _ => return Err("Usage: shina serve [--tailnet] [--port <port>]".into()),
        }
    }
    if tailnet && port == Some(0) {
        return Err("A tailnet listener requires a non-zero port.".into());
    }
    let directory = crate::store::directory().ok_or("No Shina data directory.")?;
    serve_profile(directory, tailnet, port)
}

fn serve_profile(directory: PathBuf, tailnet: bool, port: Option<u16>) -> Result<(), String> {
    // Resolve every listener before starting any remote accept loop.
    let addresses = if tailnet {
        crate::peer::bind_addresses(crate::peer::tailscale_addresses)?
    } else {
        vec![]
    };
    let _owner = Owner::acquire(&directory)?;
    server_auth::protect(&directory)?;
    // Orphans from a previous process are never resumable with its lost staging ids.
    let staging = directory.join("uploads");
    if staging.exists() {
        for entry in std::fs::read_dir(&staging).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_ok()
                && entry.file_type().map_err(|e| e.to_string())?.is_file()
            {
                std::fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
    }
    let version_path = directory.join("server-binary-version.json");
    if std::fs::read_to_string(&version_path).ok().as_deref() != Some(env!("CARGO_PKG_VERSION"))
        && std::fs::read_dir(&directory)
            .map_err(|e| e.to_string())?
            .flatten()
            .any(|entry| {
                let path = entry.path();
                path.file_name()
                    .is_some_and(|name| name != "server-owner.sqlite")
                    && path.extension().is_some_and(|ext| {
                        ext == "sqlite" || ext == "sqlite3" || ext == "db" || ext == "json"
                    })
            })
    {
        // Before RuntimeState::open, including slice 2's database migration.
        crate::maintenance::backup_state(&directory)?;
    }
    let identity_path = directory.join("server-id");
    let server_id = match std::fs::read_to_string(&identity_path) {
        Ok(id) => uuid::Uuid::parse_str(id.trim())
            .map_err(|e| e.to_string())?
            .to_string(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let id = uuid::Uuid::new_v4().to_string();
            crate::store::write_atomic(&identity_path, id.as_bytes()).map_err(|e| e.to_string())?;
            id
        }
        Err(e) => return Err(e.to_string()),
    };
    let (wake_send, wake_receive) = std::sync::mpsc::sync_channel(1);
    let wake: rune::Waker = Arc::new(move || {
        let _ = wake_send.try_send(());
    });
    let mut runtime = RuntimeState::open(directory.clone(), wake)?;
    crate::store::write_atomic(&version_path, env!("CARGO_PKG_VERSION").as_bytes())
        .map_err(|e| e.to_string())?;
    let (calls, mut incoming) = mpsc::channel::<Call>(32);
    let (snapshots, receive) = watch::channel(Arc::new(runtime.snapshot()));
    let (stop, mut stopping) = watch::channel(false);
    let stop_state = stopping.clone();
    let network = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", port.unwrap_or(0))).map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let discovery = Discovery {
        server_id,
        instance_id: uuid::Uuid::new_v4().to_string(),
        epoch: runtime.state.server_epoch().into(),
        pid: std::process::id(),
        started_at: crate::lifecycle::now(),
        port: listener.local_addr().map_err(|e| e.to_string())?.port(),
        protocol_major: protocol::MAJOR,
        protocol_minor: protocol::MINOR,
        binary_version: env!("CARGO_PKG_VERSION").into(),
        bootstrap_token: server_auth::secret(),
    };
    let maintenance_gate = Arc::new(AtomicBool::new(false));
    let shared = Shared {
        remote: false,
        pairing: Arc::default(),
        uploads: Arc::default(),
        discovery: discovery.clone(),
        directory: directory.clone(),
        database: runtime.database.clone().expect("durable runtime"),
        connections: Arc::new(tokio::sync::Semaphore::new(32)),
        calls,
        snapshots: receive,
        client_pids: Arc::default(),
        maintenance_gate: maintenance_gate.clone(),
        shutdown: stopping.clone(),
    };
    let mut remote = shared.clone();
    remote.remote = true;
    let app = router(shared);
    let _guard = network.enter();
    let uploads = remote.uploads.clone();
    let cleanup = network.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            uploads
                .lock()
                .expect("uploads")
                .retain(|_, upload| upload.expires > Instant::now());
        }
    });
    let listener = tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;
    let mut remote_servers = Vec::new();
    for address in addresses {
        let listener = std::net::TcpListener::bind((address, port.unwrap_or(crate::peer::PORT)))
            .map_err(|_| "Could not bind the Tailscale address. No wildcard fallback was used.")?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let listener = tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;
        remote_servers
            .push(network.spawn(axum::serve(listener, router(remote.clone())).into_future()));
    }
    let server = network.spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                while !*stopping.borrow_and_update() {
                    if stopping.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await
    });
    crate::store::write_atomic(
        &directory.join("server-listen.json"),
        json!({"tailnet":tailnet,"port":port})
            .to_string()
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    discovery.publish(&directory)?;
    while !*stop_state.borrow() && !server.is_finished() {
        let was_fenced = runtime.maintenance.fenced();
        let mut changed = false;
        for _ in 0..32 {
            let Ok(call) = incoming.try_recv() else {
                break;
            };
            let result = if !runtime
                .database
                .as_ref()
                .expect("durable runtime")
                .device_active(&call.device)?
            {
                Err("Authentication denied. This device was revoked.".into())
            } else if call.command.method == "command.get" {
                runtime.lookup_command(&call.device, &call.command.params)
            } else {
                runtime.command(&call.device, call.command)
            };
            let _ = call.reply.send(result);
            changed = true;
            if runtime.state.runtime_stop_requested {
                stop.send_replace(true);
                break;
            }
        }
        if !runtime.state.runtime_stop_requested {
            changed |= runtime.tick();
        }
        if runtime.state.runtime_stop_requested {
            stop.send_replace(true);
        }
        if runtime.maintenance.fenced() {
            maintenance_gate.store(true, Ordering::Release);
        } else if was_fenced {
            maintenance_gate.store(false, Ordering::Release);
        }
        if let Some(error) = runtime.failure() {
            return Err(error);
        }
        if changed {
            snapshots.send_replace(Arc::new(runtime.snapshot()));
        }
        let _ = wake_receive.recv_timeout(runtime::TICK);
    }
    runtime.shutdown()?;
    let maintenance = runtime.maintenance.status();
    if maintenance["phase"] == "restarting"
        && let Some(id) = maintenance["updateId"].as_str()
    {
        crate::store::write_atomic(&directory.join("restart-flushed"), id.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    drop(runtime);
    crate::backend::shutdown_all();
    if Discovery::read(&directory).is_ok_and(|current| current.instance_id == discovery.instance_id)
    {
        let _ = std::fs::remove_file(directory.join("server.json"));
    }
    // Let the final maintenance response/notification leave its network task.
    std::thread::sleep(Duration::from_millis(150));
    server.abort();
    for server in remote_servers {
        server.abort();
    }
    cleanup.abort();
    Ok(())
}

fn router(shared: Shared) -> Router {
    Router::new()
        .route("/v1/auth", post(authenticate))
        .route("/v1/pair", post(pair))
        .route("/v1/ws", get(upgrade))
        .route("/v1/blobs/{id}", get(blob))
        .route("/v1/uploads", post(upload_start))
        .route("/v1/uploads/{id}", axum::routing::put(upload_chunk))
        .route("/v1/uploads/{id}/commit", post(upload_commit))
        .layer(axum::extract::DefaultBodyLimit::max(protocol::MAX_CONTROL))
        .with_state(shared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message as ClientMessage, client::IntoClientRequest};

    struct Fixture {
        shared: Shared,
        address: std::net::SocketAddr,
        listener: tokio::task::JoinHandle<()>,
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
        disconnect: watch::Sender<bool>,
    }
    impl Fixture {
        async fn new(name: &str) -> Self {
            let directory = crate::store::directory().unwrap().join(name);
            std::fs::create_dir_all(&directory).unwrap();
            let database = crate::database::Database::open(&directory).unwrap();
            let mut state = crate::model::State::headless(directory.clone());
            state.load_global_for_test();
            let mut runtime = RuntimeState::from_state(state);
            runtime.database = Some(database.clone());
            let (calls, mut incoming) = mpsc::channel::<Call>(32);
            let (snapshots, receive) = watch::channel(Arc::new(runtime.snapshot()));
            let (disconnect, shutdown) = watch::channel(false);
            let stop = Arc::new(AtomicBool::new(false));
            let stopping = stop.clone();
            let profile = directory.clone();
            let worker = std::thread::spawn(move || {
                crate::store::with_directory(profile, || {
                    while !stopping.load(Ordering::Acquire) {
                        if let Ok(call) = incoming.try_recv() {
                            let result = if call.command.method == "command.get" {
                                runtime.lookup_command(&call.device, &call.command.params)
                            } else {
                                runtime.command(&call.device, call.command)
                            };
                            let _ = call.reply.send(result);
                            snapshots.send_replace(Arc::new(runtime.snapshot()));
                        } else {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                    runtime.shutdown().unwrap();
                })
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let shared = Shared {
                // Exercise remote authorization over an isolated loopback fixture.
                remote: true,
                pairing: Arc::default(),
                uploads: Arc::default(),
                discovery: Discovery {
                    server_id: uuid::Uuid::new_v4().to_string(),
                    instance_id: uuid::Uuid::new_v4().to_string(),
                    epoch: uuid::Uuid::new_v4().to_string(),
                    pid: std::process::id(),
                    started_at: 0,
                    port: address.port(),
                    protocol_major: protocol::MAJOR,
                    protocol_minor: protocol::MINOR,
                    binary_version: env!("CARGO_PKG_VERSION").into(),
                    bootstrap_token: server_auth::secret(),
                },
                directory,
                database,
                connections: Arc::new(tokio::sync::Semaphore::new(32)),
                calls,
                snapshots: receive,
                client_pids: Arc::default(),
                maintenance_gate: Arc::default(),
                shutdown,
            };
            let app = router(shared.clone());
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            Self {
                shared,
                address,
                listener: task,
                stop,
                worker: Some(worker),
                disconnect,
            }
        }
        fn code(&self) -> String {
            self.shared
                .pairing
                .lock()
                .unwrap()
                .create(crate::lifecycle::now())
                .unwrap()["code"]
                .as_str()
                .unwrap()
                .into()
        }
        async fn link(&self, profile: &std::path::Path) -> crate::peer::Link {
            crate::peer::link(profile, &self.address.to_string(), &self.code())
                .await
                .unwrap()
        }
        async fn listen(&mut self) {
            self.disconnect.send_replace(false);
            let listener = tokio::net::TcpListener::bind(self.address).await.unwrap();
            let app = router(self.shared.clone());
            self.listener = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.listener.abort();
            self.disconnect.send_replace(true);
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    async fn subscription(
        link: &crate::peer::Link,
        seq: Option<u64>,
        epoch: Value,
    ) -> crate::client::Socket {
        let mut request = format!("ws://{}/v1/ws", link.address().unwrap())
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", link.token).parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket.send(ClientMessage::Text(json!({"jsonrpc":"2.0","id":1,"method":"hello","params":{"protocolMajor":1,"protocolMinor":protocol::MINOR,"projectionSchema":1}}).to_string().into())).await.unwrap();
        let hello = next_json(&mut socket).await;
        assert!(hello["error"].is_null(), "{hello}");
        socket.send(ClientMessage::Text(json!({"jsonrpc":"2.0","id":2,"method":"workspace.subscribe","params":{"lastSeq":seq,"epoch":epoch}}).to_string().into())).await.unwrap();
        socket
    }
    async fn next_json(socket: &mut crate::client::Socket) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match socket.next().await.unwrap().unwrap() {
                    ClientMessage::Text(text) => return serde_json::from_str(&text).unwrap(),
                    ClientMessage::Ping(data) => {
                        socket.send(ClientMessage::Pong(data)).await.unwrap()
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap()
    }

    #[test]
    fn filesystem_browse_grants_bounds_home_errors_creation_and_minor_fallback() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let fixture = Fixture::new("browse").await;
            let profile = fixture.shared.directory.join("client");
            let link = fixture.link(&profile).await;
            let folder = fixture.shared.directory.join("folders");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("secret-file"), "never listed").unwrap();
            for i in 0..=protocol::MAX_BROWSE_ENTRIES {
                std::fs::create_dir(folder.join(format!("folder-{i:04}"))).unwrap();
            }
            let listing = crate::client::remote_rpc(&link, "filesystem.browse", json!({"path":folder})).await.unwrap();
            assert_eq!(listing["entries"].as_array().unwrap().len(), protocol::MAX_BROWSE_ENTRIES);
            assert_eq!(listing["truncated"], true);
            assert_eq!(listing["path"], json!(folder.canonicalize().unwrap()));
            assert_eq!(listing["parent"], json!(folder.canonicalize().unwrap().parent().unwrap()));
            assert!(!listing.to_string().contains("secret-file"));
            for path in ["", "~"] {
                let listing = crate::client::remote_rpc(&link, "filesystem.browse", json!({"path":path})).await.unwrap();
                assert_eq!(listing["path"], json!(crate::projects::home_dir().unwrap().canonicalize().unwrap()));
            }
            let missing = folder.join("new/child");
            assert!(crate::client::remote_rpc(&link, "filesystem.browse", json!({"path":missing})).await.unwrap_err().contains("does not exist"));
            let created = crate::client::remote_rpc(&link, "project.put", json!({"path":missing,"createFolder":true})).await.unwrap();
            assert_eq!(created["status"], "accepted", "{created}");
            assert_eq!(created["result"]["projectId"], 1);
            assert!(created["result"]["threadId"].is_null());
            assert!(missing.is_dir());
            assert!(crate::client::remote_rpc(&link, "filesystem.browse", json!({"path":folder.join("secret-file")})).await.unwrap_err().contains("could not be read"));
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                let denied = folder.join("denied");
                std::fs::create_dir(&denied).unwrap();
                std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0)).unwrap();
                let result = crate::client::remote_rpc(&link, "filesystem.browse", json!({"path":denied})).await;
                std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o700)).unwrap();
                assert_eq!(result.unwrap_err(), "That folder could not be read.");
            }
            fixture.shared.database.issue_scoped_device("operate-only", "operate-secret", &["paired","operate"]).unwrap();
            // A device without read cannot even complete hello or request a listing.
            assert!(fixture.shared.database.authorize("operate-only", "filesystem.browse", &json!({})).is_err());
            fixture.shared.database.issue_scoped_device("reader", "reader-secret", &["paired","read"]).unwrap();
            let mut reader = link.clone();
            reader.device_id = "reader".into();
            reader.token = "reader-secret".into();
            assert!(crate::client::remote_rpc(&reader, "filesystem.browse", json!({"path":folder})).await.is_ok());
            let rejected = crate::client::remote_rpc(&reader, "project.put", json!({"path":folder.join("forbidden"),"createFolder":true})).await.unwrap_err();
            assert!(rejected.contains("operate"));
            assert!(!folder.join("forbidden").exists());
            fixture.shared.database.revoke("reader").unwrap();
            assert!(crate::client::remote_rpc(&reader, "filesystem.browse", json!({"path":folder})).await.is_err());

            let mut request = format!("ws://{}/v1/ws", link.address().unwrap()).into_client_request().unwrap();
            request.headers_mut().insert("authorization", format!("Bearer {}", link.token).parse().unwrap());
            let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
            socket.send(ClientMessage::Text(json!({"jsonrpc":"2.0","id":1,"method":"hello","params":{"protocolMajor":1,"protocolMinor":3,"projectionSchema":1}}).to_string().into())).await.unwrap();
            let hello = next_json(&mut socket).await;
            assert_eq!(hello["result"]["protocolMinor"], 3);
            assert!(!crate::client::supports_browse(&hello["result"]));
            assert!(crate::client::supports_browse(&json!({"capabilities":[protocol::FILESYSTEM_BROWSE]})));
        });
    }

    #[test]
    fn three_machine_pairing_routing_path_isolation_replay_and_revocation() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let a = Fixture::new("a").await;
            let mut b = Fixture::new("b").await;
            let c = Fixture::new("c").await;
            let profile = crate::store::directory().unwrap().join("client");
            let link_b = b.link(&profile).await;
            let link_c = c.link(&profile).await;
            assert_eq!(crate::peer::Link::load_all(&profile).unwrap().len(), 2);
            let project_b = b.shared.directory.join("workspace");
            let project_c = c.shared.directory.join("workspace");
            std::fs::create_dir_all(&project_b).unwrap();
            std::fs::create_dir_all(&project_c).unwrap();
            for (link, path) in [(&link_b, &project_b), (&link_c, &project_c)] {
                let receipt = crate::client::remote_rpc(link, "project.put", json!({"path":path}))
                    .await
                    .unwrap();
                assert_eq!(receipt["status"], "accepted", "{receipt}");
            }
            let create = crate::client::remote_rpc(
                &link_b,
                "thread.create",
                json!({"project":1,"occupiedDrafts":[1,2]}),
            )
            .await
            .unwrap();
            assert_eq!(create["status"], "accepted", "{create}");
            let thread = create["result"]["threadId"].as_u64().unwrap();
            let stop =
                crate::client::remote_rpc(&link_b, "thread.stop", json!({"threadId":thread}))
                    .await
                    .unwrap();
            assert_eq!(stop["status"], "accepted", "{stop}");
            assert_eq!(a.shared.snapshots.borrow()["seq"], 0);
            let state = b.shared.snapshots.borrow().clone();
            assert_eq!(
                state["state"]["index"]["projects"][0]["path"],
                json!(crate::projects::normalize_path(&project_b))
            );
            assert_eq!(
                c.shared.snapshots.borrow()["state"]["index"]["projects"][0]["path"],
                json!(crate::projects::normalize_path(&project_c))
            );
            let mut socket = subscription(&link_b, None, Value::Null).await;
            let snapshot = next_json(&mut socket).await;
            assert_eq!(snapshot["method"], "subscription.snapshot");
            assert!(
                snapshot["params"]["state"]["index"]["threads"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["id"] == thread)
            );
            let seq = snapshot["params"]["seq"].as_u64();
            let epoch = snapshot["params"]["epoch"].clone();
            drop(socket);
            b.listener.abort();
            b.disconnect.send_replace(true);
            let _ = (&mut b.listener).await;
            assert!(
                crate::client::remote_rpc(&link_b, "thread.stop", json!({"threadId":thread}))
                    .await
                    .is_err()
            );
            // The server continues to own its reducer while this listener sleeps.
            let (reply, response) = oneshot::channel();
            b.shared
                .calls
                .send(Call {
                    device: link_b.device_id.clone(),
                    command: Command::new(
                        "thread.rename",
                        json!({"threadId":thread,"title":"Changed while offline"}),
                    ),
                    reply,
                })
                .await
                .unwrap();
            assert_eq!(response.await.unwrap().unwrap()["status"], "accepted");
            b.listen().await;
            let mut socket = subscription(&link_b, seq, epoch).await;
            assert_eq!(next_json(&mut socket).await["method"], "subscription.patch");
            drop(socket);
            assert!(
                crate::client::remote_rpc(&link_b, "server.stop", json!({}))
                    .await
                    .unwrap_err()
                    .contains("maintain")
            );
            assert!(
                crate::client::remote_rpc(&link_b, "devices.pair", json!({}))
                    .await
                    .is_err()
            );
            crate::peer::unlink(&profile, &link_b.server_id)
                .await
                .unwrap();
            assert!(
                crate::client::remote_rpc(&link_b, "thread.stop", json!({"threadId":thread}))
                    .await
                    .unwrap_err()
                    .contains("Authentication denied")
            );
            c.shared.database.revoke(&link_c.device_id).unwrap();
            assert!(
                crate::client::remote_rpc(&link_c, "thread.create", json!({}))
                    .await
                    .is_err()
            );
            assert_eq!(crate::peer::Link::load_all(&profile).unwrap().len(), 1);
        });
    }

    #[test]
    fn remote_listener_refuses_bootstrap_and_unpaired_devices() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let fixture = Fixture::new("remote").await;
            let response = crate::peer::http()
                .unwrap()
                .post(format!("http://{}/v1/auth", fixture.address))
                .bearer_auth(&fixture.shared.discovery.bootstrap_token)
                .json(&json!({"deviceId":uuid::Uuid::new_v4()}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            fixture
                .shared
                .database
                .issue_device("local-device", "local-token")
                .unwrap();
            for token in [
                "local-token",
                fixture.shared.discovery.bootstrap_token.as_str(),
            ] {
                let mut request = format!("ws://{}/v1/ws", fixture.address)
                    .into_client_request()
                    .unwrap();
                request
                    .headers_mut()
                    .insert("authorization", format!("Bearer {token}").parse().unwrap());
                assert!(tokio_tungstenite::connect_async(request).await.is_err());
            }
        });
    }

    #[test]
    fn one_time_pairing_expiry_rate_limit_and_grants() {
        let directory = crate::store::directory().unwrap();
        let database = crate::database::Database::open(&directory).unwrap();
        let local = uuid::Uuid::new_v4().to_string();
        database.issue_device(&local, "local-secret").unwrap();
        assert!(
            database
                .issue_scoped_device(&local, "replacement", &["paired", "read"])
                .is_err()
        );
        assert_eq!(database.device("local-secret").unwrap(), Some(local));
        let mut pairing = server_auth::Pairing::default();
        let now = 1_000_000;
        let code = pairing.create(now).unwrap()["code"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(pairing.create(now + 1).is_err());
        assert!(
            pairing
                .exchange(
                    &database,
                    &code,
                    &uuid::Uuid::new_v4().to_string(),
                    now + 300_000
                )
                .is_err()
        );
        let code = pairing.create(now + 300_001).unwrap()["code"]
            .as_str()
            .unwrap()
            .to_owned();
        for _ in 0..5 {
            assert!(
                pairing
                    .exchange(
                        &database,
                        "invalid",
                        &uuid::Uuid::new_v4().to_string(),
                        now + 300_002
                    )
                    .is_err()
            );
        }
        assert!(
            pairing
                .exchange(
                    &database,
                    &code,
                    &uuid::Uuid::new_v4().to_string(),
                    now + 300_003
                )
                .is_err()
        );
        let device = uuid::Uuid::new_v4().to_string();
        let token = pairing
            .exchange(&database, &code, &device, now + 360_003)
            .unwrap();
        assert_eq!(database.device(&token).unwrap(), Some(device.clone()));
        assert!(
            pairing
                .exchange(
                    &database,
                    &code,
                    &uuid::Uuid::new_v4().to_string(),
                    now + 360_004
                )
                .is_err()
        );
        for method in ["thread.create", "thread.stop", "request.respond"] {
            database.authorize(&device, method, &json!({})).unwrap();
        }
        for method in ["devices.pair", "server.stop", "peer.link"] {
            assert!(database.authorize(&device, method, &json!({})).is_err());
        }
        database
            .authorize(&device, "devices.revoke", &json!({"deviceId":device}))
            .unwrap();
        database.revoke(&device).unwrap();
        assert!(database.device(&token).unwrap().is_none());
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(8), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("fixture client did not reach its expected state");
    }

    #[test]
    fn independent_clients_reconnect_one_sleeping_server_without_blocking_another() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let mut b=Fixture::new("sleep-b").await;
            let c=Fixture::new("awake-c").await;
            let profile=crate::store::directory().unwrap().join("desktop");
            let link_b=b.link(&profile).await;
            let link_c=c.link(&profile).await;
            let mut desktop=crate::client::Client::new(profile.clone());
            desktop.add_link(link_b.clone(),Arc::new(|| {}));
            desktop.add_link(link_c.clone(),Arc::new(|| {}));
            wait_until(||desktop.commands_for(&link_b.server_id).unwrap().connected()
                && desktop.commands_for(&link_c.server_id).unwrap().connected()).await;
            let command=desktop.commands_for(&link_b.server_id).unwrap().send("thread.rename",json!({"threadId":1,"title":"Before sleep"})).unwrap();
            wait_until(|| {
                desktop.machine_event().is_some_and(|(server,event)| server==link_b.server_id
                    && matches!(event,crate::client::Event::Receipt(ref sent,_) if sent.command_id==command))
            }).await;
            b.disconnect.send_replace(true);
            b.listener.abort();
            let _=(&mut b.listener).await;
            wait_until(||!desktop.commands_for(&link_b.server_id).unwrap().connected()).await;
            assert!(desktop.commands_for(&link_b.server_id).unwrap().send("request.respond",json!({})).is_err());
            assert!(desktop.commands_for(&link_c.server_id).unwrap().connected());
            let command=desktop.commands_for(&link_c.server_id).unwrap().send("thread.rename",json!({"threadId":1,"title":"While B sleeps"})).unwrap();
            wait_until(||desktop.machine_event().is_some_and(|(server,event)| server==link_c.server_id
                && matches!(event,crate::client::Event::Receipt(ref sent,ref receipt) if sent.command_id==command && receipt["result"]["status"]=="accepted"))).await;
            assert_eq!(c.shared.snapshots.borrow()["state"]["index"]["threads"][0]["title"],"While B sleeps");
            assert_eq!(b.shared.snapshots.borrow()["state"]["index"]["threads"][0]["title"],"Before sleep");
            b.listen().await;
            wait_until(||desktop.commands_for(&link_b.server_id).unwrap().connected()).await;
            assert!(profile.join("linked-views").join(&link_b.server_id).join("view.json").is_file());
            // Shutdown of a linked client never stops either server.
            desktop.shutdown();
            assert!(!b.stop.load(Ordering::Acquire));
            assert!(!c.stop.load(Ordering::Acquire));
        });
    }

    #[test]
    fn authenticated_image_upload_checks_offsets_hashes_content_and_revocation() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            use sha2::Digest;
            let fixture=Fixture::new("uploads").await;
            let profile=crate::store::directory().unwrap().join("image-client");
            let link=fixture.link(&profile).await;
            let http=crate::peer::http().unwrap();
            let base=format!("http://{}",fixture.address);
            let mut encoded=std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2,2).write_to(&mut encoded,image::ImageFormat::Png).unwrap();
            let bytes=encoded.into_inner();
            let body=json!({"name":"image.png","length":bytes.len(),"sha256":format!("{:x}",sha2::Sha256::digest(&bytes))});
            let value:Value=http.post(format!("{base}/v1/uploads")).bearer_auth(&link.token).json(&body)
                .send().await.unwrap().json().await.unwrap();
            let id=value["uploadId"].as_str().unwrap();
            let url=format!("{base}/v1/uploads/{id}");
            assert!(!http.put(format!("{url}?offset=1")).bearer_auth(&link.token).body(bytes.clone()).send().await.unwrap().status().is_success());
            assert!(http.put(format!("{url}?offset=0")).bearer_auth(&link.token).body(bytes.clone()).send().await.unwrap().status().is_success());
            let response=http.post(format!("{url}/commit")).bearer_auth(&link.token).send().await.unwrap();
            assert!(response.status().is_success());
            let attachment:Value=response.json().await.unwrap();
            let id=attachment["id"].as_str().unwrap();
            assert_eq!(std::fs::read(fixture.shared.directory.join("attachments").join(id)).unwrap(),bytes);
            let downloaded=http.get(format!("{base}/v1/blobs/{id}")).bearer_auth(&link.token).send().await.unwrap();
            assert_eq!(downloaded.bytes().await.unwrap().as_ref(),bytes);
            let bad:Value=http.post(format!("{base}/v1/uploads")).bearer_auth(&link.token)
                .json(&json!({"length":4,"sha256":"0".repeat(64)})).send().await.unwrap().json().await.unwrap();
            let url=format!("{base}/v1/uploads/{}",bad["uploadId"].as_str().unwrap());
            http.put(format!("{url}?offset=0")).bearer_auth(&link.token).body("nope").send().await.unwrap();
            assert!(!http.post(format!("{url}/commit")).bearer_auth(&link.token).send().await.unwrap().status().is_success());
            fixture.shared.database.revoke(&link.device_id).unwrap();
            assert_eq!(http.post(format!("{base}/v1/uploads")).bearer_auth(&link.token).json(&body).send().await.unwrap().status(),StatusCode::UNAUTHORIZED);
        });
    }
}
