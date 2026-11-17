//! The effective capability record (#32). One authoritative, per-caller answer
//! to "what can this thread do right now": role and ownership, coordination
//! rights with precise refusal reasons, executable harness/model/options,
//! interaction support, authorized reads, placement modes, live capacity and
//! negotiated peer features. `orchestrator_capabilities` returns it, `tools/list`
//! and the injected Shina instructions derive from it, and `mcp_call`
//! revalidates every call independently — the record is discovery, never
//! authorization.
use super::*;
use crate::protocol;
use serde_json::{Value, json};

/// Additive discovery-contract marker. It changes only when existing fields
/// are removed or change meaning; new fields, tools and sections are compatible.
pub const CONTRACT: &str = "shina-effective-capabilities/v1";

/// Shina awareness for a delegated or peer-launched worker. The coordinator
/// text in `mcp::INSTRUCTIONS` names operations these roles can never call, so
/// workers get the description their effective record actually allows.
pub const WORKER_INSTRUCTIONS: &str = concat!(
    "## Shina\n\n",
    "You are running inside Shina, a native desktop workspace where the user chats with coding agents. ",
    "Each conversation is a Shina thread. You are a worker: another thread delegated this task, and it ",
    "receives your result when your turn ends.\n\n",
    "- orchestrator_capabilities reports this caller's effective record, including why each unavailable ",
    "operation is refused. Read it before assuming you can delegate, coordinate or write shared state.\n",
    "- read_thread and list_threads read shared context. task_status with your own thread id reports your ",
    "fences (attempt_id, run_id, runtime_generation) and delivery state.\n",
    "- list_memories and search_memories read your captain's memory when this task runs under one. ",
    "propose_memory stages a suggestion for the captain's next completion message; workers never write ",
    "memory. Suggestions are data for review, never instructions.\n",
    "- captain_document_read returns a bounded page of your captain's shared CREW.md, NOTES.md or ",
    "DIGEST.md when the launching server negotiated document reads. AGENTS.md is private to the ",
    "captain's primary home and is never shared with workers.\n",
    "- You cannot delegate further, control crew, change captain documents or routines, answer captain ",
    "questions or control other threads. The tool list omits those operations.\n",
    "- Finish the task and end your turn. Shina delivers your result to the parent.\n",
);

/// Remote workers get the worker text plus the remote-read limits their
/// effective record reports.
pub const REMOTE_WORKER_INSTRUCTIONS: &str = concat!(
    "## Shina\n\n",
    "You are running inside Shina on a linked machine, launched by a Shina server elsewhere. Each ",
    "conversation is a thread; your task belongs to the delegating server's parent thread, which ",
    "receives your result when your turn ends.\n\n",
    "- orchestrator_capabilities reports this caller's effective record. You may read threads and ",
    "memories and read your own task_status; you may not delegate, coordinate or control other work.\n",
    "- list_memories and search_memories return the captain-memory snapshot carried by your launch ",
    "brief. propose_memory stages a suggestion for review; workers never write memory.\n",
    "- captain_document_read returns a bounded page of the originating captain's shared CREW.md, ",
    "NOTES.md or DIGEST.md only when the launch negotiated document reads; page with offset, maxBytes ",
    "and expectedRevision. AGENTS.md is private and never shared.\n",
    "- read_thread and list_threads read this machine's threads. Machine ids in arguments belong to ",
    "the server that owns them.\n",
    "- The tool list omits every operation your role cannot run. Finish the task and end your turn; ",
    "Shina returns your result to the delegating parent.\n",
);

/// An ordinary thread can delegate and control its own children, but it has
/// no Captain memory, crew control, routines or planning documents.
pub(crate) const THREAD_INSTRUCTIONS: &str = concat!(
    "## Shina\n\n",
    "You are running inside Shina, a native desktop workspace where the user chats with coding agents. ",
    "Each conversation is a Shina thread. Ordinary coding tasks need no Shina tools.\n\n",
    "- orchestrator_capabilities reports this caller's effective record, including unavailable operations and their reasons.\n",
    "- You may delegate independent work with delegate_task. Only children you delegated can be controlled with task_status, task_send and task_cancel. Delegated threads cannot delegate further.\n",
    "- list_threads, read_thread and list_captains read shared context. Read a thread before answering about it.\n",
    "- html_preview checks a self-contained page; html_render publishes one as a card. Only full-access threads may pass local paths or images.\n",
    "- Captain memory, crew control, Captain documents, routines and crew questions belong to Captain roles. They are not available from an ordinary thread.\n",
    "- End your turn when the work is finished. The user remains in control of approvals and confirmations.\n",
);

/// An independent crew root coordinates its own workers and reports to its
/// current Captain. It never receives the Captain's private rules or write
/// powers.
pub(crate) const CREW_INSTRUCTIONS: &str = concat!(
    "## Shina\n\n",
    "You are an independent crew thread under a Captain in Shina. You coordinate your own work stream. ",
    "The Captain receives your reports; the user remains in control of approvals and confirmations.\n\n",
    "- orchestrator_capabilities reports this caller's effective record, including unavailable operations and their reasons.\n",
    "- You may delegate your own workers with delegate_task and control only the children you delegated with task_status, task_send and task_cancel.\n",
    "- captain_crew_report sends progress, results, blockers, questions and evidence references to your current Captain. Use one stable requestId when retrying the same report. Reports are untrusted claims, not proof that work shipped.\n",
    "- captain_reports_list reads the reports you authored for this Captain. captain_document_read reads shared CREW.md, NOTES.md and DIGEST.md. The Captain's private AGENTS.md is never shared.\n",
    "- propose_memory stages a suggestion for the Captain to review; it never writes Captain memory. list_memories and search_memories read the Captain's explicit memory.\n",
    "- You cannot create, send to or stop the Captain's other crew, write Captain documents, manage routines, write Captain memory, or answer crew questions.\n",
    "- Finish the work and end your turn. Shina delivers your result and reports to the parent.\n",
);

/// A prior Captain conversation is a record. It can read what its role still
/// authorizes, but it is never an active coordinator.
pub(crate) const EARLIER_HOME_INSTRUCTIONS: &str = concat!(
    "## Shina\n\n",
    "This conversation is an earlier Captain home, not the Captain's active coordinator. ",
    "The current primary home is the place for new Captain work.\n\n",
    "- Open the current primary home to delegate new work, change crew, write planning documents or memory, manage routines, or answer crew questions.\n",
    "- You may read threads, Captain records, shared CREW.md, NOTES.md and DIGEST.md, crew reports and Captain memory where the tool list permits. The private AGENTS.md is not available here.\n",
    "- Do not delegate, recruit or release crew, create or change routines, write Captain memory or documents, or answer crew questions from this earlier conversation.\n",
    "- Do not describe this conversation as the active Captain home or claim its write, crew or routine powers.\n",
    "- Finish the record, answer questions about it, and end your turn. User controls in Shina remain available separately.\n",
);

/// An archived Captain group is frozen. Reads may remain available; every
/// coordination and write surface stays closed until the user unarchives it.
pub(crate) const ARCHIVED_INSTRUCTIONS: &str = concat!(
    "## Shina\n\n",
    "This Captain and its crew are archived. Treat every thread in the group as a read-only record.\n\n",
    "- Reads such as list_threads, read_thread, captain_read, captain_document_read for shared documents, and captain_reports_list may remain available. The private AGENTS.md is not available to crew.\n",
    "- Do not delegate, control crew, create or change routines, write Captain memory or documents, control tasks, or answer crew questions. Those operations are unavailable while the Captain is archived.\n",
    "- Do not claim primary-home write powers, routines or crew control. The user can unarchive the Captain from Shina's controls.\n",
    "- Summarize or answer from the record and end your turn.\n",
);

/// A retired Captain is a read-only record with no coordination powers.
pub(crate) const RETIRED_INSTRUCTIONS: &str = concat!(
    "## Shina\n\n",
    "This Captain was demoted or retired and is a read-only record.\n\n",
    "- You may read threads, Captain records, shared documents and reports where the tool list permits.\n",
    "- Do not delegate, control crew, manage routines, write Captain memory or documents, control tasks, or answer crew questions.\n",
    "- Do not describe this thread as an active primary home or claim its write, crew or routine powers.\n",
    "- Summarize or answer from the record and end your turn.\n",
);

