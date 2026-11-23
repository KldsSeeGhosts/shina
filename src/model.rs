use crate::attachments::{self, Attachment, Prompt};
use crate::backend::{Event, HostLink};
use crate::captains::{Captain, CaptainEditor};
use crate::context::ContextMeter;
use crate::harness::Session as Pi;
use crate::lifecycle::{
    self, Action, DropPlan, Lifecycle, Section, SnoozeChoice, UndoNotice, UndoStep, Working,
};
use crate::projects::{Project, ProjectId};
use crate::session::{AgentModel, Catalog, IDLE_TIMEOUT, START_TIMEOUT};
use rune::{TextEdit, Tx, Waker};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// The label shown when a thread uses whatever model Pi is configured with.
pub const DEFAULT_MODEL: &str = "Default model";
pub const SIDEBAR_MIN: f32 = 220.0;

/// The floating thread details card, ported from KSG Code's `ThreadDetailsCard`: docked at
/// the chat pane's top right where it fits, a popover from the top bar elsewhere.
#[derive(Clone, Debug)]
pub struct DetailsCard {
    /// The user keeps the docked card on screen. Saved with the UI state.
    pub docked: bool,
    /// The popover is open, where the card cannot dock.
    pub popover: bool,
    /// "Show N more": the session rows under the workspace rows.
    pub more: bool,
    /// The "previous agents" group is unfolded.
    pub previous_open: bool,
    /// How many previous agents the group lists: KSG's 6, then 12 more at a time.
    pub previous_shown: usize,
    /// The lineage row under the pointer, for its hover card.
    pub hover: Option<String>,
    /// Whether the card can dock in the chat pane, from the last frame.
    pub fits: std::cell::Cell<bool>,
    /// The chat pane's height, from the last layout.
    pub pane_height: std::cell::Cell<f32>,
}

impl Default for DetailsCard {
    fn default() -> Self {
        Self {
            docked: true,
            popover: false,
            more: false,
            previous_open: false,
            previous_shown: 6,
            hover: None,
            fits: std::cell::Cell::new(false),
            pane_height: std::cell::Cell::new(720.0),
        }
    }
}
pub const SIDEBAR_MAX: f32 = 420.0;
// KSG Code's `SIDEBAR_WIDTH`: 16 rem.
pub const DEFAULT_SIDEBAR_WIDTH: f32 = 256.0;
const PREVIOUS_DEFAULT_SIDEBAR_WIDTH: f32 = 288.0;
pub(crate) const INJECTION_PREFIX: &str = "[Shina orchestration]\n";

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
#[path = "crew_question_state.rs"]
pub(crate) mod crew_question_state;
#[path = "crew_questions.rs"]
pub(crate) mod crew_questions;
pub use crew_question_state::CrewQuestionDraft;
#[path = "admission.rs"]
mod admission;
#[path = "crew_reports.rs"]
pub(crate) mod crew_reports;
#[path = "question_answers.rs"]
mod question_answers;
#[cfg(test)]
#[path = "question_attachment_tests.rs"]
mod question_attachment_tests;
#[cfg(test)]
#[path = "question_flow_tests.rs"]
mod question_flow_tests;
#[path = "task_plan.rs"]
mod task_plan;
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
#[path = "provider_gate.rs"]
mod provider_gate;
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
#[path = "captain_event_policy.rs"]
mod captain_event_policy;
#[path = "captain_inbox.rs"]
mod captain_inbox;
pub use captain_inbox::RecoveryMode;
#[path = "captain_state.rs"]
mod captain_state;
pub use captain_event_policy::ReportPolicy;
#[path = "captain_policy_client.rs"]
mod captain_policy_client;
pub use captain_policy_client::CaptainPolicyChange;
#[path = "captain_digest.rs"]
mod captain_digest;
#[allow(unused_imports)]
pub(crate) use captain_state::{
    CONTEXT_TRUNCATION, CREW_CONTEXT_MAX_BYTES, NOTES_CONTEXT_MAX_BYTES, RULES_CONTEXT_MAX_BYTES,
    covered_bytes,
};
#[allow(unused_imports)]
pub use captain_state::{CaptainAccess, CaptainDocument, CaptainDocumentMeta};
#[path = "captain_lifecycle.rs"]
mod captain_lifecycle;
pub use captain_lifecycle::CaptainPhase;
#[path = "captain_crew_draft.rs"]
mod captain_crew_draft;
#[path = "captain_crew_ops.rs"]
mod captain_crew_ops;
pub use captain_crew_draft::CrewDraftTarget;
#[path = "captain_delete.rs"]
mod captain_delete;
#[path = "captain_lifecycle_state.rs"]
mod captain_lifecycle_state;
#[path = "captain_promotion_state.rs"]
mod captain_promotion_state;
pub use captain_promotion_state::PromotionChange;
#[path = "captain_roles.rs"]
mod captain_roles;
#[cfg(test)]
#[path = "handoff_tests.rs"]
mod handoff_tests;
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
pub(crate) use memory::{ESSENTIAL_BUDGET, PROFILE_BUDGET};
#[allow(unused_imports)]
pub use memory::{MemoryBackendStatus, MemoryView, UserProfile};
pub use scope::RemoteProject;
#[path = "capabilities.rs"]
mod capabilities;
#[path = "codex_model.rs"]
mod codex_model;
#[path = "placement.rs"]
pub(crate) mod placement;
#[cfg(test)]
#[path = "placement_tests.rs"]
mod placement_tests;
#[path = "remote_tasks.rs"]
mod remote_tasks;
#[path = "repositories.rs"]
pub(crate) mod repositories;
pub(crate) use remote_tasks::Preparation;
pub use remote_tasks::{
    PeerOrigin, RemoteState, RemoteTask, wait_for_peer_thread, warm_repository_records,
};
#[path = "runtime_poll.rs"]
mod runtime_poll;

/// Debug-only deterministic barrier for the queued-input integration test.
/// It blocks the provider slot in an isolated profile until the test creates
/// the release file. Production builds and ordinary profiles never see it.
#[cfg(debug_assertions)]
fn test_hold_injection_flush() -> bool {
    let Some(release) = std::env::var_os("SHINA_TEST_HOLD_INJECTION_FLUSH") else {
        return false;
    };
    let release = std::path::PathBuf::from(release);
    let held = release.starts_with(std::env::temp_dir())
        && release
            .parent()
            .and_then(|path| path.file_name())
            .is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("shina-server-integration-")
            })
        && !release.exists();
    held
}

