//! Process-bound effects follow KSG EffectOutbox. Lost processes are never replayed.
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    sync::mpsc::{Sender, SyncSender},
};

/// Private dispatch representation. Public `id`/`payload` stay the durable
/// record; the live channel and record exist only in memory and are never
/// persisted or replayed at restart.
enum Dispatch {
    Harness {
        sender: SyncSender<Value>,
        record: Value,
    },
    McpReply {
        sender: Sender<Result<Value, String>>,
        result: Result<Value, String>,
    },
}

pub struct Effect {
    pub id: String,
    pub payload: Value,
    dispatch: Dispatch,
}

impl Effect {
    /// Durable prompt attribution used before the effect can reach a provider.
    pub fn prompt_run(&self) -> Option<(&str, &str)> {
        let Dispatch::Harness { record, .. } = &self.dispatch else {
            return None;
        };
        (record["type"] == "prompt").then(|| {
            (
                record["runId"].as_str().unwrap_or_default(),
                record["id"].as_str().unwrap_or_default(),
            )
        })
    }
}

thread_local! {
    static CAPTURE: RefCell<Option<Vec<Effect>>> = const { RefCell::new(None) };
}

pub struct Capture;
pub fn captured_count() -> usize {
    CAPTURE.with(|capture| capture.borrow().as_ref().map(Vec::len).unwrap_or(0))
}
impl Capture {
    pub fn begin() -> Self {
        CAPTURE.with(|capture| {
            assert!(capture.borrow().is_none(), "nested effect capture");
            *capture.borrow_mut() = Some(Vec::new());
        });
        Self
    }
    pub fn finish(self) -> Vec<Effect> {
        CAPTURE.with(|capture| capture.borrow_mut().take().unwrap_or_default())
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        CAPTURE.with(|capture| {
            capture.borrow_mut().take();
        });
    }
}

pub fn send(sender: &SyncSender<Value>, record: Value) -> Result<(), String> {
    CAPTURE.with(|capture| {
        if let Some(effects) = capture.borrow_mut().as_mut() {
            if effects.len() >= 1024 {
                return Err("Too many outbound effects in one runtime batch.".into());
            }
            effects.push(Effect {
                id: uuid::Uuid::new_v4().to_string(),
                // The durable input lives in the command/runtime input documents.
                // Do not retain raw RPC records, which can contain credentials.
                payload: json!({"type":record["type"],"id":record["id"]}),
                dispatch: Dispatch::Harness {
                    sender: sender.clone(),
                    record,
                },
            });
            Ok(())
        } else {
            sender
                .try_send(record)
                .map_err(|e| format!("The agent did not accept the command: {e}"))
        }
    })
}

/// Captures an MCP reply so `saved` results cannot reach the caller before the
/// mutation commits. Without a capture the reply stays synchronous, matching
/// direct tests and non-runtime callers.
pub fn reply(
    sender: &Sender<Result<Value, String>>,
    result: Result<Value, String>,
) -> Result<(), String> {
    CAPTURE.with(|capture| {
        if let Some(effects) = capture.borrow_mut().as_mut() {
            if effects.len() >= 1024 {
                return Err("Too many outbound effects in one runtime batch.".into());
            }
            effects.push(Effect {
                id: uuid::Uuid::new_v4().to_string(),
                // Only the marker is durable: never the tool args, result or body.
                payload: json!({"type":"mcp.reply"}),
                dispatch: Dispatch::McpReply {
                    sender: sender.clone(),
                    result,
                },
            });
            Ok(())
        } else {
            let _ = sender.send(result);
            Ok(())
        }
    })
}

/// Runs only after the state, receipt and all pending effects commit.
pub fn drain(database: &crate::database::Database, effects: Vec<Effect>) -> Result<(), String> {
    if effects.is_empty() {
        return Ok(());
    }
    let _dispatch = database
        .claim_dispatch()?
        .ok_or("Dispatch is fenced for maintenance.")?;
    for effect in effects {
        #[cfg(debug_assertions)]
        crash_boundary(&effect.payload, "pending");
        database.effect_status(&effect.id, "dispatching", None)?;
        #[cfg(debug_assertions)]
        crash_boundary(&effect.payload, "dispatching");
        match effect.dispatch {
            Dispatch::Harness { sender, record } => match sender.try_send(record) {
                Ok(()) => {
                    #[cfg(debug_assertions)]
                    crash_boundary(&effect.payload, "sent");
                    database.effect_status(&effect.id, "completed", None)?;
                }
                Err(_) => {
                    database.effect_status(
                        &effect.id,
                        "interrupted",
                        Some("Harness did not accept dispatch. Review before retrying."),
                    )?;
                    return Err("Harness dispatch interrupted. Review before retrying.".into());
                }
            },
            Dispatch::McpReply { sender, result } => match sender.send(result) {
                Ok(()) => {
                    #[cfg(debug_assertions)]
                    crash_boundary(&effect.payload, "sent");
                    database.effect_status(&effect.id, "completed", None)?;
                }
                // A dropped MCP receiver must not fail the batch or block later
                // effects; the reply is process-bound and never replays.
                Err(_) => {
                    database.effect_status(
                        &effect.id,
                        "interrupted",
                        Some("MCP caller disconnected before receiving the committed result."),
                    )?;
                }
            },
        }
    }
    Ok(())
}

/// Debug-only crash injection for real-process integration tests. It is restricted
/// to the explicit isolated profile and never accepts a general output path.
pub fn peer_ack_boundary(
    directory: &std::path::Path,
    method: &str,
    result: &Result<Value, String>,
) {
    #[cfg(debug_assertions)]
    if method == "peer.task.ack"
        && result
            .as_ref()
            .is_ok_and(|value| value["status"] == "accepted")
        && std::env::var("SHINA_TEST_CRASH_EFFECT").ok().as_deref() == Some("peer-ack")
        && directory.starts_with(std::env::temp_dir())
        && directory
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("shina-server-integration-")
            })
    {
        // The target committed acknowledgement, but the home has not received it.
        std::process::exit(86);
    }
    #[cfg(not(debug_assertions))]
    let _ = (directory, method, result);
}

/// Debug-only prompt crash injection, restricted to isolated integration profiles.
#[cfg(debug_assertions)]
fn crash_boundary(payload: &Value, phase: &str) {
    if payload["type"] == "prompt"
        && std::env::var("SHINA_TEST_CRASH_EFFECT").ok().as_deref() == Some(phase)
        && crate::store::directory().is_some_and(|directory| {
            directory.starts_with(std::env::temp_dir())
                && directory
                    .parent()
                    .and_then(|p| p.file_name())
                    .is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with("shina-server-integration-")
                    })
        })
    {
        if phase == "sent" {
            // `try_send` only proved the record entered the harness channel.
            // Let the fixture read it so this phase models a lost response
            // after the harness consumed the prompt, not a dropped write.
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        std::process::exit(86);
    }
}
