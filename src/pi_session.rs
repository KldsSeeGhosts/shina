//! Pi-native operations ported from KSG Code's provider-pi adapter and commands modules.
//! This is a child of model so transport state stays with the thread that owns the process.
#[cfg(test)]
#[path = "pi_session_tests.rs"]
mod tests;
use super::*;
use std::collections::{HashMap, HashSet};
pub const TURN_CAPABILITIES: TurnCapabilities = TurnCapabilities::new(true, false);
#[path = "questionnaire.rs"]
pub mod questionnaire;

/// Rebuild the active branch, never append order, and keep pre-compaction transcript history.
/// Pi remains the only writer of its session files.
pub fn session_history(record: &mut Value) {
    if record["type"] != "response" || record["command"] != "get_entries" {
        return;
    }
    record["command"] = json!("get_messages");
    if record["success"] == false {
        return;
    }
    let Some(entries) = record["data"]["entries"].as_array() else {
        record["success"] = json!(false);
        record["error"] = json!("Pi returned invalid session entries.");
        return;
    };
    let by_id: HashMap<_, _> = entries
        .iter()
        .filter_map(|e| Some((e["id"].as_str()?, e)))
        .collect();
    if by_id.len() != entries.len()
        || !matches!(
            record["data"].get("leafId"),
            Some(Value::Null | Value::String(_))
        )
    {
        record["success"] = json!(false);
        record["error"] = json!("Pi returned invalid or duplicate session entry IDs.");
        return;
    }
    let mut cursor = record["data"]["leafId"].as_str();
    let mut seen = HashSet::new();
    let mut branch = vec![];
    while let Some(id) = cursor {
        let Some(entry) = by_id.get(id).filter(|_| seen.insert(id)) else {
            record["success"] = json!(false);
            record["error"] = json!("Pi's active session branch is incomplete or cyclic.");
            return;
        };
        if !matches!(entry.get("parentId"), Some(Value::Null | Value::String(_))) {
            record["success"] = json!(false);
            record["error"] = json!("Pi returned an invalid session parent ID.");
            return;
        }
        branch.push(*entry);
        cursor = entry["parentId"].as_str();
    }
    branch.reverse();
    let messages: Vec<_> = branch.iter().filter(|e| e["type"] == "message" || e["type"] == "custom_message")
        .map(|e| if e["type"] == "custom_message" { json!({"role":"custom","content":e["content"],"display":e["display"],"customType":e["customType"]}) } else { e["message"].clone() }).collect();
    let compactions: Vec<_> = branch
        .iter()
        .enumerate()
        .filter(|(_, e)| e["type"] == "compaction")
        .map(|(index, e)| {
            let mut value = (*e).clone();
            value["historyIndex"] = json!(
                branch[..index]
                    .iter()
                    .filter(|e| e["type"] == "message" || e["type"] == "custom_message")
                    .count()
            );
            value
        })
        .collect();
    let fork_messages: Vec<_> = branch.iter().filter(|e| e["type"] == "message" && e["message"]["role"] == "user")
        .map(|e| json!({"entryId":e["id"],"text":content_text(&e["message"]["content"]),"content":e["message"]["content"]})).collect();
    record["data"] =
        json!({"messages":messages,"compactions":compactions,"forkMessages":fork_messages});
}

fn approval_key(request: &Value) -> String {
    let title = string(request, "title");
    format!("{}:{}{}", title.len(), title, string(request, "message"))
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationMode {
    Fork,
    Rewind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessMode {
    ReadOnly,
    Supervised,
    Edits,
    Auto,
    #[default]
    Full,
}
impl AccessMode {
    pub const ALL: [Self; 5] = [
        Self::ReadOnly,
        Self::Supervised,
        Self::Edits,
        Self::Auto,
        Self::Full,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Supervised => "approval-required",
            Self::Edits => "auto-accept-edits",
            Self::Auto => "auto",
            Self::Full => "full-access",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read only",
            Self::Supervised => "Supervised",
            Self::Edits => "Auto-accept edits",
            Self::Auto => "Auto",
            Self::Full => "Full access",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read files and coordinate. Delegate code changes to a thread.",
            Self::Supervised => "Ask before commands and file changes.",
            Self::Edits => "Auto-approve edits, ask before other actions.",
            Self::Auto => "Supported providers approve routine actions; others still ask.",
            Self::Full => "Allow commands and edits without prompts.",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read-only" => Some(Self::ReadOnly),
            "approval-required" => Some(Self::Supervised),
            "auto-accept-edits" => Some(Self::Edits),
            "auto" => Some(Self::Auto),
            "full-access" => Some(Self::Full),
            _ => None,
        }
    }
    fn rank(self) -> u8 {
        match self {
            Self::ReadOnly => 0,
            Self::Supervised => 1,
            Self::Edits => 2,
            Self::Auto => 3,
            Self::Full => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedPolicy {
    pub organization: String,
    pub max_access: Option<AccessMode>,
}

/// System policy file location, matching KSG Code's `managedPolicyPath`.
pub fn managed_policy_path() -> &'static std::path::Path {
    if cfg!(target_os = "macos") {
        std::path::Path::new("/Library/Application Support/Shina/policy.json")
    } else if cfg!(target_os = "windows") {
        std::path::Path::new(r"C:\Program Files\Shina\policy.json")
    } else {
        std::path::Path::new("/etc/shina/policy.json")
    }
}

/// Reads only `organization` and `limits.maxRuntimeMode` from `path`.
/// Missing, unreadable, or malformed files mean no managed ceiling.
pub fn read_managed_policy_from_path(path: &std::path::Path) -> Option<ManagedPolicy> {
    let bytes = std::fs::read(path).ok()?;
    let doc: Value = serde_json::from_slice(&bytes).ok()?;
    let organization = doc.get("organization")?.as_str()?.trim();
    if organization.is_empty() {
        return None;
    }
    let max_access = doc
        .get("limits")
        .and_then(|limits| limits.get("maxRuntimeMode"))
        .and_then(Value::as_str)
        .and_then(AccessMode::parse);
    Some(ManagedPolicy {
        organization: organization.to_owned(),
        max_access,
    })
}

pub fn read_managed_policy() -> Option<ManagedPolicy> {
    read_managed_policy_from_path(managed_policy_path())
}

fn is_access_mode_above(mode: AccessMode, ceiling: Option<AccessMode>) -> bool {
    ceiling.is_some_and(|ceiling| mode.rank() > ceiling.rank())
}

/// Ported from KSG Code's `managedRuntimeModeDenial`.
pub fn managed_access_mode_denial(
    policy: Option<&ManagedPolicy>,
    mode: AccessMode,
) -> Option<String> {
    let policy = policy?;
    let ceiling = policy.max_access?;
    is_access_mode_above(mode, Some(ceiling)).then(|| {
        format!(
            "{} limits threads to {}.",
            policy.organization,
            ceiling.label()
        )
    })
}

/// Ported from KSG Code's `clampManagedRuntimeMode`.
pub fn clamp_managed_access_mode(
    policy: Option<&ManagedPolicy>,
    mode: AccessMode,
    modes: &[AccessMode],
) -> Option<AccessMode> {
    let ceiling = policy.and_then(|policy| policy.max_access);
    if !is_access_mode_above(mode, ceiling) {
        return Some(mode);
    }
    modes
        .iter()
        .copied()
        .rfind(|&candidate| !is_access_mode_above(candidate, ceiling))
}

/// Supported access modes per Shina harness, ported from each KSG provider's
/// `supportedRuntimeModes` list. Harnesses with no list support all modes.
/// "Read only" is offered wherever Shina offers it today.
pub fn supported_access_modes(harness: &str) -> &'static [AccessMode] {
    match harness {
        "pi" | "antigravity" => &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Edits,
            AccessMode::Full,
        ],
        "kiro" | "muse" => &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Full,
        ],
        "fx" => &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Auto,
        ],
        "grok" => &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Auto,
            AccessMode::Full,
        ],
        _ => &AccessMode::ALL,
    }
}