/// Debug-only barrier for a peer input that has already crossed the failed
/// observation boundary. Ordinary task injections are not held by it.
#[cfg(debug_assertions)]
fn test_hold_peer_input_flush() -> bool {
    let Some(release) = std::env::var_os("SHINA_TEST_HOLD_PEER_INPUT") else {
        return false;
    };
    let release = std::path::PathBuf::from(release);
    release.starts_with(std::env::temp_dir())
        && release
            .parent()
            .and_then(|path| path.file_name())
            .is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("shina-server-integration-")
            })
        && !release.exists()
}

#[cfg(not(debug_assertions))]
fn test_hold_peer_input_flush() -> bool {
    false
}

struct Injection {
    text: String,
    shown: bool,
    /// Set for steering input: the run the caller observed. The flush uses Pi's
    /// steer behavior only while this is still the live run; a newer run means
    /// the steering input goes to the follow-up queue instead of interrupting
    /// work the caller never saw.
    steer: Option<String>,
    /// The peer command whose provider acceptance this injection can prove.
    peer_input: Option<uuid::Uuid>,
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

/// KSG `advertisedModel`: a slug or an alias match accepts the requested id.
fn advertised_delegation_model<'a>(
    models: &'a [AgentModel],
    provider: &str,
    model_id: &str,
) -> Option<&'a AgentModel> {
    models.iter().find(|model| {
        model.provider == provider
            && (model.id == model_id
                || model
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(model_id)))
    })
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
    OpenThread {
        server: String,
        thread_id: usize,
    },
    ReviewCaptainPromotion {
        server: String,
        thread_id: usize,
        profile: Captain,
        restore: Option<usize>,
    },
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

/// The thread selection an in-flight handoff replaced; restored if the destination
/// fails before the provider accepts the turn.
pub(crate) struct HandoffRevert {
    harness: String,
    session: Option<String>,
    selected_model: Option<(String, String)>,
    model_provider: String,
    model_id: String,
    model: String,
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
    input_text: String,
    folded_query: String,
    pub pending: bool,
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

#[allow(dead_code, reason = "The preview overlay lands in Slice B.")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewSource {
    #[default]
    Hover,
    Focus,
}

#[allow(dead_code, reason = "The preview overlay lands in Slice B.")]
#[derive(Default)]
pub struct SidebarPreview {
    pub target: Option<(String, usize)>,
    pub source: PreviewSource,
    pub row_hot: bool,
    pub card_hot: bool,
    pub muted_row: Option<(String, usize)>,
    pub opened_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPicker {
    Model,
    Thinking,
}

/// One in-flight gesture on the effort slider: where the thumb is held and which control
/// the gesture belongs to. `key` hashes the thread, model, option id and levels the press
/// began on, so a stale release after a thread, model or option change cannot commit a
/// level into a context the user never aimed at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThinkingDrag {
    pub frac: f32,
    pub key: u64,
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
    /// Pick the workspace for the New captain editor; the choice stays on the modal.
    CaptainWorkspace,
}

/// UI-only state for the captain Brief panel and the profile editor in `captain_ui.rs`.
/// Nothing here is persisted; committed values go through the State memory and scope APIs.
#[derive(Default)]
pub struct BriefUi {
    /// Expanded activity groups, scoped to their owning machine and Captain.
    pub activity_open: std::collections::BTreeSet<(String, usize, &'static str)>,
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
    /// The recruit palette, bound to the machine and captain it was opened for.
    pub recruit: Option<RecruitPicker>,
    /// The move/join picker for a thread.
    pub moving: Option<MovePicker>,
    /// A crew row's More menu: the member thread it was opened for.
    pub more: Option<usize>,
    /// The open lifecycle confirmation (archive, unarchive, demote or delete),
    /// bound to the machine and Captain it was opened for.
    pub lifecycle: Option<CaptainLifecycleDialog>,
    /// A retired Captain record (server, captain) opened read-only from the
    /// Archive's Former captains rows. UI-only identity; never persisted.
    pub record: Option<(String, usize)>,
    /// Demo scenes only: a stand-in for the live background status.
    pub demo_status: Option<background::BackgroundStatus>,
}

/// A Captain document draft's identity: server, Captain and one of the four document names.
/// Keys stay bound to the machine the draft was started on; they are never copied across hosts.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DocumentDraftKey {
    pub server: String,
    pub captain: usize,
    pub name: String,
}

#[derive(Clone, Debug)]
pub enum DraftState {
    Editing,
    Saving { command: uuid::Uuid },
    Rejected { message: String },
    Conflict { theirs_revision: u64 },
}

/// An open Captain document draft. `base_*` is the revision the user started from; `text` is
/// the user's edit. Only an exact accepted receipt removes a Saving draft.
pub struct DocumentDraft {
    pub base_revision: u64,
    pub base_text: String,
    pub text: TextEdit,
    pub state: DraftState,
    /// The projection revision whose text the user is previewing; None when the
    /// their-version view is closed. "Replace with mine" only fires on what was seen.
    pub show_theirs: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrewKind {
    Join,
    Release,
    Move { to: usize },
}

/// A crew change queued on a linked machine, awaiting its exact command receipt.
#[derive(Clone, Copy, Debug)]
pub struct CrewChange {
    pub captain: usize,
    pub thread: usize,
    pub kind: CrewKind,
}

/// The result of a user-invoked crew command: applied locally, or queued with a command id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrewMutation {
    Saved,
    Pending(uuid::Uuid),
}

/// A close operation parked behind the dirty-draft confirmation.
#[derive(Clone, Debug)]
pub struct DocDiscard {
    pub panel: String,
    /// The draft's own identity: confirming always drops this draft, on this server,
    /// whatever machine the user has selected since.
    pub key: DocumentDraftKey,
    pub kind: DiscardKind,
}

#[derive(Clone, Debug)]
pub enum DiscardKind {
    /// Drop the draft only: the Cancel footer and "Discard mine" leave the tab open.
    Discard,
    /// Close the viewer item: the panel stays open on the Diff segment.
    Close,
    /// Drop the draft and open a different document in its place.
    Replace { captain: usize, name: String },
}

