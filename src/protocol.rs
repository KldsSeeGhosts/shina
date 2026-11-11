//! Slice 1 JSON-RPC contract. Snapshots replace replay until the SQLite migration.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAJOR: u32 = 1;
pub const MINOR: u32 = 1;
pub const MAX_CONTROL: usize = 1024 * 1024;

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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
        Err(error) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error}}),
    }
}

pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","method":method,"params":params})
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
