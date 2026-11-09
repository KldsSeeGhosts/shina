//! The Shina MCP: a local Streamable HTTP MCP endpoint that lets agent harnesses use Shina's
//! own tools (threads, captains, delegation). Each connected thread gets its own bearer token,
//! which is how the server knows which thread is calling. Tool calls run on the UI thread
//! through `Server::requests`, so they see and change the same state the window shows.
use rune::Waker;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    hash::{BuildHasher, Hasher},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

pub const PROTOCOL: &str = "2025-06-18";
pub const URL_ENV: &str = "SHINA_MCP_URL";
pub const TOKEN_ENV: &str = "SHINA_MCP_TOKEN";

/// What every harness is told about Shina. The Pi bridge adds it to the system prompt; other
/// MCP clients receive it as the server's `instructions`.
pub const INSTRUCTIONS: &str = "## Shina

You are running inside Shina, a native desktop workspace where the user chats with coding agents. \
Each conversation is a Shina thread. Threads can belong to a captain, a named crew member with its \
own role and instructions. Shina shows the user every thread, its status, and its transcript.

Ordinary coding tasks need no Shina tools.

- `mcp__shina__delegate_task` starts independent work in a new Shina thread, under the same \
captain unless you name another. Use it for parallel, self-contained work, and give the new \
thread a complete brief: it has no access to this conversation. A delegated thread cannot \
delegate further.
- `mcp__shina__task_status` reports a delegated thread's status and latest reply. It returns \
immediately; check again later instead of calling it in a tight loop.
- `mcp__shina__list_threads`, `mcp__shina__read_thread`, and `mcp__shina__list_captains` read \
the user's other conversations and crew. Read a thread before you answer a question about it.";

const EXTENSION: &str = include_str!("../assets/pi/shina-mcp-extension.ts");

/// Writes the Pi bridge extension under `%LOCALAPPDATA%\Shina\pi` and returns its path.
/// The file is rewritten only when this build's copy differs.
pub fn extension_path() -> Result<std::path::PathBuf, String> {
    let base = std::env::var_os(if cfg!(windows) {
        "LOCALAPPDATA"
    } else {
        "HOME"
    })
    .map(std::path::PathBuf::from)
    .ok_or("Could not find a folder for Shina's Pi extension.")?;
    let dir = base.join("Shina").join("pi");
    let path = dir.join("shina-mcp-extension.ts");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(EXTENSION) {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(&path, EXTENSION)
            .map_err(|e| format!("Could not write Shina's Pi extension: {e}"))?;
    }
    Ok(path)
}

/// One tool call waiting for the UI thread.
pub struct Request {
    pub thread: usize,
    pub tool: String,
    pub args: Value,
    pub reply: Sender<Result<Value, String>>,
}

pub struct Server {
    pub url: String,
    tokens: Mutex<HashMap<String, usize>>,
    requests: Mutex<Receiver<Request>>,
    wake: Mutex<Waker>,
}

static SERVER: OnceLock<Result<Arc<Server>, String>> = OnceLock::new();

/// The process-wide server, started on first use. Workspaces share it; `reset` clears the
/// previous workspace's tokens.
pub fn server(wake: Waker) -> Result<Arc<Server>, String> {
    let server = SERVER
        .get_or_init(|| start().map_err(|e| format!("Could not start the Shina MCP: {e}")))
        .clone()?;
    *server.wake.lock().expect("mcp waker") = wake;
    Ok(server)
}

fn random_hex(words: usize) -> String {
    // RandomState is seeded from the OS, so hashing a counter with fresh states gives
    // unpredictable bits without another dependency. This guards a localhost port only.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    (0..words)
        .map(|_| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            hasher.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default(),
            );
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

fn start() -> std::io::Result<Arc<Server>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let (sender, receiver) = mpsc::channel();
    let server = Arc::new(Server {
        url: format!("http://127.0.0.1:{port}/mcp"),
        tokens: Mutex::new(HashMap::new()),
        requests: Mutex::new(receiver),
        wake: Mutex::new(Arc::new(|| {})),
    });
    let shared = server.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let server = shared.clone();
            let sender = sender.clone();
            thread::spawn(move || {
                let _ = serve(stream, &server, &sender);
            });
        }
    });
    Ok(server)
}

impl Server {
    /// The bearer token for a thread, created on first use.
    pub fn token_for(&self, thread: usize) -> String {
        let mut tokens = self.tokens.lock().expect("mcp tokens");
        if let Some((token, _)) = tokens.iter().find(|(_, id)| **id == thread) {
            return token.clone();
        }
        let token = random_hex(4);
        tokens.insert(token.clone(), thread);
        token
    }

    /// Drops every token, for example when another workspace replaces the threads.
    pub fn reset(&self) {
        self.tokens.lock().expect("mcp tokens").clear();
    }

    pub fn next_request(&self) -> Option<Request> {
        self.requests.lock().expect("mcp requests").try_recv().ok()
    }

    fn thread_for(&self, header: &str) -> Option<usize> {
        let token = header.strip_prefix("Bearer ")?.trim();
        self.tokens.lock().expect("mcp tokens").get(token).copied()
    }
}

