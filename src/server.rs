//! Headless server and bounded native-client WebSocket endpoint.
use crate::{
    protocol::{self, Command},
    runtime::{self, Call, RuntimeState},
    server_auth::{self, Discovery},
    server_lock::Owner,
};
use axum::{
    Json, Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone)]
struct Shared {
    discovery: Discovery,
    devices: Arc<Mutex<HashMap<String, String>>>,
    connections: Arc<tokio::sync::Semaphore>,
    calls: mpsc::Sender<Call>,
    snapshots: watch::Receiver<Arc<Value>>,
    client_pids: Arc<Mutex<HashMap<String, Option<u32>>>>,
    maintenance_gate: Arc<AtomicBool>,
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

async fn authenticate(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if headers.contains_key("origin")
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
    let mut devices = shared.devices.lock().expect("devices");
    if devices.len() >= 256 && !devices.values().any(|id| id == device) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    devices.retain(|_, id| id != device);
    devices.insert(token.clone(), device.into());
    Json(json!({"deviceId":device,"token":token})).into_response()
}

async fn upgrade(
    State(shared): State<Shared>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if headers.contains_key("origin") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let device = bearer(&headers)
        .and_then(|token| shared.devices.lock().expect("devices").get(token).cloned());
    let Some(device) = device else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(permit) = shared.connections.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    ws.max_message_size(protocol::MAX_CONTROL)
        .max_frame_size(protocol::MAX_CONTROL)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            connection(socket, shared, device).await
        })
}

async fn send(socket: &mut WebSocket, value: Value) -> bool {
    let text = value.to_string();
    if text.len() > 2 * protocol::MAX_CONTROL {
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

async fn snapshot(socket: &mut WebSocket, value: &Value) -> bool {
    let mut snapshot = value.clone();
    snapshot["subscriptionId"] = json!("workspace");
    send(
        socket,
        protocol::notification("subscription.snapshot", snapshot),
    )
    .await
}

async fn connection(mut socket: WebSocket, shared: Shared, device: String) {
    let connection_id = uuid::Uuid::new_v4().to_string();
    let mut snapshots = shared.snapshots.clone();
    let mut hello = false;
    let mut subscribed = false;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut last_seen = Instant::now();
    let mut last_maintenance = Value::Null;
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break };
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
                } else if method == "hello" {
                    if let Err(error) = crate::maintenance::negotiate(params["protocolMajor"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0), params["protocolMinor"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(u32::MAX), params["projectionSchema"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0)) {
                        hello = false;
                        Err(error)
                    } else if params["maintenanceClient"] != true && shared.maintenance_gate.load(Ordering::Acquire) {
                        Err("Server is draining. Reopen this client after the update completes.".into())
                    } else {
                        hello = true;
                        if params["maintenanceClient"] != true {
                            let pid = params["clientPid"].as_u64().and_then(|n| u32::try_from(n).ok()).filter(|pid| *pid > 0);
                            let mut clients = shared.client_pids.lock().expect("clients");
                            if shared.maintenance_gate.load(Ordering::Acquire) { break; }
                            clients.insert(connection_id.clone(), pid);
                        }
                        Ok(json!({"protocolMajor":protocol::MAJOR,"protocolMinor":params["protocolMinor"].as_u64().unwrap_or(0).min(u64::from(protocol::MINOR)),"projectionSchema":1,
                            "serverId":shared.discovery.server_id,"instanceId":shared.discovery.instance_id,"epoch":shared.discovery.epoch,
                            "binaryVersion":shared.discovery.binary_version,"deviceId":device,"serverTime":crate::lifecycle::now(),
                            "capabilities":["local-commands","snapshot-subscriptions","coordinated-restart"],"limits":{"controlBytes":protocol::MAX_CONTROL,"outstandingCommands":32}}))
                    }
                } else if !hello {
                    Err("Send hello before other requests.".into())
                } else if matches!(method, "workspace.subscribe" | "thread.subscribe") {
                    subscribed = true;
                    let current = snapshots.borrow_and_update().clone();
                    if !snapshot(&mut socket, &current).await { break; }
                    if !send(&mut socket, protocol::notification("subscription.ready", json!({"subscriptionId":"workspace","seq":current["seq"]}))).await { break; }
                    Ok(json!({"subscriptionId":"workspace"}))
                } else if method == "subscription.cancel" {
                    subscribed = false;
                    Ok(json!({}))
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
                if !send(&mut socket, protocol::response(id, result)).await { break; }
            }
            changed = snapshots.changed() => {
                if changed.is_err() { break; }
                let current = snapshots.borrow_and_update().clone();
                let phase = current["maintenance"]["phase"].as_str().unwrap_or("ready");
                let method = match phase { "draining" => "server.draining", "restarting" => "server.restarting", "stopping" => "server.stopping", _ => "server.drainCancelled" };
                if current["maintenance"] != last_maintenance {
                    let notify = phase != "ready" || last_maintenance["phase"].as_str().is_some_and(|phase| phase != "ready");
                    last_maintenance = current["maintenance"].clone();
                    if notify && !send(&mut socket, protocol::notification(method, last_maintenance.clone())).await { break; }
                }
                if subscribed && !snapshot(&mut socket, &current).await { break; }
            }
            _ = heartbeat.tick() => {
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
    if std::env::args().len() != 2 {
        return Err("Usage: shina serve".into());
    }
    let directory = crate::store::directory().ok_or("No Shina data directory.")?;
    let _owner = Owner::acquire(&directory)?;
    server_auth::protect(&directory)?;
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
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
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
        discovery: discovery.clone(),
        devices: Arc::default(),
        connections: Arc::new(tokio::sync::Semaphore::new(32)),
        calls,
        snapshots: receive,
        client_pids: Arc::default(),
        maintenance_gate: maintenance_gate.clone(),
    };
    let app = Router::new()
        .route("/v1/auth", post(authenticate))
        .route("/v1/ws", get(upgrade))
        .with_state(shared);
    let _guard = network.enter();
    let listener = tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;
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
    discovery.publish(&directory)?;
    while !*stop_state.borrow() && !server.is_finished() {
        let was_fenced = runtime.maintenance.fenced();
        let mut changed = false;
        for _ in 0..32 {
            let Ok(call) = incoming.try_recv() else {
                break;
            };
            let result = if call.command.method == "command.get" {
                runtime
                    .state
                    .lookup_command(&call.device, &call.command.params)
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
    Ok(())
}
