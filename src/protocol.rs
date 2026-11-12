//! Durable JSON-RPC contract. Patches use typed set/remove operations and path segments.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAJOR: u32 = 1;
pub const MINOR: u32 = 8;
pub const HISTORY_ITEMS: &str = "history-items";
pub const HISTORY_UPDATE_REQUIRED: &str =
    "Update required. Update Shina on the client and server to use scoped history and live items.";
pub const HISTORY_LEASE_GONE: &str =
    "History fragment lease expired or evicted. Reload the scoped snapshot.";
pub const HISTORY_LEASE_GONE_CODE: i64 = -32010;
pub const HISTORY_UPDATE_REQUIRED_CODE: i64 = -32012;
pub const PEER_TASKS: &str = "peer-tasks-v2";
pub const THREAD_READ: &str = "thread-read-v2";
pub const REMOTE_PROJECT_SCOPE: &str = "remote-project-scope-v1";
pub const PEER_DELEGATION: &str = "peer-delegation";
pub const LINKED_UPDATES: &str = "linked-updates";
pub const MAX_BROWSE_ENTRIES: usize = 200;
pub const FILESYSTEM_BROWSE: &str = "filesystem-browse";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    pub name: String,
    /// An opaque path on the answering host. Clients must not join or normalize it.
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<FolderEntry>,
    pub truncated: bool,
    pub target_exists: bool,
}

