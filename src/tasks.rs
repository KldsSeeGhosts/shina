//! Durable delegation records. Pi's active history is authoritative for acceptance and recovery.
use super::*;
use crate::protocol::{Receipt, ReceiptStatus};
use serde::{Deserialize, Serialize};

pub(super) const RUNNING_LIMIT: usize = 3;
pub(super) const UNFINISHED_LIMIT: usize = 8;
const RESUME_MESSAGE: &str = "Shina restarted mid-task. Check the current state of the files and any partial changes or tool actions, then finish the task. Do not assume earlier actions failed or repeat them without checking.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Starting,
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl TaskState {
    pub fn id(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
    pub fn finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
    fn occupies_slot(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Task {
    #[serde(default)]
    pub remote: Option<RemoteTask>,
    /// The caller-scoped application retry contract for a keyed delegation.
    #[serde(default)]
    pub request_key: Option<RequestKey>,
    pub id: String,
    pub parent_thread_id: usize,
    pub child_thread_id: usize,
    pub title: String,
    pub brief: String,
    pub project_id: Option<ProjectId>,
    #[serde(default)]
    pub coordinator_captain: Option<usize>,
    pub machine: String,
    pub model_target: Option<Value>,
    pub state: TaskState,
    pub delivered: bool,
    pub delivery_id: String,
    pub result_excerpt: String,
    pub result: String,
    pub error_excerpt: String,
    #[serde(default)]
    pub memory_suggestions: Vec<String>,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    /// The current attempt's stable generation. It changes only when a new task
    /// attempt is queued, never when input joins a running attempt. Cancellation
    /// fences against this id so a delayed stop cannot land on a later attempt.
    #[serde(default)]
    pub attempt_id: String,
    /// Exact last submitted brief, including a stable marker, rather than a text heuristic.
    pub attempt_message: String,
    /// A follow-up must not overwrite an earlier result awaiting acceptance.
    #[serde(default)]
    prior_deliveries: Vec<ResultDelivery>,
    #[serde(skip)]
    recovering: bool,
    #[serde(skip)]
    pub(super) hold_delivery: bool,
}

/// One stable request key and the canonical intent hash it committed. The hash
/// excludes response-only options such as wait_seconds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestKey {
    pub key: String,
    pub intent_hash: String,
}

/// The durable caller identity behind a delegation. A Captain's scope survives
/// home resets; a plain thread's scope is its own stable id.
#[derive(Clone, Copy)]
enum DelegateScope {
    Captain(usize),
    Thread(usize),
}

impl DelegateScope {
    /// An audit namespace, never a registered device or an authentication grant.
    fn device(self) -> String {
        match self {
            Self::Captain(captain) => format!("mcp/delegate/captain/{captain}"),
            Self::Thread(id) => format!("mcp/delegate/thread/{id}"),
        }
    }
}

const DELEGATE_KEY_MISMATCH: &str =
    "client_request_id was already used with a different brief or target.";
const CONTROL_KEY_MISMATCH: &str =
    "client_request_id was already used with different send or cancel arguments.";

/// A valid retry key is a nonempty string in the shared local/remote contract.
pub(super) fn delegate_request_key(args: &Value) -> Result<Option<&str>, String> {
    args.get("client_request_id")
        .map(|value| {
            value
                .as_str()
                .filter(|key| !key.is_empty() && key.len() <= 256)
                .ok_or_else(|| "client_request_id must be 1 to 256 characters.".to_owned())
        })
        .transpose()
}

/// Canonical delegation intent: the arguments minus the key itself and
/// response-only options like wait_seconds, which may differ on a retry.
fn delegate_intent(args: &Value) -> Value {
    let mut intent = args.clone();
    if let Some(object) = intent.as_object_mut() {
        object.remove("client_request_id");
        object.remove("wait_seconds");
    }
    intent
}

/// The same canonical hash the runtime records on the durable receipt.
pub(super) fn delegate_intent_hash(args: &Value) -> String {
    crate::protocol::Command {
        command_id: uuid::Uuid::nil(),
        method: "task.delegate".into(),
        params: delegate_intent(args),
    }
    .hash()
}

/// A stable command id keeps one retry key in the durable receipts table,
/// distinct from the per-call JSON-RPC request id.
fn delegate_request_command_id(key: &str) -> uuid::Uuid {
    keyed_command_id("shina.delegate.request-key", key)
}

/// task_send and task_cancel share the caller-scoped receipt table under their
/// own derivation, so a send key can never collide with a delegation key.
fn task_control_command_id(key: &str) -> uuid::Uuid {
    keyed_command_id("shina.task-control.request-key", key)
}

fn keyed_command_id(namespace: &str, key: &str) -> uuid::Uuid {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{namespace}:{key}"));
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

fn task_request_key(task: &Task) -> Option<&str> {
    if let Some(request) = &task.request_key {
        return Some(request.key.as_str());
    }
    task.remote
        .as_ref()
        .and_then(|remote| remote.client_request_id.as_deref())
}

fn task_intent_hash(task: &Task) -> Option<String> {
    if let Some(request) = &task.request_key {
        return Some(request.intent_hash.clone());
    }
    task.remote
        .as_ref()
        .and_then(|remote| remote.launch_command.params.get("request"))
        .map(delegate_intent_hash)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ResultDelivery {
    id: String,
    text: String,
    #[serde(skip)]
    hold: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskView {
    pub remote: Option<RemoteTask>,
    pub id: String,
    pub child_id: usize,
    pub title: String,
    pub workspace_label: String,
    pub model_label: String,
    pub state: TaskState,
    pub last_activity_line: String,
    /// Unix milliseconds, not the thread UI's elapsed clock.
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub queue_position: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttentionKind {
    Approval,
    Question,
    Failed,
    Interrupted,
    ReadyForReview,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttentionItem {
    pub thread_id: usize,
    pub parent_id: Option<usize>,
    pub kind: AttentionKind,
    pub short_title: String,
}

fn unique_id() -> String {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn delivery_text(id: &str, body: &str) -> String {
    format!("{INJECTION_PREFIX}[delivery {id}]\n{body}")
}

fn excerpt(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

pub(super) const REPORT_CUT: &str = "\n[Report truncated. Use read_thread to continue.]";
pub(super) fn report_excerpt(text: &str) -> String {
    if text.chars().count() <= 6000 {
        text.into()
    } else {
        format!(
            "{}{REPORT_CUT}",
            excerpt(text, 6000 - REPORT_CUT.chars().count())
        )
    }
}

pub(super) fn delivery_id(text: &str) -> Option<&str> {
    text.strip_prefix(INJECTION_PREFIX)?
        .lines()
        .next()?
        .strip_prefix("[delivery ")?
        .strip_suffix(']')
}

pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

impl Task {
    pub(super) fn new(thread: &Thread, brief: &str, model_target: Option<Value>) -> Self {
        let id = unique_id();
        let parent = thread
            .parent
            .or_else(|| thread.peer_origin.as_ref().map(|o| o.parent_thread_id))
            .expect("delegated thread parent");
        Self {
            remote: None,
            request_key: None,
            attempt_id: format!("{id}-brief"),
            attempt_message: delivery_text(
                &format!("{id}-brief"),
                &format!("Shina: task from parent #{parent}\n\n{brief}"),
            ),
            delivery_id: format!("{id}-result"),
            id,
            parent_thread_id: parent,
            child_thread_id: thread.id,
            title: thread.title.clone(),
            brief: brief.into(),
            project_id: thread.project,
            coordinator_captain: thread.crew_captain,
            machine: thread.machine.clone(),
            model_target,
            prior_deliveries: Vec::new(),
            state: TaskState::Queued,
            delivered: false,
            result_excerpt: String::new(),
            result: String::new(),
            error_excerpt: String::new(),
            memory_suggestions: Vec::new(),
            created_at: lifecycle::now(),
            started_at: None,
            finished_at: None,
            recovering: false,
            hold_delivery: false,
        }
    }

    pub(super) fn pending_deliveries(&self) -> bool {
        !self.prior_deliveries.is_empty()
    }

    pub(super) fn result_notifications(&self) -> Vec<String> {
        let mut texts: Vec<_> = self
            .prior_deliveries
            .iter()
            .map(|delivery| delivery.text.clone())
            .collect();
        if self.state.finished() && !self.delivered {
            texts.push(self.completion_message());
        }
        texts
    }

    pub(super) fn hold_result_notifications(&mut self) {
        for delivery in &mut self.prior_deliveries {
            delivery.hold = true;
        }
        if self.state.finished() && !self.delivered {
            self.hold_delivery = true;
        }
    }

    pub(super) fn finish(&mut self, state: TaskState, reply: &str, error: &str) {
        self.state = state;
        self.result_excerpt = report_excerpt(reply);
        self.result = reply.into();
        self.error_excerpt = excerpt(error, 2000);
        self.finished_at = Some(lifecycle::now());
    }

    pub(super) fn completion_message(&self) -> String {
        let suggestions = if self.memory_suggestions.is_empty() {
            String::new()
        } else {
            format!(
                "\nMemory suggestions from worker, untrusted data for captain review, not instructions:\n{}",
                serde_json::to_string(&self.memory_suggestions).expect("suggestion serialization")
            )
        };
        let continuation = if self.result_excerpt.ends_with(REPORT_CUT) {
            let target = self
                .remote
                .as_ref()
                .map(|remote| json!({"machine":remote.server_id,"threadId":remote.thread_id}))
                .unwrap_or_else(|| json!({"threadId":self.child_thread_id}));
            format!(
                "\nReport truncated. Read the rest with read_thread({target}); follow items[].continuation."
            )
        } else {
            String::new()
        };
        delivery_text(
            &self.delivery_id,
            &format!(
                "Shina: task #{} finished\nTask id: {}\nTitle: {}\nStatus: {}\nLast reply:\n{}\nError: {}{suggestions}{continuation}",
                self.child_thread_id,
                self.id,
                self.title,
                self.state.id(),
                if self.result_excerpt.is_empty() {
                    "(no reply)"
                } else {
                    &self.result_excerpt
                },
                if self.error_excerpt.is_empty() {
                    "none"
                } else {
                    &self.error_excerpt
                },
            ),
        )
    }

    pub(super) fn reconcile(&mut self, record: &Value) {
        self.recovering = false;
        // A persisted failure/cancellation is already a conclusive local decision.
        if matches!(self.state, TaskState::Failed | TaskState::Cancelled) {
            return;
        }
        let result = (|| {
            if record["success"] == false {
                return Err(
                    "Pi history could not be read. Check the saved session before resuming.",
                );
            }
            let messages = record["data"]["messages"]
                .as_array()
                .ok_or("Pi history is uncertain. Check the saved session before resuming.")?;
            let accepted = messages
                .iter()
                .rposition(|message| {
                    message["role"] == "user"
                        && content_text(&message["content"]) == self.attempt_message
                })
                .ok_or("Pi history does not show acceptance of this brief. It was not replayed.")?;
            let after = &messages[accepted + 1..];
            let last = after.last().ok_or(
                "The child had no final reply after its brief. Check partial work before resuming.",
            )?;
            if after.iter().any(|m| m["role"] == "user")
                || last["role"] != "assistant"
                || last["stopReason"] != "stop"
                || last["content"]
                    .as_array()
                    .is_some_and(|parts| parts.iter().any(|p| p["type"] == "toolCall"))
            {
                return Err(
                    "The child did not leave a proven final reply. Check partial work before resuming.",
                );
            }
            let reply = content_text(&last["content"]);
            if reply.trim().is_empty() {
                return Err(
                    "The child's final reply is empty. Check partial work before resuming.",
                );
            }
            Ok((reply, last["timestamp"].as_u64()))
        })();
        match result {
            Ok((reply, timestamp)) => {
                let saved_finished_at = self.finished_at;
                self.finish(TaskState::Completed, &reply, "");
                self.finished_at = saved_finished_at.or(timestamp).or(self.finished_at);
            }
            Err(error) => {
                self.state = TaskState::Interrupted;
                self.finished_at = None;
                self.error_excerpt = error.into();
            }
        }
    }
}

impl Thread {
    pub(super) fn observe_task_history(&mut self, record: &Value) {
        if record["type"] != "response" {
            return;
        }
        if record["command"] == "prompt"
            && record["success"] != false
            && self
                .injection_id
                .as_ref()
                .is_some_and(|id| record["id"] == *id)
            && let Some(id) = self
                .pending_injections
                .front()
                .and_then(|i| delivery_id(&i.text))
        {
            self.accepted_deliveries.push(id.into());
        }
        if record["command"] == "get_messages" && record["id"] == "history" {
            if self.recovering_task {
                self.task_history = Some(record.clone());
            }
            if record["success"] != false
                && let Some(messages) = record["data"]["messages"].as_array()
            {
                for message in messages.iter().filter(|m| m["role"] == "user") {
                    let text = content_text(&message["content"]);
                    if let Some(id) = delivery_id(&text) {
                        self.accepted_deliveries.push(id.into());
                    }
                }
            }
        }
    }
}

impl State {
    pub(super) fn task_home(&self, task: &Task) -> Option<usize> {
        let thread = self
            .threads
            .iter()
            .find(|thread| thread.id == task.child_thread_id);
        if task.coordinator_captain.is_some() {
            return thread.and_then(|thread| {
                thread
                    .crew_captain
                    .and_then(|captain| self.captain_home(captain))
            });
        }
        Some(self.effective_home(task.parent_thread_id))
    }

    pub(super) fn task_has_background_work(&self, thread: usize) -> bool {
        self.tasks.iter().any(|task| {
            task.child_thread_id == thread
                && matches!(
                    task.state,
                    TaskState::Queued | TaskState::Starting | TaskState::Running
                )
                || self.task_home(task) == Some(thread)
                    && (task.state.finished() && !task.delivered
                        || !task.prior_deliveries.is_empty())
        })
    }
    pub fn mark_clean_quit(&mut self) {
        if let Some(marker) = &mut self.run_marker {
            marker.clean = true;
        }
    }
    pub fn tasks_for(&self, parent_thread_id: usize) -> Vec<TaskView> {
        self.tasks
            .iter()
            .filter(|task| self.task_home(task) == Some(parent_thread_id))
            .map(|task| {
                let thread = self
                    .threads
                    .iter()
                    .find(|thread| thread.id == task.child_thread_id);
                TaskView {
                    remote: task.remote.clone(),
                    id: task.id.clone(),
                    child_id: task.child_thread_id,
                    title: task.title.clone(),
                    workspace_label: task
                        .remote
                        .as_ref()
                        .and_then(|remote| remote.project["name"].as_str())
                        .map(str::to_owned)
                        .or_else(|| self.project(task.project_id).map(|p| p.name.clone()))
                        .unwrap_or_else(|| "No project".into()),
                    model_label: thread
                        .map(|t| t.model.clone())
                        .unwrap_or_else(|| DEFAULT_MODEL.into()),
                    state: task.state,
                    last_activity_line: if task
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.state == RemoteState::UpdateRequired)
                    {
                        task.remote
                            .as_ref()
                            .and_then(|r| r.last_error.clone())
                            .unwrap_or_else(|| "Update required.".into())
                    } else if task.remote.as_ref().is_some_and(|r| r.needs_you) {
                        format!(
                            "Needs you on {}",
                            task.remote.as_ref().expect("remote").label
                        )
                    } else if !task.error_excerpt.is_empty() {
                        truncate(&task.error_excerpt, 160)
                    } else if let Some(thread) = thread.filter(|t| t.status.busy()) {
                        truncate(&thread.phase, 160)
                    } else {
                        truncate(
                            task.result_excerpt
                                .lines()
                                .next()
                                .unwrap_or(task.state.id()),
                            160,
                        )
                    },
                    started_at: task.started_at,
                    finished_at: task.finished_at,
                    queue_position: self.task_queue_position(task),
                }
            })
            .collect()
    }

    pub fn attention(&self) -> Vec<AttentionItem> {
        let mut items = Vec::new();
        for thread in self.threads.iter().filter(|t| !t.archived()) {
            let task = self
                .tasks
                .iter()
                .find(|task| task.child_thread_id == thread.id);
            let kind = if let Some(request) = &thread.request {
                Some(if request["method"] == "confirm" {
                    AttentionKind::Approval
                } else {
                    AttentionKind::Question
                })
            } else if task.is_some_and(|task| task.state == TaskState::Interrupted) {
                Some(AttentionKind::Interrupted)
            } else if thread.status == Status::Failed
                || task.is_some_and(|task| task.state == TaskState::Failed)
            {
                Some(AttentionKind::Failed)
            } else if self.tasks.iter().any(|task| {
                self.task_home(task) == Some(thread.id)
                    && (task.state.finished() && !task.delivered
                        || !task.prior_deliveries.is_empty())
            }) {
                Some(AttentionKind::ReadyForReview)
            } else {
                None
            };
            if let Some(kind) = kind {
                items.push(AttentionItem {
                    thread_id: thread.id,
                    parent_id: thread.parent.map(|id| self.effective_home(id)),
                    kind,
                    short_title: truncate(&thread.title, 100),
                });
            }
        }
        items
    }

    /// User-only entry point. The response has the same shape as Thread::respond.
    pub fn respond_to_request(&mut self, thread_id: usize, response: Value) -> Result<(), String> {
        let server = self
            .remote_task(thread_id)
            .map(|r| r.server_id.as_str())
            .unwrap_or(&self.machines.selected);
        if let Some(client) = &self.client {
            if !client.supports_capability(server, crate::protocol::COMPOSER_REQUESTS) {
                return Err(crate::protocol::REQUESTS_UPDATE_REQUIRED.into());
            }
            if !client.can_approve(server) {
                return Err("This connection cannot answer requests.".into());
            }
        }
        if self
            .threads
            .iter()
            .find(|t| t.id == thread_id)
            .is_some_and(|t| t.responding_request.is_some())
        {
            return Err("A response to this request is already in flight.".into());
        }
        if let Some(remote) = self.remote_task(thread_id) {
            let reference = remote.request.as_ref().ok_or("This request has expired.")?;
            let commands = self
                .client
                .as_ref()
                .and_then(|c| c.commands_for(&remote.server_id))
                .ok_or_else(|| format!("Link this desktop to {} to answer.", remote.label))?;
            let command_id = commands.send(
                "request.respond",
                json!({"threadId":remote.thread_id,
                "requestId":reference["requestId"],"generation":reference["generation"],
                "requestContract":true,"response":response}),
            )?;
            if let Some(thread) = self.threads.iter_mut().find(|t| t.id == thread_id) {
                thread.responding_request = Some(command_id);
            }
            return Ok(());
        }
        if self.client.is_some() {
            self.threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                .ok_or("Unknown thread.")?
                .respond(response);
            return Ok(());
        }
        let thread = self
            .threads
            .iter_mut()
            .find(|t| t.id == thread_id && !t.archived())
            .ok_or("The thread is unavailable.")?;
        let request = thread.request.as_ref().ok_or("This request has expired.")?;
        let request_id = request["id"].clone();
        if thread.responding_request.is_some() {
            return Err("A response to this request is already in flight.".into());
        }
        match request["method"].as_str() {
            Some("confirm") => crate::approval::validate(request, &response)?,
            Some("select")
                if response["cancelled"] == true
                    || request["options"]
                        .as_array()
                        .is_some_and(|options| options.contains(&response["value"])) => {}
            Some("input" | "editor")
                if response["value"].is_string() || response["cancelled"] == true => {}
            _ => return Err("The response does not match the pending request.".into()),
        }
        if response["allowForSession"] == true && request["method"] != "confirm" {
            return Err("Session approval requires a confirmation request.".into());
        }
        thread.respond(response);
        if thread
            .request
            .as_ref()
            .is_some_and(|remaining| remaining["id"] == request_id)
        {
            return Err(thread
                .error
                .clone()
                .unwrap_or_else(|| "Could not send the response.".into()));
        }
        self.checkpoint();
        Ok(())
    }

    /// Explicit user action. An interrupted brief is never automatically replayed.
    pub fn resume_task(&mut self, task_id: &str) -> Result<(), String> {
        if let Some(result) = self.remote("task.resume", json!({"taskId":task_id})) {
            return result.map(|_| ());
        }
        let index = self
            .tasks
            .iter()
            .position(|task| task.id == task_id)
            .ok_or("There is no such task.")?;
        if self.tasks[index].recovering || self.tasks[index].state != TaskState::Interrupted {
            return Err("Only a reconciled interrupted task can be resumed.".into());
        }
        self.require_active_thread_owner(self.tasks[index].child_thread_id)?;
        let message = format!(
            "{RESUME_MESSAGE}\n\nOriginal brief for reference. Do not repeat completed actions:\n{}",
            self.tasks[index].brief
        );
        self.queue_task_attempt(index, &message)?;
        self.start_queued_tasks();
        self.checkpoint();
        Ok(())
    }

    /// Explicit user action. The cancellation result is durable and goes to the parent.
    pub fn dismiss_task(&mut self, task_id: &str) -> Result<(), String> {
        if let Some(result) = self.remote("task.dismiss", json!({"taskId":task_id})) {
            return result.map(|_| ());
        }
        let index = self
            .tasks
            .iter()
            .position(|task| task.id == task_id)
            .ok_or("There is no such task.")?;
        if self.tasks[index].state.finished() {
            return Err("This task has already finished.".into());
        }
        let child = self.tasks[index].child_thread_id;
        self.require_active_thread_owner(child)?;
        self.tasks[index].owe_remote_stop(None);
        if let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == child) {
            if self.tasks[index].remote.is_none() {
                thread.stop_task()?;
            }
            thread.disconnect();
            thread.task_result = Some("cancelled");
            thread.completion_notified = true;
            thread.recovering_task = false;
        }
        let task = &mut self.tasks[index];
        task.recovering = false;
        task.finish(TaskState::Cancelled, "", "The user dismissed this task.");
        task.hold_delivery = true;
        task.delivered = false;
        self.checkpoint();
        Ok(())
    }

    /// Explicitly delivers recovered results without using or replacing the composer draft.
    pub fn resume_task_results(&mut self, parent_thread_id: usize) -> Result<(), String> {
        if let Some(result) =
            self.remote("task.resumeResults", json!({"threadId":parent_thread_id}))
        {
            return result.map(|_| ());
        }
        self.require_active_thread_owner(parent_thread_id)?;
        if !self
            .threads
            .iter()
            .any(|t| t.id == parent_thread_id && !t.archived())
        {
            return Err("The parent thread is unavailable.".into());
        }
        self.release_task_results(parent_thread_id);
        self.poll_tasks(Instant::now());
        self.checkpoint();
        Ok(())
    }

    pub(super) fn release_task_results(&mut self, parent_thread_id: usize) {
        let parents: Vec<_> = self
            .threads
            .iter()
            .filter(|t| self.effective_home(t.id) == parent_thread_id)
            .map(|t| t.id)
            .collect();
        let routed: Vec<(bool, bool)> = self
            .tasks
            .iter()
            .map(|task| {
                (
                    self.task_home(task) == Some(parent_thread_id),
                    task.coordinator_captain.is_some(),
                )
            })
            .collect();
        for (index, task) in self.tasks.iter_mut().enumerate() {
            let (routed, captain_origin) = routed[index];
            if !routed && (captain_origin || !parents.contains(&task.parent_thread_id)) {
                continue;
            }
            task.hold_delivery = false;
            for delivery in &mut task.prior_deliveries {
                delivery.hold = false;
            }
        }
        self.release_crew_reports(parent_thread_id);
    }

    pub(super) fn check_task_capacity(&self, parent: usize) -> Result<(), String> {
        if self
            .tasks
            .iter()
            .filter(|task| {
                self.task_home(task) == Some(self.effective_home(parent)) && !task.state.finished()
            })
            .count()
            >= UNFINISHED_LIMIT
        {
            return Err("This captain already has 8 unfinished tasks. Finish or dismiss a task before delegating more work.".into());
        }
        Ok(())
    }

    pub(super) fn task_queue_position(&self, task: &Task) -> Option<usize> {
        (task.state == TaskState::Queued).then(|| {
            self.tasks
                .iter()
                .filter(|other| {
                    self.task_home(other) == self.task_home(task)
                        && other.state == TaskState::Queued
                })
                .position(|other| other.id == task.id)
                .unwrap_or(0)
                + 1
        })
    }

    fn delegate_scope(&self, caller_index: usize) -> DelegateScope {
        let thread = &self.threads[caller_index];
        match thread.captain.filter(|_| thread.captain_home) {
            Some(captain) => DelegateScope::Captain(captain),
            None => DelegateScope::Thread(thread.id),
        }
    }

    /// Tasks route to a Captain's current home, so the scope home also covers
    /// work launched from that Captain's earlier conversations.
    fn delegate_scope_home(&self, scope: DelegateScope) -> usize {
        match scope {
            DelegateScope::Captain(captain) => self.captain_home(captain).unwrap_or(usize::MAX),
            DelegateScope::Thread(id) => id,
        }
    }

    fn delegate_receipt_lookup(
        &self,
        scope: DelegateScope,
        key: &str,
    ) -> Result<Option<Receipt>, String> {
        let id = delegate_request_command_id(key);
        if let Some(database) = crate::database::for_path(&self.data_dir.join("index.json")) {
            return database.delegate_tool_receipt(&scope.device(), id);
        }
        Ok(self.command_receipt(&scope.device(), id).cloned())
    }

    /// True when a record already names this key, so remote preparation can skip
    /// another repository lookup for a replay or a rejection.
    pub(super) fn delegate_receipt_known(&self, caller_index: usize, key: &str) -> bool {
        let scope = self.delegate_scope(caller_index);
        let home = self.delegate_scope_home(scope);
        self.tasks
            .iter()
            .any(|task| self.task_home(task) == Some(home) && task_request_key(task) == Some(key))
            || self
                .delegate_receipt_lookup(scope, key)
                .ok()
                .flatten()
                .is_some()
    }

    /// A keyed call allocates only inside the runtime batch, so the receipt and
    /// the task commit or roll back together. Legacy index.json profiles keep
    /// the same contract through their committed receipt record.
    pub(super) fn delegate_receipt_ready(&self) -> Result<(), String> {
        if let Some(database) = crate::database::for_path(&self.data_dir.join("index.json")) {
            database.require_batch()?;
        }
        Ok(())
    }

    /// An identical retry replays the original task and thread identity while
    /// running, after completion, after deletion, after a home reset and after
    /// restart. A changed brief, target, workspace or selection under a used
    /// key fails clearly.
    pub(super) fn delegate_receipt_replay(
        &mut self,
        caller_index: usize,
        key: &str,
        args: &Value,
    ) -> Result<Option<Value>, String> {
        let scope = self.delegate_scope(caller_index);
        let caller = self.threads[caller_index].id;
        let intent = delegate_intent_hash(args);
        let home = self.delegate_scope_home(scope);
        let receipt = self.delegate_receipt_lookup(scope, key)?;
        if let Some(receipt) = &receipt
            && receipt.hash != intent
        {
            return Err(DELEGATE_KEY_MISMATCH.into());
        }
        let task = self
            .tasks
            .iter()
            .find(|task| self.task_home(task) == Some(home) && task_request_key(task) == Some(key))
            .cloned();
        if let Some(task) = task {
            if task_intent_hash(&task).as_deref() != Some(intent.as_str()) {
                return Err(DELEGATE_KEY_MISMATCH.into());
            }
            let child = task.child_thread_id;
            if self.threads.iter().any(|thread| thread.id == child) {
                let mut status =
                    self.mcp_call(caller, "task_status", &json!({"thread_id":child}))?;
                status["replayed"] = json!(true);
                if receipt.is_none() {
                    // Backfill tasks that predate durable delegation receipts.
                    let _ = self.record_delegate_receipt(caller_index, key, args, &status);
                }
                return Ok(Some(status));
            }
            if receipt.is_none() {
                // A durable task record without a live thread still names the work.
                let status = json!({"thread_id":task.child_thread_id,"task_id":task.id,
                    "status":task.state.id(),"caller":caller,"requestKey":key,
                    "replayed":true,"exists":false,"finished":true});
                let _ = self.record_delegate_receipt(caller_index, key, args, &status);
                return Ok(Some(status));
            }
        }
        if let Some(receipt) = receipt {
            if let Some(id) = receipt.result["thread_id"].as_u64().map(|id| id as usize)
                && self.threads.iter().any(|thread| thread.id == id)
            {
                let mut status = self.mcp_call(caller, "task_status", &json!({"thread_id":id}))?;
                status["replayed"] = json!(true);
                return Ok(Some(status));
            }
            let mut result = receipt.result;
            result["replayed"] = json!(true);
            result["exists"] = json!(false);
            result["finished"] = json!(true);
            return Ok(Some(result));
        }
        Ok(None)
    }

    /// The receipt commits in the same batch as the allocation, so a crash
    /// after commit but before the reply cannot launch the work a second time.
    pub(super) fn record_delegate_receipt(
        &mut self,
        caller_index: usize,
        key: &str,
        args: &Value,
        status: &Value,
    ) -> Result<(), String> {
        let scope = self.delegate_scope(caller_index);
        let command = crate::protocol::Command {
            command_id: delegate_request_command_id(key),
            method: "task.delegate".into(),
            params: delegate_intent(args),
        };
        let mut result = status.clone();
        result["requestKey"] = json!(key);
        result["caller"] = json!(self.threads[caller_index].id);
        if let Some(database) = crate::database::for_path(&self.data_dir.join("index.json")) {
            return database.stage_delegate_tool_receipt(&scope.device(), &command, result);
        }
        let device = scope.device();
        let hash = command.hash();
        if let Some(existing) = self
            .command_receipts
            .iter()
            .find(|receipt| receipt.device_id == device && receipt.command_id == command.command_id)
        {
            return if existing.hash == hash {
                Ok(())
            } else {
                Err(DELEGATE_KEY_MISMATCH.into())
            };
        }
        self.command_receipts.push(Receipt {
            command_id: command.command_id,
            device_id: device,
            hash,
            status: ReceiptStatus::Accepted,
            result,
            seq: self.server_seq,
        });
        Ok(())
    }

    /// A failed receipt write poisons the batch exactly like a Captain tool receipt.
    pub(super) fn fail_delegate_receipt(&mut self, error: String) -> String {
        if let Some(database) = crate::database::for_path(&self.data_dir.join("index.json")) {
            database.fail(error.clone());
        }
        self.storage_error = Some(error.clone());
        error
    }

    /// Control intent hashes everything the caller declared except the key:
    /// message, delivery mode, observed attempt and request fences. A changed
    /// argument under a used key fails rather than running a second operation.
    fn task_control_intent(args: &Value) -> Value {
        let mut intent = args.clone();
        if let Some(object) = intent.as_object_mut() {
            object.remove("client_request_id");
        }
        intent
    }

    fn task_control_command(tool: &str, key: &str, args: &Value) -> crate::protocol::Command {
        crate::protocol::Command {
            command_id: task_control_command_id(key),
            method: tool.into(),
            params: Self::task_control_intent(args),
        }
    }

    fn task_control_receipt_lookup(
        &self,
        scope: DelegateScope,
        key: &str,
    ) -> Result<Option<Receipt>, String> {
        let id = task_control_command_id(key);
        if let Some(database) = crate::database::for_path(&self.data_dir.join("index.json")) {
            return database.delegate_tool_receipt(&scope.device(), id);
        }
        Ok(self.command_receipt(&scope.device(), id).cloned())
    }

    /// An identical keyed send or cancel replays its committed receipt: the
    /// outcome the original operation recorded, plus replayed:true. The
    /// recorded acceptance, delivery and attempt never move, so a retry can
    /// never mistake newer state for its own operation. `exists` reports
    /// whether the task's thread is still present and `current` carries a
    /// fresh task_status when it is, so the caller can make a fresh decision.
    /// The replay never queues another input, starts another attempt or stops
    /// another attempt.
    pub(super) fn task_control_receipt_replay(
        &mut self,
        caller_index: usize,
        tool: &str,
        key: &str,
        args: &Value,
    ) -> Result<Option<Value>, String> {
        let scope = self.delegate_scope(caller_index);
        let caller = self.threads[caller_index].id;
        let intent = Self::task_control_command(tool, key, args).hash();
        let Some(receipt) = self.task_control_receipt_lookup(scope, key)? else {
            return Ok(None);
        };
        if receipt.hash != intent {
            return Err(CONTROL_KEY_MISMATCH.into());
        }
        let mut result = receipt.result;
        result["replayed"] = json!(true);
        let exists = args["thread_id"]
            .as_u64()
            .map(|id| id as usize)
            .is_some_and(|id| self.threads.iter().any(|thread| thread.id == id));
        result["exists"] = json!(exists);
        result["current"] = if let Some(id) = args["thread_id"].as_u64().filter(|_| exists) {
            self.mcp_call(caller, "task_status", &json!({"thread_id":id}))?
        } else {
            Value::Null
        };
        Ok(Some(result))
    }

    /// The control receipt commits in the same batch as the saved input or the
    /// owed stop, so a crash cannot lose which attempt the caller targeted.
    pub(super) fn record_task_control_receipt(
        &mut self,
        caller_index: usize,
        tool: &str,
        key: &str,
        args: &Value,
        status: &Value,
    ) -> Result<Value, String> {
        let scope = self.delegate_scope(caller_index);
        let command = Self::task_control_command(tool, key, args);
        let mut result = status.clone();
        result["requestKey"] = json!(key);
        result["caller"] = json!(self.threads[caller_index].id);
        result["control"] = json!({
            "saved":true,
            "target_attempt_id":args.get("attempt_id").unwrap_or(&status["attempt_id"]),
            "delivery":if tool == "task_cancel" {"cancel"}
                else if args["request_id"].is_string() {"answer"}
                else {args["delivery"].as_str().unwrap_or("follow_up")},
            "worker_acceptance":"not_observed",
            "completion":"not_observed"
        });
        if let Some(database) = crate::database::for_path(&self.data_dir.join("index.json")) {
            database
                .stage_task_control_receipt(&scope.device(), &command, result.clone())
                .map_err(|error| self.fail_delegate_receipt(error))?;
            return Ok(result);
        }
        let device = scope.device();
        let hash = command.hash();
        if let Some(existing) = self
            .command_receipts
            .iter()
            .find(|receipt| receipt.device_id == device && receipt.command_id == command.command_id)
        {
            return if existing.hash == hash {
                Ok(existing.result.clone())
            } else {
                Err(CONTROL_KEY_MISMATCH.into())
            };
        }
        self.command_receipts.push(Receipt {
            command_id: command.command_id,
            device_id: device,
            hash,
            status: ReceiptStatus::Accepted,
            result: result.clone(),
            seq: self.server_seq,
        });
        Ok(result)
    }

    fn queue_task_attempt(&mut self, index: usize, message: &str) -> Result<(), String> {
        let parent = self.tasks[index].parent_thread_id;
        if self.tasks[index].state.finished() {
            self.check_task_capacity(parent)?;
        }
        let child = self.tasks[index].child_thread_id;
        let thread = self
            .threads
            .iter_mut()
            .find(|t| t.id == child && !t.archived())
            .ok_or("Restore the child thread before resuming it.")?;
        thread.task_result = None;
        thread.completion_notified = false;
        thread.error = None;
        if thread.status == Status::Failed {
            thread.status = Status::Ready;
        }
        let task = &mut self.tasks[index];
        if task.state.finished() && !task.delivered {
            task.prior_deliveries.push(ResultDelivery {
                id: task.delivery_id.clone(),
                text: task.completion_message(),
                hold: task.hold_delivery,
            });
        }
        task.state = TaskState::Queued;
        task.delivered = false;
        task.delivery_id = format!("{}-result-{}", task.id, unique_id());
        let attempt = format!("{}-brief-{}", task.id, unique_id());
        task.attempt_id = attempt.clone();
        let child_thread_id = task.child_thread_id;
        task.attempt_message = delivery_text(
            &attempt,
            &format!("Shina: message from parent #{parent}\n\n{message}"),
        );
        task.result_excerpt.clear();
        task.result.clear();
        task.error_excerpt.clear();
        task.memory_suggestions.clear();
        task.started_at = None;
        task.finished_at = None;
        task.hold_delivery = false;
        if let Some(child) = self
            .threads
            .iter()
            .position(|thread| thread.id == child_thread_id)
        {
            self.request_reprovision(child);
        }
        Ok(())
    }

    /// An observed control fence must be a nonempty bounded string when given.
    /// Each operation decides which fields are mandatory.
    fn control_fence<'a>(args: &'a Value, field: &str) -> Result<Option<&'a str>, String> {
        args.get(field)
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_str()
                    .filter(|text| !text.is_empty() && text.len() <= 256)
                    .ok_or_else(|| format!("{field} must be a nonempty string."))
            })
            .transpose()
    }

    fn control_delivery<'a>(args: &'a Value) -> Result<Option<&'a str>, String> {
        let Some(delivery) = Self::control_fence(args, "delivery")? else {
            return Ok(None);
        };
        match delivery {
            "follow_up" | "steer" => Ok(Some(delivery)),
            _ => Err(format!(
                "Unknown delivery mode {delivery}. Use follow_up or steer."
            )),
        }
    }

    /// The task attempt a stale-observation error names for the retry.
    fn attempt_changed_error(task: &Task, observed: &str) -> String {
        format!(
            "The observed attempt {observed} is no longer current; this task now runs {}. Read task_status and retry with its attempt_id.",
            task.attempt_id
        )
    }

    /// A declared answer must land on the request it names. When the pending
    /// request expired or a newer one replaced it, the send fails closed rather
    /// than answering the wrong question or becoming silent follow-up work.
    fn check_request_fence(
        &self,
        task_index: usize,
        index: usize,
        request_id: Option<&str>,
        generation: Option<&str>,
    ) -> Result<(), String> {
        let (current_id, current_generation) = if let Some(remote) = &self.tasks[task_index].remote
        {
            (
                remote
                    .request
                    .as_ref()
                    .filter(|_| !self.tasks[task_index].state.finished())
                    .map(|request| request["requestId"].clone()),
                remote
                    .request
                    .as_ref()
                    .map(|request| request["generation"].clone())
                    .unwrap_or(Value::Null),
            )
        } else {
            (
                self.threads[index]
                    .request
                    .as_ref()
                    .map(|request| request["id"].clone()),
                json!(self.threads[index].request_generation),
            )
        };
        if current_id.is_none() {
            return Err("The pending request expired before this answer arrived. Read task_status for the task's current state.".into());
        }
        if let Some(request_id) = request_id
            && current_id.as_ref() != Some(&json!(request_id))
        {
            return Err(format!(
                "Request or runtime generation changed. Current request_id: {}; runtime_generation: {current_generation}. Read task_status before answering.",
                current_id.as_ref().unwrap_or(&Value::Null)
            ));
        }
        if let Some(generation) = generation
            && current_generation != json!(generation)
        {
            return Err(format!(
                "Request or runtime generation changed. Current request_id: {}; runtime_generation: {current_generation}. Read task_status before answering.",
                current_id.as_ref().unwrap_or(&Value::Null)
            ));
        }
        Ok(())
    }

    pub(super) fn prepare_task_control(
        &mut self,
        index: usize,
        caller: usize,
        tool: &str,
        args: &Value,
    ) -> Result<bool, String> {
        self.require_active_thread_owner(self.threads[index].id)?;
        let thread = &self.threads[index];
        if !self.owns_child(caller, index) {
            return Err(format!(
                "Only the parent that delegated thread {} may cancel or send to it.",
                thread.id
            ));
        }
        let attempt = Self::control_fence(args, "attempt_id")?;
        let request_id = Self::control_fence(args, "request_id")?;
        let generation = Self::control_fence(args, "runtime_generation")?;
        if tool != "task_cancel" && request_id.is_some() != generation.is_some() {
            return Err("A question answer requires both request_id and runtime_generation from task_status.".into());
        }
        let Some(task_index) = self
            .tasks
            .iter()
            .position(|task| task.child_thread_id == thread.id)
        else {
            if tool == "task_cancel" {
                return Err("This child has no durable task or attempt; a keyed cancel cannot be fenced. Stop it from Shina instead.".into());
            }
            let delivery = Self::control_delivery(args)?;
            if attempt.is_some() || delivery == Some("steer") {
                return Err("This child has no durable attempt to fence. Use Shina's thread controls instead.".into());
            }
            match thread.request.as_ref() {
                Some(request) => {
                    if request_id != request["id"].as_str()
                        || generation != Some(thread.request_generation.as_str())
                    {
                        return Err("Answering a pending request requires its current request_id and runtime_generation from task_status.".into());
                    }
                    if !matches!(
                        request["method"].as_str(),
                        Some("input" | "editor" | "select")
                    ) {
                        return Err("Confirmations require the user's device link.".into());
                    }
                }
                None if request_id.is_some() => {
                    return Err("The pending request expired before this answer arrived. Read task_status for the task's current state.".into());
                }
                None => {}
            }
            return Ok(false);
        };
        if tool == "task_cancel" {
            for field in ["delivery", "request_id", "runtime_generation", "message"] {
                if args.get(field).is_some_and(|value| !value.is_null()) {
                    return Err(format!("task_cancel does not accept {field}."));
                }
            }
            // An observed attempt is mandatory: a delayed cancel must never
            // stop work the caller did not name.
            let observed = attempt.ok_or(
                "task_cancel requires the attempt_id observed from task_status; read task_status and retry with its attempt_id.",
            )?;
            if observed != self.tasks[task_index].attempt_id {
                return Err(Self::attempt_changed_error(
                    &self.tasks[task_index],
                    observed,
                ));
            }
            self.tasks[task_index].owe_remote_stop(Some(observed.to_owned()));
            if self.tasks[task_index].state.finished() {
                return Ok(true);
            }
            if self.tasks[task_index].remote.is_none() {
                self.threads[index].stop_task()?;
            }
            self.threads[index].task_result = Some("cancelled");
            self.threads[index].completion_notified = true;
            self.tasks[task_index].recovering = false;
            self.threads[index].recovering_task = false;
            self.tasks[task_index].finish(TaskState::Cancelled, "", "Cancelled by the parent.");
            self.checkpoint();
            return Ok(true);
        }
        if self.tasks[task_index].recovering
            || self.tasks[task_index].state == TaskState::Interrupted
        {
            return Err("This task was interrupted by a restart. The user must resume or dismiss it in Shina after checking partial work.".into());
        }
        // A live pending request makes every send a declared answer: it must
        // name the request and generation it observed so a delayed answer
        // cannot resolve a newer question or silently become follow-up work.
        // A remote request is a snapshot of the target's live attempt; once
        // the attempt ends a send starts fresh work and no longer answers it.
        let request_pending = if let Some(remote) = &self.tasks[task_index].remote {
            !self.tasks[task_index].state.finished() && remote.request.is_some()
        } else {
            self.threads[index].request.is_some()
        };
        if request_pending && (request_id.is_none() || generation.is_none()) {
            return Err("task_status shows a pending request. Answering it requires request_id and runtime_generation from task_status.".into());
        }
        if request_id.is_some() || generation.is_some() {
            self.check_request_fence(task_index, index, request_id, generation)?;
        }
        let delivery = Self::control_delivery(args)?;
        if delivery == Some("steer") && attempt.is_none() {
            return Err("Steering requires the attempt_id observed from task_status.".into());
        }
        if let Some(observed) = attempt
            && observed != self.tasks[task_index].attempt_id
        {
            return Err(Self::attempt_changed_error(
                &self.tasks[task_index],
                observed,
            ));
        }
        // Preserve the existing input/editor/select response path. Confirmations remain user-only.
        if let Some(request) = &self.threads[index].request
            && self.tasks[task_index].remote.is_none()
        {
            // A declared steer must not silently become a question answer.
            if delivery == Some("steer") {
                return Err("This attempt cannot accept steering.".into());
            }
            // A fenced send naming a request the provider will not let Shina
            // answer fails rather than silently becoming follow-up input.
            if !matches!(
                request["method"].as_str(),
                Some("input" | "editor" | "select")
            ) {
                return Err("Confirmations require the user's device link.".into());
            }
            return Ok(false);
        }
        let message = args["message"]
            .as_str()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .ok_or("message must contain a correction or follow-up brief.")?;
        if self.tasks[task_index].remote.is_some() {
            if self.tasks[task_index]
                .remote
                .as_ref()
                .is_some_and(|remote| remote.stop_owed)
            {
                return Err(
                    "Wait for the target to acknowledge the owed stop before sending more work."
                        .into(),
                );
            }
            if delivery == Some("steer")
                && (self.tasks[task_index].state.finished()
                    || self.tasks[task_index]
                        .remote
                        .as_ref()
                        .is_some_and(|remote| remote.request.is_some()))
            {
                return Err("This attempt cannot accept steering.".into());
            }
            if !self.tasks[task_index].state.finished() {
                return self
                    .queue_remote_send(task_index, message, delivery.unwrap_or("follow_up"))
                    .map(|_| true);
            }
            let message_with_context = self.remote_task_message(task_index, message)?;
            self.queue_task_attempt(task_index, message)?;
            let attempt_id = self.tasks[task_index].attempt_id.clone();
            let remote = self.tasks[task_index].remote.as_mut().expect("remote task");
            remote.send_command = Some(crate::protocol::Command::new(
                "peer.thread.send",
                json!({"threadId":remote.thread_id,"message":message_with_context,
                    "delivery":"attempt","attemptId":attempt_id}),
            ));
            remote.state = RemoteState::Launching;
            remote.next_retry_at = 0;
            self.start_queued_tasks();
            return Ok(true);
        }
        if delivery == Some("steer") {
            let thread = &mut self.threads[index];
            if self.tasks[task_index].state.finished()
                || !thread.status.busy()
                || !thread.can_steer()
            {
                return Err("This attempt cannot accept steering.".into());
            }
            let text = delivery_text(
                &format!("{}-brief-{}", self.tasks[task_index].id, unique_id()),
                &format!("Shina: message from parent #{caller}\n\n{message}"),
            );
            self.tasks[task_index].attempt_message = text.clone();
            thread.pending_injections.push_back(Injection {
                text,
                shown: false,
                steer: thread.run_id.clone(),
            });
        } else if self.tasks[task_index].state.occupies_slot() {
            let text = delivery_text(
                &format!("{}-brief-{}", self.tasks[task_index].id, unique_id()),
                &format!("Shina: message from parent #{caller}\n\n{message}"),
            );
            self.tasks[task_index].attempt_message = text.clone();
            self.threads[index].pending_injections.push_back(Injection {
                text,
                shown: false,
                steer: None,
            });
        } else {
            let message = if self.tasks[task_index].state == TaskState::Queued {
                format!(
                    "{}\n\nParent correction:\n{message}",
                    self.tasks[task_index].brief
                )
            } else {
                message.to_owned()
            };
            self.queue_task_attempt(task_index, &message)?;
            self.start_queued_tasks();
        }
        self.checkpoint();
        Ok(true)
    }

    pub(super) fn start_queued_tasks(&mut self) -> bool {
        if self.network_blocked() {
            return false;
        }
        let mut changed = false;
        for index in 0..self.tasks.len() {
            let task = &self.tasks[index];
            if task.state != TaskState::Queued || task.recovering {
                continue;
            }
            if self
                .require_active_thread_owner(task.child_thread_id)
                .is_err()
            {
                continue;
            }
            if self
                .tasks
                .iter()
                .filter(|other| {
                    self.task_home(other) == self.task_home(task) && self.task_occupies_slot(other)
                })
                .count()
                >= RUNNING_LIMIT
            {
                continue;
            }
            let Some(child) = self
                .threads
                .iter()
                .position(|thread| thread.id == task.child_thread_id && !thread.archived())
            else {
                continue;
            };
            if self.threads[child].status.busy()
                || self.threads[child].phase == "Stopping"
                || self.threads[child]
                    .placement
                    .as_ref()
                    .is_some_and(|placement| placement.phase != placement::PlacementPhase::Ready)
                || (self.threads[child].machine == "local"
                    && self
                        .provider_update_drivers
                        .contains(&self.threads[child].harness))
            {
                continue;
            }
            self.tasks[index].state = TaskState::Starting;
            self.tasks[index].started_at = Some(lifecycle::now());
            if self.tasks[index].remote.is_some() {
                changed = true;
                continue;
            }
            // Persist intent before any prompt can reach Pi. Failure leaves work stopped.
            if self.runtime_enabled
                && let Err(error) = self.checkpoint_and_flush()
            {
                self.tasks[index].state = TaskState::Interrupted;
                self.tasks[index].error_excerpt = excerpt(&error, 2000);
                self.storage_error = Some(error);
                continue;
            }
            let thread = &mut self.threads[child];
            thread.arm_task();
            thread.pending_injections.push_back(Injection {
                text: self.tasks[index].attempt_message.clone(),
                shown: false,
                steer: None,
            });
            self.connect_thread(child);
            changed = true;
        }
        changed
    }

    fn task_occupies_slot(&self, task: &Task) -> bool {
        !task.recovering
            && (task.state.occupies_slot()
                || self
                    .threads
                    .iter()
                    .any(|thread| thread.id == task.child_thread_id && thread.status.busy()))
    }

    pub(super) fn restore_tasks(
        &mut self,
        index: &Value,
        project_remap: &std::collections::HashMap<usize, usize>,
    ) -> Result<(), String> {
        let mut tasks: Vec<Task> = match index.get("tasks") {
            None => Vec::new(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|error| format!("Invalid saved tasks: {error}."))?,
        };
        if let Some(records) = index["tasks"].as_array() {
            for (task, record) in tasks.iter_mut().zip(records.iter()) {
                if record.get("coordinator_captain").is_none() {
                    task.coordinator_captain = self
                        .threads
                        .iter()
                        .find(|thread| {
                            thread.id == task.parent_thread_id
                                && thread.captain_home
                                && thread.captain.is_some()
                        })
                        .and_then(|thread| thread.captain);
                }
            }
        }
        let mut ids = std::collections::HashSet::new();
        let mut children = std::collections::HashSet::new();
        let mut deliveries = std::collections::HashSet::new();
        for task in &mut tasks {
            if task.attempt_id.is_empty() {
                // Indexes that predate attempt fencing reuse the saved attempt
                // marker. A busy-time correction's marker is equally unique.
                task.attempt_id = delivery_id(&task.attempt_message)
                    .unwrap_or(task.id.as_str())
                    .to_owned();
            }
            if let Some(remote) = &task.remote
                && (uuid::Uuid::parse_str(&remote.server_id).is_err()
                    || remote.launch_command.method != "peer.thread.launch"
                    || remote.launch_command.params["origin"]["taskId"] != task.id
                    || remote.stop_owed && remote.stop_command_id.is_none()
                    || remote.thread_id == Some(0)
                    || remote.label.len() > 256
                    || remote
                        .client_request_id
                        .as_ref()
                        .is_some_and(|id| id.is_empty() || id.len() > 256)
                    || remote
                        .stop_attempt_id
                        .as_deref()
                        .is_some_and(|attempt| !valid_id(attempt))
                    || remote.send_command.as_ref().is_some_and(|command| {
                        command.method != "peer.thread.send"
                            || command.params["threadId"] != json!(remote.thread_id)
                            || command.params["delivery"].as_str().is_some_and(|delivery| {
                                !matches!(delivery, "follow_up" | "steer" | "attempt")
                            })
                            || command.params["attemptId"]
                                .as_str()
                                .is_some_and(|attempt| !valid_id(attempt))
                    }))
            {
                return Err("The saved index contains an invalid remote task.".into());
            }
            if let Some(target) = &task.model_target {
                parse_delegation_target(&json!({"target":target}))?;
            }
            if let Some(project) = task.project_id {
                task.project_id =
                    Some(*project_remap.get(&project).ok_or("Invalid task project.")?);
            }
            if task.id.is_empty()
                || task.delivery_id.is_empty()
                || !ids.insert(task.id.clone())
                || !children.insert(task.child_thread_id)
                || !deliveries.insert(task.delivery_id.clone())
                || !valid_id(&task.delivery_id)
                || !valid_id(&task.attempt_id)
                || delivery_id(&task.attempt_message).is_none_or(|id| !valid_id(id))
                || task.result_excerpt.chars().count() > 6000
                || task.error_excerpt.chars().count() > 2000
                || task.memory_suggestions.len() > 256
                || serde_json::to_vec(&task.memory_suggestions)
                    .expect("suggestion serialization")
                    .len()
                    > memory::PROPOSAL_BUDGET
                || task
                    .memory_suggestions
                    .iter()
                    .any(|text| text.chars().count() > 500)
                || task.state.finished() != task.finished_at.is_some()
                || task.delivered && !task.state.finished()
                || task
                    .coordinator_captain
                    .is_some_and(|captain| captain >= self.captains.len())
                || !self.threads.iter().any(|t| {
                    t.id == task.child_thread_id
                        && t.parent == Some(task.parent_thread_id)
                        && t.machine == task.machine
                        && t.project == task.project_id
                })
                || !self.threads.iter().any(|t| t.id == task.parent_thread_id)
            {
                return Err("The saved index contains invalid or duplicate tasks.".into());
            }
            for delivery in &task.prior_deliveries {
                if !valid_id(&delivery.id)
                    || !deliveries.insert(delivery.id.clone())
                    || delivery_id(&delivery.text) != Some(delivery.id.as_str())
                    || delivery.text.chars().count() > 20_000
                {
                    return Err("The saved index contains an invalid completion delivery.".into());
                }
            }
        }
        // Old indexes keep their crew, but contain no proof of acceptance or completion.
        for thread in self
            .threads
            .iter()
            .filter(|t| t.delegated && t.peer_origin.is_none() && !children.contains(&t.id))
        {
            let mut task = Task::new(thread, "", None);
            task.state = TaskState::Interrupted;
            task.error_excerpt = "This legacy task has no saved brief or completion record. Check its history before resuming.".into();
            tasks.push(task);
        }
        self.tasks = tasks;
        Ok(())
    }

    pub(super) fn prepare_task_recovery(&mut self) {
        for task in &mut self.tasks {
            if let Some(remote) = &mut task.remote {
                remote.next_retry_at = 0;
            }
            for delivery in &mut task.prior_deliveries {
                delivery.hold = true;
            }
            if task.state.finished() && task.delivered {
                continue;
            }
            task.hold_delivery = true;
            if task.remote.is_some() {
                continue;
            }
            if self.threads.iter().any(|thread| {
                thread.id == task.child_thread_id
                    && thread.placement.as_ref().is_some_and(|placement| {
                        placement.phase != placement::PlacementPhase::Ready
                    })
            }) {
                // The worker never reached a provider. Leave it queued so its
                // recorded reservation provisions again after the restart.
                continue;
            }
            task.recovering = true;
            if let Some(thread) = self
                .threads
                .iter_mut()
                .find(|thread| thread.id == task.child_thread_id)
            {
                if thread.session.is_some() {
                    thread.recovering_task = true;
                } else {
                    task.reconcile(&json!({"success":true,"data":{"messages":[]}}));
                }
            }
        }
        self.prepare_crew_report_recovery();
    }

    pub(super) fn start_task_recovery(&mut self) {
        let children: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.recovering_task)
            .map(|(i, _)| i)
            .collect();
        for child in children {
            self.connect_thread(child);
        }
    }

    pub(super) fn poll_tasks(&mut self, now: Instant) -> bool {
        let mut changed = self.poll_crew_reports();
        changed |= self.poll_remote_tasks();
        let routes: Vec<Option<usize>> =
            self.tasks.iter().map(|task| self.task_home(task)).collect();
        for task in &mut self.tasks {
            if task.remote.is_some() {
                continue;
            }
            let Some(thread) = self
                .threads
                .iter_mut()
                .find(|t| t.id == task.child_thread_id)
            else {
                continue;
            };
            if task.recovering {
                if let Some(history) = thread.task_history.take() {
                    task.reconcile(&history);
                    thread.recovering_task = false;
                    // Recover history only. Never continue an agent turn at boot.
                    thread.disconnect();
                    thread.task_result = None;
                    thread.completion_notified = true;
                    changed = true;
                } else if thread.status == Status::Failed {
                    task.reconcile(&json!({"success":false}));
                    thread.recovering_task = false;
                    changed = true;
                }
                continue;
            }
            if !thread.completion_notified
                && let Some(result) = thread.task_result
            {
                if task.state == TaskState::Queued {
                    // The settled turn's result belongs to the previous
                    // attempt; a queued retry must not inherit it.
                    thread.task_result = None;
                    thread.completion_notified = true;
                    continue;
                }
                // A correction may still be local, queued by the provider, or
                // running in its next turn. The previous turn cannot finish it.
                if thread.pi.is_some()
                    && thread.status != Status::Failed
                    && (!thread.pending_injections.is_empty()
                        || !matches!(result, "cancelled" | "failed")
                            && (thread.status.busy() || !thread.native.follow_ups.is_empty()))
                {
                    thread.arm_task();
                    continue;
                }
                thread.completion_notified = true;
                let reply = thread
                    .messages
                    .iter()
                    .rposition(|message| message.text == task.attempt_message)
                    .and_then(|brief| {
                        thread.messages[brief + 1..]
                            .iter()
                            .rev()
                            .find(|m| m.role == "Pi")
                    })
                    .map(|m| m.text.as_str())
                    .unwrap_or("");
                task.finish(
                    match result {
                        "cancelled" => TaskState::Cancelled,
                        "failed" => TaskState::Failed,
                        _ => TaskState::Completed,
                    },
                    reply,
                    thread.error.as_deref().unwrap_or(""),
                );
                task.memory_suggestions = std::mem::take(&mut thread.memory_suggestions);
                changed = true;
            } else if task.state == TaskState::Starting
                && thread.injection_id.is_none()
                && thread.pending_injections.is_empty()
                && thread.status.busy()
            {
                task.state = TaskState::Running;
                changed = true;
            }
        }
        changed |= self.reconcile_task_acceptances();
        // Save outcomes before handing the result to another process.
        if changed {
            self.checkpoint();
        }
        for index in 0..self.tasks.len() {
            let task = &self.tasks[index];
            let mut messages: Vec<_> = task
                .prior_deliveries
                .iter()
                .filter(|delivery| !delivery.hold)
                .map(|delivery| delivery.text.clone())
                .collect();
            if !task.delivered && task.state.finished() && !task.hold_delivery && !task.recovering {
                messages.push(task.completion_message());
            }
            if messages.is_empty() {
                continue;
            }
            let captain_origin = task.coordinator_captain.is_some();
            let Some(parent) = (if captain_origin {
                routes[index].and_then(|home| {
                    self.threads
                        .iter()
                        .position(|t| t.id == home && !t.archived())
                })
            } else {
                routes[index]
                    .and_then(|home| {
                        self.threads
                            .iter()
                            .position(|t| t.id == home && !t.archived())
                    })
                    .or_else(|| {
                        self.threads
                            .iter()
                            .position(|t| t.id == task.parent_thread_id && !t.archived())
                    })
            }) else {
                continue;
            };
            let mut enqueued = false;
            for text in messages {
                if !self.threads[parent]
                    .pending_injections
                    .iter()
                    .any(|i| i.text == text)
                {
                    self.threads[parent]
                        .pending_injections
                        .push_back(Injection {
                            text,
                            shown: false,
                            steer: None,
                        });
                    changed = true;
                    enqueued = true;
                }
            }
            // One connection attempt per newly queued result. A failed connection waits
            // for a user retry, rather than spawning another process on every frame.
            if enqueued
                && self.threads[parent].pi.is_none()
                && self.threads[parent].status != Status::Failed
            {
                self.connect_thread(parent);
            }
        }
        changed |= self.start_queued_tasks();
        changed | self.poll_legacy_tasks(now)
    }

    /// Settles delivery records against the acceptance evidence threads observed. Ordinary
    /// tasks accept evidence from the current effective home or the original parent.
    /// Captain-origin tasks accept evidence in every Captain-home thread, including earlier
    /// homes after a reset, plus the original parent; evidence does not route delivery.
    pub(super) fn reconcile_task_acceptances(&mut self) -> bool {
        let routes: Vec<Option<usize>> =
            self.tasks.iter().map(|task| self.task_home(task)).collect();
        let mut changed = false;
        for (index, task) in self.tasks.iter_mut().enumerate() {
            let captain_origin = task.coordinator_captain.is_some();
            let accepted: Vec<_> = self
                .threads
                .iter()
                .filter(|t| {
                    if captain_origin {
                        t.captain_home || t.id == task.parent_thread_id
                    } else {
                        Some(t.id) == routes[index] || t.id == task.parent_thread_id
                    }
                })
                .flat_map(|t| t.accepted_deliveries.iter())
                .collect();
            let before = task.prior_deliveries.len();
            task.prior_deliveries
                .retain(|delivery| !accepted.contains(&&delivery.id));
            changed |= before != task.prior_deliveries.len();
            if !task.delivered && task.state.finished() && accepted.contains(&&task.delivery_id) {
                task.delivered = true;
                changed = true;
            }
        }
        changed
    }

    /// A task record for demo scenes and layout tests. It never starts a process: the child
    /// must already exist as a delegated thread, and `started_ago` is in milliseconds.
    /// Marks a demo task as stopped by a restart, as recovery does for a task that was running.
    pub fn demo_stop(&mut self, child_id: usize) {
        if let Some(task) = self
            .tasks
            .iter_mut()
            .find(|t| t.child_thread_id == child_id)
        {
            task.state = TaskState::Interrupted;
        }
    }
    pub fn demo_task(
        &mut self,
        child_id: usize,
        state: TaskState,
        started_ago: Option<u64>,
        reply: &str,
        error: &str,
    ) {
        let Some(thread) = self.threads.iter().find(|t| t.id == child_id) else {
            return;
        };
        let mut task = Task::new(thread, &thread.title.clone(), None);
        let now = lifecycle::now();
        task.started_at = started_ago.map(|ago| now.saturating_sub(ago));
        if state.finished() {
            task.finish(state, reply, error);
            task.delivered = true;
        } else {
            task.state = state;
            task.error_excerpt = error.into();
        }
        self.tasks.push(task);
    }
}

