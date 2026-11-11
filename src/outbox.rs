//! Process-bound effects follow KSG EffectOutbox. Lost processes are never replayed.
use serde_json::{Value, json};
use std::{cell::RefCell, sync::mpsc::SyncSender};

pub struct Effect {
    pub id: String,
    pub payload: Value,
    sender: SyncSender<Value>,
    record: Value,
}

thread_local! {
    static CAPTURE: RefCell<Option<Vec<Effect>>> = const { RefCell::new(None) };
}

pub struct Capture;
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
                sender: sender.clone(),
                record,
            });
            Ok(())
        } else {
            sender
                .try_send(record)
                .map_err(|e| format!("The agent did not accept the command: {e}"))
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
        match effect.sender.try_send(effect.record) {
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
        }
    }
    Ok(())
}

/// Debug-only crash injection for real-process integration tests. It is restricted
/// to the explicit isolated profile and never accepts a general output path.
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
        std::process::exit(86);
    }
}