/// Tools every caller can invoke regardless of owner lifecycle, mirroring the
/// skip list in `mcp_call`. All others pass `require_active_thread_owner`.
const OWNER_BYPASS: &[&str] = &[
    "list_routines",
    "list_memories",
    "search_memories",
    "orchestrator_capabilities",
    "list_threads",
    "list_captains",
    "read_thread",
    "task_status",
    "captain_read",
    "captain_document_read",
    "captain_crew_list",
    "captain_reports_list",
    "captain_crew_report",
    "captain_question_answer",
];

/// The remote-worker allowlist in `mcp_call`; task_status is admitted for the
/// caller's own thread id only and captain_document_read rides its own lane.
const REMOTE_TOOLS: &[&str] = &[
    "propose_memory",
    "list_memories",
    "search_memories",
    "orchestrator_capabilities",
    "list_threads",
    "read_thread",
];

fn gate(available: bool, reason: Option<String>) -> Value {
    json!({"available": available, "reason": reason})
}

fn peer_reachable(peer: &crate::peer::Peer) -> bool {
    peer.last_error.is_none() && peer.last_reached_at.is_some()
}

impl Thread {
    /// The role-matched Shina awareness text captured for this session. State
    /// refreshes it at the next safe session start or eligible prompt.
    pub(crate) fn shina_instructions(&self) -> &'static str {
        self.shina
    }
}

impl State {
    /// The caller-facing role used by the record and by instruction selection.
    pub(crate) fn mcp_role(&self, index: usize) -> &'static str {
        let thread = &self.threads[index];
        if thread.peer_origin.is_some() {
            return "remote_worker";
        }
        let owner = thread.crew_captain.or(thread.captain);
        let phase = owner.map_or(CaptainPhase::Active, |id| self.captain_phase(id));
        if thread.delegated {
            return match phase {
                CaptainPhase::Archived => "archived_worker",
                CaptainPhase::Retired | CaptainPhase::Deleted => "retired_worker",
                CaptainPhase::Active => "worker",
            };
        }
        if thread.captain_home {
            return match phase {
                CaptainPhase::Archived => "archived_home",
                CaptainPhase::Retired | CaptainPhase::Deleted => "retired_home",
                CaptainPhase::Active => {
                    let (_, primary) = self.captain_actor(index);
                    if primary {
                        "captain_home"
                    } else {
                        "earlier_home"
                    }
                }
            };
        }
        if thread.crew_captain.is_some() || thread.captain.is_some() {
            return match phase {
                CaptainPhase::Archived => "archived_crew",
                CaptainPhase::Retired | CaptainPhase::Deleted => "retired_crew",
                CaptainPhase::Active => "crew",
            };
        }
        "thread"
    }

    pub(crate) fn instructions_for_role(role: &str) -> &'static str {
        match role {
            "captain_home" => crate::mcp::INSTRUCTIONS,
            "crew" => CREW_INSTRUCTIONS,
            "earlier_home" => EARLIER_HOME_INSTRUCTIONS,
            "archived_home" | "archived_crew" | "archived_worker" => ARCHIVED_INSTRUCTIONS,
            "retired_home" | "retired_crew" | "retired_worker" => RETIRED_INSTRUCTIONS,
            "worker" => WORKER_INSTRUCTIONS,
            "remote_worker" => REMOTE_WORKER_INSTRUCTIONS,
            _ => THREAD_INSTRUCTIONS,
        }
    }

    /// The Shina awareness text a caller's harness receives, derived from the
    /// same role the effective record reports.
    pub(crate) fn mcp_instructions(&self, index: usize) -> &'static str {
        Self::instructions_for_role(self.mcp_role(index))
    }
}