/// Ported from KSG Code's `resolveCompatibleRuntimeMode`: keeps `current` if supported,
/// otherwise picks the first supported mode in picker order (skipping "Read only" unless
/// it was `current`).
pub fn resolve_compatible_access_mode(
    current: AccessMode,
    supported: &[AccessMode],
) -> (Vec<AccessMode>, AccessMode) {
    let modes: Vec<AccessMode> = if supported.is_empty() {
        AccessMode::ALL.to_vec()
    } else {
        AccessMode::ALL
            .iter()
            .copied()
            .filter(|mode| supported.contains(mode))
            .collect()
    };
    let mode = if modes.contains(&current) {
        current
    } else {
        modes
            .iter()
            .copied()
            .find(|&mode| mode != AccessMode::ReadOnly)
            .or_else(|| modes.first().copied())
            .unwrap_or(current)
    };
    (modes, mode)
}

#[derive(Default)]
pub struct PiSession {
    pub projected_operating: bool,
    pub projected_queued: bool,
    pub questionnaire: Option<questionnaire::Questionnaire>,
    questionnaires: HashMap<String, Value>,
    questionnaire_replies: Vec<(String, String, String)>,
    questionnaire_advance: Option<Instant>,
    pub notifications: VecDeque<(String, Instant)>,
    pub queue: VecDeque<QueuedMessage>,
    pub queue_held: bool,
    pub queue_expanded: bool,
    pub access: AccessMode,
    pub access_picker: bool,
    pub commands: Vec<SlashCommand>,
    pub skills: Vec<crate::composer_skills::Skill>,
    pub commands_loaded: bool,
    pub commands_requested: bool,
    pub commands_error: Option<String>,
    pub(super) pending_command: Option<Value>,
    pub slash_index: usize,
    pub slash_dismissed: Option<crate::composer::DismissedTrigger>,
    pub follow_up: bool,
    pub steering: Vec<String>,
    pub follow_ups: Vec<String>,
    pub compactions: Vec<Value>,
    pub compaction_expanded: bool,
    pub conversation: Option<ConversationMode>,
    pub fork_points: Vec<Value>,
    fork_points_requested: bool,
    fork_points_id: Option<String>,
    rewind_prompt: Option<Prompt>,
    pub confirm_rewind: Option<String>,
    pub stats: Value,
    pub(super) initial_user: bool,
    pub fork_source: Option<String>,
    pub fork_entry: Option<String>,
    pub previous_session: Option<String>,
    compact: Option<Option<String>>,
    compact_id: Option<String>,
    compact_was_running: bool,
    lifecycle: Option<(String, Value)>,
    lifecycle_id: Option<String>,
    refreshing_session: bool,
    sends: VecDeque<PendingSend>,
    queued_inputs: Vec<PendingSend>,
    recovery: Vec<PendingSend>,
    dialog_deadline: Option<Instant>,
    dialogs: VecDeque<(Value, Option<Instant>)>,
    session_approvals: HashSet<String>,
    dialog_status: Option<Status>,
    pub(super) session_name: Option<String>,
    probe: Option<String>,
    request_id: Option<String>,
    state_floor: usize,
    pub fresh_queue_ids: HashSet<usize>,
}

pub struct QueuedMessage {
    pub id: usize,
    pub prompt: Prompt,
}

struct PendingSend {
    id: Option<String>,
    prompt: Prompt,
    mode: &'static str,
    after: usize,
    delivered: bool,
}

impl PiSession {
    pub fn operating(&self) -> bool {
        self.projected_operating
            || self.compact.is_some()
            || self.compact_id.is_some()
            || self.lifecycle.is_some()
            || self.lifecycle_id.is_some()
            || self.refreshing_session
            || self.pending_command.is_some()
    }
}

impl Tool {
    pub(super) fn accept_result(&mut self, result: &Value) {
        self.output = truncate(&content_text(&result["content"]), 50_000);
        self.images = content_images(&result["content"]);
        let data = json!({"structuredContent":result["structuredContent"],"details":result["details"],"nestedCalls":result["nestedCalls"]});
        self.structured = if data.to_string().len() <= 256 * 1024 {
            data
        } else {
            json!({"notice":"Structured output exceeds the 256 KiB display limit. The full result remains in Pi's session."})
        };
        if self.name == "subagent" {
            self.subagents = result["details"]["results"].as_array().into_iter().flatten().take(32).filter_map(|agent| {
                let name = agent["agent"].as_str()?;
                let task = agent["task"].as_str()?;
                let finished = agent["finished"].as_bool().unwrap_or(self.done);
                let status = if !finished { "Working" } else if agent["stopReason"] == "aborted" { "Stopped" }
                    else if agent["exitCode"].as_i64().unwrap_or(0) != 0 || agent["stopReason"] == "error" { "Failed" } else { "Done" };
                let text = if status == "Failed" || status == "Stopped" {
                    agent["errorMessage"].as_str().or_else(||agent["stderr"].as_str()).map(str::to_owned)
                } else { None }.or_else(|| agent["messages"].as_array()?.iter().rev().find(|m|m["role"]=="assistant").map(|m|content_text(&m["content"]))).unwrap_or_default();
                Some(json!({"name":truncate(name,120),"task":truncate(task,2000),"model":agent["model"].as_str().map(|m|truncate(m,120)),"status":status,"output":truncate(&text,10000)}))
            }).collect();
        }
        self.exit_code = result["structuredContent"]["exit_code"]
            .as_i64()
            .or_else(|| result["details"]["exitCode"].as_i64());
        self.accept_work_result(result);
    }
}

impl Thread {
    pub fn can_change_conversation(&self) -> bool {
        self.harness == "pi"
            || self.harness == "codex" && self.harness_state["codexNativeExtensions"] == true
    }

    pub fn can_rewind_conversation(&self) -> bool {
        self.harness == "pi"
            || self.harness == "codex"
                && self.can_change_conversation()
                && self.harness_state["codexHistoryMode"] == "paginated"
    }
    pub fn slash_trigger(&self) -> Option<crate::composer::Trigger> {
        if !self.draft.focused {
            return None;
        }
        let trigger = crate::composer::trigger(&self.draft).filter(|t| t.marker == '/')?;
        if self
            .native
            .slash_dismissed
            .as_ref()
            .is_some_and(|d| d.matches(&trigger))
        {
            return None;
        }
        Some(trigger)
    }

    #[cfg(test)]
    pub fn slash_matches(&self) -> Vec<SlashCommand> {
        self.slash_matches_with_capabilities(
            self.fresh_session_command_available_local(),
            crate::usage_limits::has_usage_limits(&self.harness),
        )
    }

    pub fn slash_matches_with_capabilities(
        &self,
        fresh_session_available: bool,
        usage_limits_available: bool,
    ) -> Vec<SlashCommand> {
        let Some(trigger) = self.slash_trigger() else {
            return vec![];
        };
        let mut commands = vec![SlashCommand {
            name: "model".into(),
            description: "Switch response model for this thread".into(),
            source: "Shina".into(),
        }];
        if trigger.range.start == 0 && fresh_session_available {
            commands.push(SlashCommand {
                name: "clear".into(),
                description: "Start a fresh session; earlier messages stay visible".into(),
                source: "Shina".into(),
            });
        }
        if trigger.range.start == 0 && usage_limits_available {
            commands.push(SlashCommand {
                name: crate::usage_limits::USAGE_LIMITS_COMMAND_NAME.into(),
                description: crate::usage_limits::USAGE_LIMITS_COMMAND_DESCRIPTION.into(),
                source: if self.harness == "claude" {
                    "Claude"
                } else {
                    "Codex"
                }
                .into(),
            });
        }
        if trigger.range.start == 0 && crate::harness::supports_compaction(&self.harness) {
            commands.push(SlashCommand {
                name: "compact".into(),
                description: if self.harness == "codex" {
                    "Compact context with Codex".into()
                } else {
                    "Summarize context, with optional instructions".into()
                },
                source: if self.harness == "claude" {
                    "Claude"
                } else if self.harness == "codex" {
                    "Codex"
                } else {
                    "Pi"
                }
                .into(),
            });
        }
        commands.extend(
            self.native
                .commands
                .iter()
                .filter(|c| {
                    c.name != "model"
                        && (!fresh_session_available || c.name != "clear")
                        && (!usage_limits_available
                            || c.name != crate::usage_limits::USAGE_LIMITS_COMMAND_NAME)
                        && !c.name.starts_with('$')
                        && (trigger.range.start == 0 || c.name.starts_with("skill:"))
                        && (!crate::harness::supports_compaction(&self.harness)
                            || c.name != "compact")
                })
                .cloned(),
        );
        crate::composer::rank_slash_commands(commands, &trigger.query)
    }