/// Every routed method has an explicit grant. Unknown methods fail closed.
pub fn method_grant(method: &str) -> Option<&'static str> {
    Some(match method {
        "hello"
        | "workspace.subscribe"
        | "thread.subscribe"
        | "subscription.cancel"
        | "thread.history"
        | "command.get"
        | "server.status"
        | "blobs.read"
        | "filesystem.browse" => "read",
        "request.respond" => "approve",
        "server.updateStatus" | "server.requestUpdate" => "update",
        "devices.list"
        | "peer.link"
        | "peer.list"
        | "peer.unlink"
        | "devices.pair"
        | "devices.revoke"
        | "devices.grant"
        | "devices.revokeGrant"
        | "server.validateRestart"
        | "server.prepareRestart"
        | "server.cancelRestart"
        | "server.clientReady"
        | "server.commitRestart"
        | "server.stop" => "maintain",
        "peer.capabilities" | "peer.thread.launch" | "peer.thread.wait" | "peer.task.ack"
        | "peer.thread.read" | "peer.thread.list" | "peer.thread.send" | "peer.thread.stop" => {
            "peer-delegate"
        }
        "thread.send"
        | "thread.stop"
        | "thread.configure"
        | "thread.retry"
        | "thread.visit"
        | "thread.create"
        | "thread.rename"
        | "thread.delete"
        | "thread.lifecycle"
        | "thread.undo"
        | "thread.drop"
        | "thread.compact"
        | "thread.fork"
        | "thread.rewind"
        | "thread.forkPoints"
        | "settings.autoSettle"
        | "settings.defaultHarness"
        | "settings.threadDefaults"
        | "background.keepAwake"
        | "queue.remove"
        | "queue.sendNow"
        | "queue.edit"
        | "input.claim"
        | "project.put"
        | "project.remove"
        | "project.clone"
        | "project.cancelClone"
        | "catalog.refresh"
        | "commands.refresh"
        | "captain.open"
        | "captain.put"
        | "captain.scope"
        | "captain.reset"
        | "memory.put"
        | "memory.remove"
        | "profile.put"
        | "routine.pause"
        | "routine.remove"
        | "task.resume"
        | "task.dismiss"
        | "task.resumeResults"
        | "task.delegate"
        | "task.send"
        | "task.cancel"
        | "uploads.write" => "operate",
        _ => return None,
    })
}
pub const MAX_CONTROL: usize = 1024 * 1024;
pub const PAGE_BYTES: usize = 64 * 1024;
pub const MAX_PAGE_BYTES: usize = 256 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
pub const HISTORY_FIELDS: [&str; 13] = [
    "messages",
    "tools",
    "thoughts",
    "work",
    "diagnostics",
    "compactions",
    "fork_points",
    "commands",
    "models",
    "queue",
    "steering",
    "follow_ups",
    "stats",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Patch {
    Set { path: Vec<String>, value: Value },
    Remove { path: Vec<String> },
}

pub fn diff(before: &Value, after: &Value) -> Value {
    fn walk(before: &Value, after: &Value, path: &mut Vec<String>, out: &mut Vec<Patch>) {
        if before == after {
            return;
        }
        match (before, after) {
            (Value::Object(a), Value::Object(b)) => {
                for key in a.keys().filter(|key| !b.contains_key(*key)) {
                    path.push(key.clone());
                    out.push(Patch::Remove { path: path.clone() });
                    path.pop();
                }
                for (key, value) in b {
                    path.push(key.clone());
                    if let Some(before) = a.get(key) {
                        walk(before, value, path, out);
                    } else {
                        out.push(Patch::Set {
                            path: path.clone(),
                            value: value.clone(),
                        });
                    }
                    path.pop();
                }
            }
            (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
                for (index, (a, b)) in a.iter().zip(b).enumerate() {
                    path.push(index.to_string());
                    walk(a, b, path, out);
                    path.pop();
                }
            }
            _ => out.push(Patch::Set {
                path: path.clone(),
                value: after.clone(),
            }),
        }
    }
    let mut patches = Vec::new();
    walk(before, after, &mut Vec::new(), &mut patches);
    json!(patches)
}

pub fn apply(state: &mut Value, patches: Value) -> Result<(), String> {
    let patches: Vec<Patch> = serde_json::from_value(patches).map_err(|e| e.to_string())?;
    for patch in patches {
        let (path, value) = match patch {
            Patch::Set { path, value } => (path, Some(value)),
            Patch::Remove { path } => (path, None),
        };
        if path.is_empty() {
            *state = value.ok_or("Cannot remove projection root.")?;
            continue;
        }
        let mut target = &mut *state;
        for segment in &path[..path.len() - 1] {
            target = match target {
                Value::Object(object) => object.get_mut(segment),
                Value::Array(array) => segment.parse::<usize>().ok().and_then(|i| array.get_mut(i)),
                _ => None,
            }
            .ok_or("Projection patch has a missing parent.")?;
        }
        let key = path.last().expect("nonempty path");
        match (target, value) {
            (Value::Object(object), Some(value)) => {
                object.insert(key.clone(), value);
            }
            (Value::Object(object), None) => {
                object.remove(key);
            }
            (Value::Array(array), Some(value)) => {
                let index = key
                    .parse::<usize>()
                    .map_err(|_| "Invalid patch array index.")?;
                *array
                    .get_mut(index)
                    .ok_or("Patch array index out of bounds.")? = value;
            }
            _ => return Err("Invalid projection patch target.".into()),
        }
    }
    Ok(())
}

pub fn revision(value: &Value) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.to_string()))
}

pub fn history_payload(thread: &Value) -> Value {
    let mut history = json!({"id":thread["id"]});
    for key in HISTORY_FIELDS {
        history[key] = thread[key].clone();
    }
    history
}

/// History payloads travel separately. The workspace holds only lightweight thread fields.
pub fn compact(state: &Value) -> Value {
    let mut state = state.clone();
    if let Some(threads) = state["threads"].as_array_mut() {
        for thread in threads {
            let revision = revision(&history_payload(thread));
            for key in HISTORY_FIELDS {
                thread[key] = if key == "stats" { json!({}) } else { json!([]) };
            }
            thread["historyRevision"] = json!(revision);
        }
    }
    state
}

/// Cursor pins both identity and content.
/// JSON text is split only on UTF-8 boundaries, including a single oversized message.
pub fn text_page(text: &str, identity: &str, params: &Value) -> Result<Value, String> {
    let revision = revision(&json!({"identity":identity,"text":text}));
    text_page_revision(text, &revision, params)
}