pub fn tools() -> Value {
    let id = json!({"type":"integer","description":"A Shina thread id from list_threads or delegate_task."});
    json!([
        {
            "name": "list_threads",
            "description": "List the user's Shina threads with their ids, titles, captains, and status.",
            "inputSchema": {"type":"object","properties":{
                "include_archived": {"type":"boolean","description":"Include archived threads. Default false."}
            }},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "read_thread",
            "description": "Read the most recent messages of a Shina thread.",
            "inputSchema": {"type":"object","properties":{
                "thread_id": id,
                "max_messages": {"type":"integer","description":"How many recent messages to return. Default 20, at most 100."}
            },"required":["thread_id"]},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "list_captains",
            "description": "List the crew captains in this workspace with their roles and current status.",
            "inputSchema": {"type":"object","properties":{}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "delegate_task",
            "description": "Start independent work in a new Shina thread. The new thread does not see this conversation, so write a complete brief. Returns its thread_id at once; use task_status to follow it.",
            "inputSchema": {"type":"object","properties":{
                "prompt": {"type":"string","description":"The complete task brief for the new thread."},
                "title": {"type":"string","description":"A short thread title. Defaults to the start of the prompt."},
                "captain": {"type":"string","description":"A captain's name from list_captains. Defaults to this thread's captain."}
            },"required":["prompt"]}
        },
        {
            "name": "task_status",
            "description": "Report a thread's status (connecting, working, needs input, failed, done, idle) and its latest reply. Returns immediately.",
            "inputSchema": {"type":"object","properties":{"thread_id": id},"required":["thread_id"]},
            "annotations": {"readOnlyHint": true}
        }
    ])
}

fn serve(stream: TcpStream, server: &Server, requests: &Sender<Request>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut length = 0usize;
    let mut authorization = String::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "authorization" => authorization = value.trim().to_owned(),
                _ => {}
            }
        }
    }
    let mut stream = stream;
    if path.split('?').next() != Some("/mcp") {
        return respond(&mut stream, 404, None, &json!({"error":"not found"}));
    }
    if method != "POST" {
        return respond(
            &mut stream,
            405,
            None,
            &json!({"error":"POST JSON-RPC to /mcp"}),
        );
    }
    let Some(thread) = server.thread_for(&authorization) else {
        return respond(
            &mut stream,
            401,
            None,
            &json!({"error":"invalid_shina_credential"}),
        );
    };
    if length > 4 * 1024 * 1024 {
        return respond(
            &mut stream,
            413,
            None,
            &json!({"error":"request too large"}),
        );
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return respond(
            &mut stream,
            400,
            None,
            &rpc_error(Value::Null, -32700, "Parse error"),
        );
    };
    let id = message["id"].clone();
    let method = message["method"].as_str().unwrap_or("");
    if id.is_null() {
        // Notifications such as notifications/initialized need no answer.
        return respond(&mut stream, 202, None, &Value::Null);
    }
    let result = match method {
        "initialize" => {
            let session = random_hex(2);
            let result = json!({
                "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "Shina", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            });
            return respond(&mut stream, 200, Some(&session), &rpc_result(id, result));
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let (reply, answer) = mpsc::channel();
            let request = Request {
                thread,
                tool: message["params"]["name"].as_str().unwrap_or("").to_owned(),
                args: message["params"]["arguments"].clone(),
                reply,
            };
            if requests.send(request).is_err() {
                Err("Shina is shutting down.".to_owned())
            } else {
                (server.wake.lock().expect("mcp waker"))();
                let outcome = answer
                    .recv_timeout(Duration::from_secs(60))
                    .unwrap_or_else(|_| Err("Shina did not answer within 60 seconds.".into()));
                Ok(match outcome {
                    Ok(value) => json!({
                        "content": [{"type":"text","text": serde_json::to_string_pretty(&value).unwrap_or_default()}],
                        "structuredContent": value,
                    }),
                    Err(error) => json!({
                        "content": [{"type":"text","text": error}],
                        "isError": true,
                    }),
                })
            }
        }
        _ => {
            return respond(
                &mut stream,
                200,
                None,
                &rpc_error(id, -32601, &format!("Unknown method {method}")),
            );
        }
    };
    let body = match result {
        Ok(value) => rpc_result(id, value),
        Err(error) => rpc_error(id, -32603, &error),
    };
    respond(&mut stream, 200, None, &body)
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    session: Option<&str>,
    body: &Value,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Payload Too Large",
    };
    let text = if body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        text.len()
    );
    if let Some(session) = session {
        head.push_str(&format!("Mcp-Session-Id: {session}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(text.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(url: &str, token: &str, body: &Value) -> (u16, String) {
        let address = url.trim_start_matches("http://").trim_end_matches("/mcp");
        let mut stream = TcpStream::connect(address).unwrap();
        let text = body.to_string();
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{text}",
            text.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response[9..12].parse().unwrap();
        let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
        (status, body)
    }

    #[test]
    fn authenticates_lists_tools_and_routes_calls_to_the_ui_thread() {
        let server = server(Arc::new(|| {})).unwrap();
        let token = server.token_for(7);
        assert_eq!(server.token_for(7), token);
        let (status, _) = post(
            &server.url,
            "wrong",
            &json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
        );
        assert_eq!(status, 401);
        let (status, body) = post(
            &server.url,
            &token,
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        );
        assert_eq!(status, 200);
        assert!(body.contains("\"Shina\""));
        let (_, body) = post(
            &server.url,
            &token,
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        );
        assert!(body.contains("delegate_task"));
        let worker = {
            let server = server.clone();
            thread::spawn(move || {
                loop {
                    if let Some(request) = server.next_request() {
                        assert_eq!(request.thread, 7);
                        let _ = request.reply.send(Ok(json!({"tool": request.tool})));
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })
        };
        let (_, body) = post(
            &server.url,
            &token,
            &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_threads","arguments":{}}}),
        );
        worker.join().unwrap();
        assert!(body.contains("list_threads"), "{body}");
    }
}