    pub fn can_compact(&self) -> bool {
        !self.archived()
            && !self.initializing
            && !self.configuring
            && !self.native.operating()
            && self.context.phase != crate::context::ContextPhase::Compacting
            && self.request.is_none()
            && self.session.is_some()
            && crate::harness::supports_compaction(&self.harness)
            && (matches!(self.harness.as_str(), "codex" | "claude") && !self.status.busy()
                || self.harness == "pi" && self.model_provider == "cpa")
    }

    pub fn compact(&mut self, instructions: Option<String>) {
        if self.forward("thread.compact", json!({"instructions":instructions})) {
            return;
        }
        if !crate::harness::supports_compaction(&self.harness) {
            self.error = Some("This ACP agent has no native compaction API. Use its own commands if it advertises them.".into());
            return;
        }
        if self.archived()
            || self.configuring
            || self.native.operating()
            || self.request.is_some()
            || self.context.phase == crate::context::ContextPhase::Compacting
            || matches!(self.harness.as_str(), "codex" | "claude") && self.status.busy()
        {
            return;
        }
        self.native.compact_was_running = self.status.busy();
        self.native.compact = Some(instructions);
        self.status = Status::Running;
        self.phase = "Compacting context".into();
        self.drive_session();
    }

    /// Unlike the initial prompt, a steering submission must not reset the current run.
    #[cfg(test)]
    pub fn send_during_run(&mut self) {
        self.send_during_run_with_fresh(None, &[], &str::to_owned, false);
    }

    pub(crate) fn send_during_run_with_fresh(
        &mut self,
        target_server_id: Option<&str>,
        peers: &[crate::peer::Peer],
        machine_name: &dyn Fn(&str) -> String,
        fresh_context: bool,
    ) {
        if self.initializing
            || self.configuring
            || self.native.lifecycle_id.is_some()
            || self.native.refreshing_session
            || self.phase == "Stopping"
            || self.request.is_some()
            || !self.has_draft_content()
            || self.harness == "pi" && self.model_provider != "cpa"
        {
            return;
        }
        if self.draft_images.iter().any(Attachment::is_image) && self.supports_images != Some(true)
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
        // KSG owns queued runs in the application, and only submits them once the active run
        // settles. Keeping follow-ups here makes edit/cancel/promote atomic and preserves images.
        // A fresh-context turn never steers or restarts: it queues behind the active run.
        if self.follow_up_queues() || fresh_context {
            self.native.queue_held = false;
            self.serial += 1;
            let id = self.serial;
            if fresh_context {
                self.native.fresh_queue_ids.insert(id);
            }
            self.native.queue.push_back(QueuedMessage { id, prompt });
        } else {
            self.native.sends.push_back(PendingSend {
                id: None,
                prompt,
                mode: "steer",
                after: self.messages.len(),
                delivered: false,
            });
        }
        self.prompt_history.reset();
        self.paste_history = Default::default();
        self.draft.replace_text("");
        self.draft_images.clear();
        self.draft_context.clear();
        self.draft_reply_to = None;
        self.retained_context = Default::default();
        self.life.user_sent(lifecycle::now());
        self.drive_native();
    }

    pub fn cancel_queued(&mut self, id: usize) {
        if self.forward(
            "queue.remove",
            json!({"inputId":id,"generation":self.request_generation}),
        ) {
            return;
        }
        self.native.fresh_queue_ids.remove(&id);
        self.native.queue.retain(|q| q.id != id);
        self.reconcile_queued_edit();
    }

    pub fn steer_queued(&mut self, id: usize) {
        if self.forward(
            "queue.sendNow",
            json!({"inputId":id,"generation":self.request_generation}),
        ) {
            return;
        }
        if self.request.is_some() || self.configuring {
            return;
        }
        if self.status.busy() && self.native.fresh_queue_ids.contains(&id) {
            self.error = Some(
                "A fresh-session message cannot steer the active turn. It stays queued.".into(),
            );
            return;
        }
        if self.status.busy() && !self.can_steer() {
            self.error = Some(
                "This provider cannot steer the active turn. The message stays queued.".into(),
            );
            return;
        }
        if let Some(i) = self.native.queue.iter().position(|q| q.id == id)
            && let Some(queued) = self.native.queue.remove(i)
        {
            self.reconcile_queued_edit();
            if self.status.busy() {
                self.native.fresh_queue_ids.remove(&id);
                self.native.sends.push_back(PendingSend {
                    id: None,
                    prompt: queued.prompt,
                    mode: "steer",
                    after: self.messages.len(),
                    delivered: false,
                });
            } else {
                self.native.queue_held = false;
                self.native.queue.push_front(queued);
            }
            self.drive_native();
        }
    }

    pub(super) fn return_local_queue(&mut self) {
        self.cancel_queued_edit();
        let queue = std::mem::take(&mut self.native.queue);
        self.native.fresh_queue_ids.clear();
        self.native.queue_held = false;
        for queued in queue.into_iter().rev() {
            self.restore_prompt(&queued.prompt);
        }
    }

    /// Stop keeps admitted input in the server queue. Return to draft still restores it.
    fn recover_queued_prompt(&mut self, prompt: &Prompt) {
        if self.native.queue_held {
            self.serial += 1;
            self.native.queue.push_front(QueuedMessage {
                id: self.serial,
                prompt: prompt.clone(),
            });
        } else {
            self.restore_prompt(prompt);
        }
    }

    pub fn clear_user_queue(&mut self) {
        if self.forward(
            "queue.returnDraft",
            json!({"generation":self.request_generation}),
        ) {
            return;
        }
        self.return_local_queue();
        if self.pi.is_some() {
            self.command("clear_queue", json!({}));
        }
    }

    pub(super) fn native_pending_turns(&self) -> bool {
        !self.native.queue.is_empty()
            || !self.native.sends.is_empty()
            || !self.native.steering.is_empty()
            || !self.native.follow_ups.is_empty()
    }

    pub(super) fn drive_native(&mut self) -> bool {
        if self.offline_blocked || self.status == Status::Paused {
            return true;
        }
        if self.initializing || self.pi.is_none() {
            return false;
        }
        if let Some(command) = self.native.pending_command.take() {
            self.command("native_command", command);
            return true;
        }
        if self.native.lifecycle_id.is_some() || self.native.refreshing_session {
            return true;
        }
        if self.native.fork_source.is_some() && self.session == self.native.fork_source {
            self.fail_session(
                "Pi did not create a distinct fork session. The source was left unchanged.".into(),
            );
            return true;
        }
        if let Some(entry) = self.native.fork_entry.clone() {
            if self.native.rewind_prompt.is_none() {
                self.native.rewind_prompt = self
                    .native
                    .fork_points
                    .iter()
                    .find(|p| p["entryId"] == entry)
                    .map(|p| Prompt::new(string(p, "text").into(), content_images(&p["content"])));
            }
            self.native.lifecycle = Some(("fork".into(), json!({"entryId":entry})));
        }
        if let Some((kind, data)) = self.native.lifecycle.take() {
            self.native.state_floor = self.serial + 1;
            if self.command(&kind, data) {
                self.native.lifecycle_id = Some(self.command_id());
                self.configuring = true;
                self.operation_deadline = Some((
                    "session operation",
                    Instant::now() + Duration::from_secs(60),
                ));
            }
            return true;
        }
        if self.native.lifecycle_id.is_some() || self.native.refreshing_session {
            return true;
        }
        if self.native.fork_source.is_some() && self.session.is_some() {
            if self.session == self.native.fork_source {
                self.fail_session(
                    "Pi did not create a distinct fork session. The source was left unchanged."
                        .into(),
                );
                return true;
            }
            self.native.fork_source = None;
        }
        if self.native.conversation.is_some() && !self.native.fork_points_requested {
            self.load_fork_points();
        }
        if let Some(instructions) = self.native.compact.take() {
            if self.harness == "pi" && self.model_provider != "cpa" {
                self.error = Some("Choose a CPA model before compacting.".into());
                self.status = Status::Ready;
                return true;
            }
            let data =
                instructions.map_or_else(|| json!({}), |text| json!({"customInstructions":text}));
            if self.command("compact", data) {
                self.status = Status::Running;
                self.phase = "Compacting context".into();
                self.native.compact_id = Some(self.command_id());
                self.context.begin_compaction();
            } else {
                if !self.native.queue.is_empty() {
                    self.native.queue_held = true;
                }
                self.status = Status::Failed;
            }
            return true;
        }
        if self.native.compact_id.is_some()
            || self.context.phase == crate::context::ContextPhase::Compacting && !self.can_steer()
        {
            return true;
        }
        if self.native.session_name.as_deref() != Some(&self.title) && self.session.is_some() {
            let name = self.title.clone();
            if self.command("set_session_name", json!({"name":name})) {
                self.native.session_name = Some(name);
            }
        }
        if !self.status.busy()
            && !self.native.queue_held
            && self.native.sends.is_empty()
            && self.queued_prompt.is_none()
            && self.pending_prompt.is_none()
            && let Some(queued) = self.native.queue.pop_front()
        {
            let fresh_context = self.native.fresh_queue_ids.remove(&queued.id);
            self.reconcile_queued_edit();
            self.begin_prompt_with_fresh(queued.prompt, fresh_context);
            return true;
        }
        // One in-flight submission preserves order and gives rejected prompts their own owner.
        if let Some(mut send) = self.native.sends.pop_front_if(|p| p.id.is_none()) {
            let mut data = match attachments::prompt_record(&send.prompt, self.harness == "pi") {
                Ok(data) => data,
                Err(error) => {
                    self.restore_prompt(&send.prompt);
                    self.attachment_error = Some(error);
                    return false;
                }
            };
            data["streamingBehavior"] = json!(send.mode);
            if self.command("prompt", data) {
                send.id = Some(self.command_id());
                self.native.sends.push_front(send);
            } else {
                self.restore_prompt(&send.prompt);
            }
        }
        false
    }