/// What a move picker does on selection: move an existing member, or join a root thread.
#[derive(Clone, Copy, Debug)]
pub enum MoveMode {
    Move { from: usize },
    Join,
}

#[derive(Clone)]
pub struct RecruitPicker {
    pub server: String,
    pub captain: usize,
    pub query: TextEdit,
}

#[derive(Clone)]
pub struct MovePicker {
    pub server: String,
    pub thread: usize,
    pub mode: MoveMode,
    /// The element key the picker hangs from.
    pub anchor: String,
    /// Captured pointer position when opened from a context menu without a mounted row.
    pub at: Option<(f32, f32)>,
}

/// The Captain lifecycle actions the shared confirmation dialog can ask for.
/// `action` is the backend's preview/command verb; `method` is the wire command;
/// `capability` is the negotiated feature string that must be present to offer it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptainLifecycleAction {
    Archive,
    Unarchive,
    Demote,
    Delete,
}

impl CaptainLifecycleAction {
    pub fn action(&self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Unarchive => "unarchive",
            Self::Demote => "demote",
            Self::Delete => "delete",
        }
    }
    pub fn method(&self) -> &'static str {
        match self {
            Self::Archive => "captain.archive",
            Self::Unarchive => "captain.unarchive",
            Self::Demote => "captain.demote",
            Self::Delete => "captain.delete",
        }
    }
    pub fn capability(&self) -> &'static str {
        match self {
            Self::Archive | Self::Unarchive => crate::protocol::CAPTAIN_ARCHIVE,
            Self::Demote => crate::protocol::CAPTAIN_RETIRE,
            Self::Delete => crate::protocol::CAPTAIN_DELETE,
        }
    }
}

/// The shared Captain archive/unarchive/demote/delete confirmation, bound to the
/// machine and Captain it was opened for. `pending` is whichever command is in
/// flight: first the `captain.lifecycle.preview` read, then the mutation itself.
/// `preview` is the backend's own read shape (`captainVersion`, `crewSeen`,
/// `affectedThreads`, `blockers`, and the delete extras); the dialog renders from
/// it and the confirm sends its `captainVersion`/`crewSeen` back unchanged.
#[derive(Clone)]
pub struct CaptainLifecycleDialog {
    pub server: String,
    pub captain: usize,
    pub action: CaptainLifecycleAction,
    pub preview: Option<Value>,
    pub pending: Option<uuid::Uuid>,
    pub error: Option<String>,
    pub confirm_name: TextEdit,
    pub keep_conversations: bool,
}

impl CaptainLifecycleDialog {
    /// The backend refused the last confirmation because the Captain changed
    /// under the open dialog. The list must be re-reviewed before a fresh press.
    pub fn conflict(&self) -> bool {
        self.error
            .as_deref()
            .is_some_and(|error| error.starts_with("Captain changed since version"))
    }
}