/// The caller owns the immutable text associated with this revision.
pub fn text_page_revision(text: &str, revision: &str, params: &Value) -> Result<Value, String> {
    let offset = match params["cursor"].as_str() {
        None => 0,
        Some(cursor) => {
            let (saved, offset) = cursor.split_once(':').ok_or("Invalid history cursor.")?;
            if saved != revision {
                return Err("Page cursor does not match immutable data.".into());
            }
            offset
                .parse::<usize>()
                .map_err(|_| "Invalid history cursor offset.")?
        }
    };
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err("Invalid history cursor offset.".into());
    }
    let budget = params["byteBudget"]
        .as_u64()
        .unwrap_or(PAGE_BYTES as u64)
        .clamp(1024, MAX_PAGE_BYTES as u64) as usize;
    let mut end = offset.saturating_add(budget).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(
        json!({"encoding":"utf8-json","data":&text[offset..end],"revision":revision,
        "nextCursor":(end < text.len()).then(|| format!("{revision}:{end}")),"totalBytes":text.len()}),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SendInput {
    pub thread_id: usize,
    pub text: String,
    pub attachments: Vec<Value>,
    pub mode: SendMode,
    #[serde(default)]
    pub editor_revision: Option<uuid::Uuid>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SendMode {
    Prompt,
    Steer,
    Queue,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestResponse {
    pub thread_id: usize,
    pub request_id: Value,
    pub generation: String,
    pub response: Value,
    #[serde(default)]
    pub session: bool,
    #[serde(default)]
    pub answers: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub command_id: uuid::Uuid,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub command_id: uuid::Uuid,
    pub device_id: String,
    pub hash: String,
    pub status: ReceiptStatus,
    pub result: Value,
    pub seq: u64,
}

impl Command {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            command_id: uuid::Uuid::new_v4(),
            method: method.into(),
            params,
        }
    }
    pub fn hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut request = json!({"method":self.method,"params":self.params});
        // Stay canonical even if another dependency enables serde_json/preserve_order.
        request.sort_all_objects();
        format!("{:x}", Sha256::digest(request.to_string()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReceiptStatus {
    Accepted,
    Rejected,
    Interrupted,
}

pub fn response(id: Value, result: Result<Value, String>) -> Value {
    match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(error) => {
            let code = match error.as_str() {
                HISTORY_LEASE_GONE => HISTORY_LEASE_GONE_CODE,
                HISTORY_UPDATE_REQUIRED => HISTORY_UPDATE_REQUIRED_CODE,
                _ => -32000,
            };
            json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":error}})
        }
    }
}

pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","method":method,"params":params})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_routed_protocol_method_has_an_explicit_grant() {
        // Read the actual dispatch code, so adding a route without updating authorization fails.
        let reducer = include_str!("client_state.rs")
            .split("pub fn execute_command")
            .nth(1)
            .unwrap();
        let reducer = reducer.split("impl Thread").next().unwrap();
        let runtime = include_str!("runtime.rs")
            .split("fn maintenance_command")
            .nth(1)
            .unwrap();
        let runtime = runtime.split("pub fn failure").next().unwrap();
        let server = include_str!("server.rs")
            .split("async fn connection")
            .nth(1)
            .unwrap();
        let server = server.split("pub fn serve").next().unwrap();
        let mut routed = std::collections::BTreeSet::from([
            "hello",
            "blobs.read",
            "uploads.write",
            "devices.revoke",
            "devices.grant",
            "devices.revokeGrant",
            "server.updateStatus",
            "server.requestUpdate",
            "command.get",
            "peer.link",
            "peer.list",
            "peer.unlink",
            "peer.capabilities",
            "peer.thread.launch",
            "peer.thread.wait",
            "peer.task.ack",
            "peer.thread.read",
            "peer.thread.list",
            "peer.thread.send",
            "peer.thread.stop",
        ]);
        for source in [reducer, runtime, server] {
            let pieces: Vec<_> = source.split('"').collect();
            for i in (1..pieces.len()).step_by(2) {
                let name = pieces[i];
                if !name.contains('.') || !name.chars().all(|c| c.is_ascii_alphabetic() || c == '.')
                {
                    continue;
                }
                let before = pieces[i - 1].trim_end();
                let after = pieces.get(i + 1).copied().unwrap_or("").trim_start();
                if before.ends_with("method ==")
                    || (after.starts_with("=>") && !name.starts_with("subscription."))
                    || after.starts_with('|')
                    || before.ends_with('|')
                    || before.ends_with("matches!(method,")
                {
                    routed.insert(name);
                }
            }
        }
        assert!(routed.len() > 60, "{routed:?}");
        assert!(routed.contains("thread.subscribe") && routed.contains("filesystem.browse"));
        for method in routed {
            let expected = match method {
                "hello"
                | "workspace.subscribe"
                | "thread.subscribe"
                | "subscription.cancel"
                | "thread.history"
                | "command.get"
                | "server.status"
                | "blobs.read"
                | "filesystem.browse" => "read",
                "request.respond" => "approve",
                "server.updateStatus" | "server.requestUpdate" => "update",
                "devices.list"
                | "peer.link"
                | "peer.list"
                | "peer.unlink"
                | "devices.pair"
                | "devices.revoke"
                | "devices.grant"
                | "devices.revokeGrant"
                | "server.validateRestart"
                | "server.prepareRestart"
                | "server.cancelRestart"
                | "server.clientReady"
                | "server.commitRestart"
                | "server.stop" => "maintain",
                "peer.capabilities" | "peer.thread.launch" | "peer.thread.wait"
                | "peer.task.ack" | "peer.thread.read" | "peer.thread.list"
                | "peer.thread.send" | "peer.thread.stop" => "peer-delegate",
                _ => "operate",
            };
            assert_eq!(method_grant(method), Some(expected), "{method}");
        }
        for method in ["unknown.mutation", "server.newMethod", "peer.newMethod"] {
            assert_eq!(method_grant(method), None, "{method}");
        }
    }

    #[test]
    fn receipt_hashes_ignore_object_key_order_but_not_content() {
        let a = Command::new(
            "thread.configure",
            serde_json::from_str(r#"{"threadId":1,"choice":{"model":"a","provider":"cpa"}}"#)
                .unwrap(),
        );
        let b = Command::new(
            "thread.configure",
            serde_json::from_str(r#"{"choice":{"provider":"cpa","model":"a"},"threadId":1}"#)
                .unwrap(),
        );
        assert_eq!(a.hash(), b.hash());
        let mut changed = b;
        changed.params["choice"]["model"] = json!("b");
        assert_ne!(a.hash(), changed.hash());
    }

    #[test]
    fn typed_patches_roundtrip_add_remove_arrays_and_escaped_keys() {
        let before = json!({"threads":[{"id":1,"status":"ready"}],"old":true,"a/b":{"~key":1}});
        let after = json!({"threads":[{"id":1,"status":"working"}],"new":null,"a/b":{"~key":2}});
        let mut projected = before.clone();
        apply(&mut projected, diff(&before, &after)).unwrap();
        assert_eq!(projected, after);
        assert!(projected.as_object().unwrap().contains_key("new"));
    }

    #[test]
    fn immutable_text_pages_bound_unicode_and_reject_wrong_data_or_identity() {
        let text = json!({"messages":[{"text":"😀 image transcript\n".repeat(10000)}]}).to_string();
        let mut cursor = Value::Null;
        let mut assembled = String::new();
        let mut first_cursor = None;
        loop {
            let page = text_page(
                &text,
                "thread-1",
                &json!({"cursor":cursor,"byteBudget":1024}),
            )
            .unwrap();
            assert!(page["data"].as_str().unwrap().len() <= 1024);
            assembled.push_str(page["data"].as_str().unwrap());
            cursor = page["nextCursor"].clone();
            if first_cursor.is_none() {
                first_cursor = Some(cursor.clone());
            }
            if cursor.is_null() {
                break;
            }
        }
        assert_eq!(assembled, text);
        let params = json!({"cursor":first_cursor.unwrap()});
        assert!(text_page("changed", "thread-1", &params).is_err());
        assert!(text_page(&text, "thread-2", &params).is_err());
    }

    #[test]
    fn status_only_changes_do_not_invalidate_the_history_revision() {
        let mut state =
            json!({"threads":[{"id":1,"status":"Running","messages":[{"text":"Stable"}]}]});
        let before = compact(&state);
        state["threads"][0]["status"] = json!("Completed");
        assert_eq!(
            before["threads"][0]["historyRevision"],
            compact(&state)["threads"][0]["historyRevision"]
        );
    }
}