impl Task {
    /// A caller-observed attempt fences the owed stop; a home-authoritative
    /// stop passes None and lands unconditionally. The first observed fence is
    /// kept because the outstanding stop command must never change its params
    /// under the same command id.
    pub(super) fn owe_remote_stop(&mut self, attempt: Option<String>) {
        if let Some(remote) = &mut self.remote {
            if self.state == TaskState::Queued
                && self.started_at.is_none()
                && remote.thread_id.is_none()
            {
                remote.state = RemoteState::Done;
                return;
            }
            remote.stop_owed = true;
            remote
                .stop_command_id
                .get_or_insert_with(uuid::Uuid::new_v4);
            if remote.stop_attempt_id.is_none() {
                remote.stop_attempt_id = attempt;
            }
            remote.next_retry_at = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_task() -> State {
        let mut state = State::new(PathBuf::new(), false);
        let mut thread = Thread::new(2);
        thread.parent = Some(1);
        thread.delegated = true;
        thread.title = "Durable fixture".into();
        let mut task = Task::new(&thread, "Inspect the files, then finish.", None);
        task.state = TaskState::Running;
        task.started_at = Some(task.created_at);
        thread.arm_task();
        state.next_id = 3;
        state.threads.push(thread);
        state.tasks.push(task);
        state
    }

    fn history(task: &Task, tail: Vec<Value>) -> Value {
        let mut messages = vec![json!({"role":"user","content":task.attempt_message})];
        messages.extend(tail);
        json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":messages}})
    }

    fn fake_pi(thread: &mut Thread) -> (mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
        let (pi, commands, events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.model_provider = "cpa".into();
        (commands, events)
    }

    #[test]
    fn durable_task_round_trip_and_indexes_without_tasks_migrate() {
        let mut state = state_with_task();
        state.load_global_for_test();
        // load_global leaves the in-memory fixture when no index exists.
        state.tasks[0].finish(TaskState::Completed, "reply", "");
        state.checkpoint_and_flush().unwrap();
        let index = crate::store::load(&state.data_dir.join("index.json"))
            .unwrap()
            .unwrap();
        assert_eq!(index["tasks"][0]["result_excerpt"], "reply");
        assert!(index["tasks"][0]["created_at"].as_u64().unwrap() > 1_000_000_000_000);
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&index).unwrap();
        assert_eq!(restored.tasks[0].id, state.tasks[0].id);
        assert_eq!(restored.tasks[0].delivery_id, state.tasks[0].delivery_id);
        assert_eq!(
            restored.tasks[0].attempt_message,
            state.tasks[0].attempt_message
        );
        assert_eq!(restored.tasks[0].state, TaskState::Completed);
        let mut legacy = index;
        legacy.as_object_mut().unwrap().remove("tasks");
        restored.restore_global(&legacy).unwrap();
        assert_eq!(restored.tasks[0].state, TaskState::Interrupted);
        assert!(restored.tasks[0].error_excerpt.contains("legacy"));
    }

    #[test]
    fn invalid_task_records_block_saving_without_overwriting_the_index() {
        let state = state_with_task();
        let mut snapshot = state.snapshot();
        snapshot["tasks"][0]["state"] = json!("made-up");
        let path = state.data_dir.join("index.json");
        std::fs::write(&path, snapshot.to_string()).unwrap();
        let mut restored = State::new(PathBuf::new(), false);
        restored.load_global_for_test();
        assert!(restored.store.is_none());
        assert!(restored.storage_error.is_some());
        restored.checkpoint();
        assert_eq!(crate::store::load(&path).unwrap().unwrap(), snapshot);
    }

    #[test]
    fn reconciliation_requires_an_accepted_brief_and_a_proven_final_reply() {
        let state = state_with_task();
        for (tail, expected) in [
            (
                vec![json!({"role":"assistant","stopReason":"stop","content":"Finished safely."})],
                TaskState::Completed,
            ),
            (
                vec![
                    json!({"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","id":"x","name":"write"}]}),
                    json!({"role":"toolResult","content":"wrote a file"}),
                ],
                TaskState::Interrupted,
            ),
            (
                vec![json!({"role":"assistant","content":"Maybe finished"})],
                TaskState::Interrupted,
            ),
            (vec![], TaskState::Interrupted),
            (
                vec![json!({"role":"assistant","stopReason":"error","content":"Failed"})],
                TaskState::Interrupted,
            ),
            (
                vec![
                    json!({"role":"assistant","stopReason":"stop","content":[{"type":"toolCall","id":"x","name":"write"}]}),
                ],
                TaskState::Interrupted,
            ),
            (
                vec![
                    json!({"role":"assistant","stopReason":"stop","content":"done"}),
                    json!({"role":"user","content":"another task"}),
                ],
                TaskState::Interrupted,
            ),
        ] {
            let mut task = state.tasks[0].clone();
            task.reconcile(&history(&task, tail));
            assert_eq!(task.state, expected);
        }
        let mut task = state.tasks[0].clone();
        task.reconcile(&json!({"success":true,"data":{"messages":[{"role":"assistant","stopReason":"stop","content":"Unrelated old reply"}]}}));
        assert_eq!(task.state, TaskState::Interrupted);
        assert!(task.error_excerpt.contains("acceptance"));
        task.reconcile(&json!({"success":false}));
        assert_eq!(task.state, TaskState::Interrupted);
        let mut task = state.tasks[0].clone();
        let mut record = history(
            &task,
            vec![
                json!({"role":"assistant","stopReason":"stop","timestamp":1700000000123u64,"content":"Done"}),
            ],
        );
        task.reconcile(&record);
        assert_eq!(task.finished_at, Some(1700000000123));
        record["data"]["messages"][1]["timestamp"] = json!(1800000000123u64);
        task.reconcile(&record);
        assert_eq!(
            task.finished_at,
            Some(1700000000123),
            "restarts do not change a saved finish time"
        );
    }