    fn command_id(&self) -> String {
        format!("{}-{}", self.id, self.serial)
    }

    pub fn request_fork_points(&mut self, mode: ConversationMode) {
        if !self.can_rewind_conversation() {
            self.error =
                Some("This harness has no verified conversation fork or rewind protocol.".into());
            return;
        }
        if self.status.busy() || self.configuring || self.session.is_none() {
            return;
        }
        self.native.conversation = Some(mode);
        self.native.fork_points.clear();
        self.native.fork_points_requested = false;
        if self.pi.is_some() && !self.initializing {
            self.load_fork_points();
        }
    }

    fn load_fork_points(&mut self) {
        self.native.fork_points_requested = self.command("get_entries", json!({}));
        if self.native.fork_points_requested {
            self.native.fork_points_id = Some(self.command_id());
        }
    }

    pub fn rewind(&mut self, entry: String) {
        if self.forward("thread.rewind", json!({"entry":entry})) {
            return;
        }
        if self.status.busy() || self.configuring || self.native.operating() {
            return;
        }
        if !self
            .native
            .fork_points
            .iter()
            .any(|p| p["entryId"] == entry)
        {
            return;
        }
        self.native.rewind_prompt = self
            .native
            .fork_points
            .iter()
            .find(|p| p["entryId"] == entry)
            .map(|p| Prompt::new(string(p, "text").into(), content_images(&p["content"])));
        self.native.previous_session = self.session.clone();
        self.native.conversation = None;
        self.native.confirm_rewind = None;
        self.native.lifecycle = Some(("fork".into(), json!({"entryId":entry})));
        self.drive_session();
    }

    pub(super) fn probe_command_idle(&mut self) {
        self.native.compact_was_running = false;
        if self.command("get_state", json!({})) {
            self.native.probe = Some(self.command_id());
        }
    }

    pub(super) fn native_dialog_done(&mut self) {
        self.native.questionnaire_advance = None;
        self.native.questionnaire = None;
        self.native.dialog_deadline = None;
        self.status = self.native.dialog_status.take().unwrap_or(Status::Running);
        while let Some((request, deadline)) = self.native.dialogs.pop_front() {
            if deadline.is_some_and(|d| d <= Instant::now()) {
                continue;
            }
            self.native.dialog_status = Some(self.status);
            self.native.dialog_deadline = deadline;
            self.reply.replace_text(string(&request, "prefill"));
            self.prepare_questionnaire(&request);
            self.request = Some(request);
            self.status = Status::Waiting;
            break;
        }
        if self.initializing {
            self.session_started = Some(Instant::now());
        }
        if let Some((kind, _)) = self.operation_deadline {
            self.operation_deadline = Some((kind, Instant::now() + Duration::from_secs(60)));
        }
    }

    pub(super) fn cancel_native_dialogs(&mut self) {
        self.cancel_questionnaire();
        for _ in 0..33 {
            if self.request.is_none() {
                break;
            }
            let id = self.request.as_ref().map(|r| r["id"].clone());
            self.respond(json!({"cancelled":true}));
            if self.request.as_ref().map(|r| r["id"].clone()) == id {
                break;
            }
        }
    }

    pub fn approve_for_session(&mut self) {
        if self.remote.is_some() {
            self.respond(json!({"confirmed":true,"allowForSession":true}));
            return;
        }
        if let Some(request) = self.request.as_ref().filter(|r| r["method"] == "confirm") {
            self.native.session_approvals.insert(approval_key(request));
            self.respond(json!({"confirmed":true}));
        }
    }