/// A lifecycle mutation queued on a machine, awaiting its exact command receipt.
#[derive(Clone, Copy, Debug)]
pub struct LifecycleChange {
    pub captain: usize,
    pub action: CaptainLifecycleAction,
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
    /// The exact controller-issued lease currently attached to this executor
    /// thread. Numeric/string provider aliases remain unchanged beside it.
    pub execution: Option<crate::execution::Binding>,
    /// A peer assignment accepted for the next provider turn. `begin_prompt`
    /// consumes it so a provider process can serve several distinct runs.
    pub(crate) next_execution: Option<crate::execution::Binding>,
    /// Bind each durable peer input to its own controller-issued run. A single
    /// next_execution slot cannot represent several accepted follow-ups.
    pub(crate) peer_input_executions: std::collections::BTreeMap<String, crate::execution::Binding>,
    /// Inputs written to the provider while another run still owns the writer.
    /// A prompt acknowledgement is acceptance, not activation or settlement.
    pub(crate) peer_provider_inputs: std::collections::BTreeMap<String, crate::execution::Binding>,
    execution_acceptances: Vec<crate::execution::Binding>,
    /// Provider terminal observations survive changing the active binding
    /// within a poll batch, until refresh_execution commits them to the book.
    execution_settlements: Vec<(crate::execution::Binding, String)>,
    /// Durable provider evidence, matched by run id so a late event cannot
    /// settle a newer run.
    pub provider_accepted_run: Option<String>,
    pub provider_running_run: Option<String>,
    pub provider_terminal_run: Option<String>,
    pub provider_terminal_status: Option<String>,
    pub resumable_run_id: Option<String>,
    pub continuation_of_run_id: Option<String>,
    pub prompt_suggestion: Option<PromptSuggestion>,
    pub(crate) prompt_suggestions_enabled: bool,
    pub stop_control: StopControl,
    run_control_supported: bool,
    resume_command: Option<uuid::Uuid>,
    /// Device-owned draft, promoted on Send rather than while choosing a host or project.
    pub client_draft: bool,
    /// Client-only binding for an atomic first input into an independent crew root.
    pub crew_draft: Option<CrewDraftTarget>,
    pub crew_draft_notice: Option<String>,
    pub(crate) client_creating: Option<uuid::Uuid>,
    pub(crate) client_promoted: Option<usize>,
    pub remote: Option<crate::client::Commands>,
    pub(crate) remote_send: Option<(uuid::Uuid, crate::composer_rich::Snapshot)>,
    pub(crate) remote_send_fresh_arm: Option<u64>,
    pub(crate) remote_send_reply_to: Option<crate::composer_context::ComposerContextRecord>,
    pub fresh_session_arm: Option<u64>,
    pub fresh_session_messages: Vec<String>,
    /// The next controller prompt starts a new provider session after a linked turn.
    pub(super) captain_return_context: bool,
    /// Provider handoff records in order. KSG: `thread.contextHandoffs`.
    pub handoffs: Vec<crate::handoff::Handoff>,
    /// Retained provider session ids per harness, so switching back resumes natively.
    pub handoff_sessions: std::collections::BTreeMap<String, String>,
    /// The staged switch (a pending record) awaiting the next send.
    pub(crate) staged_handoff: Option<usize>,
    /// The record currently delivering its package; cleared on acceptance or failure.
    pub(crate) inflight_handoff: Option<usize>,
    /// Selection snapshot to restore when the destination fails.
    pub(crate) handoff_revert: Option<HandoffRevert>,
    /// The package text delivered with the in-flight turn.
    pub(crate) handoff_package: Option<String>,
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
    /// Project-less role changes retain their original working directory.
    pub(crate) scratch_directory: Option<String>,
    /// The worker placement reservation and resolved checkout (#20).
    pub placement: Option<placement::ThreadPlacement>,
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
    /// New turns wait while this machine replaces their provider executable.
    provider_update_held: bool,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub(crate) agent_index: crate::agents::AgentIndex,
    pub(crate) projected_agent_counts: Option<(usize, usize)>,
    pub thoughts: Vec<Thought>,
    pub work: Vec<AssistantWork>,
    /// Item lists keyed by the last applied projection's stable source ids.
    /// The next apply reuses unchanged items positionally or by id; it never
    /// rebuilds items that did not change.
    pub(crate) transcript_fields:
        std::collections::HashMap<&'static str, std::sync::Arc<crate::client_history::FieldItems>>,
    /// Last successfully applied source, not the network's previous emission.
    pub(crate) transcript_projection:
        Option<std::sync::Arc<crate::client_history::ThreadProjection>>,
    /// Disclosure keys survive unloaded history without retaining payloads.
    pub(crate) transcript_disclosures: client_state::Disclosures,
    /// Structure and content revisions of the last applied emission; the
    /// timeline consumer (#25) diffs these instead of rescanning items.
    pub(crate) transcript_structure: u64,
    pub(crate) transcript_content: u64,
    /// Derived row keys and event indexes. No transcript payloads are retained.
    pub(crate) timeline_cache: std::cell::RefCell<crate::ui::TimelineCache>,
    /// Whether earlier work history in the running turn is folded behind its `Working for` bar.
    pub live_work_collapsed: bool,
    pub draft: TextEdit,
    pub queued_edit: Option<QueuedEdit>,
    queued_save: Option<(uuid::Uuid, uuid::Uuid)>,
    queue_edit_supported: bool,
    /// Images selected for the next send, kept separate from text undo history.
    pub prompt_history: crate::composer::History,
    pub typing_guard: crate::composer::TypingGuard,
    pub paste_history: crate::composer_paste::PasteHistory,
    pub composer_rules: crate::composer_rich::Rules,
    pub composer_source: crate::composer_source::MarkdownSourceState,
    /// Per-block syntax-colour state for the draft's code fences (KSG's
    /// `composerCodeBlockHighlight` plugin state), owned per thread.
    pub code_highlight: std::cell::RefCell<crate::composer_code_highlight::ComposerCodeHighlight>,
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
    /// A held `captain_document_read` waiting on the launch origin's reply.
    /// Projected into snapshots so `peer.thread.wait` can carry it home.
    pub(crate) peer_document_read: Option<remote_tasks::PeerDocumentRead>,
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
    /// Working-shelf state: the two-second linger after a run ends and the return stamp
    /// that orders the thread back into the inbox, as KSG's `inboxReturnTracker` does.
    pub working: Working,
    pub started: Option<Instant>,
    pub elapsed: Option<u64>,
    pub captain: Option<usize>,
    pub captain_home: bool,
    pub crew_captain: Option<usize>,
    pub captain_context: String,
    /// The hashed inputs `captain_context` was last built from. `None` forces
    /// a rebuild on the next refresh; a matching hash skips it so unchanged
    /// threads keep their block and their pending-instruction state.
    pub(super) captain_context_inputs: Option<u64>,
    /// The role whose Shina awareness this session received or will receive.
    pub(crate) instruction_role: Option<&'static str>,
    pub(crate) shina: &'static str,
    /// A role change that needs a next-turn update or a fresh session.
    pub(crate) instruction_context_pending: bool,
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
    startup_cursor_option: usize,
    pending_cursor_option: Option<String>,
    pub(crate) cursor_catalog_models: Option<Vec<AgentModel>>,
    /// Live option descriptors reported by the current ACP session handshake,
    /// kept separate from fallback cached model rows.
    live_option_descriptors: Option<Vec<crate::descriptors::ProviderOptionDescriptor>>,
    startup_selection_check: Option<String>,
    startup_selection_verified: bool,
    context_startup_requested: bool,
    queued_prompt: Option<(Prompt, usize)>,
    retry_prompt: Option<Prompt>,
    pending_thinking: Option<String>,
    pending_prompt: Option<(String, Prompt, usize)>,
    pending_injections: VecDeque<Injection>,
    injection_id: Option<String>,
    /// Delivery markers whose prompt command reached the provider, with the
    /// behavior actually used. Consumed into the durable operation ledger.
    pub(super) injection_dispatches: Vec<(String, String)>,
    /// Provider rejections for exact internal delivery ids. This is separate
    /// from `accepted_deliveries`; a rejected prompt is never retried blindly.
    pub(super) rejected_injections: Vec<(String, String)>,
    /// The provider prompt command currently proving a peer input's acceptance.
    pub(super) peer_prompt: Option<(String, uuid::Uuid)>,
    /// A restored peer input whose absence must be proven by authoritative
    /// provider history before it can be dispatched again.
    pub(super) peer_recovery_held: bool,
    /// Exact input digests read from a successful provider history response.
    /// Optimistic local transcript rows never enter this transient set.
    pub(super) peer_input_history: Option<std::collections::HashSet<[u8; 32]>>,
    /// Peer input commands actually written to the adapter, with the realized
    /// delivery and run. Absence here never proves the provider did not read it.
    pub(super) peer_dispatches: Vec<(String, String, Option<String>)>,
    injection_retry_at: Option<Instant>,
    operation_deadline: Option<(&'static str, Instant)>,
    task_result: Option<&'static str>,
    completion_notified: bool,
    forwarded_question: Option<String>,
    accepted_deliveries: Vec<String>,
    crew_report_run: Option<crew_reports::CrewRun>,
    crew_report_policy: ReportPolicy,
    crew_report_outbox: Vec<crew_reports::CrewReport>,
    crew_report_last_run: Option<String>,
    recovering_task: bool,
    task_history: Option<Value>,
    task_delivery_probe_started: bool,
    task_result_history: Option<std::collections::HashSet<String>>,
    /// The live admission reservation id this crew member or peer worker
    /// holds, and the monotonic run counter its reservations key to.
    pub admission: Option<String>,
    pub admission_run: u64,
    /// Runtime-only mirror of the ledger: true while a covering reservation
    /// is queued (or a managed thread waits for lazy admission). Delivery
    /// loops consult it so queued managed work cannot reach a live provider
    /// connection ahead of its grant; unmanaged threads never set it.
    pub admission_blocked: bool,
    memory: Option<memory::MemoryLink>,
    learned_turn: Option<honcho::Turn>,
    memory_pending: VecDeque<mpsc::Receiver<Value>>,
    memory_context: std::collections::BTreeMap<String, String>,
    memory_staged: Vec<memory::Mutation>,
    memory_suggestions: Vec<String>,
    scope_context_pending: bool,
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
    reports: Vec<crew_reports::CrewReport>,
    answers: Vec<crew_questions::CrewAnswer>,
    events: Vec<captain_inbox::InboxEvent>,
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

/// Whether the named thread still exists: on the selected machine the live
/// `threads` list answers, anywhere else the machine's projected index.
fn thread_exists_on(
    threads: &[Thread],
    machines: &crate::peer::Machines,
    server: &str,
    id: usize,
) -> bool {
    if server == machines.selected {
        threads.iter().any(|t| t.id == id)
    } else {
        machines.views.get(server).is_some_and(|projection| {
            projection["index"]["threads"]
                .as_array()
                .is_some_and(|threads| {
                    threads
                        .iter()
                        .any(|thread| thread["id"].as_u64() == Some(id as u64))
                })
        })
    }
}

/// The threads the user opened, for Back and Forward (`navigation.back`,
/// `navigation.forward`). Entries are `(machine, thread id)` so a client
/// machine's rows take part too. Session state only; nothing is written to the
/// index.
#[derive(Clone, Debug, Default)]
pub struct NavHistory {
    entries: Vec<(String, usize)>,
    cursor: usize,
}

/// The history never grows past this: 64 opens is far past what Back is for.
const NAV_HISTORY_CAP: usize = 64;

impl NavHistory {
    /// Record an open. A repeat of the current entry is a no-op; a new open
    /// after going back drops every forward entry, as a browser does.
    pub fn push(&mut self, machine: &str, id: usize) {
        if self.entries.get(self.cursor) == Some(&(machine.to_string(), id)) {
            return;
        }
        self.entries.truncate(self.cursor + 1);
        self.entries.push((machine.to_string(), id));
        if self.entries.len() > NAV_HISTORY_CAP {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }
    /// The entry Back would open.
    pub fn back(&self) -> Option<&(String, usize)> {
        self.cursor.checked_sub(1).and_then(|i| self.entries.get(i))
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// The entry Forward would open.
    pub fn forward(&self) -> Option<&(String, usize)> {
        self.entries.get(self.cursor + 1)
    }
    /// Move the cursor one step. The caller checks `back`/`forward` first, so
    /// `remove` below can prune a dead entry before this moves.
    pub fn step_back(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
    pub fn step_forward(&mut self) {
        self.cursor = (self.cursor + 1).min(self.entries.len().saturating_sub(1));
    }
    /// Drop every entry naming a thread that no longer exists, keeping the
    /// cursor on the same entry where it can. Returns whether anything left.
    pub fn retain(&mut self, mut exists: impl FnMut(&str, usize) -> bool) {
        let old_cursor = self.cursor;
        let mut next = Vec::with_capacity(self.entries.len());
        let mut new_cursor = None;
        for (idx, entry) in self.entries.drain(..).enumerate() {
            if exists(entry.0.as_str(), entry.1) {
                if idx <= old_cursor {
                    new_cursor = Some(next.len());
                }
                next.push(entry);
            }
        }
        self.entries = next;
        if self.entries.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor = new_cursor.unwrap_or(0).min(self.entries.len() - 1);
        }
    }
    /// The most recent entries first, for the quick switcher's "Recent" list.
    pub fn recent(&self) -> impl Iterator<Item = &(String, usize)> {
        self.entries.iter().rev()
    }
}

pub(crate) struct ExecutionBaseline {
    runs: std::collections::BTreeMap<usize, Option<String>>,
    tasks: std::collections::BTreeMap<String, (uuid::Uuid, Option<uuid::Uuid>, Option<usize>)>,
}

pub struct State {
    /// None means a command/global change. A poll records only changed threads.
    pub(crate) persistence_scope: Option<std::collections::HashSet<usize>>,
    pub(crate) execution_command_device: Option<String>,
    /// The durable admission scheduler shared by delegated tasks, crew
    /// members and peer-launched workers. Persisted with the index.
    pub(crate) scheduler: admission::Scheduler,
    pub(crate) provider_update_drivers: std::collections::BTreeSet<String>,
    pub client: Option<crate::client::Client>,
    device_ui_dirty: bool,
    device_draft_cache: client_state::DeviceDraftCache,
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
    projected_native_catalogs: std::collections::BTreeMap<String, codex_model::NativeModelMetadata>,
    /// Which emitted thread records the last projections actually applied.
    pub(crate) apply_stats: client_state::ApplyStats,
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
    pub attachment_uploads: crate::attachment_uploads::Queue,
    pub data_dir: PathBuf,
    /// The open thread's id and whether it was a draft with content when it was opened. Like
    /// T3 Code's frozen draft row, the open draft's sidebar row neither pops in while typing nor
    /// disappears when its composer is cleared.
    opened_draft: Option<(usize, bool)>,
    launch_project: Option<PathBuf>,
    pub threads: Vec<Thread>,
    pub active: usize,
    pub tabs: Vec<usize>,
    /// The opt-in tab strip above the conversation (Settings, Appearance, "Thread
    /// tabs"). Off, `tabs` holds only the open thread and the title bar names it.
    pub tabs_enabled: bool,
    /// The provisional tab: sidebar clicks replace it in place instead of
    /// piling up, like KSG Code's ghost tab. At most one tab is a ghost, and
    /// `ghost_anchor` names the captain tab a worker ghost returns to on close.
    pub ghost_tab: Option<usize>,
    pub ghost_anchor: Option<usize>,
    /// Back and forward through the threads the user opened, as a browser's
    /// history (`mod+[` and `mod+]`). Not saved to disk.
    pub nav_history: NavHistory,
    /// Set while `nav_back`/`nav_forward` opens an entry, so the open itself
    /// does not push a fresh entry and drop the forward list.
    nav_open: bool,
    pub next_id: usize,
    pub dark: bool,
    /// Whole-window zoom, changed with Ctrl+= / Ctrl+- and reset with Ctrl+0.
    pub zoom: f32,
    /// The open thread's "More actions" menu in the chat header.
    pub thread_menu: bool,
    pub row_menu: Option<RowTarget>,
    /// A tab's context menu: the tab it names and the pointer position.
    pub tab_menu: Option<usize>,
    pub tab_menu_at: Option<(f32, f32)>,
    /// The footer's settings menu is open.
    pub settings_menu: bool,
    pub settings: crate::ui::settings_ui::Dialog,
    pub preferences: crate::ui::settings_ui::Preferences,
    pub keybindings_watcher: crate::keybindings::KeybindingsWatcher,
    pub thread_defaults: crate::ui::settings_ui::ThreadDefaults,
    pub fleet: crate::fleet_update::Fleet,
    /// Where a right-click opened the row menu, in window points.
    pub row_menu_at: Option<(f32, f32)>,
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
    /// Whether the Working, Snoozed and Settled shelves are open.
    pub shelves: [bool; 3],
    pub settled_visible: usize,
    /// Which request the unified composer request banner is showing, clamped to
    /// the live request count each frame (spec §3.6).
    pub needs_you_page: usize,
    /// Whether the dismissible Now / crew & subagents popover above the composer is open.
    pub undo: Option<UndoNotice>,
    pub toasts: Vec<ToastNotice>,
    next_toast_id: u64,
    pub composer_notices: crate::composer_notices::Store,
    fresh_arm_seq: u64,
    keep_full_history_threads: std::collections::HashSet<(String, usize)>,
    pub plan_menu: Option<String>,
    pub plan_copied: Option<(String, u64)>,
    pub plan_expanded: std::collections::HashSet<String>,
    /// Code block identities where the user toggled soft line wrapping off (default is wrapped).
    pub unwrapped_code_blocks: std::sync::Arc<std::collections::HashSet<String>>,
    /// Bumped when any code block's soft-wrap state changes so memoized markdown rebuilds.
    pub code_wrap_version: u64,
    pub plan_save_dialog: Option<PlanSaveDialog>,
    pub plan_save_last_path: Option<String>,
    next_plan_save_request_id: u64,
    pub snooze_menu: Option<(usize, Vec<SnoozeChoice>)>,
    pub snooze_anchor: Option<String>,
    pub snooze_at: Option<(f32, f32)>,
    pub snooze_targets: Vec<usize>,
    pub custom_snooze_open: bool,
    pub snooze_edit: TextEdit,
    /// The composer prompt stash, KSG Code's promptStashStore. `composer-stash.json`.
    pub stash: crate::ui::composer_stash::Stash,
    pub stash_menu: bool,
    pub stash_highlight: usize,
    /// Bumped once per stash so the badge replays its pulse, KSG's `pulseKey`.
    pub stash_pulse: u64,
    pub lifecycle_error: Option<String>,
    pub auto_settle_days: u32,
    pub default_harness: String,
    pub drag_thread: Option<usize>,
    pub drag_target: Option<(Section, Option<usize>)>,
    pub drop_rects: std::collections::HashMap<String, [f32; 4]>,
    pub viewport_height: f32,
    pub viewport_width: f32,
    pub sidebar_width: f32,
    /// The right panel's width, one app-wide value saved in the UI snapshot.
    pub right_panel_width: f32,
    /// Per-thread right-panel state: surfaces, open and maximized. Session only; only the
    /// width is saved.
    pub right_panels: crate::right_panel::RightPanels,
    /// The resize handle is mid-drag: the edge line turns coral.
    pub right_panel_resizing: bool,
    /// Per-thread diff panel state, keyed like right_panels. Session only.
    pub(crate) diffs: crate::ui::diff_panel::DiffCache,
    pub diff_blockers: std::collections::HashMap<String, crate::review::DiffBlocker>,
    pub diff_details: Option<String>,
    pub session_notices: std::collections::HashSet<String>,
    /// App-wide "hide test files" switch for the diff panel; saved in the UI snapshot
    /// only when on.
    pub diff_hide_tests: bool,
    /// KSG `fileTreeOpen`: whether the diff's file tree column shows. App-wide,
    /// saved in the UI snapshot only when on — KSG's default is hidden.
    pub diff_tree_open: bool,
    /// KSG's persisted explorer-pane width; `None` tracks its 40%/256 px default.
    pub diff_tree_width: Option<f32>,
    /// The tree lane's resize handle is mid-drag: its edge line turns coral.
    pub diff_tree_resizing: bool,
    /// Diff request ids; a newer request supersedes an older one.
    pub(crate) next_diff_request_id: u64,
    /// In-flight review.getDiffPreview RPCs: request id -> what it is for.
    pub(crate) pending_diff_requests:
        std::collections::HashMap<u64, crate::ui::diff_panel::DiffPending>,
    /// Finished diff work (local results and parsed remote answers) for the poll drain.
    pub(crate) diff_rxs: Vec<std::sync::mpsc::Receiver<crate::ui::diff_panel::DiffMsg>>,
    pub ui_store: Option<crate::store::Store>,
    last_sweep: u64,
    last_lifecycle_clock: u64,
    pub project_picker: Option<ProjectPicker>,
    /// The workspace picker's search text and the row the arrow keys have reached.
    pub picker_query: TextEdit,
    pub picker_highlight: usize,
    /// The Add workspace palette is browsing folders; the query holds the path.
    pub picker_browsing: bool,
    /// The Cmd+K quick switcher: open or not, its query, and the lit row.
    pub quick_switcher: bool,
    pub quick_switcher_query: TextEdit,
    pub quick_switcher_highlight: usize,
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
    pub(crate) attachment_video_posters:
        std::cell::RefCell<std::collections::HashMap<String, (rune::Player, u64)>>,
    pub(crate) attachment_poster_clock: std::cell::Cell<u64>,
    pub html_windows: Arc<crate::html_window::Windows>,
    pub html_lane_width: u32,
    /// The composer is showing a valid drop highlight.
    pub attachment_drop_hover: bool,
    /// The assistant message whose Copy button last copied, and when, for its check feedback.
    pub copied_message: Option<(String, std::time::Instant)>,
    /// Per-turn changed-files card folder state, keyed `changed-files-{message key}`.
    /// Session only, as KSG's `directoryExpansionState` is.
    pub(crate) changed_files: std::collections::HashMap<String, crate::ui::changed_files::CardUi>,
    pub(crate) transcript_preview: Option<crate::ui::transcript_context::Preview>,
    pub(crate) attachment_preview: Option<crate::ui::attachment_preview::Preview>,
    pasted_text_names: Option<(String, usize, std::collections::HashSet<String>)>,
    pub(crate) transcript_layout: std::collections::HashMap<String, (bool, f32)>,
    pub(crate) assistant_selection: Option<AssistantSelection>,
    pub(crate) citation_highlight: Option<CitationHighlight>,
    /// The open citation comment popover, if any (`citation_comment::Editing`).
    pub(crate) citation_comment: Option<crate::ui::citation_comment::Editing>,
    pub composer_ui: crate::ui::composer_ui::Behavior,
    pub captain_picker: bool,
    pub captain_picker_query: TextEdit,
    pub captain_picker_highlight: usize,
    pub env_mode_picker: bool,
    pub base_branch_picker: bool,
    pub base_branch_query: TextEdit,
    /// The per-code-block language picker, KSG Code's ComposerCodeBlockLanguagePicker.
    pub code_picker: crate::ui::composer_code_picker::Picker,
    pub(crate) vcs_branches: std::collections::BTreeMap<(String, String), VcsBranchCacheEntry>,
    vcs_branch_rxs: Vec<std::sync::mpsc::Receiver<VcsBranchResultMsg>>,
    worktree_bootstrap_rxs: Vec<std::sync::mpsc::Receiver<WorktreeBootstrapResult>>,
    worktree_setup_rxs: Vec<std::sync::mpsc::Receiver<WorktreeSetupResultMsg>>,
    placement_rxs: Vec<(
        usize,
        std::sync::mpsc::Receiver<Result<placement::ProvisionedPlacement, String>>,
    )>,
    checkout_rxs: Vec<(usize, std::sync::mpsc::Receiver<Result<PathBuf, String>>)>,
    multi_model_batches: Vec<MultiModelBatch>,
    #[cfg(test)]
    pub(crate) test_git_context: Option<crate::vcs::GitContext>,
    #[cfg(test)]
    pub(crate) test_connect_workspaces: Vec<(usize, PathBuf)>,
    /// (thread id, harness, session id at connect): `None` means a new session started.
    #[cfg(test)]
    pub(crate) test_connect_sessions: Vec<(usize, String, Option<String>)>,
    #[cfg(test)]
    pub(crate) test_connect_pi:
        Option<Box<dyn FnMut(usize, &std::path::Path) -> Option<Pi> + Send>>,
    /// When set, connect_thread fails with this error instead of starting a session.
    #[cfg(test)]
    pub(crate) test_connect_error: Option<String>,
    next_vcs_request_id: u64,
    pending_vcs_requests: std::collections::HashMap<u64, (String, String)>,
    pub captain_project: Option<Option<ProjectId>>,
    pub captains: Vec<Captain>,
    captain_scopes: std::collections::BTreeMap<usize, Vec<ProjectId>>,
    captain_remote_scopes: std::collections::BTreeMap<usize, Vec<RemoteProject>>,
    /// The logical repository catalog: durable repository identity separate
    /// from machine-local checkout paths.
    pub repositories: Vec<repositories::LogicalRepository>,
    repository_bindings: Vec<repositories::CheckoutBinding>,
    captain_repository_scopes: std::collections::BTreeMap<usize, Vec<String>>,
    captain_homes: std::collections::BTreeMap<usize, usize>,
    captain_books: std::collections::BTreeMap<usize, captain_state::CaptainBook>,
    captain_lifecycles: std::collections::BTreeMap<usize, captain_lifecycle::CaptainLifecycle>,
    captain_report_policies: std::collections::BTreeMap<usize, ReportPolicy>,
    captain_reports: Vec<crew_reports::CrewReport>,
    captain_question_answers: Vec<crew_questions::CrewAnswer>,
    captain_recovery_modes: std::collections::BTreeMap<usize, captain_inbox::RecoveryMode>,
    captain_inbox: Vec<captain_inbox::InboxEvent>,
    captain_waits: Vec<captain_inbox::InboxWait>,
    /// In-memory Captain document drafts, keyed by server, captain and document name. Never
    /// persisted: an unsaved draft quits with the app (separate follow-up).
    pub captain_drafts: std::collections::BTreeMap<DocumentDraftKey, DocumentDraft>,
    /// Crew changes queued on linked machines, awaiting their exact command receipts.
    pub crew_pending: std::collections::BTreeMap<(String, uuid::Uuid), CrewChange>,
    /// Lifecycle mutations queued on a machine, awaiting their exact command
    /// receipts. Entries stay Waiting across a link drop; nothing retries them.
    pub lifecycle_pending: std::collections::BTreeMap<(String, uuid::Uuid), LifecycleChange>,
    pub promotion_pending: std::collections::BTreeMap<(String, uuid::Uuid), PromotionChange>,
    pub captain_policy_pending:
        std::collections::BTreeMap<(String, uuid::Uuid), CaptainPolicyChange>,
    pub captain_policy_errors: std::collections::BTreeMap<(String, usize), String>,
    captain_recovery_pending: std::collections::BTreeMap<(String, uuid::Uuid), usize>,
    pub captain_recovery_errors: std::collections::BTreeMap<(String, usize), String>,
    pub crew_question_drafts: std::collections::BTreeMap<(String, String), CrewQuestionDraft>,
    pub crew_questions_captain_open: bool,
    /// A dirty-draft close parked behind the discard confirmation.
    pub doc_discard: Option<DocDiscard>,
    /// The audience the Context tab's document and memory figures describe:
    /// `true` previews what crew threads receive. UI-only, never persisted.
    pub context_audience_crew: bool,
    /// The Context tab's "N more, found by search" fold is open. UI-only.
    pub context_memories_expanded: bool,
    /// The Context tab's "Earlier conversations" fold is open. UI-only.
    pub context_earlier_expanded: bool,
    /// Crew change rejections per machine: an error from one host must not render on another.
    pub crew_errors: std::collections::BTreeMap<String, String>,
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
    /// (highest), and which gesture context it belongs to. The level is chosen on release.
    pub thinking_drag: Option<ThinkingDrag>,
    pub model_search: TextEdit,
    pub thread_find: ThreadFindState,
    pub sidebar_search: SidebarSearchState,
    pub(crate) sidebar_index: std::cell::RefCell<crate::ui::sidebar_ui::SidebarIndex>,
    pub(crate) sidebar_preview: crate::ui::sidebar_preview::SidebarPreview,
    #[allow(dead_code, reason = "Read by the Slice B preview overlay.")]
    pub preview: SidebarPreview,
    pub flash_tool: Option<(String, Instant)>,
    pending_tool_reveal: Option<String>,
    #[allow(dead_code, reason = "Read by the Slice B Agents panel.")]
    pub agents_filter: crate::agents::AgentsFilter,
    #[allow(dead_code, reason = "Read by the Slice B Agents panel.")]
    pub agents_expanded:
        std::collections::HashSet<(String, usize, String, crate::agents::AgentRunId)>,
    /// The floating thread details card over the chat pane.
    pub details: DetailsCard,
    pub mention_search: crate::ui::composer_menu::MentionSearch,
    /// Sequence for `mention:preview:{seq}` workspace.readFile calls.
    pub(crate) file_preview_seq: u64,
    pub pending_open_urls: Vec<String>,
    pub next_usage_limits_request_id: u64,
    pub pending_usage_limits_request: Option<(u64, String, usize, String)>,
    pub usage_limits_notice_key: Option<(String, usize, String)>,
    pub timeline_far_from_bottom: std::collections::HashMap<(String, usize), bool>,
    pub timeline_at_bottom: std::collections::HashMap<(String, usize), bool>,
    pub timeline_positions: std::cell::RefCell<crate::timeline_position::Positions>,
    pub updater: crate::update::Updater,
    pub update_popover: bool,
    pub update_details: bool,
    /// Set by "Restart to update". The restart only happens on a quit that saves cleanly.
    pub restart_for_update: bool,
    /// The explicit restart has saved and recorded its update for the pending quit.
    pub restart_approved: bool,
    /// Why the last restart click waited. Cleared while the update row shows ready.
    pub restart_notice: Option<String>,
    task_waiters: Vec<TaskWaiter>,
    tasks: Vec<Task>,
    /// Editable task plans: draft nodes, revisions, dependency edges, audited
    /// overrides and one-time launch records. Persisted with the index.
    pub(crate) task_plans: Vec<task_plan::TaskPlan>,
    pub(crate) identity: crate::identity::Catalog,
    pub(crate) execution: crate::execution::Book,
    /// Keyed control evidence, separate from the immutable saved receipts.
    pub(crate) task_operations: Vec<tasks::TaskOperation>,
    /// Target-side accepted input and stop intent. The database is canonical;
    /// this in-memory copy drives queue restoration and provider evidence.
    pub(crate) peer_task_intents: Vec<remote_tasks::PeerTaskIntent>,
    pub recovery_notice: Option<String>,
    run_marker: Option<crate::store::RunMarker>,
    /// Recent projects, the switcher menu, and the clone sheet.
    pub projects: crate::projects::Projects,
    pub catalog: Catalog,
    pub codex_catalog: crate::session::NativeCatalog,
    pub claude_catalog: crate::session::NativeCatalog,
    pub antigravity_catalog: crate::session::NativeCatalog,
    pub cursor_catalog: crate::session::NativeCatalog,
    pub(crate) acp_catalogs: std::collections::BTreeMap<String, crate::session::NativeCatalog>,
    /// KSG `refreshWorkspaceSnapshot` work in flight: a folder's own provider
    /// config is probed off the picker's catalog so a delegation retry can see
    /// the models that workspace advertises.
    pub(crate) workspace_model_scans:
        std::collections::BTreeMap<(String, PathBuf), crate::session::NativeCatalog>,
    pub harnesses: crate::harness::Registry,
    provider_availability: crate::harness::AvailabilityProbe,
    refresh_optional_catalogs: bool,
    pub managed_policy: Option<ManagedPolicy>,
    pub(crate) runtime_enabled: bool,
    /// True only when this reducer runs under the durable server batch (or a
    /// test explicitly simulates that profile). A live file-only reducer
    /// leaves it false and refuses keyed controls before any effect.
    pub(crate) durable_task_controls: bool,
    /// Set when peer dispatch runs before the runtime opens its next database
    /// batch. The next reconcile stages these exact operations in that batch.
    pub(super) task_operations_dirty: bool,
    pub(super) peer_task_intents_dirty: bool,
    ownership_denied: bool,
    pub runtime_stop_requested: bool,
    // Drop the owner only after every store has flushed and stopped its writer.
    owner: Option<crate::server_lock::Owner>,
}

#[path = "model/attachments_state.rs"]
mod attachments_state;
#[path = "model/captain_ops.rs"]
mod captain_ops;
#[path = "model/delegation.rs"]
mod delegation;
#[path = "model/execution_state.rs"]
mod execution_state;
#[path = "model/lifecycle_state.rs"]
mod lifecycle_state;
#[path = "model/navigation.rs"]
mod navigation;
#[path = "model/notices_and_send.rs"]
mod notices_and_send;
#[path = "model/persistence.rs"]
mod persistence;
#[path = "model/poll.rs"]
mod poll;
#[path = "model/search_citations.rs"]
mod search_citations;
#[path = "model/sidebar_state.rs"]
mod sidebar_state;
#[path = "model/thread.rs"]
mod thread;
#[path = "model/thread_events.rs"]
mod thread_events;
#[path = "model/util.rs"]
mod util;
#[path = "model/worktrees_and_multi.rs"]
mod worktrees_and_multi;

pub(crate) use delegation::parse_delegation_target;
pub use poll::{is_clear_command, is_compact_command, poll};
pub(crate) use util::status_word;
pub use util::{ZOOM_STEPS, snap_zoom, string, truncate};
use util::{content_images, content_text, selection_value, tool_detail};

#[cfg(test)]
#[path = "model/tests.rs"]
mod tests;