    #[test]
    fn reconciliation_uses_the_active_get_entries_branch() {
        let state = state_with_task();
        let mut record = json!({"type":"response","id":"history","command":"get_entries","success":true,"data":{"leafId":"tool","entries":[
            {"id":"brief","parentId":null,"type":"message","message":{"role":"user","content":state.tasks[0].attempt_message}},
            {"id":"done-on-other-branch","parentId":"brief","type":"message","message":{"role":"assistant","stopReason":"stop","content":"Done"}},
            {"id":"tool","parentId":"brief","type":"message","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","name":"write"}]}}
        ]}});
        session_history(&mut record);
        let mut task = state.tasks[0].clone();
        task.reconcile(&record);
        assert_eq!(task.state, TaskState::Interrupted);
        record = json!({"type":"response","id":"history","command":"get_entries","success":true,"data":{"leafId":"missing","entries":[]}});
        session_history(&mut record);
        task.reconcile(&record);
        assert_eq!(task.state, TaskState::Interrupted);
    }

    #[test]
    fn recovered_results_wait_for_a_user_turn_and_dedupe_from_parent_history() {
        let mut state = state_with_task();
        state.demo = true; // No actual process can start.
        state.tasks[0].finish(TaskState::Completed, "Recovered reply", "");
        state.tasks[0].hold_delivery = true;
        let (commands, _events) = fake_pi(&mut state.threads[0]);
        state.poll_tasks(Instant::now());
        assert!(commands.try_recv().is_err());
        assert!(state.threads[0].pending_injections.is_empty());
        assert_eq!(state.attention()[0].kind, AttentionKind::ReadyForReview);
        let text = state.tasks[0].completion_message();
        // Accepted before the crash, but the ACK and index save were lost.
        state.threads[0].apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[{"role":"user","content":text}]}}));
        state.resume_task_results(1).unwrap();
        assert!(state.tasks[0].delivered);
        assert!(
            commands
                .try_iter()
                .all(|command| command["type"] != "prompt")
        );
    }

    #[test]
    fn delivery_ack_updates_the_durable_flag_not_the_optimistic_transcript() {
        let mut state = state_with_task();
        state.demo = true;
        state.tasks[0].finish(TaskState::Completed, &"R".repeat(7000), &"E".repeat(3000));
        assert_eq!(state.tasks[0].result_excerpt.chars().count(), 6000);
        assert_eq!(state.tasks[0].error_excerpt.chars().count(), 2000);
        state.threads[1].completion_notified = true;
        let (commands, _events) = fake_pi(&mut state.threads[0]);
        state.threads[0].draft.replace_text("Keep my draft");
        state.poll_tasks(Instant::now());
        let prompt = commands.try_recv().unwrap();
        assert_eq!(prompt["type"], "prompt");
        assert!(!state.tasks[0].delivered);
        state.threads[0]
            .apply(json!({"type":"response","id":prompt["id"],"command":"prompt","success":true}));
        state.poll_tasks(Instant::now());
        assert!(state.tasks[0].delivered);
        assert_eq!(state.threads[0].draft.text, "Keep my draft");
        assert!(
            commands
                .try_iter()
                .all(|command| command["type"] != "prompt")
        );
    }

    #[test]
    fn a_follow_up_preserves_the_previous_undelivered_result_across_restart() {
        let mut state = state_with_task();
        state.demo = true;
        state.tasks[0].finish(TaskState::Completed, "First result", "");
        let previous_id = state.tasks[0].delivery_id.clone();
        state.queue_task_attempt(0, "Now do another task").unwrap();
        assert_eq!(state.tasks[0].prior_deliveries.len(), 1);
        let snapshot = state.snapshot();
        let mut restored = State::new(PathBuf::new(), false);
        restored.demo = true;
        restored.restore_global(&snapshot).unwrap();
        restored.prepare_task_recovery();
        assert_eq!(restored.tasks[0].state, TaskState::Interrupted);
        assert!(restored.tasks[0].prior_deliveries[0].hold);
        let (commands, _events) = fake_pi(&mut restored.threads[0]);
        restored.resume_task_results(1).unwrap();
        let prompt = commands.try_recv().unwrap();
        assert!(prompt["message"].as_str().unwrap().contains("First result"));
        assert!(prompt["message"].as_str().unwrap().contains(&previous_id));
        restored.threads[0]
            .apply(json!({"type":"response","id":prompt["id"],"command":"prompt","success":true}));
        restored.poll_tasks(Instant::now());
        assert!(restored.tasks[0].prior_deliveries.is_empty());
        assert!(
            commands
                .try_iter()
                .all(|command| command["type"] != "prompt")
        );
    }

    #[test]
    fn three_children_start_and_eight_unfinished_tasks_are_the_limit() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        for _ in 0..8 {
            state
                .mcp_call(1, "delegate_task", &json!({"prompt":"A bounded task"}))
                .unwrap();
        }
        assert_eq!(
            state
                .tasks
                .iter()
                .filter(|task| task.state == TaskState::Starting)
                .count(),
            3
        );
        assert_eq!(state.tasks_for(1)[3].queue_position, Some(1));
        let child = state.tasks[7].child_thread_id;
        let status = state
            .mcp_call(1, "task_status", &json!({"thread_id":child}))
            .unwrap();
        assert_eq!(status["queue_position"], 5);
        assert!(
            state
                .mcp_call(1, "delegate_task", &json!({"prompt":"Too many"}))
                .unwrap_err()
                .contains("8 unfinished")
        );
        let attempt = state
            .mcp_call(1, "task_status", &json!({"thread_id":2}))
            .unwrap()["attempt_id"]
            .clone();
        state
            .mcp_call(
                1,
                "task_cancel",
                &json!({"thread_id":2,"client_request_id":"cancel-2","attempt_id":attempt}),
            )
            .unwrap();
        state.poll_tasks(Instant::now());
        assert_eq!(state.tasks[3].state, TaskState::Starting);
        assert_eq!(state.tasks_for(1)[4].queue_position, Some(1));
        state.tasks[7].state = TaskState::Interrupted;
        assert!(!state.tasks[7].state.finished());
    }

    #[test]
    fn keyed_sends_replay_without_delivering_twice() {
        let mut state = state_with_task();
        let args = json!({"thread_id":2,"message":"More context","client_request_id":"send-1"});
        let first = state.mcp_call(1, "task_send", &args).unwrap();
        assert_eq!(state.threads[1].pending_injections.len(), 1);
        assert_eq!(first["replayed"], Value::Null);
        let replay = state.mcp_call(1, "task_send", &args).unwrap();
        assert_eq!(replay["replayed"], true);
        assert_eq!(replay["attempt_id"], first["attempt_id"]);
        assert_eq!(
            state.threads[1].pending_injections.len(),
            1,
            "a replay never queues the same input again"
        );
        let changed = state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"Different","client_request_id":"send-1"}),
            )
            .unwrap_err();
        assert!(
            changed.contains("different send or cancel arguments"),
            "{changed}"
        );
        assert!(state.threads[1].pending_injections.len() == 1);
        let missing = state
            .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"x"}))
            .unwrap_err();
        assert!(missing.contains("client_request_id"), "{missing}");
    }

    #[test]
    fn keyed_controls_replay_after_a_restart() {
        let directory =
            std::env::temp_dir().join(format!("shina-control-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut state = state_with_task();
        state.data_dir = directory.clone();
        state.load_global_for_test();
        // Loading a live profile fences mid-run work for user review; the
        // fixture resumes it so the keyed send can commit its receipt.
        state.tasks[0].state = TaskState::Running;
        state.tasks[0].recovering = false;
        state.threads[1].recovering_task = false;
        let args = json!({"thread_id":2,"message":"Persist me","client_request_id":"send-boot"});
        state.mcp_call(1, "task_send", &args).unwrap();
        state.checkpoint_and_flush().unwrap();
        let index = crate::store::load(&state.data_dir.join("index.json"))
            .unwrap()
            .unwrap();
        let mut restored = State::new(state.data_dir.clone(), false);
        restored.restore_global(&index).unwrap();
        let replay = restored.mcp_call(1, "task_send", &args).unwrap();
        assert_eq!(replay["replayed"], true);
        assert!(
            restored.threads[1]
                .pending_injections
                .iter()
                .filter(|injection| injection.text.contains("Persist me"))
                .count()
                <= 1,
            "a replayed send never queues the input again"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn keyed_control_receipts_survive_a_captain_home_reset() {
        let mut state = state_with_task();
        // The caller is a Captain home, so its control receipts are scoped to
        // the Captain rather than the thread that happens to host it.
        state.captains = Captain::presets();
        state.threads[0].captain = Some(0);
        state.threads[0].captain_home = true;
        state.captain_homes.insert(0, 1);
        let send = json!({"thread_id":2,"message":"Scoped work","client_request_id":"home-send"});
        let first = state.mcp_call(1, "task_send", &send).unwrap();
        let attempt = first["attempt_id"].clone();
        assert_eq!(state.threads[1].pending_injections.len(), 1);

        // The reset retires home 1 and opens a fresh home for the same Captain.
        state.reset_captain_conversation(0).unwrap();
        let home = state.captain_home(0).unwrap();
        assert_ne!(home, 1);

        // The same keyed send replays the committed receipt instead of
        // queueing the input a second time.
        let replay = state.mcp_call(home, "task_send", &send).unwrap();
        assert_eq!(replay["replayed"], true, "{replay}");
        assert_eq!(replay["attempt_id"], attempt);
        assert_eq!(
            state.threads[1].pending_injections.len(),
            1,
            "a replayed send never queues the input again"
        );

        // A keyed cancel issued from the new home stops the named attempt,
        // and its own retry replays the receipt without another stop.
        let cancel = json!({"thread_id":2,"client_request_id":"home-cancel","attempt_id":attempt});
        state.mcp_call(home, "task_cancel", &cancel).unwrap();
        assert_eq!(state.tasks[0].state, TaskState::Cancelled);
        let replay = state.mcp_call(home, "task_cancel", &cancel).unwrap();
        assert_eq!(replay["replayed"], true, "{replay}");
        assert_eq!(replay["status"], "cancelled");
        assert_eq!(state.tasks[0].state, TaskState::Cancelled);
    }

    #[test]
    fn a_stale_attempt_fence_never_cancels_newer_work() {
        let mut state = state_with_task();
        let status = state
            .mcp_call(1, "task_status", &json!({"thread_id":2}))
            .unwrap();
        let attempt_a = status["attempt_id"].as_str().unwrap().to_owned();
        state
            .mcp_call(
                1,
                "task_cancel",
                &json!({"thread_id":2,"client_request_id":"cancel-1","attempt_id":attempt_a}),
            )
            .unwrap();
        let replay = state
            .mcp_call(
                1,
                "task_cancel",
                &json!({"thread_id":2,"client_request_id":"cancel-1","attempt_id":attempt_a}),
            )
            .unwrap();
        assert_eq!(replay["replayed"], true);
        // New work under a new key starts attempt B.
        state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"Again","client_request_id":"send-2"}),
            )
            .unwrap();
        let status = state
            .mcp_call(1, "task_status", &json!({"thread_id":2}))
            .unwrap();
        let attempt_b = status["attempt_id"].as_str().unwrap().to_owned();
        assert_ne!(attempt_a, attempt_b);
        // The delayed cancel for attempt A must not land on attempt B.
        let error = state
            .mcp_call(
                1,
                "task_cancel",
                &json!({"thread_id":2,"client_request_id":"cancel-2","attempt_id":attempt_a}),
            )
            .unwrap_err();
        assert!(error.contains("attempt"), "{error}");
        assert!(!state.tasks[0].state.finished());
        state
            .mcp_call(
                1,
                "task_cancel",
                &json!({"thread_id":2,"client_request_id":"cancel-3","attempt_id":attempt_b}),
            )
            .unwrap();
        assert_eq!(state.tasks[0].state, TaskState::Cancelled);
    }

    #[test]
    fn sends_to_a_pending_request_must_name_it_and_its_generation() {
        let mut state = state_with_task();
        let (commands, _events) = fake_pi(&mut state.threads[1]);
        state.threads[1].request = Some(json!({"id":"request-1","method":"input"}));
        state.threads[1].request_generation = "generation-1".into();
        // An unfenced send cannot resolve a request the caller never observed.
        let error = state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"answer","client_request_id":"a-1"}),
            )
            .unwrap_err();
        assert!(error.contains("pending request"), "{error}");
        // Naming a request or generation the state does not show fails closed,
        // so a delayed answer cannot resolve a newer question.
        let error = state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"answer","client_request_id":"a-2",
                    "request_id":"request-0","runtime_generation":"generation-1"}),
            )
            .unwrap_err();
        assert!(
            error.contains("Request or runtime generation changed"),
            "{error}"
        );
        let error = state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"answer","client_request_id":"a-3",
                    "request_id":"request-1","runtime_generation":"generation-0"}),
            )
            .unwrap_err();
        assert!(
            error.contains("Request or runtime generation changed"),
            "{error}"
        );
        assert!(state.threads[1].request.is_some());
        // The observed identity answers the request once.
        state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"the answer","client_request_id":"a-4",
                    "request_id":"request-1","runtime_generation":"generation-1"}),
            )
            .unwrap();
        assert!(state.threads[1].request.is_none());
        assert_eq!(
            commands.try_recv().unwrap()["type"],
            "extension_ui_response"
        );
        // A delayed retry of the same answer now fences against the expired
        // request instead of becoming silent follow-up input.
        let error = state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"the answer","client_request_id":"a-5",
                    "request_id":"request-1","runtime_generation":"generation-1"}),
            )
            .unwrap_err();
        assert!(error.contains("expired"), "{error}");
    }

    #[test]
    fn legacy_children_cannot_answer_questions_without_both_observed_fences() {
        let mut state = state_with_task();
        state.tasks.clear();
        let (commands, _events) = fake_pi(&mut state.threads[1]);
        state.threads[1].request = Some(json!({"id":"new-question","method":"input"}));
        state.threads[1].request_generation = "new-generation".into();
        for (key, fields) in [
            ("missing", json!({})),
            ("partial", json!({"request_id":"new-question"})),
            (
                "old",
                json!({"request_id":"old-question","runtime_generation":"new-generation"}),
            ),
        ] {
            let mut args = json!({"thread_id":2,"message":"answer","client_request_id":key});
            args.as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            assert!(state.mcp_call(1, "task_send", &args).is_err());
            assert!(state.threads[1].request.is_some());
            assert!(commands.try_recv().is_err());
        }
        let args = json!({"thread_id":2,"message":"answer","client_request_id":"valid",
            "request_id":"new-question","runtime_generation":"new-generation"});
        let response = state.mcp_call(1, "task_send", &args).unwrap();
        assert_eq!(response["control"]["saved"], true);
        assert_eq!(response["control"]["delivery"], "answer");
        assert_eq!(response["control"]["worker_acceptance"], "not_observed");
        assert_eq!(
            commands.try_recv().unwrap()["type"],
            "extension_ui_response"
        );
        assert_eq!(
            state.mcp_call(1, "task_send", &args).unwrap()["control"],
            response["control"]
        );
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn keyed_task_controls_require_a_batch_before_mutating_and_commit_atomically() {
        for tool in ["task_send", "task_cancel"] {
            let directory =
                std::env::temp_dir().join(format!("shina-control-{}", uuid::Uuid::new_v4()));
            let mut runtime =
                crate::runtime::RuntimeState::open(directory, Arc::new(|| {})).unwrap();
            runtime.state.demo = true;
            runtime.state.runtime_enabled = false;
            let database = runtime.database.clone().unwrap();
            database.begin().unwrap();
            let capture = crate::outbox::Capture::begin();
            runtime
                .state
                .mcp_call(1, "delegate_task", &delegate_args("allocate"))
                .unwrap();
            runtime.persist_for_test(capture.finish()).unwrap();
            let child = runtime.state.tasks[0].child_thread_id;
            let attempt = runtime.state.tasks[0].attempt_id.clone();
            let mut args = json!({"thread_id":child,"client_request_id":"control"});
            if tool == "task_cancel" {
                args["attempt_id"] = json!(attempt);
            } else {
                args["message"] = json!("one accepted input");
            }
            let before = database.read("index.json").unwrap().unwrap();
            let state_before = runtime.state.snapshot();
            assert!(runtime.state.mcp_call(1, tool, &args).is_err());
            assert_eq!(
                runtime.state.snapshot(),
                state_before,
                "an unfenced batch mutated state"
            );
            database.test_connection(|connection| connection.execute_batch(
                "CREATE TRIGGER fail_control_receipt BEFORE INSERT ON command_receipts
                 WHEN NEW.device LIKE 'mcp/delegate/%' BEGIN SELECT RAISE(ABORT,'injected control receipt failure'); END;"
            )).unwrap();
            database.begin().unwrap();
            let capture = crate::outbox::Capture::begin();
            let (reply, received) = mpsc::channel();
            runtime.state.mcp_request(crate::mcp::Request {
                thread: 1,
                tool: tool.into(),
                args,
                reply,
            });
            let persisted = runtime.persist_for_test(capture.finish());
            assert!(
                persisted.is_err(),
                "{tool}: {persisted:?}; reply {:?}",
                received.try_recv()
            );
            assert!(
                received.try_recv().is_err(),
                "a reply escaped a failed commit"
            );
            database.test_connection(|connection| {
                assert_eq!(
                    crate::projection_store::read(connection, "index.json")
                        .unwrap()
                        .unwrap(),
                    before
                );
            });
        }
    }

    #[test]
    fn task_cancel_refuses_to_guess_the_observed_attempt() {
        let mut state = state_with_task();
        let error = state
            .mcp_call(
                1,
                "task_cancel",
                &json!({"thread_id":2,"client_request_id":"cancel-no-fence"}),
            )
            .unwrap_err();
        assert!(error.contains("attempt_id"), "{error}");
        assert!(!state.tasks[0].state.finished());
        assert!(state.threads[1].task_result.is_none());
        for field in ["message", "delivery", "request_id", "runtime_generation"] {
            let error = state
                .mcp_call(
                    1,
                    "task_cancel",
                    &json!({"thread_id":2,"client_request_id":format!("cancel-{field}"),
                        "attempt_id":"a",field:"x"}),
                )
                .unwrap_err();
            assert!(error.contains("does not accept"), "{error}");
        }
    }

    #[test]
    fn delivery_modes_and_request_fences_are_explicit() {
        let mut state = state_with_task();
        let error = state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"x","client_request_id":"d-1","delivery":"teleport"}),
            )
            .unwrap_err();
        assert!(error.contains("Unknown delivery mode"), "{error}");
        let error = state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"x","client_request_id":"d-2","delivery":"steer"}),
            )
            .unwrap_err();
        assert!(error.contains("attempt_id"), "{error}");
        // Steering a harness that cannot steer reports instead of queueing.
        state.threads[1].harness = "none".into();
        let status = state
            .mcp_call(1, "task_status", &json!({"thread_id":2}))
            .unwrap();
        let attempt = status["attempt_id"].as_str().unwrap().to_owned();
        let error = state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"x","client_request_id":"d-3",
                    "delivery":"steer","attempt_id":attempt}),
            )
            .unwrap_err();
        assert!(error.contains("cannot accept steering"), "{error}");
        // A request fence against no pending request fails closed.
        let error = state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"x","client_request_id":"d-4",
                    "request_id":"gone","runtime_generation":"gone"}),
            )
            .unwrap_err();
        assert!(error.contains("expired"), "{error}");
    }

    #[test]
    fn boot_never_replays_even_a_never_accepted_queued_brief() {
        let mut state = state_with_task();
        state.demo = true;
        state.tasks[0].state = TaskState::Queued;
        state.prepare_task_recovery();
        state.poll_tasks(Instant::now());
        assert_eq!(state.tasks[0].state, TaskState::Interrupted);
        assert!(state.threads[1].pending_injections.is_empty());
        let status = state
            .mcp_call(1, "task_status", &json!({"thread_id":2}))
            .unwrap();
        assert_eq!(status["finished"], false);
        assert_eq!(state.attention()[0].kind, AttentionKind::Interrupted);
        let id = state.tasks[0].id.clone();
        state.resume_task(&id).unwrap();
        assert_eq!(state.tasks[0].state, TaskState::Starting);
        assert!(state.tasks[0].attempt_message.contains(RESUME_MESSAGE));
        assert!(state.tasks[0].attempt_message.contains("Inspect the files"));
        state.dismiss_task(&id).unwrap();
        assert_eq!(state.tasks[0].state, TaskState::Cancelled);
        assert!(!state.tasks[0].delivered);
        assert!(state.tasks[0].hold_delivery);
    }

    #[test]
    fn boot_history_reducer_settles_without_starting_the_parent() {
        let mut state = state_with_task();
        state.demo = true;
        state.threads[1].session = Some("fixture.jsonl".into());
        state.prepare_task_recovery();
        let record = history(
            &state.tasks[0],
            vec![json!({"role":"assistant","stopReason":"stop","content":"Recovered final"})],
        );
        state.threads[1].apply(record);
        state.poll_tasks(Instant::now());
        assert_eq!(state.tasks[0].state, TaskState::Completed);
        assert!(state.threads[0].pi.is_none());
        assert!(state.threads[0].pending_injections.is_empty());
        assert_eq!(state.tasks[0].result_excerpt, "Recovered final");
    }

    #[test]
    fn the_ui_can_answer_a_child_request_but_mcp_cannot_approve_it() {
        let mut state = state_with_task();
        state.demo = true;
        let (commands, _events) = fake_pi(&mut state.threads[1]);
        state.threads[1].request = Some(json!({"id":"approval","method":"confirm"}));
        assert_eq!(state.attention()[0].kind, AttentionKind::Approval);
        state
            .respond_to_request(2, json!({"confirmed":true}))
            .unwrap();
        let response = commands.try_recv().unwrap();
        assert_eq!(response["type"], "extension_ui_response");
        assert_eq!(response["id"], "approval");
        assert_eq!(response["confirmed"], true);
        assert!(
            state
                .mcp_call(
                    1,
                    "respond_to_request",
                    &json!({"thread_id":2,"confirmed":true})
                )
                .is_err()
        );
        state.threads[1].request =
            Some(json!({"id":"select","method":"select","options":["yes","no"]}));
        assert!(
            state
                .respond_to_request(2, json!({"value":"invalid"}))
                .is_err()
        );
        state.respond_to_request(2, json!({"value":"yes"})).unwrap();
        assert_eq!(commands.try_recv().unwrap()["value"], "yes");
        state.threads[1].life.archived_at = Some(lifecycle::now());
        assert!(state.attention().is_empty());
    }

    #[test]
    fn read_only_modes_round_trip_and_children_use_the_default_worker_mode() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        state.threads[0].native.access = AccessMode::ReadOnly;
        state
            .mcp_call(
                1,
                "delegate_task",
                &json!({"prompt":"Worker changes files"}),
            )
            .unwrap();
        assert_eq!(state.threads[1].native.access, AccessMode::Full);
        assert_eq!(AccessMode::ReadOnly.label(), "Read only");
        let snapshot = state.snapshot();
        state.restore_global(&snapshot).unwrap();
        assert_eq!(state.threads[0].native.access, AccessMode::ReadOnly);
        assert_eq!(state.threads[1].native.access, AccessMode::Full);
    }

    #[test]
    fn new_captain_homes_default_to_read_only_but_saved_modes_do_not_change() {
        let mut state = State::new(PathBuf::new(), false);
        state.captains = Captain::presets();
        state.open_captain(0);
        assert!(state.current().captain_home);
        assert_eq!(state.current().native.access, AccessMode::ReadOnly);
        state.current_mut().native.access = AccessMode::Supervised;
        let snapshot = state.snapshot();
        state.restore_global(&snapshot).unwrap();
        state.open_captain(0);
        assert_eq!(state.current().native.access, AccessMode::Supervised);
    }

    #[test]
    fn recovery_notices_distinguish_clean_and_unclean_task_shutdown() {
        let mut source = state_with_task();
        source.threads[1].session = Some("test-only-session.jsonl".into());
        let mut second = Thread::new(3);
        second.parent = Some(1);
        second.delegated = true;
        second.session = Some("second-test-only-session.jsonl".into());
        let mut task = Task::new(&second, "Second task", None);
        task.state = TaskState::Running;
        source.threads.push(second);
        source.tasks.push(task);
        source.next_id = 4;
        std::fs::write(
            source.data_dir.join("index.json"),
            source.snapshot().to_string(),
        )
        .unwrap();
        {
            let mut restored = State::new(PathBuf::new(), false);
            restored.load_global_for_test();
            assert!(
                restored
                    .recovery_notice
                    .as_deref()
                    .unwrap()
                    .contains("Shina was closed while 2 tasks were running")
            );
            restored.checkpoint_and_flush().unwrap();
            restored.mark_clean_quit();
        }
        let marker = crate::store::RunMarker::start(&source.data_dir).unwrap();
        drop(marker); // Simulate an unclean termination, without starting any process.
        let mut restored = State::new(PathBuf::new(), false);
        restored.load_global_for_test();
        assert!(
            restored
                .recovery_notice
                .as_deref()
                .unwrap()
                .starts_with("Shina closed unexpectedly")
        );
    }

    #[test]
    fn a_failed_durable_start_never_sends_the_brief() {
        let mut state = state_with_task();
        state.load_global_for_test();
        let blocked = state.data_dir.join("blocked-index.json");
        std::fs::create_dir(&blocked).unwrap();
        state.store = Some(crate::store::Store::new(blocked, state.wake.clone()));
        state.runtime_enabled = true;
        state.tasks[0].state = TaskState::Queued;
        let (commands, _events) = fake_pi(&mut state.threads[1]);
        state.start_queued_tasks();
        assert_eq!(state.tasks[0].state, TaskState::Interrupted);
        assert!(state.threads[1].pending_injections.is_empty());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn a_queued_follow_up_is_not_finished_by_the_previous_turn() {
        let mut state = state_with_task();
        state.demo = true;
        let (commands, _events) = fake_pi(&mut state.threads[0]);
        let (_child_commands, _events) = fake_pi(&mut state.threads[1]);
        state.threads[1].status = Status::Running;
        // The accepted brief and its first reply are part of the transcript.
        for (role, text) in [
            ("Shina", state.tasks[0].attempt_message.clone()),
            ("Pi", "First reply".into()),
        ] {
            state.threads[1].messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role,
                text,
                images: Vec::new(),
                context: None,
            });
        }
        // A correction on an occupied child queues instead of restarting it.
        state
            .prepare_task_control(1, 1, "task_send", &json!({"message":"Extend it"}))
            .unwrap();
        assert_eq!(state.threads[1].pending_injections.len(), 1);
        // Another injection's prompt acknowledgement is still in flight, so the
        // correction cannot be sent when the current turn settles.
        state.threads[1].injection_id = Some("2-41".into());
        state.threads[1].apply(json!({"type":"agent_settled","aborted":false}));
        state.poll_tasks(Instant::now());
        assert_eq!(
            state.tasks[0].state,
            TaskState::Running,
            "an unsent follow-up keeps the task open"
        );
        assert!(commands.try_recv().is_err());
        // The correction is delivered and its own turn completes the task.
        let sent = state.threads[1].pending_injections.pop_front().unwrap();
        state.threads[1].injection_id = None;
        for (role, text) in [("Shina", sent.text), ("Pi", "Extended reply".into())] {
            state.threads[1].messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role,
                text,
                images: Vec::new(),
                context: None,
            });
        }
        state.threads[1].arm_task();
        state.threads[1].apply(json!({"type":"agent_settled","aborted":false}));
        state.poll_tasks(Instant::now());
        assert_eq!(state.tasks[0].state, TaskState::Completed);
        assert_eq!(state.tasks[0].result_excerpt, "Extended reply");
        let prompt = commands.try_recv().unwrap();
        assert!(
            prompt["message"]
                .as_str()
                .unwrap()
                .contains("Extended reply")
        );
        assert!(
            commands
                .try_iter()
                .all(|command| command["type"] != "prompt"),
            "the result is delivered exactly once"
        );
    }

    #[test]
    fn a_stale_cancelled_turn_does_not_cancel_a_pending_correction() {
        let mut state = state_with_task();
        state.demo = true;
        let (_commands, _events) = fake_pi(&mut state.threads[1]);
        state.threads[1].status = Status::Running;
        state
            .prepare_task_control(1, 1, "task_send", &json!({"message":"Correction"}))
            .unwrap();
        state.threads[1].injection_id = Some("earlier-prompt".into());
        state.threads[1].apply(json!({"type":"agent_settled","aborted":true}));
        state.poll_tasks(Instant::now());
        assert_eq!(state.tasks[0].state, TaskState::Running);
        assert!(state.threads[1].task_result.is_none());
        assert_eq!(state.threads[1].pending_injections.len(), 1);
        assert!(state.threads[0].pending_injections.is_empty());
    }

    #[test]
    fn a_dismissed_task_keeps_its_reason_when_the_settle_arrives_later() {
        let mut state = state_with_task();
        state.demo = true;
        let id = state.tasks[0].id.clone();
        state.dismiss_task(&id).unwrap();
        assert_eq!(state.tasks[0].state, TaskState::Cancelled);
        assert_eq!(
            state.tasks[0].error_excerpt,
            "The user dismissed this task."
        );
        // A stale completion from the stopped run must not overwrite the dismissal.
        state.poll_tasks(Instant::now());
        assert_eq!(
            state.tasks[0].error_excerpt,
            "The user dismissed this task."
        );
        assert!(state.tasks[0].result_excerpt.is_empty());
    }

    #[test]
    fn captain_evidence_covers_earlier_homes_and_exact_attempts() {
        let mut state = state_with_task();
        state.demo = true;
        state.captains = Captain::presets();
        // The delegating thread is a retired captain home that still holds the evidence.
        state.threads[0].captain = Some(0);
        state.threads[0].captain_home = true;
        state.threads[1].crew_captain = Some(0);
        state.tasks[0].coordinator_captain = Some(0);
        state.tasks[0].finish(TaskState::Completed, "Done.", "");
        let first_delivery = state.tasks[0].delivery_id.clone();
        state.threads[0].pending_injections.push_back(Injection {
            text: state.tasks[0].completion_message(),
            shown: false,
            steer: None,
        });
        state.threads[0].injection_id = Some("ack-1".into());
        state.threads[0]
            .observe_task_history(&json!({"type":"response","command":"prompt","id":"ack-1"}));
        // A new attempt keeps the old delivery as evidence, with a fresh delivery id.
        state.queue_task_attempt(0, "Retry it.").unwrap();
        assert_ne!(state.tasks[0].delivery_id, first_delivery);
        assert_eq!(state.tasks[0].prior_deliveries.len(), 1);
        state.tasks[0].finish(TaskState::Completed, "Retried.", "");
        assert!(state.reconcile_task_acceptances());
        assert!(
            state.tasks[0].prior_deliveries.is_empty(),
            "the accepted earlier delivery settled"
        );
        assert!(
            !state.tasks[0].delivered,
            "earlier evidence cannot ack the newer attempt"
        );
    }

    #[test]
    fn an_archived_effective_home_routes_results_back_to_the_delegating_thread() {
        let mut state = state_with_task();
        state.demo = true;
        state.captains = Captain::presets();
        // The delegating thread is the captain's retired home; the current home
        // is archived but still wins the activity ordering.
        state.threads[0].captain = Some(0);
        state.threads[0].captain_home = true;
        let mut home = Thread::new(state.next_id);
        state.next_id += 1;
        home.captain = Some(0);
        home.captain_home = true;
        home.updated = state.threads[0].updated.saturating_add(1);
        home.life.archived_at = Some(1);
        let home_id = home.id;
        state.threads.push(home);
        assert_eq!(state.effective_home(1), home_id);
        state.tasks[0].finish(TaskState::Completed, "Orphaned result", "");
        state.poll_tasks(Instant::now());
        assert!(
            state.threads[0]
                .pending_injections
                .iter()
                .any(|i| i.text.contains("Orphaned result")),
            "the result falls back to the delegating thread when its home is archived"
        );
        assert!(!state.tasks[0].delivered);
    }

    fn delegate_args(key: &str) -> Value {
        json!({"prompt":"Fix the parser","title":"Parser fix","client_request_id":key})
    }

    #[test]
    fn keyed_delegation_replays_one_task_and_rejects_changed_intent() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        let first = state
            .mcp_call(1, "delegate_task", &delegate_args("run-1"))
            .unwrap();
        assert_eq!(state.tasks.len(), 1);
        assert_eq!(state.threads.len(), 2);
        // wait_seconds is a response option outside the keyed intent.
        let mut waited = delegate_args("run-1");
        waited["wait_seconds"] = json!(30);
        let replay = state.mcp_call(1, "delegate_task", &waited).unwrap();
        assert_eq!(replay["thread_id"], first["thread_id"]);
        assert_eq!(replay["task_id"], first["task_id"]);
        assert_eq!(replay["replayed"], true);
        assert_eq!(state.tasks.len(), 1);
        assert_eq!(state.threads.len(), 2);
        for args in [
            json!({"prompt":"Fix the parser differently","title":"Parser fix","client_request_id":"run-1"}),
            json!({"prompt":"Fix the parser","title":"Another title","client_request_id":"run-1"}),
            json!({"prompt":"Fix the parser","title":"Parser fix","workspace":"/elsewhere","client_request_id":"run-1"}),
            json!({"prompt":"Fix the parser","title":"Parser fix","target":{"harness":"codex"},"client_request_id":"run-1"}),
        ] {
            let error = state.mcp_call(1, "delegate_task", &args).unwrap_err();
            assert!(error.contains("different brief or target"), "{error}");
        }
        assert_eq!(state.tasks.len(), 1);
        assert_eq!(state.threads.len(), 2);
        assert!(
            state
                .mcp_call(
                    1,
                    "delegate_task",
                    &json!({"prompt":"Fix the parser","client_request_id":""})
                )
                .unwrap_err()
                .contains("1 to 256")
        );
    }

    #[test]
    fn keyed_delegation_replays_at_capacity_and_after_completion() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        let first = state
            .mcp_call(1, "delegate_task", &delegate_args("cap-1"))
            .unwrap();
        state.tasks[0].finish(TaskState::Completed, "done", "");
        for _ in 0..UNFINISHED_LIMIT {
            state
                .mcp_call(1, "delegate_task", &json!({"prompt":"Another task"}))
                .unwrap();
        }
        assert_eq!(state.tasks.len(), UNFINISHED_LIMIT + 1);
        assert!(
            state
                .mcp_call(1, "delegate_task", &json!({"prompt":"Ninth task"}))
                .unwrap_err()
                .contains("8 unfinished")
        );
        let replay = state
            .mcp_call(1, "delegate_task", &delegate_args("cap-1"))
            .unwrap();
        assert_eq!(replay["thread_id"], first["thread_id"]);
        assert_eq!(replay["replayed"], true);
        assert_eq!(replay["status"], "completed");
    }

    #[test]
    fn keyed_delegation_replays_after_the_child_records_are_gone() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        let first = state
            .mcp_call(1, "delegate_task", &delegate_args("gone-1"))
            .unwrap();
        let child = first["thread_id"].as_u64().unwrap() as usize;
        state.remove_threads(&std::collections::HashSet::from([child]));
        assert!(state.tasks.is_empty());
        let replay = state
            .mcp_call(1, "delegate_task", &delegate_args("gone-1"))
            .unwrap();
        assert_eq!(replay["thread_id"], child);
        assert_eq!(replay["task_id"], first["task_id"]);
        assert_eq!(replay["replayed"], true);
        assert_eq!(replay["exists"], false);
        assert_eq!(replay["finished"], true);
        assert!(state.tasks.is_empty());
        assert_eq!(state.threads.len(), 1);
        // The durable receipt outlives the task record across a restart too.
        let snapshot = state.snapshot();
        let mut restored = State::new(PathBuf::new(), false);
        restored.demo = true;
        restored.restore_global(&snapshot).unwrap();
        let replay = restored
            .mcp_call(1, "delegate_task", &delegate_args("gone-1"))
            .unwrap();
        assert_eq!(replay["thread_id"], child);
        assert_eq!(replay["exists"], false);
        assert!(restored.tasks.is_empty());
    }

    #[test]
    fn keyed_delegation_survives_a_snapshot_restart() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        let first = state
            .mcp_call(1, "delegate_task", &delegate_args("boot-1"))
            .unwrap();
        let snapshot = state.snapshot();
        let mut restored = State::new(PathBuf::new(), false);
        restored.demo = true;
        restored.restore_global(&snapshot).unwrap();
        let replay = restored
            .mcp_call(1, "delegate_task", &delegate_args("boot-1"))
            .unwrap();
        assert_eq!(replay["thread_id"], first["thread_id"]);
        assert_eq!(replay["task_id"], first["task_id"]);
        assert_eq!(replay["replayed"], true);
        assert_eq!(restored.tasks.len(), 1);
        assert_eq!(restored.threads.len(), 2);
    }

    #[test]
    fn keyed_delegation_scopes_to_the_captain_across_home_resets() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        state.captains = Captain::presets();
        state.open_captain(0);
        let home = state.captain_home(0).unwrap();
        let first = state
            .mcp_call(home, "delegate_task", &delegate_args("reset-1"))
            .unwrap();
        state.reset_captain_conversation(0).unwrap();
        let new_home = state.captain_home(0).unwrap();
        assert_ne!(new_home, home);
        let replay = state
            .mcp_call(new_home, "delegate_task", &delegate_args("reset-1"))
            .unwrap();
        assert_eq!(replay["thread_id"], first["thread_id"]);
        assert_eq!(replay["replayed"], true);
        assert_eq!(state.tasks.len(), 1);
    }

    #[test]
    fn keyed_delegation_scopes_to_the_caller_and_unkeyed_calls_allocate() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        let first = state
            .mcp_call(1, "delegate_task", &delegate_args("scope-1"))
            .unwrap();
        // A different caller may reuse the same key for its own task.
        let caller = state.next_id;
        state.threads.push(Thread::new(caller));
        state.next_id += 1;
        let other = state
            .mcp_call(caller, "delegate_task", &delegate_args("scope-1"))
            .unwrap();
        assert_ne!(other["thread_id"], first["thread_id"]);
        assert_eq!(state.tasks.len(), 2);
        // Legacy callers without a key keep the old behavior: every call allocates.
        let unkeyed = json!({"prompt":"Fix the parser","title":"Parser fix"});
        let one = state.mcp_call(1, "delegate_task", &unkeyed).unwrap();
        let two = state.mcp_call(1, "delegate_task", &unkeyed).unwrap();
        assert_ne!(two["thread_id"], one["thread_id"]);
        assert_eq!(state.tasks.len(), 4);
    }

    #[test]
    fn keyed_delegation_commits_receipt_and_allocation_atomically() {
        let directory =
            std::env::temp_dir().join(format!("shina-delegate-{}", uuid::Uuid::new_v4()));
        let mut runtime = crate::runtime::RuntimeState::open(directory, Arc::new(|| {})).unwrap();
        runtime.state.demo = true;
        let database = runtime.database.clone().unwrap();
        let before = database.read("index.json").unwrap().unwrap();
        database
            .test_connection(|connection| {
                connection.execute_batch(
                    "CREATE TRIGGER fail_delegate_receipt BEFORE INSERT ON command_receipts
                     WHEN NEW.device LIKE 'mcp/delegate/%' BEGIN SELECT RAISE(ABORT,'injected delegate receipt failure'); END;",
                )
            })
            .unwrap();
        let (reply, received) = mpsc::channel();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "delegate_task".into(),
            args: delegate_args("atomic-1"),
            reply,
        });
        assert!(runtime.persist_for_test(capture.finish()).is_err());
        assert!(received.try_recv().is_err());
        assert!(database.read("index.json").is_err());
        database.test_connection(|connection| {
            assert_eq!(
                crate::projection_store::read(connection, "index.json")
                    .unwrap()
                    .unwrap(),
                before
            );
            let count: i64 = connection
                .query_row(
                    "SELECT count(*) FROM command_receipts WHERE device LIKE 'mcp/delegate/%'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn keyed_delegation_replays_after_deletion_and_restart_on_the_durable_server() {
        let directory =
            std::env::temp_dir().join(format!("shina-delegate-{}", uuid::Uuid::new_v4()));
        let child;
        {
            let mut runtime =
                crate::runtime::RuntimeState::open(directory.clone(), Arc::new(|| {})).unwrap();
            runtime.state.demo = true;
            let database = runtime.database.clone().unwrap();
            database.begin().unwrap();
            let capture = crate::outbox::Capture::begin();
            let first = runtime
                .state
                .mcp_call(1, "delegate_task", &delegate_args("db-1"))
                .unwrap();
            runtime.persist_for_test(capture.finish()).unwrap();
            child = first["thread_id"].as_u64().unwrap() as usize;
            database.test_connection(|connection| {
                let count: i64 = connection
                    .query_row(
                        "SELECT count(*) FROM command_receipts WHERE device LIKE 'mcp/delegate/%'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(count, 1);
            });
            database.begin().unwrap();
            let capture = crate::outbox::Capture::begin();
            runtime
                .state
                .remove_threads(&std::collections::HashSet::from([child]));
            runtime.persist_for_test(capture.finish()).unwrap();
        }
        let mut runtime = crate::runtime::RuntimeState::open(directory, Arc::new(|| {})).unwrap();
        runtime.state.demo = true;
        assert!(
            !runtime
                .state
                .threads
                .iter()
                .any(|thread| thread.id == child)
        );
        let replay = runtime
            .state
            .mcp_call(1, "delegate_task", &delegate_args("db-1"))
            .unwrap();
        assert_eq!(replay["thread_id"], child);
        assert_eq!(replay["replayed"], true);
        assert_eq!(replay["exists"], false);
        assert_eq!(replay["finished"], true);
        assert!(runtime.state.tasks.is_empty());
    }
}
