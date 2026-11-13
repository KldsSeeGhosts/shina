use crate::attachments::{self, Attachment, Prompt};
use crate::backend::{Event, HostLink};
use crate::captains::{Captain, CaptainEditor};
use crate::context::ContextMeter;
use crate::harness::Session as Pi;
use crate::lifecycle::{
    self, Action, DropPlan, Lifecycle, Section, SnoozeChoice, UndoNotice, UndoStep,
};
use crate::projects::{Project, ProjectId};
use crate::session::{AgentModel, Catalog, IDLE_TIMEOUT, START_TIMEOUT};
use rune::{TextEdit, Tx, Waker};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// The label shown when a thread uses whatever model Pi is configured with.
pub const DEFAULT_MODEL: &str = "Default model";
pub const DEFAULT_SIDEBAR_WIDTH: f32 = 256.0;
const INJECTION_PREFIX: &str = "[Shina orchestration]\n";

struct ContextAssetDownload {
    machine: String,
    thread_id: usize,
    new_context_id: String,
    name: String,
    mime_type: String,
    kind: &'static str,
    result: Result<Vec<u8>, String>,
    question_scope: Option<String>,
}

fn fetch_linked_blob(
    link: &crate::peer::Link,
    attachment_id: &str,
    maximum: u64,
) -> Result<Vec<u8>, String> {
    let address = link.address()?;
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|_| "Downloading it from the source failed.".to_owned())?;
    let mut bytes = Vec::new();
    loop {
        let response = client
            .get(format!(
                "http://{address}/v1/blobs/{attachment_id}?offset={}&limit={}",
                bytes.len(),
                crate::protocol::PAGE_BYTES
            ))
            .bearer_auth(&link.token)
            .send()
            .map_err(|_| "The environment it came from is not reachable right now.".to_owned())?;
        if response.status().as_u16() == 404 || response.status().as_u16() == 400 {
            return Err("The original attachment is no longer available.".into());
        }
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err("Authentication denied. Reconnect this linked machine manually before fetching assets.".into());
        }
        if !response.status().is_success() {
            return Err("Downloading it from the source failed.".into());
        }
        if response
            .content_length()
            .is_none_or(|n| n > crate::protocol::MAX_PAGE_BYTES as u64)
        {
            return Err("Invalid remote blob response.".into());
        }
        let chunk = response
            .bytes()
            .map_err(|_| "Downloading it from the source failed.".to_owned())?;
        if chunk.is_empty() {
            break;
        }
        if bytes.len() + chunk.len() > maximum as usize {
            return Err("The attachment exceeds the maximum size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
        if chunk.len() < crate::protocol::PAGE_BYTES {
            break;
        }
    }
    if bytes.is_empty() {
        return Err("The original attachment is no longer available.".into());
    }
    if attachments::safe_file_id(attachment_id) {
        use sha2::Digest;
        if attachment_id != format!("file-{:x}.bin", sha2::Sha256::digest(&bytes)) {
            return Err("File hash mismatch.".into());
        }
    }
    Ok(bytes)
}

#[path = "pi_session.rs"]
pub(crate) mod pi_session;
pub use pi_session::{
    AccessMode, ConversationMode, ManagedPolicy, PiSession, clamp_managed_access_mode,
    managed_access_mode_denial, read_managed_policy, resolve_compatible_access_mode,
    session_history, supported_access_modes,
};
#[path = "approval_send.rs"]
mod approval_send;
#[cfg(test)]
#[path = "approval_send_tests.rs"]
mod approval_send_tests;
#[path = "question_answers.rs"]
mod question_answers;
#[cfg(test)]
#[path = "question_attachment_tests.rs"]
mod question_attachment_tests;
#[cfg(test)]
#[path = "question_flow_tests.rs"]
mod question_flow_tests;
#[path = "tasks.rs"]
// Public UI entry points are consumed by the concurrent captain UI implementation.
#[allow(dead_code)]
mod tasks;
use tasks::Task;
#[allow(unused_imports)]
pub use tasks::{AttentionItem, AttentionKind, TaskState, TaskView};
#[path = "background.rs"]
mod background;
#[allow(unused_imports)]
pub use background::{BackgroundSettings, BackgroundStatus};
#[path = "client_state.rs"]
mod client_state;
#[path = "draft_routing.rs"]
mod draft_routing;
#[path = "queued_edit.rs"]
mod queued_edit;
#[path = "run_control.rs"]
mod run_control;
pub use queued_edit::QueuedEdit;
#[cfg(test)]
use queued_edit::queue_revision;
#[cfg(test)]
pub use run_control::StopPhase;
pub use run_control::{StopControl, TurnCapabilities};
#[path = "honcho.rs"]
mod honcho;
#[path = "memory.rs"]
#[allow(dead_code)] // Public entry points are shared with the captain records UI.
mod memory;
#[cfg(test)]
#[path = "queued_edit_tests.rs"]
mod queued_edit_tests;
#[cfg(test)]
#[path = "run_control_tests.rs"]
mod run_control_tests;
#[path = "scope.rs"]
#[allow(dead_code)] // Public entry points are shared with the captain records UI.
mod scope;
#[allow(unused_imports)]
pub use memory::{MemoryBackendStatus, MemoryView, UserProfile};
pub use scope::RemoteProject;
#[path = "codex_model.rs"]
mod codex_model;
#[path = "remote_tasks.rs"]
mod remote_tasks;
pub(crate) use remote_tasks::Preparation;
pub use remote_tasks::{
    PeerOrigin, RemoteState, RemoteTask, wait_for_peer_thread, warm_repository_records,
};

struct Injection {
    text: String,
    shown: bool,
}

struct TaskWaiter {
    thread: usize,
    deadline: Instant,
    reply: mpsc::Sender<Result<Value, String>>,
    // Dropping this sender cancels the deadline wake when the task finishes early.
    _timer: mpsc::Sender<()>,
}

/// A model and reasoning choice, in the same shape `delegate_task` accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Selection {
    provider: String,
    model: String,
    reasoning: Option<String>,
}

impl Selection {
    fn to_value(&self) -> Value {
        let mut value = json!({"harness":if self.provider == "cpa" {"pi"} else {&self.provider},
            "provider": self.provider, "model": self.model});
        if let Some(reasoning) = &self.reasoning {
            value["options"] = json!({"reasoning": reasoning});
        }
        value
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastAction {
    OpenThread { server: String, thread_id: usize },
}

#[derive(Clone, Debug)]
pub struct ToastNotice {
    pub id: u64,
    pub kind: ToastKind,
    pub title: String,
    pub description: Option<String>,
    pub action_label: Option<String>,
    pub action: Option<ToastAction>,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct PlanSaveDialog {
    pub machine: String,
    pub cwd: String,
    pub contents: String,
    pub path_edit: TextEdit,
    pub saving: bool,
    pub request_id: Option<u64>,
}

/// The validated fields of a `delegate_task.target` request.
pub(crate) struct DelegationTarget {
    harness: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    reasoning: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Status {
    Ready,
    Running,
    Waiting,
    Completed,
    Failed,
    Paused,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Running => "Working",
            Self::Waiting => "Needs you",
            Self::Completed => "Done",
            Self::Failed => "Error",
            Self::Paused => "Paused (offline)",
        }
    }
    pub fn busy(self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }
    /// Which state a captain's portrait should show when its threads disagree. Attention
    /// outranks work, and work outranks a ready conversation.
    pub fn urgency(self) -> u8 {
        match self {
            Self::Failed => 6,
            Self::Paused => 6,
            Self::Waiting => 5,
            Self::Running => 4,
            Self::Ready => 2,
            Self::Completed => 1,
        }
    }
}

pub struct Message {
    pub id: String,
    pub role: &'static str,
    pub text: String,
    /// Images shown with this message. Only the local reference is kept; Pi gets the typed
    /// `image` parts when the prompt is submitted.
    pub images: Vec<Attachment>,
    pub context: Option<crate::composer_context::MessageContext>,
}

impl Message {
    pub fn display_text(&self) -> &str {
        if self.role == "Shina"
            && let Some(body) = self.text.strip_prefix(INJECTION_PREFIX)
        {
            // Delivery metadata lets reconnect distinguish identical results from separate runs.
            return body
                .split_once('\n')
                .filter(|(header, _)| header.starts_with("[delivery "))
                .map(|(_, text)| text)
                .unwrap_or(body);
        }
        &self.text
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Tool {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Option<crate::work::ToolKind>,
    #[serde(default)]
    pub changes: Vec<crate::work::FileChange>,
    #[serde(default)]
    pub input: Value,
    pub detail: String,
    pub done: bool,
    pub failed: bool,
    pub output: String,
    pub images: Vec<Attachment>,
    pub structured: Value,
    pub parent_call: Option<String>,
    pub exit_code: Option<i64>,
    pub subagents: Vec<Value>,
    /// Position in the transcript, including tool calls between text blocks in one message.
    pub anchor: ToolAnchor,
    pub block: usize,
    #[serde(default)]
    pub order: u64,
    #[serde(skip)]
    pub expanded: bool,
    #[serde(skip)]
    pub raw_expanded: bool,
    #[serde(skip)]
    pub group_expanded: bool,
    #[serde(skip)]
    pub started: Option<Instant>,
    pub duration_ms: Option<u64>,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ToolAnchor {
    pub message: usize,
    /// UTF-8 byte offset into the message text. Zero also anchors tool-only messages.
    pub offset: usize,
}

impl Tool {
    fn new(id: &str, name: &str, args: &Value, anchor: ToolAnchor) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind: Some(crate::work::pi_kind(name)),
            changes: vec![],
            input: args.clone(),
            detail: tool_detail(args),
            done: false,
            failed: false,
            output: String::new(),
            images: vec![],
            structured: Value::Null,
            parent_call: None,
            exit_code: None,
            subagents: vec![],
            anchor,
            block: usize::MAX,
            order: 0,
            expanded: false,
            raw_expanded: false,
            group_expanded: false,
            started: None,
            duration_ms: None,
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Thought {
    pub anchor: ToolAnchor,
    pub block: usize,
    #[serde(default)]
    pub order: u64,
    pub text: String,
    #[serde(skip)]
    pub expanded: bool,
    pub done: bool,
    #[serde(skip)]
    pub started: Option<Instant>,
    /// Pi saves thinking text, but does not save thinking timings.
    pub duration_ms: Option<u64>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AssistantWork {
    pub start: usize,
    #[serde(default)]
    pub stopped: bool,
    #[serde(default)]
    pub stopped_order: u64,
    #[serde(skip)]
    pub expanded: bool,
    pub elapsed: Option<u64>,
    /// Unknown in old saved histories. Never infer successful completion from a timer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Status>,
}

#[derive(Default)]
pub struct ThreadFindState {
    pub open: bool,
    pub server: String,
    pub thread_id: usize,
    pub query: TextEdit,
    pub last_typed: String,
    pub debounce_until: Option<Instant>,
    pub loading: bool,
    pub error: Option<String>,
    pub unsupported: bool,
    pub total_matches: usize,
    pub active_index: usize,
    pub active_match: Option<crate::thread_find::ThreadFindMatch>,
    pub navigation: Vec<crate::thread_find::ThreadFindNavEntry>,
    pub pending_reveal: Option<crate::thread_find::ThreadFindMatch>,
    pub request_id: u64,
}

#[derive(Default)]
pub struct SidebarSearchState {
    pub last_typed: String,
    pub debounce_until: Option<Instant>,
    pub request_seq: u64,
    pub settled_query: String,
    pub matches: std::collections::HashMap<String, Vec<crate::thread_search::ThreadSearchHit>>,
    pub pending_servers: std::collections::HashSet<String>,
    pub unsupported_servers: Vec<String>,
    pub visible_query: String,
    pub visible_matches:
        std::collections::HashMap<(String, usize), crate::thread_search::ThreadSearchHit>,
    pub shown_order: Vec<(String, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPicker {
    Model,
    Thinking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowTarget {
    Thread(usize),
    Project(ProjectId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectScope {
    All,
    Project(ProjectId),
    NoProject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectPicker {
    NewThread,
    Scope,
    /// Add a workspace, as T3 Code's Add project palette: pick a source, then browse.
    AddProject,
    /// Add or remove a workspace in the open captain's scope, from its Brief panel.
    CaptainScope,
}

/// UI-only state for the captain Brief panel and the profile editor in `captain_ui.rs`.
/// Nothing here is persisted; committed values go through the State memory and scope APIs.
#[derive(Default)]
pub struct BriefUi {
    /// The memory whose text is being edited inline, and its draft.
    pub editing: Option<String>,
    pub edit: TextEdit,
    /// The Add memory field is open, and its draft.
    pub adding: bool,
    pub draft: TextEdit,
    /// The Reset conversation confirmation is open.
    pub confirm_reset: bool,
    /// The last rejected action, shown in the panel until the next success.
    pub error: Option<String>,
    /// The "You" profile editor is open, with its three drafts.
    pub profile_open: bool,
    pub profile_name: TextEdit,
    pub profile_zone: TextEdit,
    pub profile_preferences: TextEdit,
    /// Demo scenes only: a stand-in for the live background status.
    pub demo_status: Option<background::BackgroundStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSuggestion {
    pub run_id: String,
    pub text: String,
}

pub use crate::protocol::ThreadEnvMode;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VcsBranchCacheEntry {
    pub is_repo: bool,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub refs: Vec<crate::vcs::VcsRef>,
}

struct VcsBranchResultMsg {
    machine: String,
    project_path: String,
    result: Result<crate::vcs::VcsListBranchesResult, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeSetupPhase {
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSetupState {
    pub phase: WorktreeSetupPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub skipped: bool,
}

#[derive(Clone, Debug)]
enum MultiModelTargetOutcome {
    Pending,
    Started,
    Failed(String),
}

#[derive(Clone, Debug)]
struct MultiModelTargetEntry {
    selection: crate::model_selection::ModelSelection,
    display_name: String,
    thread_id: Option<usize>,
    create_cmd_id: Option<uuid::Uuid>,
    send_cmd_id: Option<uuid::Uuid>,
    outcome: MultiModelTargetOutcome,
}

struct MultiModelBatch {
    source_thread_id: usize,
    server: String,
    project: Option<ProjectId>,
    base_branch: String,
    start_from_origin: bool,
    access: AccessMode,
    prompt: Prompt,
    snapshot: crate::composer_rich::Snapshot,
    reply_to: Option<crate::composer_context::ComposerContextRecord>,
    saved_draft_model_state: crate::model_selection::DraftModelState,
    targets: Vec<MultiModelTargetEntry>,
    occupied_remote_ids: Vec<usize>,
    next_remote_index: usize,
}

struct WorktreeSetupResultMsg {
    thread_id: usize,
    run_seq: u64,
    result: Result<crate::vcs::VcsRunSetupResult, String>,
}

#[derive(Clone, Debug)]
pub(crate) struct WorktreeBootstrapState {
    pub(crate) snapshot: crate::composer_rich::Snapshot,
    pub(crate) reply_to: Option<crate::composer_context::ComposerContextRecord>,
    pub(crate) queued_id: usize,
    pub(crate) fresh_context: bool,
}

struct WorktreeBootstrapResult {
    thread_id: usize,
    base_branch: String,
    result: Result<crate::vcs::VcsCreateWorktreeResult, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposerVcsControls {
    Hidden,
    Editable {
        mode: ThreadEnvMode,
        base_branch: Option<String>,
        start_from_origin: bool,
    },
    LockedWorktree {
        branch: Option<String>,
        worktree_path: String,
    },
}

pub struct Thread {
    pub peer_origin: Option<PeerOrigin>,
    pub stop_serial: u64,
    pub run_id: Option<String>,
    pub resumable_run_id: Option<String>,
    pub continuation_of_run_id: Option<String>,
    pub prompt_suggestion: Option<PromptSuggestion>,
    pub(crate) prompt_suggestions_enabled: bool,
    pub stop_control: StopControl,
    run_control_supported: bool,
    resume_command: Option<uuid::Uuid>,
    /// Device-owned draft, promoted on Send rather than while choosing a host or project.
    pub client_draft: bool,
    pub(crate) client_creating: Option<uuid::Uuid>,
    pub(crate) client_promoted: Option<usize>,
    pub remote: Option<crate::client::Commands>,
    pub(crate) remote_send: Option<(uuid::Uuid, crate::composer_rich::Snapshot)>,
    pub(crate) remote_send_fresh_arm: Option<u64>,
    pub(crate) remote_send_reply_to: Option<crate::composer_context::ComposerContextRecord>,
    pub fresh_session_arm: Option<u64>,
    pub fresh_session_messages: Vec<String>,
    pub fallback_active: Option<crate::usage_limits::FallbackInfo>,
    pub fallback_explicit_override: bool,
    pub composer_context_supported: bool,
    pub(crate) background_start_pending: bool,
    pub(crate) background_send: Option<String>,
    pub(crate) pending_send_and_new_thread: Option<(uuid::Uuid, String)>,
    pub(crate) worktree_bootstrap: Option<WorktreeBootstrapState>,
    pub(crate) remote_bootstrap_snapshot: Option<(
        crate::composer_rich::Snapshot,
        Option<crate::composer_context::ComposerContextRecord>,
    )>,
    request_generation: String,
    server_owned: bool,
    recovery_command: Option<uuid::Uuid>,
    projected_harness_choice: bool,
    pub id: usize,
    /// `pi` or an ACP registry id. Pi remains the default for legacy indexes.
    pub harness: String,
    pub harness_context_sent: bool,
    pub harness_state: Value,
    pub project: Option<ProjectId>,
    pub worktree_path: Option<String>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub env_mode: Option<ThreadEnvMode>,
    pub start_from_origin: Option<bool>,
    pub worktree_setup: Option<WorktreeSetupState>,
    pub worktree_setup_output_open: bool,
    pub(crate) worktree_setup_seq: u64,
    pub(crate) worktree_bootstrap_fresh_context: bool,
    pub machine: String,
    pub updated: u64,
    pub title: String,
    pub renamed: bool,
    pub status: Status,
    /// Resume connectivity gate. Never retries an accepted prompt.
    offline_blocked: bool,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub thoughts: Vec<Thought>,
    pub work: Vec<AssistantWork>,
    pub draft: TextEdit,
    pub queued_edit: Option<QueuedEdit>,
    queued_save: Option<(uuid::Uuid, uuid::Uuid)>,
    queue_edit_supported: bool,
    /// Images selected for the next send, kept separate from text undo history.
    pub prompt_history: crate::composer::History,
    pub typing_guard: crate::composer::TypingGuard,
    pub paste_history: crate::composer_paste::PasteHistory,
    pub composer_rules: crate::composer_rich::Rules,
    pub draft_images: Vec<Attachment>,
    pub draft_context: Vec<crate::composer_context::ComposerContextRecord>,
    pub draft_reply_to: Option<crate::composer_context::ComposerContextRecord>,
    pub retained_context: crate::composer_context::RetainedContextPayloads,
    pub attachment_error: Option<String>,
    pub submission_error: Option<String>,
    pub pi: Option<Pi>,
    /// Image input support from Pi or the catalog. `None` when metadata is unavailable.
    pub supports_images: Option<bool>,
    pub model: String,
    pub thinking: String,
    pub model_provider: String,
    pub model_id: String,
    /// Capacity reported by the selected model's metadata, when known.
    pub context_window: Option<u64>,
    /// Live context-window usage and its refresh state, kept separate from cumulative tokens.
    pub context: ContextMeter,
    pub native: PiSession,
    pub models: Vec<AgentModel>,
    pub thinking_levels: Vec<String>,
    pub configuring: bool,
    pub phase: String,
    pub error: Option<String>,
    pub diagnostics: Vec<String>,
    pub request: Option<Value>,
    pub(crate) requests_supported: bool,
    pub(crate) question_contract_supported: bool,
    pub(crate) responding_request: Option<uuid::Uuid>,
    approval_send: Option<approval_send::ApprovalSend>,
    pub reply: TextEdit,
    question_drafts: std::collections::HashMap<String, question_answers::Draft>,
    pub(super) answer_intake: bool,
    pub(super) answer_scope: Option<String>,
    pub(super) question_submit_requested: bool,
    question_submission: Option<question_answers::Submission>,
    question_resolutions: Vec<question_answers::Resolution>,
    pub session: Option<String>,
    /// A thread made here as a new thread. Until a prompt is sent, a session on it is only a
    /// warmed harness and the thread is still a draft. Restored records without the flag keep
    /// the older rule, where any session means the thread has been used.
    pub never_sent: bool,
    pub life: Lifecycle,
    pub started: Option<Instant>,
    pub elapsed: Option<u64>,
    pub captain: Option<usize>,
    pub captain_home: bool,
    /// Started by another thread through the Shina MCP. Delegated threads cannot delegate.
    pub delegated: bool,
    pub parent: Option<usize>,
    /// How this thread's Pi reaches the Shina MCP, set when it connects.
    pub host: Option<HostLink>,
    connection_owner: Arc<crate::backend::SessionOwner>,
    assistant: Option<usize>,
    serial: usize,
    state_loaded: bool,
    history_loaded: bool,
    pub history_has_more: bool,
    pub history_loading: bool,
    pub history_error: Option<String>,
    pub deferred_work: Vec<Value>,
    pub deferred_work_supported: bool,
    pub(crate) initializing: bool,
    session_started: Option<Instant>,
    last_activity: Instant,
    prewarm_attempted: bool,
    pub selected_model: Option<(String, String)>,
    pub selected_thinking: Option<String>,
    pub model_options: Vec<crate::model_selection::ProviderOptionSelection>,
    pub draft_model_state: crate::model_selection::DraftModelState,
    startup_model_applied: bool,
    startup_thinking_applied: bool,
    startup_selection_check: Option<String>,
    startup_selection_verified: bool,
    context_startup_requested: bool,
    queued_prompt: Option<(Prompt, usize)>,
    retry_prompt: Option<Prompt>,
    pending_thinking: Option<String>,
    pending_prompt: Option<(String, Prompt, usize)>,
    pending_injections: VecDeque<Injection>,
    injection_id: Option<String>,
    injection_retry_at: Option<Instant>,
    operation_deadline: Option<(&'static str, Instant)>,
    task_result: Option<&'static str>,
    completion_notified: bool,
    forwarded_question: Option<String>,
    accepted_deliveries: Vec<String>,
    recovering_task: bool,
    task_history: Option<Value>,
    memory: Option<memory::MemoryLink>,
    learned_turn: Option<honcho::Turn>,
    memory_pending: VecDeque<mpsc::Receiver<Value>>,
    memory_context: std::collections::BTreeMap<String, String>,
    memory_staged: Vec<memory::Mutation>,
    memory_suggestions: Vec<String>,
    scope_context_pending: bool,
}

impl Thread {
    pub fn is_fresh_session_message(&self, message_id: &str) -> bool {
        self.fresh_session_messages
            .iter()
            .any(|id| id == message_id)
    }

    #[cfg(test)]
    pub fn fresh_session_command_available_local(&self) -> bool {
        !self.messages.is_empty()
            && crate::harness::supports_fresh_context(&self.harness)
            && self.request.is_none()
            && self.native.questionnaire.is_none()
    }

    pub fn can_choose_harness(&self) -> bool {
        if self.remote.is_some() {
            return self.projected_harness_choice;
        }
        !self.delegated
            && !self.archived()
            && !self.status.busy()
            && !self.configuring
            && self.messages.is_empty()
            && self.life.latest_user_message_at.is_none()
            && self.native.fork_source.is_none()
            && self.native.previous_session.is_none()
            && (self.session.is_none() || self.state_loaded && self.history_loaded)
    }
    pub fn pinned(&self) -> bool {
        self.life.pinned_at.is_some()
    }
    pub fn archived(&self) -> bool {
        self.life.archived_at.is_some()
    }
    pub fn unread(&self) -> bool {
        self.life.unread()
    }
    pub fn is_draft(&self) -> bool {
        !self.captain_home
            && !self.delegated
            && !self.archived()
            && !self.pinned()
            && !self.life.settled()
            && self.life.snoozed_at.is_none()
            && self.life.failed_at.is_none()
            && !self.worktree_bootstrapping()
            && self.status == Status::Ready
            && self.unused_session()
            && self.messages.is_empty()
            && self.life.latest_user_message_at.is_none()
            && self.life.last_completed_at.is_none()
            && self.client_creating.is_none()
            && self.remote_send.is_none()
            && self.client_promoted.is_none()
    }
    /// No session, or one that was only warmed for a thread nothing has been sent in.
    fn unused_session(&self) -> bool {
        self.session.is_none() || self.never_sent
    }
    /// A new thread slot with nothing queued, running, saved or named in it. Its title becomes
    /// whatever the user types, before any prompt is sent.
    fn blank_draft(&self) -> bool {
        !self.captain_home
            && !self.delegated
            && !self.archived()
            && !self.pinned()
            && !self.life.settled()
            && self.life.snoozed_at.is_none()
            && self.life.failed_at.is_none()
            && self.life.latest_user_message_at.is_none()
            && self.life.last_completed_at.is_none()
            && self.status == Status::Ready
            && self.unused_session()
            && self.messages.is_empty()
            && !self.has_draft_content()
            && self.draft_model_state.additional_models.is_empty()
            && self.remote_send.is_none()
            && self.client_creating.is_none()
            && !self.configuring
            && !self.queued()
            && !self.renamed
            && self.parent.is_none()
    }
    pub fn actionable(&self) -> bool {
        self.request.is_some()
            || self.status == Status::Waiting
            || matches!(self.status, Status::Failed | Status::Paused)
    }
    /// Working, in every count the user reads (the header's background line, the crew line,
    /// the band's sails): running now. Queued, interrupted, finished and approval-blocked
    /// threads are not working.
    pub fn running_now(&self) -> bool {
        self.status == Status::Running && self.request.is_none() && !self.offline_blocked
    }
    pub fn queued(&self) -> bool {
        self.native.projected_queued
            || self.queued_prompt.is_some()
            || self.pending_prompt.is_some()
            || !self.pending_injections.is_empty()
            || self.native_pending_turns()
    }
    pub fn has_draft_content(&self) -> bool {
        !self.draft.text.trim().is_empty()
            || self.draft.pills().next().is_some()
            || !self.draft_images.is_empty()
            || self.draft_reply_to.is_some()
    }
    /// Add a validated attachment while preserving the text draft and its undo history.
    pub(crate) fn insert_attachment_chips(
        &mut self,
        attachments: &[Attachment],
        skip_image_inline_chip: bool,
    ) {
        if attachments.is_empty()
            || self
                .request
                .as_ref()
                .is_some_and(|r| r["method"] != "confirm")
                && !self.answer_intake
                && !self.request_held_by_typing_guard()
        {
            return;
        }
        let prose = crate::composer_context::strip_inline_context_references(
            &crate::composer_rich::markdown(&self.draft),
        );
        let image_attachments_get_chips = !skip_image_inline_chip
            && (self.answer_intake
                || self.draft.anchor != self.draft.focus
                || !prose.trim().is_empty());
        let mut records = Vec::new();
        for attachment in attachments {
            if attachment.is_image() && !image_attachments_get_chips {
                continue;
            }
            let rec = crate::composer_context::attachment_context_record(attachment);
            if let Some(existing) = self
                .draft_context
                .iter_mut()
                .find(|r| r.context_id == rec.context_id)
            {
                *existing = rec.clone();
            } else {
                self.draft_context.push(rec.clone());
            }
            records.push(rec);
        }
        if records.is_empty() {
            return;
        }
        let (start, end) = self.draft.range();
        let prev = self.draft.text[..start].chars().next_back();
        let needs_leading_space = !matches!(prev, None | Some(' ' | '\n' | '\t' | '\r'));
        let replace_end = if self.draft.text[end..].starts_with(' ') {
            end + 1
        } else {
            end
        };
        self.draft.undo_group(|e| {
            e.set_selection(start, replace_end);
            e.insert(if needs_leading_space { " " } else { "" });
            for (index, rec) in records.iter().enumerate() {
                if index > 0 {
                    e.insert(" ");
                }
                let pill_start = e.focus;
                e.insert(&rec.label);
                let pill_end = e.focus;
                let pill_id = crate::composer_context::format_composer_context_href(
                    rec.kind(),
                    &rec.context_id,
                );
                e.replace_with_pill(pill_start..pill_end, pill_id, &rec.label);
            }
            e.insert(" ");
        });
    }

    pub(crate) fn insert_context_chip(
        &mut self,
        record: crate::composer_context::ComposerContextRecord,
    ) {
        if self.request.is_some() {
            return;
        }
        let (start, end) = if self.draft.focused {
            self.draft.range()
        } else {
            let end = self.draft.text.len();
            (end, end)
        };
        let prev = self.draft.text[..start].chars().next_back();
        let needs_leading_space = !matches!(prev, None | Some(' ' | '\n' | '\t' | '\r'));
        let replace_end = if self.draft.text[end..].starts_with(' ') {
            end + 1
        } else {
            end
        };
        let pill_id = crate::composer_context::format_composer_context_href(
            record.kind(),
            &record.context_id,
        );
        let chip_label = record.label.clone();
        if let Some(existing) = self
            .draft_context
            .iter_mut()
            .find(|r| r.context_id == record.context_id)
        {
            *existing = record;
        } else {
            self.draft_context.push(record);
        }
        self.draft.undo_group(|e| {
            e.set_selection(start, replace_end);
            e.insert(if needs_leading_space { " " } else { "" });
            let pill_start = e.focus;
            e.insert(&chip_label);
            let pill_end = e.focus;
            e.replace_with_pill(pill_start..pill_end, pill_id, &chip_label);
            e.insert(" ");
        });
    }

    pub fn add_attachment(&mut self, image: Attachment) -> Result<(), String> {
        if self
            .draft_images
            .iter()
            .any(|current| current.id == image.id)
        {
            return Err(format!("{} is already attached.", image.name));
        }
        let mut proposed = self.draft_images.clone();
        proposed.push(image);
        if self.answer_intake {
            if self.question_reserved_count() >= attachments::MAX_ATTACHMENTS {
                return Err(format!(
                    "You can attach up to {} files per message.",
                    attachments::MAX_ATTACHMENTS
                ));
            }
        } else {
            attachments::validate_prompt_images(&proposed)?;
        }
        self.draft_images = proposed;
        self.attachment_error = None;
        Ok(())
    }
    pub fn remove_attachment(&mut self, id: &str) {
        self.draft_images.retain(|image| image.id != id);
        self.attachment_error = if self.answer_intake {
            None
        } else {
            attachments::validate_prompt_images(&self.draft_images).err()
        };
    }
    fn image_model_error(&self) -> String {
        let model = if self.model.is_empty() {
            "The selected model"
        } else {
            &self.model
        };
        if self.supports_images == Some(false) {
            format!(
                "{model} does not accept image input. Remove the images or choose a model that supports images."
            )
        } else {
            format!(
                "{model} does not declare image input. Remove the images or choose a model that supports images."
            )
        }
    }
    pub fn new(id: usize) -> Self {
        Self {
            remote: None,
            remote_send: None,
            remote_send_fresh_arm: None,
            remote_send_reply_to: None,
            fresh_session_arm: None,
            fresh_session_messages: Vec::new(),
            composer_context_supported: true,
            fallback_active: None,
            fallback_explicit_override: false,
            background_start_pending: false,
            background_send: None,
            pending_send_and_new_thread: None,
            worktree_bootstrap: None,
            remote_bootstrap_snapshot: None,
            queued_edit: None,
            queued_save: None,
            queue_edit_supported: true,
            run_id: None,
            resumable_run_id: None,
            continuation_of_run_id: None,
            prompt_suggestion: None,
            prompt_suggestions_enabled: false,
            stop_control: StopControl::default(),
            run_control_supported: true,
            resume_command: None,
            request_generation: uuid::Uuid::new_v4().to_string(),
            server_owned: false,
            client_draft: false,
            client_creating: None,
            client_promoted: None,
            recovery_command: None,
            projected_harness_choice: false,
            id,
            harness: "pi".into(),
            harness_context_sent: false,
            harness_state: json!({}),
            project: None,
            worktree_path: None,
            branch: None,
            base_branch: None,
            env_mode: None,
            start_from_origin: None,
            worktree_setup: None,
            worktree_setup_output_open: false,
            worktree_setup_seq: 0,
            worktree_bootstrap_fresh_context: false,
            machine: "local".into(),
            peer_origin: None,
            stop_serial: 0,
            updated: crate::projects::now(),
            title: "New thread".into(),
            renamed: false,
            status: Status::Ready,
            offline_blocked: false,
            messages: vec![],
            tools: vec![],
            thoughts: vec![],
            work: vec![],
            draft: crate::composer_rich::draft(""),
            prompt_history: crate::composer::History::default(),
            typing_guard: crate::composer::TypingGuard::default(),
            paste_history: crate::composer_paste::PasteHistory::default(),
            composer_rules: crate::composer_rich::Rules::default(),
            draft_images: Vec::new(),
            draft_context: Vec::new(),
            draft_reply_to: None,
            retained_context: Default::default(),
            attachment_error: None,
            submission_error: None,
            pi: None,
            supports_images: None,
            model: DEFAULT_MODEL.into(),
            thinking: String::new(),
            model_provider: String::new(),
            model_id: String::new(),
            context_window: None,
            context: ContextMeter::default(),
            native: PiSession::default(),
            models: vec![],
            thinking_levels: vec![],
            configuring: false,
            phase: String::new(),
            error: None,
            diagnostics: vec![],
            request: None,
            requests_supported: true,
            question_contract_supported: true,
            responding_request: None,
            approval_send: None,
            reply: TextEdit::default(),
            question_drafts: Default::default(),
            answer_intake: false,
            answer_scope: None,
            question_submit_requested: false,
            question_submission: None,
            question_resolutions: Vec::new(),
            session: None,
            never_sent: true,
            life: Lifecycle::new(lifecycle::now()),
            started: None,
            elapsed: None,
            captain: None,
            captain_home: false,
            delegated: false,
            parent: None,
            host: None,
            connection_owner: Arc::default(),
            assistant: None,
            serial: 0,
            state_loaded: false,
            history_loaded: false,
            history_has_more: false,
            history_loading: false,
            history_error: None,
            deferred_work: Vec::new(),
            deferred_work_supported: false,
            initializing: false,
            session_started: None,
            last_activity: Instant::now(),
            prewarm_attempted: false,
            selected_model: None,
            selected_thinking: None,
            model_options: vec![],
            draft_model_state: Default::default(),
            startup_model_applied: false,
            startup_thinking_applied: false,
            startup_selection_check: None,
            startup_selection_verified: false,
            context_startup_requested: false,
            queued_prompt: None,
            retry_prompt: None,
            pending_thinking: None,
            pending_prompt: None,
            pending_injections: VecDeque::new(),
            injection_id: None,
            injection_retry_at: None,
            operation_deadline: None,
            task_result: None,
            completion_notified: true,
            forwarded_question: None,
            accepted_deliveries: Vec::new(),
            recovering_task: false,
            task_history: None,
            memory: None,
            learned_turn: None,
            memory_pending: VecDeque::new(),
            memory_context: Default::default(),
            memory_staged: Vec::new(),
            memory_suggestions: Vec::new(),
            scope_context_pending: false,
        }
    }

    #[cfg(test)]
    pub fn connect(&mut self, path: PathBuf, wake: Waker) {
        self.connect_with_instructions(path, wake, None);
    }

    pub fn connect_with_instructions(
        &mut self,
        path: PathBuf,
        wake: Waker,
        instructions: Option<String>,
    ) {
        if self.pi.is_some() {
            return;
        }
        self.request_generation = uuid::Uuid::new_v4().to_string();
        let backend = match crate::backend::BackendHost::resolve(&self.machine) {
            Ok(host) => host,
            Err(error) => {
                self.fail_session(error);
                return;
            }
        };
        self.error = None;
        if !self.pending_injections.is_empty() && !self.status.busy() {
            self.arm_task();
            self.status = Status::Running;
            self.started = None;
            self.phase = "Working".into();
        } else if self.status == Status::Failed {
            self.status = Status::Ready;
        }
        self.initializing = true;
        self.prewarm_attempted = true;
        self.session_started = Some(Instant::now());
        self.last_activity = Instant::now();
        self.state_loaded = false;
        self.history_loaded = false;
        self.startup_model_applied = false;
        self.startup_thinking_applied = false;
        self.startup_selection_check = None;
        self.startup_selection_verified = false;
        self.context_startup_requested = false;
        self.context.begin_model_change();
        let source = self.planned_session_source();
        match crate::harness::start(
            backend,
            crate::harness::Start {
                id: self.harness.clone(),
                workspace: path,
                source,
                instructions,
                link: self.host.clone(),
                owner: self.connection_owner.clone(),
                context_sent: self.harness_context_sent,
                prompt_suggestions: self.prompt_suggestions_enabled && !self.delegated,
            },
            wake,
        ) {
            Ok(session) => self.pi = Some(session),
            Err(error) => self.fail_session(error),
        }
    }

    pub(crate) fn planned_session_source(&self) -> crate::backend::SessionSource {
        self.native
            .fork_source
            .clone()
            .map(crate::backend::SessionSource::Fork)
            .or_else(|| {
                self.session
                    .clone()
                    .map(crate::backend::SessionSource::Resume)
            })
            .unwrap_or(crate::backend::SessionSource::New)
    }

    pub fn set_model(&mut self, provider: String, model_id: String) {
        if self.forward(
            "thread.configure",
            json!({"provider":provider,"model":model_id}),
        ) {
            return;
        }
        if self.harness == "pi" && provider != "cpa"
            || self.harness != "pi" && provider != self.harness
        {
            self.error = Some(if self.harness == "pi" {
                "Shina uses CPA proxy models only. Choose a model from the CPA catalog.".into()
            } else {
                "Choose a model advertised by this thread's ACP harness.".into()
            });
            return;
        }
        if self.status.busy() || self.configuring {
            return;
        }
        self.attachment_error = None;
        if self.model_id != model_id {
            self.model_options.clear();
            if self.harness == "codex" {
                self.harness_state["serviceTier"] = Value::Null;
            }
        }
        self.selected_model = Some((provider.clone(), model_id.clone()));
        if self.pi.is_none() || self.initializing {
            let known = self
                .models
                .iter()
                .find(|m| m.id == model_id)
                .map(|m| (m.name.clone(), m.context_window, m.supports_images));
            if self.model_id != model_id {
                self.context.begin_model_change();
                self.context_window = None;
                self.supports_images = None;
            }
            self.model_provider = provider;
            self.model_id = model_id;
            match known {
                Some((name, window, images)) => {
                    self.model = name;
                    if window.is_some() {
                        self.context_window = window;
                    }
                    self.supports_images = images;
                }
                None => self.model = self.model_id.clone(),
            }
            self.refresh_thinking_levels();
        } else if self.command("set_model", json!({"provider":provider,"modelId":model_id})) {
            self.configuring = true;
        }
        self.native.access = resolve_compatible_access_mode(
            self.native.access,
            supported_access_modes(&self.harness),
        )
        .1;
    }

    pub fn set_thinking(&mut self, level: String) {
        if self.forward("thread.configure", json!({"thinking":level})) {
            return;
        }
        if !matches!(self.harness.as_str(), "pi" | "codex" | "claude") {
            self.error = Some("ACP reasoning selection is not supported in Shina. Use the agent's own configuration.".into());
            return;
        }
        if self.status.busy() || self.configuring {
            return;
        }
        self.selected_thinking = Some(level.clone());
        if self.pi.is_none() || self.initializing {
            self.thinking = level;
        } else if self.command("set_thinking_level", json!({"level":level})) {
            self.configuring = true;
            self.pending_thinking = Some(level);
        }
    }

    /// The pinned choice if one exists, otherwise the model Pi last applied.
    fn selected_or_effective_model(&self) -> Option<(String, String)> {
        let expected = if self.harness == "pi" {
            "cpa"
        } else {
            &self.harness
        };
        self.selected_model
            .as_ref()
            .filter(|(provider, model)| provider == expected && !model.trim().is_empty())
            .cloned()
            .or_else(|| {
                (self.model_provider == expected && !self.model_id.trim().is_empty())
                    .then(|| (self.model_provider.clone(), self.model_id.clone()))
            })
    }

    fn selected_or_effective_reasoning(&self) -> Option<String> {
        self.selected_thinking
            .clone()
            .filter(|level| !level.trim().is_empty())
            .or_else(|| (!self.thinking.trim().is_empty()).then(|| self.thinking.clone()))
    }

    /// What a delegated child inherits when its target omits a model or level.
    fn inherited_selection(&self) -> Option<Selection> {
        let (provider, model) = self.selected_or_effective_model()?;
        Some(Selection {
            provider,
            model,
            reasoning: self.selected_or_effective_reasoning(),
        })
    }

    /// What the thread is actually running now, for status and read output.
    fn effective_selection(&self) -> Option<Selection> {
        let provider = if self.harness == "pi" {
            "cpa"
        } else {
            &self.harness
        };
        if self.model_provider != provider || self.model_id.trim().is_empty() {
            return None;
        }
        Some(Selection {
            provider: self.model_provider.clone(),
            model: self.model_id.clone(),
            reasoning: (!self.thinking.trim().is_empty()).then(|| self.thinking.clone()),
        })
    }

    pub fn set_machine(&mut self, machine: impl Into<String>) {
        let machine = machine.into();
        if self.machine != machine {
            self.machine = machine;
            self.prompt_suggestion = None;
        }
    }

    pub fn command(&mut self, kind: &str, mut data: Value) -> bool {
        if kind == "prompt" {
            self.prompt_suggestion = None;
            if data.get("runId").is_none()
                && let Some(run_id) = &self.run_id
            {
                data["runId"] = json!(run_id);
            }
            if self.harness == "codex" {
                data["serviceTier"] = self.harness_state["serviceTier"].clone();
            }
        }
        self.track_learned_turn(kind, &data);
        let context = if kind == "prompt" {
            self.prepare_memory_prompt(&mut data)
        } else {
            None
        };
        self.serial += 1;
        data["type"] = kind.into();
        data["id"] = format!("{}-{}", self.id, self.serial).into();
        let result = self.send_memory_command(data);
        if let Err(error) = result {
            self.error = Some(error);
            return false;
        }
        if let Some(context) = context {
            self.memory_context = context;
            self.scope_context_pending = false;
        }
        self.last_activity = Instant::now();
        match kind {
            "set_model" => {
                self.operation_deadline =
                    Some(("set_model", Instant::now() + Duration::from_secs(30)))
            }
            "set_thinking_level" => {
                self.operation_deadline = Some((
                    "set_thinking_level",
                    Instant::now() + Duration::from_secs(30),
                ))
            }
            "abort" => {
                self.operation_deadline = Some(("abort", Instant::now() + Duration::from_secs(10)))
            }
            _ => {}
        }
        true
    }

    pub(crate) fn refresh_estimated_breakdown(&mut self) {
        if self.harness == "claude" {
            return;
        }
        let Some(used_tokens) = self.context.usage.tokens.filter(|tokens| *tokens > 0) else {
            self.context.usage.breakdown = None;
            return;
        };
        let totals = crate::context_breakdown::thread_char_totals(
            &self.messages,
            &self.tools,
            &self.thoughts,
            &self.native.compactions,
        );
        self.context.usage.breakdown =
            crate::context_breakdown::estimate_context_breakdown(&totals, used_tokens);
    }

    /// Asks Pi for the current context window. Event-driven only: startup, a completed turn,
    /// a model change and compaction completion call this. Nothing polls on a timer.
    fn refresh_context_usage(&mut self) -> bool {
        if self.pi.is_none()
            || self.configuring
            || self.context.is_pending()
            || self.context.phase == crate::context::ContextPhase::Compacting
        {
            return false;
        }
        // Telemetry transport errors must not replace the conversation's error.
        let previous_error = self.error.clone();
        if !self.command("get_session_stats", json!({})) {
            self.error = previous_error;
            return false;
        }
        self.context
            .requested(format!("{}-{}", self.id, self.serial));
        true
    }

    #[cfg(test)]
    pub fn send(&mut self) {
        self.send_with_fresh(None, &[], &str::to_owned, false);
    }

    pub(crate) fn send_with_fresh(
        &mut self,
        target_server_id: Option<&str>,
        peers: &[crate::peer::Peer],
        machine_name: &dyn Fn(&str) -> String,
        fresh_context: bool,
    ) {
        if self.offline_blocked || self.status == Status::Paused {
            return;
        }
        if self.status.busy() || self.configuring || !self.has_draft_content() {
            return;
        }
        if self.harness == "pi" && !self.model_provider.is_empty() && self.model_provider != "cpa" {
            self.error = Some("Choose a CPA model before sending. Shina will not send prompts to a direct provider.".into());
            return;
        }
        if self.draft_images.iter().any(Attachment::is_image) && self.supports_images == Some(false)
        {
            self.attachment_error = Some(self.image_model_error());
            return;
        }
        let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
            &self.draft,
            &self.draft_images,
            &self.draft_context,
            self.draft_reply_to.as_ref(),
            Some(&self.machine),
            target_server_id,
            peers,
            machine_name,
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.attachment_error = Some(error);
                return;
            }
        };
        self.prompt_history.reset();
        self.typing_guard.on_blur_or_send();
        self.paste_history = Default::default();
        self.draft.replace_text("");
        self.draft_images.clear();
        self.draft_context.clear();
        self.draft_reply_to = None;
        self.retained_context = Default::default();
        self.native.queue_held = false;
        self.native.access = resolve_compatible_access_mode(
            self.native.access,
            supported_access_modes(&self.harness),
        )
        .1;
        self.begin_prompt_with_fresh(prompt, fresh_context);
    }

    pub fn effective_start_from_origin(&self) -> bool {
        self.start_from_origin.unwrap_or(true)
    }

    pub fn worktree_bootstrapping(&self) -> bool {
        self.worktree_bootstrap.is_some()
    }

    pub fn reconcile_draft_context(&mut self) {
        let referenced = crate::composer_tokens::collect_inline_context_ids_from_edit(&self.draft);
        crate::composer_context::reconcile_context_references(
            &referenced,
            &mut self.draft_context,
            &mut self.draft_images,
            &mut self.retained_context,
        );
    }

    pub fn validate_prompt_length(&self, prompt: &Prompt) -> Option<String> {
        let expanded =
            crate::attachments::pi_message(prompt).unwrap_or_else(|_| prompt.text.clone());
        crate::composer::prompt_length_validation_message(&expanded)
    }

    pub fn update_submission_error(&mut self) {
        if self.submission_error.is_none() {
            return;
        }
        let expanded = crate::composer_tokens::prompt_for_with_context(
            &self.draft,
            &self.draft_images,
            &self.draft_context,
            Some(&self.machine),
            &str::to_owned,
        )
        .ok()
        .and_then(|p| crate::attachments::pi_message(&p).ok())
        .unwrap_or_else(|| crate::composer_rich::markdown(&self.draft));
        self.submission_error = crate::composer::prompt_length_validation_message(&expanded);
    }

    pub(super) fn begin_prompt(&mut self, prompt: Prompt) {
        self.begin_prompt_with_fresh(prompt, false);
    }

    pub(super) fn begin_prompt_with_fresh(&mut self, prompt: Prompt, fresh_context: bool) {
        if fresh_context {
            let held_queue = !self.native.queue.is_empty();
            let previous_held = self.native.queue_held;
            if held_queue {
                self.native.queue_held = true;
            }
            if self.pi.is_some() {
                self.disconnect();
            }
            if held_queue {
                self.native.queue_held = previous_held;
            }
            self.session = None;
            self.harness_context_sent = false;
            self.native.fork_source = None;
            self.native.fork_entry = None;
            self.native.previous_session = None;
            self.native.session_name = None;
            self.context.begin_model_change();
        }
        self.run_id = Some(uuid::Uuid::new_v4().to_string());
        self.resumable_run_id = None;
        self.continuation_of_run_id = None;
        self.prompt_suggestion = None;
        self.stop_control = StopControl::default();
        self.native.initial_user = true;
        self.queued_prompt = Some((prompt.clone(), self.messages.len()));
        self.retry_prompt = Some(prompt.clone());
        self.arm_task();
        if self.messages.is_empty() && !self.captain_home && !self.renamed {
            self.title = if prompt.text.is_empty() {
                prompt
                    .images
                    .first()
                    .map(|image| {
                        format!(
                            "{}: {}",
                            if image.is_image() { "Image" } else { "File" },
                            image.name
                        )
                    })
                    .unwrap_or_else(|| "New thread".into())
                    .chars()
                    .take(45)
                    .collect()
            } else {
                prompt.text.chars().take(45).collect()
            };
        }
        let message_id = uuid::Uuid::new_v4().to_string();
        if fresh_context {
            self.fresh_session_messages.push(message_id.clone());
        }
        self.messages.push(Message {
            id: message_id,
            role: "You",
            text: prompt.text.clone(),
            images: prompt.images.to_vec(),
            context: prompt.context.clone(),
        });
        self.attachment_error = None;
        self.status = Status::Running;
        self.updated = crate::projects::now();
        self.life.user_sent(lifecycle::now());
        self.started = Some(Instant::now());
        self.elapsed = None;
        self.assistant = None;
        self.ensure_work();
        self.error = None;
        self.phase = "Working".into();
        self.last_activity = Instant::now();
        self.drive_session();
    }

    pub fn stop(&mut self) {
        if self.remote.is_some() && !self.run_control_supported {
            self.error = Some(crate::protocol::RUN_CONTROL_UPDATE_REQUIRED.into());
            return;
        }
        if self.stop_control.disabled() && self.stop_control.target() == self.run_id.as_deref() {
            return;
        }
        self.begin_stop();
        self.stop_serial = self.stop_serial.saturating_add(1);
        if self.forward("thread.stop", json!({"runId":self.run_id})) {
            return;
        }
        let _ = self.stop_task();
    }

    fn stop_task(&mut self) -> Result<(), String> {
        if self.status.busy()
            && !self.initializing
            && self.queued_prompt.is_none()
            && self.pending_prompt.is_none()
            && !self.has_pending_sends()
        {
            self.mark_stopped();
        }
        self.memory_staged.clear();
        self.memory_context.clear();
        self.scope_context_pending = true;
        self.native.queue_held = true;
        self.pending_injections.clear();
        self.injection_id = None;
        self.injection_retry_at = None;
        self.operation_deadline = None;
        if self.initializing
            || self.queued_prompt.is_some()
            || self.native.operating() && !matches!(self.harness.as_str(), "codex" | "claude")
            || self.has_pending_sends()
            || self.context.phase == crate::context::ContextPhase::Compacting
                && !matches!(self.harness.as_str(), "codex" | "claude")
        {
            self.disconnect();
            self.complete_task("cancelled");
            return Ok(());
        }
        if self.status == Status::Running || self.status == Status::Waiting {
            // Release a blocking extension dialog before asking Pi to wait for idle.
            self.cancel_native_dialogs();
            if !self.command("clear_queue", json!({})) || !self.command("abort", json!({})) {
                let error = self
                    .error
                    .clone()
                    .unwrap_or_else(|| "Pi could not stop the task.".into());
                self.disconnect();
                self.complete_task("cancelled");
                self.error = Some(error.clone());
                return Err(error);
            }
            self.phase = "Stopping".into();
        } else {
            self.complete_task("cancelled");
        }
        Ok(())
    }

    pub fn respond(&mut self, mut response: Value) {
        if self.responding_request.is_some() {
            return;
        }
        if self.native.questionnaire.is_some() && !self.question_contract_supported {
            self.error = Some(crate::protocol::QUESTIONS_UPDATE_REQUIRED.into());
            return;
        }
        if self
            .native
            .questionnaire
            .as_ref()
            .is_some_and(|form| !form.can_respond())
        {
            self.error = Some("This request is not resumable.".into());
            return;
        }
        if let Some(request) = &self.request {
            if !crate::approval::can_respond(request) {
                self.error = Some("This request is not resumable.".into());
                return;
            }
            if request["method"] == "confirm" {
                if let Err(error) = crate::approval::validate(request, &response) {
                    self.error = Some(error);
                    return;
                }
                let decision = crate::approval::decision(&response)
                    .expect("validated")
                    .to_owned();
                let feedback = response.get("feedback").cloned();
                response = crate::approval::response(&decision);
                if let Some(feedback) = feedback {
                    response["feedback"] = feedback;
                }
                if self.remote.is_none() && self.harness == "pi" && decision == "acceptAlways" {
                    response = json!({"cancelled":true});
                }
            }
        }
        if self.remote.is_some() {
            if !self.requests_supported {
                self.error = Some(crate::protocol::REQUESTS_UPDATE_REQUIRED.into());
                return;
            }
            let request = self.request.clone().unwrap_or(Value::Null);
            let params = json!({"threadId":self.id,"requestId":request["id"],"generation":request["runtimeGeneration"],
                "requestContract":true,"questionContract":self.native.questionnaire.is_some(),
                "answers":self.native.questionnaire.as_ref().map(|form| json!(form.answers)),"response":response});
            match self
                .remote
                .as_ref()
                .expect("remote")
                .send("request.respond", params)
            {
                Ok(id) => self.responding_request = Some(id),
                Err(error) => self.error = Some(error),
            }
            return;
        }
        if self.respond_message_question(&response) {
            return;
        }
        if response["cancelled"] == true {
            self.cancel_questionnaire();
        }
        let Some(request) = &self.request else {
            return;
        };
        if request["method"] == "confirm" {
            let request = request.clone();
            match self.write_approval_response(&request, response) {
                Ok(()) => {
                    self.request = None;
                    self.forwarded_question = None;
                    self.native_dialog_done();
                }
                Err(error) => self.error = Some(error),
            }
            return;
        }
        if response.get("feedback").is_some() {
            self.error = Some(
                "Feedback requires a live approval that supports it and a decline decision.".into(),
            );
            return;
        }
        response["type"] = "extension_ui_response".into();
        response["id"] = request["id"].clone();
        if self.harness == "pi"
            && let Some(fields) = response.as_object_mut()
        {
            // Pi's extension transport consumes only its boolean reply, not our contract.
            fields.remove("decision");
            fields.remove("allowForSession");
        }
        let result = self
            .pi
            .as_ref()
            .ok_or_else(|| "The session is unavailable. Retry to resume it.".to_owned())
            .and_then(|pi| crate::harness::send(pi, response));
        match result {
            Ok(()) => {
                self.request = None;
                self.forwarded_question = None;
                self.native_dialog_done();
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn disconnect(&mut self) {
        self.memory_pending.clear();
        self.learned_turn = None;
        self.memory_staged.clear();
        self.native_disconnect();
        self.finish_thoughts();
        if let Some((prompt, index)) = self.queued_prompt.take() {
            self.remove_optimistic_prompt(&prompt, index);
            self.restore_prompt(&prompt);
        }
        if let Some((_, prompt, index)) = self.pending_prompt.take() {
            self.remove_optimistic_prompt(&prompt, index);
            self.restore_prompt(&prompt);
        }
        self.pi = None;
        self.status = Status::Ready;
        self.assistant = None;
        if self.native.questionnaire.is_none() {
            self.request = None;
        }
        self.forwarded_question = None;
        self.initializing = false;
        self.session_started = None;
        self.configuring = false;
        self.pending_thinking = None;
        self.injection_id = None;
        self.injection_retry_at = None;
        self.operation_deadline = None;
        self.context.cancel_request();
        self.started = None;
        self.phase.clear();
    }

    fn arm_task(&mut self) {
        if self.delegated {
            self.task_result = None;
            self.completion_notified = false;
        }
    }

    fn complete_task(&mut self, result: &'static str) {
        if self.delegated && !self.completion_notified && self.task_result.is_none() {
            self.task_result = Some(result);
        }
    }

    fn enqueue_injection(&mut self, text: String) {
        self.serial += 1;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.pending_injections.push_back(Injection {
            text: format!(
                "{INJECTION_PREFIX}[delivery {}-{}-{stamp}]\n{text}",
                self.id, self.serial
            ),
            shown: false,
        });
    }

    fn flush_injection(&mut self) -> bool {
        if self.offline_blocked || self.status == Status::Paused {
            return false;
        }
        if self
            .injection_retry_at
            .is_some_and(|at| Instant::now() >= at)
        {
            self.injection_retry_at = None;
        }
        if self.pending_injections.is_empty()
            || self.pi.is_none()
            || self.initializing
            || self.queued_prompt.is_some()
            || self.injection_id.is_some()
            || self.configuring
            || self.context.phase == crate::context::ContextPhase::Compacting
            || self
                .injection_retry_at
                .is_some_and(|at| Instant::now() < at)
            || self.phase == "Stopping"
            || !matches!(
                self.status,
                Status::Ready | Status::Completed | Status::Running | Status::Waiting
            )
        {
            return false;
        }
        if self.harness == "pi" && self.model_provider != "cpa" {
            self.error =
                Some("Choose a CPA model before Shina can deliver queued messages.".into());
            return false;
        }
        let text = self.pending_injections[0].text.clone();
        if !self.status.busy() || self.run_id.is_none() {
            self.run_id = Some(uuid::Uuid::new_v4().to_string());
            self.prompt_suggestion = None;
        }
        // prompt starts an idle Pi; followUp also covers a run starting before Pi reads it.
        if !self.command(
            "prompt",
            json!({"message":text,"streamingBehavior":"followUp"}),
        ) {
            return false;
        }
        self.injection_id = Some(format!("{}-{}", self.id, self.serial));
        let at = lifecycle::now();
        if self.life.settled() {
            self.life.unsettle(at, false);
        }
        self.life.last_requested_at = Some(at);
        if !self.pending_injections[0].shown {
            self.messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "Shina",
                text,
                images: Vec::new(),
                context: None,
            });
            self.pending_injections[0].shown = true;
        }
        if !self.status.busy() {
            self.arm_task();
            self.status = Status::Running;
            self.started = Some(Instant::now());
            self.elapsed = None;
            self.assistant = None;
            self.error = None;
            self.phase = if self.harness == "pi" {
                "Waiting for Pi"
            } else {
                "Waiting for ACP"
            }
            .into();
            self.ensure_work();
        }
        true
    }

    fn diagnostic(&mut self, text: &str) {
        const LIMIT: usize = 128;
        if self.diagnostics.len() >= LIMIT {
            self.diagnostics.drain(..self.diagnostics.len() + 1 - LIMIT);
        }
        self.diagnostics
            .push(crate::update::redact(&crate::projects::redact(&truncate(
                text, 2048,
            ))));
    }

    fn check_deadlines(&mut self, now: Instant) -> bool {
        if self.request.is_some() {
            return false;
        }
        let Some((operation, deadline)) = self.operation_deadline else {
            return false;
        };
        if now < deadline {
            return false;
        }
        if operation == "abort" {
            self.operation_deadline = None;
            self.tick_stop(now);
        } else {
            self.fail_session(format!(
                "Pi did not answer the {operation} operation before its deadline. Retry to resume."
            ));
        }
        true
    }

    fn remove_optimistic_prompt(&mut self, prompt: &Prompt, index: usize) {
        if self.messages.get(index).is_some_and(|message| {
            message.role == "You"
                && message.text == prompt.text
                && message.images.as_slice() == prompt.images.as_ref()
        }) {
            self.messages.remove(index);
        }
    }
    fn reject_images(&mut self, prompt: &Prompt, index: usize, error: String) {
        self.restore_prompt(prompt);
        self.remove_optimistic_prompt(prompt, index);
        self.attachment_error = Some(error);
        self.status = Status::Ready;
        self.started = None;
        self.elapsed = None;
        self.assistant = None;
        self.phase.clear();
    }

    fn fail_session(&mut self, error: String) {
        self.finish_retry(Some(&error));
        self.memory_staged.clear();
        self.memory_context.clear();
        self.scope_context_pending = true;
        self.native.queue_held = true;
        self.disconnect();
        self.error = Some(error);
        self.status = Status::Failed;
        self.life.failed_at = Some(lifecycle::now());
        self.complete_task("failed");
    }

    /// Settings and history are transport details, never conversation states.
    fn drive_session(&mut self) {
        if self.pi.is_none()
            || self.configuring
            || matches!(self.status, Status::Failed | Status::Paused)
            || self.offline_blocked
        {
            return;
        }
        if self.initializing {
            if !(self.state_loaded && self.history_loaded) {
                return;
            }
            if !self.startup_model_applied {
                self.startup_model_applied = true;
                if let Some((provider, id)) = self.selected_model.clone() {
                    self.configuring =
                        self.command("set_model", json!({"provider":provider,"modelId":id}));
                    if !self.configuring {
                        self.fail_session(self.error.clone().unwrap());
                    }
                    return;
                }
            }
            if !self.startup_thinking_applied {
                self.startup_thinking_applied = true;
                if let Some(level) = self.selected_thinking.clone() {
                    self.pending_thinking = Some(level.clone());
                    self.configuring = self.command("set_thinking_level", json!({"level":level}));
                    if !self.configuring {
                        self.fail_session(self.error.clone().unwrap());
                    }
                    return;
                }
            }
            // Pi acknowledges set_thinking_level without returning the applied level.
            // Verify its state before delivering a delegated brief, since Pi can clamp it.
            if self.delegated && self.selected_model.is_some() && !self.startup_selection_verified {
                if self.startup_selection_check.is_none() {
                    if self.command("get_state", json!({})) {
                        self.startup_selection_check = Some(format!("{}-{}", self.id, self.serial));
                    } else {
                        self.fail_session(
                            self.error.clone().unwrap_or_else(|| {
                                "Pi did not accept selection verification.".into()
                            }),
                        );
                    }
                }
                return;
            }
            self.initializing = false;
            self.session_started = None;
            if self.queued_prompt.is_none()
                && self.started.is_none()
                && self.status == Status::Running
            {
                self.status = Status::Ready;
                self.phase.clear();
            }
        }
        if self.drive_native() {
            return;
        }
        // Pi has answered its session handshake, so it can report the context window it
        // resumed. A directly wired transport in a test has not answered that handshake.
        if !self.context_startup_requested && self.state_loaded && self.history_loaded {
            self.context_startup_requested = true;
            self.refresh_context_usage();
        }
        let Some((prompt, index)) = self.queued_prompt.take() else {
            return;
        };
        if self.harness == "pi" && self.model_provider != "cpa" {
            self.queued_prompt = Some((prompt, index));
            self.fail_session("Choose a CPA model before sending. Shina will not send prompts to a direct provider.".into());
            return;
        }
        if prompt.images.iter().any(Attachment::is_image) && self.supports_images != Some(true) {
            let error = self.image_model_error();
            self.reject_images(&prompt, index, error);
            return;
        }
        let mut prompt = prompt;
        if self.harness == "claude" {
            let effort = self
                .model_options
                .iter()
                .find(|o| o.id == "effort")
                .and_then(|o| o.value.as_str());
            let caps = self
                .models
                .iter()
                .find(|m| m.provider == self.model_provider && m.id == self.model_id)
                .map(|m| m.capabilities.clone())
                .unwrap_or_default();
            let injected = crate::descriptors::resolve_prompt_injected_effort(&caps, effort);
            prompt.text = crate::descriptors::apply_claude_prompt_effort_prefix(
                &prompt.text,
                injected.as_deref().or(effort),
            );
        }
        if self.harness == "pi"
            && let Some(tier) = self
                .model_options
                .iter()
                .find(|o| o.id == "serviceTier")
                .and_then(|o| o.value.as_str())
                .filter(|&t| t != "default" && !t.is_empty())
        {
            let _ = self.command(
                "prompt",
                json!({"message": format!("/shina-service-tier {tier}")}),
            );
        }
        let record = match attachments::prompt_record(&prompt, self.harness == "pi") {
            Ok(record) => record,
            Err(error) => {
                self.reject_images(&prompt, index, error);
                return;
            }
        };
        if self.command("prompt", record) {
            self.pending_prompt = Some((format!("{}-{}", self.id, self.serial), prompt, index));
        } else {
            self.queued_prompt = Some((prompt, index));
            self.fail_session(
                self.error
                    .clone()
                    .unwrap_or_else(|| "Pi did not accept the message. Retry to resume.".into()),
            );
        }
    }

    fn refresh_thinking_levels(&mut self) {
        if let Some(model) = self
            .models
            .iter()
            .find(|m| m.provider == self.model_provider && m.id == self.model_id)
        {
            if let Some(reasoning) = model
                .capabilities
                .option_descriptors
                .iter()
                .find(|d| crate::descriptors::is_reasoning_control(d))
            {
                let choices = crate::descriptors::selectable_provider_option_choices(reasoning);
                if !choices.is_empty() {
                    self.thinking_levels = choices.into_iter().map(|c| c.id).collect();
                    return;
                }
            }
            self.thinking_levels = model.thinking_levels.clone();
        }
    }

    pub fn restoring_history(&self) -> bool {
        self.initializing
            && !self.history_loaded
            && (self.session.is_some() && !self.never_sent || self.native.fork_source.is_some())
    }

    fn reclaim_idle(&mut self, now: Instant) -> bool {
        if self.pi.is_some()
            && !self.initializing
            && !self.configuring
            && !self.status.busy()
            && self.pending_injections.is_empty()
            && self.injection_id.is_none()
            && now.duration_since(self.last_activity) >= IDLE_TIMEOUT
        {
            let status = self.status;
            self.disconnect();
            self.status = status;
            return true;
        }
        false
    }

    fn assistant(&mut self) -> &mut Message {
        self.ensure_work();
        let index = *self.assistant.get_or_insert_with(|| {
            self.messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "Pi",
                text: String::new(),
                images: Vec::new(),
                context: None,
            });
            self.messages.len() - 1
        });
        &mut self.messages[index]
    }

    fn ensure_work(&mut self) -> &mut AssistantWork {
        let start = self
            .messages
            .iter()
            .rposition(|message| message.role != "Pi")
            .map_or(0, |index| index + 1);
        if self.work.last().is_none_or(|work| work.start != start) {
            // A finished turn folds its steps into one "Worked for" line, as Cursor does.
            self.work.push(AssistantWork {
                start,
                stopped: false,
                stopped_order: 0,
                expanded: false,
                elapsed: None,
                outcome: None,
            });
        }
        self.work.last_mut().unwrap()
    }

    fn thought(&mut self, block: usize) -> &mut Thought {
        self.assistant();
        let message = self.assistant.unwrap();
        let order = self.next_work_order();
        let index = self
            .thoughts
            .iter()
            .position(|thought| thought.anchor.message == message && thought.block == block)
            .unwrap_or_else(|| {
                self.thoughts.push(Thought {
                    anchor: ToolAnchor {
                        message,
                        offset: self.messages[message].text.len(),
                    },
                    block,
                    order,
                    text: String::new(),
                    expanded: false,
                    done: false,
                    started: None,
                    duration_ms: None,
                });
                self.thoughts.len() - 1
            });
        &mut self.thoughts[index]
    }

    /// Reuse Pi's content-block order for both restored history and completed live messages.
    /// Keep Thread.tools flat for existing consumers and record its transcript positions.
    fn anchor_content_work(&mut self, content: &Value, message: usize, restored: bool) {
        let Some(blocks) = content.as_array() else {
            return;
        };
        let mut offset = 0;
        let mut texts = 0;
        for (block_index, block) in blocks.iter().enumerate() {
            if block["type"] == "text" {
                if let Some(text) = block["text"].as_str() {
                    offset += text.len() + usize::from(texts > 0);
                    texts += 1;
                }
            } else if block["type"] == "toolCall" {
                let anchor = ToolAnchor { message, offset };
                let id = string(block, "id");
                if !restored
                    && let Some(tool) = self
                        .tools
                        .iter_mut()
                        .rev()
                        .find(|tool| tool.id == id && tool.anchor.message == message)
                {
                    tool.anchor = anchor;
                    tool.block = block_index;
                } else {
                    let mut tool =
                        Tool::new(id, string(block, "name"), &block["arguments"], anchor);
                    tool.accept_metadata(block);
                    tool.order = self.next_work_order();
                    tool.block = block_index;
                    tool.parent_call = block["parentToolCallId"].as_str().map(str::to_owned);
                    if restored {
                        tool.done = true;
                        tool.failed = true;
                        tool.output = "No saved result for this tool call.".into();
                    }
                    self.tools.push(tool);
                }
            } else if block["type"] == "thinking" {
                let order = self.next_work_order();
                let index = self
                    .thoughts
                    .iter()
                    .position(|thought| {
                        thought.anchor.message == message && thought.block == block_index
                    })
                    .unwrap_or_else(|| {
                        self.thoughts.push(Thought {
                            anchor: ToolAnchor { message, offset },
                            block: block_index,
                            order,
                            text: String::new(),
                            expanded: false,
                            done: true,
                            started: None,
                            duration_ms: None,
                        });
                        self.thoughts.len() - 1
                    });
                let thought = &mut self.thoughts[index];
                thought.anchor = ToolAnchor { message, offset };
                thought.text = string(block, "thinking").into();
                thought.done = true;
                thought.duration_ms = thought.duration_ms.or_else(|| {
                    thought
                        .started
                        .map(|start| start.elapsed().as_millis() as u64)
                });
            }
        }
    }

    pub fn apply(&mut self, record: Value) {
        self.observe_task_history(&record);
        self.last_activity = Instant::now();
        if self.apply_native(&record) {
            self.drive_session();
            return;
        }
        match string(&record, "type") {
            "steering_restart" => {
                let held = self.native.queue_held;
                self.mark_stopped();
                self.native.queue_held = held;
                self.resumable_run_id = None;
            }
            "steer_delivery_unknown" => {
                let mut images = content_images(&record["message"]["content"]);
                images.extend(
                    record["attachments"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Attachment::from_value),
                );
                self.messages.push(Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "You",
                    text: string(&record, "text").into(),
                    images,
                    context: None,
                });
                self.error = Some("ACP steering delivery could not be confirmed. The provider may have accepted this message, so Shina did not resend it.".into());
            }
            "harness_update" => {
                if record["field"] == "awareness" {
                    self.harness_context_sent = record["value"] == true;
                } else if let Some(field) = record["field"].as_str() {
                    self.harness_state[field] = record["value"].clone();
                    if field == "optionDescriptors"
                        && let Ok(descriptors) = serde_json::from_value::<
                            Vec<crate::descriptors::ProviderOptionDescriptor>,
                        >(record["value"].clone())
                    {
                        for model in &mut self.models {
                            model.capabilities.option_descriptors = descriptors.clone();
                        }
                        self.refresh_thinking_levels();
                    }
                    if matches!(field, "plan" | "plan_update") {
                        self.record_plan(&record["value"]);
                    }
                }
            }
            "response" => {
                let command = string(&record, "command");
                if self
                    .operation_deadline
                    .is_some_and(|(kind, _)| kind == command && kind != "abort")
                {
                    self.operation_deadline = None;
                }
                let injection = command == "prompt"
                    && self
                        .injection_id
                        .as_ref()
                        .is_some_and(|id| record["id"] == *id);
                if injection {
                    self.injection_id = None;
                    if record["success"] != false {
                        self.pending_injections.pop_front();
                        self.injection_retry_at = None;
                    } else if string(&record, "error")
                        .to_ascii_lowercase()
                        .contains("compaction")
                    {
                        self.injection_retry_at = Some(Instant::now() + Duration::from_secs(1));
                        self.diagnostic(
                            "Completion delivery deferred until Pi finishes compaction.",
                        );
                        return;
                    }
                }
                // Context stats are optional telemetry. A rejected or stale request must never
                // surface as a conversation error, and it must not disturb configuring.
                if command == "get_session_stats" {
                    if self.context.accept_response(&record) {
                        self.refresh_estimated_breakdown();
                    }
                    self.drive_session();
                    return;
                }
                if record["success"] == false {
                    if command == "prompt" {
                        self.memory_staged.clear();
                        self.memory_context.clear();
                        self.scope_context_pending = true;
                    }
                    self.error = Some(string(&record, "error").to_owned());
                    if self.initializing {
                        self.fail_session(self.error.clone().unwrap());
                        return;
                    }
                    if matches!(command, "set_model" | "set_thinking_level") {
                        self.configuring = false;
                        self.pending_thinking = None;
                    }
                    if command == "prompt"
                        && self
                            .pending_prompt
                            .as_ref()
                            .is_some_and(|(id, _, _)| record["id"] == *id)
                        && let Some((_, prompt, index)) = self.pending_prompt.take()
                    {
                        self.restore_prompt(&prompt);
                        self.remove_optimistic_prompt(&prompt, index);
                    }
                    if matches!(command, "prompt" | "get_state" | "get_messages") {
                        self.native.queue_held = true;
                        self.status = Status::Failed;
                        self.started = None;
                        self.complete_task("failed");
                    }
                    return;
                }
                match command {
                    "get_state" => {
                        let data = &record["data"];
                        if data["turnCapabilities"].is_object() {
                            self.harness_state["turnCapabilities"] =
                                data["turnCapabilities"].clone();
                        }
                        if self
                            .startup_selection_check
                            .as_ref()
                            .is_some_and(|id| record["id"] == *id)
                        {
                            self.startup_selection_check = None;
                            let model_matches =
                                self.selected_model
                                    .as_ref()
                                    .is_none_or(|(provider, model)| {
                                        data["model"]["provider"] == *provider
                                            && data["model"]["id"] == *model
                                    });
                            let reasoning_matches = self
                                .selected_thinking
                                .as_ref()
                                .is_none_or(|level| data["thinkingLevel"] == *level);
                            if !model_matches || !reasoning_matches {
                                self.fail_session(format!(
                                    "Pi applied model {}/{} with reasoning {} instead of the delegated selection {}/{} with reasoning {}. Shina did not send the task.",
                                    string(&data["model"], "provider"),
                                    string(&data["model"], "id"),
                                    string(data, "thinkingLevel"),
                                    self.selected_model.as_ref().map_or("", |v| v.0.as_str()),
                                    self.selected_model.as_ref().map_or("", |v| v.1.as_str()),
                                    self.selected_thinking.as_deref().unwrap_or("default"),
                                ));
                                return;
                            }
                            self.startup_selection_verified = true;
                        }
                        if !self.initializing || self.selected_model.is_none() {
                            self.update_model(&data["model"]);
                        }
                        if !self.initializing || self.selected_thinking.is_none() {
                            self.thinking = string(data, "thinkingLevel").to_owned();
                        }
                        self.session = data["sessionFile"].as_str().map(str::to_owned);
                        if record["id"] == "state" {
                            self.state_loaded = true;
                            if data["isStreaming"] == true {
                                self.status = Status::Running;
                                self.started.get_or_insert_with(Instant::now);
                            }
                        }
                    }
                    "get_available_models" => {
                        self.models = record["data"]["models"]
                            .as_array()
                            .map(|models| {
                                models
                                    .iter()
                                    .filter_map(|model| {
                                        AgentModel::from_harness_value(model, &self.harness)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        self.refresh_thinking_levels();
                    }
                    "get_available_thinking_levels" => {
                        self.thinking_levels = record["data"]["levels"]
                            .as_array()
                            .map(|levels| {
                                levels
                                    .iter()
                                    .filter_map(|level| level.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default();
                    }
                    "set_model" => {
                        self.update_model(&record["data"]);
                        self.refresh_thinking_levels();
                        // Refresh the effective level too. Pi may clamp it for the new model.
                        self.command("get_state", json!({}));
                        self.command("get_available_thinking_levels", json!({}));
                        self.configuring = false;
                        // Capacity and the previous percentage belonged to the old model.
                        self.refresh_context_usage();
                        if let Some(confirmed) = self.thread_model_selection() {
                            crate::model_selection::release_model_selection(
                                &mut self.draft_model_state,
                                &confirmed,
                            );
                        }
                    }
                    "set_thinking_level" => {
                        if let Some(level) = self.pending_thinking.take() {
                            self.thinking = level;
                        }
                        self.command("get_state", json!({}));
                        self.configuring = false;
                        if let Some(confirmed) = self.thread_model_selection() {
                            crate::model_selection::release_model_selection(
                                &mut self.draft_model_state,
                                &confirmed,
                            );
                        }
                    }
                    "get_messages" if record["id"] == "history" => {
                        if self.server_owned
                            && self.harness != "pi"
                            && record["data"]["replaceHistory"] != true
                            && record["data"]["messages"]
                                .as_array()
                                .is_some_and(|messages| {
                                    messages.is_empty()
                                        || record["data"]["historyReplayed"] == false
                                })
                            && !self.messages.is_empty()
                        {
                            // ACP resume need not replay history. Keep the server transcript
                            // and the existing optimistic prompt index in that case.
                            self.recover_native_inputs();
                            self.history_loaded = true;
                            self.drive_session();
                            return;
                        }
                        if let Some(messages) = record["data"]["messages"].as_array() {
                            let fresh_boundary = self
                                .messages
                                .iter()
                                .rposition(|m| self.is_fresh_session_message(&m.id));
                            if let Some(b) = fresh_boundary
                                && messages.is_empty()
                                && record["data"]["replaceHistory"] != true
                                && self.queued_prompt.is_none()
                                && self.pending_prompt.is_none()
                                && self.messages.len() > b
                            {
                                self.recover_native_inputs();
                                self.history_loaded = true;
                                self.drive_session();
                                return;
                            }
                            let saved_queued_id = self
                                .queued_prompt
                                .as_ref()
                                .and_then(|(_, idx)| self.messages.get(*idx).map(|m| m.id.clone()));
                            let _ = &saved_queued_id;
                            let saved_work = self.work.clone();
                            let saved_orders: std::collections::HashMap<_, _> = self
                                .tools
                                .iter()
                                .filter(|t| t.order > 0)
                                .map(|t| (t.id.clone(), t.order))
                                .collect();
                            let saved_thoughts: Vec<_> = self
                                .thoughts
                                .iter()
                                .filter(|t| t.order > 0)
                                .map(|t| (t.anchor, t.block, t.order))
                                .collect();
                            let mut saved_messages = std::mem::take(&mut self.messages);
                            let mut prefix_messages = Vec::new();
                            let mut prefix_tools = Vec::new();
                            let mut prefix_thoughts = Vec::new();
                            let mut prefix_work = Vec::new();
                            let mut prefix_compactions = Vec::new();
                            let message_offset = fresh_boundary.unwrap_or(0);
                            if let Some(b) = fresh_boundary {
                                let keep = b.min(saved_messages.len());
                                prefix_messages = saved_messages.drain(..keep).collect();
                                for tool in std::mem::take(&mut self.tools) {
                                    if tool.anchor.message < keep {
                                        prefix_tools.push(tool);
                                    }
                                }
                                for thought in std::mem::take(&mut self.thoughts) {
                                    if thought.anchor.message < keep {
                                        prefix_thoughts.push(thought);
                                    }
                                }
                                for work in &saved_work {
                                    if work.start < keep {
                                        prefix_work.push(work.clone());
                                    }
                                }
                                for compaction in std::mem::take(&mut self.native.compactions) {
                                    if compaction["anchor"]["message"]
                                        .as_u64()
                                        .is_some_and(|m| (m as usize) < keep)
                                    {
                                        prefix_compactions.push(compaction);
                                    }
                                }
                            } else {
                                self.tools.clear();
                                self.thoughts.clear();
                                self.native.compactions.clear();
                            }
                            self.work.clear();
                            self.assistant = None;
                            if let Some(points) = record["data"]["forkMessages"].as_array() {
                                self.native.fork_points = points.clone();
                            }
                            if let Some(compactions) = record["data"]["compactions"].as_array() {
                                self.native.compactions = compactions
                                    .iter()
                                    .rev()
                                    .take(32)
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .into_iter()
                                    .rev()
                                    .collect();
                            }
                            let messages = approval_send::history_with_approval_feedback(
                                messages,
                                &saved_messages,
                            );
                            let mut native_history_index = 0;
                            for message in &messages {
                                if message["shinaFeedbackId"].is_null() {
                                    self.anchor_compactions(native_history_index);
                                    native_history_index += 1;
                                }
                                let role = string(message, "role");
                                let index = self.messages.len();
                                if matches!(role, "user" | "assistant") {
                                    let text = content_text(&message["content"]);
                                    let images = if role == "user" {
                                        content_images(&message["content"])
                                    } else {
                                        Vec::new()
                                    };
                                    if role == "assistant" || !text.is_empty() || !images.is_empty()
                                    {
                                        if role == "assistant" {
                                            self.ensure_work();
                                        }
                                        self.messages.push(Message {
                                            id: message["shinaFeedbackId"]
                                                .as_str()
                                                .map(str::to_owned)
                                                .unwrap_or_else(|| {
                                                    uuid::Uuid::new_v4().to_string()
                                                }),
                                            role: if role != "user" {
                                                "Pi"
                                            } else if text.starts_with(INJECTION_PREFIX) {
                                                "Shina"
                                            } else {
                                                "You"
                                            },
                                            text,
                                            images,
                                            context: None,
                                        });
                                    }
                                }
                                if role == "custom" && message["display"] != false {
                                    self.messages.push(Message {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        role: "Shina",
                                        text: content_text(&message["content"]),
                                        images: content_images(&message["content"]),
                                        context: None,
                                    });
                                }
                                if role == "assistant" {
                                    self.anchor_content_work(&message["content"], index, true);
                                    if message["stopReason"] == "aborted" {
                                        let order = self.next_work_order();
                                        self.ensure_work().stopped = true;
                                        self.ensure_work().stopped_order = order;
                                    } else if matches!(
                                        message["stopReason"].as_str(),
                                        Some("stop" | "length")
                                    ) {
                                        self.ensure_work().outcome = Some(Status::Completed);
                                    } else if message["stopReason"] == "error" {
                                        self.ensure_work().outcome = Some(Status::Failed);
                                    }
                                }
                                if role == "toolResult"
                                    && let Some(tool) = self
                                        .tools
                                        .iter_mut()
                                        .rev()
                                        .find(|t| t.id == string(message, "toolCallId"))
                                {
                                    tool.failed = message["isError"] == true;
                                    tool.accept_result(message);
                                    tool.duration_ms = message["durationMs"].as_u64();
                                }
                            }
                            // Provider history replaces arrays during recovery. Preserve
                            // stored identities before publishing the reconstructed rows.
                            let mut used = std::collections::HashSet::new();
                            let mut identities = self
                                .messages
                                .iter()
                                .map(|message| {
                                    let found =
                                        saved_messages.iter().enumerate().find(|(i, old)| {
                                            !used.contains(i)
                                                && if message.id.starts_with("approval-feedback:") {
                                                    old.id == message.id
                                                } else {
                                                    !old.id.starts_with("approval-feedback:")
                                                }
                                                && old.role == message.role
                                                && old.text == message.text
                                        });
                                    found.map(|(i, old)| {
                                        used.insert(i);
                                        old.id.clone()
                                    })
                                })
                                .collect::<Vec<_>>();
                            for (i, message) in self.messages.iter_mut().enumerate() {
                                if let Some(id) = identities[i].take() {
                                    message.id = id;
                                } else if let Some(old) = saved_messages.get(i).filter(|old| {
                                    old.role == message.role
                                        && !used.contains(&i)
                                        && !old.id.starts_with("approval-feedback:")
                                }) {
                                    used.insert(i);
                                    message.id = old.id.clone();
                                }
                            }
                            self.anchor_compactions(native_history_index);
                            for tool in &mut self.tools {
                                if let Some(order) = saved_orders.get(&tool.id) {
                                    tool.order = *order;
                                }
                            }
                            for thought in &mut self.thoughts {
                                if let Some((_, _, order)) =
                                    saved_thoughts.iter().find(|(anchor, block, _)| {
                                        *anchor == thought.anchor && *block == thought.block
                                    })
                                {
                                    thought.order = *order;
                                }
                            }
                            for work in &mut self.work {
                                if work.outcome.is_none() {
                                    work.outcome = saved_work
                                        .iter()
                                        .find(|saved| saved.start == work.start + message_offset)
                                        .and_then(|saved| saved.outcome);
                                }
                                if let Some(saved) = saved_work.iter().find(|saved| {
                                    saved.start == work.start + message_offset && saved.stopped
                                }) {
                                    work.stopped = true;
                                    work.stopped_order = saved.stopped_order;
                                    work.elapsed = saved.elapsed;
                                }
                            }
                            for saved in saved_work
                                .into_iter()
                                .filter(|w| w.stopped && w.start >= message_offset)
                            {
                                let local_start = saved.start - message_offset;
                                if !self.work.iter().any(|w| w.start == local_start) {
                                    let mut shifted = saved;
                                    shifted.start = local_start;
                                    self.work.push(shifted);
                                }
                            }
                            self.work.sort_by_key(|w| w.start);
                            if message_offset > 0 {
                                for tool in &mut self.tools {
                                    tool.anchor.message += message_offset;
                                }
                                for thought in &mut self.thoughts {
                                    thought.anchor.message += message_offset;
                                }
                                for work in &mut self.work {
                                    work.start += message_offset;
                                }
                                for compaction in &mut self.native.compactions {
                                    if let Some(m) = compaction["anchor"]["message"].as_u64() {
                                        compaction["anchor"]["message"] =
                                            json!(m + message_offset as u64);
                                    }
                                }
                                if let Some(assistant) = &mut self.assistant {
                                    *assistant += message_offset;
                                }
                                prefix_messages.append(&mut self.messages);
                                self.messages = prefix_messages;
                                prefix_tools.append(&mut self.tools);
                                self.tools = prefix_tools;
                                prefix_thoughts.append(&mut self.thoughts);
                                self.thoughts = prefix_thoughts;
                                prefix_work.append(&mut self.work);
                                self.work = prefix_work;
                                prefix_compactions.append(&mut self.native.compactions);
                                self.native.compactions = prefix_compactions;
                            }
                            // A process can accept a prompt before its response reaches us.
                            // Do not replay it if its unique delivery marker is already saved.
                            self.pending_injections.retain_mut(|injection| {
                                injection.shown = false;
                                !self
                                    .messages
                                    .iter()
                                    .any(|m| m.role == "Shina" && m.text == injection.text)
                            });
                            self.injection_id = None;
                        }
                        if let Some((prompt, index)) = &mut self.queued_prompt {
                            *index = self.messages.len();
                            let mut id = uuid::Uuid::new_v4().to_string();
                            if record["data"]["messages"]
                                .as_array()
                                .is_some_and(|messages| messages.is_empty())
                                && let Some(prev_id) =
                                    self.fresh_session_messages.last().filter(|prev_id| {
                                        self.messages.iter().all(|m| m.id != **prev_id)
                                    })
                            {
                                id = prev_id.clone();
                            }
                            let _ = &id;
                            self.messages.push(Message {
                                id,
                                role: "You",
                                text: prompt.text.clone(),
                                images: prompt.images.to_vec(),
                                context: prompt.context.clone(),
                            });
                        }
                        self.recover_native_inputs();
                        self.history_loaded = true;
                    }
                    "prompt" => {
                        if !injection
                            && self
                                .pending_prompt
                                .as_ref()
                                .is_some_and(|(id, _, _)| record["id"] == *id)
                        {
                            self.pending_prompt = None;
                        }
                        if record["data"]["disposition"] == "handled"
                            && self.status == Status::Running
                        {
                            // A command can launch work independently. Check Pi before settling it.
                            self.probe_command_idle();
                        }
                        // Capture the newly created session path before a long first run finishes.
                        self.command("get_state", json!({}));
                    }
                    _ => {}
                }
            }
            "agent_start" => {
                if self.run_id.is_none() {
                    self.run_id = Some(uuid::Uuid::new_v4().to_string());
                }
                self.prompt_suggestion = None;
                self.life.last_started_at = Some(lifecycle::now());
                self.status = Status::Running;
                self.assistant = None;
                self.phase = "Waiting for the model".into();
                self.started.get_or_insert_with(Instant::now);
                self.ensure_work().outcome = Some(Status::Running);
            }
            // agent_end is deliberately NOT settlement. Pi can still retry or compact.
            "agent_settled" => {
                if self
                    .operation_deadline
                    .is_some_and(|(kind, _)| kind == "abort")
                {
                    self.operation_deadline = None;
                }
                self.life.last_completed_at = Some(lifecycle::now());
                self.expire_live_question("The provider turn ended.");
                if self.native.questionnaire.is_none() {
                    self.request = None;
                }
                let aborted = record["aborted"] == true;
                if aborted {
                    self.mark_stopped();
                } else {
                    self.resumable_run_id = None;
                }
                self.finish_run();
                if self
                    .native
                    .questionnaire
                    .as_ref()
                    .is_some_and(|form| form.capability == "message")
                {
                    self.native.dialog_status = Some(self.status);
                }
                // A stopped run is idle, not a finished task.
                if aborted && self.status == Status::Completed {
                    self.status = Status::Ready;
                }
                if aborted || self.status != Status::Completed {
                    self.prompt_suggestion = None;
                }
                self.complete_task(if aborted {
                    "cancelled"
                } else {
                    status_word(self.status)
                });
                self.settle_memory(!aborted && self.status == Status::Completed);
                // The turn that just finished is what changed the live context window.
                self.context.cancel_request();
                self.refresh_context_usage();
            }
            "message_start" if record["message"]["role"] == "assistant" => {
                self.assistant = None;
                if self.native.questionnaire.is_none() {
                    self.request = None;
                }
                self.status = Status::Running;
                self.assistant();
            }
            "message_update" => {
                let update = &record["assistantMessageEvent"];
                if update["type"] == "text_delta" {
                    self.assistant().text.push_str(string(update, "delta"));
                    self.phase = "Writing".into();
                } else if matches!(
                    string(update, "type"),
                    "thinking_start" | "thinking_delta" | "thinking_end"
                ) {
                    let thought =
                        self.thought(update["contentIndex"].as_u64().unwrap_or(0) as usize);
                    thought.started.get_or_insert_with(Instant::now);
                    match string(update, "type") {
                        "thinking_delta" => thought.text.push_str(string(update, "delta")),
                        "thinking_end" => {
                            thought.text = string(update, "content").into();
                            thought.done = true;
                            thought.duration_ms = thought
                                .started
                                .map(|start| start.elapsed().as_millis() as u64);
                        }
                        _ => {}
                    }
                    self.phase = "Thinking".into();
                }
            }
            "message_end" if record["message"]["role"] == "assistant" => {
                let message = &record["message"];
                let text = content_text(&message["content"]);
                self.assistant();
                let index = self.assistant.unwrap();
                if !text.is_empty() {
                    self.assistant().text = text;
                    self.error = None;
                }
                self.anchor_content_work(&message["content"], index, false);
                if message["stopReason"] != "aborted"
                    && let Some(error) = message["errorMessage"].as_str().filter(|s| !s.is_empty())
                {
                    self.error = Some(error.to_owned());
                }
                self.assistant = None;
            }
            "tool_execution_start" => {
                self.phase = format!("Running {}", string(&record, "toolName"));
                let id = string(&record, "toolCallId");
                if let Some(tool) = self.tools.iter_mut().rev().find(|tool| tool.id == id) {
                    tool.started.get_or_insert_with(Instant::now);
                    if let Some(parent) = record["parentToolCallId"].as_str() {
                        tool.parent_call = Some(parent.into());
                    }
                } else {
                    self.assistant();
                    let anchor = ToolAnchor {
                        message: self.assistant.unwrap_or(self.messages.len()),
                        offset: self.assistant.map_or(0, |i| self.messages[i].text.len()),
                    };
                    let mut tool =
                        Tool::new(id, string(&record, "toolName"), &record["args"], anchor);
                    tool.accept_metadata(&record);
                    tool.order = self.next_work_order();
                    tool.started = Some(Instant::now());
                    tool.parent_call = record["parentToolCallId"].as_str().map(str::to_owned);
                    self.tools.push(tool);
                }
            }
            "auto_retry_start" => {
                self.record_retry(&record, false);
                self.phase = format!("Retrying, attempt {}", record["attempt"]);
                self.diagnostic(string(&record, "errorMessage"));
            }
            "auto_retry_end" => {
                self.record_retry(&record, true);
                if record["success"] == true {
                    self.error = None;
                    self.phase = "Working".into();
                } else {
                    self.error = Some(
                        record["finalError"]
                            .as_str()
                            .filter(|error| !error.is_empty())
                            .unwrap_or("Provider error")
                            .into(),
                    );
                }
            }
            "compaction_start" => {
                self.phase = "Compacting context".into();
                // The old percentage describes the context that is being replaced.
                self.context.begin_compaction();
            }
            "compaction_end" => {
                if let Some(error) = record["errorMessage"].as_str() {
                    self.error = Some(error.into());
                    if !self.native.queue.is_empty() {
                        self.native.queue_held = true;
                    }
                }
                self.context.finish_compaction();
                // Pi reports null usage until its next assistant response, so this request
                // legitimately leaves the ring unknown rather than reusing the old figure.
                self.refresh_context_usage();
                self.flush_injection();
            }
            "thinking_level_changed" => self.thinking = string(&record, "level").into(),
            "tool_execution_update" | "tool_execution_end" => {
                let ending = record["type"] == "tool_execution_end";
                if let Some(tool) = self
                    .tools
                    .iter_mut()
                    .rev()
                    .find(|tool| tool.id == string(&record, "toolCallId"))
                {
                    tool.done = ending;
                    tool.failed = record["isError"] == true;
                    if ending {
                        tool.duration_ms = record["durationMs"].as_u64().or_else(|| {
                            tool.started.map(|start| start.elapsed().as_millis() as u64)
                        });
                    }
                    tool.accept_result(if ending {
                        &record["result"]
                    } else {
                        &record["partialResult"]
                    });
                }
            }
            "extension_ui_request" => {
                if matches!(
                    string(&record, "method"),
                    "confirm" | "select" | "input" | "editor"
                ) {
                    self.reply.replace_text(
                        self.native
                            .questionnaire
                            .as_ref()
                            .map(|form| form.answers[form.index].other.as_str())
                            .unwrap_or_else(|| string(&record, "prefill")),
                    );
                    self.request = Some(record);
                    self.refresh_typing_guard();
                    if self.native.questionnaire.is_some() && !self.request_held_by_typing_guard() {
                        self.draft.focused = false;
                        self.reply.focused = false;
                    }
                    self.status = Status::Waiting;
                } else if record["method"] == "set_editor_text" {
                    self.draft.replace_text(string(&record, "text"));
                }
            }
            "prompt_suggestion" => {
                let event_run_id = record["runId"].as_str();
                if !self.status.busy()
                    && self.status == Status::Completed
                    && self.run_id.is_some()
                    && event_run_id == self.run_id.as_deref()
                    && crate::harness::supports_prompt_suggestions(&self.harness)
                {
                    self.prompt_suggestion = record["text"]
                        .as_str()
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(|text| PromptSuggestion {
                            run_id: event_run_id.unwrap().to_owned(),
                            text: text.to_owned(),
                        });
                }
            }
            _ => {}
        }
        self.drive_session();
    }

    fn next_work_order(&self) -> u64 {
        self.tools
            .iter()
            .map(|tool| tool.order)
            .chain(self.thoughts.iter().map(|thought| thought.order))
            .chain(self.work.iter().map(|work| work.stopped_order))
            .chain(
                self.native
                    .compactions
                    .iter()
                    .filter_map(|c| c["order"].as_u64()),
            )
            .chain(
                crate::work::retry_events(self)
                    .iter()
                    .filter_map(|e| e["order"].as_u64()),
            )
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    fn mark_stopped(&mut self) {
        self.resumable_run_id = self.run_id.clone();
        self.native.queue_held = true;
        let elapsed = self
            .started
            .map(|at| at.elapsed().as_secs())
            .or(self.elapsed);
        self.finish_retry(Some("Provider retry stopped"));
        self.assistant();
        let order = self.next_work_order();
        let work = self.ensure_work();
        work.stopped = true;
        if work.stopped_order == 0 {
            work.stopped_order = order;
        }
        work.elapsed = elapsed.or(work.elapsed);
    }

    fn record_retry(&mut self, record: &Value, ending: bool) {
        let turn = self.ensure_work().start;
        let previous = crate::work::retry_events(self)
            .iter()
            .position(|e| e["turn"].as_u64() == Some(turn as u64));
        if ending && previous.is_none() {
            return;
        }
        if previous.is_none() {
            self.assistant();
            let message = self.assistant.unwrap();
            let event = json!({"turn":turn,"anchor":ToolAnchor {message,offset:self.messages[message].text.len()},
                "order":self.next_work_order()});
            if !self.harness_state["workRetries"].is_array() {
                self.harness_state["workRetries"] = json!([]);
            }
            self.harness_state["workRetries"]
                .as_array_mut()
                .unwrap()
                .push(event);
        }
        let events = self.harness_state["workRetries"].as_array_mut().unwrap();
        let index = previous.unwrap_or(events.len() - 1);
        let event = &mut events[index];
        event["running"] = json!(!ending);
        if ending {
            event["success"] = record["success"].clone();
            event["error"] = record["finalError"].clone();
        } else {
            event["attempt"] = record["attempt"].clone();
            event["maxAttempts"] = record["maxAttempts"].clone();
            event["error"] = record["errorMessage"].clone();
        }
    }

    fn finish_retry(&mut self, error: Option<&str>) {
        let turn = self.work.last().map(|w| w.start);
        if crate::work::retry_events(self)
            .iter()
            .any(|e| e["turn"].as_u64() == turn.map(|start| start as u64) && e["running"] == true)
        {
            self.record_retry(&json!({"success":error.is_none(),"finalError":error}), true);
        }
    }

    fn record_plan(&mut self, source: &Value) {
        let turn = self.ensure_work().start;
        let id = source["_shinaPlanId"]
            .as_str()
            .or_else(|| source["planId"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("plan-{turn}"));
        let index = if let Some(index) = self.tools.iter().position(|t| t.id == id) {
            index
        } else {
            self.assistant();
            let message = self.assistant.unwrap();
            let mut tool = Tool::new(
                &id,
                "plan",
                source,
                ToolAnchor {
                    message,
                    offset: self.messages[message].text.len(),
                },
            );
            tool.kind = Some(crate::work::ToolKind::Plan);
            tool.order = self.next_work_order();
            self.tools.push(tool);
            self.tools.len() - 1
        };
        let tool = &mut self.tools[index];
        tool.done = true;
        tool.input = source.clone();
        tool.structured =
            json!({"input":source,"structuredContent":{"plan":crate::work::plan_data(source)}});
    }

    fn update_model(&mut self, model: &Value) {
        if let Some(id) = model["id"].as_str() {
            let provider = string(model, "provider").to_owned();
            let known = self
                .models
                .iter()
                .find(|m| m.id == id)
                .map(|m| (m.name.clone(), m.context_window, m.supports_images));
            if self.model_id != id || self.model_provider != provider {
                // Pi changed models: the old context percentage is not a measurement of the
                // new model's window, so it stays unknown until the next stats response.
                self.context.begin_model_change();
                self.context_window = None;
                self.supports_images = None;
            }
            self.model_id = id.into();
            self.model_provider = provider;
            self.model = model["name"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| known.as_ref().map(|(name, _, _)| name.clone()))
                .unwrap_or_else(|| id.into());
            let window = model["contextWindow"]
                .as_u64()
                .filter(|window| *window > 0)
                .or_else(|| known.as_ref().and_then(|(_, window, _)| *window));
            if window.is_some() {
                self.context_window = window;
            }
            if let Some(input) = model["input"].as_array() {
                self.supports_images = Some(input.iter().any(|kind| kind == "image"));
            } else if let Some((_, _, Some(images))) = known {
                self.supports_images = Some(images);
            }
        }
    }

    fn restore_prompt(&mut self, prompt: &Prompt) {
        let split_reply = prompt.draft.as_ref().and_then(|draft| {
            crate::composer_tokens::split_leading_reply_reference(
                &prompt.text,
                Some(&draft.editor()),
                prompt.context.as_ref(),
            )
        });
        let submitted = prompt.draft.as_ref().map_or_else(
            || crate::composer_rich::draft(&prompt.text),
            crate::composer_rich::Snapshot::editor,
        );
        self.draft = crate::composer_rich::prepend(&submitted, &self.draft);
        // Keep submitted images first. A newer draft may have added images while the process was
        // starting, so merge those after the snapshot without duplicating either set.
        let mut images = prompt
            .draft
            .as_ref()
            .map_or_else(|| prompt.images.to_vec(), |draft| draft.attachments.clone());
        for image in &self.draft_images {
            if !images.iter().any(|current| current.id == image.id) {
                images.push(image.clone());
            }
        }
        // Preserve both drafts even if their combined images exceed a message limit.
        // The user can remove images before sending. Never discard a newer attachment.
        self.draft_images = images;
        let mut context = prompt
            .draft
            .as_ref()
            .filter(|draft| !draft.context.is_empty())
            .map(|draft| draft.context.clone())
            .or_else(|| prompt.context.as_ref().map(|c| c.records.clone()))
            .unwrap_or_default();
        for record in &self.draft_context {
            if !context.iter().any(|c| c.context_id == record.context_id) {
                context.push(record.clone());
            }
        }
        self.draft_context = context;
        if self.draft_reply_to.is_none()
            && let Some((reply_record, _)) = split_reply
        {
            self.draft_reply_to = Some(reply_record);
        }
        self.attachment_error = attachments::validate_prompt_images(&self.draft_images).err();
    }

    pub(crate) fn restore_snapshot(&mut self, snapshot: &crate::composer_rich::Snapshot) {
        if let Some(edit) = &mut self.queued_edit {
            edit.ordinary = crate::composer_rich::prepend(&snapshot.editor(), &edit.ordinary);
            let mut images = snapshot.attachments.clone();
            for image in &edit.ordinary_images {
                if !images.iter().any(|existing| existing.id == image.id) {
                    images.push(image.clone());
                }
            }
            edit.ordinary_images = images;
            let mut context = snapshot.context.clone();
            for record in &edit.ordinary_context {
                if !context.iter().any(|c| c.context_id == record.context_id) {
                    context.push(record.clone());
                }
            }
            edit.ordinary_context = context;
        } else {
            let mut prompt = Prompt::new(String::new(), snapshot.attachments.clone());
            prompt.draft = Some(snapshot.clone());
            self.restore_prompt(&prompt);
        }
    }

    pub(crate) fn finish_remote_send(&mut self, command: uuid::Uuid, accepted: bool) -> bool {
        if !self
            .remote_send
            .as_ref()
            .is_some_and(|(id, _)| *id == command)
        {
            return false;
        }
        let (_, snapshot) = self.remote_send.take().unwrap();
        let sent_arm = self.remote_send_fresh_arm.take();
        let sent_reply_to = self.remote_send_reply_to.take();
        if accepted {
            if let Some(arm) = sent_arm
                && self.fresh_session_arm == Some(arm)
            {
                self.fresh_session_arm = None;
            }
        } else {
            self.restore_snapshot(&snapshot);
            if self.draft_reply_to.is_none() {
                self.draft_reply_to = sent_reply_to;
            }
        }
        true
    }

    fn finish_run(&mut self) {
        self.finish_retry(self.error.clone().as_deref());
        self.status = if self.error.is_some() {
            Status::Failed
        } else {
            Status::Completed
        };
        if self.status == Status::Failed {
            self.native.queue_held = true;
            self.life.failed_at = Some(lifecycle::now());
        }
        self.elapsed = self
            .started
            .take()
            .map(|start| start.elapsed().as_secs())
            .or(self.elapsed);
        self.ensure_work().elapsed = self.elapsed;
        let start = self.ensure_work().start;
        let outcome = if self.ensure_work().stopped {
            None
        } else {
            Some(self.status)
        };
        for work in &mut self.work {
            if work.start == start || work.outcome == Some(Status::Running) {
                work.outcome = outcome;
            }
        }
        self.finish_thoughts();
        self.assistant = None;
        self.phase.clear();
        if self.error.is_none() {
            self.retry_prompt = None;
        }
    }

    fn finish_thoughts(&mut self) {
        for thought in self.thoughts.iter_mut().filter(|thought| !thought.done) {
            thought.done = true;
            thought.duration_ms = thought
                .started
                .map(|start| start.elapsed().as_millis() as u64);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AssistantSelection {
    pub thread_id: usize,
    pub machine: String,
    pub message_index: usize,
    pub message_id: String,
    pub chunk_from: Option<usize>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CitationHighlight {
    pub machine: String,
    pub thread_id: usize,
    pub message_id: String,
    pub text: String,
}

#[derive(Clone, Debug)]
enum TabIntent {
    Open(Option<usize>),
    Park(usize, bool),
    Delete(std::collections::HashSet<usize>),
    Undo(Vec<(usize, usize)>, Option<usize>),
}

struct DeletedRecords {
    tasks: Vec<Task>,
    homes: Vec<(usize, usize)>,
    projects: Vec<(ProjectId, usize)>,
}

struct RequestedTabRestore {
    server: String,
    epoch: u64,
    tabs: Vec<(usize, usize)>,
    focus: Option<usize>,
}

/// Insert beside the active tab without moving an already-open tab.
fn insert_tab(tabs: &mut Vec<usize>, active: usize, id: usize) {
    if !tabs.contains(&id) {
        let position = tabs
            .iter()
            .position(|tab| *tab == active)
            .map_or(tabs.len(), |i| i + 1);
        tabs.insert(position, id);
    }
}

/// Return the right neighbour, or the left neighbour at the end of the strip.
fn tab_neighbour(tabs: &[usize], id: usize) -> Option<usize> {
    let position = tabs.iter().position(|tab| *tab == id)?;
    tabs.get(position + 1)
        .or_else(|| position.checked_sub(1).and_then(|i| tabs.get(i)))
        .copied()
}

pub struct State {
    /// None means a command/global change. A poll records only changed threads.
    pub(crate) persistence_scope: Option<std::collections::HashSet<usize>>,
    pub client: Option<crate::client::Client>,
    pub machines: crate::peer::Machines,
    pub peers: Vec<crate::peer::Peer>,
    peer_runtime: remote_tasks::PeerRuntime,
    pub(crate) headless: bool,
    command_receipts: Vec<crate::protocol::Receipt>,
    command_intents: std::collections::BTreeMap<String, crate::protocol::Command>,
    server_epoch: String,
    server_seq: u64,
    persist_transcripts: bool,
    client_views: Value,
    next_client_draft: usize,
    tab_epoch: u64,
    tab_intents: std::collections::HashMap<(String, uuid::Uuid), (u64, TabIntent)>,
    requested_tab: Option<(String, usize, u64, Option<usize>)>,
    requested_tab_restore: Option<RequestedTabRestore>,
    pub dismissed_prompt_suggestions: std::collections::HashSet<(String, usize, String)>,
    undo_tabs: std::collections::HashMap<(String, usize), (usize, bool)>,
    deleted_threads: Vec<Thread>,
    deleted_records: Option<DeletedRecords>,
    client_recover: std::collections::HashMap<usize, uuid::Uuid>,
    recovered_inputs: std::collections::BTreeMap<usize, Value>,
    pub background: background::Background,
    pub composer_sources: crate::composer_sources::Sources,
    pub data_dir: PathBuf,
    /// The open thread's id and whether it was a draft with content when it was opened. Like
    /// T3 Code's frozen draft row, the open draft's sidebar row neither pops in while typing nor
    /// disappears when its composer is cleared.
    opened_draft: Option<(usize, bool)>,
    launch_project: Option<PathBuf>,
    pub threads: Vec<Thread>,
    pub active: usize,
    pub tabs: Vec<usize>,
    pub next_id: usize,
    pub dark: bool,
    /// Whole-window zoom, changed with Ctrl+= / Ctrl+- and reset with Ctrl+0.
    pub zoom: f32,
    pub show_activity: bool,
    /// The open thread's "More actions" menu in the chat header.
    pub thread_menu: bool,
    pub row_menu: Option<RowTarget>,
    /// The footer's settings menu is open.
    pub settings_menu: bool,
    pub settings: crate::ui::settings_ui::Dialog,
    pub preferences: crate::ui::settings_ui::Preferences,
    pub keybindings_watcher: crate::keybindings::KeybindingsWatcher,
    pub thread_defaults: crate::ui::settings_ui::ThreadDefaults,
    pub fleet: crate::fleet_update::Fleet,
    /// The sidebar's "N need you" popover is open.
    pub attention_menu: bool,
    /// Where a right-click opened the row menu, in window points.
    pub row_menu_at: Option<(f32, f32)>,
    pub hovered_row: Option<RowTarget>,
    pub rename_thread: Option<usize>,
    pub rename_edit: TextEdit,
    pub delete_thread: Option<usize>,
    pub delete_roots: Vec<usize>,
    pub confirm_remove_attachment: Option<(usize, String)>,
    answer_remove_scope: Option<String>,
    pub(crate) pending_context_imports: Vec<(String, usize, String)>,
    context_import_rxs: Vec<std::sync::mpsc::Receiver<ContextAssetDownload>>,
    pub project_scope: ProjectScope,
    pub scope_additional: Vec<ProjectId>,
    pub selected_threads: Vec<usize>,
    pub selection_anchor: Option<usize>,
    /// Whether the Snoozed and Settled shelves are open.
    pub shelves: [bool; 2],
    pub settled_visible: usize,
    pub undo: Option<UndoNotice>,
    pub toasts: Vec<ToastNotice>,
    next_toast_id: u64,
    pub composer_notices: crate::composer_notices::Store,
    fresh_arm_seq: u64,
    keep_full_history_threads: std::collections::HashSet<(String, usize)>,
    pub plan_menu: Option<String>,
    pub plan_copied: Option<(String, u64)>,
    pub plan_expanded: std::collections::HashSet<String>,
    pub plan_save_dialog: Option<PlanSaveDialog>,
    pub plan_save_last_path: Option<String>,
    next_plan_save_request_id: u64,
    pub snooze_menu: Option<(usize, Vec<SnoozeChoice>)>,
    pub snooze_targets: Vec<usize>,
    pub custom_snooze_open: bool,
    pub snooze_edit: TextEdit,
    pub lifecycle_error: Option<String>,
    pub auto_settle_days: u32,
    pub default_harness: String,
    pub drag_thread: Option<usize>,
    pub drag_target: Option<(Section, Option<usize>)>,
    pub drop_rects: std::collections::HashMap<String, [f32; 4]>,
    pub viewport_height: f32,
    pub viewport_width: f32,
    pub sidebar_width: f32,
    pub ui_store: Option<crate::store::Store>,
    last_sweep: u64,
    last_lifecycle_clock: u64,
    pub project_picker: Option<ProjectPicker>,
    /// The workspace picker's search text and the row the arrow keys have reached.
    pub picker_query: TextEdit,
    pub picker_highlight: usize,
    /// The Add workspace palette is browsing folders; the query holds the path.
    pub picker_browsing: bool,
    pub activity_full: bool,
    pub show_archive: bool,
    pub sidebar: bool,
    /// Whether the sidebar is shown while the window is too narrow for it to stay docked.
    pub compact_sidebar: bool,
    pub search: TextEdit,
    pub demo: bool,
    pub wake: Waker,
    pub store: Option<crate::store::Store>,
    pub storage_error: Option<String>,
    pub artwork: crate::runtime::UiResource<Arc<crate::visuals::Artwork>>,
    /// Sidebar portrait animation: frame stages and when they may play.
    pub portraits: crate::runtime::UiResource<crate::portraits::Portraits>,
    /// Decodes composer and conversation thumbnails off the UI thread.
    pub attachment_cache: crate::runtime::UiResource<rune::ImageCache>,
    pub html_windows: Arc<crate::html_window::Windows>,
    pub html_lane_width: u32,
    /// The composer is showing a valid drop highlight.
    pub attachment_drop_hover: bool,
    pub(crate) transcript_preview: Option<crate::ui::transcript_context::Preview>,
    pub(crate) transcript_layout: std::collections::HashMap<String, (bool, f32)>,
    pub(crate) assistant_selection: Option<AssistantSelection>,
    pub(crate) citation_highlight: Option<CitationHighlight>,
    pub composer_ui: crate::ui::composer_ui::Behavior,
    pub captain_picker: bool,
    pub captain_picker_query: TextEdit,
    pub captain_picker_highlight: usize,
    pub env_mode_picker: bool,
    pub base_branch_picker: bool,
    pub base_branch_query: TextEdit,
    pub(crate) vcs_branches: std::collections::BTreeMap<(String, String), VcsBranchCacheEntry>,
    vcs_branch_rxs: Vec<std::sync::mpsc::Receiver<VcsBranchResultMsg>>,
    worktree_bootstrap_rxs: Vec<std::sync::mpsc::Receiver<WorktreeBootstrapResult>>,
    worktree_setup_rxs: Vec<std::sync::mpsc::Receiver<WorktreeSetupResultMsg>>,
    multi_model_batches: Vec<MultiModelBatch>,
    #[cfg(test)]
    pub(crate) test_git_context: Option<crate::vcs::GitContext>,
    #[cfg(test)]
    pub(crate) test_connect_workspaces: Vec<(usize, PathBuf)>,
    #[cfg(test)]
    pub(crate) test_connect_pi:
        Option<Box<dyn FnMut(usize, &std::path::Path) -> Option<Pi> + Send>>,
    next_vcs_request_id: u64,
    pending_vcs_requests: std::collections::HashMap<u64, (String, String)>,
    pub captain_project: Option<Option<ProjectId>>,
    pub captains: Vec<Captain>,
    captain_scopes: std::collections::BTreeMap<usize, Vec<ProjectId>>,
    captain_remote_scopes: std::collections::BTreeMap<usize, Vec<RemoteProject>>,
    captain_homes: std::collections::BTreeMap<usize, usize>,
    memory: Arc<std::sync::Mutex<memory::MemoryBook>>,
    pub captain_editor: Option<CaptainEditor>,
    pub brief: BriefUi,
    pub settings_picker: Option<SettingsPicker>,
    pub open_option_menu: Option<String>,
    pub model_picker_tab: Option<crate::model_picker::ModelPickerTab>,
    pub expanded_legacy_instances: std::collections::HashSet<String>,
    pub model_favorites: Option<Vec<crate::model_picker::ModelFavorite>>,
    model_favorites_migrated: bool,
    /// Where the effort slider's thumb is held while dragged, from 0 (lowest level) to 1
    /// (highest). The level is chosen on release.
    pub thinking_drag: Option<f32>,
    pub model_search: TextEdit,
    pub thread_find: ThreadFindState,
    pub sidebar_search: SidebarSearchState,
    pub pending_open_urls: Vec<String>,
    pub next_usage_limits_request_id: u64,
    pub pending_usage_limits_request: Option<(u64, String, usize, String)>,
    pub usage_limits_notice_key: Option<(String, usize, String)>,
    pub timeline_far_from_bottom: std::collections::HashMap<(String, usize), bool>,
    pub timeline_at_bottom: std::collections::HashMap<(String, usize), bool>,
    pub timeline_positions: std::cell::RefCell<crate::timeline_position::Positions>,
    pub updater: crate::update::Updater,
    /// Set by "Restart to update". The restart only happens on a quit that saves cleanly.
    pub restart_for_update: bool,
    /// The explicit restart has saved and recorded its update for the pending quit.
    pub restart_approved: bool,
    /// Why the last restart click waited. Cleared while the update row shows ready.
    pub restart_notice: Option<String>,
    task_waiters: Vec<TaskWaiter>,
    tasks: Vec<Task>,
    pub recovery_notice: Option<String>,
    run_marker: Option<crate::store::RunMarker>,
    /// Recent projects, the switcher menu, and the clone sheet.
    pub projects: crate::projects::Projects,
    pub catalog: Catalog,
    pub codex_catalog: crate::session::NativeCatalog,
    pub claude_catalog: crate::session::NativeCatalog,
    pub harnesses: crate::harness::Registry,
    pub managed_policy: Option<ManagedPolicy>,
    pub(crate) runtime_enabled: bool,
    ownership_denied: bool,
    pub runtime_stop_requested: bool,
    // Drop the owner only after every store has flushed and stopped its writer.
    owner: Option<crate::server_lock::Owner>,
}

impl State {
    /// Headless snapshots read a supplied isolated profile without starting Pi or saving.
    pub fn load_snapshot(&mut self) {
        let result = (|| {
            if let Some(index) = crate::store::load_global(&self.data_dir, None)? {
                if index["version"] != 3 && index["version"] != 4 {
                    return Err("Unsupported index version.".into());
                }
                self.restore_global(&index)?;
            }
            self.load_memory(false)?;
            if let Some(ui) = crate::store::load(&self.data_dir.join("ui-state.json"))? {
                self.restore_ui(&ui)?;
            }
            if self.keybindings_path().exists() {
                let file_state =
                    crate::keybindings::load_runtime_keybindings_file(&self.keybindings_path());
                self.preferences.keybindings = file_state.rules;
                self.settings.keybindings_issues = file_state.issues;
                self.keybindings_watcher
                    .record_stamp(&self.keybindings_path());
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.storage_error = Some(error);
        }
    }
    pub fn new(workspace: PathBuf, demo: bool) -> Self {
        let state = Self::construct(workspace, demo);
        // Preserve desktop startup timing. Headless construction never touches these.
        let _ = &*state.artwork;
        let _ = &*state.portraits;
        let _ = &*state.attachment_cache;
        state
    }
    fn construct(workspace: PathBuf, demo: bool) -> Self {
        let workspace = if workspace.as_os_str().is_empty() {
            workspace
        } else {
            crate::projects::normalize_path(&workspace)
        };
        let art_path = workspace.clone();
        let portrait_path = workspace.clone();
        let art = Arc::new(std::sync::OnceLock::new());
        let portrait_art = art.clone();
        let artwork = crate::runtime::UiResource::new(move || {
            art.get_or_init(|| crate::visuals::Artwork::load(&art_path))
                .clone()
        });
        let attachment_cache =
            crate::runtime::UiResource::new(|| rune::ImageCache::new(Arc::new(|| {})));
        let mut state = Self {
            client: None,
            machines: Default::default(),
            peers: Vec::new(),
            peer_runtime: Default::default(),
            headless: false,
            persistence_scope: None,
            owner: None,
            command_receipts: Vec::new(),
            command_intents: Default::default(),
            server_epoch: uuid::Uuid::new_v4().to_string(),
            server_seq: 0,
            persist_transcripts: false,
            dismissed_prompt_suggestions: Default::default(),
            client_views: json!({}),
            next_client_draft: usize::MAX,
            tab_epoch: 0,
            tab_intents: Default::default(),
            requested_tab: None,
            requested_tab_restore: None,
            undo_tabs: Default::default(),
            deleted_threads: Vec::new(),
            deleted_records: None,
            client_recover: Default::default(),
            recovered_inputs: Default::default(),
            background: Default::default(),
            data_dir: crate::store::directory()
                .unwrap_or_else(|| std::env::temp_dir().join("Shina")),
            html_windows: Default::default(),
            html_lane_width: crate::html_render::COLUMN_WIDTH,
            launch_project: (!workspace.as_os_str().is_empty()).then_some(workspace.clone()),
            threads: vec![Thread::new(1)],
            active: 0,
            tabs: vec![1],
            next_id: 2,
            dark: false,
            zoom: 1.0,
            show_activity: false,
            thread_menu: false,
            row_menu: None,
            settings_menu: false,
            settings: Default::default(),
            preferences: Default::default(),
            keybindings_watcher: Default::default(),
            thread_defaults: Default::default(),
            fleet: Default::default(),
            attention_menu: false,
            row_menu_at: None,
            hovered_row: None,
            rename_thread: None,
            rename_edit: TextEdit::default(),
            delete_thread: None,
            delete_roots: vec![],
            confirm_remove_attachment: None,
            answer_remove_scope: None,
            pending_context_imports: Vec::new(),
            context_import_rxs: Vec::new(),
            project_scope: ProjectScope::All,
            scope_additional: vec![],
            selected_threads: vec![],
            selection_anchor: None,
            shelves: [false; 2],
            settled_visible: 10,
            undo: None,
            toasts: Vec::new(),
            next_toast_id: 1,
            composer_notices: crate::composer_notices::Store::default(),
            fresh_arm_seq: 0,
            keep_full_history_threads: Default::default(),
            plan_menu: None,
            plan_copied: None,
            plan_expanded: Default::default(),
            plan_save_dialog: None,
            plan_save_last_path: None,
            next_plan_save_request_id: 0,
            opened_draft: None,
            composer_sources: crate::composer_sources::Sources::default(),
            assistant_selection: None,
            citation_highlight: None,
            snooze_menu: None,
            snooze_targets: vec![],
            custom_snooze_open: false,
            snooze_edit: TextEdit::default(),
            lifecycle_error: None,
            auto_settle_days: 3,
            default_harness: "pi".into(),
            drag_thread: None,
            drag_target: None,
            drop_rects: Default::default(),
            viewport_height: 720.0,
            viewport_width: 1280.0,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            ui_store: None,
            last_sweep: 0,
            last_lifecycle_clock: 0,
            project_picker: None,
            picker_query: TextEdit::default(),
            picker_highlight: 0,
            picker_browsing: false,
            activity_full: false,
            show_archive: false,
            sidebar: true,
            compact_sidebar: false,
            search: TextEdit::default(),
            demo,
            wake: Arc::new(|| {}),
            store: None,
            storage_error: None,
            portraits: crate::runtime::UiResource::new(move || {
                crate::portraits::Portraits::new(
                    portrait_art
                        .get_or_init(|| crate::visuals::Artwork::load(&portrait_path))
                        .clone(),
                )
            }),
            artwork,
            attachment_cache,
            attachment_drop_hover: false,
            transcript_preview: None,
            transcript_layout: Default::default(),
            composer_ui: Default::default(),
            captain_picker: false,
            captain_picker_query: TextEdit::default(),
            captain_picker_highlight: 0,
            env_mode_picker: false,
            base_branch_picker: false,
            base_branch_query: TextEdit::default(),
            vcs_branches: Default::default(),
            vcs_branch_rxs: Vec::new(),
            worktree_bootstrap_rxs: Vec::new(),
            worktree_setup_rxs: Vec::new(),
            multi_model_batches: Vec::new(),
            #[cfg(test)]
            test_git_context: None,
            #[cfg(test)]
            test_connect_workspaces: Vec::new(),
            #[cfg(test)]
            test_connect_pi: None,
            next_vcs_request_id: 0,
            pending_vcs_requests: Default::default(),
            captain_project: None,
            captains: Vec::new(),
            captain_scopes: Default::default(),
            captain_remote_scopes: Default::default(),
            captain_homes: Default::default(),
            memory: Arc::default(),
            captain_editor: None,
            brief: BriefUi::default(),
            settings_picker: None,
            open_option_menu: None,
            model_picker_tab: None,
            expanded_legacy_instances: Default::default(),
            model_favorites: None,
            model_favorites_migrated: false,
            thinking_drag: None,
            model_search: TextEdit::default(),
            thread_find: ThreadFindState::default(),
            sidebar_search: SidebarSearchState::default(),
            pending_open_urls: Vec::new(),
            next_usage_limits_request_id: 0,
            pending_usage_limits_request: None,
            usage_limits_notice_key: None,
            timeline_far_from_bottom: std::collections::HashMap::new(),
            timeline_at_bottom: std::collections::HashMap::new(),
            timeline_positions: std::cell::RefCell::default(),
            updater: Default::default(),
            restart_for_update: false,
            restart_approved: false,
            restart_notice: None,
            task_waiters: vec![],
            projects: Default::default(),
            catalog: Catalog::default(),
            codex_catalog: Default::default(),
            claude_catalog: Default::default(),
            harnesses: crate::harness::Registry::read(),
            managed_policy: if cfg!(test) {
                None
            } else {
                read_managed_policy()
            },
            runtime_enabled: false,
            ownership_denied: false,
            runtime_stop_requested: false,
            tasks: Vec::new(),
            recovery_notice: None,
            run_marker: None,
        };
        if !workspace.as_os_str().is_empty() {
            state.projects.entries.push(Project::new(1, workspace));
            state.projects.next_id = 2;
            state.threads[0].project = Some(1);
        } else {
            state.projects.next_id = 1;
        }
        state.threads[0].native.access = state.effective_access_mode(&state.threads[0]);
        if demo {
            state.populate_demo();
        }
        state
    }
    pub fn current(&self) -> &Thread {
        &self.threads[self.active]
    }
    pub fn current_mut(&mut self) -> &mut Thread {
        &mut self.threads[self.active]
    }
    pub fn expire_undo(&mut self) -> bool {
        let at = lifecycle::now();
        let Some(notice) = &self.undo else {
            return false;
        };
        if notice.expires <= at {
            self.undo = None;
            self.deleted_threads.clear();
            self.deleted_records = None;
            return false;
        }
        true
    }
    pub fn undo_key(&mut self, event: &rune::KeyEvent, text_field: bool) -> bool {
        let at = lifecycle::now();
        let Some(notice) = &mut self.undo else {
            return false;
        };
        if notice.expires <= at {
            self.undo = None;
            self.deleted_threads.clear();
            self.deleted_records = None;
            return false;
        }
        if event.is_command('z') && !event.mods.shift && (!text_field || notice.owns_text_key) {
            self.undo_thread_action();
            return true;
        }
        if !matches!(event.key, rune::Key::Modifier { .. }) {
            notice.owns_text_key = false;
        }
        false
    }
    pub fn select_thread(&mut self, index: usize) {
        if index >= self.threads.len() {
            return;
        }
        let active = self.threads.get(self.active).map_or(0, |t| t.id);
        insert_tab(&mut self.tabs, active, self.threads[index].id);
        self.tab_epoch += 1;
        self.requested_tab = None;
        self.requested_tab_restore = None;
        self.focus_thread(index);
    }
    /// Focus an existing tab. Background reducers must not open tabs.
    fn focus_thread(&mut self, index: usize) {
        self.focus_tab(index, true);
    }
    pub(crate) fn focus_tab(&mut self, index: usize, visit: bool) {
        debug_assert!(self.tabs.contains(&self.threads[index].id));
        let previous = self.active;
        if previous != index && previous < self.threads.len() {
            self.threads[previous].draft.focused = false;
            self.threads[previous].cancel_questionnaire_advance();
        }
        self.active = index;
        self.plan_menu = None;
        self.env_mode_picker = false;
        self.base_branch_picker = false;
        self.assistant_selection = None;
        self.citation_highlight = None;
        if visit && !self.demo {
            self.forward("thread.visit", json!({"threadId":self.current().id}));
        }
        self.ensure_vcs_branches(false);
        self.selected_threads.clear();
        let id = self.current().id;
        if self.thread_find.open
            && (self.thread_find.server != self.machines.selected
                || self.thread_find.thread_id != id)
        {
            self.close_thread_find();
        }
        if self.opened_draft.map(|(opened, _)| opened) != Some(id) {
            self.capture_opened_draft();
        }
        let current_key = (
            self.machines.selected.clone(),
            id,
            self.current().harness.clone(),
        );
        if self.usage_limits_notice_key.as_ref() != Some(&current_key) {
            self.usage_limits_notice_key = None;
            self.composer_notices.remove("usage-limits");
        }
        self.current_mut().life.last_visited_at = Some(lifecycle::now());
        if !self.headless {
            self.composer_thread_opened();
        }
        if !self.in_scope(self.current()) {
            self.project_scope = ProjectScope::All;
            self.scope_additional.clear();
        }
        self.sync_fresh_session_notice();
        self.sync_fallback_model(lifecycle::now() as i64);
        self.checkpoint();
    }
    /// Closing a blank draft also deletes it. Written drafts survive without a tab.
    pub fn close_tab(&mut self, id: usize) {
        if !self.tabs.contains(&id) {
            return;
        }
        let Some(index) = self.threads.iter().position(|t| t.id == id) else {
            return;
        };
        if self.threads[index].blank_draft() {
            if self.threads[index].client_draft {
                self.remove_threads(&std::collections::HashSet::from([id]));
                return;
            }
            if self.forward("thread.delete", json!({"threadId":id,"blankOnly":true})) {
                return;
            }
            self.remove_threads(&std::collections::HashSet::from([id]));
            return;
        }
        if self.tabs.len() <= 1 {
            return;
        }
        self.remove_tab(id, false);
        self.checkpoint();
    }
    pub fn project(&self, id: Option<ProjectId>) -> Option<&Project> {
        self.projects.entries.iter().find(|p| Some(p.id) == id)
    }
    pub fn set_timeline_far_from_bottom(&mut self, thread_id: usize, far: bool) {
        let key = (self.machines.selected.clone(), thread_id);
        if far {
            self.timeline_far_from_bottom.insert(key, true);
        } else {
            self.timeline_far_from_bottom.remove(&key);
        }
    }
    pub fn timeline_is_far_from_bottom(&self, thread_id: usize) -> bool {
        self.timeline_far_from_bottom
            .get(&(self.machines.selected.clone(), thread_id))
            .copied()
            .unwrap_or(false)
    }
    pub fn set_timeline_at_bottom(&mut self, thread_id: usize, at: bool) {
        self.timeline_at_bottom
            .insert((self.machines.selected.clone(), thread_id), at);
    }
    pub fn thread_find_active(&self) -> bool {
        self.thread_find.open
            && self.thread_find.server == self.machines.selected
            && self.thread_find.thread_id == self.current().id
    }
    pub fn open_thread_find(&mut self) {
        let server = self.machines.selected.clone();
        let thread_id = self.current().id;
        let same = self.thread_find.open
            && self.thread_find.server == server
            && self.thread_find.thread_id == thread_id;
        if !same {
            self.thread_find = ThreadFindState {
                open: true,
                server,
                thread_id,
                ..ThreadFindState::default()
            };
        }
        self.thread_find.query.focused = true;
        self.thread_find.query.anchor = 0;
        self.thread_find.query.focus = self.thread_find.query.text.len();
        if !self.thread_find.query.text.trim().is_empty() {
            self.run_thread_find(None, 0);
        }
    }
    pub fn open_thread_find_on_query(
        &mut self,
        query: String,
        start: Option<crate::thread_find::ThreadFindStart>,
    ) {
        let server = self.machines.selected.clone();
        let thread_id = self.current().id;
        self.thread_find = ThreadFindState {
            open: true,
            server,
            thread_id,
            ..ThreadFindState::default()
        };
        let query: String = query
            .chars()
            .take(crate::thread_find::THREAD_FIND_QUERY_MAX_LENGTH)
            .collect();
        self.thread_find.query.replace_text(&query);
        self.thread_find.query.focused = true;
        self.thread_find.query.anchor = 0;
        self.thread_find.query.focus = self.thread_find.query.text.len();
        self.thread_find.last_typed = self.thread_find.query.text.clone();
        if !query.trim().is_empty() {
            self.run_thread_find(start.as_ref(), 0);
        }
    }
    pub fn close_thread_find(&mut self) {
        self.thread_find = ThreadFindState::default();
    }
    #[cfg(test)]
    pub fn set_thread_find_query(&mut self, query: String) {
        let query: String = query
            .chars()
            .take(crate::thread_find::THREAD_FIND_QUERY_MAX_LENGTH)
            .collect();
        self.thread_find.query.replace_text(&query);
        self.thread_find.last_typed = query;
        self.thread_find.debounce_until = None;
        self.run_thread_find(None, 0);
    }
    pub fn retry_thread_find(&mut self) {
        if !self.thread_find.open {
            return;
        }
        self.run_thread_find(None, 0);
    }
    pub fn step_thread_find(&mut self, delta: i64) {
        if !self.thread_find.open
            || self.thread_find.total_matches == 0
            || self.thread_find.loading
            || self.thread_find.unsupported
        {
            return;
        }
        let total = self.thread_find.total_matches;
        let next_index =
            ((self.thread_find.active_index as i64 + delta).rem_euclid(total as i64)) as usize;
        if let Some(entry) = self.thread_find.navigation.iter().find(|entry| {
            next_index >= entry.start_index && next_index < entry.start_index + entry.count
        }) {
            let matched = crate::thread_find::ThreadFindMatch {
                entry_id: entry.entry_id.clone(),
                occurrence: next_index - entry.start_index,
            };
            self.thread_find.active_index = next_index;
            self.thread_find.active_match = Some(matched.clone());
            self.thread_find.pending_reveal = Some(matched);
            return;
        }
        let start =
            self.thread_find
                .active_match
                .as_ref()
                .map(|m| crate::thread_find::ThreadFindStart {
                    entry_id: m.entry_id.clone(),
                    occurrence: m.occurrence,
                });
        self.run_thread_find(start.as_ref(), delta);
    }
    pub fn sync_thread_find_input(&mut self) -> bool {
        if !self.thread_find.open {
            return false;
        }
        let clipped: String = self
            .thread_find
            .query
            .text
            .chars()
            .take(crate::thread_find::THREAD_FIND_QUERY_MAX_LENGTH)
            .collect();
        if clipped != self.thread_find.query.text {
            self.thread_find.query.replace_text(&clipped);
        }
        if self.thread_find.query.text == self.thread_find.last_typed {
            return false;
        }
        self.thread_find.last_typed = self.thread_find.query.text.clone();
        if self.client.is_some() {
            self.thread_find.debounce_until = Some(Instant::now() + Duration::from_millis(100));
        } else {
            self.thread_find.debounce_until = None;
            self.run_thread_find(None, 0);
        }
        true
    }
    pub fn poll_thread_find(&mut self, tx: &mut Tx) -> bool {
        let mut changed = self.sync_thread_find_input();
        if let Some(until) = self.thread_find.debounce_until {
            let now = Instant::now();
            if now >= until {
                self.thread_find.debounce_until = None;
                self.run_thread_find(None, 0);
                changed = true;
            } else {
                tx.wake_after(until.saturating_duration_since(now));
            }
        }
        changed |= self.consume_pending_find_reveal(tx);
        changed
    }
    fn run_thread_find(
        &mut self,
        start: Option<&crate::thread_find::ThreadFindStart>,
        offset: i64,
    ) {
        if !self.thread_find.open {
            return;
        }
        let query = self.thread_find.query.text.trim().to_owned();
        if query.is_empty() {
            self.thread_find.loading = false;
            self.thread_find.error = None;
            self.thread_find.unsupported = false;
            self.thread_find.total_matches = 0;
            self.thread_find.active_index = 0;
            self.thread_find.active_match = None;
            self.thread_find.navigation.clear();
            self.thread_find.pending_reveal = None;
            return;
        }
        self.thread_find.request_id += 1;
        let req_id = self.thread_find.request_id;
        let thread_id = self.thread_find.thread_id;
        if let Some(client) = &self.client {
            let target = if self.thread_find.server == "local" {
                Some(client)
            } else {
                client.linked.get(&self.thread_find.server)
            };
            let Some(target) = target else {
                self.thread_find.loading = false;
                self.thread_find.error = Some("Machine unavailable.".into());
                return;
            };
            if !target.has_capability(crate::protocol::THREAD_FIND) {
                self.thread_find.loading = false;
                self.thread_find.unsupported = true;
                self.thread_find.error = Some(crate::protocol::THREAD_FIND_UPDATE_REQUIRED.into());
                return;
            }
            self.thread_find.unsupported = false;
            self.thread_find.error = None;
            self.thread_find.loading = true;
            let mut params = json!({"threadId": thread_id, "query": query});
            if let Some(start) = start {
                params["start"] = json!(start);
            }
            if offset != 0 {
                params["offset"] = json!(offset);
            }
            target.call_rpc(format!("find:{req_id}"), "thread.find", params);
            return;
        }
        let t = self.current();
        let messages: Vec<_> = t.messages.iter().map(|message|
            json!({"id":message.id,"role":message.role,"text":message.display_text()})).collect();
        let streams = crate::history::Streams::new(&json!({"seq":0,
            "state":{"threads":[{"id":t.id,"messages":messages,"tools":t.tools}]}}));
        let result = streams
            .find_thread(
                t.id as u64,
                &json!({"query":query,"start":start,"offset":offset}),
            )
            .and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()));
        self.apply_thread_find_result(req_id, result);
    }
    pub fn apply_thread_find_result(
        &mut self,
        req_id: u64,
        result: Result<crate::thread_find::ThreadFindResult, String>,
    ) {
        if !self.thread_find.open || req_id != self.thread_find.request_id {
            return;
        }
        self.thread_find.loading = false;
        match result {
            Ok(res) => {
                self.thread_find.error = None;
                self.thread_find.unsupported = false;
                self.thread_find.total_matches = res.total_matches;
                self.thread_find.active_index = res.active_index;
                self.thread_find.navigation = res.navigation;
                self.thread_find.active_match = res.active_match.clone();
                self.thread_find.pending_reveal = res.active_match;
            }
            Err(err) => {
                self.thread_find.error = Some(err);
            }
        }
    }
    pub fn consume_pending_find_reveal(&mut self, tx: &mut Tx) -> bool {
        if !self.thread_find_active() {
            return false;
        }
        let Some(matched) = self.thread_find.pending_reveal.clone() else {
            return false;
        };
        let t = self.current_mut();
        let plan_index = t.tools.iter().position(|tool| {
            tool.id == matched.entry_id && tool.resolved_kind() == crate::work::ToolKind::Plan
        });
        let msg_idx = t
            .messages
            .iter()
            .position(|m| m.id == matched.entry_id)
            .or_else(|| plan_index.map(|index| t.tools[index].anchor.message));
        let Some(msg_idx) = msg_idx else {
            if t.history_has_more && !t.history_loading && t.history_error.is_none() {
                let tid = t.id;
                self.load_earlier_history(tid, Some(matched.entry_id));
                return true;
            }
            return false;
        };
        let tid = t.id;
        let mut deferred_start = None;
        let row_key = if plan_index.is_some() || t.messages[msg_idx].role == "Pi" {
            let start = t.messages[..=msg_idx]
                .iter()
                .rposition(|m| m.role != "Pi")
                .map_or(0, |i| i + 1);
            if let Some(work) = t.work.iter_mut().find(|w| w.start == start) {
                work.expanded = true;
            } else if plan_index.is_some() {
                t.work.push(AssistantWork {
                    start,
                    stopped: false,
                    stopped_order: 0,
                    expanded: true,
                    elapsed: None,
                    outcome: None,
                });
            }
            if let Some(index) = plan_index {
                t.tools[index].expanded = true;
            }
            if t.deferred_work
                .iter()
                .any(|marker| marker["start"] == start)
            {
                deferred_start = Some(start);
            }
            format!("turn-{tid}-{}", t.messages[start].id)
        } else {
            format!("row-{tid}-{}", t.messages[msg_idx].id)
        };
        if let Some(start) = deferred_start {
            self.load_deferred_work(start, false);
            // KSG holds find's reveal until the requested fold's history is mounted.
            return false;
        }
        self.thread_find.pending_reveal = None;
        tx.scroll_center_in_instant(
            format!("conversation-{tid}"),
            format!("{row_key}/thread-find-active"),
        );
        true
    }

    pub(crate) fn cite_assistant_selection(&mut self, tx: &mut Tx) -> bool {
        let Some(sel) = self.assistant_selection.clone() else {
            return false;
        };
        if self.demo {
            return false;
        }
        let current = self.current();
        if current.id != sel.thread_id
            || current.machine != sel.machine
            || current.archived()
            || current.request.is_some()
            || current.native.questionnaire.is_some()
        {
            return false;
        }
        let Some(message) = current.messages.get(sel.message_index) else {
            return false;
        };
        if message.id != sel.message_id || message.role != "Pi" {
            return false;
        }
        let env_id = self.machine_identity(&current.machine);
        let thread_id_str = current.id.to_string();
        let Ok(citation) = crate::assistant_citation::build_assistant_citation_from_selection(
            &env_id,
            &thread_id_str,
            &message.id,
            &message.text,
            sel.chunk_from,
            &sel.text,
        ) else {
            return false;
        };
        let record =
            crate::assistant_citation::citation_context_record(citation, &message.id, None);
        self.current_mut().insert_context_chip(record);
        self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Inserted);
        self.assistant_selection = None;
        tx.clear_text_selection();
        self.checkpoint();
        true
    }

    pub(crate) fn navigate_to_citation(
        &mut self,
        citation: &crate::composer_context::AssistantCitation,
        tx: &mut Tx,
    ) -> bool {
        let Ok(id) = citation.thread_id.parse::<usize>() else {
            return false;
        };
        let machine = if citation.environment_id == self.machine_identity("local") {
            "local".to_owned()
        } else {
            citation.environment_id.clone()
        };
        self.open_machine_thread(&machine, id);
        if self.current().id != id || self.machines.selected != machine {
            return false;
        }
        let t = self.current();
        let msg_idx = t.messages.iter().position(|m| m.id == citation.message_id);
        let row_key = match msg_idx {
            Some(idx) if t.messages[idx].role == "Pi" => {
                let start = t.messages[..=idx]
                    .iter()
                    .rposition(|m| m.role != "Pi")
                    .map_or(0, |i| i + 1);
                format!("turn-{id}-{}", t.messages[start].id)
            }
            _ => format!("row-{id}-{}", citation.message_id),
        };
        self.citation_highlight = Some(CitationHighlight {
            machine,
            thread_id: id,
            message_id: citation.message_id.clone(),
            text: citation.text.clone(),
        });
        tx.scroll_center_in(format!("conversation-{id}"), row_key);
        true
    }

    pub(crate) fn reply_to_message(&mut self, message_id: &str) -> bool {
        if self.demo {
            return false;
        }
        let current = self.current();
        if current.archived() || current.request.is_some() || current.native.questionnaire.is_some()
        {
            return false;
        }
        let Some(msg_idx) = current
            .messages
            .iter()
            .position(|m| m.id == message_id && m.role == "Pi" && !m.text.trim().is_empty())
        else {
            return false;
        };
        let message = &current.messages[msg_idx];
        let Some(opening) =
            crate::assistant_citation::capture_assistant_message_opening(&message.text)
        else {
            return false;
        };
        let env_id = self.machine_identity(&current.machine);
        let thread_id_str = current.id.to_string();
        let citation = crate::composer_context::AssistantCitation {
            version: 1,
            environment_id: env_id,
            thread_id: thread_id_str,
            message_id: message.id.clone(),
            text: opening.text,
            comment: None,
            start: opening.start,
            end: opening.end,
            prefix: opening.prefix,
            suffix: opening.suffix,
        };
        let subagents = if self.is_captain_home(current) {
            let turn_start = current.messages[..=msg_idx]
                .iter()
                .rposition(|m| m.role != "Pi")
                .map_or(0, |i| i + 1);
            let tasks = self.tasks_for(current.id);
            let mut refs = Vec::new();
            let mut seen_threads = std::collections::HashSet::new();
            for tool in &current.tools {
                if !(turn_start..=msg_idx).contains(&tool.anchor.message)
                    || (tool.name != "delegate_task" && !tool.name.ends_with("__delegate_task"))
                {
                    continue;
                }
                let child_id = serde_json::from_str::<Value>(&tool.output)
                    .ok()
                    .and_then(|v| v["thread_id"].as_u64())
                    .or_else(|| tool.structured["thread_id"].as_u64());
                if let Some(child_id) = child_id
                    && let Some(task) = tasks.iter().find(|task| task.child_id == child_id as usize)
                    && seen_threads.insert(task.child_id)
                {
                    refs.push(crate::composer_context::ResponseSubagentRef {
                        thread_id: task.child_id.to_string(),
                        title: task.title.clone(),
                        environment_id: task.remote.as_ref().map(|r| r.server_id.clone()),
                    });
                }
            }
            (!refs.is_empty()).then_some(refs)
        } else {
            None
        };
        let record =
            crate::assistant_citation::response_context_record(citation, &message.id, subagents);
        self.current_mut().draft_reply_to = Some(record);
        self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Explicit);
        self.checkpoint();
        true
    }

    pub(crate) fn clear_reply_to(&mut self) {
        if self.current().draft_reply_to.is_some() {
            self.current_mut().draft_reply_to = None;
            self.checkpoint();
        }
    }
    pub fn is_captain_home(&self, thread: &Thread) -> bool {
        thread.captain_home && !thread.delegated
    }
    pub fn thread_directory(&self, index: usize) -> Result<PathBuf, String> {
        let thread = self.threads.get(index).ok_or("Thread does not exist.")?;
        if self.client.is_some() && self.machines.selected != "local" {
            return self.client_views["directories"][thread.id.to_string()]
                .as_str()
                .map(PathBuf::from)
                .ok_or("The remote server has not reported this thread's host directory.".into());
        }
        crate::backend::BackendHost::resolve(&thread.machine)?;
        if let Some(worktree_path) = &thread.worktree_path {
            return Ok(PathBuf::from(worktree_path));
        }
        match thread.project {
            Some(id) => {
                let project = self
                    .project(Some(id))
                    .ok_or("Thread project does not exist.")?;
                if project.machine != thread.machine {
                    return Err("Thread and project machines do not match.".into());
                }
                Ok(project.path.clone())
            }
            None if thread.captain_home => Ok(self
                .data_dir
                .join("captains")
                .join(
                    thread
                        .captain
                        .ok_or("Captain home has no profile.")?
                        .to_string(),
                )
                .join("scratch")),
            None => Ok(self.data_dir.join("chats").join(thread.id.to_string())),
        }
    }
    pub fn working_directory(&self) -> PathBuf {
        self.thread_directory(self.active)
            .unwrap_or_else(|_| self.data_dir.clone())
    }
    pub fn sorted_projects(&self) -> Vec<ProjectId> {
        let mut projects: Vec<_> = self
            .projects
            .entries
            .iter()
            .filter(|p| !p.hidden || self.show_archive)
            .collect();
        projects.sort_by_key(|p| {
            std::cmp::Reverse((
                self.threads
                    .iter()
                    .filter(|t| t.project == Some(p.id))
                    .map(|t| t.updated)
                    .max()
                    .unwrap_or(p.updated)
                    .max(p.updated),
                p.id,
            ))
        });
        projects.into_iter().map(|p| p.id).collect()
    }
    pub fn sidebar_threads(&self) -> Vec<usize> {
        let mut threads: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.archived() == self.show_archive
                    && (!t.delegated
                        || t.parent.is_some_and(|id| {
                            self.threads
                                .iter()
                                .any(|parent| parent.id == id && !parent.captain_home)
                        }))
                    && !self.is_captain_home(t)
                    && (!t.is_draft() || self.draft_listed(t))
            })
            .filter(|(_, t)| self.in_scope(t))
            .collect();
        let at = lifecycle::now();
        threads.sort_by(|(_, a), (_, b)| {
            let sa = self.thread_section(a, at);
            let sb = self.thread_section(b, at);
            self.draft_listed(b)
                .cmp(&self.draft_listed(a))
                .then_with(|| sa.cmp(&sb))
                .then_with(|| lifecycle::compare(sa, &a.life, &b.life))
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.machine.cmp(&b.machine))
        });
        threads.into_iter().map(|(i, _)| i).collect()
    }
    /// Coordinators share the same workspace and search scope as standalone threads.
    /// In the archive, keep a home visible when it owns archived worker chats.
    pub fn sidebar_captains(&self) -> Vec<usize> {
        self.threads
            .iter()
            .enumerate()
            .filter(|(_, home)| {
                self.is_captain_home(home)
                    && home
                        .captain
                        .is_some_and(|captain| self.captain_home(captain) == Some(home.id))
                    && (self.in_scope(home)
                        || home.captain.is_some_and(|captain| {
                            self.captain_scope(captain)
                                .iter()
                                .any(|id| self.scope_includes(Some(*id)))
                        }))
            })
            .filter(|(_, home)| {
                let children = || {
                    self.threads.iter().filter(|child| {
                        child
                            .parent
                            .is_some_and(|parent| self.effective_home(parent) == home.id)
                            && child.archived() == self.show_archive
                    })
                };
                let own_match =
                    home.archived() == self.show_archive && self.thread_search_matches(home);
                if self.show_archive {
                    own_match
                        || children().any(|child| {
                            self.thread_search_matches(home) || self.thread_search_matches(child)
                        })
                } else {
                    !home.archived()
                        && (own_match || children().any(|child| self.thread_search_matches(child)))
                }
            })
            .map(|(index, _)| index)
            .collect()
    }
    pub fn sync_sidebar_search_input(&mut self) -> bool {
        let clipped: String = self
            .search
            .text
            .trim()
            .chars()
            .take(crate::thread_search::THREAD_SEARCH_QUERY_MAX_LENGTH)
            .collect();
        let trimmed = clipped.trim().to_owned();
        if trimmed == self.sidebar_search.last_typed {
            return false;
        }
        self.sidebar_search.last_typed = trimmed.clone();
        let tokens = crate::thread_search::message_search_tokens(&trimmed);
        if tokens.long.is_empty() {
            self.sidebar_search.debounce_until = None;
            self.sidebar_search.pending_servers.clear();
            self.sidebar_search.unsupported_servers.clear();
            self.sidebar_search.matches.clear();
            self.sidebar_search.settled_query.clear();
            self.sidebar_search.visible_query.clear();
            self.sidebar_search.visible_matches.clear();
            self.sidebar_search.shown_order.clear();
            return true;
        }
        if self.client.is_some() {
            self.sidebar_search.debounce_until = Some(
                Instant::now()
                    + Duration::from_millis(crate::thread_search::THREAD_SEARCH_DEBOUNCE_MS),
            );
        } else {
            self.sidebar_search.debounce_until = None;
            self.run_sidebar_search(&trimmed);
        }
        true
    }
    pub fn poll_sidebar_search(&mut self, tx: &mut Tx) -> bool {
        let mut changed = self.sync_sidebar_search_input();
        if let Some(until) = self.sidebar_search.debounce_until {
            let now = Instant::now();
            if now >= until {
                self.sidebar_search.debounce_until = None;
                let q = self.sidebar_search.last_typed.clone();
                self.run_sidebar_search(&q);
                changed = true;
            } else {
                tx.wake_after(until.saturating_duration_since(now));
            }
        }
        changed
    }
    fn run_sidebar_search(&mut self, query: &str) {
        let tokens = crate::thread_search::message_search_tokens(query);
        if tokens.long.is_empty() {
            return;
        }
        self.sidebar_search.request_seq += 1;
        let seq = self.sidebar_search.request_seq;
        self.sidebar_search.settled_query = query.to_owned();
        self.sidebar_search.matches.clear();
        self.sidebar_search.pending_servers.clear();
        self.sidebar_search.unsupported_servers.clear();
        let Some(client) = &self.client else {
            // Keep visible_query/visible_matches in non-server mode if manually seeded in tests;
            // local message matching runs in thread_title_search_matches.
            return;
        };
        let mut targets: Vec<(String, &crate::client::Client)> = vec![("local".into(), client)];
        for (id, linked) in &client.linked {
            targets.push((id.clone(), linked));
        }
        for (server, target) in targets {
            if !target.connected_ready() {
                continue;
            }
            if !target.has_capability(crate::protocol::THREAD_SEARCH) {
                self.sidebar_search
                    .unsupported_servers
                    .push(self.machine_label(&server));
                continue;
            }
            self.sidebar_search.pending_servers.insert(server.clone());
            target.call_rpc(
                format!("search:{seq}:{server}"),
                "threads.search",
                json!({"query": query, "limit": 50}),
            );
        }
        // Clear unrelated cached matches if query neither refines nor widens prior query.
        let cur = crate::thread_search::fold_for_title(query);
        let prev = crate::thread_search::fold_for_title(&self.sidebar_search.visible_query);
        if !(!cur.is_empty()
            && !prev.is_empty()
            && (cur.starts_with(&prev) || prev.starts_with(&cur)))
        {
            self.sidebar_search.visible_query.clear();
            self.sidebar_search.visible_matches.clear();
            self.sidebar_search.shown_order.clear();
        }
    }
    pub fn apply_sidebar_search_result(
        &mut self,
        seq: u64,
        server: &str,
        result: Result<Vec<crate::thread_search::ThreadSearchHit>, String>,
    ) {
        if seq != self.sidebar_search.request_seq {
            return;
        }
        self.sidebar_search.pending_servers.remove(server);
        if let Ok(hits) = result {
            self.sidebar_search.matches.insert(server.to_owned(), hits);
            // Rank across all responding environments and preserve shown order (KSG nextShownOrder).
            let mut combined: Vec<(String, crate::thread_search::ThreadSearchHit)> = self
                .sidebar_search
                .matches
                .iter()
                .flat_map(|(srv, list)| list.iter().map(move |h| (srv.clone(), h.clone())))
                .collect();
            let weight = |hits: usize| 1.0 + (1.0 + hits as f64).log2();
            combined.sort_by(|(_, left), (_, right)| {
                let lw = left.quality >= crate::thread_search::MESSAGE_PREFIX;
                let rw = right.quality >= crate::thread_search::MESSAGE_PREFIX;
                rw.cmp(&lw)
                    .then_with(|| {
                        right
                            .quality
                            .partial_cmp(&left.quality)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| {
                        weight(right.hits)
                            .partial_cmp(&weight(left.hits))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| right.updated.cmp(&left.updated))
            });
            let ranked_keys: Vec<(String, usize)> = combined
                .iter()
                .map(|(srv, hit)| (srv.clone(), hit.thread_id))
                .collect();
            let ranked_set: std::collections::HashSet<(String, usize)> =
                ranked_keys.iter().cloned().collect();
            let same_query = self.sidebar_search.visible_query == self.sidebar_search.settled_query;
            let mut next_order: Vec<(String, usize)> = if same_query {
                self.sidebar_search
                    .shown_order
                    .iter()
                    .filter(|k| ranked_set.contains(*k))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            let held: std::collections::HashSet<(String, usize)> =
                next_order.iter().cloned().collect();
            for key in ranked_keys {
                if !held.contains(&key) {
                    next_order.push(key);
                }
            }
            self.sidebar_search.shown_order = next_order;
            self.sidebar_search.visible_query = self.sidebar_search.settled_query.clone();
            self.sidebar_search.visible_matches = combined
                .into_iter()
                .map(|(srv, hit)| ((srv, hit.thread_id), hit))
                .collect();
        }
    }
    pub fn open_thread_at_search_match(&mut self, server: &str, id: usize) {
        let hit = self
            .sidebar_search
            .visible_matches
            .get(&(server.to_owned(), id))
            .cloned();
        let query = if self.sidebar_search.visible_query.is_empty() {
            self.search.text.trim().to_owned()
        } else {
            self.sidebar_search.visible_query.clone()
        };
        if self.client.is_some() {
            self.open_machine_thread(server, id);
        } else if let Some(index) = self.threads.iter().position(|t| t.id == id) {
            self.select_thread(index);
        }
        if let Some(hit) = hit
            && self.current().id == id
        {
            let (find_query, start) =
                crate::thread_search::find_for_message_match(&hit.snippet, &hit.entry_id, &query);
            self.open_thread_find_on_query(find_query, start);
        }
    }
    pub fn thread_title_search_matches(&self, thread: &Thread) -> bool {
        let search = self.search.text.to_lowercase();
        if search.is_empty() {
            return true;
        }
        let title_or_draft = thread.title.to_lowercase().contains(&search)
            || self.draft_listed(thread) && thread.draft.text.to_lowercase().contains(&search)
            || self.is_captain_home(thread)
                && thread.captain.is_some_and(|captain| {
                    self.captains[captain].name.to_lowercase().contains(&search)
                });
        if title_or_draft {
            return true;
        }
        let server_supports = self.client.as_ref().is_some_and(|c| {
            let target = if self.machines.selected == "local" {
                Some(c)
            } else {
                c.linked.get(&self.machines.selected)
            };
            target.is_some_and(|t| t.has_capability(crate::protocol::THREAD_SEARCH))
        });
        if !server_supports {
            return thread
                .messages
                .iter()
                .any(|message| message.text.to_lowercase().contains(&search));
        }
        false
    }
    pub fn thread_search_matches(&self, thread: &Thread) -> bool {
        if self.search.text.is_empty() {
            return true;
        }
        self.thread_title_search_matches(thread)
            || self
                .sidebar_search
                .visible_matches
                .contains_key(&(self.machines.selected.clone(), thread.id))
    }
    /// Whether a captain shows its whole crew: while it or one of its workers is open, while
    /// searching, and in the archive.
    pub fn captain_expanded(&self, home: usize) -> bool {
        home == self.selected_row() || !self.search.text.is_empty() || self.show_archive
    }
    /// Workers stay inside their captain. Alerts never expand a collapsed crew.
    pub fn visible_captain_workers(&self, home: usize) -> Vec<usize> {
        if !self.captain_expanded(home) {
            return vec![];
        }
        let Some(parent) = self.threads.iter().find(|thread| thread.id == home) else {
            return vec![];
        };
        self.threads
            .iter()
            .enumerate()
            .filter(|(_, child)| {
                child
                    .parent
                    .is_some_and(|parent| self.effective_home(parent) == home)
                    && child.archived() == self.show_archive
                    && (self.thread_search_matches(parent) || self.thread_search_matches(child))
            })
            .map(|(index, _)| index)
            .collect()
    }
    /// Live or archived workers, matching the current view, whether shown or not.
    pub fn captain_worker_count(&self, home: usize) -> usize {
        self.threads
            .iter()
            .filter(|child| {
                child
                    .parent
                    .is_some_and(|parent| self.effective_home(parent) == home)
                    && child.archived() == self.show_archive
            })
            .count()
    }
    /// Keyboard traversal follows the rows that the sidebar actually displays.
    pub fn sidebar_navigation(&self) -> Vec<usize> {
        let mut rows = vec![];
        for index in self.sidebar_captains() {
            rows.push(index);
            rows.extend(self.visible_captain_workers(self.threads[index].id));
        }
        rows.extend(self.visible_sidebar_threads());
        rows
    }
    /// Whether an unsent draft has a "Drafts" row. Other drafts need content. The open draft keeps
    /// whatever it had when it was opened, so its row does not appear or vanish while typing.
    pub fn draft_listed(&self, t: &Thread) -> bool {
        if !t.is_draft() {
            return false;
        }
        match self.opened_draft {
            Some((id, listed)) if id == t.id && id == self.current().id => listed,
            _ => t.has_draft_content() || t.renamed,
        }
    }
    /// Records whether the open thread is a listed draft as it is opened or restored.
    fn capture_opened_draft(&mut self) {
        let t = self.current();
        let listed = t.is_draft() && (t.has_draft_content() || t.renamed);
        self.opened_draft = Some((t.id, listed));
    }
    pub fn in_scope(&self, t: &Thread) -> bool {
        self.scope_includes(t.project)
    }
    /// Whether the workspace filter shows this workspace (None is No workspace).
    pub fn scope_includes(&self, project: Option<ProjectId>) -> bool {
        self.scope_includes_on(&self.machines.selected, project)
    }
    pub fn scope_includes_on(&self, server: &str, project: Option<ProjectId>) -> bool {
        let owner = self
            .machines
            .scope_server
            .as_deref()
            .unwrap_or(&self.machines.selected);
        let same_server = self.client.is_none() || server == owner;
        let additional = project.is_some_and(|p| {
            same_server && self.scope_additional.contains(&p)
                || self
                    .machines
                    .scope_remote
                    .iter()
                    .any(|(owner, id)| owner == server && *id == p)
        });
        match self.project_scope {
            ProjectScope::All => true,
            ProjectScope::Project(id) => same_server && project == Some(id) || additional,
            ProjectScope::NoProject => project.is_none() || additional,
        }
    }
    pub fn set_project_scope(&mut self, project: Option<ProjectId>, multi: bool) {
        self.set_machine_scope(self.machines.selected.clone(), project, multi);
    }
    pub fn set_machine_scope(&mut self, server: String, project: Option<ProjectId>, multi: bool) {
        let owner = self
            .machines
            .scope_server
            .clone()
            .unwrap_or_else(|| self.machines.selected.clone());
        // None means the global "No workspace" choice; every project has an
        // explicit server. Keep the legacy primary/additional fields readable.
        let mut choices: Vec<Option<(String, usize)>> = if multi {
            match self.project_scope {
                ProjectScope::All => vec![],
                ProjectScope::NoProject => std::iter::once(None)
                    .chain(
                        self.scope_additional
                            .iter()
                            .map(|id| Some((owner.clone(), *id))),
                    )
                    .chain(self.machines.scope_remote.iter().cloned().map(Some))
                    .collect(),
                ProjectScope::Project(id) => std::iter::once(Some((owner.clone(), id)))
                    .chain(
                        self.scope_additional
                            .iter()
                            .map(|id| Some((owner.clone(), *id))),
                    )
                    .chain(self.machines.scope_remote.iter().cloned().map(Some))
                    .collect(),
            }
        } else {
            vec![]
        };
        let target = project.map(|id| (server, id));
        if let Some(i) = choices.iter().position(|choice| choice == &target) {
            choices.remove(i);
        } else {
            choices.push(target);
        }
        choices.sort();
        choices.dedup();
        self.project_scope = match choices.first() {
            None => ProjectScope::All,
            Some(None) => ProjectScope::NoProject,
            Some(Some((_, id))) => ProjectScope::Project(*id),
        };
        let owner = choices
            .iter()
            .flatten()
            .next()
            .map(|(server, _)| server.clone());
        self.machines.scope_server = owner.clone();
        self.scope_additional.clear();
        self.machines.scope_remote.clear();
        for (server, id) in choices.into_iter().skip(1).flatten() {
            if Some(&server) == owner.as_ref() {
                self.scope_additional.push(id);
            } else {
                self.machines.scope_remote.push((server, id));
            }
        }
        self.selected_threads.clear();
        if !multi {
            self.project_picker = None;
        }
        self.checkpoint();
    }
    /// Open the workspace picker with an empty search.
    pub fn open_picker(&mut self, mode: ProjectPicker) {
        if mode == ProjectPicker::AddProject {
            if self.project_picker != Some(ProjectPicker::AddProject) {
                self.projects.return_picker = self.project_picker;
            }
            self.projects.browser = Default::default();
        } else {
            self.machines.draft_picker = None;
        }
        if self.captain_editor.is_none() {
            self.cancel_captain_flow();
        }
        self.picker_browsing = false;
        self.project_picker = Some(mode);
        self.picker_query = TextEdit::default();
        self.picker_query.focused = true;
        self.picker_highlight = 0;
    }
    /// Initial query for the Add workspace folder browser, ported from KSG Code's
    /// `getAddProjectInitialQueryForEnvironment`: uses `addProjectBaseDirectory` from the
    /// target machine's thread defaults (ensuring a trailing separator), or `"~/"` when empty.
    pub fn add_project_initial_query(&self) -> String {
        let base = if self.client.is_none() || self.machines.selected == "local" {
            self.thread_defaults.add_project_base_directory.trim()
        } else {
            self.machines
                .views
                .get(&self.machines.selected)
                .and_then(|v| v["index"]["threadDefaults"]["add_project_base_directory"].as_str())
                .map(str::trim)
                .unwrap_or("")
        };
        if base.is_empty() {
            return "~/".into();
        }
        let mut out = base.to_string();
        if !out.ends_with(['/', '\\']) {
            out.push(if out.contains('\\') && !out.contains('/') {
                '\\'
            } else {
                '/'
            });
        }
        out
    }
    /// Switch the Add workspace palette to browsing the selected server user's home or
    /// configured "Add project starts in" directory.
    /// Ported from KSG Code's `startAddProjectBrowse`.
    pub fn browse_for_project(&mut self) {
        self.project_picker = Some(ProjectPicker::AddProject);
        self.picker_browsing = true;
        self.projects.browser = Default::default();
        self.picker_query = TextEdit::default();
        // The answering host resolves home/base. Do not derive remote parents on this machine.
        let initial = self.add_project_initial_query();
        self.picker_query.replace_text(&initial);
        self.picker_query.focused = true;
        self.picker_highlight = 0;
    }
    /// Add the typed folder, creating it first when it does not exist yet.
    pub fn add_browsed_project(&mut self) {
        let text = if self.picker_query.text.is_empty() {
            self.projects
                .browser
                .listing
                .as_ref()
                .map(|listing| listing.path.clone())
                .unwrap_or_default()
        } else {
            self.picker_query.text.clone()
        };
        if text.is_empty() {
            return;
        }
        if self.client.is_some() {
            let (purpose, captain) = self.project_selection_purpose();
            let current_listing = self.projects.browser.request.as_ref()
                == Some(&(
                    self.machines.selected.clone(),
                    self.picker_query.text.clone(),
                ))
                && self.projects.browser.pending.is_none();
            let creates = self
                .projects
                .browser
                .listing
                .as_ref()
                .is_some_and(|listing| !listing.target_exists)
                || self.projects.browser.error.as_deref() == Some("That folder does not exist.");
            let creates = current_listing && creates;
            match self.remote("project.put", json!({"path":text,"createFolder":creates,"selectFor":purpose,"captain":captain})).expect("client") {
                Ok(_) => {
                    self.project_picker = None;
                    self.picker_browsing = false;
                    self.projects.menu_error = None;
                }
                Err(error) => self.projects.menu_error = Some(error),
            }
            return;
        }
        let path = crate::projects::expand_path(&text);
        if !path.exists()
            && let Err(error) = std::fs::create_dir_all(&path)
        {
            self.projects.menu_error = Some(format!("Could not create that folder: {error}"));
            return;
        }
        match self.add_project(path) {
            Ok(_) => {
                self.project_picker = None;
                self.picker_browsing = false;
                self.projects.menu_error = None;
                self.checkpoint();
            }
            Err(error) => self.projects.menu_error = Some(error),
        }
    }
    /// The sidebar's New thread, ported from KSG Code's `shouldCreateNewThreadInCurrentProject`
    /// and `startNewThreadFromContext`: with one workspace in view, no workspaces at all, or
    /// Shift-click in a visible workspace, start at once; otherwise open the New thread picker.
    pub fn new_thread_click(&mut self, shift: bool) {
        let shown = |s: &Self, project: Option<ProjectId>| {
            s.scope_includes(project)
                && project.is_none_or(|id| s.project(Some(id)).is_some_and(|p| !p.hidden))
        };
        let in_view: Vec<_> = self
            .projects
            .entries
            .iter()
            .filter(|p| shown(self, Some(p.id)))
            .map(|p| p.id)
            .collect();
        let all_visible_empty = self.projects.entries.iter().all(|p| p.hidden);
        let current = self.current().project;
        let target = if shift && current.is_some() && shown(self, current) {
            Some(current)
        } else if in_view.len() == 1 {
            Some(Some(in_view[0]))
        } else if all_visible_empty && in_view.is_empty() {
            Some(None)
        } else {
            None
        };
        match target {
            Some(project) => {
                self.new_thread_in(project);
                self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Explicit);
            }
            None => self.open_picker(ProjectPicker::NewThread),
        }
    }
    pub fn click_thread(&mut self, index: usize, mods: rune::Mods) {
        let id = self.threads[index].id;
        if mods.cmd {
            if let Some(i) = self.selected_threads.iter().position(|t| *t == id) {
                self.selected_threads.remove(i);
            } else {
                self.selected_threads.push(id);
            }
        } else if mods.shift {
            let order = self.visible_sidebar_threads();
            let start = order
                .iter()
                .position(|i| Some(self.threads[*i].id) == self.selection_anchor);
            let end = order.iter().position(|i| self.threads[*i].id == id);
            if let (Some(a), Some(b)) = (start, end) {
                self.selected_threads = order[a.min(b)..=a.max(b)]
                    .iter()
                    .map(|i| self.threads[*i].id)
                    .collect();
            }
        } else {
            self.select_thread(index);
        }
        if !mods.shift {
            self.selection_anchor = Some(id);
        }
    }
    pub fn thread_section(&self, t: &Thread, at: u64) -> Section {
        if t.machine != "local" {
            return if t.pinned() {
                Section::Pinned
            } else {
                Section::Active
            };
        }
        lifecycle::section(&t.life, at)
    }
    pub fn row_pending(&self, t: &Thread) -> bool {
        t.request.is_some()
            || t.status == Status::Waiting
            || self.threads.iter().any(|c| {
                c.parent == Some(t.id)
                    && c.request.as_ref().is_some_and(|r| r["method"] == "confirm")
            })
    }
    pub fn selected_row(&self) -> usize {
        self.effective_home(self.current().parent.unwrap_or(self.current().id))
    }
    pub fn visible_sidebar_threads(&self) -> Vec<usize> {
        let mut settled = 0;
        self.sidebar_threads()
            .into_iter()
            .filter(|i| {
                let t = &self.threads[*i];
                if !self.search.text.is_empty() {
                    return self.thread_search_matches(t);
                }
                let section = self.thread_section(t, lifecycle::now());
                let shelf = match section {
                    Section::Snoozed => Some(0),
                    Section::Settled => Some(1),
                    _ => None,
                };
                if let Some(shelf) = shelf {
                    if !self.shelves[shelf] {
                        return t.id == self.selected_row();
                    }
                    if section == Section::Settled {
                        settled += 1;
                        return settled <= self.settled_visible || t.id == self.selected_row();
                    }
                }
                true
            })
            .collect()
    }
    pub fn drag_pointer(&mut self, id: usize, drag: &rune::Drag) {
        if drag.cancelled() {
            self.drag_thread = None;
            self.drag_target = None;
            return;
        }
        if drag.dx.hypot(drag.dy) >= 6.0 && drag.phase != rune::Phase::Up {
            self.drag_thread = Some(id);
        }
        if self.drag_thread == Some(id) {
            let [x, y] = drag.window_point();
            self.drag_target = None;
            for i in self.visible_sidebar_threads() {
                let t = &self.threads[i];
                if t.id == id {
                    continue;
                }
                if let Some([rx, ry, rw, rh]) =
                    self.drop_rects.get(&format!("thread-{}", t.id)).copied()
                    && x >= rx
                    && x <= rx + rw
                    && y >= ry
                    && y <= ry + rh
                {
                    let section = self.thread_section(t, lifecycle::now());
                    let before = if y < ry + rh / 2.0 {
                        Some(t.id)
                    } else {
                        self.sidebar_threads()
                            .into_iter()
                            .skip_while(|j| *j != i)
                            .skip(1)
                            .find(|j| {
                                self.thread_section(&self.threads[*j], lifecycle::now()) == section
                            })
                            .map(|j| self.threads[j].id)
                    };
                    self.drag_target = Some((section, before));
                    break;
                }
            }
            for section in [Section::Pinned, Section::Active, Section::Settled] {
                if let Some([rx, ry, rw, rh]) =
                    self.drop_rects.get(&format!("drop-{section:?}")).copied()
                    && x >= rx
                    && x <= rx + rw
                    && y >= ry
                    && y <= ry + rh
                {
                    self.drag_target = Some((section, None));
                }
            }
        }
        if drag.phase == rune::Phase::Up {
            if let Some((section, before)) = self.drag_target.take() {
                self.drop_thread(id, section, before);
            }
            self.drag_thread = None;
        }
    }
    pub fn remove_project(&mut self, id: ProjectId) {
        if self.forward("project.remove", json!({"projectId":id})) {
            return;
        }
        if let Some(project) = self.projects.entries.iter_mut().find(|p| p.id == id) {
            project.hidden = true;
        }
        self.checkpoint();
    }
    pub fn begin_rename(&mut self, id: usize) {
        if let Some(thread) = self.threads.iter().find(|t| t.id == id) {
            self.rename_edit.replace_text(&thread.title);
            self.rename_edit.focused = true;
            self.rename_thread = Some(id);
            self.row_menu = None;
            self.thread_menu = false;
        }
    }
    pub fn save_thread_name(&mut self) {
        if self.client.is_some() {
            if let Some(id) = self.rename_thread {
                self.forward(
                    "thread.rename",
                    json!({"threadId":id,"title":self.rename_edit.text}),
                );
                self.rename_thread = None;
            }
            return;
        }
        let name = self.rename_edit.text.trim();
        if name.is_empty() || name.chars().any(char::is_control) {
            return;
        }
        if let Some(thread) = self
            .threads
            .iter_mut()
            .find(|t| Some(t.id) == self.rename_thread)
        {
            thread.title = name.chars().take(200).collect();
            thread.renamed = true;
        }
        self.rename_thread = None;
        self.checkpoint();
    }
    pub fn request_delete(&mut self, id: usize) {
        self.delete_roots = if self.selected_threads.contains(&id) {
            self.selected_threads.clone()
        } else {
            vec![id]
        };
        self.delete_thread = Some(id);
        self.row_menu = None;
        self.thread_menu = false;
        if !self.preferences.confirm_delete {
            self.confirm_delete();
        }
    }
    pub fn deletion_ids(&self, id: usize) -> std::collections::HashSet<usize> {
        let mut ids = std::collections::HashSet::from([id]);
        loop {
            let before = ids.len();
            for thread in &self.threads {
                if thread.parent.is_some_and(|parent| ids.contains(&parent)) {
                    ids.insert(thread.id);
                }
            }
            if ids.len() == before {
                return ids;
            }
        }
    }
    /// Explicit confirmation deletes the thread and its delegated descendants, never files.
    pub fn confirm_delete(&mut self) {
        if self.client.is_some() {
            let roots = if self.delete_roots.is_empty() {
                self.delete_thread.into_iter().collect()
            } else {
                std::mem::take(&mut self.delete_roots)
            };
            let local: std::collections::HashSet<_> = roots
                .iter()
                .copied()
                .filter(|id| self.threads.iter().any(|t| t.id == *id && t.client_draft))
                .collect();
            if !local.is_empty() {
                self.delete_with_undo(local.clone(), Action::Delete);
            }
            let roots: Vec<_> = roots.into_iter().filter(|id| !local.contains(id)).collect();
            if let Some(id) = roots.first() {
                self.forward("thread.delete", json!({"threadId":id,"threadIds":roots}));
            }
            self.delete_thread = None;
            return;
        }
        let Some(id) = self.delete_thread.take() else {
            return;
        };
        let roots = if self.delete_roots.is_empty() {
            vec![id]
        } else {
            std::mem::take(&mut self.delete_roots)
        };
        let ids: std::collections::HashSet<_> = roots
            .into_iter()
            .flat_map(|id| self.deletion_ids(id))
            .collect();
        self.delete_with_undo(ids, Action::Delete);
    }
    fn delete_with_undo(&mut self, ids: std::collections::HashSet<usize>, action: Action) {
        self.deleted_threads.clear();
        self.deleted_records = Some(DeletedRecords {
            tasks: self
                .tasks
                .iter()
                .filter(|task| {
                    ids.contains(&task.parent_thread_id) || ids.contains(&task.child_thread_id)
                })
                .cloned()
                .collect(),
            homes: self
                .captain_homes
                .iter()
                .filter(|(_, home)| ids.contains(home))
                .map(|(captain, home)| (*captain, *home))
                .collect(),
            projects: self
                .projects
                .entries
                .iter()
                .filter_map(|project| {
                    project
                        .captain_thread
                        .filter(|id| ids.contains(id))
                        .map(|home| (project.id, home))
                })
                .collect(),
        });
        self.undo = None;
        for id in &ids {
            if let Some(index) = self.threads.iter().position(|t| t.id == *id) {
                self.remember(Action::Delete, index);
            }
        }
        self.remove_threads(&ids);
        if let Some(notice) = &mut self.undo {
            notice.action = action;
        }
    }
    /// Removes threads and their records, then selects the survivor and saves. The caller
    /// handles undo and tab refusals.
    fn remove_threads(&mut self, ids: &std::collections::HashSet<usize>) {
        let current = self.current().id;
        let local_only = self.client.is_some()
            && self
                .threads
                .iter()
                .filter(|t| ids.contains(&t.id))
                .all(|t| t.client_draft);
        let project = self.current().project;
        let seed = self.composer_seed();
        let neighbour = self
            .tabs
            .iter()
            .position(|id| *id == current)
            .and_then(|position| {
                self.tabs[position + 1..]
                    .iter()
                    .find(|id| !ids.contains(id))
                    .or_else(|| {
                        self.tabs[..position]
                            .iter()
                            .rev()
                            .find(|id| !ids.contains(id))
                    })
                    .copied()
            });
        let mut parents = Vec::new();
        for thread in &mut self.threads {
            if ids.contains(&thread.id) {
                let _ = thread.stop_task();
                thread.disconnect();
                if let Some(parent) = thread.parent.filter(|id| !ids.contains(id)) {
                    parents.push((parent, thread.id));
                }
                if self.runtime_enabled
                    && let Ok(server) = crate::mcp::server(self.wake.clone())
                {
                    server.revoke(thread.id);
                }
            }
        }
        let mut removed = Vec::new();
        self.threads.retain_mut(|thread| {
            if ids.contains(&thread.id) {
                // Keep the disconnected in-memory transcript for the five-second delete undo.
                if self.undo.as_ref().is_some_and(|n| {
                    n.action == Action::Delete && n.entries.iter().any(|e| e.id == thread.id)
                }) {
                    removed.push(std::mem::replace(thread, Thread::new(0)));
                }
                false
            } else {
                true
            }
        });
        self.deleted_threads.extend(removed);
        self.captain_homes.retain(|_, home| !ids.contains(home));
        self.sync_memory_links();
        self.tasks.retain(|task| {
            !ids.contains(&task.parent_thread_id) && !ids.contains(&task.child_thread_id)
        });
        self.tabs.retain(|id| !ids.contains(id));
        for project in &mut self.projects.entries {
            if project.captain_thread.is_some_and(|id| ids.contains(&id)) {
                project.captain_thread = None;
            }
        }
        for (parent, id) in parents {
            if let Some(thread) = self.threads.iter_mut().find(|t| t.id == parent) {
                thread.enqueue_injection(format!(
                    "Shina: task #{id} was deleted by the user. Status: cancelled."
                ));
            }
        }
        let waiters = std::mem::take(&mut self.task_waiters);
        for waiter in waiters {
            if ids.contains(&waiter.thread) {
                let _ = waiter
                    .reply
                    .send(Err("The task was deleted by the user.".into()));
            } else {
                self.task_waiters.push(waiter);
            }
        }
        self.active = self
            .threads
            .iter()
            .position(|t| t.id == current)
            .or_else(|| self.threads.iter().position(|t| Some(t.id) == neighbour))
            .unwrap_or(0);
        if ids.contains(&current) && neighbour.is_none() {
            if self.client.is_some() {
                self.new_client_draft_with_seed(project, &seed);
            } else {
                self.create_thread_in(project, &[], &seed);
            }
        } else if ids.contains(&current) {
            self.tab_epoch += 1;
            self.focus_tab(self.active, !local_only);
        }
        self.checkpoint();
    }
    pub fn pin_thread(&mut self, id: usize) {
        if self.forward("thread.lifecycle", json!({"threadId":id,"action":"pin"})) {
            return;
        }
        if self
            .threads
            .iter()
            .any(|t| t.id == id && self.is_captain_home(t))
        {
            return;
        }
        let Some(index) = self
            .threads
            .iter()
            .position(|t| t.id == id && t.machine == "local")
        else {
            return;
        };
        if self.threads[index].pinned() {
            self.remember(Action::Unpin, index);
            self.threads[index].life.pinned_at = None;
            self.threads[index].life.pin_order_key = None;
        } else {
            self.invalidate_undo(id);
            let key = self.first_pin_key(id);
            self.threads[index].life.pin(lifecycle::now(), Some(key));
        }
        self.checkpoint();
    }
    pub fn archive_thread(&mut self, id: usize) {
        if !self.headless
            && self.preferences.confirm_archive
            && self.threads.iter().any(|t| t.id == id && !t.archived())
        {
            if self.settings.archive.is_none() {
                self.settings.archive_targets.clear();
            }
            if !self.settings.archive_targets.contains(&id) {
                self.settings.archive_targets.push(id);
            }
            self.settings.archive = Some(id);
            return;
        }
        self.archive_confirmed(id);
    }
    pub fn archive_confirmed(&mut self, id: usize) {
        self.archive_thread_record(id, false);
    }
    fn archive_thread_record(&mut self, id: usize, keep_blank: bool) {
        if self
            .threads
            .iter()
            .any(|t| t.id == id && t.client_draft && t.blank_draft())
        {
            self.delete_with_undo(std::collections::HashSet::from([id]), Action::Archive);
            return;
        }
        if self.forward(
            "thread.lifecycle",
            json!({"threadId":id,"action":"archive","keepBlank":keep_blank}),
        ) {
            return;
        }
        let Some(index) = self
            .threads
            .iter()
            .position(|t| t.id == id && t.machine == "local")
        else {
            return;
        };
        if self.lifecycle_busy(id) {
            self.lifecycle_error = Some("Stop this thread and its crew before archiving.".into());
            return;
        }
        // A blank draft has nothing to keep, so archiving it never leaves an archive row.
        if self.threads[index].blank_draft() && !keep_blank {
            self.delete_with_undo(std::collections::HashSet::from([id]), Action::Archive);
            return;
        }
        if self.threads[index].archived() {
            self.invalidate_undo(id);
            self.threads[index].life.archived_at = None;
        } else {
            self.remember(Action::Archive, index);
            self.threads[index].life.archived_at = Some(lifecycle::now());
            self.threads[index].disconnect();
            self.remove_tab(id, false);
        }
        self.checkpoint();
    }
    fn first_pin_key(&mut self, except: usize) -> String {
        let mut pins: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.pinned() && t.id != except)
            .map(|(i, _)| i)
            .collect();
        pins.sort_by(|a, b| {
            lifecycle::compare(
                Section::Pinned,
                &self.threads[*a].life,
                &self.threads[*b].life,
            )
        });
        let after = pins
            .first()
            .and_then(|i| self.threads[*i].life.pin_order_key.as_deref());
        if let Some(key) = lifecycle::pin_key_between(None, after) {
            return key;
        }
        let keys = lifecycle::spread_pin_keys(pins.len() + 1);
        for (index, key) in pins.into_iter().zip(keys.iter().skip(1)) {
            self.threads[index].life.pin_order_key = Some(key.clone());
        }
        keys[0].clone()
    }
    pub fn lifecycle_busy(&self, id: usize) -> bool {
        self.threads.iter().any(|t| {
            (t.id == id || t.parent == Some(id))
                && (t.status.busy() || t.request.is_some() || t.configuring || t.queued())
        })
    }
    fn invalidate_undo(&mut self, id: usize) {
        if let Some(notice) = &mut self.undo {
            notice.entries.retain(|e| e.id != id);
        }
        if self.undo.as_ref().is_some_and(|n| n.entries.is_empty()) {
            self.undo = None;
        }
    }
    fn remember(&mut self, action: Action, index: usize) {
        if action != Action::Delete {
            self.deleted_threads.clear();
            self.deleted_records = None;
        }
        let t = &self.threads[index];
        let mut steps = lifecycle::plan_undo(action, &t.life, &t.draft.text);
        if action == Action::Discard && !t.draft_images.is_empty() {
            steps.push(UndoStep::RestoreImages(t.draft_images.clone()));
        }
        if matches!(
            action,
            Action::Settle | Action::Snooze | Action::Archive | Action::Discard | Action::Delete
        ) && let Some(position) = self.tabs.iter().position(|id| *id == t.id)
        {
            steps.push(UndoStep::RestoreTab(position, self.current().id == t.id));
        }
        UndoNotice::push(&mut self.undo, action, t.id, steps, lifecycle::now());
    }
    pub fn settle_thread(&mut self, id: usize) {
        if self.forward("thread.lifecycle", json!({"threadId":id,"action":"settle"})) {
            return;
        }
        if self
            .threads
            .iter()
            .any(|t| t.id == id && self.is_captain_home(t))
        {
            return;
        }
        let Some(i) = self
            .threads
            .iter()
            .position(|t| t.id == id && t.machine == "local")
        else {
            return;
        };
        if self.threads[i].life.settled() {
            self.invalidate_undo(id);
            self.threads[i].life.unsettle(lifecycle::now(), true);
        } else {
            if self.lifecycle_busy(id) {
                self.lifecycle_error =
                    Some("Stop this thread and answer pending requests before settling.".into());
                return;
            }
            self.remember(Action::Settle, i);
            self.threads[i].life.settle(lifecycle::now());
            self.threads[i].disconnect();
            self.advance_parked(id);
        }
        self.checkpoint();
    }
    fn advance_parked(&mut self, id: usize) {
        self.remove_tab(id, true);
    }
    /// Ported from KSG Code's Sidebar settle advance and useThreadActions archive fallback.
    /// Existing tabs take precedence. Only the last tab can open a replacement.
    fn remove_tab(&mut self, id: usize, settle: bool) {
        if !self.tabs.contains(&id) {
            return;
        }
        if self.current().id != id {
            self.tabs.retain(|tab| *tab != id);
            return;
        }
        self.tab_epoch += 1;
        if let Some(neighbour) = tab_neighbour(&self.tabs, id)
            && let Some(index) = self.threads.iter().position(|t| t.id == neighbour)
        {
            self.tabs.retain(|tab| *tab != id);
            self.focus_thread(index);
            return;
        }
        let project = self.current().project;
        let order = self.sidebar_threads();
        let start = order
            .iter()
            .position(|i| self.threads[*i].id == id)
            .unwrap_or(0);
        for offset in 1..=if settle { order.len() } else { 0 } {
            let i = order[(start + offset) % order.len()];
            let t = &self.threads[i];
            if self.valid_after_park(t, id) {
                self.tabs.retain(|tab| *tab != id);
                self.select_thread(i);
                return;
            }
        }
        if self.client.is_some() {
            self.tabs.retain(|tab| *tab != id);
            self.new_client_draft(project);
            return;
        }
        self.tabs.retain(|tab| *tab != id);
        self.new_thread_in(project);
    }
    fn valid_after_park(&self, t: &Thread, parked: usize) -> bool {
        t.id != parked
            && !t.archived()
            && !matches!(
                self.thread_section(t, lifecycle::now()),
                Section::Snoozed | Section::Settled
            )
    }
    pub fn can_snooze(&self, id: usize) -> bool {
        self.threads.iter().find(|t| t.id == id).is_some_and(|t| {
            t.machine == "local"
                && !self.is_captain_home(t)
                && !t.archived()
                && !self.row_pending(t)
                && !t.queued()
        })
    }
    pub fn open_snooze(&mut self, id: usize) {
        self.row_menu = None;
        self.thread_menu = false;
        self.snooze_menu = Some((
            id,
            lifecycle::snooze_choices_from(self.thread_defaults.snooze_presets()),
        ));
        self.snooze_targets = if self.selected_threads.contains(&id) {
            self.selected_threads.clone()
        } else {
            vec![id]
        };
        self.custom_snooze_open = false;
        self.lifecycle_error = None;
    }
    pub fn snooze_thread(&mut self, id: usize, until: u64) {
        if self.forward(
            "thread.lifecycle",
            json!({"threadId":id,"action":"snooze","until":until}),
        ) {
            self.snooze_menu = None;
            return;
        }
        if until <= lifecycle::now() || !self.can_snooze(id) {
            self.lifecycle_error =
                Some("Choose a future snooze time after answering pending requests.".into());
            return;
        }
        let i = self.threads.iter().position(|t| t.id == id).unwrap();
        self.remember(Action::Snooze, i);
        self.threads[i].life.snooze(lifecycle::now(), until);
        self.snooze_menu = None;
        self.advance_parked(id);
        self.checkpoint();
    }
    pub fn unsnooze_thread(&mut self, id: usize) {
        if self.forward(
            "thread.lifecycle",
            json!({"threadId":id,"action":"unsnooze"}),
        ) {
            return;
        }
        self.invalidate_undo(id);
        if let Some(t) = self
            .threads
            .iter_mut()
            .find(|t| t.id == id && t.machine == "local")
        {
            t.life.unsnooze(lifecycle::now());
        }
        self.checkpoint();
    }
    pub fn discard_draft(&mut self, id: usize) {
        if self.threads.iter().any(|t| t.id == id && t.client_draft) {
            self.delete_with_undo(std::collections::HashSet::from([id]), Action::Discard);
            return;
        }
        if self.client.is_some() {
            if let Some(index) = self.threads.iter().position(|t| t.id == id) {
                if self.threads[index].blank_draft() {
                    self.forward(
                        "thread.lifecycle",
                        json!({"threadId":id,"action":"discard"}),
                    );
                    return;
                }
                self.remember(Action::Discard, index);
                self.threads[index].draft.replace_text("");
                self.threads[index].draft_images.clear();
                if self.threads[index].is_draft() {
                    // Draft text belongs to this device. Keep the server record so local Undo
                    // can restore it without sending the discarded text to the server.
                    self.forward(
                        "thread.lifecycle",
                        json!({"threadId":id,"action":"archive","keepBlank":true}),
                    );
                    self.threads[index].life.archived_at = Some(lifecycle::now());
                } else {
                    self.remove_tab(id, false);
                }
                self.checkpoint();
            }
            return;
        }
        let Some(i) = self.threads.iter().position(|t| t.id == id) else {
            return;
        };
        // Keep an empty draft only in the short undo window, never as an archive row.
        if self.threads[i].blank_draft()
            && self.threads[i].messages.is_empty()
            && self.threads[i].unused_session()
        {
            self.delete_with_undo(std::collections::HashSet::from([id]), Action::Discard);
            return;
        }
        self.remember(Action::Discard, i);
        self.threads[i].draft.replace_text("");
        self.threads[i].draft_images.clear();
        self.threads[i].attachment_error = None;
        if self.threads[i].messages.is_empty() && self.threads[i].unused_session() {
            self.threads[i].life.archived_at = Some(lifecycle::now());
        }
        self.remove_tab(id, false);
        self.checkpoint();
    }
    fn client_undo_is_local(&self) -> bool {
        self.undo.as_ref().is_some_and(|notice| {
            self.deleted_records.is_some()
                || notice.entries.iter().all(|entry| {
                    self.threads
                        .iter()
                        .any(|t| t.id == entry.id && t.client_draft)
                })
                || notice.action == Action::Discard
                    && notice.entries.iter().any(|entry| {
                        entry
                            .steps
                            .iter()
                            .any(|step| matches!(step, UndoStep::RestoreDraft(_)))
                    })
        })
    }
    pub fn undo_thread_action(&mut self) {
        let at = lifecycle::now();
        if self
            .undo
            .as_ref()
            .is_some_and(|notice| notice.expires <= at)
        {
            self.undo = None;
            self.deleted_threads.clear();
            self.deleted_records = None;
            return;
        }
        if self.client.is_some() && self.undo.is_some() && !self.client_undo_is_local() {
            self.forward("thread.undo", json!({"expected":self.undo}));
            return;
        }
        let Some(notice) = self.undo.take().filter(|n| n.expires > at) else {
            return;
        };
        if self.deleted_records.is_some() {
            self.threads.append(&mut self.deleted_threads);
            if let Some(records) = self.deleted_records.take() {
                for task in records.tasks {
                    if !self.tasks.iter().any(|t| t.id == task.id)
                        && self.threads.iter().any(|t| t.id == task.parent_thread_id)
                        && self.threads.iter().any(|t| t.id == task.child_thread_id)
                    {
                        self.tasks.push(task);
                    }
                }
                for (captain, home) in records.homes {
                    if self.threads.iter().any(|t| t.id == home) {
                        self.captain_homes.entry(captain).or_insert(home);
                    }
                }
                for (id, home) in records.projects {
                    if let Some(project) = self.projects.entries.iter_mut().find(|p| p.id == id)
                        && project.captain_thread.is_none()
                    {
                        project.captain_thread = Some(home);
                    }
                }
                self.sync_memory_links();
            }
        }
        let mut restore_tabs = Vec::new();
        let mut restore_drafts = Vec::new();
        let mut focus = None;
        for entry in notice.entries {
            let Some(i) = self.threads.iter().position(|t| t.id == entry.id) else {
                continue;
            };
            for step in entry.steps {
                if self.threads[i].machine != "local" {
                    break;
                }
                let t = &mut self.threads[i];
                match step {
                    UndoStep::Unsettle => t.life.unsettle(at, true),
                    UndoStep::Unsnooze => t.life.unsnooze(at),
                    UndoStep::Pin(key) => t.life.pin(at, key),
                    UndoStep::Snooze(until) if until > at => t.life.snooze(at, until),
                    UndoStep::Snooze(_) => {}
                    UndoStep::Unarchive => t.life.archived_at = None,
                    UndoStep::RestoreDraft(draft) => {
                        t.draft.replace_text(&draft);
                        t.life.archived_at = None;
                        restore_drafts.push(entry.id);
                    }
                    UndoStep::RestoreImages(images) => {
                        t.draft_images = images;
                        t.attachment_error =
                            attachments::validate_prompt_images(&t.draft_images).err();
                    }
                    UndoStep::RestoreTab(position, was_active) => {
                        restore_tabs.push((entry.id, position));
                        if was_active {
                            focus = Some(entry.id);
                        }
                    }
                }
            }
        }
        if self.client.is_some() && notice.action == Action::Discard {
            for id in restore_drafts {
                self.forward(
                    "thread.lifecycle",
                    json!({"threadId":id,"action":"unarchive"}),
                );
            }
        }
        self.restore_tabs(restore_tabs, focus);
        self.checkpoint();
    }
    fn restore_tabs(&mut self, mut tabs: Vec<(usize, usize)>, preferred: Option<usize>) {
        tabs.sort_by_key(|(_, position)| *position);
        let mut focus = None;
        for (id, position) in tabs {
            if let Some(index) = self.threads.iter().position(|t| t.id == id) {
                self.tabs.retain(|tab| *tab != id);
                self.tabs.insert(position.min(self.tabs.len()), id);
                focus = Some(index);
            }
        }
        if let Some(index) = preferred
            .and_then(|id| {
                self.threads
                    .iter()
                    .position(|t| t.id == id && self.tabs.contains(&id))
            })
            .or(focus)
        {
            self.tab_epoch += 1;
            self.focus_thread(index);
        }
    }
    pub fn drop_thread(&mut self, id: usize, to: Section, before: Option<usize>) {
        if self.forward(
            "thread.drop",
            json!({"threadId":id,"to":format!("{to:?}"),"before":before}),
        ) {
            return;
        }
        if self
            .threads
            .iter()
            .any(|t| t.id == id && self.is_captain_home(t))
        {
            return;
        }
        let Some(i) = self
            .threads
            .iter()
            .position(|t| t.id == id && t.machine == "local")
        else {
            return;
        };
        let from = self.thread_section(&self.threads[i], lifecycle::now());
        match lifecycle::plan_drop(from, to) {
            DropPlan::Pin | DropPlan::ReorderPin => {
                self.invalidate_undo(id);
                let mut pins: Vec<_> = self
                    .threads
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.pinned() && t.id != id)
                    .map(|(i, _)| i)
                    .collect();
                pins.sort_by(|a, b| {
                    lifecycle::compare(
                        Section::Pinned,
                        &self.threads[*a].life,
                        &self.threads[*b].life,
                    )
                });
                let place = before
                    .and_then(|id| pins.iter().position(|i| self.threads[*i].id == id))
                    .unwrap_or(pins.len());
                let a = place
                    .checked_sub(1)
                    .and_then(|p| self.threads[pins[p]].life.pin_order_key.as_deref());
                let b = pins
                    .get(place)
                    .and_then(|p| self.threads[*p].life.pin_order_key.as_deref());
                let usable = (place == 0 || a.is_some()) && (place == pins.len() || b.is_some());
                let key = usable.then(|| lifecycle::pin_key_between(a, b)).flatten();
                self.threads[i].life.pin(lifecycle::now(), key.clone());
                if key.is_none() {
                    pins.insert(place, i);
                    for (i, key) in pins.into_iter().zip(lifecycle::spread_pin_keys(
                        self.threads.iter().filter(|t| t.pinned()).count(),
                    )) {
                        self.threads[i].life.pin_order_key = Some(key);
                    }
                }
            }
            DropPlan::Unpin => self.pin_thread(id),
            DropPlan::Unsettle | DropPlan::Settle => self.settle_thread(id),
            DropPlan::Wake => self.unsnooze_thread(id),
            DropPlan::None => return,
        }
        self.checkpoint();
    }
    /// Called each minute and after run-end events. PR integration can call this same hook.
    pub fn sweep_lifecycle(&mut self, at: u64) -> bool {
        self.last_sweep = at;
        let eligible: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                if t.is_draft() || t.machine != "local" {
                    return None;
                }
                t.life
                    .auto_settle_at(
                        at,
                        self.auto_settle_days,
                        self.lifecycle_busy(t.id),
                        t.captain_home || t.delegated || self.is_captain_home(t),
                    )
                    .map(|stamp| (i, stamp))
            })
            .collect();
        for (i, stamp) in &eligible {
            self.threads[*i].life.settle(*stamp);
            self.threads[*i].disconnect();
        }
        !eligible.is_empty()
    }
    /// Open the in-draft captain selector (from the composer footer or sidebar Captains "+").
    /// Ported from KSG Code's new-thread / composer picker flow: starts a ready draft in the
    /// current workspace if needed, focuses the composer, and opens the in-place captain selector.
    pub fn open_captain_selector(&mut self) {
        if self.current().captain.is_some() || !self.current().is_draft() {
            let project = self.current().project;
            self.new_thread_in(project);
        }
        self.captain_project = Some(self.current().project);
        self.captain_picker = true;
        self.captain_picker_query = TextEdit::default();
        self.captain_picker_query.focused = true;
        self.captain_picker_highlight = 0;
        self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Explicit);
    }

    pub fn choose_project_captain(&mut self, id: ProjectId) {
        if let Some(captain) = self.project(Some(id)).and_then(|p| p.captain) {
            self.projects.captain_target = Some(id);
            self.open_captain(captain);
        } else {
            self.new_thread_in(Some(id));
            self.projects.captain_target = Some(id);
            self.captain_project = Some(Some(id));
            self.captain_picker = true;
            self.captain_picker_query = TextEdit::default();
            self.captain_picker_query.focused = true;
            self.captain_picker_highlight = 0;
            self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Explicit);
        }
    }
    pub fn toggle_thread_auto_settle(&mut self, id: usize) {
        let disabled = self
            .threads
            .iter()
            .find(|t| t.id == id)
            .is_some_and(|t| t.life.auto_settle_disabled_at.is_none());
        if self.forward(
            "thread.lifecycle",
            json!({"threadId":id,"action":"autoSettle","disabled":disabled}),
        ) {
            return;
        }
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.life.auto_settle_disabled_at = disabled.then(lifecycle::now);
        }
        self.checkpoint();
    }
    pub fn cycle_auto_settle_days(&mut self) {
        let days = match self.auto_settle_days {
            0 => 1,
            1 => 3,
            3 => 7,
            7 => 14,
            14 => 30,
            _ => 0,
        };
        if self.forward("settings.autoSettle", json!({"days":days})) {
            return;
        }
        self.auto_settle_days = days;
        self.sweep_lifecycle(lifecycle::now());
        self.checkpoint();
    }
    pub fn new_thread(&mut self) {
        let project = self.current().project;
        self.new_thread_in(project);
    }
    pub fn new_thread_in(&mut self, project: Option<ProjectId>) {
        if self.client.is_some() {
            self.new_client_draft(project);
            return;
        }
        let seed = self.composer_seed();
        self.create_thread_in(project, &[], &seed);
    }
    // Ported from KSG Code's useHandleNewThread composer model override.
    pub(crate) fn composer_seed(&self) -> Value {
        self.threads.get(self.active).map_or(Value::Null, |thread| {
            json!({"harness":thread.harness,"model":thread.selected_or_effective_model(),
                "thinking":thread.selected_or_effective_reasoning(),"access":thread.native.access.id(),
                "serviceTier":thread.harness_state["serviceTier"]})
        })
    }
    pub(crate) fn inherit_composer(&mut self, index: usize, seed: &Value) {
        if seed.is_null() {
            let harness = self.default_harness.clone();
            self.threads[index].harness = harness.clone();
            self.threads[index].native.access =
                AccessMode::parse(&self.thread_defaults.access).unwrap_or(AccessMode::Supervised);
            self.threads[index].native.access = self.effective_access_mode(&self.threads[index]);
            if let Some(selection) = self.thread_defaults.models.get(&harness)
                && !selection.provider.is_empty()
                && !selection.model.is_empty()
            {
                let thread = &mut self.threads[index];
                thread.selected_model = Some((selection.provider.clone(), selection.model.clone()));
                thread.model_provider = selection.provider.clone();
                thread.model_id = selection.model.clone();
                thread.model = selection.model.clone();
                thread.selected_thinking =
                    (!selection.effort.is_empty()).then(|| selection.effort.clone());
                thread.thinking = selection.effort.clone();
            }
            return;
        }
        let Some(harness) = seed["harness"]
            .as_str()
            .filter(|id| self.validate_harness(id).is_ok())
        else {
            return;
        };
        if let Some(access) = seed["access"].as_str() {
            self.threads[index].native.access =
                AccessMode::parse(access).unwrap_or(AccessMode::Supervised);
        }
        let models = self.harness_models(harness);
        let selection: Option<(String, String)> =
            serde_json::from_value(seed["model"].clone()).ok().flatten();
        let Some(model) = selection.as_ref().and_then(|(provider, id)| {
            models
                .iter()
                .find(|m| &m.provider == provider && &m.id == id)
        }) else {
            self.inherit_composer(index, &Value::Null);
            // Access inheritance does not depend on model discovery. A missing model
            // must never replace the source thread's read-only mode with a broader default.
            if let Some(access) = seed["access"].as_str() {
                self.threads[index].native.access =
                    AccessMode::parse(access).unwrap_or(AccessMode::Supervised);
            }
            return;
        };
        let thread = &mut self.threads[index];
        if thread.harness != harness {
            thread.prompt_suggestion = None;
        }
        thread.harness = harness.into();
        thread.models = models.clone();
        thread.model_provider = model.provider.clone();
        thread.model_id = model.id.clone();
        thread.model = model.name.clone();
        thread.context_window = model.context_window;
        thread.supports_images = model.supports_images;
        thread.selected_model = selection;
        thread.thinking_levels = model.thinking_levels.clone();
        if let Some(level) = seed["thinking"]
            .as_str()
            .filter(|level| model.thinking_levels.iter().any(|v| v == level))
        {
            thread.selected_thinking = Some(level.into());
            thread.thinking = level.into();
        } else if !thread.thinking_levels.contains(&thread.thinking) {
            thread.selected_thinking = None;
            thread.thinking.clear();
        }
        if harness == "codex"
            && let Some(tier) = seed["serviceTier"].as_str().filter(|tier| {
                model
                    .service_tiers
                    .iter()
                    .any(|option| option["id"] == *tier)
            })
        {
            thread.harness_state["serviceTier"] = json!(tier);
        }
        thread.native.access = resolve_compatible_access_mode(
            thread.native.access,
            supported_access_modes(&thread.harness),
        )
        .1;
        let clamped = self.effective_access_mode(&self.threads[index]);
        self.threads[index].native.access = clamped;
    }
    fn create_thread_in(&mut self, project: Option<ProjectId>, occupied: &[usize], seed: &Value) {
        if project.is_some() && self.project(project).is_none() {
            return;
        }
        // T3 keeps one draft thread per logical project and reopens it instead of adding another.
        let reuse = self.threads.iter().position(|t| {
            t.project == project
                && !t.delegated
                && !t.captain_home
                && t.blank_draft()
                && !occupied.contains(&t.id)
        });
        let index = reuse.unwrap_or_else(|| {
            let mut thread = Thread::new(self.next_id);
            thread.harness = self.default_harness.clone();
            thread.project = project;
            thread.env_mode = Some(self.effective_default_thread_env_mode());
            if let Some(project) = self.project(project) {
                thread.set_machine(project.machine.clone());
            }
            self.threads.push(thread);
            self.next_id += 1;
            self.threads.len() - 1
        });
        self.show_archive = false;
        self.project_picker = None;
        if match self.project_scope {
            ProjectScope::All => false,
            ProjectScope::Project(id) => project != Some(id),
            ProjectScope::NoProject => project.is_some(),
        } {
            self.project_scope = ProjectScope::All;
            self.scope_additional.clear();
        }
        if reuse.is_none() {
            self.apply_catalog();
        }
        if reuse.is_none() || self.threads[index].selected_model.is_none() {
            self.inherit_composer(index, seed);
            if reuse.is_none() {
                self.apply_sticky_composer_to_draft(index);
            }
        }
        self.select_thread(index);
        self.sync_fallback_model(lifecycle::now() as i64);
    }
    pub fn open_captain(&mut self, captain: usize) {
        if self.forward("captain.open", json!({"captain":captain,"project":self.captain_project.unwrap_or(self.current().project),"assignProject":self.projects.captain_target})) {
            self.captain_picker = false;
            return;
        }
        if captain >= self.captains.len() {
            return;
        }
        let explicit = self.captain_project.take();
        let existing_home = self.captain_home(captain);
        let requested_project =
            explicit.unwrap_or(self.projects.captain_target.or(self.current().project));
        if !self.captain_scopes.contains_key(&captain) {
            let scope = if existing_home.is_some() {
                self.captain_scope(captain)
            } else {
                requested_project.into_iter().collect()
            };
            self.captain_scopes.insert(captain, scope);
        } else if self.projects.captain_target.is_some()
            && let Some(project) = requested_project
        {
            let mut scope = self.captain_scope(captain);
            if !scope.contains(&project) {
                scope.push(project);
                if let Err(error) = self.set_captain_scope(captain, scope) {
                    self.lifecycle_error = Some(error);
                    return;
                }
            }
        }
        let project_id = self.captain_scope(captain).first().copied();
        let machine = self
            .project(project_id)
            .map(|p| p.machine.clone())
            .unwrap_or_else(|| "local".into());
        if let Some(index) = self
            .captain_home(captain)
            .and_then(|home| self.threads.iter().position(|t| t.id == home))
        {
            self.threads[index].life.archived_at = None;
        } else {
            let mut thread = Thread::new(self.next_id);
            thread.harness = self.default_harness.clone();
            self.next_id += 1;
            thread.title = self.captains[captain].name.clone();
            thread.captain = Some(captain);
            thread.captain_home = true;
            thread.native.access = AccessMode::ReadOnly;
            thread.project = project_id;
            thread.set_machine(machine);
            self.threads.push(thread);
        }
        let index = self
            .threads
            .iter()
            .position(|t| t.captain == Some(captain) && Some(t.id) == existing_home)
            .unwrap_or(self.threads.len() - 1);
        let home = self.threads[index].id;
        self.captain_homes.insert(captain, home);
        if let Some(project) = self
            .projects
            .entries
            .iter_mut()
            .find(|p| Some(p.id) == requested_project)
            && project.captain.is_none()
            && (existing_home.is_none() || self.projects.captain_target.is_some())
        {
            project.captain = Some(captain);
            project.captain_thread = Some(home);
        }
        self.sync_memory_links();
        self.show_archive = false;
        self.captain_picker = false;
        self.projects.captain_target = None;
        self.select_thread(index);
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
        self.checkpoint();
    }

    pub fn begin_new_captain(&mut self) {
        let suggested = self
            .projects
            .entries
            .iter()
            .find(|p| {
                !p.hidden
                    && !self
                        .threads
                        .iter()
                        .any(|t| t.captain_home && t.project == Some(p.id))
            })
            .map(|p| p.id);
        if self.captain_project.is_none() {
            self.captain_project = Some(self.current().project.or(suggested));
        }
        self.captain_picker = false;
        let used_names: std::collections::HashSet<String> = self
            .captains
            .iter()
            .map(|c| c.name.to_lowercase())
            .collect();
        let used_portraits: std::collections::HashSet<usize> =
            self.captains.iter().map(|c| c.portrait).collect();
        let (portrait, suggested_name) = (0..crate::visuals::PORTRAITS)
            .find(|i| {
                !used_portraits.contains(i)
                    && !used_names.contains(&crate::visuals::CAPTAIN_NAMES[*i].to_lowercase())
            })
            .map(|i| (i, crate::visuals::CAPTAIN_NAMES[i].to_string()))
            .unwrap_or_else(|| {
                let mut idx = self.captains.len() + 1;
                let candidate = loop {
                    let c = format!("Captain {idx}");
                    if !used_names.contains(&c.to_lowercase()) {
                        break c;
                    }
                    idx += 1;
                };
                (self.captains.len() % crate::visuals::PORTRAITS, candidate)
            });
        let mut editor = CaptainEditor::new();
        editor.server = self.machines.selected.clone();
        editor.portrait = portrait;
        editor.name.replace_text(&suggested_name);
        self.captain_editor = Some(editor);
    }

    pub fn edit_captain(&mut self, captain: usize) {
        self.cancel_captain_flow();
        let Some(profile) = self.captains.get(captain) else {
            return;
        };
        let mut editor = CaptainEditor::new();
        editor.editing = Some(captain);
        editor.server = self.machines.selected.clone();
        editor.name.replace_text(&profile.name);
        editor.role.replace_text(&profile.role);
        editor.instructions.replace_text(&profile.instructions);
        editor.portrait = profile.portrait;
        self.captain_editor = Some(editor);
    }

    pub fn cancel_captain_flow(&mut self) {
        self.captain_picker = false;
        self.captain_editor = None;
        self.captain_project = None;
        self.projects.captain_target = None;
    }

    pub fn save_captain(&mut self) {
        if self.client.is_some() {
            if let Some(editor) = &self.captain_editor {
                if editor.server != self.machines.selected {
                    return;
                }
                let params = json!({"captain":editor.editing,"record":editor.profile().to_value(),"project":self.captain_project,"assignProject":self.projects.captain_target});
                match self
                    .request_tab_command("captain.put", params, TabIntent::Open(None))
                    .expect("client")
                {
                    Ok(_) => self.captain_editor = None,
                    Err(error) => self.captain_editor.as_mut().unwrap().error = Some(error),
                }
            }
            return;
        }
        let Some(editor) = &self.captain_editor else {
            return;
        };
        let profile = editor.profile();
        let editing = editor.editing;
        let error = profile
            .validate()
            .err()
            .or_else(|| {
                self.captains
                    .iter()
                    .enumerate()
                    .any(|(i, existing)| {
                        Some(i) != editing
                            && existing.name.to_lowercase() == profile.name.to_lowercase()
                    })
                    .then(|| {
                        "That captain name is already in the roster. Choose a different name."
                            .into()
                    })
            })
            .or_else(|| {
                editing
                    .filter(|&id| {
                        self.threads
                            .iter()
                            .any(|t| t.captain == Some(id) && t.status.busy())
                    })
                    .map(|_| {
                        "Stop this captain's running threads before editing its profile.".into()
                    })
            });
        if let Some(error) = error {
            self.captain_editor.as_mut().unwrap().error = Some(error);
            return;
        }
        let id = if let Some(id) = editing {
            if id >= self.captains.len() {
                return;
            }
            self.captains[id] = profile;
            // Restart idle connections on demand so the next send uses the edited profile.
            for thread in self.threads.iter_mut().filter(|t| t.captain == Some(id)) {
                thread.disconnect();
                if thread.captain_home && !thread.renamed {
                    thread.title = self.captains[id].name.clone();
                }
            }
            id
        } else {
            self.captains.push(profile);
            self.captains.len() - 1
        };
        self.captain_editor = None;
        self.open_captain(id);
        self.restore_composer_focus(crate::ui::composer_ui::FocusReason::MenuClosed);
    }
    /// The captain's most urgent live thread. Archived conversations do not vote.
    pub fn captain_status(&self, captain: usize) -> Status {
        self.captain_home(captain)
            .map(|home| self.home_status(home))
            .unwrap_or(Status::Ready)
    }
    pub fn home_status(&self, home: usize) -> Status {
        self.threads
            .iter()
            .filter(|t| {
                !t.archived()
                    && (t.id == home
                        || t.parent
                            .is_some_and(|parent| self.effective_home(parent) == home))
            })
            .filter(|t| t.id == home || t.request.as_ref().is_none_or(|r| r["method"] == "confirm"))
            .map(|t| t.status)
            .max_by_key(|s| s.urgency())
            .unwrap_or(Status::Ready)
    }
    /// Steps the zoom one notch in (`1`), out (`-1`), or back to 100% (`0`).
    pub fn step_zoom(&mut self, direction: i32) {
        let index = ZOOM_STEPS
            .iter()
            .position(|&z| (z - self.zoom).abs() < 0.001)
            .unwrap_or(4);
        self.zoom = match direction {
            0 => 1.0,
            d if d > 0 => ZOOM_STEPS[(index + 1).min(ZOOM_STEPS.len() - 1)],
            _ => ZOOM_STEPS[index.saturating_sub(1)],
        };
        self.checkpoint();
    }
    /// Models Shina can validate against, preferring the live catalog over the caller's last
    /// handshake metadata. The string records which source was used.
    fn delegation_models(&self, caller: &Thread) -> (Vec<AgentModel>, &'static str) {
        if caller.harness != "pi" {
            return (self.harness_models(&caller.harness), "caller");
        }
        let mut models = self.catalog.models.clone();
        if let Some(default) = &self.catalog.default_model
            && !models
                .iter()
                .any(|model| model.provider == default.provider && model.id == default.id)
        {
            models.push(default.clone());
        }
        if !models.is_empty() {
            return (models, "catalog");
        }
        if !caller.models.is_empty() {
            return (caller.models.clone(), "caller");
        }
        (models, "none")
    }

    fn default_selection(&self) -> Option<Selection> {
        let model = self.catalog.default_model.as_ref()?;
        let reasoning = (!self.catalog.default_thinking.trim().is_empty()
            && model
                .thinking_levels
                .iter()
                .any(|level| level == &self.catalog.default_thinking))
        .then(|| self.catalog.default_thinking.clone());
        Some(Selection {
            provider: model.provider.clone(),
            model: model.id.clone(),
            reasoning,
        })
    }

    fn inherited_selection(&self, caller_index: usize) -> Option<Selection> {
        let caller = self.threads.get(caller_index)?;
        caller.inherited_selection().or_else(|| {
            (caller.harness == "pi")
                .then(|| self.default_selection())
                .flatten()
        })
    }

    fn discovery_state(&self, source: &str) -> &'static str {
        if source == "caller" {
            return "caller_metadata";
        }
        let has_catalog = !self.catalog.models.is_empty() || self.catalog.default_model.is_some();
        if self.catalog.error.is_some() {
            return if has_catalog { "stale" } else { "failed" };
        }
        if self.catalog.remaining().is_some() {
            return if has_catalog { "refreshing" } else { "loading" };
        }
        if source == "none" {
            "not_started"
        } else {
            "ready"
        }
    }

    fn validate_delegation_selection(
        &self,
        models: &[AgentModel],
        selection: &Selection,
        explicit: bool,
    ) -> Result<(), String> {
        let provider_name = if selection.provider == "cpa" {
            "CPA"
        } else {
            &selection.provider
        };
        if models.is_empty() {
            return if explicit {
                Err("Model discovery is not ready, so Shina cannot validate the requested target. Call orchestrator_capabilities and retry when ready is true.".into())
            } else {
                Ok(())
            };
        }
        let Some(model) = models
            .iter()
            .find(|model| model.provider == selection.provider && model.id == selection.model)
        else {
            let available = models
                .iter()
                .take(6)
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(if explicit {
                format!(
                    "The requested {provider_name} model {} is not in the current catalog. Available model ids include: {available}. Call orchestrator_capabilities for the full list.",
                    selection.model
                )
            } else {
                format!(
                    "The inherited {provider_name} model {} is not in the current catalog. Pass target with an available model from orchestrator_capabilities.",
                    selection.model
                )
            });
        };
        if let Some(reasoning) = &selection.reasoning
            && !model.thinking_levels.iter().any(|level| level == reasoning)
        {
            let supported = if model.thinking_levels.is_empty() {
                "none".to_owned()
            } else {
                model.thinking_levels.join(", ")
            };
            return Err(format!(
                "Reasoning level {reasoning} is not supported by {provider_name} model {}. Supported levels: {supported}. Pass an exact level; Shina will not substitute or clamp one.",
                model.id
            ));
        }
        Ok(())
    }

    fn orchestrator_capabilities(&self, caller_index: usize) -> Value {
        let caller = &self.threads[caller_index];
        let (models, source) = self.delegation_models(caller);
        let default_selection = self.default_selection();
        let caller_selection = caller.inherited_selection();
        let inherit_selection = self.inherited_selection(caller_index);
        let model_values: Vec<Value> = models
            .iter()
            .map(|model| {
                json!({
                    "provider": model.provider,
                    "id": model.id,
                    "name": model.name,
                    "reasoning_levels": model.thinking_levels,
                })
            })
            .collect();
        let mut capabilities = json!({
            "ready": !models.is_empty(),
            "scope": self.scope_value(caller.captain),
            "workspace_required": caller.captain.is_some_and(|id| self.captain_scope(id).len() > 1),
            "discovery": {
                "state": self.discovery_state(source),
                "source": source,
                "error": self.catalog.error,
            },
            "runtime": {
                "harness": caller.harness,
                "providers": ["cpa"],
                "hosts": ["local"],
                "machine": caller.machine,
                "host_supported": caller.machine == "local",
            },
            "constraints": {
                "running_children_per_coordinator": tasks::RUNNING_LIMIT,
                "unfinished_tasks_per_coordinator": tasks::UNFINISHED_LIMIT,
                "direct_providers": false,
                "remote_hosts": true,
                "model_ids": "exact ids only",
                "reasoning_levels": "exact levels only, never clamped",
            },
            "default_selection": default_selection.map_or(Value::Null, |selection| selection.to_value()),
            "caller_selection": caller_selection.map_or(Value::Null, |selection| selection.to_value()),
            "inherit_selection": inherit_selection.map_or(Value::Null, |selection| selection.to_value()),
            "models": model_values,
            "notes": [
                "Pi uses CPA. Codex and ACP use their own configured providers and login.",
                "Linked peers run workers on their own projects and credentials. Workers cannot delegate further.",
                "Pass exact model ids and reasoning levels. Unknown or unsupported choices are rejected before the child is created and are never silently substituted or clamped.",
                "Omit target to inherit inherit_selection. Given target fields override the inherited selection.",
                "When ready is false, discovery has not produced a usable catalog. Do not invent model ids or levels."
            ],
        });
        self.add_harness_capabilities(&mut capabilities);
        let peers = crate::database::for_path(&self.data_dir.join("index.json"))
            .and_then(|database| database.peers().ok())
            .unwrap_or_else(|| self.peers.clone());
        capabilities["peers"] = json!(
            peers
                .iter()
                .map(crate::peer::Peer::summary)
                .collect::<Vec<_>>()
        );
        capabilities
    }

    /// Connects one thread's Pi with its captain's instructions and its Shina MCP link.
    pub fn connect_thread(&mut self, index: usize) {
        if index < self.threads.len()
            && self
                .tasks
                .iter()
                .any(|task| task.child_thread_id == self.threads[index].id && task.remote.is_some())
        {
            return;
        }
        if self.demo
            || !self.runtime_enabled
            || self.client.is_some()
            || self.ownership_denied
            || index >= self.threads.len()
        {
            return;
        }
        self.threads[index].server_owned = self.headless || self.persist_transcripts;
        let path = match self.thread_directory(index) {
            Ok(path) => path,
            Err(error) => {
                self.threads[index].fail_session(error);
                return;
            }
        };
        #[cfg(test)]
        {
            let tid = self.threads[index].id;
            self.test_connect_workspaces.push((tid, path.clone()));
            if let Some(hook) = self.test_connect_pi.as_mut()
                && let Some(pi) = hook(tid, &path)
            {
                let thread = &mut self.threads[index];
                thread.error = None;
                if !thread.pending_injections.is_empty() && !thread.status.busy() {
                    thread.arm_task();
                    thread.status = Status::Running;
                    thread.started = None;
                    thread.phase = "Working".into();
                } else if thread.status == Status::Failed {
                    thread.status = Status::Ready;
                }
                thread.initializing = false;
                thread.prewarm_attempted = true;
                thread.pi = Some(pi);
                thread.drive_session();
                return;
            }
        }
        if self.threads[index].project.is_none()
            && let Err(error) = std::fs::create_dir_all(&path)
        {
            self.threads[index].fail_session(format!("Could not create the chat folder: {error}"));
            return;
        }
        let wake = self.wake.clone();
        let thread_id = self.threads[index].id;
        self.sync_memory_links();
        let instructions = self.threads[index]
            .captain
            .and_then(|id| self.captains.get(id))
            .map(|captain| {
                format!(
                    "{}\n\n{}",
                    captain.prompt(),
                    self.threads[index].memory_prompt()
                )
            });
        if matches!(
            self.threads[index].harness.as_str(),
            "pi" | "codex" | "claude"
        ) || self.threads[index].session.is_none()
        {
            self.threads[index].memory_context = self.threads[index]
                .memory
                .as_ref()
                .map(|link| link.seen())
                .unwrap_or_default();
        }
        // Future project notes can augment this prompt using (thread.machine, thread.project).
        // Context storage is deliberately separate from persona and session persistence.
        // Shina's tools are an addition. If the MCP cannot start, Pi still works without them.
        let host = crate::mcp::server(wake.clone()).and_then(|server| {
            Ok(HostLink {
                url: server.url.clone(),
                token: server.token_for(thread_id),
                extension: if self.threads[index].harness == "pi" {
                    crate::mcp::extension_path()?
                } else {
                    PathBuf::new()
                },
                access_mode: self.threads[index].native.access.id().into(),
            })
        });
        let prompt_suggestions_enabled = self.thread_defaults.prompt_suggestions_enabled;
        let thread = &mut self.threads[index];
        thread.prompt_suggestions_enabled = prompt_suggestions_enabled && !thread.delegated;
        match host {
            Ok(host) => thread.host = Some(host),
            Err(error) => {
                thread.host = None;
                if thread.native.access != AccessMode::Full {
                    thread.fail_session(format!(
                        "The permission bridge could not start. Pi was not launched: {error}"
                    ));
                    return;
                }
                thread.diagnostic(&error);
            }
        }
        thread.connect_with_instructions(path, wake, instructions);
    }
    /// Answers a Shina MCP tool call on the UI thread. `caller` is the calling thread's id.
    fn mcp_call(&mut self, caller: usize, tool: &str, args: &Value) -> Result<Value, String> {
        if self
            .threads
            .iter()
            .any(|t| t.id == caller && t.peer_origin.is_some())
            && !(matches!(
                tool,
                "propose_memory"
                    | "list_memories"
                    | "search_memories"
                    | "orchestrator_capabilities"
                    | "list_threads"
                    | "read_thread"
            ) || tool == "task_status" && args["thread_id"] == caller)
        {
            return Err("A remote worker may read threads and memories, but cannot delegate or control unrelated work.".into());
        }
        if matches!(
            tool,
            "create_routine" | "list_routines" | "pause_routine" | "delete_routine"
        ) {
            return self.routine_mcp(caller, tool, args);
        }
        if matches!(tool, "delegate_task" | "task_status") {
            wait_seconds(args)?;
        }
        let caller_index = self
            .threads
            .iter()
            .position(|t| t.id == caller)
            .ok_or("The calling thread no longer exists.")?;
        let find = |state: &Self| -> Result<usize, String> {
            let id = args["thread_id"]
                .as_u64()
                .ok_or("thread_id must be a thread id from list_threads.")?;
            state
                .threads
                .iter()
                .position(|t| t.id as u64 == id)
                .ok_or_else(|| format!("There is no thread {id}."))
        };
        let captain_name = |state: &Self, captain: Option<usize>| -> Value {
            captain
                .and_then(|id| state.captains.get(id))
                .map(|c| Value::from(c.name.clone()))
                .unwrap_or(Value::Null)
        };
        match tool {
            "remember" | "forget_memory" | "list_memories" | "search_memories"
            | "propose_memory" => self.memory_call(caller_index, tool, args),
            "orchestrator_capabilities" => Ok(self.orchestrator_capabilities(caller_index)),
            "list_threads" => self.readable_threads(Some(caller), args),
            "read_thread" => {
                let id = args["threadId"].as_u64().ok_or("threadId is required.")?;
                let thread = self
                    .threads
                    .iter()
                    .find(|t| t.id as u64 == id)
                    .ok_or("Unknown thread.")?;
                let mut page = remote_tasks::transcript_page(thread, args)?;
                page["thread"]["project"] = json!(self.task_project_value(thread));
                page["thread"]["machine"] = json!("local");
                Ok(page)
            }
            "list_captains" => {
                let captains: Vec<Value> = self
                    .captains
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        json!({
                            "name": c.name,
                            "captain_id": i,
                            "home_thread_id": self.captain_home(i),
                            "scope": self.scope_value(Some(i)),
                            "earlier_conversations": self.earlier_conversations(i),
                            "role": c.role,
                            "status": status_word(self.captain_status(i)),
                            "assigned": self.threads.iter().any(|t| t.captain_home && t.captain == Some(i)),
                        })
                    })
                    .collect();
                Ok(
                    json!({ "captains": captains, "peers":self.peers.iter().map(crate::peer::Peer::summary).collect::<Vec<_>>() }),
                )
            }
            "delegate_task" => {
                if self.threads[caller_index].delegated {
                    return Err(
                        "Delegated threads cannot delegate further. Do the work in this thread."
                            .into(),
                    );
                }
                parse_delegation_target(args)?;
                if remote_tasks::target_machine(args)?.is_some_and(|machine| machine != "local") {
                    return self.delegate_remote(caller_index, args);
                }
                self.check_task_capacity(caller)?;
                let prompt = args["prompt"].as_str().map(str::trim).unwrap_or("");
                if prompt.is_empty() {
                    return Err("prompt must describe the task.".into());
                }
                if self.threads[caller_index].machine != "local" {
                    return Err(format!(
                        "Shina delegates only on the local machine. The caller's machine {} is unavailable.",
                        self.threads[caller_index].machine
                    ));
                }
                let target = parse_delegation_target(args)?;
                let harness = target
                    .as_ref()
                    .and_then(|t| t.harness.clone())
                    .unwrap_or_else(|| self.threads[caller_index].harness.clone());
                self.validate_harness(&harness)?;
                let provider = if harness == "pi" { "cpa" } else { &harness };
                if target
                    .as_ref()
                    .and_then(|t| t.provider.as_deref())
                    .is_some_and(|p| p != provider)
                {
                    return Err(format!(
                        "target.provider must be {provider} for harness {harness}."
                    ));
                }
                let models = if harness == self.threads[caller_index].harness {
                    self.delegation_models(&self.threads[caller_index]).0
                } else {
                    self.harness_models(&harness)
                };
                let inherited = if harness == self.threads[caller_index].harness {
                    self.inherited_selection(caller_index)
                } else if harness == "pi" {
                    self.default_selection()
                } else {
                    None
                };
                let selection = match &target {
                    Some(target) => target
                        .model
                        .clone()
                        .or_else(|| inherited.as_ref().map(|s| s.model.clone()))
                        .map(|model| Selection {
                            provider: provider.into(),
                            model,
                            reasoning: target
                                .reasoning
                                .clone()
                                .or_else(|| inherited.and_then(|choice| choice.reasoning)),
                        }),
                    None => inherited,
                };
                if selection.is_none() && target.as_ref().is_some_and(|t| t.reasoning.is_some()) {
                    return Err("A reasoning target requires an advertised model so Shina can validate the level.".into());
                }
                if let Some(selection) = &selection {
                    self.validate_delegation_selection(&models, selection, target.is_some())?;
                }
                let project = self.delegation_workspace(caller_index, args)?;
                if let Some(project) = self.project(project)
                    && project.machine != "local"
                {
                    return Err("Shina delegates only into local workspaces.".into());
                }
                let assigned = self.threads[caller_index]
                    .captain
                    .filter(|_| self.threads[caller_index].captain_home);
                let captain = match args["captain"]
                    .as_str()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                {
                    Some(name) => {
                        if !assigned
                            .is_some_and(|id| self.captains[id].name.eq_ignore_ascii_case(name))
                        {
                            return Err("Crew inherits the calling captain. Open that captain's home to delegate with its profile.".into());
                        }
                        assigned
                    }
                    None => assigned,
                };
                let mut thread = Thread::new(self.next_id);
                thread.harness = harness;
                thread.project = project;
                thread.machine = self.threads[caller_index].machine.clone();
                self.next_id += 1;
                thread.title = args["title"]
                    .as_str()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .unwrap_or(prompt)
                    .chars()
                    .take(45)
                    .collect();
                thread.captain = captain;
                thread.delegated = true;
                thread.native.access = match self.threads[caller_index].native.access {
                    AccessMode::ReadOnly => AccessMode::default(),
                    access => access,
                };
                thread.parent = Some(caller);
                thread.models = models;
                if let Some(selection) = &selection {
                    thread.set_model(selection.provider.clone(), selection.model.clone());
                    if let Some(reasoning) = &selection.reasoning {
                        thread.set_thinking(reasoning.clone());
                    }
                }
                thread.arm_task();
                thread.life.mark_unread();
                let task = Task::new(&thread, prompt, selection.as_ref().map(Selection::to_value));
                let id = thread.id;
                self.threads.push(thread);
                self.tasks.push(task);
                self.sync_memory_links();
                self.start_queued_tasks();
                self.checkpoint();
                self.mcp_call(caller, "task_status", &json!({"thread_id":id}))
            }
            "task_status" => {
                let thread = &self.threads[find(self)?];
                let task = self
                    .tasks
                    .iter()
                    .find(|task| task.child_thread_id == thread.id);
                let reply = thread
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "Pi")
                    .map(|m| truncate(&m.text, 6000));
                let status = thread.task_result.unwrap_or_else(|| {
                    if !thread.status.busy() && !thread.pending_injections.is_empty() {
                        "queued"
                    } else {
                        status_word(thread.status)
                    }
                });
                Ok(json!({
                    "remote":task.and_then(|task|task.remote.as_ref()),
                    "remote_state":task.and_then(|task|task.remote.as_ref()).map(|remote|remote.state.word()),
                    "needs_you_on":task.and_then(|task|task.remote.as_ref()).filter(|remote|remote.needs_you).map(|remote|&remote.label),
                    "thread_id": thread.id,
                    "title": thread.title,
                    "project": self.task_project_value(thread),
                    "machine":task.and_then(|task|task.remote.as_ref()).map(|remote|remote.server_id.as_str()).unwrap_or(&thread.machine),
                    "parent": thread.parent,
                    "captain": captain_name(self, thread.captain),
                    "status": task.map(|task| task.state.id()).unwrap_or(status),
                    "task_id": task.map(|task| &task.id),
                    "queue_position": task.and_then(|task| self.task_queue_position(task)),
                    "selection": selection_value(thread),
                    "finished": task.map(|task| task.state.finished()).unwrap_or(thread.task_result.is_some() || (thread.completion_notified && matches!(thread.status, Status::Completed | Status::Failed))),
                    "phase": thread.phase,
                    "error": task.map(|task| task.error_excerpt.as_str()).filter(|s| !s.is_empty()).or(thread.error.as_deref()),
                    "latest_reply": task.map(|task| task.result_excerpt.as_str()).filter(|s| !s.is_empty()).map(str::to_owned).or(reply),
                }))
            }
            "task_cancel" | "task_send" => {
                let index = find(self)?;
                if self.prepare_task_control(index, caller, tool, args)? {
                    return self.mcp_call(
                        caller,
                        "task_status",
                        &json!({"thread_id":self.threads[index].id}),
                    );
                }
                if !self.owns_child(caller, index) {
                    return Err(format!(
                        "Only the parent that delegated thread {} may cancel or send to it.",
                        self.threads[index].id
                    ));
                }
                let thread = &mut self.threads[index];
                if tool == "task_cancel" {
                    thread.stop_task()?;
                } else {
                    let message = args["message"]
                        .as_str()
                        .map(str::trim)
                        .filter(|m| !m.is_empty())
                        .ok_or("message must contain a correction or follow-up brief.")?;
                    if let Some(request) = thread.request.as_ref().filter(|r| {
                        matches!(r["method"].as_str(), Some("input" | "editor" | "select"))
                    }) {
                        if request["method"] == "select"
                            && !request["options"].as_array().is_some_and(|choices| {
                                choices
                                    .iter()
                                    .any(|choice| choice.as_str() == Some(message))
                            })
                        {
                            return Err("Choose one of the crew question's listed options.".into());
                        }
                        thread.respond(json!({"value":message}));
                        if thread.request.is_some() {
                            return Err(thread
                                .error
                                .clone()
                                .unwrap_or_else(|| "Could not answer the crew question.".into()));
                        }
                        return self.mcp_call(
                            caller,
                            "task_status",
                            &json!({"thread_id":self.threads[index].id}),
                        );
                    }
                    if !thread.status.busy() {
                        thread.arm_task();
                        if thread.status == Status::Failed && thread.pi.is_some() {
                            thread.status = Status::Ready;
                            thread.error = None;
                        }
                    }
                    thread.enqueue_injection(format!(
                        "Shina: message from parent #{caller}\n\n{message}"
                    ));
                    if thread.pi.is_none() {
                        self.connect_thread(index);
                    }
                }
                self.mcp_call(
                    caller,
                    "task_status",
                    &json!({"thread_id":self.threads[index].id}),
                )
            }
            _ => Err(format!("Shina has no tool named {tool}.")),
        }
    }

    fn mcp_request(&mut self, request: crate::mcp::Request) {
        if request.tool == "delegate_task"
            && let Some(preparation) = self.prepare_remote_delegation(request.thread, &request.args)
        {
            self.defer_remote_mcp(request, preparation);
            return;
        }
        self.mcp_request_ready(request);
    }

    fn mcp_request_ready(&mut self, request: crate::mcp::Request) {
        if matches!(request.tool.as_str(), "read_thread" | "list_threads")
            && self.remote_transcript_request(&request)
        {
            return;
        }
        if matches!(request.tool.as_str(), "html_render" | "html_preview") {
            let Some(thread) = self.threads.iter().find(|t| t.id == request.thread) else {
                let _ = request
                    .reply
                    .send(Err("The calling thread no longer exists.".into()));
                return;
            };
            crate::html_render::request(
                request,
                self.data_dir.join("attachments"),
                thread.native.access == AccessMode::Full,
                self.html_lane_width,
                self.wake.clone(),
            );
            return;
        }
        if self.learned_memory_request(&request) {
            return;
        }
        let wait = if matches!(request.tool.as_str(), "delegate_task" | "task_status") {
            wait_seconds(&request.args)
        } else {
            Ok(0)
        };
        let result = wait.and_then(|seconds| {
            self.mcp_call(request.thread, &request.tool, &request.args)
                .map(|value| (seconds, value))
        });
        match result {
            Ok((seconds, value))
                if seconds > 0
                    && value["finished"] == false
                    && value["status"] != "interrupted" =>
            {
                let deadline = Instant::now() + Duration::from_secs(seconds);
                let (timer, cancel) = mpsc::channel();
                let wake = self.wake.clone();
                std::thread::spawn(move || {
                    if cancel.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        == Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        wake();
                    }
                });
                self.task_waiters.push(TaskWaiter {
                    thread: value["thread_id"].as_u64().expect("task thread id") as usize,
                    deadline,
                    reply: request.reply,
                    _timer: timer,
                });
            }
            Ok((_, value)) => {
                let _ = request.reply.send(Ok(value));
            }
            Err(error) => {
                let _ = request.reply.send(Err(error));
            }
        }
    }

    /// Called after all threads' events are drained, so simultaneous results cannot overwrite
    /// each other. Each finished run contributes one message to its parent's separate queue.
    fn poll_legacy_tasks(&mut self, now: Instant) -> bool {
        let mut changed = false;
        let mut completions = vec![];
        for thread in &mut self.threads {
            if self
                .tasks
                .iter()
                .any(|task| task.child_thread_id == thread.id)
            {
                continue;
            }
            if !thread.completion_notified
                && let Some(result) = thread.task_result
            {
                thread.completion_notified = true;
                if let Some(parent) = thread.parent {
                    let reply = thread
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.role == "Pi")
                        .map(|m| truncate(&m.text, 6000))
                        .unwrap_or_else(|| "(no reply)".into());
                    completions.push((parent, format!(
                        "Shina: task #{} finished\nTitle: {}\nStatus: {result}\nLast reply:\n{reply}\nError: {}",
                        thread.id, thread.title, thread.error.as_deref().map(|e| truncate(e, 2000)).unwrap_or_else(|| "none".into())
                    )));
                }
            }
        }
        for (parent, text) in completions {
            if let Some(index) = self.threads.iter().position(|t| t.id == parent) {
                self.threads[index].enqueue_injection(text);
                if self.threads[index].pi.is_none() {
                    self.connect_thread(index);
                }
                changed = true;
            }
        }
        let waiters = std::mem::take(&mut self.task_waiters);
        for waiter in waiters {
            // Read through the same formatter, including cancellation's idle UI status.
            let result = self.mcp_call(
                waiter.thread,
                "task_status",
                &json!({"thread_id":waiter.thread}),
            );
            if result.as_ref().is_err()
                || result.as_ref().is_ok_and(|value| value["finished"] == true)
                || result
                    .as_ref()
                    .is_ok_and(|value| value["status"] == "interrupted")
                || now >= waiter.deadline
            {
                let _ = waiter.reply.send(result);
            } else {
                self.task_waiters.push(waiter);
            }
        }
        for thread in &mut self.threads {
            changed |= thread.flush_injection();
        }
        changed |= self.poll_context_imports();
        changed
    }

    pub(crate) fn machine_identity(&self, machine: &str) -> String {
        if machine == "local" {
            self.machines
                .views
                .get("local")
                .and_then(|v| v["serverId"].as_str())
                .map(str::to_owned)
                .or_else(|| {
                    std::fs::read_to_string(self.data_dir.join("server-id"))
                        .ok()
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_else(|| "local".into())
        } else {
            machine.to_owned()
        }
    }

    pub(crate) fn linked_environment_ids_for_target(&self) -> std::collections::HashSet<String> {
        let mut set: std::collections::HashSet<String> =
            self.peers.iter().map(|p| p.server_id.clone()).collect();
        if let Some(client) = &self.client {
            for id in client.linked.keys() {
                set.insert(id.clone());
            }
        }
        set.insert(self.machine_identity(&self.machines.selected));
        set.insert(self.machines.selected.clone());
        set
    }

    pub fn has_pending_context_imports(&self, machine: &str, thread_id: usize) -> bool {
        self.pending_context_imports
            .iter()
            .any(|(m, tid, _)| m == machine && *tid == thread_id)
    }

    pub fn copy_composer_selection(&mut self, cut: bool) -> Option<rune::ClipboardContent> {
        let t = self.current();
        if t.draft.anchor == t.draft.focus || t.draft.is_secure() {
            return None;
        }
        let (start, end) = t.draft.range();
        let selected_markdown = crate::composer_rich::markdown_range(&t.draft, start..end);
        let selected_html = t.draft.selected_html();
        let refs = crate::composer_context::collect_composer_context_references(&selected_markdown);
        let mut wanted: std::collections::HashSet<String> =
            refs.into_iter().map(|r| r.context_id).collect();
        for rec in &t.draft_context {
            if wanted.contains(&rec.context_id)
                && let crate::composer_context::ComposerContextData::PreviewAnnotation {
                    screenshot_context_id: Some(shot_id),
                    ..
                } = &rec.data
            {
                wanted.insert(shot_id.clone());
            }
        }
        let mut records = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut copied_files = std::collections::HashMap::new();
        for rec in &t.draft_context {
            if wanted.contains(&rec.context_id) && seen.insert(rec.context_id.clone()) {
                if let crate::composer_context::ComposerContextData::Image { attachment_id, .. }
                | crate::composer_context::ComposerContextData::File { attachment_id, .. } =
                    &rec.data
                    && let Some(att) = t.draft_images.iter().find(|a| a.id == *attachment_id)
                {
                    copied_files.insert(attachment_id.clone(), att.clone());
                }
                records.push(rec.clone());
            }
        }
        for att in &t.draft_images {
            let kind = if att.is_image() { "image" } else { "file" };
            let cid = crate::composer_context::to_kind_scoped_composer_context_id(kind, &att.id);
            if wanted.contains(&cid) && seen.insert(cid) {
                copied_files.insert(att.id.clone(), att.clone());
                records.push(crate::composer_context::attachment_context_record(att));
            }
        }
        let staged = if self.current().answer_intake {
            (|| -> Result<(), String> {
                let files = std::mem::take(&mut copied_files);
                for (id, file) in files {
                    let mut single = std::collections::HashMap::from([(id, file)]);
                    attachments::stage_clipboard_files(&mut records, &mut single)?;
                    copied_files.extend(single);
                }
                Ok(())
            })()
        } else {
            attachments::stage_clipboard_files(&mut records, &mut copied_files)
        };
        if let Err(error) = staged {
            self.current_mut().attachment_error = Some(error);
            return None;
        }
        let t = self.current();
        let source_env = self.machine_identity(&self.machines.selected);
        let content = if !records.is_empty() {
            let fragment = crate::composer_context::ComposerContextClipboardFragment {
                version: 1,
                source: crate::composer_context::ClipboardSource {
                    environment_id: source_env.clone(),
                    thread_id: Some(t.id.to_string()),
                    message_id: None,
                },
                records,
            };
            if let Some(encoded) =
                crate::composer_context::encode_composer_context_fragment(&fragment)
            {
                crate::composer_context::remember_copied_composer_attachments(
                    &source_env,
                    copied_files,
                );
                let html = crate::composer_context::encode_composer_context_clipboard_html(
                    &selected_markdown,
                    &encoded,
                    (!selected_html.is_empty()).then_some(selected_html.as_str()),
                );
                rune::ClipboardContent::new(selected_markdown).with_html(html)
            } else if !selected_html.is_empty() {
                rune::ClipboardContent::new(selected_markdown).with_html(selected_html)
            } else {
                rune::ClipboardContent::new(selected_markdown)
            }
        } else if !selected_html.is_empty() {
            rune::ClipboardContent::new(selected_markdown).with_html(selected_html)
        } else {
            rune::ClipboardContent::new(selected_markdown)
        };
        if cut {
            let t = self.current_mut();
            t.draft.undo_group(|e| {
                e.set_selection(start, end);
                e.insert("");
            });
            t.reconcile_draft_context();
            t.typing_guard.on_type(std::time::Instant::now());
            t.update_submission_error();
            self.ensure_composer_commands();
            self.checkpoint();
        }
        Some(content)
    }

    pub fn paste_composer_content(
        &mut self,
        content: &rune::ClipboardContent,
        bypass_fold: bool,
    ) -> bool {
        if !self.current().answer_intake
            && let Some(scope) = self.current().question_scope()
        {
            if !self.question_attachments_allowed(self.active) {
                self.current_mut().attachment_error =
                    Some("This question cannot accept attachments.".into());
                return true;
            }
            return self
                .with_question_draft(self.active, scope, |s| {
                    s.paste_composer_content(content, bypass_fold)
                })
                .unwrap_or(true);
        }
        let has_image = content.image().is_some();
        if let Some(image) = content.image() {
            self.attach_image_with_options("Pasted image.png", image.png(), bypass_fold);
        }
        if !content.files().is_empty() {
            self.attach_paths(content.files());
        }
        if !content.text().is_empty() && !has_image && content.files().is_empty() {
            if self.current().answer_intake
                && self
                    .current()
                    .native
                    .questionnaire
                    .as_ref()
                    .is_some_and(|f| f.questions[f.index].initial_answer.is_some())
            {
                self.current_mut().draft.insert(content.text());
                return true;
            }
            if !bypass_fold && self.fold_paste(content.text()) {
                return true;
            }
            if self.current().draft.active_block() == rune::Block::Code {
                crate::composer_paste::insert(
                    self,
                    &content.text().replace("\r\n", "\n").replace('\r', "\n"),
                );
                return true;
            }
            let target_env = self.machine_identity(&self.machines.selected);
            let linked_envs = self.linked_environment_ids_for_target();
            let (rewritten_text, outcome) = crate::composer_context::import_pasted_composer_text(
                content.text(),
                content.html(),
                &target_env,
                &linked_envs,
                &self.current().draft_context,
                &self.current().draft_images,
            );
            if self.current().answer_intake
                && outcome
                    .imported_records
                    .iter()
                    .any(|r| !matches!(r.kind(), "thread" | "file" | "image"))
            {
                self.current_mut().attachment_error =
                    Some("Answer or dismiss the question first, then add it to chat.".into());
                return true;
            }
            for rec in outcome.imported_records {
                let t = self.current_mut();
                if let Some(existing) = t
                    .draft_context
                    .iter_mut()
                    .find(|r| r.context_id == rec.context_id)
                {
                    *existing = rec;
                } else {
                    t.draft_context.push(rec);
                }
            }
            crate::composer_paste::insert(self, &rewritten_text);
            let target_machine = self.machines.selected.clone();
            let target_thread_id = self.current().id;
            for pending in outcome.pending_attachments {
                self.resolve_pending_attachment_import(&target_machine, target_thread_id, pending);
            }
            return true;
        }
        (has_image || !content.files().is_empty()) && content.text().is_empty()
    }

    fn resolve_pending_attachment_import(
        &mut self,
        target_machine: &str,
        target_thread_id: usize,
        pending: crate::composer_context::PendingAttachmentImport,
    ) {
        if self.current().answer_intake
            && self.current().question_reserved_count() >= attachments::MAX_ATTACHMENTS
        {
            self.current_mut().attachment_error = Some(format!(
                "You can attach up to {} files per message.",
                attachments::MAX_ATTACHMENTS
            ));
            return;
        }
        let (attachment_id, name, mime_type, kind) = match &pending.record.data {
            crate::composer_context::ComposerContextData::Image {
                attachment_id,
                name,
                mime_type,
                ..
            } => (
                attachment_id.clone(),
                name.clone(),
                mime_type.clone(),
                "image",
            ),
            crate::composer_context::ComposerContextData::File {
                attachment_id,
                name,
                mime_type,
                ..
            } => (
                attachment_id.clone(),
                name.clone(),
                mime_type.clone(),
                "file",
            ),
            _ => return,
        };
        let fail_msg = |reason: &str| {
            format!(
                "Couldn't bring {name} into this message. {reason} Remove the chip or attach the file again."
            )
        };
        let reserved = self
            .pending_context_imports
            .iter()
            .filter(|(machine, id, _)| machine == target_machine && *id == target_thread_id)
            .count();
        let attached = if target_machine == self.machines.selected {
            self.threads.iter().find(|t| t.id == target_thread_id)
        } else {
            self.machines
                .threads
                .get(target_machine)
                .and_then(|threads| threads.iter().find(|t| t.id == target_thread_id))
        }
        .map_or(0, |thread| thread.draft_images.len());
        if attached + reserved >= attachments::MAX_ATTACHMENTS {
            self.current_mut().attachment_error =
                Some(fail_msg("The attachment limit has been reached."));
            return;
        }
        let local_bytes: Option<Vec<u8>> = if let Some(copied) =
            crate::composer_context::copied_composer_attachment(
                &pending.source_environment_id,
                &attachment_id,
            ) {
            if copied.is_image() {
                attachments::read(&copied).ok()
            } else {
                attachments::transfer_file_bytes(&copied).ok()
            }
        } else if (pending.source_environment_id == self.machine_identity("local")
            || pending.source_environment_id == "local")
            && (kind == "image" && attachments::safe_id(&attachment_id)
                || kind == "file" && attachments::safe_file_id(&attachment_id))
        {
            attachments::directory()
                .map(|d| d.join(&attachment_id))
                .and_then(|path| {
                    use std::io::Read;
                    let maximum = if kind == "file" {
                        attachments::MAX_FILE_BYTES
                    } else {
                        attachments::MAX_IMAGE_BYTES
                    };
                    let mut bytes = Vec::new();
                    std::fs::File::open(path)
                        .ok()?
                        .take(maximum + 1)
                        .read_to_end(&mut bytes)
                        .ok()?;
                    (bytes.len() as u64 <= maximum).then_some(bytes)
                })
        } else {
            None
        };
        if let Some(bytes) = local_bytes {
            self.apply_imported_attachment_bytes(
                target_machine,
                target_thread_id,
                &pending.new_context_id,
                &name,
                &mime_type,
                kind,
                Ok(bytes),
            );
            return;
        }
        if pending.source_environment_id == self.machine_identity("local")
            || pending.source_environment_id == "local"
        {
            self.current_mut().attachment_error =
                Some(fail_msg("The original attachment is no longer available."));
            return;
        }
        let link = self
            .client
            .as_ref()
            .and_then(|c| c.linked.get(&pending.source_environment_id))
            .and_then(|c| c.remote.clone());
        let Some(link) = link else {
            self.current_mut().attachment_error =
                Some(fail_msg("The environment it came from is not connected."));
            return;
        };
        if !self.machine_connected(&pending.source_environment_id) {
            self.current_mut().attachment_error = Some(fail_msg(
                "The environment it came from is not reachable right now.",
            ));
            return;
        }
        if kind == "file"
            && !self
                .client
                .as_ref()
                .and_then(|c| c.linked.get(&pending.source_environment_id))
                .is_some_and(|c| {
                    c.info.lock().expect("connection info")["capabilities"]
                        .as_array()
                        .is_some_and(|caps| {
                            caps.iter()
                                .any(|cap| cap == crate::protocol::FILE_ATTACHMENTS)
                        })
                })
        {
            self.current_mut().attachment_error =
                Some(fail_msg(crate::protocol::FILE_ATTACHMENTS_UPDATE_REQUIRED));
            return;
        }
        if !(kind == "image" && attachments::safe_id(&attachment_id)
            || kind == "file" && attachments::safe_file_id(&attachment_id))
        {
            self.current_mut().attachment_error =
                Some(fail_msg("The original attachment is no longer available."));
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = self.wake.clone();
        let target_machine = target_machine.to_owned();
        let new_context_id = pending.new_context_id.clone();
        let question_scope = self
            .threads
            .iter()
            .find(|t| t.id == target_thread_id)
            .and_then(|t| t.answer_scope.clone());
        if let Some(scope) = &question_scope
            && let Some(t) = self.threads.iter_mut().find(|t| t.id == target_thread_id)
        {
            t.question_drafts.entry(scope.clone()).or_default().pending += 1;
        }
        self.pending_context_imports.push((
            target_machine.clone(),
            target_thread_id,
            new_context_id.clone(),
        ));
        self.context_import_rxs.push(rx);
        std::thread::spawn(move || {
            let maximum = if kind == "file" {
                attachments::MAX_FILE_BYTES
            } else {
                attachments::MAX_IMAGE_BYTES
            };
            let result = fetch_linked_blob(&link, &attachment_id, maximum);
            let _ = tx.send(ContextAssetDownload {
                machine: target_machine,
                thread_id: target_thread_id,
                new_context_id,
                name,
                mime_type,
                kind,
                result,
                question_scope,
            });
            wake();
        });
    }

    pub fn poll_context_imports(&mut self) -> bool {
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.context_import_rxs.len());
        let rxs = std::mem::take(&mut self.context_import_rxs);
        for rx in rxs {
            match rx.try_recv() {
                Ok(download) => {
                    changed = true;
                    self.pending_context_imports.retain(|(m, tid, cid)| {
                        !(m == &download.machine
                            && *tid == download.thread_id
                            && cid == &download.new_context_id)
                    });
                    if let Some(scope) = download.question_scope {
                        if download.machine == self.machines.selected
                            && let Some(index) =
                                self.threads.iter().position(|t| t.id == download.thread_id)
                        {
                            if let Some(draft) = self.threads[index].question_drafts.get_mut(&scope)
                            {
                                draft.pending = draft.pending.saturating_sub(1);
                            }
                            self.with_question_draft(index, scope, |s| {
                                s.apply_imported_attachment_bytes(
                                    &download.machine,
                                    download.thread_id,
                                    &download.new_context_id,
                                    &download.name,
                                    &download.mime_type,
                                    download.kind,
                                    download.result,
                                )
                            });
                        } else if let Some(t) = self
                            .machines
                            .threads
                            .get_mut(&download.machine)
                            .and_then(|ts| ts.iter_mut().find(|t| t.id == download.thread_id))
                            && let Some(draft) = t.question_drafts.get_mut(&scope)
                        {
                            draft.pending = draft.pending.saturating_sub(1);
                        }
                    } else {
                        self.apply_imported_attachment_bytes(
                            &download.machine,
                            download.thread_id,
                            &download.new_context_id,
                            &download.name,
                            &download.mime_type,
                            download.kind,
                            download.result,
                        );
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => changed = true,
            }
        }
        self.context_import_rxs = remaining;
        changed
    }

    fn apply_imported_attachment_bytes(
        &mut self,
        target_machine: &str,
        target_thread_id: usize,
        new_context_id: &str,
        name: &str,
        mime_type: &str,
        kind: &str,
        result: Result<Vec<u8>, String>,
    ) {
        let threads = if target_machine == self.machines.selected {
            Some(&mut self.threads)
        } else {
            self.machines.threads.get_mut(target_machine)
        };
        let Some(t) =
            threads.and_then(|threads| threads.iter_mut().find(|t| t.id == target_thread_id))
        else {
            return;
        };
        if !crate::composer_tokens::collect_inline_context_ids_from_edit(&t.draft)
            .contains(new_context_id)
        {
            return;
        }
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(reason) => {
                t.attachment_error = Some(format!(
                    "Couldn't bring {name} into this message. {reason} Remove the chip or attach the file again."
                ));
                return;
            }
        };
        if (if t.answer_intake {
            t.question_reserved_count()
        } else {
            t.draft_images.len()
        }) >= attachments::MAX_ATTACHMENTS
        {
            t.attachment_error = Some(format!(
                "Couldn't bring {name} into this message. The attachment limit has been reached. Remove the chip or attach the file again."
            ));
            return;
        }
        let attachment = if kind == "image" {
            match attachments::attach_bytes(name, bytes) {
                Ok(att) => att,
                Err(error) => {
                    t.attachment_error = Some(error);
                    return;
                }
            }
        } else {
            let dir = match attachments::directory() {
                Some(d) => d,
                None => {
                    t.attachment_error = Some("The attachment directory is unavailable.".into());
                    return;
                }
            };
            match attachments::store_uploaded_file(&dir, name, mime_type, &bytes) {
                Ok(att) => att,
                Err(error) => {
                    t.attachment_error = Some(error);
                    return;
                }
            }
        };
        if !t.draft_images.iter().any(|a| a.id == attachment.id) {
            if let Err(error) = t.add_attachment(attachment.clone()) {
                t.attachment_error = Some(format!(
                    "Couldn't bring {name} into this message. {error} Remove the chip or attach the file again."
                ));
                return;
            }
        }
        let mut rec = crate::composer_context::attachment_context_record(&attachment);
        rec.context_id = new_context_id.to_owned();
        if let Some(existing) = t
            .draft_context
            .iter_mut()
            .find(|r| r.context_id == rec.context_id)
        {
            *existing = rec;
        } else {
            t.draft_context.push(rec);
        }
    }
    fn attachment_guard(&self, index: usize) -> Option<String> {
        if self.demo {
            Some("Open a real workspace before attaching files.".into())
        } else if self.threads[index].archived() {
            Some("Restore this thread before attaching files.".into())
        } else if !self.threads[index].answer_intake
            && !self.threads[index].request_held_by_typing_guard()
            && self.threads[index]
                .request
                .as_ref()
                .is_some_and(|r| r["method"] != "confirm")
        {
            Some("This question cannot accept attachments.".into())
        } else if self.threads[index].status.busy() {
            if self.threads[index].native.operating() {
                Some("Wait for the current operation before attaching files.".into())
            } else {
                None
            }
        } else if self.threads[index].configuring {
            Some("Wait for the model change to finish before attaching files.".into())
        } else {
            None
        }
    }
    /// Attach one or more picked or dropped files. A failure leaves the text draft untouched.
    pub fn attach_paths(&mut self, paths: &[PathBuf]) {
        self.attach_paths_to(self.current().id, paths);
    }
    pub fn fold_paste(&mut self, text: &str) -> bool {
        if !crate::composer_paste::should_fold(text)
            || self.current().machine != "local" && self.client.is_none()
            || self.client.is_some()
                && self.machines.selected != "local"
                && !self
                    .client
                    .as_ref()
                    .and_then(|c| c.linked.get(&self.machines.selected))
                    .is_some_and(|c| {
                        c.info.lock().expect("connection info")["capabilities"]
                            .as_array()
                            .is_some_and(|caps| {
                                caps.iter()
                                    .any(|cap| cap == crate::protocol::FILE_ATTACHMENTS)
                            })
                    })
            || self.attachment_guard(self.active).is_some()
            || (if self.current().answer_intake {
                self.current().question_reserved_count()
            } else {
                self.current().draft_images.len()
            }) >= attachments::MAX_ATTACHMENTS
        {
            return false;
        }
        match crate::composer_paste::stage(text, &self.current().draft_images) {
            Ok(attachment) => {
                if let Some(notice) = &mut self.undo {
                    notice.owns_text_key = false;
                }
                let t = self.current_mut();
                t.draft_images.push(attachment.clone());
                t.insert_attachment_chips(&[attachment], false);
                self.checkpoint();
                true
            }
            Err(error) => {
                self.current_mut().attachment_error = Some(error);
                false
            }
        }
    }
    pub fn attach_paths_to(&mut self, thread: usize, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let Some(index) = self.threads.iter().position(|t| t.id == thread) else {
            return;
        };
        if !self.threads[index].answer_intake
            && let Some(scope) = self.threads[index].question_scope()
        {
            if !self.question_attachments_allowed(index) {
                self.threads[index].attachment_error =
                    Some("This question cannot accept attachments.".into());
                return;
            }
            self.with_question_draft(index, scope, |s| s.attach_paths_to(thread, paths));
            return;
        }
        if let Some(error) = self.attachment_guard(index) {
            self.threads[index].attachment_error = Some(error);
            return;
        }
        let mut errors = Vec::new();
        let mut attached = Vec::new();
        for path in paths {
            if (if self.threads[index].answer_intake {
                self.threads[index].question_reserved_count()
            } else {
                self.threads[index].draft_images.len()
            }) >= attachments::MAX_ATTACHMENTS
            {
                errors.push(format!(
                    "A message can carry up to {} attachments. The remaining files were not attached.",
                    attachments::MAX_ATTACHMENTS
                ));
                break;
            }
            match attachments::attach_file(path) {
                Ok(image) => match self.threads[index].add_attachment(image.clone()) {
                    Ok(()) => attached.push(image),
                    Err(error) => errors.push(error),
                },
                Err(error) => errors.push(error),
            }
        }
        self.threads[index].attachment_error = if errors.is_empty() {
            None
        } else {
            Some(errors.join(" "))
        };
        if !attached.is_empty() {
            self.threads[index].insert_attachment_chips(&attached, false);
            if index == self.active {
                self.restore_composer_focus(
                    crate::ui::composer_ui::FocusReason::AttachmentInserted,
                );
            }
            self.checkpoint();
        }
    }
    /// Attach a pasted PNG image, inserting an inline chip when contextual.
    #[cfg(test)]
    pub fn attach_image(&mut self, name: &str, bytes: &[u8]) {
        self.attach_image_with_options(name, bytes, false);
    }
    pub fn attach_image_with_options(&mut self, name: &str, bytes: &[u8], skip_inline_chip: bool) {
        if !self.current().answer_intake
            && let Some(scope) = self.current().question_scope()
        {
            if !self.question_attachments_allowed(self.active) {
                self.current_mut().attachment_error =
                    Some("This question cannot accept attachments.".into());
                return;
            }
            self.with_question_draft(self.active, scope, |s| {
                s.attach_image_with_options(name, bytes, skip_inline_chip)
            });
            return;
        }
        if let Some(error) = self.attachment_guard(self.active) {
            self.current_mut().attachment_error = Some(error);
            return;
        }
        if bytes.len() as u64 > attachments::MAX_IMAGE_BYTES {
            self.current_mut().attachment_error = Some(format!(
                "Choose an image that is {} MB or smaller.",
                attachments::MAX_IMAGE_BYTES / (1024 * 1024)
            ));
            return;
        }
        match attachments::attach_bytes(name, bytes.to_vec()) {
            Ok(image) => {
                let result = self.current_mut().add_attachment(image.clone());
                self.current_mut().attachment_error = result.as_ref().err().cloned();
                if result.is_ok() {
                    self.current_mut()
                        .insert_attachment_chips(&[image], skip_inline_chip);
                    self.restore_composer_focus(
                        crate::ui::composer_ui::FocusReason::AttachmentInserted,
                    );
                }
            }
            Err(error) => self.current_mut().attachment_error = Some(error),
        }
        self.checkpoint();
    }
    pub fn request_remove_attachment(&mut self, id: &str) {
        if !self.current().answer_intake
            && let Some(scope) = self.current().question_scope()
        {
            self.answer_remove_scope = Some(scope.clone());
            self.with_question_draft(self.active, scope, |s| s.request_remove_attachment(id));
            return;
        }
        let t = self.current();
        if let Some(attachment) = t.draft_images.iter().find(|a| a.id == id && a.is_image()) {
            let ctx_id = crate::composer_context::to_kind_scoped_composer_context_id(
                "image",
                &attachment.id,
            );
            let referenced = crate::composer_tokens::collect_inline_context_ids_from_edit(&t.draft)
                .contains(&ctx_id);
            if referenced {
                self.confirm_remove_attachment = Some((t.id, id.to_owned()));
                return;
            }
        }
        self.remove_attachment(id);
    }
    pub fn confirm_remove_attachment_dialog(&mut self) {
        if let Some(scope) = self.answer_remove_scope.clone()
            && !self.current().answer_intake
        {
            if self.current().question_scope().as_ref() != Some(&scope) {
                self.confirm_remove_attachment = None;
                self.answer_remove_scope = None;
                return;
            }
            self.with_question_draft(self.active, scope, |s| s.confirm_remove_attachment_dialog());
            self.answer_remove_scope = None;
            return;
        }
        let Some((thread_id, id)) = self.confirm_remove_attachment.take() else {
            return;
        };
        let Some(t) = self.threads.iter_mut().find(|t| t.id == thread_id) else {
            return;
        };
        let ctx_id = crate::composer_context::to_kind_scoped_composer_context_id("image", &id);
        let pill_href = crate::composer_context::format_composer_context_href("image", &ctx_id);
        let ranges: Vec<_> = t
            .draft
            .pills()
            .filter(|(_, pill)| *pill == pill_href)
            .map(|(range, _)| range)
            .collect();
        if !ranges.is_empty() {
            t.draft.undo_group(|e| {
                for range in ranges.into_iter().rev() {
                    let mut start = range.start;
                    let mut end = range.end;
                    if e.text[end..].starts_with(' ') {
                        end += 1;
                    } else if e.text[..start].ends_with(' ') {
                        start -= 1;
                    }
                    e.set_selection(start, end);
                    e.insert("");
                    if start >= e.text.len() {
                        let trimmed_len = e.text.trim_end().len();
                        if trimmed_len < e.text.len() {
                            e.set_selection(trimmed_len, e.text.len());
                            e.insert("");
                        }
                    }
                }
            });
        }
        t.draft_context.retain(|r| r.context_id != ctx_id);
        t.retained_context.records.remove(&ctx_id);
        t.remove_attachment(&id);
        self.checkpoint();
    }
    pub fn remove_attachment(&mut self, id: &str) {
        if !self.current().answer_intake
            && let Some(scope) = self.current().question_scope()
        {
            self.with_question_draft(self.active, scope, |s| s.remove_attachment(id));
            return;
        }
        self.current_mut().remove_attachment(id);
        self.checkpoint();
    }
    pub(crate) fn push_toast(
        &mut self,
        kind: ToastKind,
        title: impl Into<String>,
        description: Option<String>,
        action: Option<(String, ToastAction)>,
        timeout_ms: Option<u64>,
    ) {
        let title = title.into();
        if title == "Not connected: message not sent" {
            self.toasts.retain(|t| t.title != title);
        }
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        let (action_label, action) = match action {
            Some((label, action)) => (Some(label), Some(action)),
            None => (None, None),
        };
        let expires_at = timeout_ms.map(|ms| lifecycle::now().saturating_add(ms));
        self.toasts.push(ToastNotice {
            id,
            kind,
            title,
            description,
            action_label,
            action,
            expires_at,
        });
        while self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    pub fn activate_toast(&mut self, id: u64) {
        let Some(pos) = self.toasts.iter().position(|t| t.id == id) else {
            return;
        };
        let toast = self.toasts.remove(pos);
        let Some(action) = toast.action else {
            return;
        };
        match action {
            ToastAction::OpenThread { server, thread_id } => {
                if self.client.is_some() {
                    if server != self.machines.selected {
                        self.select_machine(&server);
                    }
                    if self.machines.selected == server
                        && let Some(index) = self.threads.iter().position(|t| t.id == thread_id)
                    {
                        self.select_thread(index);
                    }
                } else if let Some(index) = self.threads.iter().position(|t| t.id == thread_id) {
                    self.select_thread(index);
                }
            }
        }
    }

    pub(crate) fn expire_toasts(&mut self) -> bool {
        let at = lifecycle::now();
        let before = self.toasts.len();
        self.toasts
            .retain(|toast| toast.expires_at.is_none_or(|exp| exp > at));
        self.toasts.len() != before
    }

    pub(crate) fn thread_workspace_path(&self, thread_id: usize) -> Option<String> {
        let thread = self.threads.iter().find(|t| t.id == thread_id)?;
        thread.project?;
        if self.client.is_some() && thread.machine != "local" {
            return self.client_views["directories"][thread.id.to_string()]
                .as_str()
                .map(str::to_owned);
        }
        self.project(thread.project)
            .filter(|project| project.machine == thread.machine)
            .map(|project| project.path.to_string_lossy().into_owned())
    }

    pub(crate) fn can_write_project_file(&self, thread: &Thread) -> bool {
        !self.demo
            && self.thread_workspace_path(thread.id).is_some()
            && self.client.as_ref().is_none_or(|client| {
                client.supports_capability(&thread.machine, crate::protocol::PROJECT_WRITE_FILE)
            })
    }

    pub(crate) fn is_plan_copied(&self, card_id: &str) -> bool {
        self.plan_copied
            .as_ref()
            .is_some_and(|(id, expires)| id == card_id && *expires > lifecycle::now())
    }

    pub(crate) fn mark_plan_copied(&mut self, card_id: String, tx: &mut Tx) {
        self.plan_copied = Some((card_id, lifecycle::now().saturating_add(2_000)));
        tx.wake_after(Duration::from_secs(2));
    }

    pub(crate) fn toggle_plan_expanded(&mut self, card_id: &str) {
        if !self.plan_expanded.remove(card_id) {
            self.plan_expanded.insert(card_id.to_owned());
        }
    }

    pub(crate) fn open_plan_save_dialog(&mut self, thread_id: usize, plan_markdown: &str) {
        let Some(thread) = self.threads.iter().find(|t| t.id == thread_id) else {
            return;
        };
        if !self.can_write_project_file(thread) {
            return;
        }
        let Some(cwd) = self.thread_workspace_path(thread_id) else {
            return;
        };
        let machine = thread.machine.clone();
        let initial_path = self.plan_save_last_path.clone().unwrap_or_else(|| {
            crate::plan_export::build_proposed_plan_markdown_filename(plan_markdown)
        });
        let mut path_edit = TextEdit::default();
        path_edit.replace_text(&initial_path);
        path_edit.focused = true;
        self.plan_save_dialog = Some(PlanSaveDialog {
            machine,
            cwd,
            contents: crate::plan_export::normalize_plan_markdown_for_export(plan_markdown),
            path_edit,
            saving: false,
            request_id: None,
        });
    }

    pub(crate) fn close_plan_save_dialog(&mut self) {
        if !self.plan_save_dialog.as_ref().is_some_and(|d| d.saving) {
            self.plan_save_dialog = None;
        }
    }

    pub(crate) fn submit_plan_save(&mut self) {
        let Some(dialog) = self.plan_save_dialog.as_mut() else {
            return;
        };
        if dialog.saving {
            return;
        }
        let raw_path = dialog.path_edit.text.clone();
        let relative_path = raw_path.trim().to_owned();
        if relative_path.is_empty() {
            self.push_toast(
                ToastKind::Warning,
                "Enter a workspace path",
                None,
                None,
                Some(5_000),
            );
            return;
        }
        self.plan_save_last_path = Some(raw_path);
        self.next_plan_save_request_id += 1;
        let req_id = self.next_plan_save_request_id;
        dialog.saving = true;
        dialog.request_id = Some(req_id);
        let machine = dialog.machine.clone();
        let cwd = dialog.cwd.clone();
        let contents = dialog.contents.clone();
        if let Some(client) = &self.client {
            let target = if machine == "local" {
                Some(client)
            } else {
                client.linked.get(&machine)
            };
            let Some(target) = target else {
                self.apply_plan_save_result(req_id, Err("Machine unavailable.".into()));
                return;
            };
            if !target.has_capability(crate::protocol::PROJECT_WRITE_FILE) {
                self.apply_plan_save_result(
                    req_id,
                    Err("This server needs an update to save plans to the workspace.".into()),
                );
                return;
            }
            target.call_rpc(
                format!("write-plan:{req_id}"),
                "project.writeFile",
                json!({
                    "cwd": cwd,
                    "relativePath": relative_path,
                    "contents": contents,
                }),
            );
        } else if machine != "local" {
            self.apply_plan_save_result(req_id, Err("Machine unavailable.".into()));
        } else {
            let result =
                crate::projects::write_project_file(&crate::protocol::ProjectWriteFileInput {
                    cwd,
                    relative_path,
                    contents,
                });
            self.apply_plan_save_result(req_id, result);
        }
    }

    pub(crate) fn apply_plan_save_result(
        &mut self,
        req_id: u64,
        result: Result<crate::protocol::ProjectWriteFileResult, String>,
    ) {
        let Some(dialog) = self.plan_save_dialog.as_mut() else {
            return;
        };
        if dialog.request_id != Some(req_id) {
            return;
        }
        match result {
            Ok(written) => {
                self.plan_save_dialog = None;
                self.push_toast(
                    ToastKind::Success,
                    "Plan saved to workspace",
                    Some(written.relative_path),
                    None,
                    Some(5_000),
                );
            }
            Err(error) => {
                dialog.saving = false;
                dialog.request_id = None;
                self.push_toast(
                    ToastKind::Error,
                    "Could not save plan",
                    Some(error),
                    None,
                    Some(5_000),
                );
            }
        }
    }

    pub(crate) fn notify_background_started(&mut self, server: &str, thread_id: usize) {
        self.push_toast(
            ToastKind::Success,
            "Started in background",
            None,
            Some((
                "Open".into(),
                ToastAction::OpenThread {
                    server: server.to_owned(),
                    thread_id,
                },
            )),
            Some(5_000),
        );
    }

    pub(crate) fn notify_background_failure(
        &mut self,
        server: &str,
        thread_id: usize,
        error: String,
    ) {
        self.toasts.retain(|toast| {
            !(toast.title == "Started in background"
                && toast.action.as_ref().is_some_and(|action| {
                    matches!(
                        action,
                        ToastAction::OpenThread {
                            server: s,
                            thread_id: tid
                        } if s == server && *tid == thread_id
                    )
                }))
        });
        self.push_toast(
            ToastKind::Error,
            "Background task failed",
            Some(error),
            Some((
                "Open draft".into(),
                ToastAction::OpenThread {
                    server: server.to_owned(),
                    thread_id,
                },
            )),
            Some(5_000),
        );
    }

    pub fn send(&mut self) {
        let _ = self.dispatch_send();
    }

    /// Ported from KSG Code's `useComposerSend` (`onSend` with `submissionIntent: "background"`):
    /// on a draft thread, starts in the background and opens a fresh composer; on an existing
    /// thread, dispatches the turn and opens a new draft once accepted.
    pub fn send_background(&mut self) {
        self.send_background_intent(false);
    }

    fn send_background_intent(&mut self, approval_redirect: bool) {
        if !self
            .current()
            .draft_model_state
            .additional_models
            .is_empty()
            && !self.demo
            && !self.ownership_denied
            && !self.current().archived()
        {
            let _ = self.dispatch_send_intent(true, approval_redirect);
            return;
        }
        let source_index = self.active;
        let source_id = self.current().id;
        let was_draft = self.current().is_draft();
        let project = self.current().project;
        let seed = self.composer_seed();
        let server = self.machines.selected.clone();

        if !self.dispatch_send_intent(true, approval_redirect) {
            return;
        }
        if self.client.is_some() {
            if was_draft {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == source_id) {
                    thread.background_send = Some(server);
                }
                self.new_client_draft_with_seed(project, &seed);
                if !self.headless {
                    self.restore_composer_focus(crate::ui::composer_ui::FocusReason::ThreadOpened);
                }
                self.checkpoint();
            } else if let Some((command_id, _)) = self
                .threads
                .iter()
                .find(|t| t.id == source_id)
                .and_then(|t| t.remote_send.as_ref())
            {
                let command_id = *command_id;
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == source_id) {
                    thread.pending_send_and_new_thread = Some((command_id, server));
                }
            }
            return;
        }

        if was_draft {
            self.create_thread_in(project, &[source_id], &seed);
            if !self.headless {
                self.restore_composer_focus(crate::ui::composer_ui::FocusReason::ThreadOpened);
            }
            let failed = self
                .threads
                .iter()
                .find(|t| t.id == source_id)
                .is_some_and(|t| t.status == Status::Failed);
            if failed {
                let error = self
                    .threads
                    .iter()
                    .find(|t| t.id == source_id)
                    .and_then(|t| t.error.clone())
                    .unwrap_or_else(|| "Failed to send message.".into());
                self.notify_background_failure("local", source_id, error);
            } else {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == source_id) {
                    thread.background_start_pending =
                        thread.pending_prompt.is_some() || thread.queued_prompt.is_some();
                }
                self.notify_background_started("local", source_id);
            }
            self.checkpoint();
        } else {
            let failed = self
                .threads
                .get(source_index)
                .is_some_and(|t| t.id == source_id && t.status == Status::Failed);
            if !failed {
                self.create_thread_in(project, &[source_id], &seed);
                if !self.headless {
                    self.restore_composer_focus(crate::ui::composer_ui::FocusReason::ThreadOpened);
                }
            }
            self.checkpoint();
        }
    }

    pub(crate) fn fresh_session_available(&self, thread: &Thread) -> bool {
        !self.demo
            && !thread.messages.is_empty()
            && crate::harness::supports_fresh_context(&thread.harness)
            && self.client.as_ref().is_none_or(|client| {
                client.supports_capability(&thread.machine, crate::protocol::FRESH_CONTEXT)
            })
    }

    pub(crate) fn fresh_session_command_available(&self, thread: &Thread) -> bool {
        self.fresh_session_available(thread)
            && thread.request.is_none()
            && thread.native.questionnaire.is_none()
    }

    pub fn usage_limits_command_available(&self, thread: &Thread) -> bool {
        !self.demo
            && crate::usage_limits::has_usage_limits(&thread.harness)
            && self.client.as_ref().is_none_or(|client| {
                client.supports_capability(&thread.machine, crate::protocol::USAGE_LIMITS)
            })
    }

    pub fn slash_matches(&self) -> Vec<pi_session::SlashCommand> {
        self.current().slash_matches_with_capabilities(
            self.fresh_session_command_available(self.current()),
            self.usage_limits_command_available(self.current()),
        )
    }

    pub fn effective_default_thread_env_mode(&self) -> ThreadEnvMode {
        if self.client.is_some() {
            self.machines
                .views
                .get(&self.machines.selected)
                .and_then(|v| {
                    serde_json::from_value::<ThreadEnvMode>(
                        v["index"]["threadDefaults"]["default_thread_env_mode"].clone(),
                    )
                    .ok()
                })
                .unwrap_or(self.thread_defaults.default_thread_env_mode)
        } else {
            self.thread_defaults.default_thread_env_mode
        }
    }

    pub fn effective_env_mode(&self, thread: &Thread) -> ThreadEnvMode {
        if thread.worktree_path.is_some() {
            return ThreadEnvMode::Worktree;
        }
        if !thread.messages.is_empty() {
            return ThreadEnvMode::Local;
        }
        thread
            .env_mode
            .unwrap_or_else(|| self.effective_default_thread_env_mode())
    }

    pub fn supports_vcs_worktrees_on(&self, machine: &str) -> bool {
        self.client.as_ref().is_none_or(|client| {
            client.supports_capability(machine, crate::protocol::VCS_WORKTREES)
        })
    }

    pub fn thread_vcs_repo_key(&self, thread: &Thread) -> Option<(String, String)> {
        if thread.captain_home && !thread.delegated {
            return None;
        }
        let project = self.project(thread.project)?;
        let machine = if self.client.is_some() {
            self.machines.selected.clone()
        } else {
            thread.machine.clone()
        };
        Some((machine, project.path.to_string_lossy().into_owned()))
    }

    pub fn composer_vcs_controls(&self) -> ComposerVcsControls {
        if self.demo {
            return ComposerVcsControls::Hidden;
        }
        let thread = self.current();
        let Some((machine, project_path)) = self.thread_vcs_repo_key(thread) else {
            return ComposerVcsControls::Hidden;
        };
        if !self.supports_vcs_worktrees_on(&machine) {
            return ComposerVcsControls::Hidden;
        }
        if let Some(worktree_path) = &thread.worktree_path {
            return ComposerVcsControls::LockedWorktree {
                branch: thread.branch.clone(),
                worktree_path: worktree_path.clone(),
            };
        }
        if !thread.messages.is_empty() {
            return ComposerVcsControls::Hidden;
        }
        let is_repo = self
            .vcs_branches
            .get(&(machine, project_path))
            .is_some_and(|entry| entry.is_repo);
        if !is_repo {
            return ComposerVcsControls::Hidden;
        }
        let mode = if !thread.draft_model_state.additional_models.is_empty() {
            ThreadEnvMode::Worktree
        } else {
            self.effective_env_mode(thread)
        };
        ComposerVcsControls::Editable {
            mode,
            base_branch: thread.base_branch.clone(),
            start_from_origin: thread.effective_start_from_origin(),
        }
    }

    pub fn ensure_vcs_branches(&mut self, force_refresh: bool) {
        if self.demo {
            return;
        }
        if !force_refresh && !self.runtime_enabled && self.client.is_none() {
            return;
        }
        let Some((machine, project_path)) = self.thread_vcs_repo_key(self.current()) else {
            return;
        };
        if !self.supports_vcs_worktrees_on(&machine) {
            return;
        }
        let key = (machine.clone(), project_path.clone());
        if let Some(entry) = self.vcs_branches.get(&key)
            && (entry.loading || (entry.loaded && !force_refresh))
        {
            return;
        }
        if let Some(client) = &self.client {
            if !self.machine_connected(&machine) {
                return;
            }
            let target = if machine == "local" {
                Some(client)
            } else {
                client.linked.get(&machine)
            };
            let Some(target) = target else {
                return;
            };
            self.vcs_branches.entry(key.clone()).or_default().loading = true;
            self.next_vcs_request_id += 1;
            let req_id = self.next_vcs_request_id;
            self.pending_vcs_requests.insert(req_id, key);
            let input = crate::vcs::VcsListBranchesInput {
                cwd: project_path,
                query: None,
                cursor: None,
                include_matching_remote_refs: None,
                ref_kind: None,
                refresh: force_refresh.then_some(true),
                limit: Some(crate::vcs::GIT_LIST_BRANCHES_MAX_LIMIT),
            };
            if let Ok(params) = serde_json::to_value(&input) {
                target.call_rpc(format!("vcs-branches:{req_id}"), "vcs.listBranches", params);
            }
            return;
        }
        self.vcs_branches.entry(key).or_default().loading = true;
        let (tx, rx) = std::sync::mpsc::channel();
        self.vcs_branch_rxs.push(rx);
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let input = crate::vcs::VcsListBranchesInput {
                cwd: project_path.clone(),
                query: None,
                cursor: None,
                include_matching_remote_refs: None,
                ref_kind: None,
                refresh: force_refresh.then_some(true),
                limit: Some(crate::vcs::GIT_LIST_BRANCHES_MAX_LIMIT),
            };
            let result = crate::vcs::list_branches(&input);
            let _ = tx.send(VcsBranchResultMsg {
                machine,
                project_path,
                result,
            });
            wake();
        });
    }

    pub fn poll_vcs_branches(&mut self) -> bool {
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.vcs_branch_rxs.len());
        let rxs = std::mem::take(&mut self.vcs_branch_rxs);
        for rx in rxs {
            match rx.try_recv() {
                Ok(msg) => {
                    changed = true;
                    self.apply_vcs_branches_result(msg.machine, msg.project_path, msg.result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => changed = true,
            }
        }
        self.vcs_branch_rxs = remaining;
        changed
    }

    pub(crate) fn apply_vcs_branches_result(
        &mut self,
        machine: String,
        project_path: String,
        result: Result<crate::vcs::VcsListBranchesResult, String>,
    ) {
        let key = (machine.clone(), project_path.clone());
        let entry = self.vcs_branches.entry(key.clone()).or_default();
        entry.loading = false;
        entry.loaded = true;
        match result {
            Ok(res) => {
                entry.is_repo = res.is_repo;
                entry.error = None;
                entry.refs = res.refs;
            }
            Err(err) => {
                entry.error = Some(err);
            }
        }
        let default_branch = if entry.is_repo {
            crate::branch_toolbar::default_worktree_base_branch(&entry.refs, None)
        } else {
            None
        };
        if let Some(default_branch) = default_branch {
            let mut updated_active = false;
            let default_mode = self.effective_default_thread_env_mode();
            for index in 0..self.threads.len() {
                let matches = self.thread_vcs_repo_key(&self.threads[index]).as_ref() == Some(&key);
                if matches {
                    let t = &self.threads[index];
                    let mode = if t.worktree_path.is_some() {
                        ThreadEnvMode::Worktree
                    } else if !t.messages.is_empty() {
                        ThreadEnvMode::Local
                    } else {
                        t.env_mode.unwrap_or(default_mode)
                    };
                    if t.messages.is_empty()
                        && t.worktree_path.is_none()
                        && mode == ThreadEnvMode::Worktree
                        && t.base_branch.is_none()
                    {
                        self.threads[index].base_branch = Some(default_branch.clone());
                        if index == self.active {
                            updated_active = true;
                        }
                    }
                }
            }
            if updated_active {
                self.checkpoint();
            }
        }
    }

    pub(crate) fn handle_vcs_branches_rpc(&mut self, req_id: u64, result: Result<Value, String>) {
        let Some((machine, project_path)) = self.pending_vcs_requests.remove(&req_id) else {
            return;
        };
        let parsed = result.and_then(|val| {
            serde_json::from_value::<crate::vcs::VcsListBranchesResult>(val)
                .map_err(|e| format!("Invalid vcs.listBranches response: {e}"))
        });
        self.apply_vcs_branches_result(machine, project_path, parsed);
    }

    pub fn set_draft_env_mode(&mut self, mode: ThreadEnvMode) {
        let t = self.current();
        if !t.messages.is_empty() || t.worktree_path.is_some() {
            return;
        }
        let default_branch = self.thread_vcs_repo_key(t).and_then(|key| {
            self.vcs_branches
                .get(&key)
                .filter(|entry| entry.is_repo)
                .and_then(|entry| {
                    crate::branch_toolbar::default_worktree_base_branch(&entry.refs, None)
                })
        });
        let thread = self.current_mut();
        thread.env_mode = Some(mode);
        if mode == ThreadEnvMode::Local {
            if thread.submission_error.as_deref()
                == Some("Select a base branch before sending in New worktree mode.")
            {
                thread.submission_error = None;
            }
        } else {
            if thread.base_branch.is_none() {
                thread.base_branch = default_branch;
            }
            // Do not keep an idle prewarmed session in the main checkout once switched to worktree.
            if thread.pi.is_some() && thread.session.is_none() && thread.messages.is_empty() {
                thread.disconnect();
                thread.prewarm_attempted = false;
            }
        }
        self.env_mode_picker = false;
        if mode == ThreadEnvMode::Worktree {
            self.ensure_vcs_branches(false);
        }
        self.checkpoint();
    }

    pub fn set_draft_base_branch(&mut self, branch: String) {
        let thread = self.current_mut();
        if !thread.messages.is_empty() || thread.worktree_path.is_some() {
            return;
        }
        thread.base_branch = Some(branch);
        if thread.submission_error.as_deref()
            == Some("Select a base branch before sending in New worktree mode.")
        {
            thread.submission_error = None;
        }
        self.base_branch_picker = false;
        self.checkpoint();
    }

    pub fn set_draft_start_from_origin(&mut self, start_from_origin: bool) {
        let thread = self.current_mut();
        if !thread.messages.is_empty() || thread.worktree_path.is_some() {
            return;
        }
        thread.start_from_origin = Some(start_from_origin);
        self.checkpoint();
    }

    pub(crate) fn missing_worktree_base_branch_error(&self, thread: &Thread) -> Option<String> {
        let should_create = !thread.worktree_bootstrapping()
            && !thread.native.queue_held
            && thread.messages.is_empty()
            && thread.worktree_path.is_none()
            && thread.project.is_some()
            && !self.is_captain_home(thread)
            && self.effective_env_mode(thread) == ThreadEnvMode::Worktree;
        if should_create
            && thread
                .base_branch
                .as_deref()
                .is_none_or(|b| b.trim().is_empty())
        {
            Some("Select a base branch before sending in New worktree mode.".into())
        } else {
            None
        }
    }

    fn build_current_prompt(&mut self) -> Option<Prompt> {
        if self.current().harness == "pi"
            && !self.current().model_provider.is_empty()
            && self.current().model_provider != "cpa"
        {
            self.current_mut().error = Some(
                "Choose a CPA model before sending. Shina will not send prompts to a direct provider."
                    .into(),
            );
            return None;
        }
        if self.current().draft_images.iter().any(Attachment::is_image)
            && self.current().supports_images == Some(false)
        {
            let err = self.current().image_model_error();
            self.current_mut().attachment_error = Some(err);
            return None;
        }
        let local_server_id = self.machine_identity(&self.machines.selected);
        let peers = self.peers.clone();
        let labels: Vec<(String, String)> = peers
            .iter()
            .map(|p| (p.server_id.clone(), p.label.clone()))
            .collect();
        let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
            self.current().draft_reply_to.as_ref(),
            Some(&self.current().machine),
            Some(&local_server_id),
            &peers,
            &|m| {
                if m == "local" {
                    "Local".into()
                } else {
                    labels
                        .iter()
                        .find(|(id, _)| id == m)
                        .map(|(_, l)| l.clone())
                        .unwrap_or_else(|| m.to_owned())
                }
            },
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.current_mut().attachment_error = Some(error);
                return None;
            }
        };
        if !self.headless
            && let Some(error) = self.current().validate_prompt_length(&prompt)
        {
            self.current_mut().submission_error = Some(error);
            return None;
        }
        self.current_mut().submission_error = None;
        Some(prompt)
    }

    fn try_worktree_bootstrap_send(
        &mut self,
        fresh_context: bool,
        fresh_arm: Option<u64>,
    ) -> Option<bool> {
        let queue_behind_bootstrap = self.current().worktree_bootstrapping()
            || (self.current().native.queue_held
                && self.current().worktree_path.is_some()
                && !self.current().status.busy());
        if queue_behind_bootstrap {
            let prompt = match self.build_current_prompt() {
                Some(p) => p,
                None => return Some(false),
            };
            let thread = self.current_mut();
            thread.prompt_history.reset();
            thread.typing_guard.on_blur_or_send();
            thread.paste_history = Default::default();
            thread.draft.replace_text("");
            thread.draft_images.clear();
            thread.draft_context.clear();
            thread.draft_reply_to = None;
            thread.retained_context = Default::default();
            thread.native.queue_held = true;
            thread.serial += 1;
            let queued_id = thread.serial;
            if fresh_context {
                thread.native.fresh_queue_ids.insert(queued_id);
            }
            thread.native.queue.push_back(pi_session::QueuedMessage {
                id: queued_id,
                prompt,
            });
            thread.life.user_sent(lifecycle::now());
            let id = thread.id;
            let server = self.machines.selected.clone();
            self.set_keep_full_history(&server, id, false);
            if let Some(arm) = fresh_arm {
                self.disarm_fresh_session(id, Some(arm));
            }
            self.checkpoint();
            return Some(true);
        }

        let is_first_worktree = self.current().messages.is_empty()
            && self.current().worktree_path.is_none()
            && self.current().project.is_some()
            && !self.is_captain_home(self.current())
            && self.effective_env_mode(self.current()) == ThreadEnvMode::Worktree;
        if !is_first_worktree {
            return None;
        }

        if let Some(error) = self.missing_worktree_base_branch_error(self.current()) {
            self.current_mut().submission_error = Some(error);
            return Some(false);
        }
        let base_branch = self
            .current()
            .base_branch
            .clone()
            .map(|b| b.trim().to_owned())
            .filter(|b| !b.is_empty())?;
        let prompt = match self.build_current_prompt() {
            Some(p) => p,
            None => return Some(false),
        };
        let project_cwd = self
            .project(self.current().project)
            .map(|p| p.path.clone())?;
        let snapshot = crate::composer_rich::Snapshot::capture_with_context(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
        );
        let thread = self.current_mut();
        let reply_to = thread.draft_reply_to.take();
        let start_from_origin = thread.effective_start_from_origin();
        thread.prompt_history.reset();
        thread.typing_guard.on_blur_or_send();
        thread.paste_history = Default::default();
        thread.draft.replace_text("");
        thread.draft_images.clear();
        thread.draft_context.clear();
        thread.retained_context = Default::default();
        thread.error = None;
        thread.attachment_error = None;
        if thread.pi.is_some() && thread.session.is_none() {
            thread.disconnect();
            thread.prewarm_attempted = false;
        }
        thread.native.queue_held = true;
        thread.serial += 1;
        let queued_id = thread.serial;
        if fresh_context {
            thread.native.fresh_queue_ids.insert(queued_id);
        }
        thread.native.queue.push_back(pi_session::QueuedMessage {
            id: queued_id,
            prompt,
        });
        thread.worktree_bootstrap = Some(WorktreeBootstrapState {
            snapshot,
            reply_to,
            queued_id,
            fresh_context,
        });
        let thread_id = thread.id;
        let server = self.machines.selected.clone();
        self.set_keep_full_history(&server, thread_id, false);
        self.invalidate_undo(thread_id);
        if let Some(arm) = fresh_arm {
            self.disarm_fresh_session(thread_id, Some(arm));
        }

        let worktrees_dir = self.data_dir.join("worktrees");
        let wake = self.wake.clone();
        #[cfg(test)]
        let git_ctx = self.test_git_context.clone().unwrap_or_default();
        #[cfg(not(test))]
        let git_ctx = crate::vcs::GitContext::default();
        let (tx, rx) = std::sync::mpsc::channel();
        self.worktree_bootstrap_rxs.push(rx);
        std::thread::spawn(move || {
            let cwd_str = project_cwd.to_string_lossy().into_owned();
            let raw_branch = crate::vcs::build_temporary_worktree_branch_name(
                &uuid::Uuid::new_v4().simple().to_string(),
            );
            let new_branch =
                crate::vcs::resolve_available_branch_name_with(&project_cwd, &raw_branch, &git_ctx)
                    .unwrap_or(raw_branch);
            let result = crate::vcs::create_worktree_with(
                &crate::vcs::VcsCreateWorktreeInput {
                    cwd: cwd_str,
                    ref_name: base_branch.clone(),
                    new_ref_name: Some(new_branch),
                    base_ref_name: Some(base_branch.clone()),
                    path: None,
                    start_from_origin: Some(start_from_origin),
                },
                &worktrees_dir,
                &git_ctx,
            );
            let _ = tx.send(WorktreeBootstrapResult {
                thread_id,
                base_branch,
                result,
            });
            wake();
        });
        self.checkpoint();
        Some(true)
    }

    pub fn poll_worktree_bootstraps(&mut self) -> bool {
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.worktree_bootstrap_rxs.len());
        let rxs = std::mem::take(&mut self.worktree_bootstrap_rxs);
        for rx in rxs {
            match rx.try_recv() {
                Ok(msg) => {
                    changed = true;
                    let Some(index) = self.threads.iter().position(|t| t.id == msg.thread_id)
                    else {
                        continue;
                    };
                    let Some(boot) = self.threads[index].worktree_bootstrap.take() else {
                        continue;
                    };
                    let in_multi_batch = self.is_multi_model_target_thread(msg.thread_id);
                    match msg.result {
                        Ok(created) => {
                            let project_cwd = self
                                .project(self.threads[index].project)
                                .map(|p| p.path.clone());
                            let wt_path = PathBuf::from(&created.worktree.path);
                            let script_check = project_cwd
                                .as_deref()
                                .map(|cwd| crate::vcs::find_setup_script_info(&wt_path, cwd));
                            let thread = &mut self.threads[index];
                            thread.worktree_path = Some(created.worktree.path.clone());
                            thread.branch = Some(created.worktree.ref_name.clone());
                            thread.base_branch = Some(msg.base_branch);
                            thread.env_mode = Some(ThreadEnvMode::Worktree);
                            thread.submission_error = None;
                            thread.worktree_bootstrap_fresh_context = boot.fresh_context;
                            thread.life.user_sent(lifecycle::now());
                            if boot.fresh_context {
                                self.persist_transcripts = true;
                            }
                            match script_check {
                                Some(Ok(Some((script_name, script_command)))) => {
                                    thread.worktree_setup = Some(WorktreeSetupState {
                                        phase: WorktreeSetupPhase::Running,
                                        script_name: Some(script_name),
                                        script_command: Some(script_command),
                                        error: None,
                                        output: String::new(),
                                        exit_code: None,
                                        timed_out: false,
                                        duration_ms: 0,
                                        skipped: false,
                                    });
                                    thread.worktree_setup_output_open = false;
                                    thread.native.queue_held = true;
                                    let tid = thread.id;
                                    self.spawn_worktree_setup_runner(tid);
                                }
                                Some(Err(error)) => {
                                    thread.worktree_setup = Some(WorktreeSetupState {
                                        phase: WorktreeSetupPhase::Failed,
                                        script_name: None,
                                        script_command: None,
                                        error: Some(error),
                                        output: String::new(),
                                        exit_code: None,
                                        timed_out: false,
                                        duration_ms: 0,
                                        skipped: false,
                                    });
                                    thread.worktree_setup_output_open = true;
                                    thread.native.queue_held = true;
                                }
                                _ => {
                                    thread.native.queue_held = false;
                                    self.release_after_worktree_setup(index);
                                }
                            }
                            if in_multi_batch
                                && let Some(idx) =
                                    self.threads.iter().position(|t| t.id == msg.thread_id)
                            {
                                if self.threads[idx].status == Status::Failed {
                                    let error = self.threads[idx]
                                        .error
                                        .clone()
                                        .unwrap_or_else(|| "Failed to start thread.".into());
                                    self.remove_failed_multi_model_thread(msg.thread_id);
                                    self.record_multi_model_target_outcome(
                                        msg.thread_id,
                                        Err(error),
                                    );
                                } else {
                                    self.threads[idx].background_start_pending =
                                        self.runtime_enabled || self.threads[idx].pi.is_some();
                                    self.record_multi_model_target_outcome(msg.thread_id, Ok(()));
                                }
                            }
                            self.checkpoint();
                        }
                        Err(error) => {
                            if in_multi_batch {
                                self.remove_failed_multi_model_thread(msg.thread_id);
                                self.record_multi_model_target_outcome(msg.thread_id, Err(error));
                            } else {
                                let thread = &mut self.threads[index];
                                thread.native.queue.retain(|q| q.id != boot.queued_id);
                                thread.native.fresh_queue_ids.remove(&boot.queued_id);
                                thread.native.queue_held = false;
                                thread.worktree_path = None;
                                thread.branch = None;
                                thread.env_mode = Some(ThreadEnvMode::Worktree);
                                thread.restore_snapshot(&boot.snapshot);
                                if thread.draft_reply_to.is_none() {
                                    thread.draft_reply_to = boot.reply_to;
                                }
                                thread.submission_error = Some(error);
                            }
                            self.checkpoint();
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => changed = true,
            }
        }
        self.worktree_bootstrap_rxs = remaining;
        self.resolve_completed_multi_model_batches();
        changed
    }

    fn is_multi_model_target_thread(&self, thread_id: usize) -> bool {
        self.multi_model_batches.iter().any(|batch| {
            batch
                .targets
                .iter()
                .any(|target| target.thread_id == Some(thread_id))
        })
    }

    fn remove_failed_multi_model_thread(&mut self, thread_id: usize) {
        let active_id = self.threads.get(self.active).map(|t| t.id);
        if let Some(t) = self.threads.iter_mut().find(|t| t.id == thread_id) {
            let _ = t.stop_task();
            t.disconnect();
        }
        self.threads.retain(|t| t.id != thread_id);
        self.tabs.retain(|id| *id != thread_id);
        if let Some(aid) = active_id
            && let Some(new_idx) = self.threads.iter().position(|t| t.id == aid)
        {
            self.active = new_idx;
        } else if self.active >= self.threads.len() && !self.threads.is_empty() {
            self.active = self.threads.len() - 1;
        }
    }

    fn record_multi_model_target_outcome(&mut self, thread_id: usize, result: Result<(), String>) {
        for batch in &mut self.multi_model_batches {
            if let Some(target) = batch
                .targets
                .iter_mut()
                .find(|t| t.thread_id == Some(thread_id))
            {
                target.outcome = match result {
                    Ok(()) => MultiModelTargetOutcome::Started,
                    Err(err) => MultiModelTargetOutcome::Failed(err),
                };
                return;
            }
        }
    }

    pub(crate) fn handle_multi_model_async_failure(
        &mut self,
        thread_id: usize,
        error: &str,
    ) -> bool {
        let in_open_batch = self
            .multi_model_batches
            .iter()
            .any(|batch| batch.targets.iter().any(|t| t.thread_id == Some(thread_id)));
        if !in_open_batch {
            return false;
        }
        self.remove_failed_multi_model_thread(thread_id);
        self.record_multi_model_target_outcome(thread_id, Err(error.to_owned()));
        self.resolve_completed_multi_model_batches();
        true
    }

    fn resolve_completed_multi_model_batches(&mut self) {
        let mut remaining = Vec::with_capacity(self.multi_model_batches.len());
        let mut completed = Vec::new();
        for batch in self.multi_model_batches.drain(..) {
            if batch
                .targets
                .iter()
                .any(|t| matches!(t.outcome, MultiModelTargetOutcome::Pending))
            {
                remaining.push(batch);
            } else {
                completed.push(batch);
            }
        }
        self.multi_model_batches = remaining;
        for batch in completed {
            self.finalize_multi_model_batch(batch);
        }
    }

    fn finalize_multi_model_batch(&mut self, batch: MultiModelBatch) {
        let started_count = batch
            .targets
            .iter()
            .filter(|t| matches!(t.outcome, MultiModelTargetOutcome::Started))
            .count();
        let failed_entries: Vec<&MultiModelTargetEntry> = batch
            .targets
            .iter()
            .filter(|t| matches!(t.outcome, MultiModelTargetOutcome::Failed(_)))
            .collect();

        if failed_entries.is_empty() {
            self.push_toast(
                ToastKind::Success,
                format!(
                    "Started {started_count} {} in background",
                    if started_count == 1 {
                        "thread"
                    } else {
                        "threads"
                    }
                ),
                None,
                None,
                Some(5_000),
            );
            self.checkpoint();
            return;
        }

        if let Some(source_idx) = self
            .threads
            .iter()
            .position(|t| t.id == batch.source_thread_id)
            && !self.threads[source_idx].has_draft_content()
        {
            self.threads[source_idx].restore_snapshot(&batch.snapshot);
            if self.threads[source_idx].draft_reply_to.is_none() {
                self.threads[source_idx].draft_reply_to = batch.reply_to.clone();
            }
            if started_count == 0 {
                self.threads[source_idx].draft_model_state = batch.saved_draft_model_state.clone();
            } else {
                let first_failed = &failed_entries[0].selection;
                let remaining_failed: Vec<crate::model_selection::ModelSelection> = failed_entries
                    .iter()
                    .skip(1)
                    .map(|e| e.selection.clone())
                    .collect();
                let current_active = self.active;
                self.active = source_idx;
                if self.threads[source_idx].harness != first_failed.instance_id
                    && self.validate_harness(&first_failed.instance_id).is_ok()
                {
                    self.choose_harness(&first_failed.instance_id);
                }
                let provider = if first_failed.instance_id == "pi" {
                    "cpa".to_owned()
                } else {
                    first_failed.instance_id.clone()
                };
                self.threads[source_idx].set_model(provider, first_failed.model.clone());
                for opt in &first_failed.options {
                    self.threads[source_idx].set_option(opt.id.clone(), opt.value.clone());
                }
                self.threads[source_idx].draft_model_state =
                    crate::model_selection::DraftModelState {
                        active_provider: Some(first_failed.instance_id.clone()),
                        model_selection_by_provider: std::collections::BTreeMap::from([(
                            first_failed.instance_id.clone(),
                            first_failed.clone(),
                        )]),
                        model_selection_explicit: true,
                        additional_models: remaining_failed,
                    };
                self.threads[source_idx].env_mode = Some(ThreadEnvMode::Worktree);
                if self.threads[source_idx].base_branch.is_none() {
                    self.threads[source_idx].base_branch = Some(batch.base_branch.clone());
                }
                self.active = current_active;
            }
        }

        let failed_names: Vec<String> = failed_entries
            .iter()
            .map(|e| e.display_name.clone())
            .collect();
        let mut errors = Vec::new();
        for entry in &failed_entries {
            if let MultiModelTargetOutcome::Failed(err) = &entry.outcome
                && !errors.contains(err)
            {
                errors.push(err.clone());
            }
        }
        self.push_toast(
            ToastKind::Error,
            format!("Could not start {}", failed_names.join(", ")),
            (!errors.is_empty()).then(|| errors.join(" ")),
            None,
            None,
        );
        self.checkpoint();
    }

    fn model_display_name_for_selection(
        &self,
        selection: &crate::model_selection::ModelSelection,
    ) -> String {
        if selection.instance_id == "pi" {
            if let Some(m) = self.catalog.models.iter().find(|m| m.id == selection.model) {
                return m.name.clone();
            }
        }
        self.picker_models_by_instance()
            .get(&selection.instance_id)
            .and_then(|opts| opts.iter().find(|o| o.slug == selection.model))
            .map(|o| o.name.clone())
            .unwrap_or_else(|| selection.model.clone())
    }

    fn validate_local_multi_model_target(
        &self,
        selection: &crate::model_selection::ModelSelection,
        has_images: bool,
        entries: &[crate::model_picker::ProviderInstanceEntry],
        picker_models: &std::collections::BTreeMap<
            String,
            Vec<crate::model_picker::PickerModelOption>,
        >,
    ) -> Result<Vec<AgentModel>, String> {
        self.validate_harness(&selection.instance_id)?;
        if let Some(entry) = entries
            .iter()
            .find(|e| e.instance_id == selection.instance_id)
            && !crate::model_picker::is_provider_instance_picker_ready(entry)
        {
            return Err(crate::model_picker::get_provider_status_message(entry));
        }

        if selection.instance_id == "pi" {
            if let Some(model) = self.catalog.models.iter().find(|m| m.id == selection.model) {
                if !model.provider.is_empty() && model.provider != "cpa" {
                    return Err(
                        "Choose a CPA model before sending. Shina will not send prompts to a direct provider."
                            .into(),
                    );
                }
                if has_images && model.supports_images == Some(false) {
                    return Err(
                        "This model does not support image attachments. Remove the images or switch to a vision model."
                            .into(),
                    );
                }
            } else if !self.catalog.models.is_empty() && !selection.model.starts_with("cpa/") {
                return Err(
                    "Choose a CPA model before sending. Shina will not send prompts to a direct provider."
                        .into(),
                );
            }
            return Ok(self.catalog.models.clone());
        }

        if let Some(opt) = picker_models
            .get(&selection.instance_id)
            .and_then(|opts| opts.iter().find(|o| o.slug == selection.model))
            && opt.is_unavailable
        {
            return Err(format!("{} is unavailable.", opt.name));
        }

        let models = self.harness_models(&selection.instance_id);
        if has_images
            && models
                .iter()
                .find(|m| m.id == selection.model)
                .is_some_and(|m| m.supports_images == Some(false))
        {
            return Err(
                "This model does not support image attachments. Remove the images or switch to a vision model."
                    .into(),
            );
        }
        Ok(models)
    }

    fn advance_remote_multi_model_batch(&mut self, batch_idx: usize) {
        let Some(batch) = self.multi_model_batches.get_mut(batch_idx) else {
            return;
        };
        let server = batch.server.clone();
        while batch.next_remote_index < batch.targets.len() {
            let target_idx = batch.next_remote_index;
            batch.next_remote_index += 1;
            let selection = batch.targets[target_idx].selection.clone();
            let provider = if selection.instance_id == "pi" {
                "cpa".to_owned()
            } else {
                selection.instance_id.clone()
            };
            let thinking = selection
                .options
                .iter()
                .find(|o| {
                    matches!(
                        o.id.as_str(),
                        "thinking" | "effort" | "reasoningEffort" | "reasoning" | "thought_level"
                    )
                })
                .and_then(|o| o.value.as_str())
                .map(str::to_owned);
            let service_tier = selection
                .options
                .iter()
                .find(|o| o.id == "serviceTier")
                .and_then(|o| o.value.as_str())
                .map(str::to_owned);
            let mut occupied: Vec<usize> = self
                .threads
                .iter()
                .filter(|t| {
                    !t.client_draft
                        && (t.has_draft_content()
                            || !t.messages.is_empty()
                            || self.tabs.contains(&t.id))
                })
                .map(|t| t.id)
                .collect();
            for &id in &batch.occupied_remote_ids {
                if !occupied.contains(&id) {
                    occupied.push(id);
                }
            }
            let params = json!({
                "project": batch.project,
                "occupiedDrafts": occupied,
                "harness": selection.instance_id,
                "selectedModel": (provider, selection.model),
                "thinking": thinking,
                "access": batch.access.id(),
                "serviceTier": service_tier,
            });
            let result = self
                .client
                .as_ref()
                .and_then(|client| client.commands_for(&server))
                .ok_or_else(|| "This machine is not linked. Your draft was kept.".to_owned())
                .and_then(|commands| commands.send("thread.create", params));
            match result {
                Ok(cmd_id) => {
                    batch.targets[target_idx].create_cmd_id = Some(cmd_id);
                    break;
                }
                Err(error) => {
                    batch.targets[target_idx].outcome = MultiModelTargetOutcome::Failed(error);
                }
            }
        }
    }

    pub(crate) fn handle_multi_model_remote_receipt(
        &mut self,
        server: &str,
        command: &crate::protocol::Command,
        value: &Value,
    ) -> bool {
        let accepted = value["result"]["status"] == "accepted";
        let error_msg = value["error"]["message"]
            .as_str()
            .or(value["result"]["result"]["error"].as_str())
            .unwrap_or("Failed to send message.")
            .to_owned();

        if command.method == "thread.create" {
            let found = self
                .multi_model_batches
                .iter()
                .enumerate()
                .find_map(|(bi, b)| {
                    (b.server == server).then(|| {
                        b.targets
                            .iter()
                            .position(|t| t.create_cmd_id == Some(command.command_id))
                            .map(|ti| (bi, ti))
                    })?
                });
            let Some((batch_idx, target_idx)) = found else {
                return false;
            };
            if !accepted {
                self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                    MultiModelTargetOutcome::Failed(error_msg);
            } else if let Some(tid) = value["result"]["result"]["threadId"].as_u64() {
                let tid = tid as usize;
                self.multi_model_batches[batch_idx]
                    .occupied_remote_ids
                    .push(tid);
                self.multi_model_batches[batch_idx].targets[target_idx].thread_id = Some(tid);
                let batch = &self.multi_model_batches[batch_idx];
                let selection = &batch.targets[target_idx].selection;
                let commands = self
                    .client
                    .as_ref()
                    .and_then(|client| client.commands_for(server))
                    .cloned();
                let supports_options = self
                    .client
                    .as_ref()
                    .is_some_and(|c| c.supports_capability(server, crate::protocol::MODEL_OPTIONS));
                if supports_options
                    && !selection.options.is_empty()
                    && let Some(ref cmds) = commands
                {
                    let _ = cmds.send(
                        "thread.configure",
                        json!({"threadId": tid, "options": selection.options}),
                    );
                }
                let params = json!({
                    "threadId": tid,
                    "text": batch.prompt.text,
                    "attachments": batch.prompt.images.as_ref(),
                    "context": batch.prompt.context,
                    "draft": batch.prompt.draft,
                    "editorRevision": uuid::Uuid::new_v4(),
                    "mode": "prompt",
                    "envMode": ThreadEnvMode::Worktree,
                    "baseBranch": batch.base_branch,
                    "startFromOrigin": batch.start_from_origin,
                });
                let send_res = commands
                    .ok_or_else(|| "This machine is not linked. Your draft was kept.".to_owned())
                    .and_then(|cmds| cmds.send("thread.send", params));
                match send_res {
                    Ok(cmd_id) => {
                        self.multi_model_batches[batch_idx].targets[target_idx].send_cmd_id =
                            Some(cmd_id);
                    }
                    Err(err) => {
                        self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                            MultiModelTargetOutcome::Failed(err);
                        if let Some(cmds) =
                            self.client.as_ref().and_then(|c| c.commands_for(server))
                        {
                            let _ = cmds
                                .send("thread.delete", json!({"threadId": tid, "blankOnly": true}));
                        }
                    }
                }
            } else {
                self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                    MultiModelTargetOutcome::Failed(
                        "Thread creation was not confirmed. Your draft was kept.".into(),
                    );
            }
            self.advance_remote_multi_model_batch(batch_idx);
            self.resolve_completed_multi_model_batches();
            return true;
        }

        if command.method == "thread.send" {
            let found = self
                .multi_model_batches
                .iter()
                .enumerate()
                .find_map(|(bi, b)| {
                    (b.server == server).then(|| {
                        b.targets
                            .iter()
                            .position(|t| t.send_cmd_id == Some(command.command_id))
                            .map(|ti| (bi, ti))
                    })?
                });
            let Some((batch_idx, target_idx)) = found else {
                return false;
            };
            if accepted {
                self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                    MultiModelTargetOutcome::Started;
            } else {
                let tid = self.multi_model_batches[batch_idx].targets[target_idx].thread_id;
                self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                    MultiModelTargetOutcome::Failed(error_msg);
                if let Some(tid) = tid
                    && let Some(cmds) = self.client.as_ref().and_then(|c| c.commands_for(server))
                {
                    let _ = cmds.send("thread.delete", json!({"threadId": tid, "blankOnly": true}));
                }
            }
            self.resolve_completed_multi_model_batches();
            return true;
        }

        false
    }

    /// Ported from KSG Code's `components/chat/useMultipleModelSend.ts`:
    /// starts one background thread per selected model, each in its own worktree.
    fn dispatch_multi_model_send(&mut self) -> bool {
        if self.current().draft.text.trim() == "/model" {
            self.choose_slash("model".into());
            return false;
        }
        if self.current().configuring
            || self.current().native.operating()
            || self.current().phase == "Stopping"
            || !self.current().has_draft_content()
        {
            return false;
        }
        if self.client.is_some() {
            if !self.machine_connected(&self.machines.selected) {
                self.push_toast(
                    ToastKind::Warning,
                    "Not connected: message not sent",
                    Some("Reconnecting to the environment. Try again once it is connected.".into()),
                    None,
                    Some(5_000),
                );
                return false;
            }
        } else if self.network_blocked() {
            self.current_mut().error =
                Some("Waiting for the network after resume. Your draft has not been sent.".into());
            return false;
        }

        let machine = if self.client.is_some() {
            self.machines.selected.clone()
        } else {
            self.current().machine.clone()
        };
        let server_supports = self.supports_vcs_worktrees_on(&machine);
        let is_local_draft = self.current().is_draft()
            && !self.current().worktree_bootstrapping()
            && !self.current().native.queue_held
            && self.current().worktree_path.is_none();
        let repo_key = self.thread_vcs_repo_key(self.current());
        let is_git_repo = repo_key
            .as_ref()
            .is_some_and(|k| self.vcs_branches.get(k).is_some_and(|e| e.is_repo));
        let base_branch = self
            .current()
            .base_branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                repo_key
                    .as_ref()
                    .and_then(|k| self.vcs_branches.get(k))
                    .filter(|e| e.is_repo)
                    .and_then(|e| {
                        crate::branch_toolbar::default_worktree_base_branch(&e.refs, None)
                    })
            });
        let selections = self.multi_model_selections();
        match crate::model_selection::multiple_model_send_block(
            selections.len(),
            server_supports,
            is_local_draft,
            is_git_repo,
            base_branch.as_deref(),
        ) {
            Some(crate::model_selection::MultipleModelSendBlock::ServerOutdated) => {
                self.current_mut().error =
                    Some("Update this server before starting multiple models.".into());
                return false;
            }
            Some(crate::model_selection::MultipleModelSendBlock::NeedsNewGitThread) => {
                if server_supports && is_local_draft && is_git_repo && base_branch.is_none() {
                    self.current_mut().submission_error =
                        Some("Select a base branch before sending in New worktree mode.".into());
                } else {
                    self.push_toast(
                        ToastKind::Warning,
                        "Choose models and a base branch",
                        Some(
                            "Multiple models need a new thread in a Git project. Each gets its own worktree."
                                .into(),
                        ),
                        None,
                        Some(5_000),
                    );
                }
                return false;
            }
            None => {}
        }
        let base_branch = base_branch.unwrap();

        let target_server_id = self.machine_identity(&machine);
        let peers = self.peers.clone();
        let labels: Vec<(String, String)> = peers
            .iter()
            .map(|p| (p.server_id.clone(), p.label.clone()))
            .collect();
        let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
            self.current().draft_reply_to.as_ref(),
            Some(&machine),
            Some(&target_server_id),
            &peers,
            &|m| {
                if m == "local" {
                    "Local".into()
                } else {
                    labels
                        .iter()
                        .find(|(id, _)| id == m)
                        .map(|(_, l)| l.clone())
                        .unwrap_or_else(|| m.to_owned())
                }
            },
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.current_mut().attachment_error = Some(error);
                return false;
            }
        };
        if !self.headless
            && let Some(error) = self.current().validate_prompt_length(&prompt)
        {
            self.current_mut().submission_error = Some(error);
            return false;
        }

        let snapshot = crate::composer_rich::Snapshot::capture_with_context(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
        );
        let reply_to = self.current().draft_reply_to.clone();
        let saved_draft_model_state = self.current().draft_model_state.clone();
        let source_thread_id = self.current().id;
        let project = self.current().project;
        let access = self.current().native.access;
        let start_from_origin = self.current().effective_start_from_origin();
        let title_seed: String = if prompt.text.is_empty() {
            prompt
                .images
                .first()
                .map(|image| {
                    format!(
                        "{}: {}",
                        if image.is_image() { "Image" } else { "File" },
                        image.name
                    )
                })
                .unwrap_or_else(|| "New thread".into())
                .chars()
                .take(45)
                .collect()
        } else {
            prompt.text.chars().take(45).collect()
        };

        let targets: Vec<MultiModelTargetEntry> = selections
            .iter()
            .map(|sel| MultiModelTargetEntry {
                display_name: self.model_display_name_for_selection(sel),
                selection: sel.clone(),
                thread_id: None,
                create_cmd_id: None,
                send_cmd_id: None,
                outcome: MultiModelTargetOutcome::Pending,
            })
            .collect();

        // Clear the source draft before starting background threads, keeping the user on the draft.
        {
            let thread = self.current_mut();
            thread.prompt_history.reset();
            thread.typing_guard.on_blur_or_send();
            thread.paste_history = Default::default();
            thread.draft.replace_text("");
            thread.draft_images.clear();
            thread.draft_context.clear();
            thread.draft_reply_to = None;
            thread.retained_context = Default::default();
            thread.error = None;
            thread.attachment_error = None;
            thread.submission_error = None;
            thread.draft_model_state.additional_models.clear();
        }
        self.usage_limits_notice_key = None;
        self.composer_notices.remove("usage-limits");

        if self.client.is_some() {
            self.multi_model_batches.push(MultiModelBatch {
                source_thread_id,
                server: machine,
                project,
                base_branch,
                start_from_origin,
                access,
                prompt,
                snapshot,
                reply_to,
                saved_draft_model_state,
                targets,
                occupied_remote_ids: Vec::new(),
                next_remote_index: 0,
            });
            let batch_idx = self.multi_model_batches.len() - 1;
            self.advance_remote_multi_model_batch(batch_idx);
            self.resolve_completed_multi_model_batches();
            self.checkpoint();
            return !self.multi_model_batches.is_empty()
                || self
                    .toasts
                    .iter()
                    .any(|t| t.kind == ToastKind::Success && t.title.contains("in background"));
        }

        let Some(project_cwd) = self.project(project).map(|p| p.path.clone()) else {
            return false;
        };
        let entries = self.provider_instance_entries();
        let picker_models = self.picker_models_by_instance();
        let has_images = prompt.images.iter().any(Attachment::is_image);
        let worktrees_dir = self.data_dir.join("worktrees");
        #[cfg(test)]
        let git_ctx = self.test_git_context.clone().unwrap_or_default();
        #[cfg(not(test))]
        let git_ctx = crate::vcs::GitContext::default();

        let mut resolved_targets = targets;
        for target in &mut resolved_targets {
            let models = match self.validate_local_multi_model_target(
                &target.selection,
                has_images,
                &entries,
                &picker_models,
            ) {
                Ok(models) => models,
                Err(error) => {
                    target.outcome = MultiModelTargetOutcome::Failed(error);
                    continue;
                }
            };
            let thread_id = self.next_id;
            self.next_id += 1;
            let mut thread = Thread::new(thread_id);
            thread.project = project;
            thread.machine = machine.clone();
            thread.harness = target.selection.instance_id.clone();
            thread.env_mode = Some(ThreadEnvMode::Worktree);
            thread.base_branch = Some(base_branch.clone());
            thread.start_from_origin = Some(start_from_origin);
            thread.native.access =
                resolve_compatible_access_mode(access, supported_access_modes(&thread.harness)).1;
            thread.title = title_seed.clone();
            thread.never_sent = false;
            thread.models = models;
            let provider = if target.selection.instance_id == "pi" {
                "cpa".to_owned()
            } else {
                target.selection.instance_id.clone()
            };
            thread.set_model(provider, target.selection.model.clone());
            for opt in &target.selection.options {
                thread.set_option(opt.id.clone(), opt.value.clone());
            }
            thread.native.queue_held = true;
            thread.serial += 1;
            let queued_id = thread.serial;
            thread.native.queue.push_back(pi_session::QueuedMessage {
                id: queued_id,
                prompt: prompt.clone(),
            });
            thread.worktree_bootstrap = Some(WorktreeBootstrapState {
                snapshot: snapshot.clone(),
                reply_to: reply_to.clone(),
                queued_id,
                fresh_context: false,
            });
            self.threads.push(thread);

            let wake = self.wake.clone();
            let worktrees_dir_thread = worktrees_dir.clone();
            let project_cwd_thread = project_cwd.clone();
            let base_branch_thread = base_branch.clone();
            let git_ctx_thread = git_ctx.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.worktree_bootstrap_rxs.push(rx);
            std::thread::spawn(move || {
                let cwd_str = project_cwd_thread.to_string_lossy().into_owned();
                let raw_branch = crate::vcs::build_temporary_worktree_branch_name(
                    &uuid::Uuid::new_v4().simple().to_string(),
                );
                let new_branch = crate::vcs::resolve_available_branch_name_with(
                    &project_cwd_thread,
                    &raw_branch,
                    &git_ctx_thread,
                )
                .unwrap_or(raw_branch);
                let result = crate::vcs::create_worktree_with(
                    &crate::vcs::VcsCreateWorktreeInput {
                        cwd: cwd_str,
                        ref_name: base_branch_thread.clone(),
                        new_ref_name: Some(new_branch),
                        base_ref_name: Some(base_branch_thread.clone()),
                        path: None,
                        start_from_origin: Some(start_from_origin),
                    },
                    &worktrees_dir_thread,
                    &git_ctx_thread,
                );
                let _ = tx.send(WorktreeBootstrapResult {
                    thread_id,
                    base_branch: base_branch_thread,
                    result,
                });
                wake();
            });

            target.thread_id = Some(thread_id);
            target.outcome = MultiModelTargetOutcome::Pending;
        }

        let any_started_or_pending = resolved_targets
            .iter()
            .any(|t| !matches!(t.outcome, MultiModelTargetOutcome::Failed(_)));
        self.multi_model_batches.push(MultiModelBatch {
            source_thread_id,
            server: machine,
            project,
            base_branch,
            start_from_origin,
            access,
            prompt,
            snapshot,
            reply_to,
            saved_draft_model_state,
            targets: resolved_targets,
            occupied_remote_ids: Vec::new(),
            next_remote_index: 0,
        });
        self.resolve_completed_multi_model_batches();
        self.checkpoint();
        any_started_or_pending
    }

    fn release_after_worktree_setup(&mut self, index: usize) {
        if index >= self.threads.len() {
            return;
        }
        if self.threads[index].worktree_bootstrap_fresh_context {
            self.persist_transcripts = true;
        }
        self.threads[index].native.queue_held = false;
        if self.runtime_enabled || self.threads[index].pi.is_some() {
            if self.threads[index].pi.is_none() {
                self.connect_thread(index);
            }
            if self.threads[index].pi.is_some() {
                self.threads[index].drive_session();
            }
        }
    }

    fn spawn_worktree_setup_runner(&mut self, thread_id: usize) {
        let Some(index) = self.threads.iter().position(|t| t.id == thread_id) else {
            return;
        };
        let Some(project_cwd) = self
            .project(self.threads[index].project)
            .map(|p| p.path.to_string_lossy().into_owned())
        else {
            return;
        };
        let Some(wt_path) = self.threads[index].worktree_path.clone() else {
            return;
        };
        self.threads[index].worktree_setup_seq =
            self.threads[index].worktree_setup_seq.saturating_add(1);
        let run_seq = self.threads[index].worktree_setup_seq;
        let wake = self.wake.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.worktree_setup_rxs.push(rx);
        std::thread::spawn(move || {
            let result = crate::vcs::run_setup(&crate::vcs::VcsRunSetupInput {
                cwd: project_cwd,
                worktree_path: Some(wt_path),
                timeout_ms: None,
            });
            let _ = tx.send(WorktreeSetupResultMsg {
                thread_id,
                run_seq,
                result,
            });
            wake();
        });
    }

    pub fn poll_worktree_setup(&mut self) -> bool {
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.worktree_setup_rxs.len());
        let rxs = std::mem::take(&mut self.worktree_setup_rxs);
        for rx in rxs {
            match rx.try_recv() {
                Ok(msg) => {
                    changed = true;
                    let Some(index) = self.threads.iter().position(|t| t.id == msg.thread_id)
                    else {
                        continue;
                    };
                    if self.threads[index].worktree_setup_seq != msg.run_seq {
                        continue;
                    }
                    match msg.result {
                        Ok(res) if res.status == "no_script" => {
                            self.threads[index].worktree_setup = None;
                            self.release_after_worktree_setup(index);
                        }
                        Ok(res) if res.exit_code == Some(0) && !res.timed_out => {
                            let existing = self.threads[index].worktree_setup.take();
                            self.threads[index].worktree_setup = Some(WorktreeSetupState {
                                phase: WorktreeSetupPhase::Succeeded,
                                script_name: res.script_name.or_else(|| {
                                    existing.as_ref().and_then(|s| s.script_name.clone())
                                }),
                                script_command: res.script_command.or_else(|| {
                                    existing.as_ref().and_then(|s| s.script_command.clone())
                                }),
                                error: None,
                                output: res.output,
                                exit_code: res.exit_code,
                                timed_out: false,
                                duration_ms: res.duration_ms,
                                skipped: false,
                            });
                            self.release_after_worktree_setup(index);
                        }
                        Ok(res) => {
                            let existing = self.threads[index].worktree_setup.take();
                            let error = if res.timed_out {
                                "Setup script timed out.".to_owned()
                            } else if let Some(code) = res.exit_code {
                                format!("Setup script exited with code {code}.")
                            } else {
                                "Setup script failed.".to_owned()
                            };
                            self.threads[index].worktree_setup = Some(WorktreeSetupState {
                                phase: WorktreeSetupPhase::Failed,
                                script_name: res.script_name.or_else(|| {
                                    existing.as_ref().and_then(|s| s.script_name.clone())
                                }),
                                script_command: res.script_command.or_else(|| {
                                    existing.as_ref().and_then(|s| s.script_command.clone())
                                }),
                                error: Some(error),
                                output: res.output,
                                exit_code: res.exit_code,
                                timed_out: res.timed_out,
                                duration_ms: res.duration_ms,
                                skipped: false,
                            });
                            self.threads[index].worktree_setup_output_open = true;
                            self.threads[index].native.queue_held = true;
                        }
                        Err(error) => {
                            let existing = self.threads[index].worktree_setup.take();
                            self.threads[index].worktree_setup = Some(WorktreeSetupState {
                                phase: WorktreeSetupPhase::Failed,
                                script_name: existing.as_ref().and_then(|s| s.script_name.clone()),
                                script_command: existing
                                    .as_ref()
                                    .and_then(|s| s.script_command.clone()),
                                error: Some(error),
                                output: existing.map(|s| s.output).unwrap_or_default(),
                                exit_code: None,
                                timed_out: false,
                                duration_ms: 0,
                                skipped: false,
                            });
                            self.threads[index].worktree_setup_output_open = true;
                            self.threads[index].native.queue_held = true;
                        }
                    }
                    self.checkpoint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => changed = true,
            }
        }
        self.worktree_setup_rxs = remaining;
        changed
    }

    pub fn retry_worktree_setup(&mut self) {
        if self.forward("thread.worktreeSetup", json!({"action": "retry"})) {
            return;
        }
        let thread = self.current_mut();
        let Some(setup) = thread.worktree_setup.as_mut() else {
            return;
        };
        if setup.phase != WorktreeSetupPhase::Failed || thread.worktree_path.is_none() {
            return;
        }
        setup.phase = WorktreeSetupPhase::Running;
        setup.error = None;
        setup.output.clear();
        setup.exit_code = None;
        setup.timed_out = false;
        setup.duration_ms = 0;
        setup.skipped = false;
        thread.native.queue_held = true;
        let tid = thread.id;
        self.spawn_worktree_setup_runner(tid);
        self.checkpoint();
    }

    pub fn skip_worktree_setup(&mut self) {
        if self.forward("thread.worktreeSetup", json!({"action": "skip"})) {
            return;
        }
        let index = self.active;
        let thread = &mut self.threads[index];
        let Some(setup) = thread.worktree_setup.as_mut() else {
            return;
        };
        if setup.phase == WorktreeSetupPhase::Succeeded {
            return;
        }
        thread.worktree_setup_seq = thread.worktree_setup_seq.saturating_add(1);
        setup.phase = WorktreeSetupPhase::Succeeded;
        setup.skipped = true;
        setup.error = None;
        self.release_after_worktree_setup(index);
        self.checkpoint();
    }

    pub fn open_external_url(&mut self, url: &str) {
        self.pending_open_urls.push(url.to_owned());
    }

    pub fn open_usage_limits(&mut self) -> bool {
        let machine = self.machines.selected.clone();
        let thread = self.current();
        let thread_id = thread.id;
        let harness = thread.harness.clone();
        let client_draft = thread.client_draft;
        if !self.usage_limits_command_available(thread) {
            self.push_toast(
                ToastKind::Warning,
                "Usage limits are unavailable for this provider",
                None,
                None,
                Some(5_000),
            );
            return false;
        }
        if let Some(client) = &self.client {
            let target = if machine == "local" {
                Some(client)
            } else {
                client.linked.get(&machine)
            };
            let Some(target) = target else {
                self.push_toast(
                    ToastKind::Warning,
                    "Usage limits are unavailable for this provider",
                    None,
                    None,
                    Some(5_000),
                );
                return false;
            };
            if !target.has_capability(crate::protocol::USAGE_LIMITS) {
                self.push_toast(
                    ToastKind::Warning,
                    "Usage limits are unavailable for this provider",
                    None,
                    None,
                    Some(5_000),
                );
                return false;
            }
            self.next_usage_limits_request_id += 1;
            let req_id = self.next_usage_limits_request_id;
            self.pending_usage_limits_request = Some((req_id, machine, thread_id, harness.clone()));
            let params = if client_draft {
                json!({"threadId": thread_id, "draft": true, "harness": harness})
            } else {
                json!({"threadId": thread_id})
            };
            target.call_rpc(format!("usage-limits:{req_id}"), "usage.limits", params);
            return true;
        }

        let catalog = match harness.as_str() {
            "codex" => Some(&self.codex_catalog),
            "claude" => Some(&self.claude_catalog),
            _ => None,
        };
        let hs = &thread.harness_state;
        let usage_limits = (!hs["usageLimits"].is_null())
            .then_some(&hs["usageLimits"])
            .or_else(|| catalog.map(|c| &c.usage_limits).filter(|v| !v.is_null()));
        let usage_limits_raw = (!hs["usageLimitsRaw"].is_null())
            .then_some(&hs["usageLimitsRaw"])
            .or_else(|| {
                catalog
                    .map(|c| &c.usage_limits_raw)
                    .filter(|v| !v.is_null())
            });
        let usage_account = (!hs["usageAccount"].is_null())
            .then_some(&hs["usageAccount"])
            .or_else(|| catalog.map(|c| &c.usage_account).filter(|v| !v.is_null()));
        let Some(report) = crate::usage_limits::build_usage_limits_report(
            crate::usage_limits::UsageLimitsReportInput {
                harness: &harness,
                now_ms: lifecycle::now() as i64,
                usage_limits,
                usage_limits_raw,
                usage_account,
            },
        ) else {
            self.push_toast(
                ToastKind::Warning,
                "Usage limits are unavailable for this provider",
                None,
                None,
                Some(5_000),
            );
            return false;
        };
        self.show_usage_limits_report(report);
        true
    }

    pub(crate) fn apply_usage_limits_result(
        &mut self,
        req_id: u64,
        result: Result<crate::usage_limits::UsageLimitsReport, String>,
    ) {
        let Some((pending_id, machine, thread_id, harness)) =
            self.pending_usage_limits_request.as_ref()
        else {
            return;
        };
        if *pending_id != req_id {
            return;
        }
        let matches_current = self.machines.selected == *machine
            && self.current().id == *thread_id
            && self.current().harness == *harness;
        self.pending_usage_limits_request = None;
        if !matches_current {
            return;
        }
        match result {
            Ok(report) => self.show_usage_limits_report(report),
            Err(_) => {
                self.usage_limits_notice_key = None;
                self.composer_notices.remove("usage-limits");
                self.push_toast(
                    ToastKind::Warning,
                    "Usage limits are unavailable for this provider",
                    None,
                    None,
                    Some(5_000),
                );
            }
        }
    }

    pub fn show_usage_limits_report(&mut self, report: crate::usage_limits::UsageLimitsReport) {
        let description = if report.accounts.len() == 1 {
            report
                .accounts
                .first()
                .map(crate::usage_limits::account_summary)
        } else {
            Some(format!("{} accounts", report.accounts.len()))
        };
        self.usage_limits_notice_key = Some((
            self.machines.selected.clone(),
            self.current().id,
            self.current().harness.clone(),
        ));
        self.composer_notices.remove("usage-limits");
        self.composer_notices.push(
            crate::composer_notices::Notice::new(
                "usage-limits",
                crate::composer_notices::Variant::Info,
                "alert",
                "Usage limits",
            )
            .group(crate::composer_notices::Group::Attention)
            .description(description)
            .body(crate::composer_notices::NoticeBody::UsageLimits(report))
            .dismiss_label("Dismiss usage limits"),
        );
    }

    pub fn redeem_usage_reset_credit(
        &mut self,
        input: crate::usage_limits::ProviderConsumeResetCreditInput,
    ) {
        let _ = input;
        if self.client.is_none() && self.current().harness == "codex" && self.current().pi.is_some()
        {
            let key = uuid::Uuid::new_v4().to_string();
            self.current_mut()
                .command("consume_reset_credit", json!({"idempotencyKey": key}));
            return;
        }
        self.push_toast(
            ToastKind::Error,
            "Could not use the reset credit.",
            None,
            None,
            Some(5_000),
        );
    }

    pub fn arm_fresh_session(&mut self, thread_id: usize) {
        self.fresh_arm_seq = self.fresh_arm_seq.saturating_add(1).max(1);
        let arm = self.fresh_arm_seq;
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == thread_id) {
            thread.fresh_session_arm = Some(arm);
        }
        self.sync_fresh_session_notice();
    }

    pub fn disarm_fresh_session(&mut self, thread_id: usize, expected_arm: Option<u64>) {
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == thread_id)
            && expected_arm.is_none_or(|arm| thread.fresh_session_arm == Some(arm))
        {
            thread.fresh_session_arm = None;
        }
        self.sync_fresh_session_notice();
    }

    pub fn dismiss_composer_notice(&mut self, id: &str) {
        if id == "fresh-session" {
            let thread_id = self.current().id;
            self.disarm_fresh_session(thread_id, None);
            self.composer_notices.remove(id);
            self.checkpoint();
        } else if id == "usage-limits" {
            self.usage_limits_notice_key = None;
            self.composer_notices.remove(id);
        } else {
            self.composer_notices.dismiss(id);
        }
    }

    pub fn fallback_model_candidates(&self) -> Vec<crate::model_selection::ModelSelection> {
        let entries = self.provider_instance_entries();
        let models_by_instance = self.picker_models_by_instance();
        let mut out = Vec::new();
        // Order non-default providers first so toggling on prefers an alternate provider.
        let mut ordered_entries = entries.clone();
        ordered_entries.sort_by_key(|e| e.instance_id == self.default_harness);
        for entry in ordered_entries {
            if !entry.enabled || !entry.is_available {
                continue;
            }
            if let Some(opts) = models_by_instance.get(&entry.instance_id)
                && !opts.is_empty()
            {
                for opt in opts {
                    out.push(crate::model_selection::ModelSelection {
                        instance_id: entry.instance_id.clone(),
                        model: opt.slug.clone(),
                        options: Vec::new(),
                    });
                }
            } else {
                out.push(crate::model_selection::ModelSelection {
                    instance_id: entry.instance_id.clone(),
                    model: crate::model_selection::default_model_by_provider(&entry.driver_kind)
                        .to_owned(),
                    options: Vec::new(),
                });
            }
        }
        out
    }

    fn default_model_selection_for_fallback(
        &self,
    ) -> Option<crate::model_selection::ModelSelection> {
        let harness = &self.default_harness;
        if let Some(def) = self.thread_defaults.models.get(harness)
            && !def.model.trim().is_empty()
        {
            let mut options = Vec::new();
            if !def.effort.trim().is_empty() {
                let key = if harness == "codex" {
                    "reasoningEffort"
                } else {
                    "effort"
                };
                options.push(crate::model_selection::ProviderOptionSelection {
                    id: key.into(),
                    value: crate::model_selection::OptionValue::Str(def.effort.clone()),
                });
            }
            return Some(crate::model_selection::ModelSelection {
                instance_id: harness.clone(),
                model: def.model.clone(),
                options,
            });
        }
        let models = self.harness_models(harness);
        let model = models
            .iter()
            .find(|m| m.is_default)
            .or_else(|| models.first())
            .map(|m| m.id.clone())
            .unwrap_or_else(|| {
                crate::model_selection::default_model_by_provider(harness).to_owned()
            });
        Some(crate::model_selection::ModelSelection {
            instance_id: harness.clone(),
            model,
            options: Vec::new(),
        })
    }

    fn provider_usage_windows(
        &self,
        harness: &str,
        now_ms: i64,
    ) -> Vec<crate::usage_limits::ServerProviderUsageWindow> {
        if !crate::usage_limits::has_usage_limits(harness) {
            return Vec::new();
        }
        let catalog = match harness {
            "codex" => Some(&self.codex_catalog),
            "claude" => Some(&self.claude_catalog),
            _ => None,
        };
        let thread = if self.current().harness == harness {
            Some(self.current())
        } else {
            self.threads.iter().find(|t| {
                t.harness == harness
                    && (!t.harness_state["usageLimitsRaw"].is_null()
                        || !t.harness_state["usageLimits"].is_null())
            })
        };
        let hs = thread.map(|t| &t.harness_state);
        let usage_limits = hs
            .map(|h| &h["usageLimits"])
            .filter(|v| !v.is_null())
            .or_else(|| catalog.map(|c| &c.usage_limits).filter(|v| !v.is_null()));
        let usage_limits_raw = hs
            .map(|h| &h["usageLimitsRaw"])
            .filter(|v| !v.is_null())
            .or_else(|| {
                catalog
                    .map(|c| &c.usage_limits_raw)
                    .filter(|v| !v.is_null())
            });
        let usage_account = hs
            .map(|h| &h["usageAccount"])
            .filter(|v| !v.is_null())
            .or_else(|| catalog.map(|c| &c.usage_account).filter(|v| !v.is_null()));
        crate::usage_limits::build_usage_limits_report(
            crate::usage_limits::UsageLimitsReportInput {
                harness,
                now_ms,
                usage_limits,
                usage_limits_raw,
                usage_account,
            },
        )
        .and_then(|r| r.accounts.into_iter().next())
        .map(|a| a.limits.windows)
        .unwrap_or_default()
    }

    fn friendly_model_name(&self, harness: &str, model_id: &str) -> String {
        if self.current().harness == harness
            && let Some(m) = self.current().models.iter().find(|m| m.id == model_id)
        {
            return m.name.clone();
        }
        self.harness_models(harness)
            .into_iter()
            .find(|m| m.id == model_id)
            .map(|m| m.name)
            .unwrap_or_else(|| model_id.to_owned())
    }

    fn apply_selection_to_thread(
        &mut self,
        index: usize,
        selection: &crate::model_selection::ModelSelection,
    ) {
        let prev_active = self.active;
        self.active = index;
        if self.threads[index].harness != selection.instance_id {
            self.choose_harness(&selection.instance_id);
        }
        let provider = if selection.instance_id == "pi" {
            "cpa".to_owned()
        } else {
            selection.instance_id.clone()
        };
        let models = self.harness_models(&selection.instance_id);
        if self.threads[index].models.is_empty() && !models.is_empty() {
            self.threads[index].models = models;
        }
        self.threads[index].set_model(provider, selection.model.clone());
        for opt in &selection.options {
            self.threads[index].set_option(opt.id.clone(), opt.value.clone());
        }
        self.reconcile_current_access_mode();
        self.active = prev_active;
    }

    pub fn use_default_from_fallback(&mut self) {
        let Some(info) = self.current().fallback_active.clone() else {
            return;
        };
        self.current_mut().fallback_active = None;
        self.current_mut().fallback_explicit_override = true;
        self.apply_selection_to_thread(self.active, &info.default_selection);
        self.composer_notices.sync("fallback-model", None);
        self.checkpoint();
    }

    /// Evaluates KSG's new-thread fallback rule for the active thread and syncs the
    /// `fallback-model` composer notice.
    pub(crate) fn sync_fallback_model(&mut self, now_ms: i64) -> bool {
        let t = self.current();
        let eligible = !self.demo
            && t.messages.is_empty()
            && t.life.latest_user_message_at.is_none()
            && t.session.is_none()
            && !t.fallback_explicit_override;
        if !eligible {
            self.current_mut().fallback_active = None;
            return self.composer_notices.sync("fallback-model", None);
        }
        let Some(fallback_sel) = self.thread_defaults.fallback_model_selection.clone() else {
            if let Some(prev) = self.current_mut().fallback_active.take() {
                self.apply_selection_to_thread(self.active, &prev.default_selection);
            }
            return self.composer_notices.sync("fallback-model", None);
        };
        let default_sel = self
            .current()
            .fallback_active
            .as_ref()
            .map(|f| f.default_selection.clone())
            .or_else(|| self.default_model_selection_for_fallback());
        let entries = self.provider_instance_entries();
        let windows_by_id: Vec<(String, Vec<crate::usage_limits::ServerProviderUsageWindow>)> =
            entries
                .iter()
                .map(|e| {
                    (
                        e.instance_id.clone(),
                        self.provider_usage_windows(&e.instance_id, now_ms),
                    )
                })
                .collect();
        let candidates: Vec<crate::usage_limits::ProviderFallbackCandidate<'_>> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| crate::usage_limits::ProviderFallbackCandidate {
                instance_id: &e.instance_id,
                driver_kind: &e.driver_kind,
                display_name: &e.display_name,
                enabled: e.enabled,
                installed: e.installed,
                authenticated: !matches!(
                    e.auth_status,
                    crate::model_picker::ProviderAuthStatus::Unauthenticated
                ),
                is_available: e.is_available,
                windows: &windows_by_id[i].1,
            })
            .collect();
        let res = crate::usage_limits::resolve_new_thread_model_selection(
            default_sel.as_ref(),
            None,
            Some(&fallback_sel),
            &candidates,
            now_ms,
        );
        if let (Some(target_sel), Some(info)) = (res.selection, res.fallback) {
            let needs_apply = self.current().fallback_active.as_ref() != Some(&info)
                || self.current().harness != target_sel.instance_id
                || self.current().model_id != target_sel.model;
            if needs_apply {
                self.apply_selection_to_thread(self.active, &target_sel);
                self.current_mut().fallback_active = Some(info.clone());
            }
            let fallback_model_name =
                self.friendly_model_name(&target_sel.instance_id, &target_sel.model);
            let default_model_name = self.friendly_model_name(
                &info.default_selection.instance_id,
                &info.default_selection.model,
            );
            let resets_suffix = info
                .resets_at
                .as_deref()
                .and_then(crate::usage_limits::format_reset_clock_iso)
                .map(|clock| format!(" · resets {clock}"))
                .unwrap_or_default();
            let notice = crate::composer_notices::Notice::new(
                "fallback-model",
                crate::composer_notices::Variant::Info,
                "alert",
                format!("Using {fallback_model_name}"),
            )
            .group(crate::composer_notices::Group::Attention)
            .description(Some(format!(
                "{} is out of usage{resets_suffix}",
                info.default_provider_name
            )))
            .actions(vec![crate::composer_notices::NoticeAction::new(
                format!("Use {default_model_name}"),
                |s| s.use_default_from_fallback(),
            )])
            .dismiss_label("Dismiss fallback model notice");
            self.composer_notices.sync("fallback-model", Some(notice))
        } else {
            if let Some(prev) = self.current_mut().fallback_active.take() {
                self.apply_selection_to_thread(self.active, &prev.default_selection);
            }
            self.composer_notices.sync("fallback-model", None)
        }
    }

    pub(crate) fn sync_fresh_session_notice(&mut self) -> bool {
        let t = self.current();
        let show =
            self.fresh_session_available(t) && t.fresh_session_arm.is_some() && t.request.is_none();
        let notice = show.then(|| {
            crate::composer_notices::Notice::new(
                "fresh-session",
                crate::composer_notices::Variant::Info,
                "alert",
                "Your next message starts a fresh session",
            )
            .group(crate::composer_notices::Group::Attention)
            .description(Some(
                "The agent won't see earlier messages in this thread.".into(),
            ))
            .dismiss_label("Keep the current session")
        });
        self.composer_notices.sync("fresh-session", notice)
    }

    pub(crate) fn keep_full_history(&self, thread: &Thread) -> bool {
        self.keep_full_history_threads
            .contains(&(self.machines.selected.clone(), thread.id))
    }

    pub(crate) fn set_keep_full_history(&mut self, server: &str, thread_id: usize, keep: bool) {
        let key = (server.to_owned(), thread_id);
        if keep {
            self.keep_full_history_threads.insert(key);
        } else {
            self.keep_full_history_threads.remove(&key);
        }
    }

    pub fn toggle_keep_full_history(&mut self) {
        let server = self.machines.selected.clone();
        let thread_id = self.current().id;
        let next = !self.keep_full_history(self.current());
        self.set_keep_full_history(&server, thread_id, next);
    }

    /// Ported from KSG Code's `compactBeforeSendTokens` / `resumeCompactionTokens`
    /// (`packages/client-runtime/src/state/contextWindow.ts` and `ChatView.tsx`).
    pub(crate) fn resume_compaction_tokens(&self, thread: &Thread, now_ms: u64) -> Option<u64> {
        if self.demo {
            return None;
        }
        if self.fresh_session_available(thread) && thread.fresh_session_arm.is_some() {
            return None;
        }
        if thread.archived()
            || thread.status.busy()
            || thread.initializing
            || thread.configuring
            || thread.native.operating()
            || thread.context.phase == crate::context::ContextPhase::Compacting
            || thread.native.queue_held
            || thread.request.is_some()
            || thread.native.questionnaire.is_some()
            || !crate::harness::supports_compaction(&thread.harness)
        {
            return None;
        }
        let has_prior_turn = thread.session.is_some()
            || thread.messages.iter().any(|m| {
                m.role == "You"
                    && (!m.text.trim().eq_ignore_ascii_case("/compact") || !m.images.is_empty())
            });
        if !has_prior_turn {
            return None;
        }
        let used_tokens = thread.context.usage.tokens?;
        crate::context::should_offer_resume_compaction(crate::context::ResumeCompactionInput {
            harness: &thread.harness,
            used_tokens: Some(used_tokens),
            updated_at_ms: thread.context.updated_at,
            prompt_cache_ttl_ms: thread.context.prompt_cache_ttl_ms,
            now_ms,
        })
        .then_some(used_tokens)
    }

    pub fn prompt_suggestions_enabled(&self) -> bool {
        if self.client.is_some() {
            self.machines
                .views
                .get(&self.machines.selected)
                .and_then(|v| v["index"]["threadDefaults"]["prompt_suggestions_enabled"].as_bool())
                .unwrap_or(self.thread_defaults.prompt_suggestions_enabled)
        } else {
            self.thread_defaults.prompt_suggestions_enabled
        }
    }

    /// Ported condition by condition from KSG Code's `usePromptSuggestion.ts` and `ChatComposer.tsx`.
    pub fn offered_prompt_suggestion(&self) -> Option<&str> {
        if !self.prompt_suggestions_enabled()
            || self.demo
            || self.ownership_denied
            || self.unreachable_selected().is_some()
        {
            return None;
        }
        let t = self.current();
        if t.delegated
            || t.archived()
            || t.offline_blocked
            || t.status != Status::Completed
            || t.configuring
            || t.initializing
            || t.phase == "Stopping"
            || t.remote_send.is_some()
            || t.client_creating.is_some()
            || t.queued_edit.is_some()
            || t.request.is_some()
            || t.native.questionnaire.is_some()
            || !t.draft.text.is_empty()
            || t.draft.is_composing()
            || t.draft.pills().next().is_some()
            || !t.draft_images.is_empty()
            || !t.draft_context.is_empty()
            || t.draft_reply_to.is_some()
            || !crate::harness::supports_prompt_suggestions(&t.harness)
        {
            return None;
        }
        let run_id = t.run_id.as_deref()?;
        let suggestion = t.prompt_suggestion.as_ref()?;
        if suggestion.run_id != run_id || suggestion.text.trim().is_empty() {
            return None;
        }
        if self
            .dismissed_prompt_suggestions
            .contains(&(t.machine.clone(), t.id, run_id.to_owned()))
        {
            return None;
        }
        Some(suggestion.text.as_str())
    }

    pub fn dismiss_prompt_suggestion(&mut self) -> bool {
        if self.offered_prompt_suggestion().is_none() {
            return false;
        }
        let machine = self.current().machine.clone();
        let thread_id = self.current().id;
        if let Some(run_id) = self.current().run_id.clone() {
            self.dismissed_prompt_suggestions
                .insert((machine, thread_id, run_id));
            return true;
        }
        false
    }

    pub fn accept_prompt_suggestion(&mut self) -> bool {
        let Some(text) = self.offered_prompt_suggestion().map(str::to_owned) else {
            return false;
        };
        let t = self.current_mut();
        t.draft.anchor = 0;
        t.draft.focus = 0;
        t.draft.insert(&text);
        t.native.slash_index = 0;
        t.composer_rules.changed(&mut t.draft);
        t.reconcile_draft_context();
        t.typing_guard.on_type(std::time::Instant::now());
        t.update_submission_error();
        self.ensure_composer_commands();
        self.checkpoint();
        true
    }

    fn dispatch_send(&mut self) -> bool {
        self.dispatch_send_full(false, false, None, None)
    }

    pub(crate) fn dispatch_send_with_options(
        &mut self,
        forced_fresh_context: Option<bool>,
        forced_compact_before_send: Option<bool>,
    ) -> bool {
        self.dispatch_send_full(
            false,
            false,
            forced_fresh_context,
            forced_compact_before_send,
        )
    }

    fn dispatch_send_intent(&mut self, background: bool, approval_redirect: bool) -> bool {
        self.dispatch_send_full(background, approval_redirect, None, None)
    }

    fn dispatch_send_full(
        &mut self,
        background: bool,
        approval_redirect: bool,
        forced_fresh_context: Option<bool>,
        forced_compact_before_send: Option<bool>,
    ) -> bool {
        if self.demo || self.ownership_denied || self.current().archived() {
            return false;
        }
        if self.has_pending_context_imports(&self.machines.selected, self.current().id) {
            self.current_mut().attachment_error = Some(
                "Still bringing attached context into this message. Send again once its chip resolves."
                    .into(),
            );
            return false;
        }
        if self.current().resume_pending() || self.current().responding_request.is_some() {
            return false;
        }
        if self.current().queued_edit.is_some() {
            let target_server_id = self.machine_identity(&self.machines.selected);
            let peers = self.peers.clone();
            let labels: Vec<(String, String)> = peers
                .iter()
                .map(|p| (p.server_id.clone(), p.label.clone()))
                .collect();
            self.current_mut()
                .save_queued_edit_with_peers(Some(&target_server_id), &peers, &|m| {
                    if m == "local" {
                        "Local".into()
                    } else {
                        labels
                            .iter()
                            .find(|(id, _)| id == m)
                            .map(|(_, l)| l.clone())
                            .unwrap_or_else(|| m.to_owned())
                    }
                });
            self.checkpoint();
            return false;
        }
        if self.current().approval_send.is_some() {
            return false;
        }
        if !self.headless
            && !self.current().request_held_by_typing_guard()
            && (self.current().native.questionnaire.is_some()
                || self
                    .current()
                    .request
                    .as_ref()
                    .is_some_and(|r| r["method"] != "confirm"))
        {
            return false;
        }
        if self.current().request.is_some() {
            if self.current().draft.text.trim() == "/model" {
                self.choose_slash("model".into());
                return false;
            }
            if self.client.is_none() && self.handle_composer_local_command() {
                return false;
            }
            if !self.approval_send_is_local_command()
                && !approval_redirect
                && !self.headless
                && !self.current().request_held_by_typing_guard()
            {
                if let Some(error) = self.approval_send_command_error() {
                    self.current_mut().error = Some(error);
                    return false;
                }
                return self.begin_approval_send(background);
            }
        }
        self.reconcile_current_access_mode();
        if !self
            .current()
            .draft_model_state
            .additional_models
            .is_empty()
        {
            return self.dispatch_multi_model_send();
        }
        let fresh_session_available = self.fresh_session_available(self.current());
        let draft_markdown = crate::composer_rich::markdown(&self.current().draft);
        let composer_holds_only_text = self.current().draft_images.is_empty()
            && self.current().draft_context.is_empty()
            && self.current().draft_reply_to.is_none()
            && self.current().draft.pills().next().is_none();
        if forced_fresh_context.is_none()
            && fresh_session_available
            && composer_holds_only_text
            && is_clear_command(&draft_markdown)
        {
            let thread_id = self.current().id;
            self.arm_fresh_session(thread_id);
            let t = self.current_mut();
            t.prompt_history.reset();
            t.typing_guard.on_blur_or_send();
            t.paste_history = Default::default();
            t.draft.replace_text("");
            t.submission_error = None;
            t.attachment_error = None;
            self.checkpoint();
            return false;
        }
        if composer_holds_only_text
            && self.usage_limits_command_available(self.current())
            && crate::usage_limits::is_usage_limits_command(&draft_markdown)
        {
            if self.open_usage_limits() {
                let t = self.current_mut();
                t.prompt_history.reset();
                t.typing_guard.on_blur_or_send();
                t.paste_history = Default::default();
                t.draft.replace_text("");
                t.submission_error = None;
                t.attachment_error = None;
                self.checkpoint();
            }
            return false;
        }
        // Read once: an arm made while this send is in flight belongs to the next message.
        let fresh_arm = if forced_fresh_context.is_some() {
            None
        } else if fresh_session_available {
            self.current().fresh_session_arm
        } else {
            None
        };
        let fresh_context = forced_fresh_context.unwrap_or(fresh_arm.is_some());
        let compact_before_send = forced_compact_before_send.unwrap_or_else(|| {
            !fresh_context
                && self
                    .resume_compaction_tokens(self.current(), lifecycle::now())
                    .is_some()
                && !self.keep_full_history(self.current())
                && !is_compact_command(&draft_markdown)
        });
        if self.client.is_some() {
            if self.current().draft.text.trim() == "/model" {
                self.choose_slash("model".into());
                return false;
            }
            if !self.current().has_draft_content() {
                return false;
            }
            if !self.machine_connected(&self.machines.selected) {
                self.push_toast(
                    ToastKind::Warning,
                    "Not connected: message not sent",
                    Some("Reconnecting to the environment. Try again once it is connected.".into()),
                    None,
                    Some(5_000),
                );
                return false;
            }
            if self.current().remote_send.is_some() {
                return false;
            }
            // KSG snapshots and clears at dispatch. A refusal merges it with later edits.
            let revision = uuid::Uuid::new_v4();
            let target_machine = self.machines.selected.clone();
            let target_server_id = self.machine_identity(&target_machine);
            let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
                &self.current().draft,
                &self.current().draft_images,
                &self.current().draft_context,
                self.current().draft_reply_to.as_ref(),
                Some(&target_machine),
                Some(&target_server_id),
                &self.peers,
                &|machine| self.machine_label(machine),
            ) {
                Ok(prompt) => prompt,
                Err(error) => {
                    self.current_mut().attachment_error = Some(error);
                    return false;
                }
            };
            if !self.headless
                && let Some(error) = self.current().validate_prompt_length(&prompt)
            {
                self.current_mut().submission_error = Some(error);
                return false;
            }
            if let Some(error) = self.missing_worktree_base_branch_error(self.current()) {
                self.current_mut().submission_error = Some(error);
                return false;
            }
            self.current_mut().submission_error = None;
            if self.current().client_draft {
                self.promote_client_draft();
                return self.current().client_creating.is_some();
            }
            let supports_fresh_cap = self.client.as_ref().is_some_and(|c| {
                c.supports_capability(&self.current().machine, crate::protocol::FRESH_CONTEXT)
            });
            let supports_vcs_cap = self.client.as_ref().is_some_and(|c| {
                c.supports_capability(&self.current().machine, crate::protocol::VCS_WORKTREES)
            });
            let mut mode = if (self.current().status.busy()
                && (self.current().follow_up_queues() || fresh_context))
                || self.current().native.queue_held
            {
                "queue"
            } else if self.current().status.busy() {
                "steer"
            } else {
                "prompt"
            };
            let mut params = json!({"threadId":self.current().id,"text":prompt.text,
                "attachments":prompt.images.as_ref(),"context":prompt.context,"draft":prompt.draft,"editorRevision":revision,"mode":mode});
            let sending_worktree_bootstrap = supports_vcs_cap
                && self.current().worktree_path.is_none()
                && self.current().messages.is_empty()
                && self.current().project.is_some()
                && !self.is_captain_home(self.current())
                && self.effective_env_mode(self.current()) == ThreadEnvMode::Worktree;
            if sending_worktree_bootstrap {
                params["envMode"] = json!(ThreadEnvMode::Worktree);
                if let Some(base) = &self.current().base_branch {
                    params["baseBranch"] = json!(base);
                }
                params["startFromOrigin"] = json!(self.current().effective_start_from_origin());
            }
            if fresh_context {
                params["freshContext"] = json!(true);
            }
            if compact_before_send {
                if supports_fresh_cap {
                    params["compactBeforeSend"] = json!(true);
                } else {
                    let _ = self.remote("thread.compact", json!({"instructions": Value::Null}));
                    mode = "queue";
                    params["mode"] = json!(mode);
                }
            }
            let result = self.remote("thread.send", params);
            return match result {
                Some(Ok(id)) => {
                    let snapshot = crate::composer_rich::Snapshot::capture_with_context(
                        &self.current().draft,
                        &self.current().draft_images,
                        &self.current().draft_context,
                    );
                    let thread = self.current_mut();
                    let reply_for_remote = thread.draft_reply_to.clone();
                    if sending_worktree_bootstrap {
                        thread.remote_bootstrap_snapshot =
                            Some((snapshot.clone(), reply_for_remote));
                    }
                    thread.remote_send = Some((id, snapshot));
                    thread.remote_send_fresh_arm = fresh_arm;
                    thread.remote_send_reply_to = thread.draft_reply_to.take();
                    thread.draft.replace_text("");
                    thread.draft_images.clear();
                    thread.draft_context.clear();
                    thread.retained_context = Default::default();
                    thread.typing_guard.on_blur_or_send();
                    self.usage_limits_notice_key = None;
                    self.composer_notices.remove("usage-limits");
                    true
                }
                Some(Err(error)) => {
                    self.storage_error = Some(error);
                    false
                }
                None => false,
            };
        }
        if self.network_blocked() {
            self.current_mut().error =
                Some("Waiting for the network after resume. Your draft has not been sent.".into());
            return false;
        }
        if self.current().status == Status::Paused {
            self.current_mut().status = Status::Ready;
            self.current_mut().error = None;
        }
        if self.current().configuring
            || self.current().native.operating()
            || self.current().phase == "Stopping"
            || !self.current().has_draft_content()
        {
            return false;
        }
        let text = draft_markdown.trim().to_owned();
        let mut words = text.splitn(2, char::is_whitespace);
        match words.next().filter(|_| {
            !self
                .current()
                .draft
                .pills()
                .any(|(r, _)| self.current().draft.text[..r.start].trim().is_empty())
                && !crate::composer_tokens::starts_with_file_path(
                    &text,
                    &self.current().draft_images,
                )
        }) {
            Some("/model") => {
                self.choose_slash("model".into());
                return false;
            }
            Some(command) if command.eq_ignore_ascii_case("/clear") => {}
            Some(command)
                if command.eq_ignore_ascii_case("/usage-limits")
                    && self.usage_limits_command_available(self.current()) => {}
            Some("/compact") if crate::harness::supports_compaction(&self.current().harness) => {
                if !self.current().draft_images.is_empty() {
                    self.current_mut().attachment_error =
                        Some("Compaction does not accept attachments. Your draft was kept.".into());
                    return false;
                }
                if self.current().can_compact() {
                    let instructions = words
                        .next()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    self.current_mut().draft.replace_text("");
                    self.compact_context(instructions);
                }
                return false;
            }
            Some(command @ ("/goal" | "/feedback")) if self.current().harness == "codex" => {
                if self.current().harness_state["codexNativeExtensions"] != true {
                    self.current_mut().error = Some("This Codex version has no verified native command protocol. Update Codex outside Shina.".into());
                    return false;
                }
                let arguments = words.next().unwrap_or("").trim().to_owned();
                if self.current().status.busy()
                    && command == "/goal"
                    && !matches!(arguments.as_str(), "" | "pause" | "clear")
                {
                    self.current_mut().error = Some("Wait for Codex to finish before running this command. Your draft was kept.".into());
                    return false;
                }
                if !self.current().draft_images.is_empty() {
                    self.current_mut().attachment_error = Some(
                        "Native commands do not accept attachments. Your draft was kept.".into(),
                    );
                    return false;
                }
                self.current_mut().native.pending_command =
                    Some(json!({"name":command.trim_start_matches('/'),"arguments":arguments}));
                self.current_mut().draft.replace_text("");
                if self.current().pi.is_none() {
                    self.connect_thread(self.active);
                } else {
                    self.current_mut().drive_session();
                }
                return false;
            }
            Some(command) if command.starts_with('/') => {
                let name = command.trim_start_matches('/');
                if !self
                    .current()
                    .native
                    .commands
                    .iter()
                    .any(|c| c.name == name)
                {
                    self.current_mut().error = Some(format!(
                        "Unknown {} command {command}. Use / to browse the workspace's commands.",
                        if self.current().harness == "pi" {
                            "Pi"
                        } else {
                            "ACP"
                        }
                    ));
                    return false;
                }
            }
            _ => {}
        }
        if self
            .current()
            .request
            .as_ref()
            .is_some_and(crate::approval::not_resumable)
        {
            self.current_mut().request = None;
        }
        self.release_task_results(self.current().id);
        if let Some(sent) = self.try_worktree_bootstrap_send(fresh_context, fresh_arm) {
            return sent;
        }
        if compact_before_send && !self.current().status.busy() {
            if self.current().harness == "pi"
                && !self.current().model_provider.is_empty()
                && self.current().model_provider != "cpa"
            {
                self.current_mut().error = Some(
                    "Choose a CPA model before sending. Shina will not send prompts to a direct provider.".into(),
                );
                return false;
            }
            if self.current().draft_images.iter().any(Attachment::is_image)
                && self.current().supports_images == Some(false)
            {
                let err = self.current().image_model_error();
                self.current_mut().attachment_error = Some(err);
                return false;
            }
            let local_server_id = self.machine_identity(&self.machines.selected);
            let peers = self.peers.clone();
            let labels: Vec<(String, String)> = peers
                .iter()
                .map(|p| (p.server_id.clone(), p.label.clone()))
                .collect();
            let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
                &self.current().draft,
                &self.current().draft_images,
                &self.current().draft_context,
                self.current().draft_reply_to.as_ref(),
                Some(&self.current().machine),
                Some(&local_server_id),
                &peers,
                &|m| {
                    if m == "local" {
                        "Local".into()
                    } else {
                        labels
                            .iter()
                            .find(|(id, _)| id == m)
                            .map(|(_, l)| l.clone())
                            .unwrap_or_else(|| m.to_owned())
                    }
                },
            ) {
                Ok(prompt) => prompt,
                Err(error) => {
                    self.current_mut().attachment_error = Some(error);
                    return false;
                }
            };
            if !self.headless
                && let Some(error) = self.current().validate_prompt_length(&prompt)
            {
                self.current_mut().submission_error = Some(error);
                return false;
            }
            self.current_mut().submission_error = None;
            let thread = self.current_mut();
            thread.prompt_history.reset();
            thread.typing_guard.on_blur_or_send();
            thread.paste_history = Default::default();
            thread.draft.replace_text("");
            thread.draft_images.clear();
            thread.draft_context.clear();
            thread.draft_reply_to = None;
            thread.retained_context = Default::default();
            thread.native.queue_held = false;
            thread.serial += 1;
            let queued_id = thread.serial;
            thread.native.queue.push_back(pi_session::QueuedMessage {
                id: queued_id,
                prompt,
            });
            thread.life.user_sent(lifecycle::now());
            let id = thread.id;
            let server = self.machines.selected.clone();
            self.set_keep_full_history(&server, id, false);
            self.invalidate_undo(id);
            self.compact_context(None);
            self.checkpoint();
            return true;
        }
        if self.current().status.busy() {
            if !self.headless {
                if let Ok(prompt) = crate::composer_tokens::prompt_for_target_with_reply(
                    &self.current().draft,
                    &self.current().draft_images,
                    &self.current().draft_context,
                    self.current().draft_reply_to.as_ref(),
                    Some(&self.current().machine),
                    Some(&self.machine_identity(&self.machines.selected)),
                    &self.peers,
                    &|machine| self.machine_label(machine),
                ) && let Some(error) = self.current().validate_prompt_length(&prompt)
                {
                    self.current_mut().submission_error = Some(error);
                    return false;
                }
                self.current_mut().submission_error = None;
            }
            let local_server_id = self.machine_identity(&self.machines.selected);
            let peers = self.peers.clone();
            let labels: Vec<(String, String)> = peers
                .iter()
                .map(|p| (p.server_id.clone(), p.label.clone()))
                .collect();
            self.current_mut().send_during_run_with_fresh(
                Some(&local_server_id),
                &peers,
                &|m| {
                    if m == "local" {
                        "Local".into()
                    } else {
                        labels
                            .iter()
                            .find(|(id, _)| id == m)
                            .map(|(_, l)| l.clone())
                            .unwrap_or_else(|| m.to_owned())
                    }
                },
                fresh_context,
            );
            let dispatched = !self.current().has_draft_content();
            if dispatched {
                if fresh_context {
                    self.persist_transcripts = true;
                }
                let thread_id = self.current().id;
                let server = self.machines.selected.clone();
                self.set_keep_full_history(&server, thread_id, false);
                if let Some(arm) = fresh_arm {
                    self.disarm_fresh_session(thread_id, Some(arm));
                }
                self.checkpoint();
            }
            return dispatched;
        }
        if !self.headless {
            if let Ok(prompt) = crate::composer_tokens::prompt_for_target_with_reply(
                &self.current().draft,
                &self.current().draft_images,
                &self.current().draft_context,
                self.current().draft_reply_to.as_ref(),
                Some(&self.current().machine),
                Some(&self.machine_identity(&self.machines.selected)),
                &self.peers,
                &|machine| self.machine_label(machine),
            ) && let Some(error) = self.current().validate_prompt_length(&prompt)
            {
                self.current_mut().submission_error = Some(error);
                return false;
            }
            self.current_mut().submission_error = None;
        }
        self.usage_limits_notice_key = None;
        self.composer_notices.remove("usage-limits");
        let local_server_id = self.machine_identity(&self.machines.selected);
        let peers = self.peers.clone();
        let labels: Vec<(String, String)> = peers
            .iter()
            .map(|p| (p.server_id.clone(), p.label.clone()))
            .collect();
        self.current_mut().send_with_fresh(
            Some(&local_server_id),
            &peers,
            &|m| {
                if m == "local" {
                    "Local".into()
                } else {
                    labels
                        .iter()
                        .find(|(id, _)| id == m)
                        .map(|(_, l)| l.clone())
                        .unwrap_or_else(|| m.to_owned())
                }
            },
            fresh_context,
        );
        if self.current().has_draft_content() || self.current().status != Status::Running {
            return false;
        }
        let id = self.current().id;
        let server = self.machines.selected.clone();
        self.set_keep_full_history(&server, id, false);
        self.invalidate_undo(id);
        if fresh_context {
            self.persist_transcripts = true;
        }
        if let Some(arm) = fresh_arm {
            self.disarm_fresh_session(id, Some(arm));
        }
        if self.current().pi.is_none() && self.current().queued_prompt.is_some() {
            self.connect_thread(self.active);
        }
        self.checkpoint();
        true
    }

    pub fn retry(&mut self) {
        if self.forward("thread.retry", json!({"threadId":self.current().id})) {
            return;
        }
        if self.demo
            || self.network_blocked()
            || self.current().archived()
            || self.current().status.busy()
        {
            return;
        }
        if !self.current().has_draft_content()
            && let Some(prompt) = self.current().retry_prompt.clone()
        {
            self.current_mut().restore_prompt(&prompt);
        }
        if !self.current().has_draft_content() {
            self.current_mut().disconnect();
            self.current_mut().error = None;
            self.current_mut().status = Status::Ready;
            self.connect_thread(self.active);
        } else {
            self.send();
        }
    }

    pub fn refresh_catalog(&mut self) {
        if self.forward("catalog.refresh", json!({})) {
            return;
        }
        self.harnesses = crate::harness::Registry::read();
        self.refresh_native_catalogs(true);
        if matches!(self.current().harness.as_str(), "codex" | "claude")
            && self.current().pi.is_some()
        {
            self.current_mut()
                .command("get_available_models", json!({}));
            self.current_mut().refresh_commands();
        }
        if !self.demo && self.runtime_enabled {
            self.catalog
                .refresh(self.data_dir.clone(), self.wake.clone());
        }
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
    }

    pub fn supports_model_catalog_refresh(&self) -> bool {
        self.client.as_ref().is_none_or(|client| {
            client.supports_capability(
                &self.machines.selected,
                crate::protocol::MODEL_CATALOG_REFRESH,
            )
        })
    }

    /// Ported from KSG Code's `ProviderModelPicker.tsx`: opening the picker refreshes the model
    /// catalog through the capability-gated `modelCatalogOnly` path, throttled by 5 minutes.
    pub fn refresh_catalog_on_open(&mut self) {
        if self.client.is_some() {
            if !self.supports_model_catalog_refresh() {
                return;
            }
            self.forward("catalog.refresh", json!({"modelCatalogOnly": true}));
            return;
        }
        self.harnesses = crate::harness::Registry::read();
        self.refresh_native_catalogs(false);
        if !self.demo && self.runtime_enabled {
            self.catalog
                .refresh_with_mode(self.data_dir.clone(), self.wake.clone(), false);
        }
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
    }

    /// Explicit retry button bypasses the on-demand throttle and retry backoff.
    pub fn force_refresh_catalog(&mut self) {
        self.catalog.error = None;
        if self.forward("catalog.refresh", json!({"force": true})) {
            return;
        }
        self.harnesses = crate::harness::Registry::read();
        self.refresh_native_catalogs(true);
        if matches!(self.current().harness.as_str(), "codex" | "claude")
            && self.current().pi.is_some()
        {
            self.current_mut()
                .command("get_available_models", json!({}));
            self.current_mut().refresh_commands();
        }
        if !self.demo && self.runtime_enabled {
            self.catalog
                .refresh_with_mode(self.data_dir.clone(), self.wake.clone(), true);
        }
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
    }

    /// Warm metadata on a command/skill trigger without sending a prompt. A failed startup
    /// or denied login stays failed until the user takes an explicit recovery action.
    pub fn ensure_composer_commands(&mut self) {
        if self.demo || !self.runtime_enabled && self.client.is_none() {
            return;
        }
        let t = self.current();
        if !crate::composer::trigger(&t.draft).is_some_and(|t| matches!(t.marker, '/' | '$'))
            || t.native.commands_loaded
            || t.native.commands_requested
            || t.native.commands_error.is_some()
            || t.initializing
            || t.error.is_some()
            || t.status == Status::Failed
            || t.archived()
        {
            return;
        }
        self.current_mut().native.commands_requested = true;
        if self.client.is_some() || self.current().pi.is_some() {
            self.current_mut().refresh_commands();
        } else {
            self.connect_thread(self.active);
        }
    }

    /// Changing a new thread's harness retires only its unused prewarmed process.
    pub fn choose_harness(&mut self, id: &str) {
        if self.forward(
            "thread.configure",
            json!({"threadId":self.current().id,"harness":id}),
        ) {
            return;
        }
        if !self.current().can_choose_harness() || self.current().harness == id {
            return;
        }
        if let Err(error) = self.validate_harness(id) {
            self.current_mut().error = Some(error);
            return;
        }
        let thread = self.current_mut();
        thread.disconnect();
        thread.harness = id.into();
        thread.harness_context_sent = false;
        thread.harness_state = json!({});
        thread.session = None;
        thread.selected_model = None;
        thread.selected_thinking = None;
        thread.model_provider.clear();
        thread.model_id.clear();
        thread.model = DEFAULT_MODEL.into();
        thread.thinking.clear();
        thread.models.clear();
        thread.thinking_levels.clear();
        thread.context_window = None;
        thread.supports_images = None;
        thread.context.begin_model_change();
        thread.native.commands.clear();
        thread.native.skills.clear();
        thread.native.commands_requested = false;
        thread.native.commands_loaded = false;
        thread.error = None;
        thread.prewarm_attempted = false;
        let access = self.effective_access_mode(self.current());
        self.current_mut().native.access = access;
        if id == "pi" {
            self.apply_catalog();
        } else {
            self.apply_native_catalogs();
        }
        self.checkpoint();
        self.refresh_native_catalogs(false);
    }

    fn apply_catalog(&mut self) {
        for thread in &mut self.threads {
            if thread.harness != "pi" {
                continue;
            }
            thread.models = self.catalog.models.clone();
            if thread.model_id.is_empty()
                && thread.session.is_none()
                && let Some(model) = &self.catalog.default_model
            {
                thread.model_provider = model.provider.clone();
                thread.model_id = model.id.clone();
                thread.model = model.name.clone();
                thread.context_window = model.context_window;
                thread.supports_images = model.supports_images;
                if thread.selected_thinking.is_none() {
                    thread.thinking = self.catalog.default_thinking.clone();
                }
            }
            let metadata = thread
                .models
                .iter()
                .find(|m| m.id == thread.model_id)
                .map(|m| (m.name.clone(), m.context_window, m.supports_images));
            if let Some((name, window, images)) = metadata {
                thread.model = name;
                if window.is_some() {
                    thread.context_window = window;
                }
                if images.is_some() {
                    thread.supports_images = images;
                }
            }
            thread.refresh_thinking_levels();
        }
    }

    fn prewarm_current(&mut self) -> bool {
        let thread = self.current();
        if self.remote_task(thread.id).is_some() {
            return false;
        }
        if self.runtime_enabled
            && !self.demo
            && !thread.archived()
            && thread.pi.is_none()
            && !thread.prewarm_attempted
            && thread.status != Status::Failed
            && !(self.effective_env_mode(thread) == ThreadEnvMode::Worktree
                && thread.worktree_path.is_none())
            && (thread.has_draft_content() || (thread.session.is_some() && !thread.history_loaded))
        {
            self.connect_thread(self.active);
            return true;
        }
        false
    }
    pub fn add_project(&mut self, path: PathBuf) -> Result<Option<ProjectId>, String> {
        let (purpose, captain) = self.project_selection_purpose();
        if let Some(result) = self.remote(
            "project.put",
            json!({"path":path,"selectFor":purpose,"captain":captain}),
        ) {
            result?;
            return Ok(None);
        }
        if !path.is_absolute() {
            return Err("A workspace needs an absolute path on this server.".into());
        }
        let path = match path.canonicalize() {
            Ok(path) if path.is_dir() => crate::projects::clean_path(&path),
            _ => return Err("That project folder could not be opened.".into()),
        };
        let id = if let Some(project) = self
            .projects
            .entries
            .iter_mut()
            .find(|p| crate::projects::same_path(&p.path, &path) && p.machine == "local")
        {
            project.hidden = false;
            project.updated = crate::projects::now();
            project.id
        } else {
            let id = self.projects.next_id.max(1);
            self.projects.next_id = id + 1;
            self.projects.entries.push(Project::new(id, path));
            id
        };
        if let Some(project) = self.project(Some(id)) {
            remote_tasks::repository(&project.path);
        }
        if let Some(index) = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.project == Some(id) && !t.archived())
            .max_by_key(|(_, t)| (t.updated, t.id))
            .map(|(i, _)| i)
        {
            self.select_thread(index);
        } else {
            self.new_thread_in(Some(id));
        }
        self.show_archive = false;
        self.checkpoint();
        if purpose != "project" {
            self.machines.project_selection.insert(
                self.machines.selected.clone(),
                (id, purpose.into(), captain),
            );
            self.finish_project_selection();
        }
        Ok(Some(id))
    }
    #[cfg(test)]
    pub fn archive(&mut self) {
        self.archive_thread(self.current().id);
    }
    pub fn setup(&mut self, wake: Waker) {
        self.wake = wake.clone();
        let image_wake = wake.clone();
        self.attachment_cache =
            crate::runtime::UiResource::new(move || rune::ImageCache::new(image_wake));
        if self.demo {
            return;
        }
        // Migrated profiles always attach/supervise a server, including plain launches.
        // Keep load_global's refusal as a defense against direct legacy access.
        if self.client.is_none() && self.data_dir.join("state.sqlite").exists() {
            self.client = Some(crate::client::Client::new(self.data_dir.clone()));
        }
        if self.client.is_some() {
            self.setup_client(wake);
            return;
        }
        if self.owner.is_none() {
            match crate::server_lock::Owner::acquire(&self.data_dir) {
                Ok(owner) => self.owner = Some(owner),
                Err(error) => {
                    self.ownership_denied = true;
                    self.storage_error = Some(error);
                    return;
                }
            }
        }
        if let Err(error) = std::fs::create_dir_all(&self.data_dir) {
            self.storage_error = Some(format!("Could not create Shina's data folder: {error}"));
            return;
        }
        self.load_global(wake.clone());
        if self.ownership_denied {
            return;
        }
        self.init_keybindings_file(wake.clone());
        self.setup_background();
        self.runtime_enabled = true;
        self.start_task_recovery();
        self.refresh_catalog();
        self.updater.start(true, Instant::now(), wake.clone());
        self.prewarm_current();
    }
    pub fn leave_demo(&mut self) {
        if !self.demo {
            return;
        }
        let mut live = Self::new(std::env::current_dir().unwrap_or_default(), false);
        live.setup(self.wake.clone());
        *self = live;
    }

    fn load_global(&mut self, wake: Waker) {
        if !self.headless && self.data_dir.join("state.sqlite").exists() {
            self.ownership_denied = true;
            self.storage_error = Some("This profile uses the durable server database. Launch with --server-client. Legacy JSON mode cannot write a migrated profile.".into());
            return;
        }
        if self.run_marker.is_none() {
            match crate::store::RunMarker::start(&self.data_dir) {
                Ok(marker) => self.run_marker = Some(marker),
                Err(error) => self.storage_error = Some(error),
            }
        }
        let path = self.data_dir.join("index.json");
        let mut runtime_inputs = None;
        match crate::store::load_global(&self.data_dir, self.launch_project.as_deref()) {
            Ok(Some(index)) if index["version"] == 3 || index["version"] == 4 => {
                if let Err(error) = self.restore_global(&index) {
                    self.storage_error =
                        Some(format!("{error} The index has not been overwritten."));
                    return;
                }
                if let Some(inputs) = index.get("runtimeInputs") {
                    if !inputs.is_object() {
                        self.storage_error = Some(
                            "Invalid durable runtime inputs. The index has not been overwritten."
                                .into(),
                        );
                        return;
                    }
                    runtime_inputs = Some(inputs.clone());
                }
            }
            Ok(Some(_)) => {
                self.storage_error = Some("The saved thread index uses an unsupported version. It has not been overwritten.".into());
                return;
            }
            Ok(None) => {}
            Err(error) => {
                self.storage_error = Some(error);
                return;
            }
        }
        let ui_path = self.data_dir.join(if self.headless {
            "server-inputs.json"
        } else {
            "ui-state.json"
        });
        if let Err(error) = self.load_memory(true) {
            self.storage_error = Some(error);
            return;
        }
        let saved_ui = crate::store::load(&ui_path).and_then(|ui| {
            if self.headless && ui.is_none() {
                crate::store::load(&self.data_dir.join("ui-state.json"))
            } else {
                Ok(ui)
            }
        });
        match saved_ui {
            Ok(Some(mut ui)) => {
                if self.headless {
                    let mut defaults = self.ui_snapshot();
                    for key in ["pendingMessages", "background"] {
                        if let Some(value) = ui.get(key) {
                            defaults[key] = value.clone();
                        }
                    }
                    ui = defaults;
                }
                if let Some(inputs) = &runtime_inputs {
                    ui["pendingMessages"] = inputs.clone();
                }
                if let Err(error) = self.restore_ui(&ui) {
                    self.storage_error = Some(error);
                    return;
                }
            }
            Ok(None) => {
                if let Some(inputs) = &runtime_inputs {
                    for thread in &mut self.threads {
                        if let Err(error) =
                            thread.restore_pending_inputs(&inputs[thread.id.to_string()])
                        {
                            self.storage_error = Some(error);
                            return;
                        }
                    }
                }
            }
            Err(error) => {
                self.storage_error = Some(error);
                return;
            }
        }
        if self.headless {
            for thread in &mut self.threads {
                thread.draft.replace_text("");
                thread.draft_images.clear();
            }
        }
        self.ui_store = Some(crate::store::Store::new(ui_path, wake.clone()));
        self.store = Some(crate::store::Store::new(path, wake));
        let running_tasks = self
            .tasks
            .iter()
            .filter(|task| matches!(task.state, TaskState::Starting | TaskState::Running))
            .count();
        self.prepare_task_recovery();
        if let Some(marker) = &self.run_marker
            && (running_tasks > 0 || marker.previous_unclean)
        {
            self.recovery_notice = Some(if marker.previous_unclean {
                "Shina closed unexpectedly. Recovered tasks need review. No task was automatically replayed.".into()
            } else {
                format!(
                    "Shina was closed while {running_tasks} tasks were running. Review or resume them explicitly."
                )
            });
        }
        if let Some(path) = self.launch_project.take()
            && let Err(error) = self.add_project(path)
        {
            self.current_mut().error = Some(error);
        }
        if !self.headless {
            self.checkpoint();
        }
    }

    fn restore_global(&mut self, index: &Value) -> Result<(), String> {
        self.restore_records(index, true)
    }
    /// Projections share record validation with session restore, but never restore navigation.
    fn restore_records(&mut self, index: &Value, restore_session: bool) -> Result<(), String> {
        self.default_harness = match index.get("defaultHarness") {
            None => "pi".into(),
            Some(value) => value
                .as_str()
                .filter(|id| crate::harness::valid_id(id))
                .ok_or("Invalid default harness.")?
                .into(),
        };
        self.thread_defaults =
            serde_json::from_value(index.get("threadDefaults").cloned().unwrap_or(json!({})))
                .map_err(|_| "Invalid thread defaults.")?;
        if AccessMode::parse(&self.thread_defaults.access).is_none() {
            self.thread_defaults.access = AccessMode::Supervised.id().into();
        }
        self.thread_defaults.validate()?;
        self.model_favorites = match index.get("modelFavorites") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                serde_json::from_value(value.clone()).map_err(|_| "Invalid model favorites.")?,
            ),
        };
        self.persist_transcripts = self.headless || index["threadViews"].is_array();
        self.server_seq = index["serverSeq"].as_u64().unwrap_or(0);
        self.command_receipts =
            serde_json::from_value(index.get("commandReceipts").cloned().unwrap_or(json!([])))
                .map_err(|e| format!("Invalid command receipts: {e}"))?;
        self.command_intents =
            serde_json::from_value(index.get("commandIntents").cloned().unwrap_or(json!({})))
                .map_err(|e| format!("Invalid command intents: {e}"))?;
        self.recovered_inputs =
            serde_json::from_value(index.get("recoveredInputs").cloned().unwrap_or(json!({})))
                .map_err(|e| format!("Invalid recovered inputs: {e}"))?;
        if let Some(epoch) = index["serverEpoch"].as_str() {
            self.server_epoch = epoch.into();
        }
        let Some(profiles) = index["captains"].as_array() else {
            return Err("The saved captain profiles are invalid.".into());
        };
        let captains: Option<Vec<Captain>> = profiles.iter().map(Captain::from_value).collect();
        let Some(captains) = captains else {
            return Err("A saved captain profile is invalid.".into());
        };
        let projects: Option<Vec<_>> = index["projects"]
            .as_array()
            .ok_or("Invalid project list.")?
            .iter()
            .map(|value| Project::from_record(value, self.client.is_none()))
            .collect();
        let mut canonical_projects: Vec<Project> = Vec::new();
        let mut project_remap = std::collections::HashMap::new();
        let mut maximum_project_id = 0;
        for project in projects.ok_or("Invalid project record.")? {
            maximum_project_id = maximum_project_id.max(project.id);
            if project_remap.contains_key(&project.id) {
                return Err("Duplicate project ids.".into());
            }
            if let Some(existing) = canonical_projects.iter_mut().find(|existing| {
                existing.machine == project.machine
                    && if self.client.is_some() {
                        existing.path == project.path
                    } else {
                        crate::projects::same_path(&existing.path, &project.path)
                    }
            }) {
                project_remap.insert(project.id, existing.id);
                existing.hidden &= project.hidden;
                existing.updated = existing.updated.max(project.updated);
                if existing.captain.is_none() {
                    existing.captain = project.captain;
                    existing.captain_thread = project.captain_thread;
                }
            } else {
                project_remap.insert(project.id, project.id);
                canonical_projects.push(project);
            }
        }
        let projects = canonical_projects;
        let project_ids: std::collections::HashSet<_> = projects.iter().map(|p| p.id).collect();
        if project_ids.len() != projects.len() {
            return Err("Duplicate project ids.".into());
        }
        if let Some(threads) = index["threads"].as_array() {
            let restored: Vec<Thread> = threads
                .iter()
                .filter_map(|record| {
                    let id = usize::try_from(record["id"].as_u64()?).ok()?;
                    if id == 0 || id == usize::MAX {
                        return None;
                    }
                    let mut thread = Thread::new(id);
                    if let Some(harness) = record.get("harness") {
                        let harness = harness.as_str()?;
                        if !crate::harness::valid_id(harness) {
                            return None;
                        }
                        thread.harness = harness.into();
                    }
                    if let Some(sent) = record.get("harnessContextSent") {
                        thread.harness_context_sent = sent.as_bool()?;
                    }
                    if let Some(context) = record.get("memoryContext") {
                        thread.memory_context = serde_json::from_value(context.clone()).ok()?;
                        if thread.memory_context.len() > 257
                            || thread.memory_context.iter().any(|(id, hash)| {
                                id != "profile" && !id.starts_with("memory-")
                                    || hash.len() != 64
                                    || !hash.bytes().all(|b| b.is_ascii_hexdigit())
                            })
                        {
                            return None;
                        }
                    }
                    if let Some(pending) = record.get("scopeContextPending") {
                        thread.scope_context_pending = pending.as_bool()?;
                    }
                    thread.never_sent = record["neverSent"] == true;
                    thread.native.fork_source = record["forkSource"].as_str().map(str::to_owned);
                    thread.native.fork_entry = record["forkEntry"].as_str().map(str::to_owned);
                    thread.native.previous_session =
                        record["previousSession"].as_str().map(str::to_owned);
                    if let Some(access) = record.get("accessMode") {
                        thread.native.access =
                            AccessMode::parse(access.as_str()?).unwrap_or(AccessMode::Supervised);
                    }
                    if index["version"] == 4 && record.get("lifecycle").is_none() {
                        return None;
                    }
                    for field in ["renamed", "captainHome", "delegated"] {
                        if record
                            .get(field)
                            .is_some_and(|v| !v.is_null() && !v.is_boolean())
                        {
                            return None;
                        }
                    }
                    for field in ["session", "model", "thinking"] {
                        if record
                            .get(field)
                            .is_some_and(|v| !v.is_null() && !v.is_string())
                        {
                            return None;
                        }
                    }
                    if record
                        .get("updated")
                        .is_some_and(|v| !v.is_null() && !v.is_u64())
                    {
                        return None;
                    }
                    thread.project = if record["project"].is_null() {
                        None
                    } else {
                        let original = usize::try_from(record["project"].as_u64()?).ok()?;
                        Some(*project_remap.get(&original)?)
                    };
                    thread.machine = record["machine"].as_str()?.into();
                    thread.updated = record["updated"].as_u64().unwrap_or(0);
                    if thread.machine.is_empty()
                        || thread.project.is_some_and(|id| !project_ids.contains(&id))
                    {
                        return None;
                    }
                    thread.title = record["title"].as_str()?.into();
                    thread.renamed = record["renamed"] == true;
                    thread.session = record["session"].as_str().map(str::to_owned);
                    thread.run_id = record["runId"].as_str().map(str::to_owned);
                    thread.resumable_run_id = record["resumableRunId"].as_str().map(str::to_owned);
                    thread.continuation_of_run_id =
                        record["continuationOfRunId"].as_str().map(str::to_owned);
                    if let Some(model) = record["model"].as_str() {
                        // Pi restores only CPA choices. ACP selections belong to its agent id.
                        let provider = if thread.harness == "pi" {
                            "cpa"
                        } else {
                            &thread.harness
                        };
                        thread.selected_model = Some((provider.into(), model.into()));
                        thread.model_provider = provider.into();
                        thread.model_id = model.into();
                        thread.model = model.into();
                    }
                    thread.selected_thinking = record["thinking"].as_str().map(str::to_owned);
                    if let Some(options) = record.get("options")
                        && let Ok(parsed) = serde_json::from_value::<
                            Vec<crate::model_selection::ProviderOptionSelection>,
                        >(options.clone())
                    {
                        thread.model_options = parsed;
                    }
                    if let Some(breakdown) = record.get("contextBreakdown")
                        && let Ok(parsed) = serde_json::from_value::<
                            Option<Vec<crate::context_breakdown::ContextBreakdownRow>>,
                        >(breakdown.clone())
                    {
                        thread.context.usage.breakdown = parsed;
                    }
                    thread.worktree_path = record
                        .get("worktreePath")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    thread.branch = record
                        .get("branch")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    thread.base_branch = record
                        .get("baseBranch")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    thread.env_mode = record
                        .get("envMode")
                        .and_then(|v| serde_json::from_value::<ThreadEnvMode>(v.clone()).ok());
                    thread.start_from_origin =
                        record.get("startFromOrigin").and_then(Value::as_bool);
                    if let Some(setup_val) = record.get("worktreeSetup")
                        && let Ok(mut setup) =
                            serde_json::from_value::<Option<WorktreeSetupState>>(setup_val.clone())
                    {
                        if let Some(ref mut s) = setup
                            && s.phase == WorktreeSetupPhase::Running
                        {
                            s.phase = WorktreeSetupPhase::Failed;
                            s.error = Some(
                                "Worktree setup was interrupted before restart. Retry or skip to continue."
                                    .into(),
                            );
                            thread.worktree_setup_output_open = true;
                        }
                        thread.worktree_setup = setup;
                    }
                    if let Some(suggestion) = record.get("promptSuggestion")
                        && let Ok(parsed) =
                            serde_json::from_value::<Option<PromptSuggestion>>(suggestion.clone())
                    {
                        thread.prompt_suggestion = parsed;
                    }
                    if thread.harness == "codex" {
                        thread.harness_state["serviceTier"] = record["serviceTier"].clone();
                    }
                    if let Some(level) = &thread.selected_thinking {
                        thread.thinking = level.clone();
                    }
                    thread.life = Lifecycle::from_record(record).ok()?;
                    thread.captain = if record["captain"].is_null() {
                        None
                    } else {
                        let id = usize::try_from(record["captain"].as_u64()?).ok()?;
                        if id >= captains.len() {
                            return None;
                        }
                        Some(id)
                    };
                    thread.captain_home = record["captainHome"] == true;
                    thread.delegated = record["delegated"] == true;
                    thread.peer_origin = record
                        .get("peerOrigin")
                        .filter(|v| !v.is_null())
                        .map(|v| serde_json::from_value(v.clone()))
                        .transpose()
                        .ok()?;
                    thread.stop_serial = record["stopSerial"].as_u64().unwrap_or(0);
                    thread.parent = if record["parent"].is_null() {
                        None
                    } else {
                        Some(usize::try_from(record["parent"].as_u64()?).ok()?)
                    };
                    Some(thread)
                })
                .collect();
            let ids: std::collections::HashSet<_> = restored.iter().map(|t| t.id).collect();
            let parents_valid = restored.iter().all(|thread| {
                (!thread.captain_home || thread.captain.is_some() && !thread.delegated)
                    && thread.delegated == (thread.parent.is_some() || thread.peer_origin.is_some())
                    && thread.parent.is_none_or(|parent| {
                        thread.delegated
                            && parent != thread.id
                            && restored.iter().any(|t| {
                                t.id == parent
                                    && !t.delegated
                                    && (t.project == thread.project
                                        || t.captain_home && t.captain == thread.captain)
                                    && t.machine == thread.machine
                            })
                    })
            });
            if restored.len() != threads.len() || ids.len() != restored.len() || !parents_valid {
                return Err("The saved index contains invalid or duplicate threads.".into());
            }
            for project in &projects {
                if project.captain.is_some_and(|id| id >= profiles.len())
                    || project.captain_thread.is_some_and(|id| {
                        !restored.iter().any(|t| {
                            t.id == id
                                && t.captain == project.captain
                                && t.captain_home
                                && !t.delegated
                        })
                    })
                    || restored
                        .iter()
                        .any(|t| t.project == Some(project.id) && t.machine != project.machine)
                {
                    return Err("Invalid project captain or machine.".into());
                }
            }
            let next_thread = next_counter(
                &index["nextId"],
                restored.iter().map(|t| t.id).max().unwrap_or(0),
            )?;
            let next_project = next_counter(&index["nextProjectId"], maximum_project_id)?;
            self.threads = restored;
            // v0.8 kept home flags but hid all except the project's default. Every
            // flagged home is now an independent coordinator, with its original id.
            for thread in &mut self.threads {
                if thread.captain_home {
                    thread.life.settled_override = None;
                    thread.life.settled_at = None;
                    thread.life.snoozed_at = None;
                    thread.life.snoozed_until = None;
                    thread.life.pinned_at = None;
                    thread.life.pin_order_key = None;
                    thread.life.auto_settle_disabled_at = Some(thread.life.created_at);
                }
            }
            self.captains = captains;
            self.projects.entries = projects;
            self.restore_captain_records(index, &project_remap)?;
            self.projects.next_id = next_project;
            self.next_id = next_thread;
            if restore_session {
                self.tabs = index["tabs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_u64)
                    .filter_map(|id| usize::try_from(id).ok())
                    .filter(|id| self.threads.iter().any(|t| t.id == *id && !t.archived()))
                    .collect();
                let mut seen_tabs = std::collections::HashSet::new();
                self.tabs.retain(|id| seen_tabs.insert(*id));
                // An archived thread is never reopened at launch. Without any live thread, land on
                // the most recent workspace's draft, as KSG Code's index route does.
                let active = self
                    .threads
                    .iter()
                    .position(|t| Some(t.id as u64) == index["active"].as_u64() && !t.archived())
                    .or_else(|| self.threads.iter().position(|t| !t.archived()));
                if let Some(active) = active {
                    self.active = active;
                    if !self.tabs.contains(&self.current().id) {
                        self.tabs.push(self.current().id);
                    }
                } else {
                    let project = self
                        .projects
                        .entries
                        .iter()
                        .filter(|p| !p.hidden)
                        .max_by_key(|p| (p.updated, p.id))
                        .map(|p| p.id);
                    self.active = self.active.min(self.threads.len().saturating_sub(1));
                    self.create_thread_in(project, &[], &Value::Null);
                }
            } else {
                self.active = 0;
                self.tabs.clear();
            }
        } else {
            return Err("The saved index has no valid thread list.".into());
        }
        self.dark = index["dark"] == true;
        self.auto_settle_days = index["autoSettleAfterDays"].as_u64().unwrap_or(3).min(3650) as u32;
        if let Some(zoom) = index["zoom"].as_f64() {
            self.zoom = snap_zoom(zoom as f32);
        }
        self.projects.recents.clone_parent = index["cloneParent"].as_str().map(PathBuf::from);
        self.project_scope = match index["projectScope"].as_str() {
            Some("none") => ProjectScope::NoProject,
            _ => index["projectScope"]
                .as_u64()
                .and_then(|id| usize::try_from(id).ok())
                .and_then(|id| project_remap.get(&id).copied())
                .filter(|id| project_ids.contains(id))
                .map(ProjectScope::Project)
                .unwrap_or(ProjectScope::All),
        };
        if restore_session {
            self.capture_opened_draft();
        }
        self.restore_tasks(index, &project_remap)?;
        if self.persist_transcripts && !self.headless && self.client.is_none() {
            self.restore_persisted_history(&index["threadViews"])?;
        }
        Ok(())
    }
    pub fn keybindings_path(&self) -> PathBuf {
        self.data_dir.join("keybindings.json")
    }
    pub fn ensure_keybindings_file(&mut self) {
        let path = self.keybindings_path();
        match crate::keybindings::sync_default_keybindings_on_startup(
            &path,
            &self.preferences.keybindings,
        ) {
            Ok(file_state) => {
                self.preferences.keybindings = file_state.rules;
                self.settings.keybindings_issues = file_state.issues;
                self.keybindings_watcher.record_stamp(&path);
            }
            Err(error) => {
                self.settings.keybindings_issues =
                    vec![crate::keybindings::KeybindingsIssue::MalformedConfig { message: error }];
            }
        }
    }
    pub fn init_keybindings_file(&mut self, wake: Waker) {
        self.ensure_keybindings_file();
        if !self.demo && !self.headless {
            self.keybindings_watcher
                .start(self.keybindings_path(), wake);
        }
    }
    pub fn poll_keybindings(&mut self) -> bool {
        if self.demo {
            return false;
        }
        let Some(file_state) = self.keybindings_watcher.poll(&self.keybindings_path()) else {
            return false;
        };
        self.preferences.keybindings = file_state.rules;
        self.settings.keybindings_issues = file_state.issues;
        true
    }
    pub fn save_keybinding_override(
        &mut self,
        command: &str,
        key: &str,
        when: Option<&str>,
        replace: Option<(&str, &str, Option<&str>)>,
    ) -> Result<(), String> {
        let path = self.keybindings_path();
        let file_state =
            crate::keybindings::upsert_keybinding_in_file(&path, command, key, when, replace)?;
        self.preferences.keybindings = file_state.rules;
        self.settings.keybindings_issues = file_state.issues;
        self.keybindings_watcher.record_stamp(&path);
        Ok(())
    }
    pub fn reset_keybinding_override(
        &mut self,
        command: &str,
        key: &str,
        when: Option<&str>,
        default_key: Option<&str>,
        default_when: &str,
    ) -> Result<(), String> {
        let path = self.keybindings_path();
        let file_state = crate::keybindings::reset_keybinding_in_file(
            &path,
            command,
            key,
            when,
            default_key,
            default_when,
        )?;
        self.preferences.keybindings = file_state.rules;
        self.settings.keybindings_issues = file_state.issues;
        self.keybindings_watcher.record_stamp(&path);
        Ok(())
    }

    pub(crate) fn ui_snapshot(&self) -> Value {
        let mut prefs_value = serde_json::to_value(&self.preferences).unwrap_or_else(|_| json!({}));
        if let Some(map) = prefs_value.as_object_mut() {
            map.remove("keybindings");
        }
        let draft_model_states: serde_json::Map<String, Value> = self
            .threads
            .iter()
            .filter(|t| {
                t.draft_model_state.model_selection_explicit
                    || !t.draft_model_state.additional_models.is_empty()
                    || t.has_draft_content()
            })
            .filter(|t| {
                !t.draft_model_state.model_selection_by_provider.is_empty()
                    || t.draft_model_state.active_provider.is_some()
                    || !t.draft_model_state.additional_models.is_empty()
            })
            .filter_map(|t| {
                serde_json::to_value(&t.draft_model_state)
                    .ok()
                    .map(|val| (t.id.to_string(), val))
            })
            .collect();
        json!({
            "background": self.background.settings,
            "preferences": prefs_value,
            "draftModelStates": draft_model_states,
            "version":1, "shelves":[false, self.shelves[0], self.shelves[1]], "sidebar":self.sidebar, "sidebarWidth":self.sidebar_width,
            "scope": match self.project_scope { ProjectScope::All => json!("all"),
                ProjectScope::NoProject => json!("none"), ProjectScope::Project(id) => json!(id) },
            "scopeAdditional":self.scope_additional,
            "scopeServer":self.machines.scope_server,
            "scopeRemote":self.machines.scope_remote,
            "pendingMessages":self.threads.iter().map(|t| (t.id.to_string(),t.pending_inputs_value())).collect::<serde_json::Map<_,_>>(),
            "drafts":self.threads.iter().map(|t| {
                let (draft, images, context, reply_to) = t.queued_edit.as_ref().map_or(
                    (&t.draft, &t.draft_images, &t.draft_context, &t.draft_reply_to),
                    |edit| (&edit.ordinary, &edit.ordinary_images, &edit.ordinary_context, &edit.ordinary_reply_to),
                );
                let value = if images.is_empty()
                    && context.is_empty()
                    && reply_to.is_none()
                    && !crate::composer_rich::has_format(draft)
                    && draft.pills().next().is_none()
                    && t.fresh_session_arm.is_none()
                {
                    json!(draft.text)
                } else {
                    let mut obj = json!({
                        "text": draft.text,
                        "richHtml": draft.to_html(),
                        "images": images.iter().map(Attachment::to_value).collect::<Vec<_>>(),
                        "context": if context.is_empty() { Value::Null } else { json!({ "version": 1, "records": context }) },
                    });
                    if let Some(reply) = reply_to {
                        obj["replyTo"] = json!(reply);
                    }
                    if let Some(arm) = t.fresh_session_arm {
                        obj["freshSession"] = json!(true);
                        obj["freshSessionArm"] = json!(arm);
                    }
                    obj
                };
                (t.id.to_string(), value)
            }).collect::<serde_json::Map<_,_>>(),
        })
    }
    pub(crate) fn runtime_input_snapshot(
        &self,
        scope: Option<&std::collections::HashSet<usize>>,
    ) -> Value {
        let pending = self
            .threads
            .iter()
            .filter(|t| scope.is_none_or(|s| s.contains(&t.id)))
            .map(|t| (t.id.to_string(), t.pending_inputs_value()))
            .collect::<serde_json::Map<_, _>>();
        if scope.is_some() {
            json!({"_partial":true,"pendingMessages":pending})
        } else {
            json!({"version":1,"pendingMessages":pending,"background":{"keep_awake":self.background_settings().keep_awake}})
        }
    }
    pub(crate) fn restore_ui(&mut self, ui: &Value) -> Result<(), String> {
        if ui["version"] != 1 {
            return Err("Unsupported UI state. Existing files have not been overwritten.".into());
        }
        self.background.settings = serde_json::from_value(
            ui.get("background").cloned().unwrap_or(json!({})),
        )
        .map_err(|_| "Invalid background settings. Existing UI state has not been overwritten.")?;
        self.preferences =
            serde_json::from_value(ui.get("preferences").cloned().unwrap_or(json!({})))
                .map_err(|_| "Invalid UI preferences.")?;
        // The first entry belonged to the removed Working shelf. It is still written so
        // earlier versions can read the file.
        let shelves: [bool; 3] =
            serde_json::from_value(ui["shelves"].clone()).map_err(|_| "Invalid shelf state.")?;
        self.shelves = [shelves[1], shelves[2]];
        self.sidebar = ui["sidebar"].as_bool().ok_or("Invalid sidebar state.")?;
        self.sidebar_width = ui
            .get("sidebarWidth")
            .map(|v| {
                v.as_f64()
                    .filter(|n| n.is_finite())
                    .map(|n| n.clamp(200.0, 400.0) as f32)
                    .ok_or("Invalid sidebar width.")
            })
            .transpose()?
            .unwrap_or(DEFAULT_SIDEBAR_WIDTH);
        self.project_scope = match ui["scope"].as_str() {
            Some("all") => ProjectScope::All,
            Some("none") => ProjectScope::NoProject,
            _ => ProjectScope::Project(
                ui["scope"]
                    .as_u64()
                    .and_then(|id| usize::try_from(id).ok())
                    .ok_or("Invalid project scope.")?,
            ),
        };
        self.scope_additional = serde_json::from_value(ui["scopeAdditional"].clone())
            .map_err(|_| "Invalid project scopes.")?;
        self.machines.scope_server = ui["scopeServer"].as_str().map(str::to_owned);
        self.machines.scope_remote =
            serde_json::from_value(ui.get("scopeRemote").cloned().unwrap_or(json!([])))
                .map_err(|_| "Invalid remote project scopes.")?;
        if let ProjectScope::Project(id) = self.project_scope
            && self
                .machines
                .scope_server
                .as_ref()
                .is_none_or(|owner| owner == &self.machines.selected)
            && self.project(Some(id)).is_none()
            && self
                .scope_additional
                .iter()
                .all(|id| self.project(Some(*id)).is_none())
        {
            self.project_scope = ProjectScope::All;
        }
        for (id, draft) in ui["drafts"].as_object().ok_or("Invalid saved drafts.")? {
            let (text, images, missing_images) = if let Some(text) = draft.as_str() {
                (text, Vec::new(), false)
            } else {
                let text = draft["text"].as_str().ok_or("Invalid draft text.")?;
                let saved_images = draft["images"]
                    .as_array()
                    .ok_or("Invalid saved draft images.")?;
                let images: Vec<_> = saved_images
                    .iter()
                    .filter_map(Attachment::from_value)
                    .collect();
                let missing = images.len() != saved_images.len();
                (text, images, missing)
            };
            if let Some(t) = id
                .parse::<usize>()
                .ok()
                .and_then(|id| self.threads.iter_mut().find(|t| t.id == id))
            {
                t.draft = crate::composer_rich::restore(text, draft["richHtml"].as_str());
                crate::composer_tokens::stamp_thread_owner(&mut t.draft, &t.machine);
                t.draft_images = images;
                t.draft_context = draft
                    .get("context")
                    .filter(|v| !v.is_null())
                    .and_then(|v| crate::composer_context::decode_message_context(v).ok())
                    .map(|c| c.records)
                    .unwrap_or_default();
                t.draft_reply_to = draft
                    .get("replyTo")
                    .filter(|v| !v.is_null())
                    .and_then(|v| {
                        serde_json::from_value::<crate::composer_context::ComposerContextRecord>(
                            v.clone(),
                        )
                        .ok()
                    })
                    .filter(|r| {
                        matches!(
                            r.data,
                            crate::composer_context::ComposerContextData::Response { .. }
                        )
                    });
                t.retained_context = Default::default();
                if draft["freshSession"] == true {
                    let arm = draft["freshSessionArm"].as_u64().unwrap_or_else(|| {
                        self.fresh_arm_seq = self.fresh_arm_seq.saturating_add(1).max(1);
                        self.fresh_arm_seq
                    });
                    self.fresh_arm_seq = self.fresh_arm_seq.max(arm);
                    t.fresh_session_arm = Some(arm);
                } else {
                    t.fresh_session_arm = None;
                }
                t.attachment_error = if missing_images {
                    Some("Some saved draft images are missing or invalid. Attach those images again.".into())
                } else {
                    attachments::validate_prompt_images(&t.draft_images).err()
                };
            }
        }
        if let Some(pending) = ui["pendingMessages"].as_object() {
            for (id, value) in pending {
                if let Some(thread) = id
                    .parse::<usize>()
                    .ok()
                    .and_then(|id| self.threads.iter_mut().find(|t| t.id == id))
                {
                    thread.restore_pending_inputs(value)?;
                }
            }
        }
        if let Some(draft_models) = ui.get("draftModelStates").and_then(Value::as_object) {
            for (id, value) in draft_models {
                if let Some(thread) = id
                    .parse::<usize>()
                    .ok()
                    .and_then(|id| self.threads.iter_mut().find(|t| t.id == id))
                    && let Ok(state) = serde_json::from_value(value.clone())
                {
                    thread.draft_model_state = state;
                }
            }
        }
        self.capture_opened_draft();
        self.sync_fresh_session_notice();
        Ok(())
    }
    pub fn checkpoint(&mut self) {
        if self.threads.iter().any(|t| t.answer_intake) {
            return;
        }
        if self.demo {
            return;
        }
        if self.client.is_some() {
            self.checkpoint_client();
            return;
        }
        // The runtime checkpoints once, after the entire reducer batch and before
        // dispatch. Reducer helpers must not serialize intermediate workspaces.
        if self.headless && crate::database::for_path(&self.data_dir.join("index.json")).is_some() {
            return;
        }
        let snapshot = self.snapshot();
        if let Some(store) = &mut self.store {
            store.save(snapshot);
        }
        let mut ui = self.ui_snapshot();
        if self.headless && crate::database::for_path(&self.data_dir.join("index.json")).is_some() {
            ui = json!({"version":1,"pendingMessages":ui["pendingMessages"],
                "background":{"keep_awake":ui["background"]["keep_awake"]}});
        }
        if let Some(store) = &mut self.ui_store {
            store.save(ui);
        }
    }
    /// Starts both stores against this state's data folder, as `setup` does, without Pi.
    #[cfg(test)]
    pub fn load_global_for_test(&mut self) {
        self.load_global(self.wake.clone());
    }

    /// Saves both stores and waits until the writes are on disk. A disabled or failed save
    /// blocks an update restart.
    pub fn checkpoint_and_flush(&mut self) -> Result<(), String> {
        self.checkpoint();
        self.flush_stores()
    }

    #[cfg(test)]
    pub fn take_restart_request(&mut self) -> bool {
        std::mem::take(&mut self.restart_approved)
    }

    pub fn cancel_update_exit(&mut self) {
        self.restart_approved = false;
        self.restart_for_update = false;
        crate::update::take_exit();
    }

    /// Says why a restart waits, next to the update row rather than as a thread error.
    pub fn report_restart_blocker(&mut self, reason: String) {
        self.restart_notice = Some(reason);
    }

    fn flush_stores(&mut self) -> Result<(), String> {
        if self.client.is_some() {
            return self
                .ui_store
                .as_ref()
                .ok_or("Client drafts are not saving.")?
                .flush();
        }
        if self.demo {
            return Err("Demo runs do not save.".into());
        }
        let (Some(store), Some(ui)) = (self.store.as_ref(), self.ui_store.as_ref()) else {
            return Err("Shina is not saving this session.".into());
        };
        // Flush both even if the first one failed.
        let index = store.flush();
        let drafts = ui.flush();
        let memory = self.flush_memory();
        index.and(drafts).and(memory)
    }

    /// Why a restart to update must wait, or `None` when nothing is active. It checks every
    /// thread, including delegated, settled and archived ones.
    pub fn update_restart_blocker(&self) -> Option<&str> {
        if self.client.is_some() && self.machines.selected != "local" {
            if self.rename_thread.is_some() || self.captain_editor.is_some() {
                return Some("Finish editing before restarting.");
            }
            // Updating this desktop concerns Local, not the remote thread being read.
            return self
                .machines
                .views
                .get("local")
                .and_then(|view| view["updateStatus"]["blocker"].as_str());
        }
        if self.routines_running() {
            return Some("A routine is still running. Restart when it finishes.");
        }
        if self.tasks.iter().any(|task| {
            matches!(
                task.state,
                TaskState::Queued | TaskState::Starting | TaskState::Running
            )
        }) {
            return Some("A delegated task is running or queued. Restart when it finishes.");
        }
        if self.threads.iter().any(|t| {
            t.status.busy()
                || t.configuring
                || t.session_started.is_some()
                || t.queued()
                || t.request.is_some()
        }) {
            return Some("A thread is still working. Restart when it finishes.");
        }
        if self.projects.cloning() {
            return Some("A repository is still cloning.");
        }
        if self.rename_thread.is_some() || self.captain_editor.is_some() {
            return Some("Finish editing before restarting.");
        }
        None
    }

    /// Only an explicit restart installs updates. Every normal quit still saves and flushes.
    pub fn prepared_exit(
        &mut self,
        explicit: bool,
        quit_reply: Option<&mut String>,
    ) -> Result<(), String> {
        if !explicit {
            self.cancel_update_exit();
            return self.checkpoint_and_flush();
        }
        let blocker = self.update_restart_blocker();
        if explicit && let Some(blocker) = blocker {
            if let Some(reply) = quit_reply {
                *reply = blocker.to_owned();
            }
            return Err("The update cannot restart yet.".into());
        }
        let update = self
            .updater
            .prepared_update()
            .filter(|_| !self.demo && blocker.is_none())
            .cloned();
        let Some(update) = update else {
            if explicit {
                if let Some(reply) = quit_reply {
                    *reply = "The update is not ready yet.".into();
                }
                return Err("The update is not ready yet.".into());
            }
            return Ok(());
        };
        // A failed save blocks the install. Keep the download for another restart attempt.
        if let Err(error) = self.checkpoint_and_flush() {
            if let Some(reply) = quit_reply {
                *reply = error.clone();
            }
            return Err(error);
        }
        if self.client.is_some() {
            if !self
                .client
                .as_ref()
                .is_some_and(|client| client.commands.connected())
            {
                return Err("Reconnect to the local server before requesting an update.".into());
            }
            crate::update::start_coordinated_helper(&update)?;
            self.restart_for_update = false;
            self.restart_notice =
                Some("Waiting for the server and local clients to save before restarting.".into());
            return Err(
                "Coordinated restart requested. This window will close after every client saves."
                    .into(),
            );
        }
        crate::update::record_exit(crate::update::ExitUpdate {
            update,
            relaunch: explicit,
        });
        self.restart_approved |= explicit;
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Value {
        self.snapshot_scoped(None)
    }
    pub(crate) fn snapshot_scoped(
        &self,
        scope: Option<&std::collections::HashSet<usize>>,
    ) -> Value {
        if let Some(scope) = scope {
            return json!({"_partial":true,
                "threads":self.threads.iter().filter(|t|scope.contains(&t.id)).map(Thread::stored_metadata).collect::<Vec<_>>(),
                "threadViews":self.thread_views_scoped(Some(scope)),
                "runtimeInputs":self.threads.iter().filter(|t|scope.contains(&t.id)).map(|t|(t.id.to_string(),t.runtime_inputs_value())).collect::<serde_json::Map<_,_>>()
            });
        }
        let mut index = json!({
            "version":4, "dark":self.dark, "zoom":self.zoom, "active":self.current().id, "tabs":self.tabs,
            "nextId":self.next_id, "nextProjectId":self.projects.next_id,
            "autoSettleAfterDays": self.auto_settle_days,
            "defaultHarness": self.default_harness,
            "threadDefaults": self.thread_defaults,
            "modelFavorites": self.model_favorites,
            "tasks": self.tasks,
            "projects": self.projects.entries.iter().map(Project::to_value).collect::<Vec<_>>(),
            "cloneParent": self.projects.recents.clone_parent,
            "captains":self.captains.iter().map(Captain::to_value).collect::<Vec<_>>(),
            "captainScopes":self.captain_scopes,
            "captainRemoteScopes":self.captain_remote_scopes,
            "captainHomes":self.captain_homes,
            "threads":self.threads.iter().filter(|thread| {
                // A blank draft is saved only while it is open or has a tab.
                self.headless || self.persist_transcripts || !thread.blank_draft() || thread.id == self.current().id || self.tabs.contains(&thread.id)
            }).map(Thread::stored_metadata).collect::<Vec<_>>()
        });
        index["commandReceipts"] = json!(self.command_receipts);
        index["commandIntents"] = json!(self.command_intents);
        index["serverEpoch"] = json!(self.server_epoch);
        index["serverSeq"] = json!(self.server_seq);
        index["recoveredInputs"] = json!(self.recovered_inputs);
        if self.headless
            || self.persist_transcripts
            || self
                .threads
                .iter()
                .any(|t| !t.fresh_session_messages.is_empty())
        {
            index["threadViews"] = self.thread_views();
            index["runtimeInputs"] = json!(
                self.threads
                    .iter()
                    .map(|thread| (thread.id.to_string(), thread.runtime_inputs_value()))
                    .collect::<serde_json::Map<_, _>>()
            );
        }
        index
    }
    pub fn populate_demo(&mut self) {
        self.demo = true;
        self.captains = Captain::presets();
        self.threads.clear();
        let mut thread = Thread::new(1);
        thread.title = "Fix the reconnect listener".into();
        thread.captain = Some(0);
        thread.status = Status::Completed;
        thread.model = DEFAULT_MODEL.into();
        thread.thinking = "medium".into();
        thread.elapsed = Some(42);
        thread.work.push(AssistantWork {
            start: 1,
            stopped: false,
            stopped_order: 0,
            expanded: true,
            elapsed: Some(42),
            outcome: Some(Status::Completed),
        });
        thread.messages = vec![
            Message { id: uuid::Uuid::new_v4().to_string(), role: "You", text: "Fix the reconnect race, and add a regression test for the duplicate listener.".into(), images: Vec::new(), context: None },
            Message { id: uuid::Uuid::new_v4().to_string(), role: "Pi", text: "I'll check the socket lifecycle and where the reconnect listener attaches before making the change.".into(), images: Vec::new(), context: None },
            Message { id: uuid::Uuid::new_v4().to_string(), role: "Pi", text: "The reconnect listener attaches too late. I'll move it before `socket.open()` and cover the initialization race.".into(), images: Vec::new(), context: None },
            Message { id: uuid::Uuid::new_v4().to_string(), role: "Pi", text: "The listener now attaches before the socket opens. Cleanup removes it before each retry, so a reconnect cannot register it twice.\n\n```diff\n- socket.open();\n  socket.on(\"reconnect\", reconnect);\n+ socket.open();\n```\n\nThe regression test passes. Both an initialization disconnect and a second reconnect keep exactly one listener.".into(), images: Vec::new(), context: None },
        ];
        thread.tools = vec![
            Tool {
                id: "example-1".into(),
                kind: None, changes: vec![], input: Value::Null, raw_expanded: false,
                order: 0,
                name: "read".into(),
                detail: "src/transport/socket.ts".into(),
                done: true,
                failed: false,
                output: "Example activity, not a real tool execution.".into(),
                anchor: ToolAnchor { message: 2, offset: 0 },
                block: 0,
                expanded: false,
                group_expanded: false,
                started: None,
                duration_ms: Some(180),
                subagents: vec![],
                images: vec![], structured: Value::Null, parent_call: None, exit_code: None,
            },
            Tool {
                id: "example-2".into(),
                kind: None, changes: vec![], input: Value::Null, raw_expanded: false,
                order: 0,
                name: "read".into(),
                detail: "tests/reconnect.test.ts".into(),
                done: true,
                failed: false,
                output: "Example activity, not a real tool execution.".into(),
                anchor: ToolAnchor { message: 2, offset: 0 },
                block: 1,
                expanded: false,
                group_expanded: false,
                started: None,
                duration_ms: Some(120),
                subagents: vec![],
                images: vec![], structured: Value::Null, parent_call: None, exit_code: None,
            },
            Tool {
                id: "example-3".into(),
                kind: None, changes: vec![], input: Value::Null, raw_expanded: false,
                order: 0,
                name: "edit".into(),
                detail: "src/transport/socket.ts".into(),
                done: true,
                failed: false,
                output: "Example output\nReplaced 2 lines in src/transport/socket.ts.".into(),
                anchor: ToolAnchor { message: 3, offset: 0 },
                block: 0,
                expanded: false,
                group_expanded: true,
                started: None,
                duration_ms: Some(240),
                subagents: vec![],
                images: vec![], structured: Value::Null, parent_call: None, exit_code: None,
            },
            Tool {
                id: "example-4".into(),
                kind: None, changes: vec![], input: Value::Null, raw_expanded: false,
                order: 0,
                name: "powershell".into(),
                detail: "npm test -- reconnect.test.ts".into(),
                done: true,
                failed: false,
                output: "Example output\nPASS tests/reconnect.test.ts\n  reconnect lifecycle\n    attaches before opening the socket\n    removes the listener before retrying\n\nTests: 2 passed, 2 total".into(),
                anchor: ToolAnchor { message: 3, offset: 0 },
                block: 1,
                expanded: false,
                group_expanded: false,
                started: None,
                duration_ms: Some(1100),
                subagents: vec![],
                images: vec![], structured: Value::Null, parent_call: None, exit_code: None,
            },
        ];
        thread.thoughts.push(Thought {
            order: 0,
            anchor: ToolAnchor { message: 2, offset: 0 },
            block: 2,
            text: "Example thinking, not a real model response.\n\nThe listener must attach before socket.open() can emit a reconnect. Cleanup must remove the old listener before another attempt.".into(),
            expanded: false,
            done: true,
            started: None,
            duration_ms: Some(18000),
        });
        self.threads.push(thread);
        for (id, title, status) in [
            (2, "Review sidebar keys", Status::Running),
            (3, "Reduce cold-start work", Status::Completed),
            (4, "Add a native file picker", Status::Completed),
            (5, "Plan the Pi session adapter", Status::Completed),
            (6, "Audit the Markdown renderer", Status::Ready),
        ] {
            let mut thread = Thread::new(id);
            thread.title = title.into();
            thread.status = status;
            thread.life.pinned_at = (id == 3).then_some(lifecycle::now());
            if id == 2 {
                thread.captain = Some(0);
            }
            self.threads.push(thread);
        }
        // One home per captain, in different states, so the crew shows its range.
        for (id, captain, status) in [
            (7, 0, Status::Ready),
            (8, 1, Status::Waiting),
            (9, 2, Status::Completed),
            (10, 3, Status::Ready),
        ] {
            let mut thread = Thread::new(id);
            thread.title = crate::visuals::CAPTAIN_NAMES[captain].into();
            thread.captain = Some(captain);
            thread.captain_home = true;
            thread.status = status;
            self.threads.push(thread);
        }
        self.next_id = 11;
        self.active = 0;
        self.projects = crate::projects::Projects::demo();
        self.projects.entries = ["shina", "rune", "ksgcode"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let mut project = Project::new(i + 1, PathBuf::from("/workspaces").join(name));
                project.captain = Some(i);
                project.captain_thread = Some(i + 7);
                project.updated = crate::projects::now().saturating_sub(i as u64 * 120);
                project
            })
            .collect();
        self.projects.next_id = 4;
        for thread in &mut self.threads {
            thread.project = match thread.id {
                1 | 2 | 7 => Some(1),
                3 | 4 | 8 => Some(2),
                5 | 9 => Some(3),
                _ => None,
            };
            thread.captain = thread.project.map(|id| id - 1);
            thread.captain_home = matches!(thread.id, 7..=9);
            thread.updated = crate::projects::now().saturating_sub(thread.id as u64 * 60);
        }
        self.threads[9].title = "Plan a weekend tool".into();
        self.threads[1].life.last_completed_at = Some(lifecycle::now());
        self.threads[1].life.last_visited_at = Some(0);
        for t in &mut self.threads {
            t.life.created_at = t.updated.saturating_mul(1000);
            t.life.updated_at = t.life.created_at;
            if !t.captain_home {
                t.life.latest_user_message_at = Some(t.life.created_at);
            }
        }
        self.threads[2].life.pin_order_key = Some("n".into());
        self.threads[3]
            .life
            .snooze(lifecycle::now(), lifecycle::now() + 10_800_000);
        self.threads[4].life.settle(lifecycle::now() - 86_400_000);
        self.shelves = [false; 2];
        self.undo = None;
        self.tabs = vec![1, 3, 10];
    }
}

impl Thread {
    fn stored_metadata(&self) -> Value {
        let mut record = json!({
            "id":self.id, "title":self.title, "renamed":self.renamed, "session":self.session, "lifecycle":self.life,
            "harness":self.harness, "harnessContextSent":self.harness_context_sent,
            "runId":self.run_id,"resumableRunId":self.resumable_run_id,"continuationOfRunId":self.continuation_of_run_id,
            "memoryContext":self.memory_context, "scopeContextPending":self.scope_context_pending,
            "neverSent":self.never_sent && self.life.latest_user_message_at.is_none(),
            "forkSource":self.native.fork_source, "forkEntry":self.native.fork_entry,
            "previousSession":self.native.previous_session,"accessMode":self.native.access.id(),
            "captain":self.captain, "captainHome":self.captain_home, "delegated":self.delegated,"parent":self.parent
        });
        record.as_object_mut().unwrap().extend(json!({
            "peerOrigin":self.peer_origin,"stopSerial":self.stop_serial,
            "project":self.project,"machine":self.machine,"updated":self.updated,
            "model":self.selected_model.as_ref().map(|(_,id)|id),"thinking":self.selected_thinking,
            "options":self.model_options,
            "contextBreakdown":self.context.usage.breakdown,
            "promptSuggestion":self.prompt_suggestion,
            "serviceTier":self.harness_state["serviceTier"]
        }).as_object().unwrap().clone());
        if let Some(worktree_path) = &self.worktree_path {
            record["worktreePath"] = json!(worktree_path);
        }
        if let Some(branch) = &self.branch {
            record["branch"] = json!(branch);
        }
        if let Some(base_branch) = &self.base_branch {
            record["baseBranch"] = json!(base_branch);
        }
        if let Some(env_mode) = self.env_mode {
            record["envMode"] = json!(env_mode);
        }
        if let Some(start_from_origin) = self.start_from_origin {
            record["startFromOrigin"] = json!(start_from_origin);
        }
        if let Some(worktree_setup) = &self.worktree_setup {
            record["worktreeSetup"] = json!(worktree_setup);
        }
        record
    }
}

/// Ported from KSG Code's `isClearComposerCommand` (`packages/shared/src/composerTrigger.ts`).
pub fn is_clear_command(text: &str) -> bool {
    text.trim().eq_ignore_ascii_case("/clear")
}

/// Ported from KSG Code's `COMPACT_COMMAND_PATTERN` (`packages/client-runtime/src/state/contextWindow.ts`).
pub fn is_compact_command(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("/compact"))
        && trimmed[8..].chars().next().is_none_or(char::is_whitespace)
}

pub fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    let composer_changed = state.composer_sources.take_changed()
        | (!state.headless && crate::ui::composer_ui::tick(state, now, tx));
    let mut html_changed = false;
    while let Some(outcome) = state.html_windows.next() {
        html_changed = true;
        match outcome {
            Ok(Some(url)) => {
                let _ = tx.open_url(&url);
            }
            Ok(None) => {}
            Err(error) => state.storage_error = Some(error),
        }
    }
    state.html_windows.theme(state.dark);
    for url in std::mem::take(&mut state.pending_open_urls) {
        html_changed = true;
        if let Err(error) = tx.open_url(&url) {
            state.push_toast(
                ToastKind::Error,
                "Could not open the link",
                Some(error.to_string()),
                None,
                Some(5_000),
            );
        }
    }
    let keybindings_changed = !state.headless && state.poll_keybindings();
    let browsing = state.poll_folder_browser()
        | state.poll_settings()
        | state.poll_thread_find(tx)
        | state.poll_sidebar_search(tx)
        | html_changed
        | keybindings_changed;
    // Linked-machine examples are offline fixtures. Animate their artwork, but
    // never run native provider maintenance or client commands for those records.
    if state.demo && state.client.is_some() {
        if let Some(wait) = crate::ui::tick_portraits(state, Instant::now()) {
            tx.wake_after(wait);
            return true;
        }
        return browsing | composer_changed;
    }
    if state.client.is_some() {
        return state.poll_client(tx) | browsing | composer_changed;
    }
    let browsing = browsing | state.poll_fleet();
    let background_changed = state.poll_background_start(tx);
    if state.background.quitting {
        return true;
    }
    if let Some(marker) = &mut state.run_marker {
        if let Err(error) = marker.tick() {
            state.storage_error = Some(error);
        }
        tx.wake_after(Duration::from_secs(60));
    }
    // A resumed event loop is no longer committed to the earlier quit attempt.
    if state.restart_approved && !rune::quitting() {
        state.cancel_update_exit();
    }
    let mut changed = browsing
        | composer_changed
        | background_changed
        | state.updater.poll(Instant::now(), state.wake.clone());
    if let Some(remaining) = state.updater.remaining(Instant::now()) {
        tx.wake_after(remaining);
    }
    changed |= state.poll_projects();
    for thread in &mut state.threads {
        if !thread.draft.focused {
            thread.typing_guard.on_blur_or_send();
        }
    }
    if state.catalog.poll() {
        state.apply_catalog();
        changed = true;
    }
    if let Some(remaining) = state.catalog.remaining() {
        tx.wake_after(remaining);
    }
    state.refresh_native_catalogs(false);
    if state.codex_catalog.poll("codex") | state.claude_catalog.poll("claude") {
        state.apply_native_catalogs();
        changed = true;
    }
    if state.codex_catalog.loading() || state.claude_catalog.loading() {
        tx.wake_after(Duration::from_millis(50));
    }
    changed |= state.prewarm_current();
    changed |= state.poll_vcs_branches();
    changed |= state.poll_worktree_bootstraps();
    changed |= state.poll_worktree_setup();
    changed |= state.poll_question_answers();
    if state.runtime_enabled && state.client.is_none() {
        for index in 0..state.threads.len() {
            if std::mem::take(&mut state.threads[index].native.question_message_connect) {
                state.connect_thread(index);
                changed = true;
            }
        }
    }
    if state.runtime_enabled
        && !state.demo
        && let Ok(server) = crate::mcp::server(state.wake.clone())
    {
        for _ in 0..16 {
            let Some(request) = server.next_request() else {
                break;
            };
            state.mcp_request(request);
            changed = true;
            tx.wake_after(Duration::ZERO);
        }
    }
    let mut run_ended = false;
    let mut crew_questions = Vec::new();
    let mut background_failures = Vec::new();
    let mut event_budget_exhausted = false;
    if changed {
        state.persistence_scope = None;
    }
    let remote_children: std::collections::HashSet<_> = state
        .tasks
        .iter()
        .filter(|t| t.remote.is_some())
        .map(|t| t.child_thread_id)
        .collect();
    let mut connect_indices = Vec::new();
    for (index, thread) in state.threads.iter_mut().enumerate() {
        if remote_children.contains(&thread.id) {
            continue;
        }
        let effects_before = crate::outbox::captured_count();
        let mut thread_changed = thread.poll_memory_command();
        let was_bg_pending = thread.background_start_pending;
        // Batch incoming deltas into one view update, with a bounded per-frame drain.
        for event_index in 0..256 {
            let event = thread.pi.as_ref().and_then(|pi| pi.events.try_recv().ok());
            let Some(event) = event else { break };
            if event_index == 255 {
                // Even if this was the final event, reject this click conservatively.
                event_budget_exhausted = true;
                tx.wake_after(Duration::ZERO);
            }
            thread_changed = true;
            thread.updated = crate::projects::now();
            thread.life.updated_at = lifecycle::now();
            match event {
                Event::Record(record) => {
                    state.background.observe_record(&record);
                    let settled = record["type"] == "agent_settled";
                    if matches!(
                        record["type"].as_str(),
                        Some("agent_start" | "message_start" | "agent_settled")
                    ) || (record["type"] == "response"
                        && record["command"] == "prompt"
                        && record["success"] != false)
                    {
                        thread.background_start_pending = false;
                    }
                    if record["type"] == "extension_ui_request"
                        && !thread.recovering_task
                        && matches!(
                            record["method"].as_str(),
                            Some("input" | "editor" | "select")
                        )
                        && let Some(parent) = thread.parent
                    {
                        let token = record["id"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| record.to_string());
                        if thread.forwarded_question.as_deref() != Some(token.as_str()) {
                            thread.forwarded_question = Some(token);
                            crew_questions.push((parent, format!(
                            "Shina: crew #{} asks a question.\n{}\nAnswer with mcp__shina__task_send using thread_id {} and message containing your answer. For select, use one listed option exactly. Approval requests still require the user.",
                            thread.id, record, thread.id
                            )));
                        }
                    }
                    run_ended |= settled;
                    thread.apply(record);
                    if settled {
                        thread.command("get_state", json!({}));
                    }
                    if !state.headless && index == state.active {
                        thread.life.last_visited_at = Some(lifecycle::now());
                    }
                }
                Event::Error(error) => {
                    run_ended = true;
                    thread.fail_session(error);
                }
                Event::Diagnostic(diagnostic) => thread.diagnostic(&diagnostic),
                Event::Exited => {
                    run_ended = true;
                    if !matches!(thread.status, Status::Failed | Status::Paused) {
                        thread.fail_session(
                            "Pi stopped unexpectedly. Retry to resume the saved conversation."
                                .into(),
                        );
                    }
                }
            }
        }
        thread.drive_session();
        thread_changed |= thread.check_deadlines(Instant::now());
        thread_changed |= thread.native_tick(Instant::now());
        if let Some(deadline) = thread.native_deadline() {
            tx.wake_after(deadline.saturating_duration_since(Instant::now()));
        }
        if !thread.offline_blocked
            && thread.request.is_none()
            && thread
                .session_started
                .is_some_and(|at| at.elapsed() >= START_TIMEOUT)
        {
            thread.fail_session(
                "Pi did not finish preparing the session. Retry to resume it.".into(),
            );
            thread_changed = true;
        }
        if was_bg_pending && thread.status == Status::Failed {
            thread.background_start_pending = false;
            thread_changed = true;
            if index != state.active {
                background_failures.push((
                    thread.id,
                    thread
                        .error
                        .clone()
                        .unwrap_or_else(|| "Failed to send message.".into()),
                ));
            }
        }
        thread_changed |= thread.reclaim_idle(Instant::now());
        // A quiet RPC stream must not freeze the visible work/thinking clocks.
        if !state.headless && index == state.active && thread.status.busy() {
            tx.wake_after(Duration::from_secs(1));
            changed = true;
        }
        // A quiet session can become dispatchable without another provider record.
        // Its intent must join this batch even when a different thread streamed.
        thread_changed |= crate::outbox::captured_count() != effects_before;
        if thread_changed {
            changed = true;
            if let Some(scope) = &mut state.persistence_scope {
                scope.insert(thread.id);
            }
        }
        if thread.pi.is_some() {
            if let Some((_, deadline)) = thread.operation_deadline {
                tx.wake_after(deadline.saturating_duration_since(Instant::now()));
            }
            if !thread.offline_blocked
                && let Some(deadline) = thread.injection_retry_at
            {
                tx.wake_after(deadline.saturating_duration_since(Instant::now()));
            }
            let remaining = if thread.offline_blocked {
                Duration::from_secs(2)
            } else if let Some(started) = thread.session_started {
                START_TIMEOUT.saturating_sub(started.elapsed())
            } else if !thread.status.busy() && !thread.configuring {
                IDLE_TIMEOUT.saturating_sub(thread.last_activity.elapsed())
            } else {
                IDLE_TIMEOUT
            };
            tx.wake_after(remaining);
        }
        if thread.pi.is_none()
            && thread.queued_prompt.is_some()
            && thread.status == Status::Running
            && !thread.offline_blocked
            && thread.session_started.is_none()
        {
            connect_indices.push(index);
        }
    }
    for index in connect_indices {
        state.connect_thread(index);
        changed = true;
    }
    for (parent, question) in crew_questions {
        state.persistence_scope = None;
        let parent = state.effective_home(parent);
        if let Some(i) = state.threads.iter().position(|t| t.id == parent) {
            state.threads[i].enqueue_injection(question);
            if state.threads[i].pi.is_none() {
                state.connect_thread(i);
            }
        }
    }
    let mut global_changed = false;
    for (thread_id, error) in background_failures {
        if !state.handle_multi_model_async_failure(thread_id, &error) {
            state.notify_background_failure("local", thread_id, error);
        }
        global_changed = true;
    }
    global_changed |= state.pause_offline_errors()
        | state.poll_tasks(Instant::now())
        | state.poll_background_end(tx);
    changed |= state.sync_fresh_session_notice();
    changed |= state.sync_fallback_model(lifecycle::now() as i64);
    if let Some(outcome) = state
        .current_mut()
        .harness_state
        .as_object_mut()
        .and_then(|obj| obj.remove("resetCreditOutcome"))
    {
        changed = true;
        if let Some(err) = outcome["error"].as_str() {
            state.push_toast(
                ToastKind::Error,
                "Could not use the reset credit.",
                Some(err.to_owned()),
                None,
                Some(5_000),
            );
        } else {
            let msg = match outcome["outcome"].as_str() {
                Some("reset") => "Reset applied. Your windows have cleared.",
                Some("nothingToReset") => "Nothing to reset right now.",
                Some("noCredit") => "No reset credit left.",
                Some("alreadyRedeemed") => "That credit was already redeemed.",
                _ => "Could not use the reset credit.",
            };
            let kind = if outcome["outcome"].as_str() == Some("reset") {
                ToastKind::Success
            } else {
                ToastKind::Warning
            };
            state.push_toast(kind, msg, None, None, Some(5_000));
            if state.usage_limits_notice_key.is_some() {
                state.open_usage_limits();
            }
        }
    }
    if global_changed {
        state.persistence_scope = None;
        changed = true;
    }
    if !state.headless
        && let Some(wait) = crate::ui::tick_portraits(state, Instant::now())
    {
        tx.wake_after(wait);
    }
    // A restart click only asks for one. The guard runs after Pi events drained and tasks
    // were delivered, and an approved request stops all further work in this poll.
    if state.restart_for_update {
        state.restart_for_update = false;
        let mut reply = String::new();
        let approval = if event_budget_exhausted {
            reply =
                "Shina is still processing events. Click restart again when it finishes.".into();
            Err("Pending events".to_owned())
        } else {
            state.prepared_exit(true, Some(&mut reply))
        };
        match approval {
            Ok(()) => {
                tx.quit();
                return true;
            }
            Err(_) => {
                state.report_restart_blocker(reply);
                changed = true;
            }
        }
    }
    let at = lifecycle::now();
    let early_wake: Vec<_> = state
        .threads
        .iter()
        .map(|t| {
            state.row_pending(t)
                || t.life
                    .failed_at
                    .is_some_and(|failed| failed > t.life.snoozed_at.unwrap_or(u64::MAX))
        })
        .collect();
    for (i, t) in state.threads.iter_mut().enumerate() {
        if t.life
            .snoozed_until
            .is_some_and(|wake| wake > state.last_lifecycle_clock && wake <= at)
        {
            if !state.headless {
                changed = true;
            }
        }
        if t.life.snoozed(at) && early_wake[i] {
            // Store the early edge once. Answering the request must not re-snooze it
            // or make the scheduled wake display a second Woke notification.
            t.life.last_snooze_wake_at = Some(at);
            if let Some(scope) = &mut state.persistence_scope {
                scope.insert(t.id);
            }
            changed = true;
        }
        if let Some(wake) = t.life.snoozed_until.filter(|wake| *wake > at) {
            tx.wake_after(Duration::from_millis(wake - at));
        }
        if !state.headless && i == state.active && t.life.woke(at) {
            t.life.last_visited_at = Some(at);
            changed = true;
        }
    }
    state.last_lifecycle_clock = at;
    if run_ended || at.saturating_sub(state.last_sweep) >= 60000 {
        if state.sweep_lifecycle(at) {
            state.persistence_scope = None;
            changed = true;
        }
        // The minute clock also repaints wake countdowns and row ages.
        changed |= !state.headless;
    }
    tx.wake_after(Duration::from_millis(
        60000 - at.saturating_sub(state.last_sweep).min(60000),
    ));
    if let Some(notice) = &state.undo {
        if notice.expires <= at {
            state.undo = None;
            state.deleted_threads.clear();
            state.deleted_records = None;
            changed = true;
        } else {
            tx.wake_after(Duration::from_millis(notice.expires - at));
        }
    }
    changed |= state.expire_toasts();
    for toast in &state.toasts {
        if let Some(expires) = toast.expires_at
            && expires > at
        {
            tx.wake_after(Duration::from_millis(expires - at));
        }
    }
    if let Some((_, expires)) = &state.plan_copied {
        if *expires <= at {
            state.plan_copied = None;
            changed = true;
        } else {
            tx.wake_after(Duration::from_millis(*expires - at));
        }
    }
    if let Some(error) = state
        .store
        .as_ref()
        .and_then(|store| store.errors.try_recv().ok())
    {
        state.storage_error = Some(error);
        changed = true;
    }
    if let Some(error) = state
        .ui_store
        .as_ref()
        .and_then(|store| store.errors.try_recv().ok())
    {
        state.storage_error = Some(error);
        changed = true;
    }
    let (memory_error, memory_healthy) = state.memory_storage_status();
    if let Some(error) = memory_error {
        state.storage_error = Some(error);
        changed = true;
    }
    if state.storage_error.is_some()
        && memory_healthy
        && state
            .store
            .as_ref()
            .is_some_and(crate::store::Store::healthy)
        && state
            .ui_store
            .as_ref()
            .is_some_and(crate::store::Store::healthy)
    {
        state.storage_error = None;
        changed = true;
    }
    changed |= state.poll_approval_sends();
    if state.approval_sends_pending() {
        tx.wake_after(Duration::from_millis(16));
    }
    if changed {
        state.checkpoint();
    }
    changed
}

pub const ZOOM_STEPS: [f32; 10] = [0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

fn next_counter(saved: &Value, maximum: usize) -> Result<usize, String> {
    let minimum = maximum.checked_add(1).ok_or("Invalid id counter.")?;
    let next = if saved.is_null() {
        minimum
    } else {
        usize::try_from(saved.as_u64().ok_or("Invalid id counter.")?)
            .map_err(|_| "Invalid id counter.")?
            .max(minimum)
    };
    if next == usize::MAX {
        return Err("Thread or project ids exhausted.".into());
    }
    Ok(next)
}

pub(crate) fn parse_delegation_target(args: &Value) -> Result<Option<DelegationTarget>, String> {
    let Some(target) = args.get("target") else {
        return Ok(None);
    };
    let object = target.as_object().ok_or(
        "target must be an object with harness or model, and optional provider and options.",
    )?;
    if let Some(key) = object.keys().find(|key| {
        !matches!(
            key.as_str(),
            "machine"
                | "environmentId"
                | "projectId"
                | "harness"
                | "provider"
                | "model"
                | "options"
        )
    }) {
        return Err(format!(
            "Unknown target setting {key}. Use harness, provider, model and options."
        ));
    }
    let harness = object
        .get("harness")
        .map(|value| {
            value
                .as_str()
                .filter(|id| crate::harness::valid_id(id))
                .map(str::to_owned)
                .ok_or("target.harness must be pi, codex, claude, or an installed ACP id.")
        })
        .transpose()?;
    remote_tasks::target_machine(args)?;
    if let Some(project) = object.get("projectId") {
        if project
            .as_u64()
            .is_none_or(|id| id == 0 || id >= usize::MAX as u64)
        {
            return Err(
                "target.projectId must be a positive project id on the named machine.".into(),
            );
        }
        if remote_tasks::target_machine(args)?.is_none_or(|machine| machine == "local") {
            return Err("target.projectId requires target.environmentId or target.machine.".into());
        }
        if args.get("workspace").is_some() {
            return Err("Pass target.projectId or workspace, not both.".into());
        }
    }
    if !object.contains_key("model")
        && harness.is_none()
        && remote_tasks::target_machine(args)?.is_none()
    {
        return Err("target.model is required when target is set without a harness.".into());
    }
    let model = object
        .get("model")
        .map(|value| {
            value.as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .ok_or(
            "target.model must be a non-empty exact model id from orchestrator_capabilities.",
        )
        })
        .transpose()?;
    let provider = object
        .get("provider")
        .map(|value| {
            value
                .as_str()
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .ok_or("target.provider must be a nonempty string.")
        })
        .transpose()?;
    let reasoning = match object.get("options") {
        None => None,
        Some(options) => {
            let options = options
                .as_object()
                .ok_or("target.options must be an object with reasoning, or its alias thinking.")?;
            let mut reasoning: Option<String> = None;
            for (key, value) in options {
                let level = match key.as_str() {
                    "reasoning" | "thinking" => value
                        .as_str()
                        .map(str::trim)
                        .filter(|level| !level.is_empty())
                        .ok_or_else(|| {
                            format!(
                                "target.options.{key} must be a non-empty reasoning level from orchestrator_capabilities."
                            )
                        })?,
                    _ => {
                        return Err(format!(
                            "Unknown target.options setting {key}. Use reasoning, or its alias thinking."
                        ));
                    }
                };
                if let Some(existing) = &reasoning
                    && existing != level
                {
                    return Err(
                        "target.options.reasoning and target.options.thinking disagree. Pass one level."
                            .into(),
                    );
                }
                reasoning = Some(level.to_owned());
            }
            reasoning
        }
    };
    Ok(Some(DelegationTarget {
        harness,
        provider,
        model,
        reasoning,
    }))
}

fn wait_seconds(args: &Value) -> Result<u64, String> {
    match args.get("wait_seconds") {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|&seconds| seconds <= 600)
            .ok_or_else(|| "wait_seconds must be an integer from 0 to 600.".into()),
    }
}

/// The closest supported zoom step, so a hand-edited index cannot pick an odd scale.
pub fn snap_zoom(zoom: f32) -> f32 {
    ZOOM_STEPS
        .iter()
        .copied()
        .min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs()))
        .unwrap_or(1.0)
}

/// The words the Shina MCP uses for a thread's status.
fn status_word(status: Status) -> &'static str {
    match status {
        Status::Ready => "idle",
        Status::Running => "working",
        Status::Waiting => "needs input",
        Status::Failed => "failed",
        Status::Paused => "paused",
        Status::Completed => "done",
    }
}

fn selection_value(thread: &Thread) -> Value {
    thread
        .effective_selection()
        .map_or(Value::Null, |selection| selection.to_value())
}

pub fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn content_text(content: &Value) -> String {
    let text = if let Some(text) = content.as_str() {
        text.to_owned()
    } else {
        content
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|block| block["type"] == "text")
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };
    memory::without_diff(&without_ansi(&text)).into()
}
/// Pi extensions colour their messages for a terminal. Drop the escape sequences (CSI such
/// as `ESC[38;2;144;144;144m`, OSC up to BEL or `ESC\`, and two-byte escapes) so they do not
/// show as stray characters.
fn without_ansi(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('\u{1b}') {
        return text.into();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next_if_eq(&'\\');
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out.into()
}
/// Pi keeps user images as typed content blocks. Import each into the local store so the
/// conversation can draw it again without retaining Pi's base64 copy in Shina state.
fn content_images(content: &Value) -> Vec<Attachment> {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block["type"] == "image")
                .filter_map(attachments::from_history_part)
                .collect()
        })
        .unwrap_or_default()
}
fn tool_detail(args: &Value) -> String {
    ["path", "file_path", "command", "pattern", "query"]
        .iter()
        .find_map(|key| args[key].as_str())
        .map(|s| truncate(s, 140))
        .unwrap_or_default()
}
pub fn truncate(text: &str, count: usize) -> String {
    let mut characters = text.chars();
    let mut result: String = characters.by_ref().take(count).collect();
    if characters.next().is_some() {
        result.push_str("...");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_threads_new_effect_joins_another_threads_streaming_checkpoint() {
        let mut state = State::headless(crate::store::directory().unwrap());
        state.demo = true;
        poll(&mut state, 0.0, &mut Tx::default());
        let (_a_commands, a_events) = fake_pi(&mut state.threads[0], Status::Running);
        let first = state.threads[0].id;
        let mut other = Thread::new(2);
        let (b_commands, _b_events) = fake_pi(&mut other, Status::Ready);
        other.queued_prompt = Some((Prompt::new("quiet dispatch".into(), vec![]), 0));
        state.threads.push(other);
        a_events.send(Event::Record(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"stream"}}))).unwrap();
        state.persistence_scope = Some(Default::default());
        let capture = crate::outbox::Capture::begin();
        assert!(poll(&mut state, 0.0, &mut Tx::default()));
        let effects = capture.finish();
        assert!(!effects.is_empty());
        assert!(
            b_commands.try_recv().is_err(),
            "effects must remain captured until commit"
        );
        let scope = state
            .persistence_scope
            .as_ref()
            .expect("only changed threads");
        assert!(scope.contains(&first));
        assert!(scope.contains(&2));
        assert!(state.threads[1].pending_prompt.is_some());
    }

    #[test]
    fn message_text_drops_terminal_escape_sequences() {
        let content = json!(
            "\u{1b}[38;2;144;144;144m✻ Worked for 6s\u{1b}[0m \u{1b}]8;;http://x\u{7}link\u{1b}]8;;\u{1b}\\ end"
        );
        assert_eq!(content_text(&content), "✻ Worked for 6s link end");
        assert_eq!(content_text(&json!("plain [0m text")), "plain [0m text");
    }

    #[test]
    fn harness_selection_and_awareness_round_trip_and_legacy_indexes_default_to_pi() {
        let mut state = State::new(PathBuf::new(), false);
        state.current_mut().harness = "fixture".into();
        state.current_mut().harness_context_sent = true;
        state.current_mut().selected_model = Some(("fixture".into(), "agent-model".into()));
        state.current_mut().session = Some("agent-session".into());
        let snapshot = state.snapshot();
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&snapshot).unwrap();
        assert_eq!(restored.current().harness, "fixture");
        assert!(restored.current().harness_context_sent);
        assert_eq!(
            restored.current().selected_model.as_ref().unwrap().0,
            "fixture"
        );
        assert_eq!(restored.current().session.as_deref(), Some("agent-session"));
        let mut legacy = snapshot.clone();
        legacy["threads"][0]
            .as_object_mut()
            .unwrap()
            .remove("harness");
        legacy["threads"][0]
            .as_object_mut()
            .unwrap()
            .remove("harnessContextSent");
        restored.restore_global(&legacy).unwrap();
        assert_eq!(restored.current().harness, "pi");
        assert!(!restored.current().harness_context_sent);
        for value in [json!(1), json!("bad id"), json!("")] {
            let mut invalid = snapshot.clone();
            invalid["threads"][0]["harness"] = value;
            assert!(restored.restore_global(&invalid).is_err());
        }
    }

    #[test]
    fn restore_rejects_invalid_relationships_and_wrong_typed_session_fields() {
        let state = State::new(PathBuf::new(), false);
        let baseline = state.snapshot();
        for (field, value) in [
            ("captainHome", json!(true)),
            ("delegated", json!(true)),
            ("session", json!(123)),
            ("model", json!(false)),
            ("thinking", json!(4)),
            ("updated", json!("bad")),
        ] {
            let mut index = baseline.clone();
            index["threads"][0][field] = value;
            let mut restored = State::new(PathBuf::new(), false);
            assert!(restored.restore_global(&index).is_err(), "{field}");
        }
    }

    #[test]
    fn ordinary_parents_children_are_visible_in_navigation_and_mcp_listing() {
        let mut state = State::new(PathBuf::new(), false);
        let (index, _commands, _events) = child(&mut state);
        assert!(state.sidebar_navigation().contains(&index));
        let listing = state
            .mcp_call(state.threads[0].id, "list_threads", &json!({}))
            .unwrap();
        assert!(
            listing["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|thread| thread["threadId"] == state.threads[index].id)
        );
    }

    #[test]
    fn completion_injections_wait_for_compaction_and_retry_a_compaction_race() {
        let mut thread = Thread::new(1);
        let (commands, _events) = fake_pi(&mut thread, Status::Running);
        thread.enqueue_injection("child finished".into());
        thread.apply(json!({"type":"compaction_start"}));
        assert!(!thread.flush_injection());
        assert!(commands.try_recv().is_err());
        thread.apply(json!({"type":"compaction_end"}));
        let prompt = commands
            .try_iter()
            .find(|command| command["type"] == "prompt")
            .unwrap();
        thread.apply(
            json!({"type":"response","id":prompt["id"],"command":"prompt",
            "success":false,"error":"Cannot submit a prompt while compaction is in progress."}),
        );
        assert_ne!(thread.status, Status::Failed);
        assert_eq!(thread.pending_injections.len(), 1);
        assert!(!thread.flush_injection());
        thread.injection_retry_at = Some(Instant::now());
        assert!(thread.flush_injection());
        let retry = commands.try_recv().unwrap();
        acknowledge(&mut thread, &retry);
        assert!(thread.pending_injections.is_empty());
    }

    #[test]
    fn configuration_deadlines_disconnect_unresponsive_sessions() {
        for operation in ["set_model", "set_thinking_level"] {
            let mut thread = Thread::new(1);
            let (_commands, _events) = fake_pi(&mut thread, Status::Running);
            thread.configuring = operation != "abort";
            thread.operation_deadline = Some((operation, Instant::now()));
            assert!(thread.check_deadlines(Instant::now()));
            assert!(thread.pi.is_none());
            assert!(!thread.configuring);
            assert!(!thread.status.busy());
            assert!(thread.error.is_some());
        }
    }

    #[test]
    fn diagnostics_are_bounded_redacted_and_truncated_at_ingestion() {
        let mut thread = Thread::new(1);
        for _ in 0..1000 {
            thread.diagnostic(&format!("ghp_secret {}", "x".repeat(9000)));
        }
        assert_eq!(thread.diagnostics.len(), 128);
        assert!(
            thread
                .diagnostics
                .iter()
                .all(|line| line.len() < 2100 && !line.contains("ghp_secret"))
        );
    }

    #[test]
    fn cancelling_a_captain_flow_clears_all_project_overrides() {
        let mut state = State::new(PathBuf::new(), false);
        state.projects.captain_target = Some(99);
        state.captain_project = Some(Some(99));
        state.captain_picker = true;
        state.cancel_captain_flow();
        assert!(state.projects.captain_target.is_none());
        assert!(state.captain_project.is_none());
        assert!(!state.captain_picker);
    }

    #[test]
    fn a_threads_process_owner_survives_disconnect_before_its_session_path_is_known() {
        let mut thread = Thread::new(1);
        let owner = thread.connection_owner.clone();
        assert!(thread.session.is_none());
        thread.disconnect();
        thread.session = Some("newly-learned-session.jsonl".into());
        assert!(Arc::ptr_eq(&owner, &thread.connection_owner));
    }

    #[test]
    fn a_specific_backend_error_survives_a_following_exit_event() {
        let mut state = State::new(PathBuf::new(), false);
        let (_commands, events) = fake_pi(&mut state.threads[0], Status::Ready);
        events
            .send(Event::Error("Pi was not found.".into()))
            .unwrap();
        events.send(Event::Exited).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.threads[0].error.as_deref(), Some("Pi was not found."));
    }

    #[test]
    fn captain_scope_search_and_keyboard_order_share_the_visible_hierarchy() {
        let mut state = State::new(".".into(), false);
        state.populate_demo();
        state.project_scope = ProjectScope::Project(1);
        assert_eq!(state.sidebar_captains(), vec![6]);
        assert!(
            state
                .sidebar_threads()
                .iter()
                .all(|i| state.threads[*i].project == Some(1))
        );
        state.threads[0].parent = Some(7);
        state.threads[0].delegated = true;
        state.threads[0].messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: "Pi",
            text: "A unique worker transcript phrase".into(),
            images: vec![],
            context: None,
        });
        state.search.replace_text("unique worker");
        assert_eq!(state.sidebar_captains(), vec![6]);
        assert_eq!(state.visible_captain_workers(7), vec![0]);
        assert_eq!(state.sidebar_navigation(), vec![6, 0]);
        state.search.replace_text("Juniper");
        assert!(
            state.sidebar_navigation().is_empty(),
            "search cannot escape workspace scope"
        );
        state.search.replace_text("");
        state.project_scope = ProjectScope::NoProject;
        assert!(state.sidebar_captains().is_empty());
        assert!(
            state
                .sidebar_threads()
                .iter()
                .all(|i| state.threads[*i].project.is_none())
        );
    }

    #[test]
    fn collapsed_captains_keep_all_workers_nested_even_when_they_need_attention() {
        let mut state = State::new(".".into(), false);
        state.populate_demo();
        state.project_scope = ProjectScope::Project(1);
        for index in [0, 1] {
            state.threads[index].parent = Some(7);
            state.threads[index].delegated = true;
        }
        // Thread 2 has an unread result; thread 1 has been read.
        state.threads[0].life.last_completed_at = Some(1);
        state.threads[0].life.last_visited_at = Some(2);
        let solo = state.threads.iter().position(|t| t.id == 6).unwrap();
        state.threads[solo].project = Some(1);
        state.select_thread(solo);
        assert!(!state.captain_expanded(7));
        assert!(state.visible_captain_workers(7).is_empty());
        assert_eq!(state.captain_worker_count(7), 2);
        state.threads[1].life.last_visited_at = state.threads[1].life.last_completed_at;
        assert!(state.visible_captain_workers(7).is_empty());
        state.threads[0].request = Some(serde_json::json!({"method": "confirm"}));
        assert!(state.visible_captain_workers(7).is_empty());
        assert!(state.row_pending(&state.threads[6]));
        assert_eq!(state.sidebar_navigation()[0], 6);
        assert!(!state.sidebar_navigation().contains(&0));
        state.threads[0].request = None;
        state.threads[0].status = Status::Failed;
        assert!(state.visible_captain_workers(7).is_empty());
        state.threads[0].status = Status::Ready;
        state.threads[0].request = Some(serde_json::json!({"method": "input"}));
        assert!(state.visible_captain_workers(7).is_empty());
        state.select_thread(6);
        assert!(state.captain_expanded(7));
        assert_eq!(state.visible_captain_workers(7), vec![0, 1]);
    }

    #[test]
    fn settling_a_worker_preserves_its_captain_while_solo_threads_use_the_settled_shelf() {
        let mut state = State::new(".".into(), false);
        state.populate_demo();
        state.threads[0].parent = Some(7);
        state.threads[0].delegated = true;
        state.select_thread(6);
        state.settle_thread(1);
        assert!(state.threads[0].life.settled());
        assert_eq!(state.threads[0].parent, Some(7));
        assert!(state.visible_captain_workers(7).contains(&0));
        assert!(!state.sidebar_threads().contains(&0));
        state.undo_thread_action();
        assert!(!state.threads[0].life.settled());
        state.settle_thread(6);
        assert!(state.threads[5].life.settled());
        assert_eq!(
            state.thread_section(&state.threads[5], lifecycle::now()),
            Section::Settled
        );
        state.shelves[1] = true;
        assert!(state.visible_sidebar_threads().contains(&5));
        state.settle_thread(6);
        assert!(!state.threads[5].life.settled());
    }

    #[test]
    fn archive_search_finds_worker_transcripts_without_archiving_the_captain() {
        let mut state = State::new(".".into(), false);
        state.populate_demo();
        state.threads[0].parent = Some(7);
        state.threads[0].delegated = true;
        state.threads[0].life.archived_at = Some(1);
        state.search.replace_text("duplicate listener");
        state.show_archive = true;
        assert_eq!(state.sidebar_captains(), vec![6]);
        assert_eq!(state.visible_captain_workers(7), vec![0]);
        assert!(!state.threads[6].archived());
        assert!(state.visible_sidebar_threads().is_empty());
        state.search.replace_text("not present");
        assert!(state.sidebar_navigation().is_empty());
    }

    #[test]
    fn restoring_preserves_profiles_and_stable_ids_including_untouched_presets() {
        let mut old = State::new(".".into(), false);
        old.captains = crate::captains::Captain::presets();
        let mut custom = old.captains[0].clone();
        custom.name = "Riku".into();
        old.captains.push(custom);
        // Sable (2) has a home; Riku (4) is custom; the other presets are untouched.
        old.open_captain(2);
        let sable_home = old.current().id;
        let mut index = old.snapshot();
        // Legacy indexes also keep ids stable. Migration must not delete unused profiles.
        index.as_object_mut().unwrap().remove("captainHomes");
        index.as_object_mut().unwrap().remove("captainScopes");
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&index).unwrap();
        let names: Vec<_> = restored.captains.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Ember", "Juniper", "Sable", "Pip", "Riku"]);
        let home = restored
            .threads
            .iter()
            .find(|t| t.id == sable_home)
            .unwrap();
        assert_eq!(home.captain, Some(2));
        assert!(
            restored
                .projects
                .entries
                .iter()
                .all(|p| p.captain.is_none_or(|id| id < 5))
        );
    }

    #[test]
    fn all_captain_homes_restore_without_demoting_or_settling_and_keep_their_crew() {
        let mut old = State::new(".".into(), false);
        // Rosters start empty; this test restores homes for the earlier named presets.
        old.captains = crate::captains::Captain::presets();
        // Text keeps the first thread's tab, so it is saved and restored with the homes.
        old.current_mut().draft.replace_text("Saved draft");
        for captain in 0..4 {
            old.open_captain(captain);
        }
        let homes: Vec<_> = old
            .threads
            .iter()
            .filter(|t| t.captain_home)
            .map(|t| t.id)
            .collect();
        for id in homes {
            let home = old.threads.iter_mut().find(|t| t.id == id).unwrap();
            home.session = Some(format!("original-home-{id}.jsonl"));
            let mut crew = Thread::new(old.next_id);
            old.next_id += 1;
            crew.project = home.project;
            crew.captain = home.captain;
            crew.parent = Some(id);
            crew.delegated = true;
            crew.session = Some(format!("original-crew-{id}.jsonl"));
            old.threads.push(crew);
        }
        let mut index = old.snapshot();
        index["version"] = 3.into();
        for t in index["threads"].as_array_mut().unwrap() {
            t.as_object_mut().unwrap().remove("lifecycle");
            t["pinned"] = true.into();
            t["unread"] = true.into();
        }
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&index).unwrap();
        assert_eq!(restored.threads.len(), old.threads.len());
        assert_eq!(
            restored.threads.iter().filter(|t| t.captain_home).count(),
            4
        );
        for (before, after) in old.threads.iter().zip(&restored.threads) {
            assert_eq!(
                (before.id, &before.session, before.parent),
                (after.id, &after.session, after.parent)
            );
            if after.captain_home {
                assert!(!after.life.settled());
                assert!(
                    !restored.sidebar_threads().contains(
                        &restored
                            .threads
                            .iter()
                            .position(|t| t.id == after.id)
                            .unwrap()
                    )
                );
                assert!(!after.pinned());
            }
        }
        restored.search.replace_text("Pip");
        assert!(restored.visible_sidebar_threads().is_empty());
        restored.sweep_lifecycle(lifecycle::now() + 100 * 86_400_000);
        assert!(
            restored
                .threads
                .iter()
                .filter(|t| t.captain_home)
                .all(|t| !t.life.settled())
        );
        // Opening an existing captain ignores a workspace hint. Scope changes are explicit.
        let original_home = restored.captain_home(0);
        restored.captain_project = Some(None);
        restored.open_captain(0);
        assert_eq!(Some(restored.current().id), original_home);
        restored.set_captain_scope(0, vec![]).unwrap();
        assert!(restored.current().project.is_none());
        let mut again = State::new(PathBuf::new(), false);
        again.restore_global(&restored.snapshot()).unwrap();
    }

    #[test]
    fn lifecycle_refusals_undo_invalidation_expiry_and_text_focus() {
        let mut state = State::new(".".into(), false);
        state.pin_thread(1);
        let key = state.current().life.pin_order_key.clone();
        state.settle_thread(1);
        assert!(state.threads[0].life.settled());
        assert_ne!(state.current().id, 1);
        state.undo_thread_action();
        assert!(state.threads[0].pinned());
        assert_eq!(state.threads[0].life.pin_order_key, key);
        assert_eq!(state.threads[0].life.settled_override, None); // re-pin clears the override
        state.pin_thread(1);
        state.pin_thread(1);
        assert!(
            state.undo.is_none(),
            "a new pin invalidates an old unpin undo"
        );
        state.threads[0].status = Status::Running;
        state.settle_thread(1);
        assert!(!state.threads[0].life.settled());
        state.threads[0].status = Status::Ready;
        state.pin_thread(1);
        let ctrl_z = rune::KeyEvent {
            key: rune::Key::Command('z'),
            mods: rune::Mods {
                cmd: true,
                ctrl: true,
                ..Default::default()
            },
        };
        assert!(
            state.undo_key(&ctrl_z, true),
            "fresh notice owns undo inside a field"
        );
        state.pin_thread(1);
        state.undo_key(
            &rune::KeyEvent {
                key: rune::Key::Text("a".into()),
                mods: Default::default(),
            },
            true,
        );
        assert!(
            !state.undo_key(&ctrl_z, true),
            "typing gives undo back to the text field"
        );
        state.undo.as_mut().unwrap().expires = 0;
        assert!(!state.undo_key(&ctrl_z, false));
        assert!(state.undo.is_none());
    }

    #[test]
    fn undo_restores_unpin_snooze_archive_and_discarded_draft() {
        let mut state = State::new(".".into(), false);
        let at = lifecycle::now();
        state.current_mut().life.pin(at, Some("nb".into()));
        state.current_mut().life.snooze(at, at + 3_600_000);
        state.pin_thread(1);
        state.undo_thread_action();
        assert_eq!(state.threads[0].life.pin_order_key.as_deref(), Some("nb"));
        assert_eq!(state.threads[0].life.snoozed_until, Some(at + 3_600_000));
        state.archive_thread(1);
        assert!(state.threads[0].archived());
        state.undo_thread_action();
        assert!(!state.threads[0].archived());
        assert_eq!(state.threads[0].life.pin_order_key.as_deref(), Some("nb"));
        let draft = state.current().id;
        state.current_mut().draft.replace_text("Keep the draft");
        state.discard_draft(draft);
        state.undo_thread_action();
        assert_eq!(
            state
                .threads
                .iter()
                .find(|t| t.id == draft)
                .unwrap()
                .draft
                .text,
            "Keep the draft"
        );
    }

    #[test]
    fn early_snooze_wake_rolls_up_approval_but_not_crew_questions() {
        let mut state = State::new(".".into(), false);
        let mut crew = Thread::new(2);
        crew.delegated = true;
        crew.parent = Some(1);
        crew.request = Some(json!({"method":"input"}));
        state.threads.push(crew);
        assert!(!state.row_pending(&state.threads[0]));
        state.threads[1].request = Some(json!({"method":"confirm"}));
        assert!(state.row_pending(&state.threads[0]));
        state.threads[0].life.snooze(1, lifecycle::now() + 100000);
        poll(&mut state, 0.0, &mut Tx::default());
        assert!(!state.threads[0].life.snoozed(lifecycle::now()));
        assert!(state.threads[0].life.last_snooze_wake_at.is_some());
    }
    #[test]
    fn crew_questions_reach_the_coordinator_once_and_task_send_answers_without_approving() {
        let mut state = State::new(".".into(), false);
        let (parent_commands, _parent_events) = fake_pi(&mut state.threads[0], Status::Ready);
        let (index, child_commands, events) = child(&mut state);
        let request = json!({"type":"extension_ui_request","id":"question","method":"select","title":"Which file?","options":["a.rs","b.rs"]});
        events.send(Event::Record(request.clone())).unwrap();
        events.send(Event::Record(request)).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.threads[0].pending_injections.len(), 1);
        assert!(
            parent_commands.try_recv().unwrap()["message"]
                .as_str()
                .unwrap()
                .contains("crew #2 asks")
        );
        assert!(!state.row_pending(&state.threads[0]));
        assert!(
            state
                .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"invalid"}))
                .is_err()
        );
        state
            .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"b.rs"}))
            .unwrap();
        let response = child_commands.try_recv().unwrap();
        assert_eq!(response["type"], "extension_ui_response");
        assert_eq!(response["value"], "b.rs");
        assert!(state.threads[index].request.is_none());
        state.threads[index].apply(json!({"type":"extension_ui_request","id":"approval","method":"confirm","title":"Run tool?"}));
        state
            .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"yes"}))
            .unwrap();
        assert!(
            state.threads[index].request.is_some(),
            "a coordinator cannot approve for the user"
        );
        assert!(
            child_commands
                .try_iter()
                .all(|r| r["type"] != "extension_ui_response")
        );
    }

    #[test]
    fn ui_state_is_separate_and_corrupt_lifecycle_never_enables_saving() {
        let mut state = State::new(".".into(), false);
        state.shelves = [false, true];
        state.sidebar = false;
        state
            .current_mut()
            .draft
            .replace_text("Keep this unsent draft");
        state.set_project_scope(Some(1), false);
        let mut restored = State::new(".".into(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        restored.restore_ui(&state.ui_snapshot()).unwrap();
        assert_eq!(restored.current().draft.text, "Keep this unsent draft");
        assert_eq!(restored.shelves, [false, true]);
        assert!(!restored.sidebar);
        assert!(state.snapshot().get("projectScope").is_none());
        let mut damaged = state.snapshot();
        damaged["threads"][0]["lifecycle"]["pinnedAt"] = "bad".into();
        let path = state.data_dir.join("index.json");
        std::fs::write(&path, damaged.to_string()).unwrap();
        let original = std::fs::read(&path).unwrap();
        state.launch_project = None;
        state.load_global(Arc::new(|| {}));
        assert!(state.storage_error.is_some());
        assert!(state.store.is_none());
        state.checkpoint();
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[test]
    fn blank_drafts_never_hold_a_sidebar_row_or_accumulate() {
        let mut state = State::new(".".into(), false);
        let workspace = state.data_dir.join("blank-drafts");
        state.projects.entries.push(Project::new(2, workspace));
        assert!(state.sidebar_threads().is_empty());
        for _ in 0..3 {
            state.new_thread();
        }
        state.new_thread_in(Some(2));
        state.new_thread_in(None);
        assert_eq!(
            state.threads.len(),
            3,
            "one blank draft per project, including no project"
        );
        assert!(state.sidebar_threads().is_empty(), "no empty Drafts rows");
        state.current_mut().draft.replace_text("A first prompt");
        let active = state.active;
        assert!(
            state.sidebar_threads().is_empty(),
            "the open draft's row does not pop in while typing"
        );
        state.select_thread((active + 1) % state.threads.len());
        assert_eq!(state.sidebar_threads(), vec![active]);
        state.select_thread(active);
        state.current_mut().draft.replace_text("");
        assert_eq!(
            state.sidebar_threads(),
            vec![active],
            "the open draft keeps its row while its composer is cleared"
        );
        state.discard_draft(state.current().id);
        assert_eq!(state.threads.len(), 2, "the blank thread is gone");
        assert!(
            state.undo.is_some(),
            "discarding a blank draft can restore its tab"
        );
        assert!(state.sidebar_threads().is_empty());
        state.show_archive = true;
        assert!(state.sidebar_threads().is_empty(), "nothing was archived");
        assert!(state.tabs.contains(&state.current().id));
    }

    #[test]
    fn reusing_a_blank_draft_navigates_and_saves_it_like_a_new_one() {
        let mut state = State::new(".".into(), false);
        let a = state.data_dir.join("project-a");
        let b = state.data_dir.join("project-b");
        state.projects.entries.push(Project::new(2, a));
        state.projects.entries.push(Project::new(3, b));
        state.new_thread_in(Some(2));
        let draft = state.current().id;
        state.new_thread_in(Some(3));
        state.current_mut().draft.replace_text("Project B work");
        state.project_scope = ProjectScope::Project(3);
        state.open_picker(ProjectPicker::NewThread);
        state.new_thread_in(Some(2));
        assert_eq!(state.current().id, draft, "the blank draft is reused");
        assert!(state.project_picker.is_none());
        assert_eq!(state.project_scope, ProjectScope::All);
        assert!(state.tabs.contains(&draft));
        let saved = state.snapshot();
        assert!(
            saved["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == draft),
            "the open blank draft is saved with the active id and tabs that name it"
        );
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&saved).unwrap();
        assert_eq!(restored.current().id, draft);
    }

    #[test]
    fn restored_blank_draft_stays_unlisted_and_named_drafts_keep_a_row() {
        let state = State::new(".".into(), false);
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        restored.restore_ui(&state.ui_snapshot()).unwrap();
        restored
            .current_mut()
            .draft
            .replace_text("Typing after launch");
        assert!(
            restored.sidebar_threads().is_empty(),
            "a restored blank draft's row does not pop in while typing"
        );

        let mut state = State::new(".".into(), false);
        let named = state.current().id;
        state.current_mut().title = "Plan the release".into();
        state.current_mut().renamed = true;
        state.current_mut().draft.replace_text("x");
        state.new_thread_in(None);
        state.threads[0].draft.replace_text("");
        state.close_tab(named);
        assert!(state.threads.iter().any(|t| t.id == named));
        assert!(
            state
                .sidebar_threads()
                .iter()
                .any(|&i| state.threads[i].id == named),
            "a named empty thread stays reachable from the sidebar"
        );
    }

    #[test]
    fn closing_a_blank_drafts_tab_removes_it_and_keeps_a_tab() {
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("A prompt");
        state.select_thread(0);
        state.new_thread_in(None);
        state.current_mut().draft.replace_text("Keep this draft");
        state.select_thread(0);
        state.open_picker(ProjectPicker::NewThread);
        state.new_thread_in(Some(state.projects.entries.len()));
        let blank = state.current().id;
        assert!(state.current().blank_draft());
        let written = 2;
        assert_eq!(state.tabs, vec![1, blank, written]);
        state.close_tab(blank);
        assert!(!state.threads.iter().any(|t| t.id == blank));
        assert_eq!(state.tabs, vec![1, written]);
        assert_eq!(state.current().draft.text, "Keep this draft");
        state.select_thread(1);
        assert_eq!(state.current().draft.text, "Keep this draft");
        state.close_tab(written);
        assert!(state.threads.iter().any(|t| t.id == written));
        assert_eq!(state.current().id, 1);
        assert_eq!(state.current().draft.text, "A prompt");
        state.close_tab(1);
        assert_eq!(state.tabs, vec![1], "the tab strip keeps one tab");
        assert!(state.undo.is_none());
    }

    #[test]
    fn leaving_a_blank_draft_keeps_its_tab_and_reopening_reuses_it() {
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("A prompt");
        let written = state.current().id;
        let workspace = state.data_dir.join("blank-tab");
        state.projects.entries.push(Project::new(2, workspace));
        state.new_thread_in(Some(2));
        let blank = state.current().id;
        assert_eq!(state.tabs, vec![written, blank]);
        state.select_thread(0);
        assert_eq!(state.tabs, vec![written, blank]);
        assert!(
            state.snapshot()["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == blank),
            "an open blank draft is saved"
        );
        for _ in 0..3 {
            state.new_thread_in(Some(2));
            state.select_thread(0);
        }
        assert_eq!(
            state.tabs,
            vec![written, blank],
            "repeated new threads never pile up tabs"
        );
        state.new_thread_in(Some(2));
        assert_eq!(state.current().id, blank, "the workspace's draft is reused");
        state.current_mut().draft.replace_text("Typed");
        state.select_thread(0);
        assert_eq!(
            state.tabs,
            vec![written, blank],
            "a draft with text keeps its tab"
        );
    }

    fn tab_fixture() -> State {
        let mut state = State::new(".".into(), false);
        state.threads = (1..=5)
            .map(|id| {
                let mut thread = Thread::new(id);
                thread.title = format!("Thread {id}");
                thread.renamed = true;
                thread.project = Some(1);
                thread
            })
            .collect();
        state.next_id = 6;
        state.tabs = vec![1, 2, 3, 4];
        state.active = 1;
        state
    }

    #[test]
    fn tab_insertion_and_neighbours_are_stable() {
        let mut tabs = vec![1, 2, 3];
        insert_tab(&mut tabs, 1, 4);
        assert_eq!(tabs, [1, 4, 2, 3]);
        insert_tab(&mut tabs, 4, 3);
        assert_eq!(tabs, [1, 4, 2, 3]);
        assert_eq!(tab_neighbour(&tabs, 4), Some(2));
        assert_eq!(tab_neighbour(&tabs, 3), Some(2));
        assert_eq!(tab_neighbour(&[1], 1), None);
        assert_eq!(tab_neighbour(&tabs, 99), None);
    }

    #[test]
    fn explicit_opens_insert_next_to_active_and_existing_tabs_never_move() {
        let mut state = tab_fixture();
        state.select_thread(4);
        assert_eq!(state.tabs, [1, 2, 5, 3, 4]);
        state.select_thread(0);
        assert_eq!(state.tabs, [1, 2, 5, 3, 4]);
        assert_eq!(state.current().id, 1);
    }

    #[test]
    fn closing_middle_rightmost_and_inactive_tabs_uses_neighbours() {
        let mut state = tab_fixture();
        state.close_tab(2);
        assert_eq!(state.current().id, 3);
        assert_eq!(state.tabs, [1, 3, 4]);
        state.close_tab(1);
        assert_eq!(state.current().id, 3);
        state.select_thread(3);
        state.close_tab(4);
        assert_eq!(state.current().id, 3);
        assert_eq!(state.tabs, [3]);
        state.close_tab(3);
        assert_eq!(state.tabs, [3]);
    }

    #[test]
    fn parking_uses_open_neighbours_and_undo_restores_the_old_slot() {
        for action in [
            Action::Settle,
            Action::Archive,
            Action::Discard,
            Action::Delete,
        ] {
            let mut state = tab_fixture();
            state.current_mut().draft.replace_text("Keep this text");
            match action {
                Action::Settle => state.settle_thread(2),
                Action::Archive => state.archive_thread(2),
                Action::Discard => state.discard_draft(2),
                Action::Delete => {
                    state.request_delete(2);
                    state.confirm_delete();
                }
                _ => unreachable!(),
            }
            assert_eq!(state.tabs, [1, 3, 4], "{action:?}");
            assert_eq!(state.current().id, 3, "{action:?}");
            assert!(!state.tabs.contains(&5));
            state.undo_thread_action();
            assert_eq!(state.tabs, [1, 2, 3, 4], "{action:?}");
            assert_eq!(state.current().id, 2, "{action:?}");
            assert_eq!(state.current().draft.text, "Keep this text");
        }
    }

    #[test]
    fn last_settle_tab_replaces_itself_with_the_next_sidebar_row() {
        let mut state = tab_fixture();
        state.tabs = vec![2];
        state.settle_thread(2);
        let order = state.sidebar_threads();
        let position = order
            .iter()
            .position(|i| state.threads[*i].id == 2)
            .unwrap();
        let expected = state.threads[order[(position + 1) % order.len()]].id;
        assert_eq!(state.tabs, [expected]);
        assert_eq!(state.current().id, expected);
        state.undo_thread_action();
        assert_eq!(state.tabs, [2, expected]);
        assert_eq!(state.current().id, 2);
    }

    #[test]
    fn last_archive_discard_and_delete_tab_get_a_same_project_draft() {
        for action in [Action::Archive, Action::Discard, Action::Delete] {
            let mut state = tab_fixture();
            state.tabs = vec![2];
            state.current_mut().draft.replace_text("Keep");
            match action {
                Action::Archive => state.archive_thread(2),
                Action::Discard => state.discard_draft(2),
                Action::Delete => {
                    state.request_delete(2);
                    state.confirm_delete();
                }
                _ => unreachable!(),
            }
            assert!(state.current().blank_draft(), "{action:?}");
            assert_eq!(state.current().project, Some(1));
            assert_eq!(state.tabs, [state.current().id]);
        }
    }

    #[test]
    fn archiving_or_discarding_an_empty_draft_can_undo_its_tab_removal() {
        for action in [Action::Archive, Action::Discard] {
            let mut state = State::new(".".into(), false);
            let old = state.current().id;
            if action == Action::Archive {
                state.archive_thread(old);
            } else {
                state.discard_draft(old);
            }
            assert!(!state.threads.iter().any(|t| t.id == old));
            assert_eq!(state.undo.as_ref().unwrap().action, action);
            state.undo_thread_action();
            assert_eq!(state.current().id, old);
            assert_eq!(state.tabs[0], old);
            assert!(state.current().blank_draft());
        }
    }

    #[test]
    fn background_settle_and_crew_creation_do_not_open_tabs_or_move_focus() {
        let mut state = tab_fixture();
        let before = (state.tabs.clone(), state.current().id);
        state.sweep_lifecycle(lifecycle::now() + 100 * 86_400_000);
        state.demo = true;
        state
            .mcp_call(2, "delegate_task", &json!({"prompt":"A background worker"}))
            .unwrap();
        assert_eq!((state.tabs.clone(), state.current().id), before);
    }

    #[test]
    fn new_thread_inherits_valid_composer_selection_and_rejects_stale_models() {
        let mut state = tab_fixture();
        let model = AgentModel {
            provider: "codex".into(),
            id: "chosen".into(),
            name: "Chosen".into(),
            thinking_levels: vec!["high".into()],
            context_window: Some(100),
            supports_images: Some(true),
            service_tiers: vec![],
            ..Default::default()
        };
        state.current_mut().harness = "codex".into();
        state.current_mut().models = vec![model];
        state.current_mut().selected_model = Some(("codex".into(), "chosen".into()));
        state.current_mut().selected_thinking = Some("high".into());
        state.new_thread_in(Some(1));
        let draft = state.current().id;
        assert_eq!(state.tabs, [1, 2, draft, 3, 4]);
        assert_eq!(state.current().harness, "codex");
        assert_eq!(
            state.current().selected_model,
            Some(("codex".into(), "chosen".into()))
        );
        assert_eq!(state.current().selected_thinking.as_deref(), Some("high"));
        state.current_mut().draft.replace_text("Occupied");
        state.current_mut().selected_model = Some(("codex".into(), "removed".into()));
        state.new_thread_in(Some(1));
        assert_eq!(state.current().harness, state.default_harness);
        assert_eq!(state.current().selected_model, None);
    }

    #[test]
    fn new_threads_keep_default_effort_when_the_inherited_effort_is_invalid() {
        let mut state = tab_fixture();
        let model = AgentModel::from_value(
            &json!({"provider":"cpa","id":"chosen","name":"Chosen","reasoning":true}),
        )
        .unwrap();
        state.catalog.models = vec![model.clone()];
        state.catalog.default_model = Some(model);
        state.catalog.default_thinking = "low".into();
        state.current_mut().selected_model = Some(("cpa".into(), "chosen".into()));
        state.current_mut().selected_thinking = Some("invalid".into());
        state.new_thread_in(None);
        assert_eq!(
            state.current().selected_model,
            Some(("cpa".into(), "chosen".into()))
        );
        assert_eq!(state.current().thinking, "low");
        assert_eq!(state.current().selected_thinking, None);
    }

    #[test]
    fn an_attachment_only_draft_survives_its_tab_closing_locally_and_in_client_mode() {
        for client in [false, true] {
            let mut state = tab_fixture();
            state.threads[1] = Thread::new(2);
            state.threads[1]
                .draft_images
                .push(test_attachment("only.png", 1));
            if client {
                state.client = Some(crate::client::Client::new(state.data_dir.clone()));
            }
            state.close_tab(2);
            assert_eq!(state.tabs, [1, 3, 4]);
            assert_eq!(state.current().id, 3);
            assert_eq!(
                state
                    .threads
                    .iter()
                    .find(|t| t.id == 2)
                    .unwrap()
                    .draft_images
                    .len(),
                1
            );
        }
    }

    #[test]
    fn restoring_a_session_preserves_blank_tabs_and_the_active_tab() {
        let mut state = tab_fixture();
        state.new_thread_in(None);
        let blank = state.current().id;
        state.select_thread(1);
        let mut restored = State::new(".".into(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        assert_eq!(restored.tabs, state.tabs);
        assert!(restored.tabs.contains(&blank));
        assert_eq!(restored.current().id, 2);
    }

    #[test]
    fn restoring_without_a_live_active_thread_lands_on_the_recent_workspace_draft() {
        let mut state = State::new(".".into(), false);
        let old = state.data_dir.join("old-workspace");
        let recent = state.data_dir.join("recent-workspace");
        let mut old = Project::new(1, old);
        old.updated = 1;
        let mut recent = Project::new(2, recent);
        recent.updated = 2;
        state.projects.entries = vec![old, recent];
        state.projects.next_id = 3;
        state.threads[0].project = Some(1);
        state.threads[0].life.pinned_at = Some(1);
        state.archive_thread(1);
        let mut saved = state.snapshot();
        // Every saved thread is archived, as in a profile whose live drafts were never saved.
        saved["threads"]
            .as_array_mut()
            .unwrap()
            .retain(|t| !t["lifecycle"]["archivedAt"].is_null());
        saved["active"] = json!(1);
        saved["tabs"] = json!([1]);
        for _ in 0..3 {
            let mut restored = State::new(PathBuf::new(), false);
            restored.restore_global(&saved).unwrap();
            assert!(
                !restored.current().archived(),
                "an archived thread is not reopened"
            );
            assert_eq!(restored.current().project, Some(2));
            assert!(restored.current().blank_draft());
            assert_eq!(restored.tabs, vec![restored.current().id]);
            saved = restored.snapshot();
        }
    }

    #[test]
    fn discarding_a_draft_with_content_archives_it_and_undo_restores_it() {
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("Keep me");
        state
            .current_mut()
            .draft_images
            .push(test_attachment("kept.png", 1));
        assert_eq!(state.sidebar_threads(), vec![0]);
        state.discard_draft(1);
        assert!(
            state.threads.iter().any(|t| t.id == 1 && t.archived()),
            "a written draft stays as an archive record"
        );
        assert!(state.threads[0].archived());
        assert!(state.undo.is_some(), "a draft with content offers Undo");
        assert!(state.sidebar_threads().is_empty());
        assert!(state.current().draft_images.is_empty());
        state.undo_thread_action();
        assert!(!state.threads[0].archived());
        assert_eq!(state.threads[0].draft.text, "Keep me");
        assert_eq!(state.threads[0].draft_images.len(), 1);
        assert_eq!(state.sidebar_threads(), vec![0]);
    }

    #[test]
    fn parked_new_threads_leave_nothing_blank_to_save_or_restore() {
        let mut state = State::new(".".into(), false);
        state.archive_thread(1);
        assert!(state.current().blank_draft());
        let slot = state.current().id;
        for _ in 0..3 {
            state.show_archive = false;
            state.new_thread();
        }
        assert_eq!(state.threads.iter().filter(|t| !t.archived()).count(), 1);
        assert_eq!(state.current().id, slot, "the same slot is reopened");
        let saved = state.snapshot();
        assert_eq!(
            saved["threads"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|t| t["title"] == "New thread")
                .count(),
            1,
            "one blank draft is saved"
        );
        // Restoring never lists it or opens a second tab for it.
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&saved).unwrap();
        assert!(restored.sidebar_threads().is_empty());
        assert_eq!(restored.tabs, vec![restored.current().id]);
        assert!(saved["nextId"].as_u64().unwrap() > 1, "ids are not reused");
        // Restoring never lists the slot, and reloading the restored index never adds another.
        let mut reloaded = State::new(PathBuf::new(), false);
        reloaded.restore_global(&saved).unwrap();
        assert_eq!(
            reloaded.threads.len(),
            1,
            "a restart restores one blank draft"
        );
        reloaded.restore_global(&reloaded.snapshot()).unwrap();
        assert_eq!(
            reloaded.threads.len(),
            1,
            "blank drafts do not accumulate across restarts"
        );
        assert!(reloaded.sidebar_threads().is_empty());
    }

    #[test]
    fn multi_project_scope_persists_and_opening_a_hidden_thread_widens_it() {
        let mut state = State::new(".".into(), false);
        state
            .projects
            .entries
            .push(Project::new(2, state.data_dir.join("other-project")));
        state.current_mut().draft.replace_text("Work in one");
        state.new_thread_in(Some(2));
        state.current_mut().draft.replace_text("Work in two");
        state.new_thread_in(None);
        state.set_project_scope(Some(1), false);
        state.set_project_scope(Some(2), true);
        assert_eq!(state.sidebar_threads().len(), 2);
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        restored.restore_ui(&state.ui_snapshot()).unwrap();
        assert_eq!(restored.sidebar_threads().len(), 2);
        restored.select_thread(2);
        assert_eq!(restored.project_scope, ProjectScope::All);
        assert!(restored.scope_additional.is_empty());
        for (i, t) in restored.threads.iter_mut().enumerate() {
            t.life.created_at = (3 - i) as u64;
        }
        restored.click_thread(
            0,
            rune::Mods {
                cmd: true,
                ..Default::default()
            },
        );
        restored.click_thread(
            1,
            rune::Mods {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(restored.selected_threads.len(), 2);
    }

    use crate::update::test_exit_guard as exit_record_guard;

    /// A verified update, as the updater would have prepared it.
    fn prepared(tag: &str) -> crate::update::PreparedUpdate {
        let root = crate::store::directory().unwrap().join("prepared");
        crate::update::PreparedUpdate {
            tag: tag.into(),
            target: crate::install::InstalledTarget::Windows {
                exe: root.join("shina.exe"),
            },
            asset: "shina.exe".into(),
            size: 1,
            sha256: "0".repeat(64),
            download: root.join("download.exe"),
            prepared: root.join("prepared.exe"),
        }
    }

    fn ready_state() -> State {
        let mut state = State::new(PathBuf::new(), false);
        state.runtime_enabled = true;
        state
    }

    #[test]
    fn running_waiting_queued_and_cloning_threads_block_an_update_restart() {
        let mut state = ready_state();
        assert_eq!(state.update_restart_blocker(), None);
        for status in [Status::Running, Status::Waiting] {
            state.threads[0].status = status;
            assert!(
                state.update_restart_blocker().is_some(),
                "{status:?} blocks"
            );
        }
        state.threads[0].status = Status::Ready;
        state.threads[0].configuring = true;
        assert!(state.update_restart_blocker().is_some());
        state.threads[0].configuring = false;
        state.threads[0].session_started = Some(Instant::now());
        assert!(state.update_restart_blocker().is_some());
        state.threads[0].session_started = None;
        state.threads[0].queued_prompt = Some((
            Prompt {
                text: "queued".into(),
                draft: None,
                context: None,
                images: Default::default(),
            },
            0,
        ));
        assert!(state.update_restart_blocker().is_some());
        state.threads[0].queued_prompt = None;
        state.threads[0].request = Some(json!({"id":"1","method":"confirm"}));
        assert!(state.update_restart_blocker().is_some());
        state.threads[0].request = None;
        // Delegated and hidden threads count too.
        let mut hidden = Thread::new(2);
        hidden.status = Status::Running;
        hidden.delegated = true;
        hidden.life.settle(lifecycle::now());
        state.threads.push(hidden);
        assert!(state.update_restart_blocker().is_some());
        state.threads.pop();
        // A clone in progress is active work.
        state.projects.clone_job = Some(crate::projects::CloneJob::test_idle());
        assert!(state.update_restart_blocker().is_some());
        state.projects.clone_job = None;
        state.rename_thread = Some(1);
        assert!(state.update_restart_blocker().is_some());
        state.rename_thread = None;
        // A composer draft is saved, so it never blocks.
        state.threads[0].draft.replace_text("unsent");
        assert_eq!(state.update_restart_blocker(), None);
    }

    #[test]
    fn thread_defaults_survive_the_index_and_only_apply_without_inheritance() {
        let mut state = State::new(PathBuf::new(), false);
        state.default_harness = "codex".into();
        state.thread_defaults.access = "approval-required".into();
        state.thread_defaults.snooze_minutes = vec![30, 90];
        state.thread_defaults.models.insert(
            "codex".into(),
            crate::ui::settings_ui::ModelDefault {
                provider: "openai".into(),
                model: "fixture-model".into(),
                effort: "high".into(),
            },
        );
        state.create_thread_in(None, &[], &Value::Null);
        assert_eq!(state.current().harness, "codex");
        assert_eq!(state.current().native.access, AccessMode::Supervised);
        assert_eq!(
            state.current().selected_model,
            Some(("openai".into(), "fixture-model".into()))
        );
        assert_eq!(state.current().selected_thinking.as_deref(), Some("high"));
        let saved = state.snapshot();
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&saved).unwrap();
        assert_eq!(restored.thread_defaults.snooze_minutes, vec![30, 90]);
        restored.threads.push(Thread::new(99));
        restored.threads[0].harness = "claude".into();
        restored.threads[0].models = vec![AgentModel {
            provider: "claude".into(),
            id: "inherited".into(),
            name: "Inherited".into(),
            thinking_levels: vec!["high".into()],
            context_window: None,
            supports_images: None,
            service_tiers: vec![],
            ..Default::default()
        }];
        let index = restored.threads.len() - 1;
        restored.inherit_composer(
            index,
            &json!({"harness":"claude","access":"read-only","model":null}),
        );
        assert_eq!(restored.threads[index].native.access, AccessMode::ReadOnly);
        restored.inherit_composer(
            index,
            &json!({"harness":"claude","access":"read-only","model":["claude","inherited"],"thinking":"high"}),
        );
        assert_eq!(restored.threads[index].harness, "claude");
        assert_eq!(restored.threads[index].native.access, AccessMode::ReadOnly);
        assert_eq!(
            restored.threads[index].selected_model,
            Some(("claude".into(), "inherited".into()))
        );
        restored.preferences.confirm_archive = true;
        restored.preferences.confirm_delete = false;
        let ui = restored.ui_snapshot();
        state.restore_ui(&ui).unwrap();
        assert!(state.preferences.confirm_archive);
        assert!(!state.preferences.confirm_delete);
    }
    #[test]
    fn desktop_update_blockers_belong_to_local_even_while_reading_a_remote_thread() {
        let mut state = ready_state();
        state.client = Some(crate::client::Client::new(state.data_dir.clone()));
        state.machines.selected = "remote".into();
        state.current_mut().status = Status::Running;
        state
            .machines
            .views
            .insert("local".into(), json!({"updateStatus":{"blocker":null}}));
        assert_eq!(state.update_restart_blocker(), None);
        state.machines.views.get_mut("local").unwrap()["updateStatus"]["blocker"] =
            json!("2 local threads are running");
        assert_eq!(
            state.update_restart_blocker(),
            Some("2 local threads are running")
        );
        state.rename_thread = Some(1);
        assert_eq!(
            state.update_restart_blocker(),
            Some("Finish editing before restarting.")
        );
    }

    #[test]
    fn becoming_idle_never_restarts_on_its_own() {
        let _exit_record = exit_record_guard();
        let mut state = ready_state();
        fake_pi(&mut state.threads[0], Status::Running);
        state.threads[0].draft.replace_text("work");
        state.threads[0].queued_prompt = Some((
            Prompt {
                draft: None,
                context: None,
                text: "work".into(),
                images: Default::default(),
            },
            0,
        ));
        poll(&mut state, 0.0, &mut Tx::default());
        assert!(!state.take_restart_request());
        // The run settles; a normal poll must not restart the app.
        state.threads[0].queued_prompt = None;
        state.threads[0].pi = None;
        state.threads[0].status = Status::Ready;
        let mut tx = Tx::default();
        poll(&mut state, 0.0, &mut tx);
        assert!(!state.take_restart_request());
        assert_eq!(state.threads[0].status, Status::Ready);
        assert!(crate::update::take_exit().is_none());
    }

    #[test]
    fn a_restart_needs_a_successful_save_and_a_normal_quit_only_saves() {
        let _exit_record = exit_record_guard();
        let _ = crate::update::take_exit();
        let mut state = State::new(PathBuf::new(), false);
        // No stores were started, so the save fails and nothing is recorded.
        state.updater = crate::update::Updater::test_ready(prepared("v0.10.0"));
        assert!(state.prepared_exit(true, None).is_err());
        assert!(crate::update::take_exit().is_none());
        // A normal quit saves but never approves installation.
        let mut state = ready_state();
        state.load_global_for_test();
        let update = prepared("v0.10.1");
        state.updater = crate::update::Updater::test_ready(update);
        state
            .prepared_exit(false, None)
            .expect("a ready update with no work saves");
        assert!(crate::update::take_exit().is_none());
        // A busy normal quit also saves without approving an install.
        state.threads[0].status = Status::Running;
        state.updater = crate::update::Updater::test_ready(prepared("v0.10.2"));
        state.prepared_exit(false, None).unwrap();
        assert!(crate::update::take_exit().is_none());
        // A normal quit with no ready update still saves and flushes, and records nothing.
        state.threads[0].status = Status::Ready;
        state.updater = crate::update::Updater::test_idle();
        state
            .current_mut()
            .draft
            .replace_text("saved without an update");
        state
            .prepared_exit(false, None)
            .expect("a normal quit flushes even without an update");
        assert!(crate::update::take_exit().is_none());
        let saved = crate::store::load(&crate::store::directory().unwrap().join("ui-state.json"))
            .unwrap()
            .unwrap();
        assert_eq!(
            saved["drafts"][state.current().id.to_string()],
            "saved without an update"
        );
        // Approval records one restart, independent of the repeated quit saves.
        state.updater = crate::update::Updater::test_ready(prepared("v0.10.3"));
        state.prepared_exit(true, None).unwrap();
        assert!(state.take_restart_request());
        assert!(!state.take_restart_request());
        let _ = crate::update::take_exit();
    }

    #[test]
    fn an_explicit_restart_requires_no_active_work_and_relaunches() {
        let _exit_record = exit_record_guard();
        let _ = crate::update::take_exit();
        let mut state = ready_state();
        state.load_global_for_test();
        state.updater = crate::update::Updater::test_ready(prepared("v0.11.0"));
        state.threads[0].status = Status::Waiting;
        let mut reply = String::new();
        assert!(state.prepared_exit(true, Some(&mut reply)).is_err());
        assert!(reply.contains("thread"));
        assert!(crate::update::take_exit().is_none());
        state.threads[0].status = Status::Ready;
        state
            .prepared_exit(true, Some(&mut reply))
            .expect("an idle app restarts");
        let exit = crate::update::take_exit().expect("the restart is recorded");
        assert!(exit.relaunch);
    }

    #[test]
    fn a_failed_index_write_blocks_restart_but_still_flushes_the_ui_store() {
        let _exit_record = exit_record_guard();
        let mut state = ready_state();
        state.load_global_for_test();
        state.checkpoint_and_flush().unwrap();
        let root = crate::store::directory().unwrap();
        std::fs::remove_file(root.join("index.json")).unwrap();
        std::fs::create_dir(root.join("index.json")).unwrap();
        state.current_mut().title = "force a new index write".into();
        state
            .current_mut()
            .draft
            .replace_text("draft saved despite index failure");
        state.updater = crate::update::Updater::test_ready(prepared("v99.0.0"));
        assert!(state.prepared_exit(true, None).is_err());
        assert!(!crate::update::exit_recorded());
        let saved = crate::store::load(&root.join("ui-state.json"))
            .unwrap()
            .unwrap();
        assert_eq!(
            saved["drafts"][state.current().id.to_string()],
            "draft saved despite index failure"
        );
    }

    #[test]
    fn a_blocked_restart_click_is_dropped_instead_of_restarting_later() {
        let _exit_record = exit_record_guard();
        let _ = crate::update::take_exit();
        let mut state = ready_state();
        state.load_global_for_test();
        state.updater = crate::update::Updater::test_ready(prepared("v0.11.1"));
        state.threads[0].status = Status::Running;
        state.restart_for_update = true;
        let mut tx = Tx::default();
        poll(&mut state, 0.0, &mut tx);
        assert!(!state.restart_for_update);
        assert!(state.restart_notice.is_some());
        assert!(crate::update::take_exit().is_none());
        // Finishing the run does not restart on its own.
        state.threads[0].status = Status::Ready;
        poll(&mut state, 0.0, &mut tx);
        assert!(crate::update::take_exit().is_none());
    }

    #[test]
    fn a_restart_guard_runs_after_pi_events_drain() {
        let _exit_record = exit_record_guard();
        let _ = crate::update::take_exit();
        let mut state = ready_state();
        state.load_global_for_test();
        state.updater = crate::update::Updater::test_ready(prepared("v0.12.0"));
        let (_commands, events) = fake_pi(&mut state.threads[0], Status::Ready);
        // Pi has started a run, and the app has not read the event yet.
        events
            .send(Event::Record(json!({"type":"agent_start"})))
            .unwrap();
        state.restart_for_update = true;
        poll(&mut state, 0.0, &mut Tx::default());
        // Drained first, so the guard sees the work the click could not see.
        assert_eq!(state.threads[0].status, Status::Running);
        assert!(
            !state.restart_for_update,
            "the request is never kept for later"
        );
        assert!(state.restart_notice.is_some(), "the row says why it waits");
        assert!(crate::update::take_exit().is_none());
    }

    #[test]
    fn exhausting_the_event_budget_rejects_the_click_even_if_the_thread_still_looks_idle() {
        let _exit_record = exit_record_guard();
        let mut state = ready_state();
        state.load_global_for_test();
        state.updater = crate::update::Updater::test_ready(prepared("v99.0.0"));
        let (pi, _commands, events) = Pi::test_connection_with_capacity(257);
        state.threads[0].pi = Some(pi);
        state.threads[0].status = Status::Ready;
        for _ in 0..256 {
            events
                .send(Event::Diagnostic("queued output".into()))
                .unwrap();
        }
        events
            .send(Event::Record(json!({"type":"agent_start"})))
            .unwrap();
        state.restart_for_update = true;
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(
            state.threads[0].status,
            Status::Ready,
            "agent_start is still unread"
        );
        assert!(!state.restart_for_update);
        assert!(
            state
                .restart_notice
                .as_deref()
                .unwrap()
                .contains("processing events")
        );
        assert!(!crate::update::exit_recorded());
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.threads[0].status, Status::Running);
        assert!(
            !crate::update::exit_recorded(),
            "the click was not retained"
        );
    }

    #[test]
    fn ready_updates_do_not_suppress_normal_checkpoints() {
        let mut state = ready_state();
        state.load_global_for_test();
        state.updater = crate::update::Updater::test_ready(prepared("v99.0.0"));
        let (_commands, events) = fake_pi(&mut state.threads[0], Status::Ready);
        state
            .current_mut()
            .draft
            .replace_text("changed while update was ready");
        events.send(Event::Diagnostic("wake".into())).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        // Only flush what poll queued. Do not checkpoint here, which would hide a regression.
        state.flush_stores().unwrap();
        let saved = crate::store::load(&crate::store::directory().unwrap().join("ui-state.json"))
            .unwrap()
            .unwrap();
        assert_eq!(
            saved["drafts"][state.current().id.to_string()],
            "changed while update was ready"
        );
    }

    #[test]
    fn resuming_after_a_cancelled_update_quit_clears_its_record() {
        let _exit_record = exit_record_guard();
        let mut state = ready_state();
        state.load_global_for_test();
        state.updater = crate::update::Updater::test_ready(prepared("v99.0.0"));
        state.prepared_exit(true, None).unwrap();
        assert!(crate::update::exit_recorded());
        // A running event loop with quitting false is the state after Rune cancels a quit.
        poll(&mut state, 0.0, &mut Tx::default());
        assert!(!state.restart_approved);
        assert!(!crate::update::exit_recorded());
        state.current_mut().draft.replace_text("resumed");
        state.prepared_exit(false, None).unwrap();
        let saved = crate::store::load(&crate::store::directory().unwrap().join("ui-state.json"))
            .unwrap()
            .unwrap();
        assert_eq!(saved["drafts"][state.current().id.to_string()], "resumed");
    }

    #[test]
    fn a_restart_guard_runs_after_task_delivery() {
        let _exit_record = exit_record_guard();
        let _ = crate::update::take_exit();
        let mut state = ready_state();
        state.load_global_for_test();
        state.updater = crate::update::Updater::test_ready(prepared("v0.13.0"));
        // A delegated task finished, and its report is queued for the parent. The parent
        // itself runs, so it already has a live Pi session.
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Completed);
        let mut crew = Thread::new(2);
        crew.parent = Some(state.threads[0].id);
        crew.delegated = true;
        crew.status = Status::Completed;
        crew.task_result = Some("done");
        crew.completion_notified = false;
        state.threads.push(crew);
        state.threads[0].status = Status::Ready;
        state.restart_for_update = true;
        poll(&mut state, 0.0, &mut Tx::default());
        // Delivery queued more work, so the approved request could not have run yet.
        assert!(state.threads[0].queued());
        assert_eq!(
            state.update_restart_blocker(),
            Some("A thread is still working. Restart when it finishes.")
        );
        assert!(crate::update::take_exit().is_none());
    }

    #[test]
    fn a_restart_click_on_an_idle_app_records_the_restart() {
        let _exit_record = exit_record_guard();
        let _ = crate::update::take_exit();
        let mut state = ready_state();
        state.load_global_for_test();
        state.updater = crate::update::Updater::test_ready(prepared("v0.14.0"));
        state.restart_for_update = true;
        poll(&mut state, 0.0, &mut Tx::default());
        assert!(!state.restart_for_update);
        // The poll approved and recorded the restart; Rune quits after the handler.
        let exit = crate::update::take_exit().expect("an idle app restarts");
        assert!(exit.relaunch);
        assert!(
            state.take_restart_request(),
            "the quit hook sees the approval"
        );
        assert!(matches!(
            state.updater.status(),
            crate::update::Status::Ready { .. }
        ));
    }

    #[test]
    fn saved_state_round_trips_drafts_images_tabs_projects_captains_and_sessions() {
        let mut state = State::new(PathBuf::new(), false);
        // Rosters start empty; these tests exercise the earlier named presets.
        state.captains = crate::captains::Captain::presets();
        let base = crate::store::directory().unwrap().join("round-trip");
        std::fs::create_dir_all(&base).unwrap();
        // Two projects, three threads, two drafts, one image reference and two tabs.
        state.projects.entries = vec![
            Project::new(1, base.join("one")),
            Project::new(2, base.join("two")),
        ];
        state.projects.next_id = 3;
        let mut first = Thread::new(1);
        first.title = "First".into();
        first.project = Some(1);
        first.captain = Some(1);
        first.session = Some("one.jsonl".into());
        first.draft.replace_text("unsent draft");
        // Draft images are references to the deduplicated attachment store.
        let attachments = crate::attachments::directory().unwrap();
        std::fs::create_dir_all(&attachments).unwrap();
        std::fs::write(attachments.join("image-1.png"), b"png").unwrap();
        first.draft_images.push(Attachment {
            id: "image-1.png".into(),
            name: "shot.png".into(),
            mime: "image/png".into(),
            width: 4,
            height: 4,
            bytes: 3,
            path: None,
        });
        let mut second = Thread::new(2);
        second.title = "Second".into();
        second.project = Some(2);
        second.session = Some("two.jsonl".into());
        second.draft.replace_text("second draft");
        let mut third = Thread::new(3);
        third.title = "Third".into();
        third.delegated = true;
        third.project = Some(1);
        third.parent = Some(1);
        third.session = Some("three.jsonl".into());
        state.threads = vec![first, second, third];
        state.tabs = vec![1, 2];
        state.active = 1;
        state.captains[0].name = "Renamed captain".into();
        for (project, name) in [
            ("one", "one.jsonl"),
            ("two", "two.jsonl"),
            ("one", "three.jsonl"),
        ] {
            std::fs::create_dir_all(base.join(project)).unwrap();
            std::fs::write(
                base.join(project).join(name),
                format!("saved session {name}\n"),
            )
            .unwrap();
        }
        state.load_global_for_test();
        state.checkpoint_and_flush().unwrap();
        drop(state);
        let mut restored = State::new(PathBuf::new(), false);
        restored.load_global_for_test();
        assert!(
            restored.storage_error.is_none(),
            "{:?}",
            restored.storage_error
        );
        let draft = |state: &State, id: usize| {
            let thread = state.threads.iter().find(|t| t.id == id).unwrap();
            (
                thread.draft.text.clone(),
                thread.draft_images.len(),
                thread.session.clone(),
                thread.project,
                thread.title.clone(),
            )
        };
        assert_eq!(
            draft(&restored, 1),
            (
                "unsent draft".to_owned(),
                1,
                Some("one.jsonl".into()),
                Some(1),
                "First".into()
            )
        );
        assert_eq!(
            draft(&restored, 2),
            (
                "second draft".to_owned(),
                0,
                Some("two.jsonl".into()),
                Some(2),
                "Second".into()
            )
        );
        assert_eq!(draft(&restored, 3).2, Some("three.jsonl".into()));
        assert_eq!(restored.tabs, vec![1, 2]);
        assert_eq!(restored.current().id, 2);
        assert_eq!(restored.projects.entries.len(), 2);
        assert_eq!(restored.captains[0].name, "Renamed captain");
        assert_eq!(
            std::fs::read(attachments.join("image-1.png")).unwrap(),
            b"png"
        );
        for (project, name) in [
            ("one", "one.jsonl"),
            ("two", "two.jsonl"),
            ("one", "three.jsonl"),
        ] {
            assert_eq!(
                std::fs::read_to_string(base.join(project).join(name)).unwrap(),
                format!("saved session {name}\n")
            );
        }
        assert!(
            restored
                .threads
                .iter()
                .any(|t| t.delegated && t.parent == Some(1))
        );
    }

    fn fake_pi(
        thread: &mut Thread,
        status: Status,
    ) -> (mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
        let (pi, commands, events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.status = status;
        thread.model_provider = "cpa".into();
        (commands, events)
    }

    fn model_with_levels(id: &str, levels: &[&str]) -> AgentModel {
        let mut map = serde_json::Map::new();
        for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
            map.insert(
                level.into(),
                if levels.contains(&level) {
                    json!(level)
                } else {
                    Value::Null
                },
            );
        }
        let mut value = json!({
            "provider": "cpa",
            "id": id,
            "name": format!("{id} model"),
            "reasoning": true,
        });
        value["thinkingLevelMap"] = Value::Object(map);
        AgentModel::from_value(&value).unwrap()
    }

    fn preparing_pi(thread: &mut Thread) -> (mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
        let result = fake_pi(thread, Status::Ready);
        thread.initializing = true;
        thread.state_loaded = false;
        thread.history_loaded = false;
        result
    }

    fn child(state: &mut State) -> (usize, mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
        let index = state.threads.len();
        let mut thread = Thread::new(state.next_id);
        state.next_id += 1;
        thread.title = format!("Child {}", thread.id);
        thread.delegated = true;
        thread.parent = Some(state.threads[0].id);
        thread.project = state.threads[0].project;
        thread.machine = state.threads[0].machine.clone();
        thread.arm_task();
        let (commands, events) = fake_pi(&mut thread, Status::Running);
        state.threads.push(thread);
        (index, commands, events)
    }

    fn acknowledge(thread: &mut Thread, prompt: &Value) {
        thread.apply(json!({"type":"response","id":prompt["id"],"command":"prompt","success":true,"data":{"disposition":"queued"}}));
    }

    #[test]
    fn simultaneous_completions_are_delivered_exactly_once_to_idle_or_busy_parents() {
        for status in [
            Status::Ready,
            Status::Completed,
            Status::Running,
            Status::Waiting,
        ] {
            let mut state = State::new(".".into(), false);
            let (commands, _parent_events) = fake_pi(&mut state.threads[0], status);
            state.threads[0].draft.replace_text("Unsent user draft");
            let (first, _first_commands, first_events) = child(&mut state);
            let (second, _second_commands, second_events) = child(&mut state);
            for events in [&first_events, &second_events] {
                events.send(Event::Record(json!({"type":"message_end","message":{"role":"assistant","content":"PONG"}}))).unwrap();
                events
                    .send(Event::Record(
                        json!({"type":"agent_settled","aborted":false}),
                    ))
                    .unwrap();
            }
            poll(&mut state, 0.0, &mut Tx::default());
            assert_eq!(state.threads[0].pending_injections.len(), 2);
            let first_prompt = commands.try_recv().unwrap();
            assert_eq!(first_prompt["type"], "prompt");
            assert_eq!(first_prompt["streamingBehavior"], "followUp");
            assert!(
                first_prompt["message"]
                    .as_str()
                    .unwrap()
                    .contains("task #2 finished")
            );
            assert!(first_prompt["message"].as_str().unwrap().contains("PONG"));
            assert!(
                commands.try_recv().is_err(),
                "await acceptance before the next injection"
            );
            acknowledge(&mut state.threads[0], &first_prompt);
            poll(&mut state, 0.0, &mut Tx::default());
            let second_prompt = commands.try_iter().find(|c| c["type"] == "prompt").unwrap();
            assert!(
                second_prompt["message"]
                    .as_str()
                    .unwrap()
                    .contains("task #3 finished")
            );
            acknowledge(&mut state.threads[0], &second_prompt);
            for index in [first, second] {
                state.threads[index].apply(json!({"type":"agent_settled","aborted":false}));
            }
            poll(&mut state, 0.0, &mut Tx::default());
            assert!(state.threads[0].pending_injections.is_empty());
            assert!(commands.try_iter().all(|c| c["type"] != "prompt"));
            assert_eq!(state.threads[0].draft.text, "Unsent user draft");
            assert_eq!(
                state.threads[0]
                    .messages
                    .iter()
                    .filter(|m| m.role == "Shina")
                    .count(),
                2
            );
        }
    }

    #[test]
    fn completion_waits_for_parent_handshake_without_sending_its_draft() {
        let mut state = State::new(".".into(), false);
        let (commands, events) = preparing_pi(&mut state.threads[0]);
        state.threads[0].draft.replace_text("Keep typing");
        let (index, _commands, _events) = child(&mut state);
        state.threads[index].apply(json!({"type":"agent_settled","aborted":false}));
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.threads[0].pending_injections.len(), 1);
        assert!(commands.try_recv().is_err());
        events.send(Event::Record(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}))).unwrap();
        events.send(Event::Record(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false,"model":{"provider":"cpa","id":"test"}}}))).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        let prompt = commands
            .try_iter()
            .find(|command| command["type"] == "prompt")
            .expect("completion prompt after the handshake");
        assert!(
            prompt["message"]
                .as_str()
                .unwrap()
                .contains("task #2 finished")
        );
        assert_eq!(state.threads[0].draft.text, "Keep typing");
    }

    #[test]
    fn failures_and_error_exits_wake_once_with_truncated_results() {
        for exit in [false, true] {
            let mut state = State::new(".".into(), false);
            let (commands, _events) = fake_pi(&mut state.threads[0], Status::Ready);
            let (index, _commands, events) = child(&mut state);
            state.threads[index].messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "Pi",
                text: "界".repeat(6100),
                images: Vec::new(),
                context: None,
            });
            if exit {
                events
                    .send(Event::Error("Pi exited with code 1".into()))
                    .unwrap();
                events.send(Event::Exited).unwrap();
            } else {
                events.send(Event::Record(json!({"type":"response","command":"prompt","success":false,"error":"Rejected"}))).unwrap();
            }
            poll(&mut state, 0.0, &mut Tx::default());
            let prompt = commands.try_recv().unwrap();
            let text = prompt["message"].as_str().unwrap();
            assert!(text.contains("Status: failed"));
            assert!(text.contains(if exit {
                "Pi exited with code 1"
            } else {
                "Rejected"
            }));
            assert!(text.contains(&format!("{}...", "界".repeat(6000))));
            acknowledge(&mut state.threads[0], &prompt);
            poll(&mut state, 0.0, &mut Tx::default());
            assert!(commands.try_iter().all(|c| c["type"] != "prompt"));
        }
    }

    #[test]
    fn task_waits_park_finish_and_timeout_without_blocking() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, _commands, events) = child(&mut state);
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":2,"wait_seconds":600}),
            reply,
        });
        assert_eq!(state.task_waiters.len(), 1);
        assert!(answer.try_recv().is_err());
        state.poll_tasks(Instant::now());
        assert!(answer.try_recv().is_err());
        events
            .send(Event::Record(
                json!({"type":"agent_settled","aborted":false}),
            ))
            .unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        let result = answer.try_recv().unwrap().unwrap();
        assert_eq!(result["finished"], true);
        assert_eq!(result["parent"], 1);
        assert!(state.task_waiters.is_empty());

        state.threads[index].arm_task();
        state.threads[index].status = Status::Running;
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":2,"wait_seconds":10}),
            reply,
        });
        let deadline = state.task_waiters[0].deadline;
        state.poll_tasks(deadline);
        assert_eq!(answer.try_recv().unwrap().unwrap()["finished"], false);
        assert!(state.task_waiters.is_empty());
        for value in [json!(-1), json!(601), json!(1.5), json!("1"), Value::Null] {
            assert!(
                state
                    .mcp_call(
                        1,
                        "delegate_task",
                        &json!({"prompt":"No task","wait_seconds":value})
                    )
                    .is_err()
            );
        }
        assert_eq!(
            state.threads.len(),
            2,
            "invalid waits must not start children"
        );
    }

    #[test]
    fn delegate_wait_parks_and_queues_the_result_for_a_disconnected_parent() {
        let mut state = State::new(".".into(), false);
        // Prevent a real subprocess in this unit test; connect_thread is tested live below.
        state.demo = true;
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "delegate_task".into(),
            args: json!({"prompt":"Reply PONG","title":"Ping","wait_seconds":600}),
            reply,
        });
        assert_eq!(state.threads[1].parent, Some(1));
        assert!(state.threads[1].draft.text.is_empty());
        assert_eq!(state.threads[1].pending_injections.len(), 1);
        assert!(answer.try_recv().is_err());
        assert_eq!(state.task_waiters.len(), 1);
        state.threads[1].apply(json!({"type":"agent_settled","aborted":false}));
        state.poll_tasks(Instant::now());
        assert_eq!(answer.try_recv().unwrap().unwrap()["finished"], true);
        assert_eq!(state.threads[0].pending_injections.len(), 1);
        let (commands, events) = preparing_pi(&mut state.threads[0]);
        events.send(Event::Record(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false,"model":{"provider":"cpa","id":"test"}}}))).unwrap();
        events.send(Event::Record(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}))).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        let prompt = commands
            .try_iter()
            .find(|command| command["type"] == "prompt")
            .expect("queued result prompt");
        assert!(prompt["message"].as_str().unwrap().contains("Ping"));
    }

    #[test]
    fn explicit_delegation_selection_applies_before_the_first_prompt_and_is_reported() {
        let mut state = State::new(".".into(), false);
        state.demo = true;
        let fast = model_with_levels("fast", &["off", "low"]);
        let deep = model_with_levels(
            "deep",
            &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        );
        state.catalog.models = vec![fast.clone(), deep.clone()];
        state.catalog.default_model = Some(fast);
        state.catalog.default_thinking = "low".into();
        let result = state
            .mcp_call(
                1,
                "delegate_task",
                &json!({
                    "prompt":"Inspect the test fixture",
                    "target":{"provider":"cpa","model":"deep","options":{"reasoning":"max"}}
                }),
            )
            .unwrap();
        assert_eq!(result["selection"]["model"], "deep");
        assert_eq!(result["selection"]["options"]["reasoning"], "max");
        let index = state.threads.len() - 1;
        let child_id = state.threads[index].id;
        {
            let child = &state.threads[index];
            assert_eq!(child.selected_model, Some(("cpa".into(), "deep".into())));
            assert_eq!(child.selected_thinking.as_deref(), Some("max"));
            assert_eq!(child.model_id, "deep");
            assert_eq!(child.thinking, "max");
            assert_eq!(
                child.thinking_levels.last().map(String::as_str),
                Some("max")
            );
            assert_eq!(child.models.len(), 2);
        }
        let status = state
            .mcp_call(1, "task_status", &json!({"thread_id":child_id}))
            .unwrap();
        assert_eq!(status["selection"]["model"], "deep");
        assert_eq!(status["selection"]["options"]["reasoning"], "max");
        let read = state
            .mcp_call(1, "read_thread", &json!({"threadId":child_id}))
            .unwrap();
        assert_eq!(read["thread"]["selection"]["model"], "deep");
        let list = state
            .mcp_call(1, "list_threads", &json!({"allProjects":true}))
            .unwrap();
        let listed = list["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|thread| thread["threadId"] == json!(child_id))
            .unwrap();
        assert_eq!(listed["selection"]["model"], "deep");
        assert_eq!(listed["selection"]["options"]["reasoning"], "max");

        // Mock a fresh Pi handshake. The child must acknowledge model and reasoning before
        // its injected brief becomes a prompt.
        let (commands, events) = preparing_pi(&mut state.threads[index]);
        for record in [
            json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
                "isStreaming":false,"model":{"provider":"cpa","id":"fast"},"thinkingLevel":"low"
            }}),
            json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}),
        ] {
            events.send(Event::Record(record)).unwrap();
        }
        poll(&mut state, 0.0, &mut Tx::default());
        let model = commands.try_recv().unwrap();
        assert_eq!(model["type"], "set_model");
        assert_eq!(model["modelId"], "deep");
        assert!(
            commands.try_recv().is_err(),
            "the prompt must wait for the model acknowledgement"
        );
        state.threads[index].apply(json!({"type":"response","command":"set_model","success":true,"data":{"provider":"cpa","id":"deep"}}));
        let thinking = loop {
            let command = commands.try_recv().expect("reasoning command");
            assert_ne!(
                command["type"], "prompt",
                "the prompt must wait for the reasoning choice"
            );
            if command["type"] == "set_thinking_level" {
                break command;
            }
        };
        assert_eq!(thinking["level"], "max");
        assert!(
            commands.try_recv().is_err(),
            "the prompt must wait for the reasoning acknowledgement"
        );
        state.threads[index]
            .apply(json!({"type":"response","command":"set_thinking_level","success":true}));
        let verification_commands: Vec<_> = commands.try_iter().collect();
        assert!(
            verification_commands
                .iter()
                .all(|command| command["type"] != "prompt"),
            "the prompt must wait for startup to finish"
        );
        let verification = verification_commands
            .iter()
            .rev()
            .find(|command| command["type"] == "get_state")
            .expect("Pi must confirm its actual selection");
        state.poll_tasks(Instant::now());
        assert!(commands.try_recv().is_err());
        state.threads[index].apply(json!({
            "type":"response","id":verification["id"],"command":"get_state","success":true,
            "data":{"model":{"provider":"cpa","id":"deep"},"thinkingLevel":"max"}
        }));
        state.poll_tasks(Instant::now());
        let prompt = commands.try_recv().unwrap();
        assert_eq!(prompt["type"], "prompt");
        assert!(
            prompt["message"]
                .as_str()
                .unwrap()
                .contains("Inspect the test fixture")
        );
        assert_eq!(state.threads[index].model_id, "deep");
        assert_eq!(state.threads[index].thinking, "max");
    }

    #[test]
    fn delegated_briefs_are_not_sent_when_pi_changes_the_requested_selection() {
        for data in [
            json!({"model":{"provider":"cpa","id":"deep"},"thinkingLevel":"high"}),
            json!({"model":{"provider":"cpa","id":"other"},"thinkingLevel":"max"}),
            json!({"model":{"provider":"direct","id":"deep"},"thinkingLevel":"max"}),
            json!({"model":{"provider":"cpa","id":"deep"}}),
        ] {
            let mut thread = Thread::new(2);
            thread.delegated = true;
            thread.selected_model = Some(("cpa".into(), "deep".into()));
            thread.selected_thinking = Some("max".into());
            thread.arm_task();
            thread.enqueue_injection("Do not send under another selection".into());
            let (commands, _events) = preparing_pi(&mut thread);
            thread.state_loaded = true;
            thread.history_loaded = true;
            thread.startup_model_applied = true;
            thread.startup_thinking_applied = true;
            thread.drive_session();
            let verification = commands.try_recv().unwrap();
            assert_eq!(verification["type"], "get_state");
            assert!(!thread.flush_injection());
            thread.apply(json!({
                "type":"response","id":verification["id"],
                "command":"get_state","success":true,"data":data
            }));
            assert_eq!(thread.status, Status::Failed);
            assert_eq!(thread.task_result, Some("failed"));
            assert!(thread.pi.is_none());
            assert!(
                thread
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("Shina did not send the task")
            );
            assert!(
                commands
                    .try_iter()
                    .all(|command| command["type"] != "prompt")
            );
        }
    }

    #[test]
    fn invalid_delegation_targets_are_rejected_without_creating_children() {
        let mut state = State::new(".".into(), false);
        state.demo = true;
        let fast = model_with_levels("fast", &["off", "low"]);
        let deep = model_with_levels(
            "deep",
            &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        );
        state.catalog.models = vec![fast, deep];
        state.catalog.default_model = Some(state.catalog.models[0].clone());
        let before = state.threads.len();
        let cases = [
            ("null target", Value::Null),
            ("wrong target type", json!("deep")),
            ("missing model", json!({"provider":"cpa"})),
            ("empty model", json!({"provider":"cpa","model":"  "})),
            ("wrong model type", json!({"provider":"cpa","model":7})),
            ("wrong provider type", json!({"provider":7,"model":"fast"})),
            (
                "direct provider",
                json!({"provider":"google","model":"fast"}),
            ),
            (
                "unknown target setting",
                json!({"provider":"cpa","model":"fast","role":"review"}),
            ),
            (
                "wrong options type",
                json!({"provider":"cpa","model":"fast","options":[]}),
            ),
            (
                "unknown option",
                json!({"provider":"cpa","model":"fast","options":{"temperature":"hot"}}),
            ),
            (
                "wrong reasoning type",
                json!({"provider":"cpa","model":"fast","options":{"reasoning":5}}),
            ),
            (
                "empty reasoning",
                json!({"provider":"cpa","model":"fast","options":{"reasoning":" "}}),
            ),
            (
                "conflicting aliases",
                json!({"provider":"cpa","model":"fast","options":{"reasoning":"low","thinking":"high"}}),
            ),
            ("unknown model", json!({"provider":"cpa","model":"missing"})),
        ];
        for (label, target) in cases {
            let error = state
                .mcp_call(
                    1,
                    "delegate_task",
                    &json!({"prompt":"No child","target":target}),
                )
                .unwrap_err();
            assert!(!error.is_empty(), "{label}");
            assert_eq!(
                state.threads.len(),
                before,
                "{label} must not create a child: {error}"
            );
        }
        let error = state
            .mcp_call(
                1,
                "delegate_task",
                &json!({
                    "prompt":"No child",
                    "target":{"provider":"cpa","model":"fast","options":{"reasoning":"max"}}
                }),
            )
            .unwrap_err();
        assert!(error.contains("Supported levels: off, low"), "{error}");
        assert!(
            error.contains("will not substitute or clamp"),
            "an explicit level must never be clamped: {error}"
        );
        assert_eq!(state.threads.len(), before);
        state.threads[0].machine = "remote".into();
        let error = state
            .mcp_call(1, "delegate_task", &json!({"prompt":"No child"}))
            .unwrap_err();
        assert!(
            error.contains("only on the local machine"),
            "unsupported hosts must fail before a child starts: {error}"
        );
        assert_eq!(state.threads.len(), before);
    }

    #[test]
    fn delegation_inherits_pinned_selection_and_overrides_only_given_fields() {
        let mut state = State::new(".".into(), false);
        state.demo = true;
        state.threads[0].model_provider = "cpa".into();
        state.threads[0].model_id = "fast".into();
        state.threads[0].thinking = "low".into();
        state.threads[0].selected_model = Some(("cpa".into(), "deep".into()));
        state.threads[0].selected_thinking = Some("max".into());
        let before = state.threads.len();
        let error = state
            .mcp_call(
                1,
                "delegate_task",
                &json!({"prompt":"No child","target":{"model":"deep"}}),
            )
            .unwrap_err();
        assert!(error.contains("discovery is not ready"), "{error}");
        assert_eq!(state.threads.len(), before);

        let result = state
            .mcp_call(1, "delegate_task", &json!({"prompt":"Inherit my choice"}))
            .unwrap();
        let child = state.threads.last().unwrap();
        assert_eq!(child.selected_model, Some(("cpa".into(), "deep".into())));
        assert_eq!(child.selected_thinking.as_deref(), Some("max"));
        assert_eq!(result["selection"]["model"], "deep");
        assert_eq!(result["selection"]["options"]["reasoning"], "max");

        let fast = model_with_levels("fast", &["off", "low"]);
        let deep = model_with_levels(
            "deep",
            &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        );
        state.catalog.models = vec![fast, deep];
        state.catalog.default_model = Some(state.catalog.models[0].clone());
        state.catalog.default_thinking = "low".into();
        let error = state
            .mcp_call(
                1,
                "delegate_task",
                &json!({"prompt":"No child","target":{"model":"fast"}}),
            )
            .unwrap_err();
        assert!(
            error.contains("Reasoning level max is not supported by CPA model fast"),
            "{error}"
        );
        assert_eq!(state.threads.len(), before + 1);

        let result = state
            .mcp_call(
                1,
                "delegate_task",
                &json!({
                    "prompt":"Use the alias",
                    "target":{"provider":"cpa","model":"fast","options":{"thinking":"low"}}
                }),
            )
            .unwrap();
        assert_eq!(result["selection"]["model"], "fast");
        assert_eq!(result["selection"]["options"]["reasoning"], "low");
        let child = state.threads.last().unwrap();
        assert_eq!(child.selected_model, Some(("cpa".into(), "fast".into())));
        assert_eq!(child.selected_thinking.as_deref(), Some("low"));
    }

    #[test]
    fn delegation_uses_the_catalog_default_when_the_caller_has_no_selection() {
        let mut state = State::new(".".into(), false);
        state.demo = true;
        let fast = model_with_levels("fast", &["off", "low"]);
        let deep = model_with_levels(
            "deep",
            &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        );
        state.catalog.models = vec![fast.clone(), deep];
        state.catalog.default_model = Some(fast);
        state.catalog.default_thinking = "low".into();
        let result = state
            .mcp_call(1, "delegate_task", &json!({"prompt":"Use the default"}))
            .unwrap();
        assert_eq!(result["selection"]["model"], "fast");
        assert_eq!(result["selection"]["options"]["reasoning"], "low");
        let child = state.threads.last().unwrap();
        assert_eq!(child.selected_model, Some(("cpa".into(), "fast".into())));
        assert_eq!(child.selected_thinking.as_deref(), Some("low"));
    }

    #[test]
    fn orchestrator_capabilities_report_catalog_defaults_caller_choice_and_limits() {
        let mut state = State::new(".".into(), false);
        let fast = model_with_levels("fast", &["off", "low"]);
        let deep = model_with_levels(
            "deep",
            &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        );
        state.catalog.models = vec![fast.clone(), deep.clone()];
        state.catalog.default_model = Some(fast);
        state.catalog.default_thinking = "low".into();
        state.threads[0].selected_model = Some(("cpa".into(), "deep".into()));
        state.threads[0].selected_thinking = Some("max".into());
        let capabilities = state
            .mcp_call(1, "orchestrator_capabilities", &json!({}))
            .unwrap();
        assert_eq!(capabilities["ready"], true);
        assert_eq!(capabilities["discovery"]["state"], "ready");
        assert_eq!(capabilities["discovery"]["source"], "catalog");
        assert_eq!(capabilities["discovery"]["error"], Value::Null);
        let models = capabilities["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        let deep_entry = models.iter().find(|model| model["id"] == "deep").unwrap();
        assert_eq!(
            deep_entry["reasoning_levels"],
            json!(["off", "minimal", "low", "medium", "high", "xhigh", "max"])
        );
        assert_eq!(capabilities["default_selection"]["model"], "fast");
        assert_eq!(
            capabilities["default_selection"]["options"]["reasoning"],
            "low"
        );
        assert_eq!(capabilities["caller_selection"]["model"], "deep");
        assert_eq!(
            capabilities["caller_selection"]["options"]["reasoning"],
            "max"
        );
        assert_eq!(capabilities["inherit_selection"]["model"], "deep");
        assert_eq!(capabilities["runtime"]["harness"], "pi");
        assert_eq!(
            capabilities["runtime"]["providers"],
            json!(["cpa", "codex", "claude"])
        );
        assert_eq!(capabilities["runtime"]["hosts"], json!(["local"]));
        assert_eq!(capabilities["runtime"]["machine"], "local");
        assert_eq!(capabilities["runtime"]["host_supported"], true);
        assert_eq!(capabilities["constraints"]["direct_providers"], false);
        assert_eq!(capabilities["constraints"]["remote_hosts"], true);
    }

    #[test]
    fn capabilities_report_discovery_not_ready_without_inventing_models() {
        let mut state = State::new(".".into(), false);
        state.catalog.error = Some("Model discovery timed out.".into());
        let capabilities = state
            .mcp_call(1, "orchestrator_capabilities", &json!({}))
            .unwrap();
        assert_eq!(capabilities["ready"], false);
        assert_eq!(capabilities["discovery"]["state"], "failed");
        assert_eq!(capabilities["discovery"]["source"], "none");
        assert_eq!(
            capabilities["discovery"]["error"],
            "Model discovery timed out."
        );
        assert!(capabilities["models"].as_array().unwrap().is_empty());
        assert!(capabilities["default_selection"].is_null());
        assert!(capabilities["caller_selection"].is_null());
        assert!(capabilities["inherit_selection"].is_null());
        let before = state.threads.len();
        let error = state
            .mcp_call(
                1,
                "delegate_task",
                &json!({"prompt":"No child","target":{"model":"deep"}}),
            )
            .unwrap_err();
        assert!(error.contains("discovery is not ready"), "{error}");
        assert_eq!(state.threads.len(), before);
    }

    #[test]
    fn capabilities_use_caller_metadata_when_catalog_discovery_is_unavailable() {
        let mut state = State::new(".".into(), false);
        let deep = model_with_levels(
            "deep",
            &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
        );
        state.catalog.error = Some("Model discovery timed out.".into());
        state.threads[0].models = vec![deep];
        state.threads[0].selected_model = Some(("cpa".into(), "deep".into()));
        state.threads[0].selected_thinking = Some("max".into());
        let capabilities = state
            .mcp_call(1, "orchestrator_capabilities", &json!({}))
            .unwrap();
        assert_eq!(capabilities["ready"], true);
        assert_eq!(capabilities["discovery"]["state"], "caller_metadata");
        assert_eq!(capabilities["discovery"]["source"], "caller");
        assert_eq!(
            capabilities["discovery"]["error"],
            "Model discovery timed out."
        );
        assert_eq!(capabilities["models"].as_array().unwrap().len(), 1);
        assert_eq!(capabilities["models"][0]["id"], "deep");
        assert_eq!(capabilities["inherit_selection"]["model"], "deep");
        assert_eq!(
            capabilities["inherit_selection"]["options"]["reasoning"],
            "max"
        );
        state.demo = true;
        let before = state.threads.len();
        let result = state
            .mcp_call(
                1,
                "delegate_task",
                &json!({"prompt":"Use caller metadata","target":{"model":"deep"}}),
            )
            .unwrap();
        assert_eq!(state.threads.len(), before + 1);
        assert_eq!(result["selection"]["model"], "deep");
        assert_eq!(result["selection"]["options"]["reasoning"], "max");
    }

    #[test]
    fn only_a_childs_parent_can_cancel_or_send_and_cancellation_finishes_waits() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, commands, events) = child(&mut state);
        let mut stranger = Thread::new(3);
        stranger.status = Status::Ready;
        state.threads.push(stranger);
        for caller in [2, 3] {
            for tool in ["task_cancel", "task_send"] {
                let error = state
                    .mcp_call(caller, tool, &json!({"thread_id":2,"message":"Correction"}))
                    .unwrap_err();
                assert!(error.contains("Only the parent"));
            }
        }
        assert!(
            state
                .mcp_call(1, "task_cancel", &json!({"thread_id":3}))
                .is_err()
        );
        assert!(commands.try_recv().is_err());
        state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"Correction"}),
            )
            .unwrap();
        state.poll_tasks(Instant::now());
        let prompt = commands.try_recv().unwrap();
        assert_eq!(prompt["streamingBehavior"], "followUp");
        assert!(prompt["message"].as_str().unwrap().contains("Correction"));
        acknowledge(&mut state.threads[index], &prompt);
        assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":2,"wait_seconds":600}),
            reply,
        });
        state
            .mcp_call(1, "task_cancel", &json!({"thread_id":2}))
            .unwrap();
        assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
        assert_eq!(commands.try_recv().unwrap()["type"], "abort");
        events
            .send(Event::Record(
                json!({"type":"agent_settled","aborted":true}),
            ))
            .unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        let result = answer.try_recv().unwrap().unwrap();
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result["finished"], true);
        assert_eq!(state.threads[index].status, Status::Ready);
        assert!(
            state
                .mcp_call(2, "delegate_task", &json!({"prompt":"Nested"}))
                .unwrap_err()
                .contains("cannot delegate")
        );
        state
            .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"Retry"}))
            .unwrap();
        state.poll_tasks(Instant::now());
        assert_eq!(state.threads[index].status, Status::Running);
        assert_eq!(
            state
                .mcp_call(1, "task_status", &json!({"thread_id":2}))
                .unwrap()["finished"],
            false
        );
        assert!(commands.try_iter().any(|c| c["type"] == "prompt"));
    }

    #[test]
    fn restored_injections_are_shina_messages_and_rejections_never_change_drafts() {
        let mut thread = Thread::new(1);
        let (commands, _events) = fake_pi(&mut thread, Status::Ready);
        thread.draft.replace_text("User draft");
        thread.enqueue_injection("Shina: task #2 finished".into());
        thread.flush_injection();
        let prompt = commands.try_recv().unwrap();
        thread.apply(json!({"type":"response","id":prompt["id"],"command":"prompt","success":false,"error":"Rejected"}));
        assert_eq!(thread.draft.text, "User draft");
        assert_eq!(thread.pending_injections.len(), 1);
        thread.status = Status::Ready;
        thread.flush_injection();
        assert_eq!(
            thread.messages.len(),
            1,
            "a retry must not duplicate its transcript entry"
        );
        let prompt = commands.try_recv().unwrap();
        acknowledge(&mut thread, &prompt);
        let history = json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":prompt["message"]},
            {"role":"assistant","content":"Acknowledged"}
        ]}});
        thread.apply(history);
        assert_eq!(thread.messages[0].role, "Shina");
        assert_eq!(thread.messages[1].role, "Pi");
        assert_eq!(thread.draft.text, "User draft");
    }

    #[test]
    fn reconnect_reconciles_an_unacknowledged_injection_against_saved_history() {
        for accepted in [true, false] {
            let mut thread = Thread::new(1);
            let (commands, _events) = fake_pi(&mut thread, Status::Ready);
            thread.draft.replace_text("Keep this draft");
            thread.enqueue_injection("Shina: task #2 finished".into());
            thread.flush_injection();
            let prompt = commands.try_recv().unwrap();
            thread.disconnect();
            let (commands, _events) = preparing_pi(&mut thread);
            let messages = if accepted {
                json!([{"role":"user","content":prompt["message"]}])
            } else {
                json!([])
            };
            thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":messages}}));
            thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false,"model":{"provider":"cpa","id":"test"}}}));
            assert_eq!(commands.try_recv().unwrap()["type"], "get_session_stats");
            assert_eq!(thread.flush_injection(), !accepted);
            assert_eq!(commands.try_recv().is_ok(), !accepted);
            assert_eq!(thread.messages.len(), 1);
            assert_eq!(thread.messages[0].display_text(), "Shina: task #2 finished");
            assert_eq!(thread.draft.text, "Keep this draft");
        }
    }

    #[test]
    fn a_wait_deadline_wakes_an_otherwise_idle_ui() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (_index, _commands, _events) = child(&mut state);
        let (wake, woken) = mpsc::channel();
        state.wake = Arc::new(move || {
            let _ = wake.send(());
        });
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":2,"wait_seconds":1}),
            reply,
        });
        assert!(answer.try_recv().is_err());
        woken
            .recv_timeout(Duration::from_secs(3))
            .expect("the deadline must wake the UI without Pi events");
        state.poll_tasks(Instant::now());
        assert_eq!(answer.try_recv().unwrap().unwrap()["finished"], false);
        assert!(state.task_waiters.is_empty());
    }

    #[test]
    fn cancelling_a_starting_child_discards_its_pending_brief() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, commands, _events) = child(&mut state);
        state.threads[index].initializing = true;
        state.threads[index].enqueue_injection("Shina: task brief".into());
        let result = state
            .mcp_call(1, "task_cancel", &json!({"thread_id":2}))
            .unwrap();
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result["finished"], true);
        assert!(state.threads[index].pi.is_none());
        assert!(state.threads[index].pending_injections.is_empty());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn sending_to_a_stopping_child_waits_for_abort_then_starts_a_fresh_run() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, commands, _events) = child(&mut state);
        state
            .mcp_call(1, "task_cancel", &json!({"thread_id":2}))
            .unwrap();
        assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
        assert_eq!(commands.try_recv().unwrap()["type"], "abort");
        state
            .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"New work"}))
            .unwrap();
        state.poll_tasks(Instant::now());
        assert!(
            commands.try_recv().is_err(),
            "new work must not enter the queue being cleared"
        );
        state.threads[index].apply(json!({"type":"agent_settled","aborted":true}));
        state.poll_tasks(Instant::now());
        let prompt = commands
            .try_iter()
            .find(|command| command["type"] == "prompt")
            .expect("new work prompt");
        assert!(prompt["message"].as_str().unwrap().contains("New work"));
        assert!(state.threads[index].task_result.is_none());
        assert!(!state.threads[index].completion_notified);
        assert!(
            state.threads[0]
                .messages
                .iter()
                .any(|m| m.text.contains("Status: cancelled"))
        );
    }

    #[test]
    fn task_send_can_restart_a_connected_failed_child() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, commands, _events) = child(&mut state);
        state.threads[index].error = Some("Provider failure".into());
        state.threads[index].apply(json!({"type":"agent_settled","aborted":false}));
        state.poll_tasks(Instant::now());
        let result = state
            .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"Retry"}))
            .unwrap();
        assert_eq!(result["status"], "queued");
        assert_eq!(result["finished"], false);
        state.poll_tasks(Instant::now());
        assert_eq!(state.threads[index].status, Status::Running);
        assert!(state.threads[index].error.is_none());
        assert!(commands.try_iter().any(|c| c["type"] == "prompt"));
    }
    #[test]
    fn low_level_agent_end_is_not_settlement() {
        let mut thread = Thread::new(1);
        thread.status = Status::Running;
        thread.apply(json!({"type":"agent_end"}));
        assert_eq!(thread.status, Status::Running);
        thread.apply(json!({"type":"agent_settled","aborted":false}));
        assert_eq!(thread.status, Status::Completed);
    }
    #[test]
    fn final_message_replaces_deltas_without_duplicating_text() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"message_start","message":{"role":"assistant"}}));
        thread.apply(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Part"}}));
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Complete"}]}}));
        assert_eq!(thread.messages.len(), 1);
        assert_eq!(thread.messages[0].text, "Complete");
    }
    #[test]
    fn switching_and_archiving_preserve_other_drafts() {
        let mut state = State::new(PathBuf::from("."), false);
        state.current_mut().draft.replace_text("Keep this");
        state.new_thread();
        let blank = state.current().id;
        state.archive();
        assert_eq!(state.current().draft.text, "Keep this");
        assert!(
            !state.threads.iter().any(|t| t.id == blank),
            "an archived blank draft leaves no record"
        );
    }
    #[test]
    fn captain_portrait_follows_the_most_urgent_thread() {
        let mut state = State::new(PathBuf::from("."), false);
        state.captains = Captain::presets();
        state.open_captain(0);
        let home = state.current().id;
        state.current_mut().status = Status::Completed;
        state.new_thread();
        state.current_mut().captain = Some(0);
        state.current_mut().delegated = true;
        state.current_mut().parent = Some(home);
        state.current_mut().status = Status::Running;
        state.new_thread();
        state.current_mut().captain = Some(0);
        state.current_mut().delegated = true;
        state.current_mut().parent = Some(home);
        state.current_mut().status = Status::Waiting;
        assert_eq!(state.captain_status(0), Status::Waiting);
        state.current_mut().status = Status::Failed;
        assert_eq!(state.captain_status(0), Status::Failed);
        state.current_mut().life.archived_at = Some(1);
        assert_eq!(state.captain_status(0), Status::Running);
        assert_eq!(state.captain_status(1), Status::Ready);
    }
    #[test]
    fn background_startup_does_not_change_the_conversation_status() {
        let mut thread = Thread::new(1);
        let (pi, _, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false}}));
        assert_eq!(thread.status, Status::Ready);
        assert!(thread.initializing);
        thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}));
        assert_eq!(thread.status, Status::Ready);
        assert!(!thread.initializing);
        thread.status = Status::Running;
        thread.apply(json!({"type":"response","id":"later-state","command":"get_state","success":true,"data":{"isStreaming":false}}));
        assert_eq!(
            thread.status,
            Status::Running,
            "stale state must not settle a newer prompt"
        );
    }
    #[test]
    fn a_confirmation_waits_for_the_user_and_is_never_auto_approved() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"extension_ui_request","id":"permission","method":"confirm","title":"Run command?"}));
        assert_eq!(thread.status, Status::Waiting);
        assert_eq!(thread.request.as_ref().unwrap()["id"], "permission");
        thread.apply(json!({"type":"agent_settled","aborted":false}));
        assert!(thread.request.is_none());
    }
    #[test]
    fn reconnect_restores_tool_calls_and_results_without_duplicates() {
        let mut thread = Thread::new(1);
        let history = json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":"Check the file"},
            {"role":"assistant","content":[{"type":"toolCall","id":"read-1","name":"read","arguments":{"path":"example.txt"}}]},
            {"role":"toolResult","toolCallId":"read-1","toolName":"read","content":[{"type":"text","text":"saved output"}],"isError":false},
            {"role":"assistant","content":[{"type":"text","text":"Done"}]}
        ]}});
        thread.apply(history.clone());
        thread.apply(history);
        assert_eq!(thread.messages.len(), 3);
        assert_eq!(thread.tools.len(), 1);
        assert_eq!(thread.tools[0].detail, "example.txt");
        assert_eq!(thread.tools[0].output, "saved output");
        assert!(thread.tools[0].done && !thread.tools[0].failed);
    }
    #[test]
    fn live_tools_stay_between_the_messages_where_they_ran() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Looking now."},{"type":"toolCall","id":"read","name":"read","arguments":{"path":"README.md"}}]}}));
        thread.apply(json!({"type":"tool_execution_start","toolCallId":"read","toolName":"read","args":{"path":"README.md"}}));
        thread.apply(json!({"type":"tool_execution_update","toolCallId":"read","partialResult":{"content":[{"type":"text","text":"partial"}]}}));
        assert_eq!(
            thread.tools.len(),
            1,
            "message blocks and execution events share one call"
        );
        assert_eq!(thread.tools[0].output, "partial");
        assert!(!thread.tools[0].done);
        thread.apply(json!({"type":"tool_execution_end","toolCallId":"read","durationMs":75,"result":{"content":[{"type":"text","text":"contents"}]},"isError":false}));
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Here is the answer."}]}}));
        thread.apply(json!({"type":"tool_execution_start","toolCallId":"command","toolName":"powershell","args":{"command":"Get-ChildItem"}}));
        assert_eq!(
            thread.tools[0].anchor,
            ToolAnchor {
                message: 0,
                offset: "Looking now.".len()
            }
        );
        assert_eq!(thread.tools[0].duration_ms, Some(75));
        assert_eq!(
            thread.tools[1].anchor,
            ToolAnchor {
                message: 2,
                offset: 0
            }
        );
        assert_eq!(thread.messages[1].text, "Here is the answer.");
    }
    #[test]
    fn live_tools_can_anchor_inside_a_streamed_unicode_message() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"message_start","message":{"role":"assistant"}}));
        thread.apply(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"日本語"}}));
        thread.apply(json!({"type":"tool_execution_start","toolCallId":"read","toolName":"read","args":{"path":"例.txt"}}));
        thread.apply(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"\nFinished"}}));
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"日本語"},{"type":"toolCall","id":"read","name":"read","arguments":{"path":"例.txt"}},{"type":"text","text":"Finished"}]}}));
        assert_eq!(thread.tools.len(), 1);
        assert_eq!(
            thread.tools[0].anchor,
            ToolAnchor {
                message: 0,
                offset: "日本語".len()
            }
        );
        assert_eq!(thread.messages[0].text, "日本語\nFinished");
        thread.apply(json!({"type":"tool_execution_end","toolCallId":"read","result":{"content":[]},"isError":true}));
        assert!(thread.tools[0].done && thread.tools[0].failed);
        assert!(thread.tools[0].duration_ms.is_some());
    }
    #[test]
    fn restored_tools_preserve_block_order_and_tool_only_turns() {
        let mut thread = Thread::new(1);
        let history = json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":"Read the files"},
            {"role":"assistant","content":[
                {"type":"text","text":"First"},
                {"type":"toolCall","id":"a","name":"read","arguments":{"path":"a.txt"}},
                {"type":"toolCall","id":"b","name":"read","arguments":{"path":"b.txt"}},
                {"type":"text","text":"Then"},
                {"type":"toolCall","id":"c","name":"bash","arguments":{"command":"pwd"}},
                {"type":"text","text":"Done"}
            ]},
            {"role":"toolResult","toolCallId":"b","content":[{"type":"text","text":"b contents"}],"isError":false,"durationMs":40},
            {"role":"toolResult","toolCallId":"a","content":[{"type":"text","text":"a contents"}],"isError":false},
            {"role":"assistant","content":[{"type":"toolCall","id":"d","name":"read","arguments":{"path":"d.txt"}}]},
            {"role":"assistant","content":[{"type":"text","text":"Final reply"}]}
        ]}});
        thread.apply(history.clone());
        thread.tools[0].expanded = true;
        thread.apply(history);
        assert_eq!(thread.messages.len(), 4);
        assert_eq!(thread.messages[1].text, "First\nThen\nDone");
        let anchors: Vec<_> = thread.tools.iter().map(|tool| tool.anchor).collect();
        assert_eq!(
            anchors,
            vec![
                ToolAnchor {
                    message: 1,
                    offset: 5
                },
                ToolAnchor {
                    message: 1,
                    offset: 5
                },
                ToolAnchor {
                    message: 1,
                    offset: 10
                },
                ToolAnchor {
                    message: 2,
                    offset: 0
                },
            ]
        );
        assert_eq!(thread.tools[1].output, "b contents");
        assert_eq!(thread.tools[1].duration_ms, Some(40));
        assert_eq!(
            thread.tools[0].duration_ms, None,
            "restored timings are not invented"
        );
        assert!(thread.tools[2].failed, "missing saved results stay visible");
        assert!(
            !thread.tools[0].expanded,
            "restoring rebuilds transient disclosure state"
        );
    }

    #[test]
    fn thinking_deltas_use_content_indices_and_completed_blocks_replace_them() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"agent_start"}));
        thread.apply(json!({"type":"message_start","message":{"role":"assistant"}}));
        for event in [
            json!({"type":"thinking_start","contentIndex":0}),
            json!({"type":"thinking_delta","contentIndex":0,"delta":"Partial reasoning"}),
            json!({"type":"thinking_end","contentIndex":0,"content":"Complete reasoning"}),
            json!({"type":"text_delta","contentIndex":1,"delta":"I'll read the file."}),
            json!({"type":"thinking_start","contentIndex":2}),
            json!({"type":"thinking_delta","contentIndex":2,"delta":"Second thought"}),
        ] {
            thread.apply(json!({"type":"message_update","assistantMessageEvent":event}));
        }
        assert_eq!(thread.thoughts.len(), 2);
        assert_eq!(thread.thoughts[0].text, "Complete reasoning");
        assert!(thread.thoughts[0].done);
        assert!(thread.thoughts[0].duration_ms.is_some());
        assert!(!thread.thoughts[1].done);
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[
            {"type":"thinking","thinking":"Authoritative reasoning","thinkingSignature":"opaque"},
            {"type":"text","text":"I'll read the file."},
            {"type":"thinking","thinking":"Complete second thought"},
            {"type":"toolCall","id":"read","name":"read","arguments":{"path":"file.txt"}}
        ]}}));
        thread.apply(json!({"type":"tool_execution_start","toolCallId":"read","toolName":"read","args":{"path":"file.txt"}}));
        assert_eq!(thread.messages.len(), 1);
        assert_eq!(thread.thoughts.len(), 2);
        assert_eq!(thread.thoughts[0].text, "Authoritative reasoning");
        assert_eq!(thread.thoughts[1].text, "Complete second thought");
        assert!(thread.thoughts[1].done);
        assert_eq!(thread.thoughts[1].anchor, thread.tools[0].anchor);
        assert!(thread.thoughts[1].block < thread.tools[0].block);
        assert_eq!(thread.tools.len(), 1);
    }

    #[test]
    fn restored_thinking_keeps_text_and_redaction_without_inventing_timings() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"response","command":"get_messages","id":"history","success":true,"data":{"messages":[
            {"role":"user","content":"Check"},
            {"role":"assistant","content":[{"type":"thinking","thinking":"Saved thinking"}]},
            {"role":"assistant","content":[
                {"type":"thinking","redacted":true,"thinkingSignature":"never render this"},
                {"type":"text","text":"Done"}
            ]}
        ]}}));
        assert_eq!(thread.messages.len(), 3);
        assert_eq!(thread.thoughts.len(), 2);
        assert_eq!(thread.thoughts[0].text, "Saved thinking");
        assert!(thread.thoughts[1].text.is_empty());
        assert!(
            thread
                .thoughts
                .iter()
                .all(|thought| thought.done && thought.duration_ms.is_none())
        );
        assert_eq!(thread.work.len(), 1);
        assert!(!thread.work[0].expanded, "restored turns start folded");
        assert!(thread.work[0].elapsed.is_none());
    }

    #[test]
    fn each_user_turn_keeps_its_own_work_duration_and_disclosure() {
        let mut thread = Thread::new(1);
        for prompt in ["First", "Second"] {
            thread.messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "You",
                text: prompt.into(),
                images: Vec::new(),
                context: None,
            });
            thread.apply(json!({"type":"agent_start"}));
            thread.started = Some(Instant::now() - Duration::from_secs(3));
            thread.apply(
                json!({"type":"message_end","message":{"role":"assistant","content":[
                    {"type":"thinking","thinking":"Consider the answer"},
                    {"type":"text","text":"Answer"}
                ]}}),
            );
            thread.apply(json!({"type":"agent_settled"}));
            thread.work.last_mut().unwrap().expanded = false;
        }
        assert_eq!(thread.work.len(), 2);
        assert_eq!(thread.work[0].start, 1);
        assert_eq!(thread.work[1].start, 3);
        assert!(
            thread
                .work
                .iter()
                .all(|work| work.elapsed == Some(3) && !work.expanded)
        );
    }
    #[test]
    fn rejected_prompt_returns_to_the_draft_and_does_not_claim_it_was_sent() {
        let mut thread = Thread::new(1);
        thread.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: "You",
            text: "Try this".into(),
            images: Vec::new(),
            context: None,
        });
        thread.pending_prompt = Some(("1-1".into(), Prompt::new("Try this".into(), vec![]), 0));
        thread.status = Status::Running;
        thread.apply(json!({"type":"response","id":"1-1","command":"prompt","success":false,"error":"No model configured"}));
        assert_eq!(thread.status, Status::Failed);
        assert_eq!(thread.draft.text, "Try this");
        assert!(thread.messages.is_empty());
        assert!(thread.queued_prompt.is_none());
    }
    #[test]
    fn failed_settings_leave_the_effective_model_and_level_unchanged() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"response","command":"set_model","success":true,"data":{"provider":"test","id":"actual","name":"Actual model"}}));
        thread.configuring = true;
        thread.pending_thinking = Some("high".into());
        thread.thinking = "low".into();
        thread.apply(json!({"type":"response","command":"set_thinking_level","success":false,"error":"Unsupported level"}));
        assert_eq!(thread.model_id, "actual");
        assert_eq!(thread.model_provider, "test");
        assert_eq!(thread.thinking, "low");
        assert!(!thread.configuring);
        assert!(thread.pending_thinking.is_none());
    }
    #[test]
    fn zoom_steps_clamp_and_reset() {
        let mut state = State::new(std::env::temp_dir(), true);
        state.step_zoom(1);
        assert_eq!(state.zoom, 1.1);
        for _ in 0..20 {
            state.step_zoom(1);
        }
        assert_eq!(state.zoom, 2.0);
        state.step_zoom(0);
        assert_eq!(state.zoom, 1.0);
        for _ in 0..20 {
            state.step_zoom(-1);
        }
        assert_eq!(state.zoom, 0.67);
        assert_eq!(snap_zoom(1.3), 1.25);
    }
    #[test]
    fn aborted_message_is_not_a_provider_failure() {
        let mut thread = Thread::new(1);
        thread.status = Status::Running;
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"aborted","errorMessage":"Request was aborted"}}));
        thread.apply(json!({"type":"agent_settled","aborted":true}));
        assert_eq!(thread.status, Status::Ready);
        assert!(thread.error.is_none());
        thread.apply(json!({"type":"response","command":"abort","success":true}));
        assert_eq!(thread.status, Status::Ready);
    }
    #[test]
    fn successful_retry_clears_the_transient_provider_error() {
        let mut thread = Thread::new(1);
        thread.status = Status::Running;
        thread.error = Some("Temporary provider error".into());
        thread.apply(json!({"type":"auto_retry_end","success":true}));
        thread.apply(json!({"type":"agent_settled","aborted":false}));
        assert_eq!(thread.status, Status::Completed);
        assert!(thread.error.is_none());
    }
    #[test]
    fn long_pastes_on_linked_threads_remain_inline_until_text_file_uploads_exist() {
        let mut state = State::new(".".into(), false);
        state.current_mut().machine = "linked-machine".into();
        let text = "x".repeat(crate::composer_paste::THRESHOLD);
        assert!(!state.fold_paste(&text));
        assert!(state.current().draft_images.is_empty());
        assert!(state.current().attachment_error.is_none());
    }

    #[test]
    fn linked_file_capability_enables_long_paste_folding_and_older_peers_keep_text_inline() {
        let mut state = State::new(".".into(), false);
        let (mut client, _local) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, _commands) = crate::client::Client::test_connected(state.data_dir.clone());
        client.linked.insert("linked-machine".into(), remote);
        state.client = Some(client);
        state.machines.selected = "linked-machine".into();
        state.current_mut().machine = "linked-machine".into();
        let text = "x".repeat(crate::composer_paste::THRESHOLD);
        assert!(!state.fold_paste(&text));
        *state.client.as_ref().unwrap().linked["linked-machine"]
            .info
            .lock()
            .unwrap() = json!({"capabilities":[crate::protocol::FILE_ATTACHMENTS]});
        assert!(state.fold_paste(&text));
        assert_eq!(state.current().draft_images.len(), 1);
        assert_eq!(
            attachments::transfer_file_bytes(&state.current().draft_images[0]).unwrap(),
            text.as_bytes()
        );
    }

    #[test]
    fn rich_draft_survives_ui_restore_and_sends_exact_markdown() {
        let mut state = State::new(".".into(), false);
        let edit = &mut state.current_mut().draft;
        edit.replace_text("bold\none\ntwo\nquote\ncode");
        edit.set_selection(0, 4);
        edit.toggle_mark(rune::Mark::Bold);
        edit.set_blocks(vec![
            rune::Block::Paragraph,
            rune::Block::Numbered(0),
            rune::Block::Numbered(0),
            rune::Block::Quote,
            rune::Block::Code,
        ]);
        let ui = state.ui_snapshot();
        let mut restored = State::new(".".into(), false);
        restored.restore_ui(&ui).unwrap();
        let expected = "**bold**\n1. one\n2. two\n> quote\n```\ncode\n```";
        assert_eq!(
            crate::composer_rich::markdown(&restored.current().draft),
            expected
        );
        assert!(restored.current().draft.is_rich());
        restored.current_mut().send();
        assert_eq!(restored.current().messages[0].text, expected);
        assert!(restored.current().draft.is_rich());
        let mut legacy = ui;
        legacy["drafts"][state.current().id.to_string()] = json!("plain old draft");
        state.restore_ui(&legacy).unwrap();
        assert!(state.current().draft.is_rich());
        assert_eq!(state.current().draft.text, "plain old draft");
    }

    #[test]
    fn failed_send_restores_rich_document_pills_and_later_rich_edits() {
        let mut thread = Thread::new(1);
        let original_image = test_attachment("original.png", 1);
        let later_image = test_attachment("later.png", 2);
        thread.draft_images.push(original_image.clone());
        thread.draft = crate::composer_rich::draft("Bold reference");
        thread.draft.set_selection(0, 4);
        thread.draft.toggle_mark(rune::Mark::Bold);
        let token = crate::composer_tokens::Token::Skill {
            name: "review".into(),
            harness: "pi".into(),
        };
        thread.draft.replace_with_pill(5..14, token.id(), "review");
        let original = crate::composer_rich::markdown(&thread.draft);
        thread.send();
        thread.draft.replace_text("Later");
        thread.draft.set_selection(0, 5);
        thread.draft.toggle_mark(rune::Mark::Italic);
        thread.draft_images.push(later_image.clone());
        thread.disconnect();
        assert_eq!(
            crate::composer_rich::markdown(&thread.draft),
            format!("{original}\n\n*Later*")
        );
        assert_eq!(thread.draft.pills().count(), 1);
        assert_eq!(thread.draft.text, "Bold review\n\nLater");
        assert!(thread.messages.is_empty());
        assert_eq!(thread.draft_images, vec![original_image, later_image]);
    }

    #[test]
    fn send_without_a_connection_is_immediate_and_dispatches_the_submitted_draft_once() {
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("Original draft");
        state.current_mut().send();
        assert_eq!(state.current().status, Status::Running);
        assert_eq!(state.current().messages[0].text, "Original draft");
        assert!(state.current().draft.text.is_empty());
        let (pi, commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().initializing = true;
        events.send(Event::Record(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}))).unwrap();
        let mut tx = Tx::default();
        poll(&mut state, 0.0, &mut tx);
        assert!(
            commands.try_recv().is_err(),
            "history alone must not start work"
        );
        state
            .current_mut()
            .draft
            .replace_text("Revised before connection");
        events.send(Event::Record(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false,"model":{"provider":"cpa","id":"model-a"}}}))).unwrap();
        poll(&mut state, 0.0, &mut tx);
        assert_eq!(commands.try_recv().unwrap()["type"], "get_session_stats");
        let prompt = commands.try_recv().unwrap();
        assert_eq!(prompt["type"], "prompt");
        assert_eq!(prompt["message"], "Original draft");
        assert_eq!(state.current().status, Status::Running);
        assert_eq!(state.current().draft.text, "Revised before connection");
        assert!(state.current().queued_prompt.is_none());
        assert_eq!(state.current().messages.len(), 1);
        poll(&mut state, 0.0, &mut tx);
        assert!(commands.try_recv().is_err(), "submit exactly once");
    }
    #[test]
    fn stopping_cancels_a_dialog_clears_the_queue_and_then_aborts() {
        let (pi, commands, _events) = Pi::test_connection();
        let mut thread = Thread::new(1);
        thread.pi = Some(pi);
        thread.apply(json!({"type":"extension_ui_request","id":"dialog-1","method":"confirm"}));
        thread.stop();
        let cancel = commands.try_recv().unwrap();
        assert_eq!(cancel["type"], "extension_ui_response");
        assert_eq!(cancel["id"], "dialog-1");
        assert_eq!(cancel["cancelled"], true);
        assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
        assert_eq!(commands.try_recv().unwrap()["type"], "abort");
        assert!(thread.request.is_none());
    }
    #[test]
    fn prewarming_alone_never_sends_a_model_prompt() {
        let (pi, commands, events) = Pi::test_connection();
        let mut state = State::new(".".into(), false);
        state.current_mut().pi = Some(pi);
        state.current_mut().initializing = true;
        state.current_mut().draft.replace_text("Do not send this");
        for record in [
            json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false}}),
            json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}),
        ] {
            events.send(Event::Record(record)).unwrap();
        }
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.current().status, Status::Ready);
        assert_eq!(state.current().draft.text, "Do not send this");
        assert_eq!(commands.try_recv().unwrap()["type"], "get_session_stats");
        assert!(
            commands.try_recv().is_err(),
            "startup stats never send a model prompt"
        );
    }
    #[test]
    fn direct_provider_models_are_hidden_and_cannot_receive_prompts() {
        let (pi, commands, _events) = Pi::test_connection();
        let mut thread = Thread::new(1);
        thread.pi = Some(pi);
        thread.status = Status::Ready;
        thread.apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"provider":"google","id":"direct","name":"Direct Google"},
            {"provider":"cpa","id":"proxy","name":"CPA proxy"}
        ]}}));
        assert_eq!(thread.models.len(), 1);
        assert_eq!(thread.models[0].id, "proxy");
        thread.model_provider = "google".into();
        thread.draft.replace_text("Do not send directly");
        thread.send();
        assert!(commands.try_recv().is_err());
        assert_eq!(thread.draft.text, "Do not send directly");
        assert!(thread.error.as_ref().unwrap().contains("CPA model"));
        thread.set_model("google".into(), "direct".into());
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn rejected_prompt_preserves_a_newer_draft_too() {
        let mut thread = Thread::new(1);
        thread.pending_prompt = Some((
            "1-1".into(),
            Prompt::new("Rejected prompt".into(), vec![]),
            0,
        ));
        thread.draft.replace_text("New draft");
        thread.apply(json!({"type":"response","id":"1-1","command":"prompt","success":false,"error":"Rejected"}));
        assert_eq!(thread.draft.text, "Rejected prompt\n\nNew draft");
    }
    #[test]
    fn stopping_startup_cancels_the_queued_first_send() {
        let (pi, commands, _events) = Pi::test_connection();
        let mut thread = Thread::new(1);
        thread.draft.replace_text("Keep this draft");
        thread.send();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.stop();
        assert_eq!(thread.status, Status::Ready);
        assert!(thread.pi.is_none());
        assert!(thread.queued_prompt.is_none());
        assert_eq!(thread.draft.text, "Keep this draft");
        assert!(thread.messages.is_empty());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn offline_picker_choices_are_applied_before_the_first_prompt() {
        let mut thread = Thread::new(1);
        thread.set_model("cpa".into(), "chosen".into());
        thread.set_thinking("high".into());
        assert_eq!(thread.model_id, "chosen");
        assert_eq!(thread.thinking, "high");
        assert!(thread.pi.is_none());
        thread.draft.replace_text("Use my choices");
        thread.send();
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
            "model":{"provider":"cpa","id":"default"},"thinkingLevel":"low","sessionFile":"saved.jsonl"
        }}));
        assert_eq!(
            thread.model_id, "chosen",
            "startup must not replace the saved choice"
        );
        thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}));
        let model = commands.try_recv().unwrap();
        assert_eq!(model["type"], "set_model");
        assert_eq!(model["modelId"], "chosen");
        assert!(
            commands.try_recv().is_err(),
            "wait for model acknowledgement"
        );
        thread.apply(json!({"type":"response","command":"set_model","success":true,"data":{"provider":"cpa","id":"chosen"}}));
        assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
        assert_eq!(
            commands.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        assert_eq!(
            commands.try_recv().unwrap()["type"],
            "get_session_stats",
            "a model change refreshes the live context window"
        );
        let thinking = commands.try_recv().unwrap();
        assert_eq!(thinking["type"], "set_thinking_level");
        assert_eq!(thinking["level"], "high");
        assert!(
            commands.try_recv().is_err(),
            "wait for thinking acknowledgement"
        );
        thread.apply(json!({"type":"response","command":"set_thinking_level","success":true}));
        assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
        assert_eq!(commands.try_recv().unwrap()["type"], "set_session_name");
        assert_eq!(commands.try_recv().unwrap()["type"], "prompt");
        assert!(!thread.initializing);
        assert_eq!(thread.session.as_deref(), Some("saved.jsonl"));
    }

    fn next_stats_request(commands: &mpsc::Receiver<Value>) -> Value {
        let mut request = commands
            .try_recv()
            .expect("expected a context stats request");
        if request["type"] == "set_session_name" {
            request = commands.try_recv().unwrap();
        }
        assert_eq!(request["type"], "get_session_stats");
        request
    }

    fn answer_stats(thread: &mut Thread, request: &Value, context_usage: Value) {
        thread.apply(json!({
            "type": "response",
            "id": request["id"],
            "command": "get_session_stats",
            "success": true,
            "data": {"contextUsage": context_usage}
        }));
    }

    #[test]
    fn context_stats_refresh_on_startup_turns_and_model_changes() {
        let mut thread = Thread::new(1);
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.context_startup_requested = false;
        thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
            "isStreaming":false,
            "model":{"provider":"cpa","id":"vision","name":"Vision","contextWindow":200000,"input":["text","image"]},
            "sessionFile":"saved.jsonl"
        }}));
        thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}));

        let startup = next_stats_request(&commands);
        assert!(thread.context.is_pending());
        assert_eq!(thread.context_window, Some(200000));
        assert_eq!(thread.supports_images, Some(true));
        answer_stats(
            &mut thread,
            &startup,
            json!({"tokens":60000,"contextWindow":200000,"percent":30}),
        );
        assert_eq!(thread.context.usage.tokens, Some(60000));
        assert_eq!(thread.context.usage.percent, Some(30.0));
        assert!(!thread.context.is_pending());

        thread.apply(json!({"type":"agent_settled","aborted":false}));
        let turn = next_stats_request(&commands);
        answer_stats(
            &mut thread,
            &turn,
            json!({"tokens":90000,"contextWindow":200000,"percent":45}),
        );
        assert_eq!(thread.context.usage.percent, Some(45.0));

        thread.models = vec![
            AgentModel::from_value(&json!({
                "provider":"cpa","id":"other","name":"Other","contextWindow":100000,"input":["text"]
            }))
            .unwrap(),
        ];
        thread.set_model("cpa".into(), "other".into());
        assert_eq!(commands.try_recv().unwrap()["type"], "set_model");
        thread.apply(
            json!({"type":"response","command":"set_model","success":true,"data":{
                "provider":"cpa","id":"other","name":"Other","contextWindow":100000,"input":["text"]
            }}),
        );
        assert!(
            thread.context.usage.percent.is_none(),
            "the old model's percentage must not survive a model change"
        );
        assert_eq!(thread.context_window, Some(100000));
        assert_eq!(thread.supports_images, Some(false));
        assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
        assert_eq!(
            commands.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        let changed = next_stats_request(&commands);
        answer_stats(
            &mut thread,
            &changed,
            json!({"tokens":20000,"contextWindow":100000,"percent":20}),
        );
        assert_eq!(thread.context.usage.percent, Some(20.0));
    }

    #[test]
    fn compaction_marks_context_unknown_until_fresh_usage_arrives() {
        let mut thread = Thread::new(1);
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.context_window = Some(200000);
        thread.context.absorb(&json!({
            "contextUsage":{"tokens":120000,"contextWindow":200000,"percent":60}
        }));
        thread.apply(json!({"type":"compaction_start"}));
        assert_eq!(
            thread.context.phase,
            crate::context::ContextPhase::Compacting
        );
        assert_eq!(thread.context.usage.tokens, None);
        assert_eq!(thread.context.usage.percent, None);
        assert_eq!(thread.context.usage.context_window, Some(200000));
        assert!(
            commands.try_recv().is_err(),
            "compaction start must not start a polling loop"
        );

        thread.apply(json!({"type":"compaction_end"}));
        assert_eq!(
            thread.context.phase,
            crate::context::ContextPhase::AwaitingUsage
        );
        let after_compaction = next_stats_request(&commands);
        answer_stats(
            &mut thread,
            &after_compaction,
            json!({"tokens":null,"contextWindow":200000,"percent":null}),
        );
        assert_eq!(thread.context.usage.tokens, None);
        assert_eq!(thread.context.usage.percent, None);
        assert!(
            thread
                .context
                .accessible_label(None)
                .contains("unknown after compaction")
        );

        thread.apply(json!({"type":"agent_settled","aborted":false}));
        let fresh = next_stats_request(&commands);
        answer_stats(
            &mut thread,
            &fresh,
            json!({"tokens":5000,"contextWindow":200000,"percent":2.5}),
        );
        assert_eq!(thread.context.phase, crate::context::ContextPhase::Idle);
        assert_eq!(thread.context.usage.percent, Some(2.5));
    }

    #[test]
    fn rejected_context_stats_never_become_a_conversation_error() {
        let mut thread = Thread::new(1);
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.context.requested("1-9".into());
        thread.apply(json!({
            "type":"response","id":"1-9","command":"get_session_stats","success":false,
            "error":"get_session_stats is unavailable"
        }));
        assert!(!thread.context.is_pending());
        assert!(thread.error.is_none());
        assert!(!thread.configuring);
        assert_eq!(thread.status, Status::Ready);
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn failed_context_transport_preserves_the_conversation_error() {
        let mut thread = Thread::new(1);
        let (pi, commands, _events) = Pi::test_connection();
        drop(commands);
        thread.pi = Some(pi);
        thread.error = Some("Preserve this provider error".into());
        assert!(!thread.refresh_context_usage());
        assert_eq!(
            thread.error.as_deref(),
            Some("Preserve this provider error")
        );
        assert!(!thread.context.is_pending());
        assert!(!thread.configuring);
        assert_eq!(thread.status, Status::Ready);
    }

    #[test]
    fn a_completed_turn_invalidates_an_older_in_flight_context_measurement() {
        let mut thread = Thread::new(1);
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        assert!(thread.refresh_context_usage());
        let old = next_stats_request(&commands);
        thread.apply(json!({"type":"agent_settled","aborted":false}));
        let current = next_stats_request(&commands);
        assert_ne!(old["id"], current["id"]);
        answer_stats(
            &mut thread,
            &old,
            json!({"tokens":10,"contextWindow":100,"percent":10}),
        );
        assert!(thread.context.usage.percent.is_none());
        assert!(thread.context.is_pending());
        answer_stats(
            &mut thread,
            &current,
            json!({"tokens":30,"contextWindow":100,"percent":30}),
        );
        assert_eq!(thread.context.usage.percent, Some(30.0));
        assert!(!thread.context.is_pending());
    }

    #[test]
    fn catalog_image_metadata_and_rejected_model_changes_preserve_actual_support() {
        let mut state = State::new(".".into(), false);
        state.current_mut().model_id = "vision".into();
        state.catalog.models = vec![
            AgentModel::from_value(&json!({
                "provider":"cpa","id":"vision","input":["text","image"],"contextWindow":200000
            }))
            .unwrap(),
        ];
        state.apply_catalog();
        assert_eq!(state.current().supports_images, Some(true));
        assert_eq!(state.current().context_window, Some(200000));
        // An incomplete discovery record must not erase Pi's already-known capability.
        state.catalog.models = vec![
            AgentModel::from_value(&json!({
                "provider":"cpa","id":"vision"
            }))
            .unwrap(),
        ];
        state.apply_catalog();
        assert_eq!(state.current().supports_images, Some(true));
        let (commands, _events) = fake_pi(state.current_mut(), Status::Ready);
        state
            .current_mut()
            .set_model("cpa".into(), "missing".into());
        assert_eq!(commands.try_recv().unwrap()["type"], "set_model");
        state.current_mut().apply(json!({
            "type":"response","command":"set_model","success":false,"error":"Unavailable"
        }));
        assert_eq!(state.current().model_id, "vision");
        assert_eq!(state.current().supports_images, Some(true));
    }

    #[test]
    fn idle_reclaim_preserves_the_session_and_reattaches_without_duplicate_history() {
        let mut thread = Thread::new(1);
        let (pi, _commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.session = Some("saved.jsonl".into());
        thread.model_provider = "cpa".into();
        thread.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: "Pi",
            text: "Previous answer".into(),
            images: Vec::new(),
            context: None,
        });
        thread.status = Status::Completed;
        let idle = thread.last_activity + IDLE_TIMEOUT;
        thread.status = Status::Running;
        assert!(!thread.reclaim_idle(idle), "never reclaim work");
        thread.status = Status::Waiting;
        assert!(!thread.reclaim_idle(idle), "never reclaim a dialog");
        thread.status = Status::Completed;
        assert!(thread.reclaim_idle(idle));
        assert!(thread.pi.is_none());
        assert_eq!(thread.status, Status::Completed);
        assert_eq!(thread.session.as_deref(), Some("saved.jsonl"));
        assert_eq!(thread.messages.len(), 1);
        thread.draft.replace_text("Continue");
        thread.send();
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.state_loaded = false;
        thread.history_loaded = false;
        thread.apply(
            json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{
                "messages":[{"role":"assistant","content":"Previous answer"}]
            }}),
        );
        thread.apply(
            json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
                "model":{"provider":"cpa","id":"restored"},"sessionFile":"saved.jsonl"
            }}),
        );
        assert_eq!(commands.try_recv().unwrap()["type"], "set_session_name");
        assert_eq!(commands.try_recv().unwrap()["type"], "get_session_stats");
        assert_eq!(commands.try_recv().unwrap()["message"], "Continue");
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.messages[0].text, "Previous answer");
        assert_eq!(thread.messages[1].text, "Continue");
    }

    #[test]
    fn a_crash_restores_an_unaccepted_message_and_retry_uses_a_fresh_transport() {
        let mut state = State::new(".".into(), false);
        let (pi, _commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().model_provider = "cpa".into();
        state.current_mut().session = Some("saved.jsonl".into());
        state.current_mut().draft.replace_text("Try this");
        state.send();
        events.send(Event::Exited).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.current().status, Status::Failed);
        assert!(state.current().pi.is_none());
        assert_eq!(state.current().draft.text, "Try this");
        assert!(state.current().messages.is_empty());
        let (pi, commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().initializing = true;
        state.retry();
        assert_eq!(state.current().status, Status::Running);
        assert_eq!(state.current().messages.len(), 1);
        for record in [
            json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"model":{"provider":"cpa","id":"restored"},"sessionFile":"saved.jsonl"}}),
            json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}),
        ] {
            events.send(Event::Record(record)).unwrap();
        }
        poll(&mut state, 0.0, &mut Tx::default());
        let prompt = commands
            .try_iter()
            .find(|command| command["type"] == "prompt")
            .expect("retried prompt after the handshake");
        assert_eq!(prompt["message"], "Try this");
        assert!(state.current().error.is_none());
    }

    #[test]
    fn an_accepted_run_is_not_replayed_automatically_after_a_crash() {
        let mut state = State::new(".".into(), false);
        let (pi, commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().model_provider = "cpa".into();
        state.current_mut().draft.replace_text("Edit a file");
        state.send();
        let prompt = commands.try_recv().unwrap();
        state
            .current_mut()
            .apply(json!({"type":"response","command":"prompt","id":prompt["id"],"success":true}));
        events
            .send(Event::Error("Pi process crashed".into()))
            .unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.current().status, Status::Failed);
        assert!(state.current().draft.text.is_empty());
        assert_eq!(
            state
                .current()
                .retry_prompt
                .as_ref()
                .map(|prompt| prompt.text.as_str()),
            Some("Edit a file")
        );
        assert!(state.current().pi.is_none());
        assert_eq!(state.current().messages.len(), 1);
    }

    #[test]
    fn failed_startup_settings_restore_the_message_instead_of_sending_with_defaults() {
        let mut thread = Thread::new(1);
        thread.set_model("cpa".into(), "missing".into());
        thread.draft.replace_text("Keep my model choice");
        thread.send();
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.state_loaded = true;
        thread.history_loaded = true;
        thread.drive_session();
        assert_eq!(commands.try_recv().unwrap()["type"], "set_model");
        thread.apply(json!({"type":"response","command":"set_model","success":false,"error":"Model unavailable"}));
        assert_eq!(thread.status, Status::Failed);
        assert_eq!(thread.draft.text, "Keep my model choice");
        assert!(thread.pi.is_none());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn a_closed_startup_command_channel_cannot_skip_the_saved_model_choice() {
        let mut thread = Thread::new(1);
        thread.set_model("cpa".into(), "chosen".into());
        thread.draft.replace_text("Use the chosen model");
        thread.send();
        let (pi, commands, _events) = Pi::test_connection();
        drop(commands);
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.state_loaded = true;
        thread.history_loaded = true;
        thread.drive_session();
        assert_eq!(thread.status, Status::Failed);
        assert_eq!(thread.draft.text, "Use the chosen model");
        assert!(thread.pi.is_none());
    }

    #[test]
    fn a_partial_handshake_times_out_without_losing_the_submitted_message() {
        let mut state = State::new(".".into(), false);
        let (pi, commands, _events) = Pi::test_connection();
        state.current_mut().draft.replace_text("Keep this message");
        state.current_mut().send();
        let thread = state.current_mut();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.session_started = Some(Instant::now() - START_TIMEOUT);
        thread.state_loaded = true;
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.current().status, Status::Failed);
        assert_eq!(state.current().draft.text, "Keep this message");
        assert!(state.current().pi.is_none());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn catalog_defaults_do_not_overwrite_a_saved_thinking_choice() {
        let mut state = State::new(".".into(), false);
        state.current_mut().set_thinking("high".into());
        let model = AgentModel::from_value(
            &json!({"provider":"cpa","id":"default","name":"Default CPA","reasoning":true}),
        )
        .unwrap();
        state.catalog.models.push(model.clone());
        state.catalog.default_model = Some(model);
        state.catalog.default_thinking = "low".into();
        state.apply_catalog();
        assert_eq!(state.current().thinking, "high");
        assert_eq!(state.current().model, "Default CPA");
        state.current_mut().draft.replace_text("Write the report");
        state.new_thread();
        assert_eq!(state.current().model, "Default CPA");
        assert_eq!(state.current().thinking, "high");
        assert!(!state.current().models.is_empty());
    }

    #[test]
    fn creating_and_editing_a_captain_preserves_conversations_and_drafts() {
        let mut state = State::new(".".into(), false);
        // Rosters start empty; these tests exercise the earlier named presets.
        state.captains = crate::captains::Captain::presets();
        state
            .current_mut()
            .draft
            .replace_text("Original thread draft");
        state.begin_new_captain();
        let editor = state.captain_editor.as_mut().unwrap();
        editor.name.replace_text("Atlas");
        editor.role.replace_text("Review");
        editor.instructions.replace_text("Cite failing tests.");
        editor.portrait = 2;
        state.save_captain();
        assert_eq!(state.captains.len(), 5);
        assert_eq!(state.current().captain, Some(4));
        assert_eq!(state.current().title, "Atlas");
        assert!(
            state.current().pi.is_none(),
            "creation must not start agent work"
        );
        assert_eq!(state.threads[0].draft.text, "Original thread draft");
        let home_id = state.current().id;
        state.new_thread();
        state.current_mut().draft.replace_text("Crew draft");
        let crew_id = state.current().id;
        assert_eq!(
            state.current().captain,
            None,
            "New thread creates a lone conversation"
        );
        state.edit_captain(4);
        state
            .captain_editor
            .as_mut()
            .unwrap()
            .name
            .replace_text("Atlas Review");
        state.save_captain();
        assert_eq!(state.current().id, home_id);
        assert_eq!(state.current().title, "Atlas Review");
        assert_eq!(
            state
                .threads
                .iter()
                .find(|t| t.id == crew_id)
                .unwrap()
                .draft
                .text,
            "Crew draft"
        );
        assert!(state.captains[4].prompt().contains("Cite failing tests."));
    }

    #[test]
    fn duplicate_names_and_busy_captain_edits_do_not_save() {
        let mut state = State::new(".".into(), false);
        // Rosters start empty; these tests exercise the earlier named presets.
        state.captains = crate::captains::Captain::presets();
        state.begin_new_captain();
        state
            .captain_editor
            .as_mut()
            .unwrap()
            .name
            .replace_text(" ember ");
        state.save_captain();
        assert_eq!(state.captains.len(), 4);
        assert!(
            state
                .captain_editor
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("already")
        );
        state.captain_editor = None;
        state.open_captain(2);
        state.current_mut().status = Status::Running;
        state.edit_captain(2);
        state
            .captain_editor
            .as_mut()
            .unwrap()
            .name
            .replace_text("Changed");
        state.save_captain();
        assert_eq!(state.captains[2].name, "Sable");
        assert_eq!(state.current().status, Status::Running);
        assert!(
            state
                .captain_editor
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("Stop")
        );
    }

    #[test]
    fn global_migration_remaps_collisions_parents_sessions_and_captain_rosters() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("shina-profile-{}-{unique}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let a = directory.join("a");
        let b = directory.join("b");
        let missing = directory.join("disconnected-drive");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::create_dir(&missing).unwrap();
        fs::write(
            directory.join("projects.json"),
            json!({"projects":[{"path":a},{"path":b}]}).to_string(),
        )
        .unwrap();
        fs::write(
            directory.join("last-workspace.txt"),
            missing.to_string_lossy().as_bytes(),
        )
        .unwrap();
        let wake: Waker = Arc::new(|| {});
        let mut profiles = Captain::presets();
        profiles[2].name = "Atlas".into();
        for (i, workspace) in [&a, &b, &missing].into_iter().enumerate() {
            let index_path = crate::store::legacy_path(&directory, workspace);
            let mut store = crate::store::Store::new(index_path, wake.clone());
            store.save(json!({"version":2,"dark":true,"captains":profiles.iter().map(Captain::to_value).collect::<Vec<_>>(),"threads":[
                {"id":10,"title":"Original title","captain":2,"captainHome":true,"session":format!("session-{i}-home"),"model":"saved-model","thinking":"high"},
                {"id":11,"title":"Crew","captain":2,"delegated":true,"parent":10,"pinned":true,"archived":true,"session":format!("session-{i}-child")},
                {"id":12,"title":"Former home","captain":1,"captainHome":true,"session":format!("session-{i}-old-home")}
            ]}));
        }
        let index = crate::store::load_global(&directory, None)
            .unwrap()
            .unwrap();
        assert_eq!(index["projects"].as_array().unwrap().len(), 3);
        let before: Vec<_> = [&a, &b, &missing]
            .iter()
            .map(|p| fs::read(crate::store::legacy_path(&directory, p)).unwrap())
            .collect();
        {
            let mut state = State::new(PathBuf::new(), false);
            state.data_dir = directory.clone();
            state.load_global(wake.clone());
            assert!(state.storage_error.is_none());
            assert_eq!(state.threads.len(), 9);
            let ids: std::collections::HashSet<_> = state.threads.iter().map(|t| t.id).collect();
            assert_eq!(ids.len(), 9);
            assert_eq!(state.threads[0].id, 10);
            for (i, project) in state.projects.entries.iter().enumerate() {
                let child = state
                    .threads
                    .iter()
                    .find(|t| t.project == Some(project.id) && t.delegated)
                    .unwrap();
                let parent = state
                    .threads
                    .iter()
                    .find(|t| Some(t.id) == child.parent)
                    .unwrap();
                assert_eq!(
                    child.session.as_deref(),
                    Some(format!("session-{i}-child").as_str())
                );
                assert_eq!(
                    parent.session.as_deref(),
                    Some(format!("session-{i}-home").as_str())
                );
                assert_eq!(parent.project, child.project);
                assert_eq!(parent.title, "Original title");
                assert!(child.pinned() && child.archived() && child.delegated);
                assert_eq!(
                    parent.selected_model,
                    Some(("cpa".into(), "saved-model".into()))
                );
                assert_eq!(parent.selected_thinking.as_deref(), Some("high"));
                assert_eq!(state.captains[project.captain.unwrap()].name, "Atlas");
                assert_eq!(project.captain_thread, Some(parent.id));
                assert_eq!(
                    state
                        .threads
                        .iter()
                        .filter(|t| t.project == Some(project.id) && t.captain_home)
                        .count(),
                    2
                );
                assert_eq!(
                    state
                        .threads
                        .iter()
                        .filter(|t| t.project == Some(project.id) && state.is_captain_home(t))
                        .count(),
                    2
                );
            }
            state.checkpoint();
        }
        assert_eq!(
            crate::store::load(&directory.join("index.json"))
                .unwrap()
                .unwrap()["version"],
            4
        );
        {
            let mut restored = State::new(PathBuf::new(), false);
            restored.data_dir = directory.clone();
            restored.load_global(wake);
            assert!(restored.storage_error.is_none());
            assert_eq!(restored.threads.len(), 9);
        }
        for (i, workspace) in [&a, &b, &missing].iter().enumerate() {
            assert_eq!(
                fs::read(crate::store::legacy_path(&directory, workspace)).unwrap(),
                before[i]
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn concurrent_project_threads_keep_their_cwds_and_scratch_threads_are_isolated() {
        let mut state = State::new(".".into(), false);
        let a = state.thread_directory(0).unwrap();
        let b_id = state.add_project(std::env::temp_dir()).unwrap().unwrap();
        let b = state.active;
        let (a_commands, _a_events) = fake_pi(&mut state.threads[0], Status::Ready);
        let (b_commands, _b_events) = fake_pi(&mut state.threads[b], Status::Ready);
        state.active = 0;
        state.current_mut().draft.replace_text("Work in A");
        state.send();
        state.active = b;
        state.current_mut().draft.replace_text("Work in B");
        state.send();
        assert!(state.threads[0].status.busy() && state.threads[b].status.busy());
        assert_eq!(a_commands.try_recv().unwrap()["message"], "Work in A");
        assert_eq!(b_commands.try_recv().unwrap()["message"], "Work in B");
        assert_eq!(state.thread_directory(0).unwrap(), a);
        assert_eq!(
            state.thread_directory(b).unwrap(),
            state.project(Some(b_id)).unwrap().path
        );
        assert_ne!(
            state.thread_directory(0).unwrap(),
            state.thread_directory(b).unwrap()
        );
        state.current_mut().draft.replace_text("Scratch work");
        state.new_thread_in(None);
        let scratch = state.working_directory();
        let id = state.current().id;
        assert_eq!(scratch, state.data_dir.join("chats").join(id.to_string()));
        assert!(state.current().captain.is_none());
        state.current_mut().draft.replace_text("More scratch work");
        state.new_thread_in(None);
        assert_ne!(scratch, state.working_directory());
        state.current_mut().machine = "unavailable-host".into();
        assert!(state.thread_directory(state.active).is_err());
        assert!(crate::backend::BackendHost::resolve("local").is_ok());
        state.current_mut().machine = "local".into();
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        let index = restored.threads.iter().position(|t| t.id == id).unwrap();
        assert_eq!(restored.thread_directory(index).unwrap(), scratch);
    }

    #[test]
    fn mcp_lists_the_callers_project_without_changing_captain_ownership() {
        let mut state = State::new(".".into(), false);
        // Rosters start empty; these tests exercise the earlier named presets.
        state.captains = crate::captains::Captain::presets();
        state.open_captain(0);
        let first = state.current().id;
        state.open_captain(2);
        assert_ne!(
            state.current().id,
            first,
            "a second persona has an independent coordinator"
        );
        let b = state.add_project(std::env::temp_dir()).unwrap().unwrap();
        state.open_captain(1);
        let second = state.current().id;
        assert_ne!(first, second);
        assert_eq!(state.current().project, Some(b));
        state.current_mut().draft.replace_text("Scratch work");
        state.new_thread_in(None);
        let scratch = state.current().id;
        state.open_captain(3);
        assert_ne!(state.current().id, scratch);
        assert!(state.current().project.is_none());
        assert!(state.current().captain_home);
        // Active UI project must not affect the authenticated caller's scope.
        let list = state.mcp_call(first, "list_threads", &json!({})).unwrap();
        assert_eq!(
            list["threads"].as_array().unwrap().len(),
            state
                .threads
                .iter()
                .filter(|t| t.project == Some(1))
                .count()
        );
        assert!(
            list["threads"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["project"]["id"] == 1 && t["machine"] == "local")
        );
        let all = state
            .mcp_call(first, "list_threads", &json!({"allProjects":true}))
            .unwrap();
        assert_eq!(
            all["threads"].as_array().unwrap().len(),
            state.threads.len()
        );
        let scratch_list = state.mcp_call(scratch, "list_threads", &json!({})).unwrap();
        assert_eq!(
            scratch_list["threads"].as_array().unwrap().len(),
            state.threads.iter().filter(|t| t.project.is_none()).count()
        );
        state.demo = true; // Do not launch Pi in a unit test.
        let task = state
            .mcp_call(first, "delegate_task", &json!({"prompt":"A task"}))
            .unwrap();
        let child = state.threads.last().unwrap();
        assert_eq!(child.parent, Some(first));
        assert_eq!(child.project, Some(1));
        assert_eq!(child.captain, Some(0));
        assert_eq!(task["project"]["id"], 1);
        assert_eq!(task["machine"], "local");
        assert!(
            state
                .mcp_call(
                    first,
                    "delegate_task",
                    &json!({"prompt":"Other captain","captain":"Juniper"})
                )
                .is_err()
        );
    }

    #[test]
    fn sidebar_is_pinned_then_recent_and_scopes_without_project_groups() {
        let mut state = State::new(".".into(), false);
        state.threads[0].updated = 10;
        state.threads[0].life.created_at = 10000;
        state.current_mut().draft.replace_text("First");
        state.new_thread();
        state.current_mut().updated = 30;
        state.current_mut().life.created_at = 30000;
        state.current_mut().draft.replace_text("Second");
        state.new_thread_in(None);
        state.current_mut().updated = 20;
        state.current_mut().life.created_at = 20000;
        state.current_mut().life.pinned_at = Some(1);
        for t in &mut state.threads {
            t.life.latest_user_message_at = Some(t.life.created_at);
        }
        let ids = |s: &State| {
            s.sidebar_threads()
                .iter()
                .map(|&i| s.threads[i].id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&state), vec![3, 2, 1]);
        state.project_scope = ProjectScope::Project(1);
        assert_eq!(ids(&state), vec![2, 1]);
        state.project_scope = ProjectScope::NoProject;
        assert_eq!(ids(&state), vec![3]);
        state.current_mut().life.archived_at = Some(1);
        assert!(ids(&state).is_empty());
        state.show_archive = true;
        assert_eq!(ids(&state), vec![3]);
    }

    #[test]
    #[cfg(windows)]
    fn restoring_and_focusing_verbatim_paths_keeps_one_project_and_its_threads() {
        let state = State::new(".".into(), false);
        let clean = state.projects.entries[0].path.clone();
        let verbatim = clean.canonicalize().unwrap();
        let mut index = state.snapshot();
        let mut duplicate = index["projects"][0].clone();
        duplicate["id"] = 2.into();
        duplicate["path"] = verbatim.to_string_lossy().as_ref().into();
        index["projects"].as_array_mut().unwrap().push(duplicate);
        index["threads"][0]["project"] = 2.into();
        index["projectScope"] = 2.into();
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&index).unwrap();
        assert_eq!(restored.projects.entries.len(), 1);
        assert_eq!(restored.projects.entries[0].path, clean);
        assert_eq!(restored.current().project, Some(1));
        assert_eq!(restored.project_scope, ProjectScope::Project(1));
        assert_eq!(restored.add_project(verbatim).unwrap(), Some(1));
        assert_eq!(restored.projects.entries.len(), 1);
        assert!(restored.projects.next_id > 2);
    }

    #[test]
    fn rename_and_confirmed_deletion_remove_children_not_project_files_or_other_threads() {
        let mut state = State::new(".".into(), false);
        state.begin_rename(1);
        state.rename_edit.replace_text("  A named thread  ");
        state.save_thread_name();
        assert_eq!(state.threads[0].title, "A named thread");
        state.new_thread_in(None);
        let survivor = state.current().id;
        state.active = 0;
        let (commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, child_commands, _events) = child(&mut state);
        let child_id = state.threads[index].id;
        state.tabs.push(child_id);
        let project_path = state.working_directory();
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":child_id,"wait_seconds":600}),
            reply,
        });
        state.request_delete(1);
        assert_eq!(
            state.threads.len(),
            3,
            "opening confirmation deletes nothing"
        );
        state.confirm_delete();
        assert_eq!(state.threads.len(), 1);
        assert_eq!(state.current().id, survivor);
        assert_eq!(state.tabs, vec![survivor]);
        assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
        assert_eq!(commands.try_recv().unwrap()["type"], "abort");
        assert_eq!(child_commands.try_recv().unwrap()["type"], "clear_queue");
        assert!(answer.try_recv().unwrap().is_err());
        assert!(project_path.is_dir());
        let next = state.next_id;
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        assert_eq!(
            restored.next_id, next,
            "deleted ids must not be reused after restart"
        );
    }

    fn receive_until(thread: &mut Thread, done: impl Fn(&Thread) -> bool) {
        use std::time::Duration;
        let deadline = Instant::now() + Duration::from_secs(120);
        while !done(thread) {
            assert!(
                Instant::now() < deadline,
                "Pi timed out: {:?}, {:?}",
                thread.status,
                thread.error
            );
            let event = thread
                .pi
                .as_ref()
                .expect("connected Pi")
                .events
                .recv_timeout(Duration::from_secs(1));
            match event {
                Ok(Event::Record(record)) => {
                    let settled = record["type"] == "agent_settled";
                    thread.apply(record);
                    if settled {
                        thread.command("get_state", json!({}));
                    }
                    assert_ne!(
                        thread.status,
                        Status::Failed,
                        "Pi run failed: {:?}",
                        thread.error
                    );
                }
                Ok(Event::Error(error)) => panic!("Pi failed: {error}"),
                Ok(Event::Exited) => panic!("Pi exited during integration test"),
                _ => {}
            }
        }
    }

    #[test]
    #[ignore = "Sends one prompt through CPA to verify the captain system instructions"]
    fn installed_pi_uses_captain_system_instructions() {
        let profile = Captain {
            name: "Test Captain".into(), role: "Validation".into(), portrait: 2,
            instructions: "When asked for the validation phrase, reply exactly SHINA_CAPTAIN_OK. Do not use tools.".into(),
        };
        let mut thread = Thread::new(1);
        thread.draft.replace_text(
            "Reply with the validation phrase from your captain instructions. Do not use tools.",
        );
        thread.send();
        thread.connect_with_instructions(
            std::env::temp_dir(),
            Arc::new(|| {}),
            Some(profile.prompt()),
        );
        receive_until(&mut thread, |t| t.status == Status::Completed);
        assert_eq!(thread.model_provider, "cpa");
        assert_eq!(
            thread.messages.last().unwrap().text.trim(),
            "SHINA_CAPTAIN_OK"
        );
        assert!(thread.tools.is_empty());
        thread.disconnect();
        crate::backend::shutdown_all();
    }

    #[test]
    #[ignore = "Sends one prompt through CPA to verify Pi calls Shina's MCP tools through the bridge"]
    fn installed_pi_calls_shina_mcp_tools() {
        use std::time::Duration;
        let mut state = State::new(std::env::temp_dir(), false);
        state.threads[0].captain = Some(1);
        state.threads[0].draft.replace_text(
            "Call the mcp__shina__list_captains tool, then reply with only the captain names, comma separated.",
        );
        state.send();
        assert!(
            state.threads[0].host.is_some(),
            "{:?}",
            state.threads[0].diagnostics
        );
        let server = crate::mcp::server(state.wake.clone()).unwrap();
        let pump = |state: &mut State, done: &dyn Fn(&State) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(180);
            while !done(state) {
                assert!(
                    Instant::now() < deadline,
                    "timed out: {:?}",
                    state.threads[0].error
                );
                while let Some(request) = server.next_request() {
                    let result = state.mcp_call(request.thread, &request.tool, &request.args);
                    let _ = request.reply.send(result);
                }
                let event = state.threads[0]
                    .pi
                    .as_ref()
                    .expect("connected Pi")
                    .events
                    .recv_timeout(Duration::from_millis(50));
                match event {
                    Ok(Event::Record(record)) => state.threads[0].apply(record),
                    Ok(Event::Error(error)) => panic!("Pi failed: {error}"),
                    Ok(Event::Exited) => panic!("Pi exited"),
                    _ => {}
                }
                assert_ne!(
                    state.threads[0].status,
                    Status::Failed,
                    "{:?}",
                    state.threads[0].error
                );
            }
        };
        pump(&mut state, &|s| s.threads[0].status == Status::Completed);
        let thread = &state.threads[0];
        assert!(
            thread
                .tools
                .iter()
                .any(|t| t.name == "mcp__shina__list_captains" && !t.failed),
            "tools: {:?}",
            thread.tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
        let reply = &thread.messages.last().unwrap().text;
        assert!(reply.contains("Ember") && reply.contains("Pip"), "{reply}");
        state.threads[0].disconnect();
        crate::backend::shutdown_all();
    }

    #[test]
    #[ignore = "Sends three CPA prompts to verify concurrent per-thread working directories"]
    fn installed_pi_runs_projects_and_scratch_concurrently() {
        use std::fs;
        let directory = std::env::temp_dir().join(format!(
            "shina-cwds-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let a = directory.join("project-a");
        let b = directory.join("project-b");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let mut state = State::new(a, false);
        state.data_dir = directory.join("app-data");
        state.add_project(b).unwrap();
        state.new_thread_in(None);
        let markers = ["CWD_PROJECT_A", "CWD_PROJECT_B", "CWD_SCRATCH"];
        for (index, marker) in markers.iter().enumerate() {
            let cwd = state.thread_directory(index).unwrap();
            fs::create_dir_all(&cwd).unwrap();
            fs::write(cwd.join("cwd-marker.txt"), marker).unwrap();
            state.active = index;
            state.current_mut().draft.replace_text(
                "Use the read tool to read the relative path cwd-marker.txt. Reply with only its contents. Do not edit files, run commands, or delegate."
            );
            state.send();
        }
        assert!(
            state
                .threads
                .iter()
                .all(|t| t.pi.is_some() && t.status.busy())
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        while state.threads.iter().any(|t| t.status.busy()) {
            poll(&mut state, 0.0, &mut Tx::default());
            assert!(
                state.threads.iter().all(|t| t.error.is_none()),
                "Pi errors: {:?}",
                state.threads.iter().map(|t| &t.error).collect::<Vec<_>>()
            );
            assert!(Instant::now() < deadline, "Concurrent cwd test timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
        for (thread, marker) in state.threads.iter_mut().zip(markers) {
            let reply = thread
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "Pi")
                .expect("assistant reply");
            assert!(reply.text.contains(marker), "{marker}: {}", reply.text);
            assert!(
                thread
                    .tools
                    .iter()
                    .any(|t| t.name == "read" && t.done && !t.failed)
            );
            thread.disconnect();
        }
        crate::backend::shutdown_all();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "Sends paid GPT 6.1 Sol and DeepSeek Flash max requests through CPA to verify explicit delegation and wake-ups"]
    fn installed_pi_parent_is_woken_by_child_completion() {
        let workspace = std::env::temp_dir().join(format!(
            "shina-crew-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&workspace).unwrap();
        let mut state = State::new(workspace.clone(), false);
        // Exercise the same MCP request pump as setup, without loading user state or
        // starting the updater. Otherwise model tool calls wait forever in this test.
        state.runtime_enabled = true;
        state.threads[0].set_model("cpa".into(), "gpt-6.1-sol".into());
        state.threads[0].set_thinking("off".into());
        state.threads[0].draft.replace_text(
            "This is an orchestration integration test. Call mcp__shina__orchestrator_capabilities first. Then call mcp__shina__delegate_task exactly once with title Ping, prompt 'Reply with only the word PONG. Do not use tools or delegate.', and target {\"provider\":\"cpa\",\"model\":\"opencode-go/deepseek-v4.1-flash\",\"options\":{\"reasoning\":\"max\"}}. Do not set wait_seconds. Then immediately reply with only WAITING and end your turn. Do not call any other tools or poll. When a later Shina task completion message arrives containing PONG, reply with only WOKEN_PONG. Do not delegate again."
        );
        state.send();
        let deadline = Instant::now() + Duration::from_secs(240);
        let mut previous = vec![];
        loop {
            poll(&mut state, 0.0, &mut Tx::default());
            let statuses: Vec<_> = state.threads.iter().map(|t| (t.id, t.status)).collect();
            if statuses != previous {
                println!("Pi crew: {statuses:?}");
                previous = statuses;
            }
            for thread in &state.threads {
                assert_ne!(
                    thread.status,
                    Status::Failed,
                    "thread {} failed: {:?}; diagnostics: {:?}",
                    thread.id,
                    thread.error,
                    thread.diagnostics
                );
            }
            let finished = state.threads[0].status == Status::Completed
                && state.threads[0]
                    .messages
                    .iter()
                    .any(|m| m.role == "Pi" && m.text.contains("WOKEN_PONG"));
            if finished {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "crew timed out; parent transcript: {:?}",
                state.threads[0]
                    .messages
                    .iter()
                    .map(|m| (&m.role, &m.text))
                    .collect::<Vec<_>>()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            state.threads.len(),
            2,
            "the parent must delegate exactly one child"
        );
        assert_eq!(state.threads[1].parent, Some(1));
        assert_eq!(state.threads[1].model_id, "opencode-go/deepseek-v4.1-flash");
        assert_eq!(state.threads[1].thinking, "max");
        assert!(state.threads[1].startup_selection_verified);
        assert!(
            state.threads[0]
                .tools
                .iter()
                .any(|t| t.name == "mcp__shina__delegate_task" && t.done && !t.failed)
        );
        assert!(
            state.threads[0]
                .messages
                .iter()
                .any(|m| m.role == "Pi" && m.text.trim() == "WAITING")
        );
        // A fast child can finish before WAITING settles. Both an idle wake-up and
        // a queued follow-up are valid. The two replies and single notice prove delivery.
        let notices: Vec<_> = state.threads[0]
            .messages
            .iter()
            .filter(|m| m.role == "Shina")
            .collect();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].text.contains("PONG"));
        println!("Parent first reply: WAITING");
        println!("Completion: {}", notices[0].text);
        println!(
            "Parent final reply: {}",
            state.threads[0].messages.last().unwrap().text
        );
        for thread in &mut state.threads {
            thread.disconnect();
        }
        crate::backend::shutdown_all();
        std::fs::remove_dir(&workspace).unwrap();
    }

    #[test]
    #[ignore = "Sends a paid prompt to the configured Pi model, edits only a temporary fixture"]
    fn installed_pi_edits_a_file_and_restores_the_session() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let workspace =
            std::env::temp_dir().join(format!("shina-live-{}-{unique}", std::process::id()));
        fs::create_dir(&workspace).unwrap();
        let fixture = workspace.join("fixture.txt");
        fs::write(&fixture, "answer=0\n").unwrap();
        let mut thread = Thread::new(1);
        thread.draft.replace_text(
            "This is an integration test in a disposable folder. Use your file tools to read fixture.txt, then change its entire contents to exactly answer=42 followed by one newline. Do not run shell commands, access any other file, or delegate. After the edit, reply briefly that it is done."
        );
        thread.send();
        thread.connect(workspace.clone(), Arc::new(|| {}));
        receive_until(&mut thread, |t| {
            t.tools
                .iter()
                .any(|tool| tool.started.is_some() && !tool.done)
        });
        thread = crate::ui::tests::verify_tool_rows(thread);
        receive_until(&mut thread, |t| {
            t.status == Status::Completed && t.session.is_some()
        });
        assert_eq!(fs::read_to_string(&fixture).unwrap(), "answer=42\n");
        assert!(
            thread
                .tools
                .iter()
                .any(|t| t.done && !t.failed && matches!(t.name.as_str(), "edit" | "write"))
        );
        assert!(
            thread
                .messages
                .iter()
                .any(|m| m.role == "Pi" && !m.text.is_empty())
        );
        let session = thread.session.clone().unwrap();
        let tool_count = thread.tools.len();
        let message_count = thread.messages.len();
        let thinking_text: Vec<_> = thread
            .thoughts
            .iter()
            .map(|thought| thought.text.clone())
            .collect();
        println!(
            "Live Pi tool rows: {tool_count}, thinking rows: {}",
            thread.thoughts.len()
        );
        thread = crate::ui::tests::verify_tool_rows(thread);
        thread.disconnect();
        crate::backend::shutdown_all();
        let mut restored = Thread::new(2);
        restored.session = Some(session);
        restored.connect(workspace.clone(), Arc::new(|| {}));
        receive_until(&mut restored, |t| !t.initializing);
        assert_eq!(restored.messages.len(), message_count);
        assert_eq!(restored.tools.len(), tool_count);
        assert!(restored.tools.iter().all(|t| t.done && !t.failed));
        assert_eq!(
            restored
                .thoughts
                .iter()
                .map(|thought| thought.text.clone())
                .collect::<Vec<_>>(),
            thinking_text
        );
        restored = crate::ui::tests::verify_tool_rows(restored);
        println!("Restored {tool_count} rendered tool rows from the saved Pi session.");
        restored.disconnect();
        crate::backend::shutdown_all();
        fs::remove_file(fixture).unwrap();
        fs::remove_dir(workspace).unwrap();
    }
    fn test_png(width: u32) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            1,
            image::Rgba([20, 40, 60, 255]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
        bytes.into_inner()
    }

    fn test_attachment(name: &str, width: u32) -> Attachment {
        crate::attachments::attach_bytes(name, test_png(width)).unwrap()
    }

    #[test]
    #[ignore = "Sends a paid GPT 6.1 Sol vision request through CPA and resumes its saved image history"]
    fn installed_pi_accepts_images_and_restores_them_from_history() {
        let workspace = crate::store::directory().unwrap().join("image-workspace");
        std::fs::create_dir(&workspace).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            64,
            64,
            image::Rgba([255, 0, 0, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let image = attachments::attach_bytes("color.png", png.into_inner()).unwrap();
        let mut thread = Thread::new(1);
        thread.set_model("cpa".into(), "gpt-6.1-sol".into());
        thread.set_thinking("off".into());
        thread.add_attachment(image.clone()).unwrap();
        thread.draft.replace_text(
            "What is the dominant color of the attached image? Reply only RED, GREEN or BLUE. Do not use tools."
        );
        thread.send();
        thread.connect(workspace.clone(), Arc::new(|| {}));
        receive_until(&mut thread, |t| {
            t.status == Status::Completed
                && t.session.is_some()
                && !t.context.is_pending()
                && t.context.usage.percent.is_some()
        });
        assert!(thread.context.usage.tokens.is_some_and(|tokens| tokens > 0));
        assert_eq!(thread.context.usage.context_window, thread.context_window);
        let reply = thread
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "Pi")
            .unwrap();
        assert_eq!(reply.text.trim(), "RED");
        let session = thread.session.clone();
        thread.disconnect();
        let mut resumed = Thread::new(2);
        resumed.session = session;
        resumed.connect(workspace, Arc::new(|| {}));
        receive_until(&mut resumed, |t| {
            !t.restoring_history()
                && !t.messages.is_empty()
                && !t.context.is_pending()
                && t.context.usage.percent.is_some()
        });
        let user = resumed.messages.iter().find(|m| m.role == "You").unwrap();
        assert_eq!(user.images.len(), 1);
        assert_eq!(
            attachments::read(&user.images[0]).unwrap(),
            attachments::read(&image).unwrap()
        );
        resumed.disconnect();
        crate::backend::shutdown_all();
    }

    #[test]
    fn image_only_prompts_send_typed_pi_images_with_ksg_path_context() {
        let mut thread = Thread::new(1);
        thread.model_provider = "cpa".into();
        thread.supports_images = Some(true);
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        let image = test_attachment("screen.png", 1);
        thread.add_attachment(image.clone()).unwrap();
        thread.send();
        let prompt = commands.try_recv().unwrap();
        assert_eq!(prompt["type"], "prompt");
        assert_eq!(
            prompt["message"],
            attachments::pi_message(&Prompt::new(String::new(), vec![image.clone()])).unwrap()
        );
        assert_eq!(prompt["images"][0]["type"], "image");
        assert_eq!(prompt["images"][0]["mimeType"], "image/png");
        assert_eq!(
            crate::attachments::decode_base64(prompt["images"][0]["data"].as_str().unwrap())
                .unwrap(),
            crate::attachments::read(&image).unwrap()
        );
        assert_eq!(thread.messages.len(), 1);
        assert_eq!(thread.messages[0].text, "");
        assert_eq!(thread.messages[0].images, vec![image]);
        assert_eq!(thread.title, "Image: screen.png");
    }

    #[test]
    fn non_vision_models_refuse_images_without_sending_or_clearing_the_draft() {
        let mut thread = Thread::new(1);
        thread.model = "Text Only".into();
        thread.model_provider = "cpa".into();
        thread.supports_images = Some(false);
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        let image = test_attachment("screen.png", 1);
        thread.add_attachment(image.clone()).unwrap();
        thread.draft.replace_text("Keep this text");
        thread.send();
        assert!(commands.try_recv().is_err(), "no prompt may be sent");
        assert!(thread.messages.is_empty());
        assert!(thread.queued_prompt.is_none());
        assert_eq!(thread.status, Status::Ready);
        assert_eq!(thread.draft.text, "Keep this text");
        assert_eq!(thread.draft_images, vec![image]);
        assert!(
            thread
                .attachment_error
                .as_ref()
                .unwrap()
                .contains("Text Only")
        );
    }

    #[test]
    fn an_undeclared_vision_capability_blocks_before_pi_receives_the_prompt() {
        let mut thread = Thread::new(1);
        thread.model = "Unknown Model".into();
        thread.model_provider = "cpa".into();
        thread.supports_images = None;
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        let image = test_attachment("screen.png", 1);
        thread.add_attachment(image.clone()).unwrap();
        thread.send();
        assert!(commands.try_recv().is_err());
        assert!(thread.messages.is_empty());
        assert_eq!(thread.status, Status::Ready);
        assert_eq!(thread.draft_images, vec![image.clone()]);
        assert!(
            thread
                .attachment_error
                .as_ref()
                .unwrap()
                .contains("does not declare")
        );
        assert_eq!(
            thread.retry_prompt.as_ref().unwrap().images.as_ref(),
            &[image]
        );
    }

    #[test]
    fn pi_model_metadata_drives_the_image_capability_checked_before_sending() {
        let mut thread = Thread::new(1);
        thread.apply(
            json!({"type":"response","command":"get_state","success":true,"data":{
                "model":{"provider":"cpa","id":"text-only","name":"Text Only","input":["text"]}
            }}),
        );
        assert_eq!(thread.supports_images, Some(false));
        thread.apply(
            json!({"type":"response","command":"get_state","success":true,"data":{
                "model":{"provider":"cpa","id":"vision","name":"Vision","input":["text","image"]}
            }}),
        );
        assert_eq!(thread.supports_images, Some(true));
    }

    #[test]
    fn stopping_a_queued_image_send_restores_the_submitted_snapshot_and_new_edits() {
        let mut thread = Thread::new(1);
        thread.model_provider = "cpa".into();
        thread.supports_images = Some(true);
        let original = test_attachment("original.png", 1);
        let later = test_attachment("later.png", 2);
        thread.draft.replace_text("First");
        thread.add_attachment(original.clone()).unwrap();
        thread.send();
        assert_eq!(
            thread.queued_prompt.as_ref().unwrap().0.images.as_ref(),
            std::slice::from_ref(&original)
        );
        assert_eq!(thread.messages[0].images, vec![original.clone()]);
        thread.draft.replace_text("Edited during startup");
        thread.add_attachment(later.clone()).unwrap();
        thread.stop();
        assert_eq!(thread.status, Status::Ready);
        assert!(thread.messages.is_empty());
        assert!(thread.queued_prompt.is_none());
        assert_eq!(thread.draft.text, "First\n\nEdited during startup");
        assert_eq!(thread.draft_images, vec![original.clone(), later]);
        assert_eq!(
            thread.retry_prompt.as_ref().unwrap().images.as_ref(),
            &[original]
        );
    }

    #[test]
    fn a_rejected_image_prompt_returns_its_images_to_the_draft() {
        let mut thread = Thread::new(1);
        let image = test_attachment("screen.png", 1);
        let prompt = Prompt::new("Try this".into(), vec![image.clone()]);
        thread.messages.push(Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: "You",
            text: prompt.text.clone(),
            images: prompt.images.to_vec(),
            context: None,
        });
        thread.pending_prompt = Some(("1-1".into(), prompt, 0));
        thread.status = Status::Running;
        thread.apply(json!({"type":"response","id":"1-1","command":"prompt","success":false,"error":"No model configured"}));
        assert_eq!(thread.status, Status::Failed);
        assert_eq!(thread.draft.text, "Try this");
        assert_eq!(thread.draft_images, vec![image]);
        assert!(thread.messages.is_empty());
        assert!(thread.queued_prompt.is_none());
    }

    #[test]
    fn history_user_images_are_imported_into_the_local_store_without_base64_state() {
        let png = test_png(1);
        let thread = &mut Thread::new(1);
        thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":[
                {"type":"image","data":crate::attachments::encode_base64(&png),"mimeType":"image/png"},
                {"type":"text","text":"What is this?"}
            ]},
            {"role":"assistant","content":[{"type":"text","text":"A test image."}]}
        ]}}));
        assert_eq!(thread.messages[0].text, "What is this?");
        assert_eq!(thread.messages[0].images.len(), 1);
        assert_eq!(
            crate::attachments::read(&thread.messages[0].images[0]).unwrap(),
            png
        );
        assert_eq!(thread.messages[1].text, "A test image.");
        assert!(thread.messages[1].images.is_empty());
    }

    #[test]
    fn draft_images_persist_as_references_and_restore_without_inline_base64() {
        let mut state = State::new(".".into(), false);
        let image = test_attachment("draft.png", 1);
        state.current_mut().draft.replace_text("Keep this draft");
        state.current_mut().add_attachment(image.clone()).unwrap();
        let snapshot = state.ui_snapshot();
        assert!(snapshot["drafts"]["1"]["images"][0].get("data").is_none());
        assert!(
            !snapshot
                .to_string()
                .contains(&crate::attachments::encode_base64(&test_png(1)))
        );
        let mut restored = State::new(".".into(), false);
        restored.restore_ui(&snapshot).unwrap();
        assert_eq!(restored.current().draft.text, "Keep this draft");
        assert_eq!(restored.current().draft_images, vec![image]);
    }

    #[test]
    fn ordinary_file_drafts_round_trip_and_keep_missing_file_errors() {
        let path = crate::store::directory()
            .unwrap()
            .join("draft-reference.txt");
        std::fs::write(&path, "Draft text stays outside ui-state.").unwrap();
        let file = attachments::attach_file(&path).unwrap();
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("Review this");
        state.current_mut().add_attachment(file.clone()).unwrap();
        let snapshot = state.ui_snapshot();
        assert!(!snapshot.to_string().contains("Draft text stays outside"));
        let mut restored = State::new(".".into(), false);
        restored.restore_ui(&snapshot).unwrap();
        assert_eq!(restored.current().draft.text, "Review this");
        assert_eq!(restored.current().draft_images, vec![file.clone()]);
        std::fs::remove_file(path).unwrap();
        restored.restore_ui(&snapshot).unwrap();
        assert_eq!(restored.current().draft_images, vec![file]);
        assert!(
            restored
                .current()
                .attachment_error
                .as_deref()
                .unwrap()
                .contains("Missing file")
        );
        let again = restored.ui_snapshot();
        restored.restore_ui(&again).unwrap();
        assert!(
            restored
                .current()
                .attachment_error
                .as_deref()
                .unwrap()
                .contains("Missing file")
        );
    }

    #[test]
    fn attachment_count_and_size_limits_keep_the_text_draft_untouched() {
        let mut thread = Thread::new(1);
        thread.draft.replace_text("Keep me");
        for width in 0..8 {
            thread
                .add_attachment(test_attachment("image.png", width + 1))
                .unwrap();
        }
        let error = thread
            .add_attachment(test_attachment("extra.png", 20))
            .unwrap_err();
        assert!(error.contains("up to 8"));
        assert_eq!(thread.draft.text, "Keep me");
        thread.draft_images.clear();
        let mut oversized = test_attachment("ok.png", 20);
        oversized.bytes = crate::attachments::MAX_TOTAL_BYTES + 1;
        assert!(thread.add_attachment(oversized).is_err());
        assert_eq!(thread.draft.text, "Keep me");
    }

    #[test]
    fn restoring_submitted_images_preserves_a_newer_full_draft() {
        let mut thread = Thread::new(1);
        let submitted = test_attachment("submitted.png", 30);
        let prompt = Prompt::new("Submitted".into(), vec![submitted.clone()]);
        let newer: Vec<_> = (1..=attachments::MAX_IMAGES as u32)
            .map(|width| test_attachment("newer.png", width))
            .collect();
        thread.draft_images = newer.clone();
        thread.draft.replace_text("Newer draft");
        thread.restore_prompt(&prompt);
        assert_eq!(thread.draft_images.len(), attachments::MAX_IMAGES + 1);
        assert_eq!(thread.draft_images[0], submitted);
        assert_eq!(&thread.draft_images[1..], newer.as_slice());
        assert_eq!(thread.draft.text, "Submitted\n\nNewer draft");
        assert!(
            thread
                .attachment_error
                .as_deref()
                .unwrap()
                .contains("Remove images")
        );
        assert!(attachments::encode_images(&thread.draft_images).is_err());
        thread.remove_attachment(&submitted.id);
        assert!(thread.attachment_error.is_none());
        assert_eq!(thread.draft_images, newer);
    }

    #[test]
    fn an_image_picker_result_stays_with_its_original_thread() {
        let mut state = State::new(".".into(), false);
        let origin = state.current().id;
        let image = test_attachment("picked.png", 1);
        state.current_mut().draft.replace_text("Write the report");
        state.new_thread();
        assert_ne!(state.current().id, origin);
        state.attach_paths_to(origin, &[attachments::path_for(&image).unwrap()]);
        assert!(state.current().draft_images.is_empty());
        assert_eq!(state.threads[0].draft_images.len(), 1);
        assert_eq!(state.threads[0].draft_images[0].id, image.id);
    }

    #[test]
    fn inline_file_and_contextual_image_chips_insert_at_caret_and_retain_records() {
        let dir = crate::store::directory().unwrap();
        let file_path = dir.join("spec.txt");
        std::fs::write(&file_path, "spec body").unwrap();
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("Read and reply");
        state.current_mut().draft.set_caret(4);
        state.attach_paths(&[file_path]);
        assert_eq!(state.current().draft_images.len(), 1);
        let file_id = state.current().draft_images[0].id.clone();
        let expected_file_ctx =
            crate::composer_context::to_kind_scoped_composer_context_id("file", &file_id);
        assert_eq!(state.current().draft.pills().count(), 1);
        assert_eq!(
            crate::composer_rich::markdown(&state.current().draft),
            format!("Read [spec.txt](t3-context://v1/file/{expected_file_ctx}) and reply")
        );
        assert!(
            state
                .current()
                .draft_context
                .iter()
                .any(|r| r.context_id == expected_file_ctx)
        );

        // Contextual image (when draft has prose text) inserts inline chip AND stays in tray.
        let end = state.current().draft.text.len();
        state.current_mut().draft.set_caret(end);
        state.attach_image("shot.png", &test_png(1));
        assert_eq!(state.current().draft_images.len(), 2);
        let img_id = state.current().draft_images[1].id.clone();
        let expected_img_ctx =
            crate::composer_context::to_kind_scoped_composer_context_id("image", &img_id);
        assert_eq!(state.current().draft.pills().count(), 2);
        assert!(
            crate::composer_rich::markdown(&state.current().draft).contains(&format!(
                "![shot.png](t3-context://v1/image/{expected_img_ctx})"
            ))
        );

        // Undo removes the inline image chip (image stays in tray); deleting the file chip
        // moves the file attachment into retained_context.
        state.current_mut().draft.undo();
        state.current_mut().reconcile_draft_context();
        assert_eq!(state.current().draft_images.len(), 2);
        assert_eq!(state.current().draft.pills().count(), 1);

        state.current_mut().draft.undo();
        state.current_mut().reconcile_draft_context();
        assert_eq!(state.current().draft.pills().count(), 0);
        assert_eq!(state.current().draft_images.len(), 1); // only image remains!
        assert!(
            state
                .current()
                .retained_context
                .files
                .contains_key(&expected_file_ctx)
        );

        // Redo brings both the file chip and backing file attachment back.
        state.current_mut().draft.redo();
        state.current_mut().reconcile_draft_context();
        assert_eq!(state.current().draft.pills().count(), 1);
        assert_eq!(state.current().draft_images.len(), 2);
    }

    #[test]
    fn discard_undo_restores_images_and_missing_saved_images_are_reported() {
        let mut state = State::new(".".into(), false);
        let image = test_attachment("undo.png", 1);
        state
            .current_mut()
            .draft
            .replace_text("Draft with an image");
        state.current_mut().add_attachment(image.clone()).unwrap();
        state.discard_draft(state.current().id);
        assert!(state.current().draft_images.is_empty());
        state.undo_thread_action();
        state.select_thread(0);
        assert_eq!(state.current().draft.text, "Draft with an image");
        assert_eq!(state.current().draft_images, vec![image.clone()]);
        let snapshot = state.ui_snapshot();
        std::fs::remove_file(attachments::path_for(&image).unwrap()).unwrap();
        let mut restored = State::new(".".into(), false);
        restored.restore_ui(&snapshot).unwrap();
        assert_eq!(restored.current().draft.text, "Draft with an image");
        assert!(restored.current().draft_images.is_empty());
        assert!(
            restored
                .current()
                .attachment_error
                .as_deref()
                .unwrap()
                .contains("missing")
        );
    }
    #[test]
    fn retry_after_a_crash_resends_the_snapshotted_image_message() {
        let mut state = State::new(".".into(), false);
        state.current_mut().model_provider = "cpa".into();
        state.current_mut().supports_images = Some(true);
        state.current_mut().session = Some("saved.jsonl".into());
        let image = test_attachment("retry.png", 1);
        state.current_mut().add_attachment(image.clone()).unwrap();
        let (pi, _commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.send();
        events.send(Event::Exited).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.current().status, Status::Failed);
        assert!(state.current().messages.is_empty());
        assert_eq!(state.current().draft_images, vec![image.clone()]);
        let (pi, commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().initializing = true;
        state.retry();
        for record in [
            json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
                "model":{"provider":"cpa","id":"vision","input":["text","image"]},"sessionFile":"saved.jsonl"
            }}),
            json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}),
        ] {
            events.send(Event::Record(record)).unwrap();
        }
        poll(&mut state, 0.0, &mut Tx::default());
        let prompt = commands.try_iter().find(|c| c["type"] == "prompt").unwrap();
        assert_eq!(
            prompt["message"],
            attachments::pi_message(&Prompt::new(String::new(), vec![image.clone()])).unwrap()
        );
        assert_eq!(prompt["images"][0]["type"], "image");
        assert_eq!(
            crate::attachments::decode_base64(prompt["images"][0]["data"].as_str().unwrap())
                .unwrap(),
            crate::attachments::read(&image).unwrap()
        );
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn drafts_stay_listed_with_text_pills_or_warmed_session_and_sent_threads_do_not() {
        let mut state = State::new(".".into(), false);
        assert!(state.sidebar_threads().is_empty(), "blank thread");

        // A warmed session on an unsent thread does not end its draft state.
        state.current_mut().draft.replace_text("First draft");
        state.current_mut().session = Some("warmed-session.jsonl".into());
        assert!(state.current().is_draft());

        // Once it is no longer the open thread it has a Drafts row, and keeps it when reopened.
        state.new_thread();
        assert!(state.current().blank_draft());
        assert_eq!(state.sidebar_threads(), vec![0]);
        state.select_thread(0);
        assert_eq!(
            state.sidebar_threads(),
            vec![0],
            "the reopened draft keeps its row"
        );
        assert!(state.draft_listed(&state.threads[0]));

        // A pill alone is content.
        state.select_thread(1);
        let token = crate::composer_tokens::Token::File {
            path: PathBuf::from("src/main.rs"),
            directory: false,
            source: None,
        };
        state.current_mut().draft.insert_pill(token.id(), "main.rs");
        assert!(state.current().has_draft_content());
        state.select_thread(0);
        let mut listed = state.sidebar_threads();
        listed.sort();
        assert_eq!(listed, vec![0, 1]);

        // A restart keeps the warmed draft a draft. A record from before the marker existed
        // keeps the old rule: its session means it has been used, even with no messages loaded.
        let index = state.snapshot();
        let record = index["threads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["id"] == state.threads[0].id)
            .unwrap();
        assert_eq!(record["neverSent"], true);
        let mut old = Thread::new(9);
        old.never_sent = false;
        old.session = Some("old-session.jsonl".into());
        assert!(!old.is_draft() && !old.blank_draft());
        assert!(old.restoring_history() || !old.initializing);

        // Sending ends the draft.
        state.current_mut().model_provider = "cpa".into();
        state.current_mut().send();
        assert!(!state.threads[0].is_draft());
        assert_eq!(state.snapshot()["threads"][0]["neverSent"], false);
    }

    #[test]
    fn restoring_history_only_triggers_for_threads_with_a_saved_session() {
        let mut thread = Thread::new(1);
        thread.initializing = true;
        thread.history_loaded = false;
        assert!(
            !thread.restoring_history(),
            "new thread without a session has no conversation to restore"
        );

        thread.session = Some("warmed.jsonl".into());
        assert!(
            !thread.restoring_history(),
            "a warmed session on an unsent thread has nothing to restore"
        );
        thread.never_sent = false;
        assert!(thread.restoring_history());

        thread.history_loaded = true;
        assert!(!thread.restoring_history());
    }

    #[test]
    fn send_background_starts_draft_in_background_inherits_composer_and_notifies() {
        let mut state = State::new(".".into(), false);
        let dir_b = std::env::temp_dir().join(format!("shina-bg-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir_b);
        let p2 = state.add_project(dir_b.clone()).unwrap().unwrap();
        let model = AgentModel {
            provider: "codex".into(),
            id: "codex-fast".into(),
            name: "Codex Fast".into(),
            thinking_levels: vec!["low".into(), "high".into()],
            context_window: Some(100_000),
            supports_images: Some(true),
            service_tiers: vec![
                json!({"id":"default","name":"Standard"}),
                json!({"id":"fast","name":"Fast"}),
            ],
            ..Default::default()
        };
        state.codex_catalog.models = vec![model.clone()];
        let (pi, _cmds, _events) = Pi::test_connection();
        {
            let t = state.current_mut();
            t.harness = "codex".into();
            t.models = vec![model];
            t.set_model("codex".into(), "codex-fast".into());
            t.set_thinking("high".into());
            t.set_service_tier("fast".into());
            state.set_access_mode(AccessMode::ReadOnly);
            let t = state.current_mut();
            t.pi = Some(pi);
            t.draft.replace_text("Background task");
            t.add_attachment(test_attachment("shot.png", 1)).unwrap();
        }
        let bg_id = state.current().id;

        state.send_background();

        // Fresh draft composer is active, focused, empty, and inherits selections.
        assert_ne!(state.current().id, bg_id);
        assert!(state.current().is_draft());
        assert!(state.current().draft.focused);
        assert!(state.current().draft.text.is_empty());
        assert!(state.current().draft_images.is_empty());
        assert!(state.current().messages.is_empty());
        assert!(state.current().error.is_none());
        assert_eq!(state.current().project, Some(p2));
        assert_eq!(state.current().machine, "local");
        assert_eq!(state.current().harness, "codex");
        assert_eq!(
            state.current().selected_model,
            Some(("codex".into(), "codex-fast".into()))
        );
        assert_eq!(state.current().selected_thinking.as_deref(), Some("high"));
        assert_eq!(state.current().harness_state["serviceTier"], "fast");
        assert_eq!(state.current().native.access, AccessMode::ReadOnly);
        assert!(state.current().captain.is_none());

        // Background thread is running and visible in sidebar Active section.
        let bg = state.threads.iter().find(|t| t.id == bg_id).unwrap();
        assert_eq!(bg.status, Status::Running);
        assert_eq!(bg.messages.len(), 1);
        assert_eq!(bg.messages[0].text, "Background task");
        assert!(!bg.is_draft());
        assert!(
            state
                .sidebar_threads()
                .iter()
                .any(|&i| state.threads[i].id == bg_id)
        );

        // 5-second "Started in background" toast with "Open" action.
        assert_eq!(state.toasts.len(), 1);
        let toast = &state.toasts[0];
        assert_eq!(toast.kind, ToastKind::Success);
        assert_eq!(toast.title, "Started in background");
        assert_eq!(toast.action_label.as_deref(), Some("Open"));
        assert!(toast.expires_at.is_some());

        // Completing in background marks the thread unread; clicking Open navigates to it and clears unread.
        let bg_idx = state.threads.iter().position(|t| t.id == bg_id).unwrap();
        state.threads[bg_idx].apply(json!({"type":"agent_settled"}));
        let completed_at = state.threads[bg_idx].life.last_completed_at.unwrap();
        state.threads[bg_idx].life.last_visited_at = Some(completed_at.saturating_sub(100));
        assert!(state.threads[bg_idx].unread());

        let toast_id = toast.id;
        state.activate_toast(toast_id);
        assert_eq!(state.current().id, bg_id);
        assert!(state.toasts.is_empty());
        assert!(!state.current().unread());
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[test]
    fn send_and_new_thread_works_on_idle_and_running_threads_and_respects_guards() {
        let mut state = State::new(".".into(), false);
        let (pi, commands, _events) = Pi::test_connection();
        let original_id = state.current().id;
        {
            let t = state.current_mut();
            t.pi = Some(pi);
            t.model_provider = "cpa".into();
            t.messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "You",
                text: "Prior message".into(),
                images: vec![],
                context: None,
            });
            t.draft.replace_text("First message");
        }
        state.send_background();
        let fresh_id = state.current().id;
        assert_ne!(fresh_id, original_id);
        // Existing idle thread -> sendAndNewThread opens a fresh draft without "Started in background" toast.
        assert!(state.toasts.is_empty());

        // Switch back to original_id (which is now running!) and test sendAndNewThread while running.
        let orig_idx = state
            .threads
            .iter()
            .position(|t| t.id == original_id)
            .unwrap();
        state.select_thread(orig_idx);
        state.current_mut().native.follow_up = true;
        state.current_mut().draft.replace_text("Queued follow up");
        state.send_background();
        let after_running_id = state.current().id;
        assert_ne!(after_running_id, original_id);
        assert!(state.current().is_draft());
        assert!(state.current().draft.focused);
        assert!(state.toasts.is_empty());
        let orig = state.threads.iter().find(|t| t.id == original_id).unwrap();
        assert_eq!(orig.native.queue.len(), 1);
        assert_eq!(orig.native.queue[0].prompt.text, "Queued follow up");
        let _ = commands;
    }

    #[test]
    fn send_background_guards_block_empty_validation_pending_archived_and_queued_edit() {
        // 1. Empty prompt
        let mut state = State::new(".".into(), false);
        let id = state.current().id;
        state.send_background();
        assert_eq!(state.threads.len(), 1);
        assert_eq!(state.current().id, id);
        assert!(state.toasts.is_empty());

        // 2. Prompt length validation error (> 120,000 chars)
        state
            .current_mut()
            .draft
            .replace_text(&"x".repeat(crate::composer::PROVIDER_SEND_TURN_MAX_INPUT_CHARS + 1));
        state.send_background();
        assert_eq!(state.threads.len(), 1);
        assert!(state.current().submission_error.is_some());
        assert!(state.toasts.is_empty());
        state.current_mut().submission_error = None;

        // 3. Pending request / question blocks send_background
        state.current_mut().draft.replace_text("Blocked by request");
        state.current_mut().request = Some(json!({"id":"req-1","method":"confirm"}));
        state.send_background();
        assert_eq!(state.threads.len(), 1);
        assert_eq!(state.current().draft.text, "Blocked by request");
        state.current_mut().request = None;

        // 4. Archived thread blocks send_background
        state.current_mut().life.archived_at = Some(1);
        state.send_background();
        assert_eq!(state.threads.len(), 1);
        assert_eq!(state.current().draft.text, "Blocked by request");
        state.current_mut().life.archived_at = None;

        // 5. Queued edit mode saves in place without opening a new thread
        let (pi, _cmds, _evs) = Pi::test_connection();
        let t = state.current_mut();
        t.pi = Some(pi);
        t.model_provider = "cpa".into();
        t.status = Status::Running;
        t.native.follow_up = true;
        t.draft.replace_text("Queued original");
        t.send_during_run();
        let qid = t.native.queue[0].id;
        t.edit_queued(qid);
        t.draft.replace_text("Queued edited");
        state.send_background();
        assert_eq!(state.threads.len(), 1);
        assert_eq!(state.current().native.queue[0].prompt.text, "Queued edited");
        assert!(state.current().queued_edit.is_none());
    }

    #[test]
    fn failed_background_send_recovers_draft_sets_error_and_offers_open_draft_toast() {
        let mut state = State::new(".".into(), false);
        state.runtime_enabled = true;
        let bg_id = state.current().id;
        let img = test_attachment("fail.png", 1);
        state.current_mut().model_provider = "cpa".into();
        state.current_mut().supports_images = Some(true);
        state
            .current_mut()
            .draft
            .replace_text("Will fail on connect");
        state.current_mut().add_attachment(img.clone()).unwrap();
        // Force synchronous connect_thread failure via missing directory on unknown machine.
        state.current_mut().machine = "missing-host".into();

        state.send_background();

        // User landed on a fresh composer.
        assert_ne!(state.current().id, bg_id);
        assert!(state.current().is_draft());
        assert!(state.current().draft.focused);

        // Background task failed toast with "Open draft" action is shown.
        assert_eq!(state.toasts.len(), 1);
        let toast = &state.toasts[0];
        assert_eq!(toast.kind, ToastKind::Error);
        assert_eq!(toast.title, "Background task failed");
        assert!(toast.description.is_some());
        assert_eq!(toast.action_label.as_deref(), Some("Open draft"));

        // Clicking "Open draft" opens the failed draft with restored text, attachments, and error.
        let toast_id = toast.id;
        state.activate_toast(toast_id);
        assert_eq!(state.current().id, bg_id);
        assert_eq!(state.current().draft.text, "Will fail on connect");
        assert_eq!(state.current().draft_images, vec![img]);
        assert!(state.current().error.is_some());

        // Async failure during startup in poll() replaces "Started in background" with "Background task failed".
        let mut async_state = State::new(".".into(), false);
        let (pi, _cmds, events) = Pi::test_connection();
        let async_bg_id = async_state.current().id;
        async_state.current_mut().pi = Some(pi);
        async_state.current_mut().initializing = true;
        async_state.current_mut().model_provider = "cpa".into();
        async_state
            .current_mut()
            .draft
            .replace_text("Async failure");
        async_state.send_background();
        assert_eq!(async_state.toasts[0].title, "Started in background");

        events.send(Event::Error("Process crashed".into())).unwrap();
        poll(&mut async_state, 0.0, &mut Tx::default());

        assert_eq!(async_state.toasts.len(), 1);
        assert_eq!(async_state.toasts[0].kind, ToastKind::Error);
        assert_eq!(async_state.toasts[0].title, "Background task failed");
        assert_eq!(
            async_state.toasts[0].description.as_deref(),
            Some("Process crashed")
        );
        assert_eq!(
            async_state.toasts[0].action_label.as_deref(),
            Some("Open draft")
        );
        let bg_after = async_state
            .threads
            .iter()
            .find(|t| t.id == async_bg_id)
            .unwrap();
        assert_eq!(bg_after.draft.text, "Async failure");
        assert_eq!(bg_after.error.as_deref(), Some("Process crashed"));

        // Dismiss and 5s timeout expiration remove toasts cleanly.
        async_state.toasts[0].expires_at = Some(1);
        poll(&mut async_state, 0.0, &mut Tx::default());
        assert!(async_state.toasts.is_empty());
        async_state.notify_background_started("local", async_bg_id);
        assert_eq!(async_state.toasts.len(), 1);
        let dismiss_id = async_state.toasts[0].id;
        async_state.dismiss_toast(dismiss_id);
        assert!(async_state.toasts.is_empty());
    }

    #[test]
    fn context_breakdown_metadata_round_trips_and_decodes_legacy_records() {
        let mut state = State::new(".".into(), false);
        let rows = vec![
            crate::context_breakdown::ContextBreakdownRow {
                label: "Messages".into(),
                tokens: 12_000,
                kind: crate::context_breakdown::BreakdownRowKind::Used,
            },
            crate::context_breakdown::ContextBreakdownRow {
                label: "Autocompact buffer".into(),
                tokens: 30_000,
                kind: crate::context_breakdown::BreakdownRowKind::Buffer,
            },
        ];
        state.current_mut().context.usage.breakdown = Some(rows.clone());
        let with_breakdown = state.snapshot();

        let mut restored = State::new(".".into(), false);
        restored.restore_global(&with_breakdown).unwrap();
        assert_eq!(restored.current().context.usage.breakdown, Some(rows));

        // Removing contextBreakdown simulates an older peer / legacy index.
        let mut legacy = with_breakdown;
        legacy["threads"][0]
            .as_object_mut()
            .unwrap()
            .remove("contextBreakdown");
        let mut restored_legacy = State::new(".".into(), false);
        restored_legacy.restore_global(&legacy).unwrap();
        assert_eq!(restored_legacy.current().context.usage.breakdown, None);

        // Without breakdown on the thread, round-trip stays None.
        state.current_mut().context.usage.breakdown = None;
        let without_breakdown = state.snapshot();
        let mut restored_none = State::new(".".into(), false);
        restored_none.restore_global(&without_breakdown).unwrap();
        assert_eq!(restored_none.current().context.usage.breakdown, None);

        // Wire compatibility for ContextUsage projection without breakdown.
        let old_wire = json!({"tokens": 1000, "context_window": 200000, "percent": 0.5});
        let decoded: crate::context::ContextUsage = serde_json::from_value(old_wire).unwrap();
        assert_eq!(decoded.breakdown, None);
    }

    #[test]
    fn worktree_thread_metadata_round_trips_and_decodes_legacy_records() {
        let mut state = State::new(".".into(), false);
        let thread = state.current_mut();
        thread.worktree_path = Some("/tmp/worktrees/repo/feat-abc".into());
        thread.branch = Some("feature/worktree".into());
        thread.base_branch = Some("main".into());
        thread.env_mode = Some(ThreadEnvMode::Worktree);

        let with_worktree = state.snapshot();
        assert_eq!(
            with_worktree["threads"][0]["worktreePath"],
            "/tmp/worktrees/repo/feat-abc"
        );
        assert_eq!(with_worktree["threads"][0]["branch"], "feature/worktree");
        assert_eq!(with_worktree["threads"][0]["baseBranch"], "main");
        assert_eq!(with_worktree["threads"][0]["envMode"], "worktree");

        let mut restored = State::new(".".into(), false);
        restored.restore_global(&with_worktree).unwrap();
        assert_eq!(
            restored.current().worktree_path.as_deref(),
            Some("/tmp/worktrees/repo/feat-abc")
        );
        assert_eq!(
            restored.current().branch.as_deref(),
            Some("feature/worktree")
        );
        assert_eq!(restored.current().base_branch.as_deref(), Some("main"));
        assert_eq!(restored.current().env_mode, Some(ThreadEnvMode::Worktree));

        // Thread projection also carries the worktree fields on the wire.
        let projection = state.projection();
        assert_eq!(
            projection["threads"][0]["worktreePath"],
            "/tmp/worktrees/repo/feat-abc"
        );
        assert_eq!(projection["threads"][0]["branch"], "feature/worktree");
        assert_eq!(projection["threads"][0]["baseBranch"], "main");
        assert_eq!(projection["threads"][0]["envMode"], "worktree");

        // Legacy metadata without worktree fields decodes to None.
        let mut legacy = with_worktree;
        let obj = legacy["threads"][0].as_object_mut().unwrap();
        for key in ["worktreePath", "branch", "baseBranch", "envMode"] {
            obj.remove(key);
        }
        let mut restored_legacy = State::new(".".into(), false);
        restored_legacy.restore_global(&legacy).unwrap();
        assert_eq!(restored_legacy.current().worktree_path, None);
        assert_eq!(restored_legacy.current().branch, None);
        assert_eq!(restored_legacy.current().base_branch, None);
        assert_eq!(restored_legacy.current().env_mode, None);

        // Absent fields stay omitted and round-trip as None.
        let empty = State::new(".".into(), false);
        let without_worktree = empty.snapshot();
        for key in ["worktreePath", "branch", "baseBranch", "envMode"] {
            assert!(without_worktree["threads"][0].get(key).is_none(), "{key}");
        }
        let mut restored_none = State::new(".".into(), false);
        restored_none.restore_global(&without_worktree).unwrap();
        assert_eq!(restored_none.current().worktree_path, None);
        assert_eq!(restored_none.current().branch, None);
        assert_eq!(restored_none.current().base_branch, None);
        assert_eq!(restored_none.current().env_mode, None);

        // Unknown envMode value decodes leniently as None rather than failing restore.
        let mut unknown_env = empty.snapshot();
        unknown_env["threads"][0]["envMode"] = json!("future-mode");
        let mut restored_unknown = State::new(".".into(), false);
        restored_unknown.restore_global(&unknown_env).unwrap();
        assert_eq!(restored_unknown.current().env_mode, None);

        // Local envMode round-trips too.
        let mut local_state = State::new(".".into(), false);
        local_state.current_mut().env_mode = Some(ThreadEnvMode::Local);
        let local_snap = local_state.snapshot();
        assert_eq!(local_snap["threads"][0]["envMode"], "local");
        let mut restored_local = State::new(".".into(), false);
        restored_local.restore_global(&local_snap).unwrap();
        assert_eq!(
            restored_local.current().env_mode,
            Some(ThreadEnvMode::Local)
        );
    }

    #[test]
    fn clear_command_arms_when_alone_sends_text_with_attachment_and_unavailable_with_approval() {
        let mut state = State::new(".".into(), false);
        // Empty thread: /clear is unavailable.
        state.current_mut().draft.replace_text("/");
        state.current_mut().draft.focused = true;
        assert!(!state.slash_matches().iter().any(|c| c.name == "clear"));
        state.current_mut().draft.replace_text("/clear");
        state.send();
        assert!(state.current().fresh_session_arm.is_none());
        assert_eq!(state.current().messages.len(), 1);
        assert_eq!(state.current().messages[0].text, "/clear");

        // Thread with history: /clear alone arms, clears draft, and sends nothing.
        state.current_mut().status = Status::Completed;
        state.current_mut().draft.replace_text("/");
        assert!(state.slash_matches().iter().any(|c| c.name == "clear"));
        state.current_mut().draft.replace_text("/clear");
        state.send();
        assert!(state.current().fresh_session_arm.is_some());
        assert!(state.current().draft.text.is_empty());
        assert_eq!(state.current().messages.len(), 1);
        assert!(
            state
                .composer_notices
                .ordered()
                .iter()
                .any(|n| n.id == "fresh-session")
        );

        // With an attachment present, /clear is sent as ordinary text and does not arm.
        state.disarm_fresh_session(state.current().id, None);
        state.current_mut().draft.replace_text("/clear");
        state
            .current_mut()
            .add_attachment(test_attachment("note.png", 1))
            .unwrap();
        state.send();
        assert!(state.current().fresh_session_arm.is_none());
        assert_eq!(state.current().messages.len(), 2);
        assert_eq!(state.current().messages[1].text, "/clear");

        // Unavailable with a pending approval.
        state.current_mut().status = Status::Waiting;
        state.current_mut().request = Some(json!({"id":"perm-1","method":"confirm"}));
        state.current_mut().draft.replace_text("/");
        assert!(!state.slash_matches().iter().any(|c| c.name == "clear"));
        state.current_mut().draft.replace_text("/clear");
        state.send();
        assert!(state.current().fresh_session_arm.is_none());
        assert_eq!(state.current().messages.len(), 2);

        // Unavailable on a linked machine lacking the `fresh-context` capability.
        state.current_mut().status = Status::Completed;
        state.current_mut().request = None;
        let (old_client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        *old_client.info.lock().unwrap() = json!({"capabilities": []});
        state.client = Some(old_client);
        state.current_mut().draft.replace_text("/");
        assert!(!state.slash_matches().iter().any(|c| c.name == "clear"));
    }

    #[test]
    fn fresh_session_arm_is_read_once_keeps_on_failed_send_and_persists_with_draft() {
        let dir = crate::store::directory().unwrap().join("fresh-arm-persist");
        std::fs::create_dir_all(&dir).unwrap();

        let mut state = State::new(dir.clone(), false);
        state.load_global_for_test();
        state.persist_transcripts = true;
        state.current_mut().messages.push(Message {
            id: "m1".into(),
            role: "You",
            text: "Earlier".into(),
            images: vec![],
            context: None,
        });
        state.current_mut().status = Status::Completed;
        state.current_mut().draft.replace_text("/clear");
        state.send();
        let first_arm = state.current().fresh_session_arm.expect("armed");

        // Persist draft and reload in a fresh State against the same temp directory.
        state.current_mut().draft.replace_text("Carry over");
        state.checkpoint_and_flush().unwrap();
        drop(state);

        let mut reloaded = State::new(dir, false);
        reloaded.load_global_for_test();
        assert_eq!(reloaded.current().draft.text, "Carry over");
        assert_eq!(reloaded.current().fresh_session_arm, Some(first_arm));
        assert!(
            reloaded
                .composer_notices
                .ordered()
                .iter()
                .any(|n| n.id == "fresh-session")
        );

        // Failed local send (e.g., Pi with non-CPA model) keeps the arm.
        reloaded.current_mut().harness = "pi".into();
        reloaded.current_mut().model_provider = "anthropic".into();
        reloaded.send();
        assert_eq!(reloaded.current().fresh_session_arm, Some(first_arm));
        assert_eq!(reloaded.current().draft.text, "Carry over");

        // Accepted local send clears the arm, keeps visible transcript, and marks the message.
        reloaded.current_mut().model_provider = "cpa".into();
        reloaded.send();
        assert!(reloaded.current().fresh_session_arm.is_none());
        assert_eq!(reloaded.current().messages.len(), 2);
        assert_eq!(reloaded.current().messages[0].text, "Earlier");
        assert_eq!(reloaded.current().messages[1].text, "Carry over");
        assert!(
            reloaded
                .current()
                .is_fresh_session_message(&reloaded.current().messages[1].id)
        );

        // Read-once semantics for in-flight remote sends: re-arming during flight stays armed.
        let (client, mut rx_cmd) = crate::client::Client::test_connected(reloaded.data_dir.clone());
        *client.info.lock().unwrap() = json!({"capabilities": [crate::protocol::FRESH_CONTEXT]});
        let commands = client.commands.clone();
        let mut remote_state = State::new(reloaded.data_dir.clone(), false);
        remote_state.client = Some(client);
        remote_state.current_mut().remote = Some(commands);
        remote_state.current_mut().messages.push(Message {
            id: "rm1".into(),
            role: "You",
            text: "First".into(),
            images: vec![],
            context: None,
        });
        remote_state.current_mut().draft.replace_text("/clear");
        remote_state.send();
        let inflight_arm = remote_state.current().fresh_session_arm.unwrap();
        remote_state.current_mut().draft.replace_text("Second");
        assert!(remote_state.dispatch_send());
        let sent = rx_cmd.try_recv().unwrap();
        assert_eq!(sent.params["freshContext"], true);
        let cmd_id = sent.command_id;

        // User re-arms while first send is in flight.
        remote_state.current_mut().draft.replace_text("/clear");
        remote_state.send();
        let newer_arm = remote_state.current().fresh_session_arm.unwrap();
        assert_ne!(newer_arm, inflight_arm);

        // First send acceptance must not clear the newer arm.
        assert!(remote_state.current_mut().finish_remote_send(cmd_id, true));
        assert_eq!(remote_state.current().fresh_session_arm, Some(newer_arm));

        // A failed remote send keeps its arm.
        remote_state.current_mut().draft.replace_text("Third");
        assert!(remote_state.dispatch_send());
        let failed_cmd = rx_cmd.try_recv().unwrap().command_id;
        assert!(
            remote_state
                .current_mut()
                .finish_remote_send(failed_cmd, false)
        );
        assert_eq!(remote_state.current().fresh_session_arm, Some(newer_arm));
    }

    #[test]
    fn fresh_turn_with_steer_mode_queues_instead_and_cannot_steer() {
        let mut state = State::new(".".into(), false);
        let (pi, commands, _events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().model_provider = "cpa".into();
        state.current_mut().status = Status::Running;
        state.current_mut().native.follow_up = false; // steer mode
        state.current_mut().harness_state["turnCapabilities"] =
            json!({"supportsInterrupt": true, "supportsActiveSteering": true});
        state.current_mut().messages.push(Message {
            id: "m1".into(),
            role: "You",
            text: "Running turn".into(),
            images: vec![],
            context: None,
        });
        state.current_mut().draft.replace_text("/clear");
        state.send();
        assert!(state.current().fresh_session_arm.is_some());

        state.current_mut().draft.replace_text("Fresh queued turn");
        state.send();
        assert!(state.current().fresh_session_arm.is_none());
        assert_eq!(state.current().native.queue.len(), 1);
        let queued_id = state.current().native.queue[0].id;
        assert!(state.current().native.fresh_queue_ids.contains(&queued_id));
        assert!(
            commands.try_recv().is_err(),
            "fresh turn must queue, never steer"
        );

        // Attempting to steer the queued fresh-context item is rejected.
        state.current_mut().steer_queued(queued_id);
        assert_eq!(state.current().native.queue.len(), 1);
        assert!(state.current().error.is_some());

        // When the active turn settles, the queued item pops and starts a fresh session.
        state.current_mut().error = None;
        state.current_mut().session = Some("old-session.jsonl".into());
        state
            .current_mut()
            .apply(json!({"type":"agent_settled","aborted":false}));
        assert!(state.current().native.queue.is_empty());
        assert!(state.current().session.is_none());
        assert!(state.current().pi.is_none());
        assert_eq!(state.current().messages.len(), 2);
        assert!(
            state
                .current()
                .is_fresh_session_message(&state.current().messages[1].id)
        );
    }

    #[test]
    fn compact_before_send_queues_behind_compaction_respects_keep_and_skips_slash_compact() {
        let stale_at = lifecycle::now().saturating_sub(80 * 60_000);
        let setup_stale_claude = |state: &mut State| {
            let (pi, commands, _events) = Pi::test_connection();
            let t = state.current_mut();
            t.pi = Some(pi);
            t.harness = "claude".into();
            t.model_provider = "claude".into();
            t.session = Some("11111111-1111-4111-8111-111111111111".into());
            t.status = Status::Completed;
            t.messages.push(Message {
                id: "u1".into(),
                role: "You",
                text: "Earlier work".into(),
                images: vec![],
                context: None,
            });
            t.context.absorb(&json!({
                "contextUsage": {
                    "tokens": 220_000,
                    "contextWindow": 1_000_000,
                    "updatedAt": stale_at
                }
            }));
            commands
        };

        // 1. Default "Compact" queues the message behind compaction and sends after compaction ends.
        let mut state = State::new(".".into(), false);
        let commands = setup_stale_claude(&mut state);
        assert_eq!(
            state.resume_compaction_tokens(state.current(), lifecycle::now()),
            Some(220_000)
        );
        state.current_mut().draft.replace_text("Follow up");
        state.send();
        assert_eq!(state.current().native.queue.len(), 1);
        assert_eq!(state.current().native.queue[0].prompt.text, "Follow up");
        let compact_cmd = commands.try_recv().unwrap();
        assert_eq!(compact_cmd["type"], "compact");
        let compact_id = compact_cmd["id"].as_str().unwrap().to_owned();
        assert!(
            commands.try_recv().is_err(),
            "prompt must wait for compaction"
        );

        // Compaction completes -> queued message begins its prompt.
        state.current_mut().apply(json!({"type":"compaction_end"}));
        state.current_mut().apply(json!({
            "type":"response","id":compact_id,"command":"compact","success":true,"data":{}
        }));
        let probe = commands
            .try_iter()
            .find(|c| c["type"] == "get_state")
            .unwrap();
        state.current_mut().apply(json!({
            "type":"response","id":probe["id"],"command":"get_state","success":true,"data":{"isStreaming":false,"isCompacting":false}
        }));
        assert!(state.current().native.queue.is_empty());
        let sent_prompt = commands
            .try_iter()
            .find(|c| c["type"] == "prompt")
            .expect("queued prompt sent after compaction");
        assert_eq!(sent_prompt["message"], "Follow up");

        // 2. If compaction fails, the message stays queued and the failure is shown.
        let mut fail_state = State::new(".".into(), false);
        let commands = setup_stale_claude(&mut fail_state);
        fail_state
            .current_mut()
            .draft
            .replace_text("Wait for compact");
        fail_state.send();
        let compact_cmd = commands.try_recv().unwrap();
        assert_eq!(compact_cmd["type"], "compact");
        let compact_id = compact_cmd["id"].as_str().unwrap().to_owned();
        fail_state.current_mut().apply(json!({
            "type":"compaction_end","errorMessage":"Compaction failed"
        }));
        fail_state.current_mut().apply(json!({
            "type":"response","id":compact_id,"command":"compact","success":false,"error":"Compaction failed"
        }));
        let probe = commands
            .try_iter()
            .find(|c| c["type"] == "get_state")
            .unwrap();
        fail_state.current_mut().apply(json!({
            "type":"response","id":probe["id"],"command":"get_state","success":true,"data":{"isStreaming":false,"isCompacting":false}
        }));
        assert_eq!(fail_state.current().native.queue.len(), 1);
        assert!(fail_state.current().native.queue_held);
        assert_eq!(
            fail_state.current().error.as_deref(),
            Some("Compaction failed")
        );
        assert!(
            !commands.try_iter().any(|c| c["type"] == "prompt"),
            "failed compaction must not send queued prompt"
        );

        // 3. "Keep" sends directly without compacting and resets after sending.
        let mut keep_state = State::new(".".into(), false);
        let commands = setup_stale_claude(&mut keep_state);
        keep_state.toggle_keep_full_history();
        assert!(keep_state.keep_full_history(keep_state.current()));
        keep_state.current_mut().draft.replace_text("Direct send");
        keep_state.send();
        assert!(keep_state.current().native.queue.is_empty());
        assert!(!keep_state.keep_full_history(keep_state.current()));
        let sent_cmds: Vec<_> = commands.try_iter().collect();
        assert!(!sent_cmds.iter().any(|c| c["type"] == "compact"));
        let sent = sent_cmds
            .iter()
            .find(|c| c["type"] == "prompt")
            .expect("direct prompt");
        assert_eq!(sent["message"], "Direct send");

        // 4. Typing `/compact` does not double-compact.
        let mut cmd_state = State::new(".".into(), false);
        let commands = setup_stale_claude(&mut cmd_state);
        cmd_state
            .current_mut()
            .draft
            .replace_text("/compact focus on tests");
        cmd_state.send();
        assert!(cmd_state.current().native.queue.is_empty());
        let compact = commands.try_recv().unwrap();
        assert_eq!(compact["type"], "compact");
        assert_eq!(compact["customInstructions"], "focus on tests");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn prompt_suggestion_metadata_round_trip_and_legacy_decode() {
        let mut state = State::new(".".into(), false);
        let suggestion = PromptSuggestion {
            run_id: "run-42".into(),
            text: "Run the test suite".into(),
        };
        state.current_mut().run_id = Some("run-42".into());
        state.current_mut().prompt_suggestion = Some(suggestion.clone());
        let with_suggestion = state.snapshot();

        let mut restored = State::new(".".into(), false);
        restored.restore_global(&with_suggestion).unwrap();
        assert_eq!(
            restored.current().prompt_suggestion,
            Some(suggestion.clone())
        );

        // Removing promptSuggestion simulates an older peer / legacy index.
        let mut legacy = with_suggestion;
        legacy["threads"][0]
            .as_object_mut()
            .unwrap()
            .remove("promptSuggestion");
        let mut restored_legacy = State::new(".".into(), false);
        restored_legacy.restore_global(&legacy).unwrap();
        assert_eq!(restored_legacy.current().prompt_suggestion, None);

        // Without suggestion on the thread, round-trip stays None.
        state.current_mut().prompt_suggestion = None;
        let without_suggestion = state.snapshot();
        let mut restored_none = State::new(".".into(), false);
        restored_none.restore_global(&without_suggestion).unwrap();
        assert_eq!(restored_none.current().prompt_suggestion, None);

        // Changing machine clears the suggestion.
        state.current_mut().prompt_suggestion = Some(suggestion);
        state.current_mut().set_machine("remote-host");
        assert_eq!(state.current().prompt_suggestion, None);
    }

    #[test]
    fn prompt_suggestion_show_rule_conditions() {
        let mut state = State::new(".".into(), false);
        state.thread_defaults.prompt_suggestions_enabled = true;
        let t = state.current_mut();
        t.harness = "claude".into();
        t.status = Status::Completed;
        t.run_id = Some("run-1".into());
        t.prompt_suggestion = Some(PromptSuggestion {
            run_id: "run-1".into(),
            text: "Run cargo test".into(),
        });

        // 1. Shows on empty draft after completion.
        assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

        // 2. Hidden with text, shows again after deleting back to empty.
        state.current_mut().draft.replace_text("hi");
        assert_eq!(state.offered_prompt_suggestion(), None);
        state.current_mut().draft.replace_text("");
        assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

        // 3. Hidden with an attachment.
        state.current_mut().draft_images.push(Attachment {
            id: "att-1".into(),
            name: "a.png".into(),
            mime: "image/png".into(),
            width: 1,
            height: 1,
            bytes: 3,
            path: None,
        });
        assert_eq!(state.offered_prompt_suggestion(), None);
        state.current_mut().draft_images.clear();
        assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

        // 4. Hidden with a pending approval.
        state.current_mut().request = Some(json!({"id": "req-1", "method": "confirm"}));
        assert_eq!(state.offered_prompt_suggestion(), None);
        state.current_mut().request = None;
        assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

        // 5. Hidden while running.
        state.current_mut().status = Status::Running;
        assert_eq!(state.offered_prompt_suggestion(), None);
        state.current_mut().status = Status::Completed;
        assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

        // 6. Hidden after dismissal for this run.
        assert!(state.dismiss_prompt_suggestion());
        assert_eq!(state.offered_prompt_suggestion(), None);

        // 7. Shows again for the next run.
        let t = state.current_mut();
        t.run_id = Some("run-2".into());
        t.prompt_suggestion = Some(PromptSuggestion {
            run_id: "run-2".into(),
            text: "Inspect the diff".into(),
        });
        assert_eq!(state.offered_prompt_suggestion(), Some("Inspect the diff"));
    }

    #[test]
    fn usage_limits_command_raises_notice_without_sending_and_requires_capability() {
        let mut state = State::new(".".into(), false);
        // Pi has no report, so `/usage-limits` is not offered and sends as ordinary text.
        state.current_mut().draft.focused = true;
        state.current_mut().draft.replace_text("/");
        assert!(
            !state
                .slash_matches()
                .iter()
                .any(|c| c.name == crate::usage_limits::USAGE_LIMITS_COMMAND_NAME)
        );

        let (pi, commands, _events) = Pi::test_connection();
        {
            let t = state.current_mut();
            t.harness = "codex".into();
            t.model_provider = "codex".into();
            t.pi = Some(pi);
            t.harness_state["usageLimitsRaw"] = json!({
                "checkedAtMs": 1_700_000_000_000i64,
                "snapshot": {
                    "limitId": "codex",
                    "planType": "plus",
                    "primary": {"usedPercent": 35.0, "windowDurationMins": 300, "resetsAt": 1_700_003_600}
                }
            });
            t.draft.replace_text("/");
        }
        assert!(
            state
                .slash_matches()
                .iter()
                .any(|c| c.name == crate::usage_limits::USAGE_LIMITS_COMMAND_NAME)
        );

        // Sending `/usage-limits` alone raises the notice, clears the draft, and sends nothing.
        state.current_mut().draft.replace_text("/usage-limits");
        state.send();
        assert!(state.current().draft.text.is_empty());
        assert!(commands.try_recv().is_err());
        let notices = state.composer_notices.ordered();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].id, "usage-limits");
        assert_eq!(notices[0].title, "Usage limits");
        assert!(matches!(
            notices[0].body,
            Some(crate::composer_notices::NoticeBody::UsageLimits(_))
        ));

        // Dismissing removes it; choosing from the slash menu raises it again without sending.
        state.dismiss_composer_notice("usage-limits");
        assert!(state.composer_notices.ordered().is_empty());
        state.current_mut().draft.replace_text("/usage");
        state.choose_slash(crate::usage_limits::USAGE_LIMITS_COMMAND_NAME.into());
        assert!(state.current().draft.text.is_empty());
        assert!(commands.try_recv().is_err());
        assert_eq!(state.composer_notices.ordered().len(), 1);

        // Sending `/usage-limits` with extra text sends as ordinary text (KSG `isUsageLimitsCommand`).
        state
            .current_mut()
            .draft
            .replace_text("/usage-limits explain");
        state.send();
        let sent = commands.try_recv().expect("ordinary prompt sent");
        assert_eq!(sent["type"], "prompt");
        assert_eq!(sent["message"], "/usage-limits explain");

        // Machine lacking `usage-limits` capability hides the command.
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        *client.info.lock().unwrap() = json!({"capabilities": []});
        state.client = Some(client);
        state.current_mut().draft.focused = true;
        state.current_mut().draft.replace_text("/");
        assert!(
            !state
                .slash_matches()
                .iter()
                .any(|c| c.name == crate::usage_limits::USAGE_LIMITS_COMMAND_NAME)
        );
    }

    #[test]
    fn fallback_model_switches_when_out_of_usage_restores_on_reset_and_use_default_clears_notice() {
        let mut state = State::new(".".into(), false);
        state.default_harness = "codex".into();
        state.codex_catalog.models.push(
            AgentModel::from_harness_value(
                &json!({
                    "provider": "codex",
                    "id": "gpt-6-astra",
                    "name": "GPT-6 Astra",
                    "isDefault": true,
                    "thinkingLevels": ["medium"]
                }),
                "codex",
            )
            .unwrap(),
        );
        state.claude_catalog.models.push(
            AgentModel::from_harness_value(
                &json!({
                    "provider": "claude",
                    "id": "claude-opus-5-5",
                    "name": "Claude Opus 5.5",
                    "isDefault": true,
                    "thinkingLevels": ["high"]
                }),
                "claude",
            )
            .unwrap(),
        );
        state.thread_defaults.fallback_model_selection =
            Some(crate::model_selection::ModelSelection {
                instance_id: "claude".into(),
                model: "claude-opus-5-5".into(),
                options: vec![],
            });
        let now_ms = 1_700_000_000_000i64;
        // Codex primary window is exhausted (100% used, resets 1h from now_ms).
        state.codex_catalog.usage_limits_raw = json!({
            "checkedAtMs": now_ms,
            "snapshot": {
                "limitId": "codex",
                "planType": "plus",
                "primary": {"usedPercent": 100.0, "windowDurationMins": 300, "resetsAt": 1_700_003_600}
            }
        });

        state.sync_fallback_model(now_ms);
        assert_eq!(state.current().harness, "claude");
        assert_eq!(state.current().model_id, "claude-opus-5-5");
        let notices = state.composer_notices.ordered();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].id, "fallback-model");
        assert_eq!(notices[0].title, "Using Claude Opus 5.5");
        assert!(
            notices[0]
                .description
                .as_deref()
                .unwrap()
                .starts_with("Codex is out of usage · resets ")
        );
        assert_eq!(notices[0].actions.len(), 1);
        assert_eq!(notices[0].actions[0].label, "Use GPT-6 Astra");
        assert_eq!(
            notices[0].dismiss_label.as_deref(),
            Some("Dismiss fallback model notice")
        );

        // Dismissing keeps the notice hidden while the fallback state continues.
        state.dismiss_composer_notice("fallback-model");
        assert!(state.composer_notices.ordered().is_empty());
        state.sync_fallback_model(now_ms + 60_000);
        assert!(state.composer_notices.ordered().is_empty());

        // When usage resets (now passes resetsAt), the thread restores its default model and
        // clears dismissal so a later exhaustion can show the notice again.
        state.sync_fallback_model(1_700_003_601_000);
        assert_eq!(state.current().harness, "codex");
        assert_eq!(state.current().model_id, "gpt-6-astra");
        assert!(state.composer_notices.ordered().is_empty());

        // Out of usage again -> switches and re-raises notice; clicking "Use default" switches back and removes notice.
        state.codex_catalog.usage_limits_raw = json!({
            "checkedAtMs": 1_700_010_000_000i64,
            "snapshot": {
                "limitId": "codex",
                "planType": "plus",
                "primary": {"usedPercent": 100.0, "windowDurationMins": 300, "resetsAt": 1_700_020_000}
            }
        });
        state.sync_fallback_model(1_700_010_000_000);
        assert_eq!(state.current().harness, "claude");
        let action = state.composer_notices.ordered()[0].actions[0]
            .handler
            .clone();
        action.call(&mut state);
        assert_eq!(state.current().harness, "codex");
        assert_eq!(state.current().model_id, "gpt-6-astra");
        assert!(state.composer_notices.ordered().is_empty());
        // Explicit override persists while staying on that thread.
        state.sync_fallback_model(1_700_010_060_000);
        assert_eq!(state.current().harness, "codex");
        assert!(state.composer_notices.ordered().is_empty());
    }

    #[test]
    fn worktree_send_without_base_branch_is_refused_with_exact_message() {
        let mut state = State::new(".".into(), false);
        state.current_mut().harness = "claude".into();
        state.current_mut().model_provider = "claude".into();
        state.current_mut().env_mode = Some(ThreadEnvMode::Worktree);
        state.current_mut().base_branch = None;
        state.current_mut().draft.replace_text("Do work");
        state.send();
        assert_eq!(
            state.current().submission_error.as_deref(),
            Some("Select a base branch before sending in New worktree mode.")
        );
        assert_eq!(state.current().draft.text, "Do work");
        assert!(state.current().messages.is_empty());
        assert!(state.current().native.queue.is_empty());
        assert!(state.current().worktree_path.is_none());
    }

    fn init_temp_git_repo(root: &std::path::Path, empty_config: &std::path::Path) -> PathBuf {
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let run = |cwd: &std::path::Path, args: &[&str]| {
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-c")
                .arg("user.name=Test User")
                .arg("-c")
                .arg("user.email=test@example.com")
                .args(args)
                .current_dir(cwd)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_CONFIG_GLOBAL", empty_config)
                .env("GIT_CONFIG_SYSTEM", empty_config);
            let out = crate::process::output(&mut cmd, Duration::from_secs(10), 1024 * 1024)
                .expect("spawn git");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&repo, &["init", "-b", "main", "--quiet"]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        run(&repo, &["add", "README.md"]);
        run(&repo, &["commit", "-m", "initial", "--quiet"]);
        repo
    }

    #[test]
    fn worktree_bootstrap_creates_worktree_starts_harness_and_queues_second_send() {
        if !crate::vcs::git_available() {
            return;
        }
        let root = std::env::temp_dir().join(format!("shina-wt-boot-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let empty_cfg = root.join("empty.gitconfig");
        std::fs::write(&empty_cfg, "").unwrap();
        let repo = init_temp_git_repo(&root, &empty_cfg);
        let git_ctx = crate::vcs::GitContext {
            git_config_global: Some(empty_cfg.clone()),
            git_config_system: Some(empty_cfg.clone()),
        };

        let mut state = State::new(repo.clone(), false);
        state.data_dir = root.join("data");
        std::fs::create_dir_all(&state.data_dir).unwrap();
        state.runtime_enabled = true;
        state.test_git_context = Some(git_ctx);

        let commands_slot: std::sync::Arc<
            std::sync::Mutex<Option<std::sync::mpsc::Receiver<Value>>>,
        > = std::sync::Arc::default();
        let commands_slot_hook = commands_slot.clone();
        state.test_connect_pi = Some(Box::new(move |_tid, _path| {
            let (pi, commands, _events) = Pi::test_connection();
            *commands_slot_hook.lock().unwrap() = Some(commands);
            Some(pi)
        }));

        {
            let t = state.current_mut();
            t.harness = "claude".into();
            t.model_provider = "claude".into();
            t.env_mode = Some(ThreadEnvMode::Worktree);
            t.base_branch = Some("main".into());
            t.start_from_origin = Some(true);
            t.draft.replace_text("First turn in worktree");
        }
        state.send();
        assert!(state.current().worktree_bootstrapping());
        assert!(state.current().native.queue_held);
        assert_eq!(state.current().native.queue.len(), 1);
        assert!(state.current().draft.text.is_empty());

        // Second send while bootstrap is in progress queues behind the first message.
        state.current_mut().draft.replace_text("Second queued turn");
        state.send();
        assert_eq!(state.current().native.queue.len(), 2);
        assert!(state.current().native.queue_held);

        let deadline = Instant::now() + Duration::from_secs(10);
        while state.current().worktree_bootstrapping() && Instant::now() < deadline {
            state.poll_worktree_bootstraps();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !state.current().worktree_bootstrapping(),
            "bootstrap did not finish in time"
        );

        let wt_path = state
            .current()
            .worktree_path
            .clone()
            .expect("worktreePath recorded");
        assert!(std::path::Path::new(&wt_path).exists());
        assert!(
            state
                .current()
                .branch
                .as_deref()
                .is_some_and(|b| b.starts_with("t3/"))
        );
        assert_eq!(state.current().base_branch.as_deref(), Some("main"));
        assert_eq!(state.current().env_mode, Some(ThreadEnvMode::Worktree));
        assert_eq!(state.test_connect_workspaces.len(), 1);
        assert_eq!(state.test_connect_workspaces[0].1, PathBuf::from(&wt_path));
        assert_eq!(
            state.thread_directory(state.active).unwrap(),
            PathBuf::from(&wt_path)
        );

        // First prompt was released and sent to the harness; second stays queued behind it.
        assert_eq!(state.current().native.queue.len(), 1);
        assert_eq!(
            state.current().native.queue[0].prompt.text,
            "Second queued turn"
        );
        let commands = commands_slot.lock().unwrap().take().expect("pi commands");
        let first_cmd = commands
            .try_iter()
            .find(|c| c["type"] == "prompt")
            .expect("first prompt dispatched");
        assert_eq!(first_cmd["message"], "First turn in worktree");

        // Settling turn 1 releases the queued second turn in the same worktree.
        state.current_mut().apply(json!({
            "type": "response",
            "id": first_cmd["id"],
            "command": "prompt",
            "success": true,
            "data": {}
        }));
        state
            .current_mut()
            .apply(json!({"type": "agent_settled", "aborted": false}));
        assert!(state.current().native.queue.is_empty());
        let second_cmd = commands
            .try_iter()
            .find(|c| c["type"] == "prompt")
            .expect("second prompt dispatched");
        assert_eq!(second_cmd["message"], "Second queued turn");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn worktree_bootstrap_failure_restores_draft_and_records_nothing() {
        if !crate::vcs::git_available() {
            return;
        }
        let root = std::env::temp_dir().join(format!("shina-wt-fail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let empty_cfg = root.join("empty.gitconfig");
        std::fs::write(&empty_cfg, "").unwrap();
        let repo = init_temp_git_repo(&root, &empty_cfg);
        let git_ctx = crate::vcs::GitContext {
            git_config_global: Some(empty_cfg.clone()),
            git_config_system: Some(empty_cfg.clone()),
        };

        let mut state = State::new(repo, false);
        state.data_dir = root.join("data");
        std::fs::create_dir_all(&state.data_dir).unwrap();
        state.runtime_enabled = true;
        state.test_git_context = Some(git_ctx);

        {
            let t = state.current_mut();
            t.harness = "claude".into();
            t.model_provider = "claude".into();
            t.env_mode = Some(ThreadEnvMode::Worktree);
            t.base_branch = Some("missing-branch-does-not-exist".into());
            t.start_from_origin = Some(false);
            t.draft.replace_text("Keep this draft on failure");
        }
        state.send();
        assert!(state.current().worktree_bootstrapping());

        let deadline = Instant::now() + Duration::from_secs(10);
        while state.current().worktree_bootstrapping() && Instant::now() < deadline {
            state.poll_worktree_bootstraps();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!state.current().worktree_bootstrapping());
        assert_eq!(state.current().draft.text, "Keep this draft on failure");
        assert_eq!(state.current().env_mode, Some(ThreadEnvMode::Worktree));
        assert!(state.current().worktree_path.is_none());
        assert!(state.current().branch.is_none());
        assert!(state.current().native.queue.is_empty());
        assert!(!state.current().native.queue_held);
        assert!(state.test_connect_workspaces.is_empty());
        assert!(
            state
                .current()
                .submission_error
                .as_deref()
                .is_some_and(|e| e.contains("Git command failed")),
            "expected git error, got {:?}",
            state.current().submission_error
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn worktree_setup_script_succeeds_fails_and_retries() {
        if !crate::vcs::git_available() {
            return;
        }

        // 1. Setup script that succeeds holds the first message until setup completes, then runs it.
        let root_ok = std::env::temp_dir().join(format!("shina-setup-ok-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root_ok).unwrap();
        let empty_cfg_ok = root_ok.join("empty.gitconfig");
        std::fs::write(&empty_cfg_ok, "").unwrap();
        let repo_ok = init_temp_git_repo(&root_ok, &empty_cfg_ok);
        std::fs::write(
            repo_ok.join("t3.json"),
            r#"{"scripts":[{"name":"Setup","command":"echo line1 && echo setup-done > setup.marker","runOnWorktreeCreate":true}]}"#,
        )
        .unwrap();
        {
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-c")
                .arg("user.name=Test User")
                .arg("-c")
                .arg("user.email=test@example.com")
                .args(["add", "t3.json"])
                .current_dir(&repo_ok)
                .env("GIT_CONFIG_GLOBAL", &empty_cfg_ok)
                .env("GIT_CONFIG_SYSTEM", &empty_cfg_ok);
            assert!(cmd.status().unwrap().success());
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-c")
                .arg("user.name=Test User")
                .arg("-c")
                .arg("user.email=test@example.com")
                .args(["commit", "-m", "add t3.json", "--quiet"])
                .current_dir(&repo_ok)
                .env("GIT_CONFIG_GLOBAL", &empty_cfg_ok)
                .env("GIT_CONFIG_SYSTEM", &empty_cfg_ok);
            assert!(cmd.status().unwrap().success());
        }

        let mut state_ok = State::new(repo_ok, false);
        state_ok.data_dir = root_ok.join("data");
        std::fs::create_dir_all(&state_ok.data_dir).unwrap();
        state_ok.runtime_enabled = true;
        state_ok.test_git_context = Some(crate::vcs::GitContext {
            git_config_global: Some(empty_cfg_ok.clone()),
            git_config_system: Some(empty_cfg_ok.clone()),
        });
        let cmds_ok: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<Value>>>> =
            std::sync::Arc::default();
        let cmds_ok_hook = cmds_ok.clone();
        state_ok.test_connect_pi = Some(Box::new(move |_tid, _path| {
            let (pi, commands, _events) = Pi::test_connection();
            *cmds_ok_hook.lock().unwrap() = Some(commands);
            Some(pi)
        }));
        {
            let t = state_ok.current_mut();
            t.harness = "claude".into();
            t.model_provider = "claude".into();
            t.env_mode = Some(ThreadEnvMode::Worktree);
            t.base_branch = Some("main".into());
            t.start_from_origin = Some(false);
            t.draft.replace_text("After setup succeeds");
        }
        state_ok.send();

        let deadline = Instant::now() + Duration::from_secs(10);
        while state_ok.current().worktree_bootstrapping() && Instant::now() < deadline {
            state_ok.poll_worktree_bootstraps();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            state_ok.current().worktree_setup.as_ref().map(|s| s.phase),
            Some(WorktreeSetupPhase::Running)
        );
        assert!(state_ok.current().native.queue_held);
        assert!(state_ok.test_connect_workspaces.is_empty());

        let deadline = Instant::now() + Duration::from_secs(10);
        while state_ok
            .current()
            .worktree_setup
            .as_ref()
            .is_some_and(|s| s.phase == WorktreeSetupPhase::Running)
            && Instant::now() < deadline
        {
            state_ok.poll_worktree_setup();
            std::thread::sleep(Duration::from_millis(10));
        }
        let setup = state_ok.current().worktree_setup.as_ref().unwrap();
        assert_eq!(setup.phase, WorktreeSetupPhase::Succeeded);
        assert!(setup.output.contains("line1"));
        assert!(!state_ok.current().native.queue_held);
        let wt_path = state_ok.current().worktree_path.clone().unwrap();
        assert!(std::path::Path::new(&wt_path).join("setup.marker").exists());
        let sent = cmds_ok
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .try_iter()
            .find(|c| c["type"] == "prompt")
            .expect("first message released after setup succeeded");
        assert_eq!(sent["message"], "After setup succeeds");

        // Snapshot + restore preserves setup card state.
        let snap = state_ok.snapshot();
        let mut restored = State::new(".".into(), false);
        restored.restore_global(&snap).unwrap();
        assert_eq!(
            restored.current().worktree_setup.as_ref().map(|s| s.phase),
            Some(WorktreeSetupPhase::Succeeded)
        );
        let _ = std::fs::remove_dir_all(&root_ok);

        // 2. Setup script that fails holds the first message; retry succeeds and releases it.
        let root_fail =
            std::env::temp_dir().join(format!("shina-setup-fail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root_fail).unwrap();
        let empty_cfg_fail = root_fail.join("empty.gitconfig");
        std::fs::write(&empty_cfg_fail, "").unwrap();
        let repo_fail = init_temp_git_repo(&root_fail, &empty_cfg_fail);
        let gate_file = root_fail.join("allow_pass");
        let cmd_str = format!(
            "if [ -f '{}' ]; then echo retry-ok; else echo failing-tail; exit 2; fi",
            gate_file.display()
        );
        std::fs::write(
            repo_fail.join("t3.json"),
            json!({
                "scripts": [{
                    "name": "Setup",
                    "command": cmd_str,
                    "runOnWorktreeCreate": true
                }]
            })
            .to_string(),
        )
        .unwrap();
        {
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-c")
                .arg("user.name=Test User")
                .arg("-c")
                .arg("user.email=test@example.com")
                .args(["add", "t3.json"])
                .current_dir(&repo_fail)
                .env("GIT_CONFIG_GLOBAL", &empty_cfg_fail)
                .env("GIT_CONFIG_SYSTEM", &empty_cfg_fail);
            assert!(cmd.status().unwrap().success());
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-c")
                .arg("user.name=Test User")
                .arg("-c")
                .arg("user.email=test@example.com")
                .args(["commit", "-m", "add t3.json", "--quiet"])
                .current_dir(&repo_fail)
                .env("GIT_CONFIG_GLOBAL", &empty_cfg_fail)
                .env("GIT_CONFIG_SYSTEM", &empty_cfg_fail);
            assert!(cmd.status().unwrap().success());
        }

        let mut state_fail = State::new(repo_fail, false);
        state_fail.data_dir = root_fail.join("data");
        std::fs::create_dir_all(&state_fail.data_dir).unwrap();
        state_fail.runtime_enabled = true;
        state_fail.test_git_context = Some(crate::vcs::GitContext {
            git_config_global: Some(empty_cfg_fail.clone()),
            git_config_system: Some(empty_cfg_fail.clone()),
        });
        let cmds_fail: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<Value>>>> =
            std::sync::Arc::default();
        let cmds_fail_hook = cmds_fail.clone();
        state_fail.test_connect_pi = Some(Box::new(move |_tid, _path| {
            let (pi, commands, _events) = Pi::test_connection();
            *cmds_fail_hook.lock().unwrap() = Some(commands);
            Some(pi)
        }));
        {
            let t = state_fail.current_mut();
            t.harness = "claude".into();
            t.model_provider = "claude".into();
            t.env_mode = Some(ThreadEnvMode::Worktree);
            t.base_branch = Some("main".into());
            t.start_from_origin = Some(false);
            t.draft.replace_text("After retry succeeds");
        }
        state_fail.send();

        let deadline = Instant::now() + Duration::from_secs(10);
        while state_fail.current().worktree_bootstrapping() && Instant::now() < deadline {
            state_fail.poll_worktree_bootstraps();
            std::thread::sleep(Duration::from_millis(10));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while state_fail
            .current()
            .worktree_setup
            .as_ref()
            .is_some_and(|s| s.phase == WorktreeSetupPhase::Running)
            && Instant::now() < deadline
        {
            state_fail.poll_worktree_setup();
            std::thread::sleep(Duration::from_millis(10));
        }
        let failed = state_fail.current().worktree_setup.as_ref().unwrap();
        assert_eq!(failed.phase, WorktreeSetupPhase::Failed);
        assert_eq!(failed.exit_code, Some(2));
        assert!(failed.output.contains("failing-tail"));
        assert!(state_fail.current().native.queue_held);
        assert!(state_fail.test_connect_workspaces.is_empty());

        // Fix condition and retry: setup succeeds and releases the queued message.
        std::fs::write(&gate_file, "ok").unwrap();
        state_fail.retry_worktree_setup();
        assert_eq!(
            state_fail
                .current()
                .worktree_setup
                .as_ref()
                .map(|s| s.phase),
            Some(WorktreeSetupPhase::Running)
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while state_fail
            .current()
            .worktree_setup
            .as_ref()
            .is_some_and(|s| s.phase == WorktreeSetupPhase::Running)
            && Instant::now() < deadline
        {
            state_fail.poll_worktree_setup();
            std::thread::sleep(Duration::from_millis(10));
        }
        let retried = state_fail.current().worktree_setup.as_ref().unwrap();
        assert_eq!(retried.phase, WorktreeSetupPhase::Succeeded);
        assert!(retried.output.contains("retry-ok"));
        assert!(!state_fail.current().native.queue_held);
        let sent_retry = cmds_fail
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .try_iter()
            .find(|c| c["type"] == "prompt")
            .expect("first message released after retry");
        assert_eq!(sent_retry["message"], "After retry succeeds");

        let _ = std::fs::remove_dir_all(&root_fail);
    }

    #[test]
    fn draft_workspace_mode_and_base_branch_persist_across_thread_switch_and_reload() {
        let dir =
            std::env::temp_dir().join(format!("shina-draft-persist-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let index_path = dir.join("index.json");
        let mut store = crate::store::Store::new(index_path.clone(), Arc::new(|| {}));

        let mut state = State::new(".".into(), false);
        state.data_dir = dir.clone();
        state.current_mut().messages.push(Message {
            id: "u1".into(),
            role: "You",
            text: "First thread".into(),
            images: vec![],
            context: None,
        });
        state.thread_defaults.default_thread_env_mode = ThreadEnvMode::Worktree;
        state.new_thread();
        assert_eq!(state.threads.len(), 2);
        let t2_idx = state.active;
        assert_eq!(
            state.threads[t2_idx].env_mode,
            Some(ThreadEnvMode::Worktree)
        );
        state.set_draft_base_branch("main".into());
        state.set_draft_start_from_origin(false);

        // Switch away to thread 0 and switch back: draft choices persist.
        state.select_thread(0);
        state.select_thread(t2_idx);
        assert_eq!(state.current().env_mode, Some(ThreadEnvMode::Worktree));
        assert_eq!(state.current().base_branch.as_deref(), Some("main"));
        assert_eq!(state.current().start_from_origin, Some(false));
        assert!(!state.current().effective_start_from_origin());

        // Save to disk and reload from temporary data directory.
        let snapshot = state.snapshot();
        store.save(snapshot);
        store.flush().unwrap();
        let loaded = crate::store::load(&index_path)
            .unwrap()
            .expect("saved index");
        let mut reloaded = State::new(".".into(), false);
        reloaded.data_dir = dir.clone();
        reloaded.restore_global(&loaded).unwrap();
        assert_eq!(
            reloaded.thread_defaults.default_thread_env_mode,
            ThreadEnvMode::Worktree
        );
        let restored_t2 = reloaded
            .threads
            .iter()
            .find(|t| t.id == state.threads[t2_idx].id)
            .expect("restored draft thread");
        assert_eq!(restored_t2.env_mode, Some(ThreadEnvMode::Worktree));
        assert_eq!(restored_t2.base_branch.as_deref(), Some("main"));
        assert_eq!(restored_t2.start_from_origin, Some(false));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn setup_multi_model_repo_state(root: &std::path::Path, models: Vec<AgentModel>) -> State {
        let empty_cfg = root.join("empty.gitconfig");
        std::fs::write(&empty_cfg, "").unwrap();
        let repo = init_temp_git_repo(root, &empty_cfg);
        let git_ctx = crate::vcs::GitContext {
            git_config_global: Some(empty_cfg.clone()),
            git_config_system: Some(empty_cfg),
        };
        let mut state = State::new(repo.clone(), false);
        state.data_dir = root.join("data");
        std::fs::create_dir_all(&state.data_dir).unwrap();
        state.runtime_enabled = true;
        state.test_git_context = Some(git_ctx);
        let project_path = state
            .project(state.current().project)
            .unwrap()
            .path
            .to_string_lossy()
            .into_owned();
        state.vcs_branches.insert(
            ("local".into(), project_path),
            VcsBranchCacheEntry {
                is_repo: true,
                loaded: true,
                loading: false,
                error: None,
                refs: vec![crate::vcs::VcsRef {
                    name: "main".into(),
                    is_remote: Some(false),
                    remote_name: None,
                    upstream_ref: None,
                    current: true,
                    is_default: true,
                    worktree_path: None,
                }],
            },
        );
        state.catalog.models = models.clone();
        state.catalog.default_model = models.first().cloned();
        state.current_mut().harness = "pi".into();
        state.current_mut().models = models;
        if let Some(first) = state.catalog.models.first().cloned() {
            state.current_mut().set_model(first.provider, first.id);
        }
        state
    }

    #[test]
    fn multi_model_send_three_models_creates_three_worktrees_and_clears_draft() {
        if !crate::vcs::git_available() {
            return;
        }
        let root = std::env::temp_dir().join(format!("shina-mm-three-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let models = vec![
            AgentModel {
                provider: "cpa".into(),
                id: "cpa/claude-sonnet-4-5".into(),
                name: "Claude Sonnet 4.5".into(),
                thinking_levels: vec!["low".into(), "high".into()],
                supports_images: Some(true),
                ..Default::default()
            },
            AgentModel {
                provider: "cpa".into(),
                id: "cpa/gpt-5".into(),
                name: "GPT-5".into(),
                thinking_levels: vec!["medium".into(), "high".into()],
                supports_images: Some(true),
                ..Default::default()
            },
            AgentModel {
                provider: "cpa".into(),
                id: "cpa/gemini-2-5-pro".into(),
                name: "Gemini 2.5 Pro".into(),
                supports_images: Some(true),
                ..Default::default()
            },
        ];
        let mut state = setup_multi_model_repo_state(&root, models);
        let captured: std::sync::Arc<
            std::sync::Mutex<Vec<(usize, std::sync::mpsc::Receiver<Value>)>>,
        > = std::sync::Arc::default();
        let captured_hook = captured.clone();
        state.test_connect_pi = Some(Box::new(move |tid, _path| {
            let (pi, commands, _events) = Pi::test_connection();
            captured_hook.lock().unwrap().push((tid, commands));
            Some(pi)
        }));

        state.set_composer_thinking_sticky("high".into());
        state.toggle_model_in_multi_selection("pi", "cpa/gpt-5");
        state.toggle_model_in_multi_selection("pi", "cpa/gemini-2-5-pro");
        assert_eq!(state.multi_model_selections().len(), 3);

        let img = test_attachment("shared.png", 1);
        state
            .current_mut()
            .draft
            .replace_text("Compare these three models");
        state.current_mut().add_attachment(img.clone()).unwrap();
        let draft_id = state.current().id;

        state.send();

        let deadline = Instant::now() + Duration::from_secs(10);
        while state.threads.iter().any(|t| t.worktree_bootstrapping()) && Instant::now() < deadline
        {
            state.poll_worktree_bootstraps();
            std::thread::sleep(Duration::from_millis(10));
        }

        // User stays on the draft, and the draft + multi-selection are cleared.
        assert_eq!(state.current().id, draft_id);
        assert!(state.current().draft.text.is_empty());
        assert!(state.current().draft_images.is_empty());
        assert!(
            state
                .current()
                .draft_model_state
                .additional_models
                .is_empty()
        );
        assert_eq!(state.threads.len(), 4);

        let started: Vec<&Thread> = state.threads.iter().filter(|t| t.id != draft_id).collect();
        assert_eq!(started.len(), 3);
        let worktrees: std::collections::HashSet<&str> = started
            .iter()
            .map(|t| t.worktree_path.as_deref().expect("worktree path"))
            .collect();
        let branches: std::collections::HashSet<&str> = started
            .iter()
            .map(|t| t.branch.as_deref().expect("branch"))
            .collect();
        assert_eq!(worktrees.len(), 3);
        assert_eq!(branches.len(), 3);
        for wt in &worktrees {
            assert!(std::path::Path::new(wt).exists(), "missing worktree {wt}");
        }
        for b in &branches {
            assert!(b.starts_with("t3/"), "unexpected branch {b}");
        }
        assert_eq!(
            started
                .iter()
                .map(|t| t.model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["cpa/claude-sonnet-4-5", "cpa/gpt-5", "cpa/gemini-2-5-pro"]
        );
        for t in &started {
            assert_eq!(t.title, "Compare these three models");
            assert_eq!(t.base_branch.as_deref(), Some("main"));
            assert_eq!(t.env_mode, Some(ThreadEnvMode::Worktree));
            assert_eq!(t.messages.len(), 1);
            assert_eq!(t.messages[0].text, "Compare these three models");
            assert_eq!(t.messages[0].images, vec![img.clone()]);
        }

        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].kind, ToastKind::Success);
        assert_eq!(state.toasts[0].title, "Started 3 threads in background");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn multi_model_send_partial_failure_keeps_two_threads_restores_failed_selection_and_prompt() {
        if !crate::vcs::git_available() {
            return;
        }
        let root = std::env::temp_dir().join(format!("shina-mm-partial-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let models = vec![
            AgentModel {
                provider: "cpa".into(),
                id: "cpa/claude-sonnet-4-5".into(),
                name: "Claude Sonnet 4.5".into(),
                supports_images: Some(true),
                ..Default::default()
            },
            AgentModel {
                provider: "cpa".into(),
                id: "cpa/text-only".into(),
                name: "Text Only Model".into(),
                supports_images: Some(false),
                ..Default::default()
            },
            AgentModel {
                provider: "cpa".into(),
                id: "cpa/gemini-2-5-pro".into(),
                name: "Gemini 2.5 Pro".into(),
                supports_images: Some(true),
                ..Default::default()
            },
        ];
        let mut state = setup_multi_model_repo_state(&root, models);
        let kept_channels: std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Receiver<Value>>>> =
            std::sync::Arc::default();
        let kept_hook = kept_channels.clone();
        state.test_connect_pi = Some(Box::new(move |_tid, _path| {
            let (pi, commands, _events) = Pi::test_connection();
            kept_hook.lock().unwrap().push(commands);
            Some(pi)
        }));

        state.toggle_model_in_multi_selection("pi", "cpa/text-only");
        state.toggle_model_in_multi_selection("pi", "cpa/gemini-2-5-pro");
        let img = test_attachment("keep-me.png", 1);
        state
            .current_mut()
            .draft
            .replace_text("Inspect this screenshot");
        state.current_mut().add_attachment(img.clone()).unwrap();
        let draft_id = state.current().id;

        state.send();

        let deadline = Instant::now() + Duration::from_secs(10);
        while state.threads.iter().any(|t| t.worktree_bootstrapping()) && Instant::now() < deadline
        {
            state.poll_worktree_bootstraps();
            std::thread::sleep(Duration::from_millis(10));
        }

        // Two started threads remain (draft + 2 started = 3 threads total).
        let started: Vec<&Thread> = state.threads.iter().filter(|t| t.id != draft_id).collect();
        assert_eq!(started.len(), 2);
        assert_eq!(
            started
                .iter()
                .map(|t| t.model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["cpa/claude-sonnet-4-5", "cpa/gemini-2-5-pro"]
        );
        for t in &started {
            assert_eq!(t.messages[0].images, vec![img.clone()]);
        }

        // Failed middle selection stays selected and prompt + attachment are restored on draft.
        assert_eq!(state.current().id, draft_id);
        assert_eq!(state.current().draft.text, "Inspect this screenshot");
        assert_eq!(state.current().draft_images, vec![img]);
        assert_eq!(
            state
                .multi_model_selections()
                .iter()
                .map(|s| s.model.as_str())
                .collect::<Vec<_>>(),
            vec!["cpa/text-only"]
        );

        // One notification names the failed model.
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].kind, ToastKind::Error);
        assert_eq!(state.toasts[0].title, "Could not start Text Only Model");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn multi_model_send_total_failure_restores_full_draft_and_missing_base_branch_blocks() {
        if !crate::vcs::git_available() {
            return;
        }
        let root =
            std::env::temp_dir().join(format!("shina-mm-total-fail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let models = vec![
            AgentModel {
                provider: "cpa".into(),
                id: "cpa/claude-sonnet-4-5".into(),
                name: "Claude Sonnet 4.5".into(),
                ..Default::default()
            },
            AgentModel {
                provider: "cpa".into(),
                id: "cpa/gpt-5".into(),
                name: "GPT-5".into(),
                ..Default::default()
            },
            AgentModel {
                provider: "cpa".into(),
                id: "cpa/gemini-2-5-pro".into(),
                name: "Gemini 2.5 Pro".into(),
                ..Default::default()
            },
        ];
        let mut state = setup_multi_model_repo_state(&root, models);

        // Missing base branch blocks before sending anything.
        let project_path = state
            .project(state.current().project)
            .unwrap()
            .path
            .to_string_lossy()
            .into_owned();
        state.vcs_branches.insert(
            ("local".into(), project_path),
            VcsBranchCacheEntry {
                is_repo: true,
                loaded: true,
                loading: false,
                error: None,
                refs: vec![],
            },
        );
        state.toggle_model_in_multi_selection("pi", "cpa/gpt-5");
        state.toggle_model_in_multi_selection("pi", "cpa/gemini-2-5-pro");
        let img = test_attachment("retry.png", 1);
        state
            .current_mut()
            .draft
            .replace_text("Keep full draft on failure");
        state.current_mut().add_attachment(img.clone()).unwrap();
        state.current_mut().base_branch = None;

        state.send();
        assert_eq!(state.threads.len(), 1);
        assert_eq!(
            state.current().submission_error.as_deref(),
            Some("Select a base branch before sending in New worktree mode.")
        );
        assert_eq!(state.current().draft.text, "Keep full draft on failure");

        // Total failure when all three worktree creations fail restores full draft and all selections.
        state.current_mut().submission_error = None;
        state.current_mut().base_branch = Some("missing-branch-does-not-exist".into());
        state.send();

        let deadline = Instant::now() + Duration::from_secs(10);
        while state.threads.iter().any(|t| t.worktree_bootstrapping()) && Instant::now() < deadline
        {
            state.poll_worktree_bootstraps();
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(state.threads.len(), 1);
        assert_eq!(state.current().draft.text, "Keep full draft on failure");
        assert_eq!(state.current().draft_images, vec![img]);
        assert_eq!(
            state
                .multi_model_selections()
                .iter()
                .map(|s| s.model.as_str())
                .collect::<Vec<_>>(),
            vec!["cpa/claude-sonnet-4-5", "cpa/gpt-5", "cpa/gemini-2-5-pro"]
        );
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].kind, ToastKind::Error);
        assert_eq!(
            state.toasts[0].title,
            "Could not start Claude Sonnet 4.5, GPT-5, Gemini 2.5 Pro"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