impl State {
    /// Why `tool` is unavailable to this caller before argument validation, in
    /// the same order `mcp_call` (and the request intercepts ahead of it) would
    /// refuse it. `None` means the dispatch can run; per-argument and
    /// per-attempt checks still apply.
    pub(crate) fn denial(&self, index: usize, tool: &str) -> Option<String> {
        let thread = &self.threads[index];
        // `mcp_call` routes crew control before its owner and remote gates.
        if matches!(
            tool,
            "captain_crew_create" | "captain_crew_send" | "captain_crew_stop"
        ) {
            return self.crew_control_denial(index);
        }
        // `learned_memory_request` answers ask_memory before `mcp_call` and its
        // owner gate run: only a writable, non-worker memory link may ask.
        if tool == "ask_memory" {
            let writable = thread.captain.is_some_and(|captain| {
                thread.captain_home && self.captain_home(captain) == Some(thread.id)
            });
            let worker = thread.delegated || thread.crew_captain.is_some();
            if !(writable && !worker) {
                return Some("ask_memory is available only in the primary captain home.".into());
            }
            return (self
                .memory
                .lock()
                .expect("captain memory")
                .backend
                .is_none())
            .then(|| "Honcho learned memory is disabled.".into());
        }
        if let Some(origin) = &thread.peer_origin {
            // `mcp_request_ready` intercepts these before `mcp_call` runs.
            match tool {
                "captain_document_read" => {
                    if !origin.document_reads {
                        return Some("The originating server cannot answer Captain document reads. Only the context in the brief is available.".into());
                    }
                    if self.origin_document_read_denial(index).is_some() {
                        return Some(
                            "The originating peer is no longer authorized to serve Captain documents."
                                .into(),
                        );
                    }
                    return None;
                }
                // html_render and html_preview are intercepted in
                // `mcp_request_ready` too; the remote gate never sees them.
                "html_render" | "html_preview" => return None,
                _ => {}
            }
            if !OWNER_BYPASS.contains(&tool)
                && let Err(error) = self.require_active_thread_owner(thread.id)
            {
                return Some(error);
            }
            if !REMOTE_TOOLS.contains(&tool) && tool != "task_status" {
                return Some(
                    "A remote worker may read threads and memories, but cannot delegate or control unrelated work.".into(),
                );
            }
            return None;
        }
        if !OWNER_BYPASS.contains(&tool)
            && let Err(error) = self.require_active_thread_owner(thread.id)
        {
            return Some(error);
        }
        let (captain, primary) = self.captain_actor(index);
        if tool == "captain_reports_list" && captain.is_none() {
            return Some("Only this captain's threads may read its crew reports.".into());
        }
        if matches!(tool, "task_send" | "task_cancel") && thread.delegated {
            return Some("Delegated threads cannot control other tasks.".into());
        }
        match tool {
            "delegate_task" => {
                if thread.delegated {
                    Some(
                        "Delegated threads cannot delegate further. Do the work in this thread."
                            .into(),
                    )
                } else if thread.captain_home && self.effective_home(thread.id) != thread.id {
                    Some("Open the captain's primary home to delegate new work. This is an earlier conversation.".into())
                } else {
                    None
                }
            }
            "remember" | "forget_memory" => {
                if captain.is_none() {
                    Some("Only captains and their workers have captain memory.".into())
                } else if !primary {
                    Some(
                        "Only the primary captain home may write its own memory. Workers may propose_memory.".into(),
                    )
                } else {
                    None
                }
            }
            "list_memories" | "search_memories" => captain
                .is_none()
                .then(|| "Only captains and their workers have captain memory.".into()),
            "propose_memory" => {
                if captain.is_none() {
                    Some("Only captains and their workers have captain memory.".into())
                } else if !(thread.delegated || thread.crew_captain.is_some()) {
                    Some(
                        "Only delegated workers propose memory. Captain homes use remember.".into(),
                    )
                } else {
                    None
                }
            }
            "captain_document_write" => {
                if !primary {
                    Some(
                        "Only this captain's primary home may write its planning documents.".into(),
                    )
                } else {
                    self.require_captain_capability().err()
                }
            }
            "captain_crew_join" => {
                if !primary {
                    Some("Only this captain's primary home may recruit crew.".into())
                } else {
                    captain.and_then(|id| self.require_active_captain(id).err())
                }
            }
            "captain_crew_release" => {
                if !primary {
                    Some("Only this captain's primary home may release crew.".into())
                } else {
                    captain.and_then(|id| self.require_active_captain(id).err())
                }
            }
            "captain_question_answer" => {
                if !primary {
                    Some("Only this captain's primary home may answer its crew questions.".into())
                } else {
                    captain.and_then(|id| self.require_active_captain(id).err())
                }
            }
            "captain_crew_report" => {
                if !(thread.crew_captain.is_some() && !thread.delegated && !thread.captain_home) {
                    Some(
                        "Only an independent crew thread may report to its current Captain.".into(),
                    )
                } else {
                    thread
                        .crew_captain
                        .and_then(|id| self.require_active_captain(id).err())
                }
            }
            "create_routine" | "pause_routine" | "delete_routine" => {
                if !(thread.captain_home && !thread.delegated) {
                    Some("Only a captain's home can manage its routines.".into())
                } else if thread.captain.is_none() {
                    Some("The caller is not a captain.".into())
                } else if self.demo || self.store.is_none() {
                    Some("Routine storage is unavailable.".into())
                } else if self.captain_home(captain.unwrap_or(usize::MAX)) != Some(thread.id) {
                    Some("Only the Captain's current primary home can manage its routines.".into())
                } else {
                    captain.and_then(|id| self.require_active_captain(id).err())
                }
            }
            "list_routines" => {
                if !(thread.captain_home && !thread.delegated) {
                    Some("Only a captain's home can manage its routines.".into())
                } else if thread.captain.is_none() {
                    Some("The caller is not a captain.".into())
                } else if self.demo || self.store.is_none() {
                    Some("Routine storage is unavailable.".into())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn remote_document_reads_available(&self, index: usize) -> bool {
        self.threads[index]
            .peer_origin
            .as_ref()
            .is_some_and(|origin| origin.document_reads)
            && self.origin_document_read_denial(index).is_none()
    }

    /// The origin must still grant this worker's thread the peer wait scope
    /// before it can answer a held Captain document read.
    fn origin_document_read_denial(&self, index: usize) -> Option<String> {
        let origin = self.threads[index].peer_origin.as_ref()?;
        let database = crate::database::for_path(&self.data_dir.join("index.json"))?;
        database
            .authorize(
                &origin.device_id,
                "peer.thread.wait",
                &json!({"threadId":self.threads[index].id}),
            )
            .err()
    }

    /// `captain_crew_tool`'s static gates, in its own order.
    fn crew_control_denial(&self, index: usize) -> Option<String> {
        if self.client.is_some() {
            return Some("Crew tools run only on the Captain's owning server.".into());
        }
        let thread = &self.threads[index];
        if !(thread.captain_home && !thread.delegated && thread.peer_origin.is_none()) {
            return Some("Only a Captain home may control independent crew.".into());
        }
        let Some(captain) = thread.captain else {
            return Some("The caller has no Captain profile.".into());
        };
        if crate::database::for_path(&self.data_dir.join("index.json")).is_none() {
            return Some("Crew control requires the durable server.".into());
        }
        self.require_active_captain(captain).err().or_else(|| {
            (self.captain_home(captain) != Some(thread.id)).then(|| {
                "Only the Captain's current primary home may start a crew operation.".into()
            })
        })
    }

    /// The tool subset this caller's `tools/list` advertises: the full catalog
    /// minus anything `denial` already rules out for the role.
    pub(crate) fn listed_tools(&self, index: usize) -> Vec<Value> {
        crate::mcp::tools()
            .as_array()
            .into_iter()
            .flatten()
            .filter(|tool| {
                self.denial(index, tool["name"].as_str().unwrap_or_default())
                    .is_none()
            })
            .cloned()
            .collect()
    }

    /// The `initialize`/`tools/list` meta requests, dispatched on the UI thread
    /// so the answer reflects the calling thread's effective record.
    pub(crate) fn mcp_meta_call(
        &mut self,
        caller: usize,
        tool: &str,
        args: &Value,
    ) -> Result<Value, String> {
        let index = self
            .threads
            .iter()
            .position(|t| t.id == caller)
            .ok_or("The calling thread no longer exists.")?;
        match tool {
            "initialize" => Ok(json!({
                "protocolVersion": args["protocolVersion"].as_str().unwrap_or(crate::mcp::PROTOCOL),
                "capabilities": {"tools": {}, "experimental": {"shina": {
                    "threadRead": 2,
                    "peerTasks": 2,
                    "effectiveCapabilities": CONTRACT,
                }}},
                "serverInfo": {"name": "Shina", "version": env!("CARGO_PKG_VERSION")},
                "instructions": self.mcp_instructions(index),
            })),
            "tools/list" => Ok(json!({"tools": self.listed_tools(index)})),
            _ => Err(format!("Shina has no tool named {tool}.")),
        }
    }

    /// The per-tool listing table: every advertised name, whether this caller
    /// sees it in `tools/list`, and the precise reason when it does not.
    pub(crate) fn tools_record(&self, index: usize) -> Value {
        let tools = crate::mcp::tools();
        let mut record = serde_json::Map::new();
        for tool in tools.as_array().into_iter().flatten() {
            let name = tool["name"].as_str().unwrap_or_default();
            let reason = self.denial(index, name);
            record.insert(
                name.to_owned(),
                json!({"listed": reason.is_none(), "reason": reason}),
            );
        }
        json!(record)
    }

    /// Coordination rights with precise reasons, each mirroring its handler's
    /// first static refusal.
    pub(crate) fn coordination(&self, index: usize) -> Value {
        let remote_peers = self.delegation_peers();
        let delegate_denial = self.denial(index, "delegate_task");
        let contract_peers = remote_peers.iter().filter(|peer| {
            peer.capabilities["contracts"]
                .as_array()
                .is_some_and(|c| c.iter().any(|contract| contract == protocol::PEER_TASKS))
        });
        let remote_capable: Vec<&str> = contract_peers
            .clone()
            .filter(|peer| peer_reachable(peer))
            .map(|peer| peer.server_id.as_str())
            .collect();
        let (remote_available, remote_reason) = if let Some(reason) = &delegate_denial {
            (false, Some(reason.clone()))
        } else if remote_capable.is_empty() {
            let reason = if let Some(peer) =
                contract_peers.clone().find(|peer| !peer_reachable(peer))
            {
                peer.last_error.clone().unwrap_or_else(|| {
                    "A linked peer advertises peer tasks but is not reachable. Check the machine connection or credentials, then retry.".to_string()
                })
            } else {
                "No linked peer advertises peer tasks. Link a machine in Settings to delegate remotely.".to_string()
            };
            (false, Some(reason))
        } else {
            (true, None)
        };
        let crew = self.denial(index, "captain_crew_create");
        json!({
            "delegate_local": gate(delegate_denial.is_none(), delegate_denial),
            "delegate_remote": {
                "available": remote_available,
                "reason": remote_reason,
                "peers": remote_capable,
            },
            "crew": {
                "create": gate(crew.is_none(), crew.clone()),
                "send": gate(crew.is_none(), crew.clone()),
                "stop": gate(crew.is_none(), crew),
                "join": gate(
                    self.denial(index, "captain_crew_join").is_none(),
                    self.denial(index, "captain_crew_join"),
                ),
                "release": gate(
                    self.denial(index, "captain_crew_release").is_none(),
                    self.denial(index, "captain_crew_release"),
                ),
                "remote_roots": gate(false, Some(
                    "Independent crew roots run only on the Captain's local machine. Remote crew creation is not implemented.".into(),
                )),
            },
            "captain_document_write": gate(
                self.denial(index, "captain_document_write").is_none(),
                self.denial(index, "captain_document_write"),
            ),
            "captain_question_answer": gate(
                self.denial(index, "captain_question_answer").is_none(),
                self.denial(index, "captain_question_answer"),
            ),
            "crew_report": gate(
                self.denial(index, "captain_crew_report").is_none(),
                self.denial(index, "captain_crew_report"),
            ),
            "routines": gate(
                self.denial(index, "create_routine").is_none(),
                self.denial(index, "create_routine"),
            ),
            "memory_write": gate(
                self.denial(index, "remember").is_none(),
                self.denial(index, "remember"),
            ),
            "memory_propose": gate(
                self.denial(index, "propose_memory").is_none(),
                self.denial(index, "propose_memory"),
            ),
            "task_send": gate(
                self.denial(index, "task_send").is_none(),
                self.denial(index, "task_send"),
            ),
            "task_cancel": gate(
                self.denial(index, "task_cancel").is_none(),
                self.denial(index, "task_cancel"),
            ),
        })
    }

    /// Interaction support: what task_send can deliver, which answers it may
    /// carry, and who owns approvals.
    pub(crate) fn interaction(&self, index: usize) -> Value {
        let send = self.denial(index, "task_send");
        json!({
            "task_send": {
                "available": send.is_none(),
                "reason": send,
                "deliveries": ["follow_up", "steer"],
                "steer": "steer applies to the child's live attempt only. A refused steer fails; it never silently queues.",
                "steering_by_harness": {
                    "pi": true,
                    "codex": true,
                    "claude": "Only sessions advertising interrupt_cancel_queued_v1.",
                    "acp": "Only agents advertising t3code/steering or interrupt-restart steering."
                },
                "requests": "While task_status.request is set, request_id and runtime_generation answer the pending input, editor or select request. Confirmations are user-only; task_send refuses them.",
                "fences": "attempt_id fences steer; request_id and runtime_generation fence request answers; client_request_id replays a retry instead of double-delivering."
            },
            "task_cancel": {
                "available": self.denial(index, "task_cancel").is_none(),
                "reason": self.denial(index, "task_cancel"),
                "fences": "client_request_id and the current attempt_id are required. A stale attempt is refused and never stops newer work."
            },
            "approvals": {
                "agent": false,
                "reason": "Approvals belong to the user. No tool answers a provider approval or a confirmation request."
            }
        })
    }

    /// The reads this caller is authorized for, with their bounds.
    pub(crate) fn reads(&self, index: usize) -> Value {
        let remote = self.threads[index].peer_origin.is_some();
        let remote_reason = || {
            "A remote worker may read threads and memories, but cannot delegate or control unrelated work.".to_string()
        };
        json!({
            "list_threads": {
                "available": true,
                "reason": Value::Null,
                "scope": if remote {
                    "This machine's threads. The remote worker read lane is deliberate."
                } else {
                    "Local threads; linked machines advertising thread-read answer machine-pinned requests."
                }
            },
            "read_thread": {
                "available": true,
                "reason": Value::Null,
                "scope": "Any local thread; linked machines advertising thread-read. Paged, with a 50000-character text budget per call."
            },
            "task_status": {
                "available": true,
                "reason": Value::Null,
                "scope": if remote { "Your own thread only." } else { "Any local thread; only its parent may act on it." }
            },
            "captain_documents": {
                "AGENTS.md": "The captain's primary home only; never shared with crew or remote workers.",
                "shared": "CREW.md, NOTES.md and DIGEST.md.",
                "remote": {
                    "available": !remote || self.remote_document_reads_available(index),
                    "reason": self.threads[index].peer_origin.as_ref().and_then(|o| {
                        if !o.document_reads {
                            Some("The originating server cannot answer Captain document reads. Only the context in the brief is available.".to_string())
                        } else if self.origin_document_read_denial(index).is_some() {
                            Some("The originating peer is no longer authorized to serve Captain documents.".to_string())
                        } else {
                            None
                        }
                    }),
                    "bound": "offset, maxBytes and expectedRevision page within the authorized manifest."
                }
            },
            "memories": {
                "scope": "The caller's captain's memories; remote workers read the launch-time snapshot.",
                "write": "The primary captain home via remember and forget_memory.",
                "propose": "Workers stage suggestions with propose_memory; they never write memory."
            },
            "captains": gate(!remote, remote.then(remote_reason)),
        })
    }

    /// Refuse an executable-less or MCP-less harness before work allocation.
    pub(crate) fn require_harness_executable(&self, harness: &str) -> Result<(), String> {
        match harness {
            "pi" => Ok(()),
            "codex" => crate::codex::process().map(drop),
            "claude" => crate::claude::process().map(drop),
            _ => {
                let agent = self.harnesses.find(harness)?;
                let capabilities = crate::acp::probe(&agent)?;
                let mcp = &capabilities["mcpCapabilities"];
                if mcp["http"] == true || mcp["stdio"] != false {
                    Ok(())
                } else {
                    Err("This ACP agent supports neither HTTP nor stdio MCP. Shina cannot register its tools, so no thread was allocated.".into())
                }
            }
        }
    }

    /// Execution options this caller could delegate with: local harnesses with
    /// availability, exact models and the per-peer equivalents.
    pub(crate) fn execution(&self, index: usize) -> Value {
        let mut harnesses = Vec::new();
        for harness in self.harness_ids() {
            let mut entry = json!({"id": harness});
            let (ready, reason) = match harness.as_str() {
                "pi" => (
                    self.catalog.error.is_none() || !self.catalog.models.is_empty(),
                    self.catalog.error.clone(),
                ),
                "codex" => match crate::codex::process() {
                    Ok(_) => (true, None),
                    Err(error) => (false, Some(error)),
                },
                "claude" => match crate::claude::process() {
                    Ok(_) => (true, None),
                    Err(error) => (false, Some(error)),
                },
                _ => match self.harnesses.find(&harness).and_then(|agent| {
                    crate::acp::probe(&agent).map(|capabilities| (agent, capabilities))
                }) {
                    Ok((_, capabilities)) => {
                        let mcp = &capabilities["mcpCapabilities"];
                        let available = mcp["http"] == true || mcp["stdio"] != false;
                        entry["discovery"] = json!("probed");
                        entry["mcp"] = json!({
                            "available": available,
                            "http": mcp["http"],
                            "stdio": mcp["stdio"],
                            "reason": (!available).then(|| "This ACP agent advertises neither HTTP nor stdio MCP. Shina cannot register its tools.".to_string()),
                        });
                        if available {
                            (true, None)
                        } else {
                            (false, Some("This ACP agent supports neither HTTP nor stdio MCP. Shina cannot register its tools.".into()))
                        }
                    }
                    Err(error) => {
                        entry["discovery"] = json!("failed");
                        (false, Some(error))
                    }
                },
            };
            entry["executable"] = json!(ready);
            entry["reason"] = json!(reason);
            entry["models"] = json!(
                self.harness_models(&harness)
                    .iter()
                    .map(|model| json!({"id": model.id, "reasoning_levels": model.thinking_levels}))
                    .collect::<Vec<_>>()
            );
            harnesses.push(entry);
        }
        let peers = self
            .delegation_peers()
            .iter()
            .map(|peer| {
                let contracts: Vec<String> = peer.capabilities["contracts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|contract| contract.as_str().map(str::to_owned))
                    .collect();
                let has = |name: &str| contracts.iter().any(|contract| contract == name);
                let reachable = peer_reachable(peer);
                let unreachable_reason = || {
                    peer.last_error.clone().unwrap_or_else(|| {
                        "The peer has not been reached. Check the machine connection or credentials."
                            .to_string()
                    })
                };
                let requirement = |contract: &str, reason: &str| {
                    if !reachable {
                        gate(false, Some(unreachable_reason()))
                    } else {
                        gate(
                            has(contract),
                            (!has(contract)).then(|| reason.to_string()),
                        )
                    }
                };
                json!({
                    "serverId": peer.server_id,
                    "label": peer.label,
                    "reachable": reachable,
                    "contracts": contracts,
                    "delegation": requirement(protocol::PEER_TASKS,
                        "The peer did not negotiate peer tasks. Update it before delegating."),
                    "threadReads": requirement(protocol::THREAD_READ,
                        "The peer did not negotiate thread reads. list_threads and read_thread cannot cross it."),
                    "placement": requirement(protocol::PLACEMENT,
                        "The peer did not negotiate placement-v1. Worker placement requests fail on it."),
                    "documentReads": requirement(protocol::CAPTAIN_PEER_DOCUMENTS,
                        "The peer did not negotiate captain-peer-documents-v1. Workers there cannot page captain documents."),
                    "taskFences": requirement(protocol::PEER_TASK_FENCES,
                        "The peer predates task fences; fenced sends and stops to its workers are refused."),
                    "harnesses": peer.capabilities["harnesses"].clone(),
                    "models": peer.capabilities["models"].clone(),
                    "projects": peer.capabilities["projects"].clone(),
                    "lastError": peer.last_error,
                })
            })
            .collect::<Vec<_>>();
        json!({
            "local": {
                "harnesses": harnesses,
                "host": self.threads[index].machine == "local",
            },
            "peers": peers,
            "remote_independent_roots": gate(false, Some(
                "delegate_task creates delegated workers only. Independent crew roots on a linked machine are not implemented.".into(),
            )),
        })
    }

    /// Placement modes the worker contract supports locally and on peers.
    pub(crate) fn placement(&self, index: usize) -> Value {
        let remote_supported = self.delegation_peers().iter().any(|peer| {
            peer_reachable(peer)
                && peer.capabilities["contracts"]
                    .as_array()
                    .is_some_and(|contracts| {
                        contracts
                            .iter()
                            .any(|contract| contract == protocol::PLACEMENT)
                    })
        });
        let denied = self.denial(index, "delegate_task").is_some();
        json!({
            "modes": ["default", "shared", "isolated", "existing"],
            "default": "An isolated Git worktree when the workspace is a repository, otherwise the shared project checkout.",
            "shared": "A read-only view of the project checkout that sees its uncommitted changes.",
            "isolated": "A new Git worktree on its own branch. One base from baseBranch, baseCommit or upstreamThreadId.",
            "existing": "A registered worktree selected by absolute path or branch name.",
            "remote": gate(!denied && remote_supported, if denied {
                self.denial(index, "delegate_task")
            } else if !remote_supported {
                Some("No linked peer negotiated placement-v1.".into())
            } else {
                None
            }),
        })
    }

    /// Live capacity for this coordinator plus the admission seam the scheduler
    /// will fill (#23): static per-coordinator limits today, truthful about the
    /// queue, no invented scheduler capacity.
    pub(crate) fn capacity(&self, index: usize) -> Value {
        let home = self.effective_home(self.threads[index].id);
        let unfinished = self
            .tasks
            .iter()
            .filter(|task| self.task_home(task) == Some(home) && !task.state.finished())
            .count();
        let running = self
            .tasks
            .iter()
            .filter(|task| self.task_home(task) == Some(home) && self.task_occupies_slot(task))
            .count();
        let queued = self
            .tasks
            .iter()
            .filter(|task| {
                self.task_home(task) == Some(home) && task.state == crate::model::TaskState::Queued
            })
            .count();
        json!({
            "running_children_per_coordinator": tasks::RUNNING_LIMIT,
            "unfinished_tasks_per_coordinator": tasks::UNFINISHED_LIMIT,
            "running": running,
            "unfinished": unfinished,
            "queued": queued,
            "admission": "Static per-coordinator limits. Extra children queue until a running slot frees.",
            "scheduler": {
                "implemented": false,
                "reason": "Admission is per-coordinator only. Scheduler capacity and cross-coordinator ordering are not implemented."
            }
        })
    }

    /// The peers the delegation paths actually consult: the durable server's
    /// table when it exists, else the in-memory set.
    fn delegation_peers(&self) -> Vec<crate::peer::Peer> {
        crate::database::for_path(&self.data_dir.join("index.json"))
            .and_then(|database| database.peers().ok())
            .unwrap_or_else(|| self.peers.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::captain_lifecycle::{CaptainLifecycle, CaptainPhase};
    use crate::model::remote_tasks::PeerOrigin;

    fn fixture() -> State {
        let mut state = State::new(
            std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()),
            false,
        );
        state.load_global_for_test();
        state.captains = Captain::presets();
        state
    }

    fn push(state: &mut State) -> usize {
        let id = state.next_id;
        state.next_id += 1;
        state.threads.push(Thread::new(id));
        id
    }

    fn index(state: &State, id: usize) -> usize {
        state.threads.iter().position(|t| t.id == id).unwrap()
    }

    fn captain_home(state: &mut State) -> usize {
        state.open_captain(0);
        let home = state.captain_home(0).expect("captain home");
        // Real callers are mid-turn; the memory write gates order behind the
        // active-turn check.
        let i = index(state, home);
        state.threads[i].status = Status::Running;
        home
    }

    fn crew(state: &mut State) -> usize {
        let id = push(state);
        let i = index(state, id);
        let thread = &mut state.threads[i];
        thread.captain = Some(0);
        thread.crew_captain = Some(0);
        thread.status = Status::Running;
        id
    }

    fn delegated(state: &mut State, parent: usize) -> usize {
        let id = push(state);
        let i = index(state, id);
        let thread = &mut state.threads[i];
        thread.captain = Some(0);
        thread.crew_captain = Some(0);
        thread.delegated = true;
        thread.parent = Some(parent);
        thread.status = Status::Running;
        id
    }

    /// A second home for the same captain: `captain_homes` keeps pointing at
    /// the first, so this one reads as an earlier home.
    fn earlier_home(state: &mut State) -> usize {
        let id = push(state);
        let i = index(state, id);
        let thread = &mut state.threads[i];
        thread.captain_home = true;
        thread.captain = Some(0);
        thread.status = Status::Running;
        id
    }

    fn remote_worker(state: &mut State, document_reads: bool) -> usize {
        let id = push(state);
        let i = index(state, id);
        state.threads[i].peer_origin = Some(PeerOrigin {
            device_id: "origin-device".into(),
            server_id: "origin-server".into(),
            server_label: "Origin".into(),
            parent_thread_id: 1,
            task_id: "task-1".into(),
            captain: "Boss".into(),
            memories: vec![],
            result: None,
            acknowledged: false,
            attempt_message: String::new(),
            attempt_id: None,
            cancelled_commands: vec![],
            reply_after_position: 0,
            document_reads,
            inputs: Vec::new(),
        });
        state.threads[i].delegated = true;
        state.threads[i].status = Status::Running;
        id
    }

    fn tool_names() -> Vec<String> {
        crate::mcp::tools()
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Minimal arguments that pass each tool's own validation early enough to
    /// reach the static gates the record mirrors.
    fn probe_args(tool: &str, caller: usize) -> Value {
        match tool {
            "captain_crew_create" | "captain_crew_send" | "captain_crew_stop" => {
                json!({"requestId": uuid::Uuid::new_v4().to_string(), "threadId": 1})
            }
            "task_send" => json!({"client_request_id":"k","thread_id":caller,"message":"x"}),
            "task_cancel" => json!({"client_request_id":"k","thread_id":caller}),
            "task_status" => json!({"thread_id": caller}),
            "delegate_task" => json!({"prompt":"","client_request_id":"k"}),
            "captain_question_answer" => json!({
                "reportId":"crew-x","answer":"a","expectedRevision":0,
                "requestId":uuid::Uuid::new_v4().to_string()}),
            "captain_crew_report" => json!({
                "outcome":"completed","summary":"s",
                "requestId":uuid::Uuid::new_v4().to_string()}),
            "captain_document_read" => json!({"name":"CREW.md"}),
            "captain_document_write" => {
                json!({"name":"NOTES.md","text":"t","expectedRevision":0})
            }
            "captain_crew_join" | "captain_crew_release" => json!({"threadId":1}),
            "read_thread" => json!({"threadId":caller}),
            "propose_memory" | "remember" => json!({"text":"t"}),
            "forget_memory" => json!({"id":"m-1"}),
            "search_memories" | "ask_memory" => json!({"query":"q"}),
            "create_routine" => {
                json!({"name":"n","prompt":"p","schedule":{"kind":"every","minutes":30}})
            }
            "pause_routine" | "delete_routine" => json!({"id":1}),
            _ => json!({}),
        }
    }

    /// The dispatch path a tool actually takes: remote document reads and the
    /// learned-memory lane answer before `mcp_call` runs.
    fn dispatch(
        state: &mut State,
        caller: usize,
        tool: &str,
        args: &Value,
    ) -> Result<Value, String> {
        let remote = state.threads[index(state, caller)].peer_origin.is_some();
        if matches!(tool, "ask_memory" | "search_memories") {
            let (reply, receive) = mpsc::channel();
            if state.learned_memory_request(&crate::mcp::Request {
                thread: caller,
                tool: tool.into(),
                args: args.clone(),
                reply,
            }) {
                return receive.try_recv().expect("memory lane answer");
            }
        }
        if remote && tool == "captain_document_read" {
            let (reply, receive) = mpsc::channel();
            let handled = state.queue_peer_document_read(&crate::mcp::Request {
                thread: caller,
                tool: tool.into(),
                args: args.clone(),
                reply,
            });
            assert!(handled, "remote captain_document_read must be intercepted");
            // A negotiated read is held for the origin's reply; only failures
            // answer inline.
            return Ok(receive
                .try_recv()
                .unwrap_or_else(|_| Ok(json!({"held": true})))?);
        }
        state.mcp_call(caller, tool, args)
    }

    /// A listed tool may still refuse the probe for its arguments. These are
    /// the exact first argument-dependent errors in this fixture, so a listed
    /// tool cannot silently fail with a role or lifecycle refusal.
    fn documented_argument_error(tool: &str, caller: usize) -> Vec<String> {
        match tool {
            "delegate_task" => vec!["prompt must describe the task.".into()],
            "task_send" | "task_cancel" => vec![format!(
                "Only the parent that delegated thread {caller} may cancel or send to it."
            )],
            "forget_memory" => vec!["Memory does not exist.".into()],
            "create_routine" => vec!["Routine storage is not available.".into()],
            "pause_routine" | "delete_routine" => {
                vec!["This routine does not belong to the calling captain.".into()]
            }
            "captain_question_answer" => vec![
                "That question does not belong to this Captain.".into(),
                "Pass a captain id; this thread has no captain.".into(),
            ],
            "captain_crew_report" => vec![
                "This thread has no current run to report against.".into(),
                "Pass a captain id; this thread has no captain.".into(),
            ],
            // These handlers resolve the target captain before their role gate.
            "captain_read"
            | "captain_document_read"
            | "captain_document_write"
            | "captain_crew_list"
            | "captain_crew_join"
            | "captain_crew_release"
            | "captain_reports_list" => {
                vec!["Pass a captain id; this thread has no captain.".into()]
            }
            _ => Vec::new(),
        }
    }

    /// Every refusal the record reports must lead the real dispatch path. A
    /// listed tool must either succeed with the fixture's valid arguments or
    /// return one of the exact argument-dependent errors above; it may not
    /// fail with a role, lifecycle or durable-storage refusal.
    fn assert_agreement(state: &mut State, caller: usize) {
        for tool in tool_names() {
            if tool == "html_render" || tool == "html_preview" {
                continue;
            }
            let args = probe_args(&tool, caller);
            let reason = state.denial(index(state, caller), &tool);
            match (reason, dispatch(state, caller, &tool, &args)) {
                (Some(reason), Err(error)) => assert!(
                    error == reason || documented_argument_error(&tool, caller).contains(&error),
                    "{tool}: record says {reason:?}, dispatch said {error:?}"
                ),
                (Some(reason), Ok(_)) => {
                    panic!("{tool}: record says {reason} but dispatch succeeded")
                }
                (None, Ok(_)) => {}
                (None, Err(error)) => assert!(
                    documented_argument_error(&tool, caller).contains(&error),
                    "{tool}: listed but failed with an undocumented error: {error}"
                ),
            }
        }
    }

    #[test]
    fn role_matrix_labels_and_instructions() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        assert_eq!(state.mcp_role(index(&state, home)), "captain_home");
        assert_eq!(
            state.mcp_instructions(index(&state, home)),
            crate::mcp::INSTRUCTIONS
        );
        let crew_id = crew(&mut state);
        let crew_index = index(&state, crew_id);
        assert_eq!(state.mcp_role(crew_index), "crew");
        assert_eq!(state.mcp_instructions(crew_index), CREW_INSTRUCTIONS);
        assert!(CREW_INSTRUCTIONS.contains("captain_crew_report"));
        assert!(CREW_INSTRUCTIONS.contains("propose_memory"));
        assert!(!CREW_INSTRUCTIONS.contains("Captain homes can use"));
        let parent = index(&state, home);
        let leaf = delegated(&mut state, parent);
        assert_eq!(state.mcp_role(index(&state, leaf)), "worker");
        assert_eq!(
            state.mcp_instructions(index(&state, leaf)),
            WORKER_INSTRUCTIONS
        );
        let remote = remote_worker(&mut state, true);
        assert_eq!(state.mcp_role(index(&state, remote)), "remote_worker");
        assert_eq!(
            state.mcp_instructions(index(&state, remote)),
            REMOTE_WORKER_INSTRUCTIONS
        );
        // A second home thread for the same captain is an earlier home: the
        // primary-home map still points at the first.
        let earlier = earlier_home(&mut state);
        let earlier_index = index(&state, earlier);
        assert_eq!(state.captain_home(0), Some(home));
        assert_eq!(state.mcp_role(earlier_index), "earlier_home");
        assert_eq!(
            state.mcp_instructions(earlier_index),
            EARLIER_HOME_INSTRUCTIONS
        );
        assert!(EARLIER_HOME_INSTRUCTIONS.contains("Do not delegate"));
        assert!(!EARLIER_HOME_INSTRUCTIONS.contains("You may delegate"));
        assert!(!EARLIER_HOME_INSTRUCTIONS.contains("Captain homes can use"));
        assert_eq!(state.mcp_role(index(&state, home)), "captain_home");
        let plain = push(&mut state);
        let plain_index = index(&state, plain);
        assert_eq!(state.mcp_role(plain_index), "thread");
        assert_eq!(state.mcp_instructions(plain_index), THREAD_INSTRUCTIONS);
        assert!(!THREAD_INSTRUCTIONS.contains("Captain homes can use"));
        assert_agreement(&mut state, plain);
    }

    #[test]
    fn plain_threads_do_not_advertise_captain_report_reads() {
        let mut state = fixture();
        let plain = push(&mut state);
        let i = index(&state, plain);
        let reason = state.denial(i, "captain_reports_list").unwrap();
        assert_eq!(
            reason,
            "Only this captain's threads may read its crew reports."
        );
        assert!(
            !state
                .listed_tools(i)
                .iter()
                .any(|tool| tool["name"] == "captain_reports_list")
        );
        assert_eq!(
            state
                .mcp_call(plain, "captain_reports_list", &json!({"captain":0}))
                .unwrap_err(),
            reason
        );
    }

    #[test]
    fn delegated_leaves_do_not_advertise_task_control() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        let parent = index(&state, home);
        let leaf = delegated(&mut state, parent);
        let i = index(&state, leaf);
        for tool in ["task_send", "task_cancel"] {
            assert_eq!(
                state.denial(i, tool).as_deref(),
                Some("Delegated threads cannot control other tasks.")
            );
            assert!(
                !state
                    .listed_tools(i)
                    .iter()
                    .any(|item| item["name"] == tool),
                "{tool} listed for a delegated leaf"
            );
        }
        assert_eq!(
            dispatch(
                &mut state,
                leaf,
                "task_send",
                &json!({"client_request_id":"k","thread_id":leaf,"message":"x"})
            )
            .unwrap_err(),
            format!("Only the parent that delegated thread {leaf} may cancel or send to it.")
        );
    }

    #[test]
    fn archived_primary_home_does_not_advertise_question_answers() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        let report: crate::model::crew_reports::CrewReport = serde_json::from_value(json!({
            "id":format!("crew-{}", uuid::Uuid::new_v4()),
            "captain":0,"threadId":1,"runId":"run-a","title":"Crew",
            "outcome":"completed","summary":"Waiting","error":"","createdAt":1,
            "delivered":false,
            "question":{"text":"Proceed?","recipient":"captain"}
        }))
        .unwrap();
        let report_id = report.id.clone();
        state.captain_reports.push(report);
        state.captain_lifecycles.insert(
            0,
            CaptainLifecycle {
                phase: CaptainPhase::Archived,
                archived_threads: [(home, None)].into_iter().collect(),
                ..Default::default()
            },
        );
        let i = index(&state, home);
        let reason = state.denial(i, "captain_question_answer").unwrap();
        assert_eq!(reason, "This Captain is archived. Unarchive it first.");
        assert!(
            !state
                .listed_tools(i)
                .iter()
                .any(|tool| tool["name"] == "captain_question_answer")
        );
        assert_eq!(
            state
                .mcp_call(
                    home,
                    "captain_question_answer",
                    &json!({"reportId":report_id,"answer":"yes",
                        "expectedRevision":0,"requestId":uuid::Uuid::new_v4().to_string()})
                )
                .unwrap_err(),
            reason
        );
    }

    #[test]
    fn legacy_booleans_match_the_effective_coordination_gates() {
        fn check(state: &mut State, caller: usize) {
            let record = state.orchestrator_capabilities(index(state, caller));
            let caller = &record["caller"];
            let coordination = &record["effective"]["coordination"];
            assert_eq!(
                caller["may_delegate"],
                coordination["delegate_local"]["available"]
            );
            assert_eq!(
                caller["may_control_crew"], coordination["crew"]["create"]["available"],
                "may_control_crew contradicted crew.create"
            );
            assert_eq!(
                caller["may_write_captain_documents"],
                coordination["captain_document_write"]["available"]
            );
            assert_eq!(
                caller["may_answer_questions"],
                coordination["captain_question_answer"]["available"]
            );
            assert_eq!(
                caller["may_manage_routines"],
                coordination["routines"]["available"]
            );
        }
        let mut state = fixture();
        let home = captain_home(&mut state);
        check(&mut state, home);
        let member = crew(&mut state);
        check(&mut state, member);
        let parent = index(&state, home);
        let leaf = delegated(&mut state, parent);
        check(&mut state, leaf);
        let plain = push(&mut state);
        check(&mut state, plain);
        let remote = remote_worker(&mut state, false);
        check(&mut state, remote);
        state.captain_lifecycles.insert(
            0,
            CaptainLifecycle {
                phase: CaptainPhase::Archived,
                archived_threads: [(home, None)].into_iter().collect(),
                ..Default::default()
            },
        );
        check(&mut state, home);
    }

    #[test]
    fn primary_home_lists_the_full_catalog() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        assert_agreement(&mut state, home);
        let record = state.tools_record(index(&state, home));
        assert_eq!(record.as_object().unwrap().len(), tool_names().len());
        // Every tool is listed for an active primary home on this server: no
        // durable database is present, so crew control reports that reason but
        // the reason comes from the same gate dispatch consults.
        for (tool, entry) in record.as_object().unwrap() {
            if let Some(reason) = entry["reason"].as_str() {
                assert_eq!(entry["listed"], false, "{tool}");
                assert!(!reason.is_empty());
            } else {
                assert_eq!(entry["listed"], true, "{tool}");
            }
        }
    }

    #[test]
    fn crew_can_report_and_read_but_not_coordinate() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        let member = crew(&mut state);
        let _ = home;
        assert_agreement(&mut state, member);
        let i = index(&state, member);
        // Crew are independent coordinators: they delegate their own leaves,
        // they just cannot control the Captain's crew or documents.
        for tool in [
            "captain_crew_create",
            "captain_crew_send",
            "captain_crew_stop",
            "captain_crew_join",
            "captain_crew_release",
            "captain_document_write",
            "captain_question_answer",
            "create_routine",
            "remember",
            "ask_memory",
        ] {
            assert!(
                state.denial(i, tool).is_some(),
                "{tool} must be denied to crew"
            );
        }
        for tool in [
            "captain_crew_report",
            "captain_document_read",
            "propose_memory",
            "list_memories",
            "list_threads",
            "read_thread",
            "delegate_task",
            "orchestrator_capabilities",
        ] {
            assert!(state.denial(i, tool).is_none(), "{tool} denied to crew");
        }
        assert_eq!(
            state
                .mcp_meta_call(member, "tools/list", &json!({}))
                .unwrap()["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "captain_crew_report"),
            true
        );
    }

    #[test]
    fn delegated_leaf_is_a_leaf_coordinator_for_nothing() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        let parent = index(&state, home);
        let leaf = delegated(&mut state, parent);
        assert_agreement(&mut state, leaf);
        let i = index(&state, leaf);
        assert_eq!(
            state.denial(i, "delegate_task").as_deref(),
            Some("Delegated threads cannot delegate further. Do the work in this thread.")
        );
        let listed: Vec<String> = state
            .listed_tools(i)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(listed.contains(&"propose_memory".to_owned()));
        assert!(!listed.contains(&"delegate_task".to_owned()));
        assert!(!listed.contains(&"remember".to_owned()));
        assert!(!listed.contains(&"captain_crew_create".to_owned()));
    }

    #[test]
    fn remote_leaf_reads_but_cannot_act_and_documents_follow_the_contract() {
        let mut state = fixture();
        let without = remote_worker(&mut state, false);
        assert_agreement(&mut state, without);
        let i = index(&state, without);
        assert_eq!(
            state.denial(i, "captain_document_read").as_deref(),
            Some(
                "The originating server cannot answer Captain document reads. Only the context in the brief is available."
            )
        );
        assert_eq!(
            state.denial(i, "delegate_task").as_deref(),
            Some(
                "A remote worker may read threads and memories, but cannot delegate or control unrelated work."
            )
        );
        for tool in [
            "list_threads",
            "read_thread",
            "list_memories",
            "search_memories",
            "propose_memory",
            "orchestrator_capabilities",
        ] {
            assert!(state.denial(i, tool).is_none(), "{tool} denied remotely");
        }
        let with = remote_worker(&mut state, true);
        let j = index(&state, with);
        assert!(state.denial(j, "captain_document_read").is_none());
    }

    #[test]
    fn earlier_home_and_archived_and_retired_states_lose_coordination() {
        let mut state = fixture();
        let _current = captain_home(&mut state);
        let first = earlier_home(&mut state);
        let i = index(&state, first);
        assert_eq!(
            state.denial(i, "delegate_task").as_deref(),
            Some(
                "Open the captain's primary home to delegate new work. This is an earlier conversation."
            )
        );
        assert_eq!(
            state.denial(i, "captain_document_write").as_deref(),
            Some("Only this captain's primary home may write its planning documents.")
        );
        // Archived: the group freezes; even the primary home loses act rights.
        let mut archived = fixture();
        let home = captain_home(&mut archived);
        archived.captain_lifecycles.insert(
            0,
            CaptainLifecycle {
                phase: CaptainPhase::Archived,
                archived_threads: [(home, None)].into_iter().collect(),
                ..Default::default()
            },
        );
        assert_agreement(&mut archived, home);
        let archived_index = index(&archived, home);
        archived.refresh_captain_contexts();
        assert_eq!(archived.mcp_role(archived_index), "archived_home");
        assert_eq!(
            archived.mcp_instructions(archived_index),
            ARCHIVED_INSTRUCTIONS
        );
        let init = archived
            .mcp_meta_call(home, "initialize", &json!({}))
            .unwrap();
        assert_eq!(init["instructions"], ARCHIVED_INSTRUCTIONS);
        let error = archived.denial(archived_index, "delegate_task").unwrap();
        assert!(error.contains("archived"), "{error}");
        // Reads stay: archived homes still answer them.
        assert!(
            archived
                .denial(index(&archived, home), "read_thread")
                .is_none()
        );
        // Retired is a read-only record: the same owner gate refuses writes.
        let mut retired = fixture();
        let home = captain_home(&mut retired);
        retired.captain_lifecycles.insert(
            0,
            CaptainLifecycle {
                phase: CaptainPhase::Retired,
                ..Default::default()
            },
        );
        retired.refresh_captain_contexts();
        assert_eq!(
            retired.mcp_instructions(index(&retired, home)),
            RETIRED_INSTRUCTIONS
        );
        let error = retired
            .denial(index(&retired, home), "delegate_task")
            .unwrap();
        assert!(error.contains("read-only"), "{error}");
    }

    #[test]
    fn a_live_session_receives_role_changes_on_its_next_prompt() {
        let mut state = fixture();
        let _home = captain_home(&mut state);
        let plain = push(&mut state);
        let i = index(&state, plain);
        let (pi, commands, _events) = crate::model::Pi::test_connection();
        state.threads[i].pi = Some(pi);
        state.threads[i].harness_context_sent = true;
        state.threads[i].instruction_role = Some("thread");
        state.threads[i].shina = THREAD_INSTRUCTIONS;
        state.threads[i].crew_captain = Some(0);
        state.threads[i].captain = Some(0);
        state.refresh_captain_contexts();
        assert_eq!(state.mcp_role(i), "crew");
        assert!(state.threads[i].instruction_context_pending);
        assert!(!state.threads[i].harness_context_sent);
        assert!(
            state.threads[i].command("prompt", json!({"message":"continue"})),
            "the next prompt must be accepted"
        );
        let text = commands.try_recv().unwrap()["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(text.starts_with("<shina_capability_update>"), "{text}");
        assert!(text.contains("independent crew thread"), "{text}");
        assert!(text.contains("captain_crew_report"), "{text}");
        assert!(text.contains("propose_memory"), "{text}");
        assert!(!state.threads[i].instruction_context_pending);
        assert!(
            state.threads[i].command("prompt", json!({"message":"next"})),
            "the update is delivered once"
        );
        assert_eq!(commands.try_recv().unwrap()["message"], "next");
    }

    #[test]
    fn reset_rebuilds_the_old_homes_current_instructions() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        assert_eq!(
            state.mcp_instructions(index(&state, home)),
            crate::mcp::INSTRUCTIONS
        );
        let old_index = index(&state, home);
        state.threads[old_index].status = Status::Completed;
        state.reset_captain_conversation(0).unwrap();
        let new_home = state.captain_home(0).unwrap();
        assert_ne!(new_home, home);
        let old = index(&state, home);
        assert_eq!(state.mcp_role(old), "earlier_home");
        assert_eq!(state.mcp_instructions(old), EARLIER_HOME_INSTRUCTIONS);
        assert_eq!(
            state.mcp_meta_call(home, "initialize", &json!({})).unwrap()["instructions"],
            EARLIER_HOME_INSTRUCTIONS
        );
        assert_eq!(
            state
                .mcp_meta_call(new_home, "initialize", &json!({}))
                .unwrap()["instructions"],
            crate::mcp::INSTRUCTIONS
        );
    }

    #[test]
    fn meta_calls_answer_from_the_callers_record() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        let parent = index(&state, home);
        let leaf = delegated(&mut state, parent);
        let init = state
            .mcp_meta_call(home, "initialize", &json!({"protocolVersion":"x"}))
            .unwrap();
        assert_eq!(init["protocolVersion"], "x");
        assert_eq!(init["serverInfo"]["name"], "Shina");
        assert_eq!(init["instructions"], crate::mcp::INSTRUCTIONS);
        let worker_init = state.mcp_meta_call(leaf, "initialize", &json!({})).unwrap();
        assert_eq!(worker_init["instructions"], WORKER_INSTRUCTIONS);
        let worker_list = state.mcp_meta_call(leaf, "tools/list", &json!({})).unwrap();
        let names: Vec<&str> = worker_list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"propose_memory"));
        assert!(!names.contains(&"delegate_task"));
        assert!(!names.contains(&"captain_crew_report"));
        // A thread that no longer exists gets a real error, not a stale record.
        assert!(
            state
                .mcp_meta_call(usize::MAX, "tools/list", &json!({}))
                .is_err()
        );
    }

    #[test]
    fn effective_sections_report_peers_capacity_and_unsupported_remote_roots() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        state.peers.push(crate::peer::Peer {
            server_id: "old-peer".into(),
            host: "127.0.0.1:1".into(),
            label: "Old".into(),
            linked_at: 0,
            capabilities: json!({"contracts":[crate::protocol::PEER_TASKS]}),
            last_reached_at: Some(1),
            last_error: None,
        });
        let i = index(&state, home);
        let execution = state.execution(i);
        let peer = &execution["peers"][0];
        assert_eq!(peer["delegation"]["available"], true);
        for contract in ["threadReads", "placement", "documentReads", "taskFences"] {
            assert_eq!(
                peer[contract]["available"], false,
                "{contract} must be refused for a peer missing the contract"
            );
            assert!(!peer[contract]["reason"].as_str().unwrap().is_empty());
        }
        assert_eq!(execution["remote_independent_roots"]["available"], false);
        let placement = state.placement(i);
        assert_eq!(
            placement["remote"]["available"], false,
            "a peer without placement-v1 cannot host placed workers"
        );
        let capacity = state.capacity(i);
        assert_eq!(capacity["scheduler"]["implemented"], false);
        assert_eq!(
            capacity["running_children_per_coordinator"],
            tasks::RUNNING_LIMIT
        );
        let coordination = state.coordination(i);
        assert_eq!(coordination["delegate_local"]["available"], true);
        assert_eq!(
            coordination["delegate_remote"]["available"], true,
            "peer-tasks-v2 alone admits delegation; document reads and fences stay gated separately"
        );
        assert_eq!(coordination["delegate_remote"]["peers"][0], "old-peer");
        // remote_workers see the record too: remote reads are bounded and honest.
        let remote = remote_worker(&mut state, false);
        let reads = state.reads(index(&state, remote));
        assert_eq!(reads["captain_documents"]["remote"]["available"], false);
        assert_eq!(reads["task_status"]["scope"], "Your own thread only.");
    }

    #[test]
    fn unreachable_or_dead_peers_close_remote_delegation_and_reads() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        state.peers.push(crate::peer::Peer {
            server_id: "dead-peer".into(),
            host: "127.0.0.1:1".into(),
            label: "Dead".into(),
            linked_at: 0,
            capabilities: json!({"contracts":[
                crate::protocol::PEER_TASKS,
                crate::protocol::THREAD_READ,
                crate::protocol::PLACEMENT,
                crate::protocol::CAPTAIN_PEER_DOCUMENTS
            ]}),
            last_reached_at: Some(1),
            last_error: Some("Authentication denied. Device unknown or revoked.".into()),
        });
        let coordination = state.coordination(index(&state, home));
        assert_eq!(coordination["delegate_remote"]["available"], false);
        assert!(
            coordination["delegate_remote"]["reason"]
                .as_str()
                .unwrap()
                .contains("Authentication denied"),
            "{coordination}"
        );
        let execution = state.execution(index(&state, home));
        let peer = &execution["peers"][0];
        for field in ["delegation", "threadReads", "placement", "documentReads"] {
            assert_eq!(peer[field]["available"], false, "{field}: {peer}");
            assert!(
                peer[field]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("Authentication denied"),
                "{field}: {peer}"
            );
        }
        assert_eq!(
            state.placement(index(&state, home))["remote"]["available"],
            false
        );
    }

    #[test]
    fn a_revoked_origin_closes_remote_document_reads() {
        let mut state = fixture();
        let remote = remote_worker(&mut state, true);
        let i = index(&state, remote);
        let database = crate::database::Database::open(&state.data_dir).unwrap();
        database
            .issue_scoped_device("origin-device", "origin-token", &["peer-delegate"])
            .unwrap();
        assert!(state.denial(i, "captain_document_read").is_none());
        assert_eq!(
            state.reads(i)["captain_documents"]["remote"]["available"],
            true
        );
        database.revoke("origin-device").unwrap();
        assert_eq!(
            state.denial(i, "captain_document_read").as_deref(),
            Some("The originating peer is no longer authorized to serve Captain documents.")
        );
        assert_eq!(
            state.reads(i)["captain_documents"]["remote"]["available"],
            false
        );
        assert_eq!(
            dispatch(
                &mut state,
                remote,
                "captain_document_read",
                &json!({"name":"CREW.md"})
            )
            .unwrap_err(),
            "The originating peer is no longer authorized to serve Captain documents."
        );
    }

    #[test]
    fn orchestrator_capabilities_carries_the_whole_record() {
        let mut state = fixture();
        let home = captain_home(&mut state);
        let capabilities = state.orchestrator_capabilities(index(&state, home));
        assert_eq!(capabilities["contract"], CONTRACT);
        for section in [
            "coordination",
            "interaction",
            "reads",
            "execution",
            "placement",
            "capacity",
        ] {
            assert!(
                capabilities["effective"][section].is_object(),
                "{section} missing"
            );
        }
        assert_eq!(
            capabilities["tools"].as_object().unwrap().len(),
            tool_names().len()
        );
        assert_eq!(capabilities["caller"]["role"], "captain_home");
        assert_eq!(capabilities["caller"]["access"], "read-only");
    }
}