    fn prepare_questionnaire(&mut self, request: &Value) {
        if !matches!(
            request["method"].as_str(),
            Some("select" | "input" | "editor")
        ) {
            self.native.questionnaire = None;
            return;
        }
        self.native.questionnaire = questionnaire::Questionnaire::parse(request).or_else(|| {
            self.native
                .questionnaires
                .iter()
                .find_map(|(id, params)| questionnaire::Questionnaire::rpiv(request, id, params))
        });
        // A previously focused, hidden composer must not receive questionnaire shortcuts.
        if self.native.questionnaire.is_some() && !self.request_held_by_typing_guard() {
            self.draft.focused = false;
            self.reply.focused = false;
        }
    }
    pub(super) fn prepare_remote_questionnaire(&mut self) {
        if let Some(request) = self.request.clone() {
            self.prepare_questionnaire(&request);
        } else {
            self.native.questionnaire = None;
        }
    }
    pub(super) fn projected_form(&self) -> Value {
        json!(self.native.questionnaire)
    }
    pub(super) fn apply_projected_form(&mut self, value: Value) -> Result<(), String> {
        self.native.questionnaire = serde_json::from_value(value).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub(super) fn respond_projected_form(&mut self, answers: Value) -> Result<(), String> {
        let answers: Vec<questionnaire::Answer> =
            serde_json::from_value(answers).map_err(|e| e.to_string())?;
        let form = self
            .native
            .questionnaire
            .as_mut()
            .ok_or("Questionnaire expired.")?;
        if answers.len() != form.questions.len() || answers.is_empty() {
            return Err("Invalid questionnaire answers.".into());
        }
        for (answer, question) in answers.iter().zip(&form.questions) {
            if answer.other.len() > 32_768
                || answer
                    .selected
                    .iter()
                    .any(|value| !question.options.iter().any(|option| option.value == *value))
            {
                return Err("Invalid questionnaire option.".into());
            }
        }
        form.answers = answers;
        form.index = form.questions.len() - 1;
        self.reply.replace_text(&form.answers[form.index].other);
        self.questionnaire_step(false);
        Ok(())
    }

    pub fn questionnaire_toggle(&mut self, index: usize) {
        if let Some(form) = &mut self.native.questionnaire {
            form.answers[form.index].other = self.reply.text.clone();
            if form.toggle(index) {
                self.reply.replace_text("");
                if !form.questions[form.index].multi {
                    self.native.questionnaire_advance =
                        Some(Instant::now() + Duration::from_millis(200));
                }
            }
        }
    }

    pub fn questionnaire_edit(&mut self) -> &mut rune::TextEdit {
        self.native.questionnaire_advance = None;
        &mut self.reply
    }

    pub fn questionnaire_step(&mut self, previous: bool) {
        self.native.questionnaire_advance = None;
        let Some(form) = &mut self.native.questionnaire else {
            return;
        };
        form.answers[form.index].other = self.reply.text.clone();
        if previous {
            form.index = form.index.saturating_sub(1);
        } else if form.answer(form.index).is_some() {
            if form.index + 1 == form.questions.len() {
                if let Some(response) = form.response() {
                    let replies = form.remaining_rpc();
                    let request = self.request.as_ref().map(|r| r["id"].clone());
                    self.respond(response);
                    if self.request.as_ref().map(|r| r["id"].clone()) != request {
                        self.native.questionnaire_replies.extend(replies);
                    }
                }
                return;
            }
            form.index += 1;
        }
        self.reply.replace_text(&form.answers[form.index].other);
        self.reply.focused = false;
    }

    pub(super) fn cancel_questionnaire(&mut self) {
        self.native.questionnaire_replies.clear();
        self.native.questionnaire_advance = None;
    }

    pub(super) fn refresh_typing_guard(&mut self) {
        let request_ids: Vec<String> = self
            .request
            .as_ref()
            .and_then(|r| {
                r["id"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| Some(r.to_string()))
            })
            .into_iter()
            .collect();
        self.typing_guard
            .on_requests(&request_ids, self.draft.focused, Instant::now());
    }

    pub(crate) fn request_held_by_typing_guard(&self) -> bool {
        let Some(request) = &self.request else {
            return false;
        };
        let id = request["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| request.to_string());
        self.typing_guard.is_held(&id)
    }

    pub(super) fn native_deadline(&self) -> Option<Instant> {
        self.native
            .notifications
            .iter()
            .map(|(_, deadline)| *deadline)
            .chain(self.native.dialog_deadline)
            .chain(self.native.questionnaire_advance)
            .chain(self.typing_guard.deadline())
            .chain(self.stop_deadline())
            .min()
    }

    pub fn refresh_commands(&mut self) {
        if self.forward("commands.refresh", json!({})) {
            return;
        }
        self.native.commands_error = None;
        self.command("get_commands", json!({}));
    }

    pub(super) fn has_pending_sends(&self) -> bool {
        !self.native.sends.is_empty()
    }

    pub(super) fn pending_inputs_value(&self) -> Value {
        let mut values: Vec<_> = self.native.sends.iter().chain(&self.native.queued_inputs).chain(&self.native.recovery).map(|s| json!({
            "text":s.prompt.text,"images":s.prompt.images.iter().map(Attachment::to_value).collect::<Vec<_>>(),"after":s.after,"draft":s.prompt.draft,"context":s.prompt.context
        })).collect();
        values.extend(self.native.queue.iter().map(|q| json!({"kind":"queued","inputId":q.id,
            "held":self.native.queue_held,"freshContext":self.native.fresh_queue_ids.contains(&q.id),
            "text":q.prompt.text,"images":q.prompt.images.iter().map(Attachment::to_value).collect::<Vec<_>>(),"after":self.messages.len(),"draft":q.prompt.draft,"context":q.prompt.context})));
        json!(values)
    }

    pub(super) fn restore_pending_inputs(&mut self, value: &Value) -> Result<(), String> {
        if value.is_null() {
            return Ok(());
        }
        for item in value.as_array().ok_or("Invalid queued messages.")? {
            if item.get("kind").is_some_and(|kind| kind != "queued") {
                continue;
            }
            let text = item["text"]
                .as_str()
                .ok_or("Invalid queued message text.")?;
            let saved_images = item["images"]
                .as_array()
                .ok_or("Invalid queued message images.")?;
            let images: Vec<_> = saved_images
                .iter()
                .filter_map(Attachment::from_value)
                .collect();
            if images.len() != saved_images.len() {
                self.attachment_error = Some("Some queued images are missing. Reattach them before sending the recovered draft.".into());
            }
            let context = item
                .get("context")
                .filter(|v| !v.is_null())
                .map(crate::composer_context::decode_message_context)
                .transpose()?;
            let prompt = crate::composer_tokens::received_prompt_with_context(
                text.into(),
                images,
                item.get("draft").cloned(),
                context,
            )?;
            if item["kind"] == "queued" {
                let id = item["inputId"]
                    .as_u64()
                    .and_then(|id| usize::try_from(id).ok())
                    .ok_or("Invalid queued input identity.")?;
                self.serial = self.serial.max(id);
                self.native.queue_held = true;
                if item["freshContext"] == true {
                    self.native.fresh_queue_ids.insert(id);
                }
                if !self.native.queue.iter().any(|q| q.id == id) {
                    self.native.queue.push_back(QueuedMessage { id, prompt });
                }
                continue;
            }
            self.native.recovery.push(PendingSend {
                id: None,
                prompt,
                mode: "steer",
                after: item["after"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .unwrap_or(0),
                delivered: false,
            });
        }
        Ok(())
    }

    pub(super) fn recover_native_inputs(&mut self) {
        let missing_images = self.attachment_error.clone();
        let recovery = std::mem::take(&mut self.native.recovery);
        for send in recovery.into_iter().rev() {
            if !self.messages.iter().skip(send.after).any(|m| {
                m.role == "You"
                    && (m.text == send.prompt.text
                        || attachments::pi_message(&send.prompt).is_ok_and(|text| m.text == text))
            }) {
                self.restore_prompt(&send.prompt);
                self.diagnostic(
                    "Recovered a queued message into the draft. It was not automatically resent.",
                );
            }
        }
        if missing_images.is_some() {
            self.attachment_error = missing_images;
        }
    }

    pub(super) fn native_disconnect(&mut self) {
        if let Some(command) = self.native.pending_command.take() {
            self.draft.replace_text(&format!(
                "/{} {}",
                string(&command, "name"),
                string(&command, "arguments")
            ));
        }
        self.native.questionnaire = None;
        self.native.questionnaire_advance = None;
        self.native.questionnaires.clear();
        self.native.questionnaire_replies.clear();
        self.native.notifications.clear();
        if !self.native.queue_held {
            self.return_local_queue();
        }
        let queued = std::mem::take(&mut self.native.queued_inputs);
        for send in queued.into_iter().rev() {
            self.recover_queued_prompt(&send.prompt);
        }
        self.native.steering.clear();
        self.native.follow_ups.clear();
        let sends = std::mem::take(&mut self.native.sends);
        for send in sends.into_iter().rev() {
            self.restore_prompt(&send.prompt);
        }
        self.native.compact = None;
        if self.native.compact_id.is_some()
            || self.context.phase == crate::context::ContextPhase::Compacting
        {
            self.record_compaction(json!({"aborted":true}));
            self.context.finish_compaction();
        }
        self.native.compact_id = None;
        self.native.lifecycle = None;
        self.native.lifecycle_id = None;
        self.native.refreshing_session = false;
        self.native.dialog_deadline = None;
        self.native.dialog_status = None;
        self.native.dialogs.clear();
        self.native.session_approvals.clear();
        self.native.probe = None;
        self.native.request_id = None;
        self.native.initial_user = false;
        self.native.fork_points_id = None;
        self.native.fork_points_requested = false;
        self.native.rewind_prompt = None;
    }

    pub(super) fn native_tick(&mut self, now: Instant) -> bool {
        let before = self.native.notifications.len();
        self.native
            .notifications
            .retain(|(_, deadline)| *deadline > now);
        let mut changed = (before != self.native.notifications.len()) | self.tick_stop(now);
        if self.native.dialog_deadline.is_some_and(|at| now >= at) {
            self.cancel_questionnaire();
            self.native.dialog_deadline = None;
            self.request = None;
            self.native_dialog_done();
            self.diagnostic("Pi's input request expired.");
            return true;
        }
        if self
            .native
            .questionnaire_advance
            .is_some_and(|at| now >= at)
        {
            self.questionnaire_step(false);
            return true;
        }
        changed |= self.typing_guard.release_if_idle(now);
        changed
    }

    /// Returns true when this record is fully handled here rather than by the chat reducer.
    pub(super) fn apply_native(&mut self, record: &Value) -> bool {
        if self.apply_codex(record) {
            return true;
        }
        match string(record, "type") {
            "extension_ui_cancel" => {
                self.native
                    .dialogs
                    .retain(|(request, _)| request["id"] != record["id"]);
                if self
                    .request
                    .as_ref()
                    .is_some_and(|request| request["id"] == record["id"])
                {
                    self.cancel_questionnaire();
                    self.request = None;
                    self.native_dialog_done();
                }
                return true;
            }
            "tool_execution_start" if record["toolName"] == "ask_user_question" => {
                if let Some(id) = record["toolCallId"].as_str()
                    && self.native.questionnaires.len() < 32
                {
                    self.native
                        .questionnaires
                        .insert(id.into(), record["args"].clone());
                }
            }
            "tool_execution_end" => {
                if let Some(id) = record["toolCallId"].as_str() {
                    self.native.questionnaires.remove(id);
                    self.native
                        .questionnaire_replies
                        .retain(|(tool, _, _)| tool != id);
                }
            }
            "extension_ui_request" if record["method"] == "notify" => {
                let message = truncate(string(record, "message"), 2000);
                self.diagnostic(&message);
                if self.native.notifications.len() >= 4 {
                    self.native.notifications.pop_front();
                }
                self.native
                    .notifications
                    .push_back((message, Instant::now() + Duration::from_secs(6)));
                return true;
            }
            "response" => {
                let command = string(record, "command");
                if command == "get_state" && self.native.state_floor > 0 {
                    let serial = record["id"]
                        .as_str()
                        .and_then(|id| id.rsplit_once('-'))
                        .and_then(|(_, n)| n.parse::<usize>().ok());
                    if serial.is_some_and(|n| n < self.native.state_floor) {
                        return true;
                    }
                }
                if command == "consume_reset_credit" {
                    self.harness_state["resetCreditOutcome"] = if record["success"] == false {
                        json!({"error": string(record, "error")})
                    } else {
                        json!({"outcome": record["data"]["outcome"]})
                    };
                    return true;
                }
                if command == "get_commands" {
                    self.native.commands_requested = false;
                    if record["success"] == false {
                        self.native.commands_error = Some(string(record, "error").into());
                    } else if let Some(commands) = record["data"]["commands"].as_array() {
                        self.native.skills = crate::composer_skills::from_commands(commands);
                        let mut seen = std::collections::HashSet::new();
                        self.native.commands = commands
                            .iter()
                            .filter_map(|c| {
                                let name = c["name"].as_str()?.trim();
                                if name.is_empty() || !seen.insert(name.to_owned()) {
                                    return None;
                                }
                                Some(SlashCommand {
                                    name: name.into(),
                                    description: string(c, "description").into(),
                                    source: string(c, "source").into(),
                                })
                            })
                            .collect();
                        self.native.commands_loaded = true;
                        self.native.commands_error = None;
                    }
                    return true;
                }
                if command == "get_messages"
                    && self
                        .native
                        .fork_points_id
                        .as_deref()
                        .is_some_and(|id| record["id"] == id)
                {
                    self.native.fork_points_id = None;
                    if record["success"] == false {
                        self.error = Some(string(record, "error").into());
                    } else {
                        self.native.fork_points = record["data"]["forkMessages"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                    }
                    return true;
                }
                if command == "get_fork_messages" {
                    if record["success"] == false {
                        self.error = Some(string(record, "error").into());
                    } else {
                        self.native.fork_points = record["data"]["messages"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                    }
                    return true;
                }
                if command == "prompt"
                    && self
                        .native
                        .sends
                        .front()
                        .is_some_and(|p| p.id.as_deref() == record["id"].as_str())
                {
                    let mut send = self.native.sends.pop_front().unwrap();
                    if record["success"] == false {
                        if record["queueInstead"] == true {
                            self.serial += 1;
                            self.native.queue.push_back(QueuedMessage {
                                id: self.serial,
                                prompt: send.prompt,
                            });
                        } else if string(record, "error").contains("compaction") {
                            send.id = None;
                            self.native.sends.push_front(send);
                            self.context.begin_compaction();
                        } else {
                            self.restore_prompt(&send.prompt);
                            self.error = Some(string(record, "error").into());
                        }
                    } else if record["data"]["disposition"] == "queued" && !send.delivered {
                        self.native.queued_inputs.push(send);
                    }
                    return true;
                }
                if self
                    .native
                    .lifecycle_id
                    .as_deref()
                    .is_some_and(|id| record["id"] == id)
                {
                    self.native.lifecycle_id = None;
                    self.operation_deadline = None;
                    if record["success"] == false || record["data"]["cancelled"] == true {
                        self.configuring = false;
                        self.native.fork_source = None;
                        self.native.rewind_prompt = None;
                        self.native.fork_entry = None;
                        self.error = Some(if record["data"]["cancelled"] == true {
                            "A Pi extension cancelled the session operation.".into()
                        } else {
                            string(record, "error").into()
                        });
                    } else {
                        if let Some(prompt) = self.native.rewind_prompt.take() {
                            self.restore_prompt(&prompt);
                        } else if let Some(text) =
                            record["data"]["text"].as_str().filter(|s| !s.is_empty())
                        {
                            self.restore_prompt(&Prompt::new(text.into(), vec![]));
                        }
                        self.reload_native_session();
                    }
                    return true;
                }
                if command == "get_state"
                    && self
                        .native
                        .request_id
                        .as_deref()
                        .is_some_and(|id| record["id"] == id)
                {
                    self.native.request_id = None;
                    if record["success"] == false {
                        self.fail_session(string(record, "error").into());
                        return true;
                    }
                    if record["data"]["isStreaming"] == true
                        || record["data"]["isCompacting"] == true
                    {
                        self.configuring = false;
                        self.fail_session("Pi started work while changing the conversation. Retry to reload its saved state.".into());
                        return true;
                    }
                    let Some(session) = record["data"]["sessionFile"]
                        .as_str()
                        .filter(|p| !p.is_empty())
                    else {
                        self.fail_session(
                            "Pi did not return a saved session after the operation.".into(),
                        );
                        return true;
                    };
                    if self.harness != "codex" && self.session.as_deref() == Some(session) {
                        self.fail_session(
                            "Pi did not create a distinct session after the fork. Retry to reload."
                                .into(),
                        );
                        return true;
                    }
                    self.session = Some(session.into());
                    self.native.fork_source = None;
                    self.native.fork_entry = None;
                    self.native.session_name = None;
                    self.update_model(&record["data"]["model"]);
                    if let Some(level) = record["data"]["thinkingLevel"].as_str() {
                        self.thinking = level.into();
                    }
                    self.command("get_entries", json!({}));
                    // Normalize the authoritative history through the same reducer as startup.
                    self.native.request_id = Some(self.command_id());
                    return true;
                }
                if command == "get_messages"
                    && self.native.refreshing_session
                    && self
                        .native
                        .request_id
                        .as_deref()
                        .is_some_and(|id| record["id"] == id)
                {
                    if record["success"] == false {
                        self.fail_session(string(record, "error").into());
                        return true;
                    }
                    self.native.refreshing_session = false;
                    self.native.request_id = None;
                    self.operation_deadline = None;
                    self.configuring = false;
                    self.initializing = true;
                    self.session_started = Some(Instant::now());
                    self.state_loaded = true;
                    self.history_loaded = false;
                    self.startup_model_applied = false;
                    self.startup_thinking_applied = false;
                    self.startup_selection_verified = false;
                    self.startup_selection_check = None;
                    self.context_startup_requested = false;
                    self.status = Status::Ready;
                    self.context.begin_model_change();
                    // Route through the existing authoritative history reducer.
                    let mut history = record.clone();
                    history["id"] = json!("history");
                    history["data"]["replaceHistory"] = json!(true);
                    self.apply(history);
                    self.refresh_context_usage();
                    return true;
                }
                if command == "compact"
                    && self
                        .native
                        .compact_id
                        .as_deref()
                        .is_some_and(|id| record["id"] == id)
                {
                    self.native.compact_id = None;
                    if record["success"] == false {
                        self.error = Some(string(record, "error").into());
                        if !self.native.queue.is_empty() {
                            self.native.queue_held = true;
                        }
                        if !matches!(self.harness.as_str(), "codex" | "claude")
                            || self
                                .native
                                .compactions
                                .last()
                                .is_none_or(|row| row["errorMessage"] != record["error"])
                        {
                            self.record_compaction(json!({"errorMessage":string(record,"error")}));
                        }
                    }
                    self.context.finish_compaction();
                    self.refresh_context_usage();
                    self.command("get_state", json!({}));
                    self.native.probe = Some(self.command_id());
                    return true;
                }
                if command == "get_state"
                    && self
                        .native
                        .probe
                        .as_deref()
                        .is_some_and(|id| record["id"] == id)
                {
                    self.native.probe = None;
                    if record["success"] == false {
                        self.fail_session(string(record, "error").into());
                        return true;
                    }
                    if record["data"]["isStreaming"] == true
                        || record["data"]["isCompacting"] == true
                    {
                        self.status = Status::Running;
                        self.started.get_or_insert_with(Instant::now);
                    } else {
                        self.native.initial_user = false;
                        if !self.native.compact_was_running {
                            self.status = Status::Ready;
                        } else {
                            self.finish_run();
                            self.complete_task(status_word(self.status));
                        }
                        self.phase.clear();
                    }
                }
                if command == "get_session_stats"
                    && record["success"] != false
                    && self.context.accepts(record["id"].as_str())
                {
                    self.native.stats = record["data"].clone();
                }
                if command == "clear_queue" && record["success"] != false {
                    let mut texts = vec![];
                    for key in ["steering", "followUp"] {
                        if let Some(items) = record["data"][key].as_array() {
                            texts.extend(
                                items
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .filter(|s| !s.starts_with(INJECTION_PREFIX))
                                    .map(|s| {
                                        if self.harness == "claude" {
                                            memory::without_diff(s)
                                        } else {
                                            s
                                        }
                                        .to_owned()
                                    }),
                            );
                        }
                    }
                    let mut recovered = std::mem::take(&mut self.native.queued_inputs);
                    // Pi returns only text here. Keep the original local image references as well.
                    for send in recovered.drain(..).rev() {
                        if let Some(i) = texts.iter().position(|text| *text == send.prompt.text) {
                            texts.remove(i);
                        }
                        self.recover_queued_prompt(&send.prompt);
                    }
                    // Claude's interrupt receipt can prove an already-streamed
                    // input was cancelled. Preserve its images and path references.
                    if self.harness == "claude" {
                        for message in record["data"]["messages"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .rev()
                        {
                            let text = content_text(&message["content"]);
                            if text.starts_with(INJECTION_PREFIX) {
                                continue;
                            }
                            if let Some(i) = texts.iter().position(|s| *s == text) {
                                texts.remove(i);
                            }
                            let mut images = content_images(&message["content"]);
                            images.extend(
                                message["attachments"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(Attachment::from_value),
                            );
                            self.recover_queued_prompt(&Prompt::new(text, images));
                        }
                    }
                    if !texts.is_empty() {
                        for text in texts.into_iter().rev() {
                            self.recover_queued_prompt(&Prompt::new(text, vec![]));
                        }
                    }
                    self.native.steering.clear();
                    self.native.follow_ups.clear();
                }
            }
            "message_end"
                if record["message"]["role"] == "custom"
                    && record["message"]["display"] != false =>
            {
                self.messages.push(Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "Shina",
                    text: content_text(&record["message"]["content"]),
                    images: content_images(&record["message"]["content"]),
                    context: None,
                });
            }
            "message_update" => {
                if let (Some(tokens), Some(window)) = (
                    record["usage"]["totalTokens"].as_u64().filter(|n| *n > 0),
                    self.context_window,
                ) && self.context.phase != crate::context::ContextPhase::Compacting
                {
                    self.context.cancel_request();
                    self.context
                        .absorb(&json!({"contextUsage":{"tokens":tokens,"contextWindow":window}}));
                    self.refresh_estimated_breakdown();
                }
            }
            "queue_update" => {
                let texts = |key| {
                    record[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().or_else(|| v["text"].as_str()))
                        .map(str::to_owned)
                        .collect()
                };
                self.native.steering = texts("steering");
                self.native.follow_ups = texts("followUp");
            }
            "compaction_start" => {
                self.record_compaction(json!({"running":true,"reason":record["reason"]}));
            }
            "compaction_end" => {
                self.record_compaction(record.clone());
            }
            "message_start" if record["message"]["role"] == "user" => {
                let message = &record["message"];
                let text = content_text(&message["content"]);
                if text.starts_with(INJECTION_PREFIX) {
                    // Internal deliveries already have a transcript row and a durable marker.
                    if !self
                        .messages
                        .iter()
                        .any(|m| m.role == "Shina" && m.text == text)
                    {
                        self.messages.push(Message {
                            id: uuid::Uuid::new_v4().to_string(),
                            role: "Shina",
                            text,
                            images: content_images(&message["content"]),
                            context: None,
                        });
                    }
                    return false;
                }
                if std::mem::take(&mut self.native.initial_user) {
                    // The initial prompt already has an optimistic row. Pi can expand its text.
                    return false;
                }
                let queued = self
                    .native
                    .queued_inputs
                    .iter()
                    .position(|s| s.prompt.text == text)
                    .or_else(|| (!self.native.queued_inputs.is_empty()).then_some(0));
                let original = if let Some(i) = queued {
                    self.native.queued_inputs.remove(i).prompt
                } else if let Some(send) = self.native.sends.front_mut() {
                    send.delivered = true;
                    send.prompt.clone()
                } else {
                    Prompt::new(text, content_images(&message["content"]))
                };
                self.messages.push(Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "You",
                    text: original.text,
                    images: original.images.to_vec(),
                    context: original.context,
                });
            }
            "extension_error" => {
                let message = format!(
                    "Pi extension failed during {}: {}",
                    string(record, "event"),
                    string(record, "error")
                );
                self.diagnostic(&message);
                self.messages.push(Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "Shina",
                    text: message,
                    images: vec![],
                    context: None,
                });
                return true;
            }
            "extension_ui_request"
                if matches!(
                    string(record, "method"),
                    "confirm" | "select" | "input" | "editor"
                ) =>
            {
                let signature = questionnaire::signature(record);
                if let Some(at) =
                    self.native
                        .questionnaire_replies
                        .iter()
                        .position(|(tool, key, _)| {
                            self.native.questionnaires.len() == 1
                                && *key == signature
                                && self.native.questionnaires.contains_key(tool)
                                && record["timeout"].as_i64().is_none_or(|n| n >= 0)
                        })
                {
                    let (_, _, value) = self.native.questionnaire_replies.remove(at);
                    if let Some(pi) = &self.pi {
                        if let Err(error) = pi.send(
                            json!({"type":"extension_ui_response","id":record["id"],"value":value}),
                        ) {
                            self.error = Some(error);
                        } else {
                            return true;
                        }
                    }
                }
                if record["method"] == "confirm"
                    && self
                        .native
                        .session_approvals
                        .contains(&approval_key(record))
                {
                    if let Some(pi) = &self.pi
                        && let Err(error) = pi.send(json!({"type":"extension_ui_response","id":record["id"],"confirmed":true})) { self.error = Some(error); }
                    return true;
                }
                if self
                    .request
                    .as_ref()
                    .is_some_and(|r| r["id"] == record["id"])
                    || self
                        .native
                        .dialogs
                        .iter()
                        .any(|(r, _)| r["id"] == record["id"])
                {
                    return true;
                }
                let deadline = record["timeout"].as_i64().filter(|n| *n != 0).map(|n| {
                    Instant::now()
                        + Duration::from_millis((n.max(0) as u64).min(24 * 60 * 60 * 1000))
                });
                if self.request.is_some() {
                    if self.native.dialogs.len() >= 32 {
                        if let Some(pi) = &self.pi {
                            let _ = pi.send(json!({"type":"extension_ui_response","id":record["id"],"cancelled":true}));
                        }
                        self.diagnostic(
                            "Too many concurrent Pi dialogs. The new request was cancelled.",
                        );
                    } else {
                        self.native.dialogs.push_back((record.clone(), deadline));
                    }
                    return true;
                }
                self.native.dialog_status.get_or_insert(self.status);
                self.native.dialog_deadline = deadline;
                self.prepare_questionnaire(record);
            }
            "agent_settled"
                if self.native.compact_id.is_some() || self.native.lifecycle_id.is_some() =>
            {
                return true;
            }
            _ => {}
        }
        false
    }

    fn reload_native_session(&mut self) {
        self.native.refreshing_session = true;
        self.configuring = true;
        self.operation_deadline =
            Some(("session reload", Instant::now() + Duration::from_secs(30)));
        self.command("get_state", json!({}));
        self.native.request_id = Some(self.command_id());
    }

    pub(super) fn anchor_compactions(&mut self, history_index: usize) {
        for index in 0..self.native.compactions.len() {
            if self.native.compactions[index]["historyIndex"].as_u64() == Some(history_index as u64)
            {
                self.native.compactions[index]["anchor"] = json!(ToolAnchor {
                    message: self.messages.len(),
                    offset: 0
                });
                self.native.compactions[index]["order"] = json!(self.next_work_order());
            }
        }
    }

    fn record_compaction(&mut self, mut value: Value) {
        if self
            .native
            .compactions
            .last()
            .is_some_and(|v| v["running"] == true)
        {
            for key in ["anchor", "order", "historyIndex"] {
                if !self.native.compactions.last().unwrap()[key].is_null() {
                    value[key] = self.native.compactions.last().unwrap()[key].clone();
                }
            }
            *self.native.compactions.last_mut().unwrap() = value;
        } else {
            let message = self.assistant.unwrap_or(self.messages.len());
            value["anchor"] = json!(ToolAnchor {
                message,
                offset: self.assistant.map_or(0, |i| self.messages[i].text.len())
            });
            value["order"] = json!(self.next_work_order());
            if self.native.compactions.len() >= 32 {
                self.native.compactions.remove(0);
            }
            self.native.compactions.push(value);
        }
    }
}

impl State {
    pub fn effective_access_mode(&self, thread: &crate::model::Thread) -> AccessMode {
        let (modes, compatible) = resolve_compatible_access_mode(
            thread.native.access,
            supported_access_modes(&thread.harness),
        );
        clamp_managed_access_mode(self.managed_policy.as_ref(), compatible, &modes)
            .unwrap_or(compatible)
    }

    pub fn reconcile_current_access_mode(&mut self) {
        let access = self.effective_access_mode(self.current());
        if self.current().native.access == access {
            return;
        }
        if self.forward(
            "thread.configure",
            json!({"threadId":self.current().id,"access":access.id()}),
        ) {
            self.current_mut().native.access = access;
            return;
        }
        if self.demo || self.current().configuring || self.current().archived() {
            self.current_mut().native.access = access;
            return;
        }
        if self.current().harness == "claude"
            && self.current().pi.is_some()
            && !self.current().initializing
        {
            self.current_mut().native.access = access;
            self.current_mut()
                .command("set_access_mode", json!({"accessMode":access.id()}));
            self.checkpoint();
            return;
        }
        if self.current().status.busy() {
            return;
        }
        if self.current().pi.is_some() {
            self.current_mut().disconnect();
            self.current_mut().native.access = access;
            self.connect_thread(self.active);
        } else {
            self.current_mut().native.access = access;
        }
        self.checkpoint();
    }

    pub fn open_access_picker(&mut self) {
        self.reconcile_current_access_mode();
        self.current_mut().native.access_picker = true;
    }

    pub fn set_access_mode(&mut self, access: AccessMode) {
        let (modes, compatible) =
            resolve_compatible_access_mode(access, supported_access_modes(&self.current().harness));
        let access = clamp_managed_access_mode(self.managed_policy.as_ref(), compatible, &modes)
            .unwrap_or(compatible);
        if self.forward(
            "thread.configure",
            json!({"threadId":self.current().id,"access":access.id()}),
        ) {
            return;
        }
        if self.demo
            || self.current().status.busy() && self.current().harness != "claude"
            || self.current().configuring
            || self.current().archived()
        {
            return;
        }
        self.current_mut().native.access_picker = false;
        if self.current().native.access == access {
            return;
        }
        if self.current().harness == "claude"
            && self.current().pi.is_some()
            && !self.current().initializing
        {
            self.current_mut().native.access = access;
            self.current_mut()
                .command("set_access_mode", json!({"accessMode":access.id()}));
            self.checkpoint();
            return;
        }
        self.current_mut().disconnect();
        self.current_mut().native.access = access;
        self.connect_thread(self.active);
        self.checkpoint();
    }

    pub fn choose_slash(&mut self, name: String) {
        if name == "clear" && !self.fresh_session_command_available(self.current()) {
            return;
        }
        if name == crate::usage_limits::USAGE_LIMITS_COMMAND_NAME
            && !self.usage_limits_command_available(self.current())
        {
            return;
        }
        let range = self.current().slash_trigger().map(|t| t.range).or_else(|| {
            (name == "model" && self.current().draft.text.trim() == "/model")
                .then(|| 0..self.current().draft.text.len())
        });
        let Some(range) = range else {
            return;
        };
        let replacement = if name == "model"
            || name == "clear"
            || name == crate::usage_limits::USAGE_LIMITS_COMMAND_NAME
        {
            String::new()
        } else {
            format!("/{name} ")
        };
        let edit = &mut self.current_mut().draft;
        edit.undo_group(|edit| {
            edit.set_selection(range.start, range.end);
            edit.insert(&replacement);
        });
        edit.focused = true;
        if name == "model" {
            self.open_model_picker(SettingsPicker::Model);
        } else if name == "clear" {
            let thread_id = self.current().id;
            self.arm_fresh_session(thread_id);
            self.checkpoint();
        } else if name == crate::usage_limits::USAGE_LIMITS_COMMAND_NAME {
            self.open_usage_limits();
        }
    }

    pub fn compact_context(&mut self, instructions: Option<String>) {
        if self.demo || self.current().archived() {
            return;
        }
        self.current_mut().compact(instructions);
        if self.current().pi.is_none() && self.current().native.compact.is_some() {
            self.connect_thread(self.active);
        }
        self.checkpoint();
    }

    pub fn conversation_points(&mut self, mode: ConversationMode) {
        if self.forward(
            "thread.forkPoints",
            json!({"threadId":self.current().id,"rewind":mode == ConversationMode::Rewind}),
        ) {
            self.current_mut().native.conversation = Some(mode);
            return;
        }
        if self.demo || self.current().archived() {
            return;
        }
        self.current_mut().request_fork_points(mode);
        if self.current().pi.is_none() && self.current().native.conversation.is_some() {
            self.connect_thread(self.active);
        }
    }

    /// Fork in a separate process, as KSG does, so the source session and its owner never move.
    pub fn fork_conversation(&mut self, entry: Option<String>) {
        if self.forward(
            "thread.fork",
            json!({"threadId":self.current().id,"entry":entry}),
        ) {
            return;
        }
        if !self.current().can_change_conversation() {
            self.current_mut().error =
                Some("This harness has no verified conversation fork protocol.".into());
            return;
        }
        if self.demo
            || self.current().archived()
            || self.current().status.busy()
            || self.current().configuring
        {
            return;
        }
        let source = self.current();
        let Some(session) = source.session.clone() else {
            return;
        };
        let mut thread = Thread::new(self.next_id);
        thread.never_sent = false;
        thread.harness = source.harness.clone();
        thread.models = source.models.clone();
        thread.harness_state["codexNativeExtensions"] =
            source.harness_state["codexNativeExtensions"].clone();
        thread.harness_state["serviceTier"] = source.harness_state["serviceTier"].clone();
        thread.title = format!("{} (fork)", source.title);
        thread.renamed = true;
        thread.project = source.project;
        thread.machine = source.machine.clone();
        thread.captain = source.captain;
        thread.selected_model = source.selected_or_effective_model();
        thread.selected_thinking = source.selected_or_effective_reasoning();
        thread.native.access = source.native.access;
        thread.native.previous_session = Some(session.clone());
        thread.native.fork_source = Some(session);
        thread.native.rewind_prompt = entry
            .as_ref()
            .and_then(|id| {
                source
                    .native
                    .fork_points
                    .iter()
                    .find(|p| p["entryId"] == *id)
            })
            .map(|p| Prompt::new(string(p, "text").into(), content_images(&p["content"])));
        thread.native.fork_entry = entry;
        self.current_mut().native.conversation = None;
        self.threads.push(thread);
        self.next_id += 1;
        self.select_thread(self.threads.len() - 1);
        self.connect_thread(self.active);
        self.checkpoint();
    }
}
