//! One durable admission scheduler for every provider-start route: delegated
//! task attempts (local and remote), independent crew runs and peer-launched
//! workers. A reservation commits with the work it gates, carries an
//! observable queue reason, and releases exactly once on an explicit terminal
//! transition or on reconciliation. Disconnected remote work stays held and
//! uncertain until its outcome is proven; it is never silently replaced.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Defaults preserve single-user behavior: the delegated caps that already
/// applied, plus account, executor and provider ceilings so independent roots
/// cannot start more providers than one coordinator was ever allowed to run.
pub const COORDINATOR_RUNNING: usize = 3;
pub const COORDINATOR_UNFINISHED: usize = 8;
pub const ACCOUNT_RUNNING: usize = 8;
pub const ACCOUNT_PENDING: usize = 64;
pub const EXECUTOR_RUNNING: usize = 8;
pub const PROVIDER_RUNNING: usize = 8;
/// Released reservations kept as a bounded audit ledger.
const RELEASED_KEEP: usize = 200;
/// A provider refusal backs its scope off before the next grant.
const BACKOFF_BASE_MS: u64 = 15_000;
const BACKOFF_MAX_MS: u64 = 300_000;

fn default_coordinator_running() -> Option<usize> {
    Some(COORDINATOR_RUNNING)
}
fn default_coordinator_unfinished() -> Option<usize> {
    Some(COORDINATOR_UNFINISHED)
}
fn default_account_running() -> Option<usize> {
    Some(ACCOUNT_RUNNING)
}
fn default_account_pending() -> Option<usize> {
    Some(ACCOUNT_PENDING)
}
fn default_executor_running() -> Option<usize> {
    Some(EXECUTOR_RUNNING)
}
fn default_provider_running() -> Option<usize> {
    Some(PROVIDER_RUNNING)
}

/// Effective limits. `None` means unlimited; `Some(n)` requires n >= 1, so
/// `Some(0)` cannot accidentally mean "pause everything" or "unlimited".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Providers running under one coordinator (Captain home or parent thread).
    #[serde(default = "default_coordinator_running")]
    pub coordinator_running: Option<usize>,
    /// Unfinished delegated tasks admitted per coordinator.
    #[serde(default = "default_coordinator_unfinished")]
    pub coordinator_unfinished: Option<usize>,
    /// Providers this account may run across every coordinator.
    #[serde(default = "default_account_running")]
    pub account_running: Option<usize>,
    /// Live reservations waiting for capacity, across all subjects.
    #[serde(default = "default_account_pending")]
    pub account_pending: Option<usize>,
    /// Providers per executor ("local" or a linked peer's server id). For
    /// remote work this server only counts the reservations it dispatched;
    /// the target's own scheduler is authoritative there.
    #[serde(default = "default_executor_running")]
    pub executor_running: Option<usize>,
    /// Providers per provider id (cpa, codex, claude, acp) across machines.
    #[serde(default = "default_provider_running")]
    pub provider_running: Option<usize>,
    /// Providers per local project. Remote work does not map onto local
    /// project ids, so it does not count here.
    #[serde(default)]
    pub workspace_running: Option<usize>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            coordinator_running: default_coordinator_running(),
            coordinator_unfinished: default_coordinator_unfinished(),
            account_running: default_account_running(),
            account_pending: default_account_pending(),
            executor_running: default_executor_running(),
            provider_running: default_provider_running(),
            workspace_running: None,
        }
    }
}

impl Limits {
    fn valid(&self) -> bool {
        [
            self.coordinator_running,
            self.coordinator_unfinished,
            self.account_running,
            self.account_pending,
            self.executor_running,
            self.provider_running,
            self.workspace_running,
        ]
        .iter()
        .all(|limit| limit.is_none_or(|limit| limit >= 1))
    }

    pub fn value(&self) -> Value {
        json!({
            "coordinatorRunning": self.coordinator_running,
            "coordinatorUnfinished": self.coordinator_unfinished,
            "accountRunning": self.account_running,
            "accountPending": self.account_pending,
            "executorRunning": self.executor_running,
            "providerRunning": self.provider_running,
            "workspaceRunning": self.workspace_running,
        })
    }
}

/// What a reservation keys to: the actual attempt or run, never merely a
/// task or thread. A superseded identity can never pass for newer work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Subject {
    /// A delegated task attempt, identified by the stable attempt id the
    /// task fences already track.
    Task { task_id: String, attempt_id: String },
    /// A crew member's provider occupancy. `run` is the member's monotonic
    /// admission run, stored on the thread itself.
    Crew { thread_id: usize, run: u64 },
    /// A worker launched by a linked peer; same run discipline as crew.
    PeerWorker { thread_id: usize, run: u64 },
}

impl Subject {
    fn thread_id(&self) -> Option<usize> {
        match self {
            Self::Crew { thread_id, .. } | Self::PeerWorker { thread_id, .. } => Some(*thread_id),
            Self::Task { .. } => None,
        }
    }

    fn value(&self) -> Value {
        match self {
            Self::Task {
                task_id,
                attempt_id,
            } => json!({
                "kind": "task", "taskId": task_id, "attemptId": attempt_id,
            }),
            Self::Crew { thread_id, run } => json!({
                "kind": "crew", "threadId": thread_id, "run": run,
            }),
            Self::PeerWorker { thread_id, run } => json!({
                "kind": "peer_worker", "threadId": thread_id, "run": run,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationState {
    Queued,
    Held,
    Released,
}

impl ReservationState {
    fn id(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Held => "held",
            Self::Released => "released",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reservation {
    pub id: String,
    pub subject: Subject,
    pub state: ReservationState,
    /// Coordinator home thread id. Local work only; peer workers on this
    /// server have no local coordinator and are not capped per-coordinator.
    #[serde(default)]
    pub coordinator: Option<usize>,
    /// Fairness key: `coordinator:<home>` for local work, `origin:<server>`
    /// for peer-launched workers. One busy key cannot starve the others.
    #[serde(default)]
    pub fair: String,
    /// "local" or a peer server id.
    #[serde(default)]
    pub executor: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub workspace: Option<usize>,
    #[serde(default)]
    pub captain: Option<usize>,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub queued_at: u64,
    #[serde(default)]
    pub granted_at: Option<u64>,
    #[serde(default)]
    pub released_at: Option<u64>,
    #[serde(default)]
    pub release_reason: Option<String>,
    /// Why a queued reservation has not started: offline, recovering,
    /// placement, placement_failed, provider_update, owner_inactive,
    /// worker_busy, provider_backoff, or a scope name.
    #[serde(default)]
    pub blocked_by: Option<String>,
    #[serde(default)]
    pub not_before: u64,
    /// Held while its outcome is unproven (restart or target unreachable).
    #[serde(default)]
    pub uncertain: bool,
}

impl Reservation {
    fn value(&self, position: Option<usize>) -> Value {
        json!({
            "reservationId": self.id,
            "subject": self.subject.value(),
            "state": self.state.id(),
            "position": position,
            "coordinator": self.coordinator,
            "fair": self.fair,
            "executor": self.executor,
            "provider": self.provider,
            "workspace": self.workspace,
            "captain": self.captain,
            "priority": self.priority,
            "queuedAt": self.queued_at,
            "grantedAt": self.granted_at,
            "releasedAt": self.released_at,
            "releaseReason": self.release_reason,
            "blockedBy": self.blocked_by,
            "notBefore": self.not_before,
            "uncertain": self.uncertain,
        })
    }
}

/// The durable scheduler record, persisted inside the same index/batch as
/// the tasks and threads it gates.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Scheduler {
    pub limits: Limits,
    pub reservations: Vec<Reservation>,
    /// Provider id -> epoch millis until which that scope grants nothing.
    pub provider_backoff: BTreeMap<String, u64>,
    /// Consecutive refusals per provider; reset by a completed release.
    pub backoff_strikes: BTreeMap<String, u32>,
    pub sequence: u64,
}

/// Held reservations tallied per scope.
#[derive(Default)]
struct HeldCounts {
    account: usize,
    coordinator: HashMap<usize, usize>,
    fair: HashMap<String, usize>,
    executor: HashMap<String, usize>,
    provider: HashMap<String, usize>,
    workspace: HashMap<usize, usize>,
}

impl HeldCounts {
    fn fair_load(&self, key: &str) -> usize {
        self.fair.get(key).copied().unwrap_or(0)
    }
}

impl Scheduler {
    fn held_counts(&self) -> HeldCounts {
        let mut counts = HeldCounts::default();
        for reservation in self
            .reservations
            .iter()
            .filter(|r| r.state == ReservationState::Held)
        {
            counts.account += 1;
            if let Some(home) = reservation.coordinator {
                *counts.coordinator.entry(home).or_insert(0) += 1;
            }
            *counts.fair.entry(reservation.fair.clone()).or_insert(0) += 1;
            *counts
                .executor
                .entry(reservation.executor.clone())
                .or_insert(0) += 1;
            *counts
                .provider
                .entry(reservation.provider.clone())
                .or_insert(0) += 1;
            if let Some(workspace) = reservation.workspace {
                *counts.workspace.entry(workspace).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Queued reservations in grant order: priority first, then the least
    /// loaded fairness key, then FIFO.
    fn grant_order(&self, counts: &HeldCounts) -> Vec<usize> {
        let mut queued: Vec<usize> = self
            .reservations
            .iter()
            .enumerate()
            .filter(|(_, r)| r.state == ReservationState::Queued)
            .map(|(index, _)| index)
            .collect();
        queued.sort_by(|&a, &b| {
            let (a, b) = (&self.reservations[a], &self.reservations[b]);
            b.priority
                .cmp(&a.priority)
                .then(counts.fair_load(&a.fair).cmp(&counts.fair_load(&b.fair)))
                .then(a.queued_at.cmp(&b.queued_at))
                .then(a.id.cmp(&b.id))
        });
        queued
    }

    /// The 1-based position of a queued reservation in grant order.
    pub(super) fn queue_position(&self, id: &str) -> Option<usize> {
        let counts = self.held_counts();
        self.grant_order(&counts)
            .iter()
            .position(|index| self.reservations[*index].id == id)
            .map(|position| position + 1)
    }

    /// Pre-creation backpressure: the durable queue is bounded so retries
    /// cannot accumulate unbounded pending work.
    pub(super) fn check_pending(&self) -> Result<(), String> {
        if let Some(limit) = self.limits.account_pending
            && self
                .reservations
                .iter()
                .filter(|r| r.state == ReservationState::Queued)
                .count()
                >= limit
        {
            return Err(format!(
                "The admission queue is full ({limit} pending reservations). Finish or dismiss work before starting more."
            ));
        }
        Ok(())
    }

    fn next_id(&mut self) -> String {
        self.sequence = self.sequence.saturating_add(1);
        format!("adm-{}", self.sequence)
    }

    /// Restore-time validation. Corrupt durable state fails like any other
    /// record; superseded or vanished subjects are not errors here because
    /// reconciliation releases them on the next pump.
    pub(super) fn validate(&mut self) -> Result<(), String> {
        if !self.limits.valid() {
            return Err("The saved scheduler has an invalid limit.".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut live = 0usize;
        for reservation in &self.reservations {
            if reservation.id.is_empty()
                || reservation.id.len() > 64
                || !ids.insert(reservation.id.clone())
                || reservation.fair.len() > 256
                || reservation.executor.len() > 128
                || reservation.provider.len() > 64
                || reservation
                    .release_reason
                    .as_ref()
                    .is_some_and(|reason| reason.len() > 64)
            {
                return Err("The saved scheduler contains an invalid reservation.".into());
            }
            match &reservation.subject {
                Subject::Task {
                    task_id,
                    attempt_id,
                } => {
                    if !tasks::valid_id(task_id) || !tasks::valid_id(attempt_id) {
                        return Err(
                            "The saved scheduler contains an invalid task reservation.".into()
                        );
                    }
                }
                Subject::Crew { .. } | Subject::PeerWorker { .. } => {}
            }
            if matches!(reservation.state, ReservationState::Released) {
                if reservation.released_at.is_none() {
                    return Err(
                        "The saved scheduler contains a reservation released without a timestamp."
                            .into(),
                    );
                }
            } else {
                live += 1;
            }
        }
        if live > 4_096 {
            return Err("The saved scheduler holds too many live reservations.".into());
        }
        if self.provider_backoff.len() > 64 || self.backoff_strikes.len() > 64 {
            return Err("The saved scheduler has unbounded provider backoff state.".into());
        }
        // Sequence must never collide with a surviving id.
        let highest = self
            .reservations
            .iter()
            .filter_map(|r| r.id.strip_prefix("adm-"))
            .filter_map(|suffix| suffix.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        self.sequence = self.sequence.max(highest);
        Ok(())
    }

    /// Drop the oldest released records past the audit bound.
    fn prune_released(&mut self) {
        let released = self
            .reservations
            .iter()
            .filter(|r| r.state == ReservationState::Released)
            .count();
        if released <= RELEASED_KEEP {
            return;
        }
        let mut drop = released - RELEASED_KEEP;
        self.reservations.retain(|r| {
            if drop > 0 && r.state == ReservationState::Released {
                drop -= 1;
                false
            } else {
                true
            }
        });
    }
}

/// What a provider refusal looks like, matched case-insensitively.
const RATE_LIMIT_SIGNALS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "rate-limit",
    "rate limiter",
    "429",
    "too many requests",
    "quota exceeded",
    "insufficient_quota",
];

fn rate_limited(error: &str) -> bool {
    let error = error.to_lowercase();
    RATE_LIMIT_SIGNALS
        .iter()
        .any(|signal| error.contains(signal))
}

impl State {
    /// Provider id a reservation counts under.
    fn thread_provider(thread: &Thread) -> String {
        if thread.model_provider.is_empty() {
            if thread.harness == "pi" {
                "cpa".into()
            } else {
                thread.harness.clone()
            }
        } else {
            thread.model_provider.clone()
        }
    }

    fn task_provider(&self, task: &Task) -> String {
        task.model_target
            .as_ref()
            .and_then(|target| target["provider"].as_str())
            .map(str::to_owned)
            .or_else(|| {
                self.threads
                    .iter()
                    .find(|t| t.id == task.child_thread_id)
                    .map(Self::thread_provider)
            })
            .unwrap_or_else(|| "cpa".into())
    }

    /// The live reservation a subject currently holds, if any.
    fn live_reservation_for(&self, thread_id: usize) -> Option<&Reservation> {
        self.scheduler.reservations.iter().find(|reservation| {
            reservation.state != ReservationState::Released
                && reservation.subject.thread_id() == Some(thread_id)
        })
    }

    /// `connect_thread` calls this before spawning: a queued reservation
    /// blocks every provider-start route, not only the scheduler's own
    /// grants. Held and released reservations never block; unmanaged
    /// (interactive) threads have no reservation and are unaffected.
    pub(super) fn admission_blocks_connect(&self, thread_id: usize) -> bool {
        self.scheduler.reservations.iter().any(|reservation| {
            reservation.state == ReservationState::Queued
                && match &reservation.subject {
                    Subject::Task {
                        task_id,
                        attempt_id,
                    } => self.tasks.iter().any(|task| {
                        task.id == *task_id
                            && task.child_thread_id == thread_id
                            && task.attempt_id == *attempt_id
                    }),
                    Subject::Crew { thread_id: id, .. }
                    | Subject::PeerWorker { thread_id: id, .. } => *id == thread_id,
                }
        })
    }

    /// A new delegated task attempt reserves admission before its thread can
    /// connect. A still-queued reservation adopts the new attempt identity so
    /// a pre-start correction keeps queue position; any other live
    /// reservation for an older attempt is superseded.
    pub(super) fn admit_task_index(&mut self, task_index: usize) {
        let task = &self.tasks[task_index];
        let subject = Subject::Task {
            task_id: task.id.clone(),
            attempt_id: task.attempt_id.clone(),
        };
        let task_id = task.id.clone();
        let coordinator = self.task_home(task);
        let executor = task
            .remote
            .as_ref()
            .map(|remote| remote.server_id.clone())
            .unwrap_or_else(|| "local".into());
        let provider = self.task_provider(task);
        let workspace = if task.remote.is_some() {
            None
        } else {
            task.project_id
        };
        let captain = task.coordinator_captain;
        let fair = coordinator
            .map(|home| format!("coordinator:{home}"))
            .unwrap_or_else(|| format!("task:{task_id}"));
        for index in 0..self.scheduler.reservations.len() {
            let stale = matches!(
                &self.scheduler.reservations[index].subject,
                Subject::Task { task_id: id, .. } if *id == task_id
            ) && self.scheduler.reservations[index].state != ReservationState::Released;
            if !stale {
                continue;
            }
            if self.scheduler.reservations[index].state == ReservationState::Queued {
                let reservation = &mut self.scheduler.reservations[index];
                reservation.subject = subject;
                reservation.coordinator = coordinator;
                reservation.fair = fair;
                reservation.executor = executor;
                reservation.provider = provider;
                reservation.workspace = workspace;
                reservation.captain = captain;
                reservation.blocked_by = None;
                reservation.uncertain = false;
            } else {
                self.release_reservation_at(index, "superseded");
            }
            return;
        }
        if let Some(cap) = self.scheduler.limits.account_pending {
            let queued = self
                .scheduler
                .reservations
                .iter()
                .filter(|r| r.state == ReservationState::Queued)
                .count();
            if queued >= cap {
                // The account queue is a bounded wait, not an unbounded
                // backlog; overflow fails the attempt like a refused start.
                let error = format!("The admission queue is full ({cap} pending).");
                self.tasks[task_index].finish(TaskState::Failed, "", &error);
                return;
            }
        }
        let id = self.scheduler.next_id();
        self.scheduler.reservations.push(Reservation {
            id,
            subject,
            state: ReservationState::Queued,
            coordinator,
            fair,
            executor,
            provider,
            workspace,
            captain,
            priority: 0,
            queued_at: lifecycle::now(),
            granted_at: None,
            released_at: None,
            release_reason: None,
            blocked_by: None,
            not_before: 0,
            uncertain: false,
        });
    }

    /// A crew member or peer worker occupies admission while its provider or
    /// pending work lives. `new_attempt` releases the live reservation and
    /// issues a fresh run, which is how an `attempt` delivery supersedes the
    /// worker's current occupancy without a duplicate.
    pub(super) fn ensure_thread_admission(
        &mut self,
        index: usize,
        peer_worker: bool,
        new_attempt: bool,
    ) {
        let thread_id = self.threads[index].id;
        if let Some(current) = self.threads[index].admission.clone() {
            if let Some(position) = self
                .scheduler
                .reservations
                .iter()
                .position(|r| r.id == current && r.state != ReservationState::Released)
            {
                if !new_attempt {
                    return;
                }
                self.release_reservation_at(position, "superseded");
            }
        }
        self.threads[index].admission_run += 1;
        let run = self.threads[index].admission_run;
        let thread = &self.threads[index];
        let captain = if peer_worker {
            None
        } else {
            thread.crew_captain
        };
        let coordinator = captain.and_then(|captain| self.captain_home(captain));
        let fair = if let Some(origin) = thread.peer_origin.as_ref() {
            format!("origin:{}", origin.server_id)
        } else if let Some(home) = coordinator {
            format!("coordinator:{home}")
        } else {
            format!("thread:{thread_id}")
        };
        let provider = Self::thread_provider(thread);
        let workspace = thread.project;
        // A connected provider already occupies the capacity this reservation
        // records; it cannot wait behind its own running process.
        let running = self.threads[index].pi.is_some();
        let id = self.scheduler.next_id();
        self.scheduler.reservations.push(Reservation {
            id: id.clone(),
            subject: if peer_worker {
                Subject::PeerWorker { thread_id, run }
            } else {
                Subject::Crew { thread_id, run }
            },
            state: if running {
                ReservationState::Held
            } else {
                ReservationState::Queued
            },
            coordinator,
            fair,
            executor: "local".into(),
            provider,
            workspace,
            captain,
            priority: 0,
            queued_at: lifecycle::now(),
            granted_at: running.then(lifecycle::now),
            released_at: None,
            release_reason: None,
            blocked_by: None,
            not_before: 0,
            uncertain: false,
        });
        self.threads[index].admission = Some(id);
    }

    /// Whether a crew member or peer worker still has something a live
    /// provider slot should cover: a running process, pending input, an
    /// unacknowledged result, or a checkout still being provisioned.
    fn admission_active(&self, index: usize) -> bool {
        let thread = &self.threads[index];
        thread.pi.is_some()
            || thread.status.busy()
            || thread.queued()
            || thread.task_result.is_some()
            || thread
                .peer_origin
                .as_ref()
                .is_some_and(|origin| origin.result.is_some() && !origin.acknowledged)
            || thread
                .placement
                .as_ref()
                .is_some_and(|placement| placement.phase == placement::PlacementPhase::Pending)
    }

    /// Idempotent release: the first call decides; later calls are no-ops.
    fn release_reservation_at(&mut self, index: usize, reason: &'static str) {
        let reservation = &mut self.scheduler.reservations[index];
        if reservation.state == ReservationState::Released {
            return;
        }
        let provider = reservation.provider.clone();
        reservation.state = ReservationState::Released;
        reservation.released_at = Some(lifecycle::now());
        reservation.release_reason = Some(reason.into());
        reservation.blocked_by = None;
        reservation.uncertain = false;
        if reason == "completed" || reason == "settled" {
            self.scheduler.backoff_strikes.remove(&provider);
            self.scheduler.provider_backoff.remove(&provider);
        }
        self.scheduler.prune_released();
    }

    /// A rate-limit signal backs off the provider scope with bounded
    /// exponential delay; the queue holds instead of spin-granting.
    pub(super) fn scheduler_note_failure(&mut self, provider: &str, error: &str) {
        if provider.is_empty() || !rate_limited(error) {
            return;
        }
        let strikes = self
            .scheduler
            .backoff_strikes
            .entry(provider.to_owned())
            .or_insert(0);
        *strikes = (*strikes).saturating_add(1).min(32);
        let shift = (*strikes - 1).min(4);
        let delay = BACKOFF_BASE_MS
            .saturating_mul(1u64 << shift)
            .min(BACKOFF_MAX_MS);
        self.scheduler
            .provider_backoff
            .insert(provider.to_owned(), lifecycle::now() + delay);
    }

    /// Whether `connect_thread` could spawn a provider at all. In demo, headless
    /// client, or disabled-runtime states it returns without spawning, which
    /// must not read as a refused start.
    fn spawn_possible(&self) -> bool {
        !self.demo && self.runtime_enabled && self.client.is_none() && !self.ownership_denied
    }

    /// Release reservations whose subject reached an explicit terminal
    /// transition, was superseded, or vanished. Uncertain subjects keep
    /// their slot until the outcome is proven.
    fn scheduler_reconcile(&mut self) -> bool {
        let mut releases: Vec<(usize, &'static str)> = Vec::new();
        let mut failures: Vec<(String, String)> = Vec::new();
        let mut uncertain: Vec<(usize, bool)> = Vec::new();
        let mut requeues: Vec<usize> = Vec::new();
        for index in 0..self.scheduler.reservations.len() {
            let reservation = &self.scheduler.reservations[index];
            if reservation.state == ReservationState::Released {
                continue;
            }
            match &reservation.subject {
                Subject::Task {
                    task_id,
                    attempt_id,
                } => {
                    let Some(task) = self.tasks.iter().find(|task| task.id == *task_id) else {
                        releases.push((index, "subject_gone"));
                        continue;
                    };
                    if task.attempt_id != *attempt_id {
                        releases.push((index, "superseded"));
                        continue;
                    }
                    if task.recovering {
                        uncertain.push((index, true));
                        continue;
                    }
                    uncertain.push((
                        index,
                        task.remote
                            .as_ref()
                            .is_some_and(|remote| remote.state == RemoteState::TargetOffline),
                    ));
                    if task.state.finished() {
                        let reason: &'static str = match task.state {
                            TaskState::Completed => "completed",
                            TaskState::Cancelled => "cancelled",
                            _ => "failed",
                        };
                        if reason == "failed" && !task.error_excerpt.is_empty() {
                            failures
                                .push((reservation.provider.clone(), task.error_excerpt.clone()));
                        }
                        releases.push((index, reason));
                    } else if task.state == TaskState::Interrupted {
                        releases.push((index, "interrupted"));
                    } else if reservation.state == ReservationState::Held
                        && task.remote.is_none()
                        && task.state == TaskState::Starting
                        && self.spawn_possible()
                    {
                        // A granted local attempt whose provider refused to
                        // start returns to the queue; a failed child releases.
                        match self
                            .threads
                            .iter()
                            .find(|thread| thread.id == task.child_thread_id)
                        {
                            Some(child)
                                if child.pi.is_none()
                                    && !child.recovering_task
                                    && child.status != Status::Failed
                                    && !child.archived() =>
                            {
                                requeues.push(index);
                            }
                            Some(child) if child.status == Status::Failed => {
                                if let Some(error) = child.error.clone() {
                                    failures.push((reservation.provider.clone(), error));
                                }
                                releases.push((index, "failed"));
                            }
                            None => releases.push((index, "subject_gone")),
                            _ => {}
                        }
                    }
                }
                Subject::Crew { thread_id, .. } | Subject::PeerWorker { thread_id, .. } => {
                    let peer_worker = matches!(reservation.subject, Subject::PeerWorker { .. });
                    let Some(position) = self
                        .threads
                        .iter()
                        .position(|thread| thread.id == *thread_id)
                    else {
                        releases.push((index, "thread_gone"));
                        continue;
                    };
                    let thread = &self.threads[position];
                    if thread.admission.as_deref() != Some(reservation.id.as_str())
                        || thread.archived()
                    {
                        releases.push((
                            index,
                            if thread.archived() {
                                "thread_gone"
                            } else {
                                "superseded"
                            },
                        ));
                        continue;
                    }
                    if !peer_worker && thread.crew_captain != reservation.captain {
                        releases.push((index, "left_crew"));
                        continue;
                    }
                    if thread.recovering_task {
                        uncertain.push((index, true));
                        continue;
                    }
                    uncertain.push((index, false));
                    if !self.admission_active(position) {
                        if thread.status == Status::Failed
                            && let Some(error) = thread.error.clone()
                        {
                            failures.push((reservation.provider.clone(), error));
                        }
                        releases.push((
                            index,
                            if thread.status == Status::Failed {
                                "failed"
                            } else {
                                "settled"
                            },
                        ));
                    } else if reservation.state == ReservationState::Held
                        && thread.pi.is_none()
                        && !thread.recovering_task
                        && self.spawn_possible()
                    {
                        // A granted worker whose provider refused to start
                        // returns to the queue unless it already failed.
                        if thread.status == Status::Failed {
                            if let Some(error) = thread.error.clone() {
                                failures.push((reservation.provider.clone(), error));
                            }
                            releases.push((index, "failed"));
                        } else {
                            requeues.push(index);
                        }
                    }
                }
            }
        }
        let mut changed = !releases.is_empty()
            || uncertain
                .iter()
                .any(|(index, value)| self.scheduler.reservations[*index].uncertain != *value);
        for (index, value) in uncertain {
            self.scheduler.reservations[index].uncertain = value;
        }
        for (provider, error) in failures {
            self.scheduler_note_failure(&provider, &error);
            changed = true;
        }
        for (index, reason) in releases {
            self.release_reservation_at(index, reason);
        }
        for index in requeues {
            let reservation = &mut self.scheduler.reservations[index];
            if reservation.state != ReservationState::Held {
                continue;
            }
            // A backoff before retry keeps a persistently refused start from
            // spinning; each strike doubles the hold within the cap.
            let provider = reservation.provider.clone();
            let strikes = self
                .scheduler
                .backoff_strikes
                .entry(provider.clone())
                .or_insert(0);
            *strikes = (*strikes).saturating_add(1).min(32);
            let shift = (*strikes - 1).min(4);
            reservation.state = ReservationState::Queued;
            reservation.blocked_by = Some("start_refused".into());
            reservation.not_before =
                lifecycle::now() + (BACKOFF_BASE_MS << shift).min(BACKOFF_MAX_MS);
            if let Subject::Task { task_id, .. } = &reservation.subject
                && let Some(task) = self.tasks.iter_mut().find(|task| task.id == *task_id)
                && task.state == TaskState::Starting
            {
                task.state = TaskState::Queued;
                task.started_at = None;
            }
            changed = true;
        }
        changed
    }

    /// Why this queued reservation cannot start yet, or None when it can.
    /// Scope order reports the first binding constraint.
    fn reservation_blocked(
        &self,
        reservation: &Reservation,
        counts: &HeldCounts,
        now: u64,
    ) -> Option<String> {
        if self.network_blocked() {
            return Some("offline".into());
        }
        if reservation.not_before > now {
            return Some("backoff".into());
        }
        match &reservation.subject {
            Subject::Task {
                task_id,
                attempt_id,
            } => {
                let Some(task) = self.tasks.iter().find(|task| task.id == *task_id) else {
                    return Some("subject_gone".into());
                };
                if task.attempt_id != *attempt_id || task.state.finished() {
                    return Some("superseded".into());
                }
                if task.recovering {
                    return Some("recovering".into());
                }
                if task.state != TaskState::Queued {
                    return Some("superseded".into());
                }
                let Some(child) = self
                    .threads
                    .iter()
                    .find(|thread| thread.id == task.child_thread_id)
                else {
                    return Some("subject_gone".into());
                };
                if child.archived() {
                    return Some("subject_gone".into());
                }
                if child.status.busy() || child.phase == "Stopping" {
                    return Some("worker_busy".into());
                }
                match child.placement.as_ref().map(|p| &p.phase) {
                    Some(placement::PlacementPhase::Pending) => {
                        return Some("placement".into());
                    }
                    Some(placement::PlacementPhase::Failed) => {
                        return Some("placement_failed".into());
                    }
                    _ => {}
                }
                if child.machine == "local" && self.provider_update_drivers.contains(&child.harness)
                {
                    return Some("provider_update".into());
                }
                if self
                    .require_active_thread_owner(task.child_thread_id)
                    .is_err()
                {
                    return Some("owner_inactive".into());
                }
            }
            Subject::Crew { thread_id, .. } | Subject::PeerWorker { thread_id, .. } => {
                let Some(thread) = self.threads.iter().find(|thread| thread.id == *thread_id)
                else {
                    return Some("subject_gone".into());
                };
                if thread.archived() {
                    return Some("subject_gone".into());
                }
                if thread.recovering_task {
                    return Some("recovering".into());
                }
                match thread.placement.as_ref().map(|p| &p.phase) {
                    Some(placement::PlacementPhase::Pending) => {
                        return Some("placement".into());
                    }
                    Some(placement::PlacementPhase::Failed) => {
                        return Some("placement_failed".into());
                    }
                    _ => {}
                }
                if thread.machine == "local"
                    && self.provider_update_drivers.contains(&thread.harness)
                {
                    return Some("provider_update".into());
                }
                if self.require_active_thread_owner(thread.id).is_err() {
                    return Some("owner_inactive".into());
                }
            }
        }
        if let Some(until) = self.scheduler.provider_backoff.get(&reservation.provider)
            && *until > now
        {
            return Some("provider_backoff".into());
        }
        let limits = &self.scheduler.limits;
        if let (Some(home), Some(limit)) = (reservation.coordinator, limits.coordinator_running)
            && counts.coordinator.get(&home).copied().unwrap_or(0) >= limit
        {
            return Some("coordinator".into());
        }
        if let (Some(workspace), Some(limit)) = (reservation.workspace, limits.workspace_running)
            && counts.workspace.get(&workspace).copied().unwrap_or(0) >= limit
        {
            return Some("workspace".into());
        }
        if let Some(limit) = limits.provider_running
            && counts
                .provider
                .get(&reservation.provider)
                .copied()
                .unwrap_or(0)
                >= limit
        {
            return Some("provider".into());
        }
        if let Some(limit) = limits.executor_running
            && counts
                .executor
                .get(&reservation.executor)
                .copied()
                .unwrap_or(0)
                >= limit
        {
            return Some("executor".into());
        }
        if let Some(limit) = limits.account_running
            && counts.account >= limit
        {
            return Some("account".into());
        }
        None
    }

    /// Grant one reservation and start its subject. Returns false without
    /// state change when the subject can no longer start (reconciliation or
    /// the blocker check will settle it on the next pass).
    fn admission_grant(&mut self, id: &str) -> bool {
        let Some(reservation_index) = self
            .scheduler
            .reservations
            .iter()
            .position(|r| r.id == id && r.state == ReservationState::Queued)
        else {
            return false;
        };
        let subject = self.scheduler.reservations[reservation_index]
            .subject
            .clone();
        match &subject {
            Subject::Task {
                task_id,
                attempt_id,
            } => {
                let Some(task_index) = self.tasks.iter().position(|task| task.id == *task_id)
                else {
                    return false;
                };
                if self.tasks[task_index].attempt_id != *attempt_id
                    || self.tasks[task_index].state != TaskState::Queued
                    || self.tasks[task_index].recovering
                {
                    return false;
                }
                let Some(child) = self
                    .threads
                    .iter()
                    .position(|thread| thread.id == self.tasks[task_index].child_thread_id)
                else {
                    return false;
                };
                self.scheduler.reservations[reservation_index].state = ReservationState::Held;
                self.scheduler.reservations[reservation_index].granted_at = Some(lifecycle::now());
                self.scheduler.reservations[reservation_index].blocked_by = None;
                self.tasks[task_index].state = TaskState::Starting;
                self.tasks[task_index].started_at = Some(lifecycle::now());
                if self.tasks[task_index].remote.is_some() {
                    // Committed state only: dispatch_peer_work sends the
                    // launch command on the next tick.
                    return true;
                }
                // Persist intent before any prompt can reach the provider.
                if self.runtime_enabled
                    && let Err(error) = self.checkpoint_and_flush()
                {
                    self.tasks[task_index].state = TaskState::Interrupted;
                    self.tasks[task_index].error_excerpt = error.chars().take(2000).collect();
                    self.storage_error = Some(error);
                    return true;
                }
                let thread = &mut self.threads[child];
                thread.arm_task();
                let text = self.tasks[task_index].attempt_message.clone();
                thread.pending_injections.push_back(Injection {
                    text,
                    shown: false,
                    steer: None,
                });
                self.connect_thread(child);
                true
            }
            Subject::Crew { thread_id, .. } | Subject::PeerWorker { thread_id, .. } => {
                let Some(index) = self
                    .threads
                    .iter()
                    .position(|thread| thread.id == *thread_id)
                else {
                    return false;
                };
                self.scheduler.reservations[reservation_index].state = ReservationState::Held;
                self.scheduler.reservations[reservation_index].granted_at = Some(lifecycle::now());
                self.scheduler.reservations[reservation_index].blocked_by = None;
                self.connect_thread(index);
                true
            }
        }
    }

    /// Reconcile every live reservation, refresh queue reasons, then grant
    /// in fairness order until no candidate can start. Replaces the old
    /// per-home running-count loop in `start_queued_tasks`.
    pub(super) fn admission_pump(&mut self) -> bool {
        // Lazily admit subjects that predate the scheduler or lost their
        // reservation to a crash: queued delegated attempts and crew or peer
        // threads still holding live work.
        let mut changed = false;
        for index in 0..self.tasks.len() {
            if self.tasks[index].state != TaskState::Queued {
                continue;
            }
            let covered = self.scheduler.reservations.iter().any(|r| {
                r.state != ReservationState::Released
                    && matches!(&r.subject, Subject::Task { task_id, .. }
                        if *task_id == self.tasks[index].id)
            });
            if !covered {
                self.admit_task_index(index);
                changed = true;
            }
        }
        for index in 0..self.threads.len() {
            let managed = self.threads[index].peer_origin.is_some()
                || self.threads[index].crew_captain.is_some();
            if managed && self.threads[index].admission.is_none() && self.admission_active(index) {
                let peer_worker = self.threads[index].peer_origin.is_some();
                self.ensure_thread_admission(index, peer_worker, false);
                changed = true;
            }
        }
        changed |= self.scheduler_reconcile();
        loop {
            let now = lifecycle::now();
            let counts = self.scheduler.held_counts();
            let order = self.scheduler.grant_order(&counts);
            if order.is_empty() {
                break;
            }
            let mut grant = None;
            for index in order {
                let blocked = {
                    let reservation = &self.scheduler.reservations[index];
                    self.reservation_blocked(reservation, &counts, now)
                };
                if self.scheduler.reservations[index].blocked_by != blocked {
                    self.scheduler.reservations[index].blocked_by = blocked.clone();
                    changed = true;
                }
                if blocked.is_none() && grant.is_none() {
                    grant = Some(self.scheduler.reservations[index].id.clone());
                }
            }
            let Some(id) = grant else { break };
            if !self.admission_grant(&id) {
                // The blocker check races nothing in this single-threaded
                // reducer; a grant refusal means the subject changed between
                // evaluation and grant. Mark it blocked so the loop cannot
                // spin on the same candidate.
                if let Some(index) = self.scheduler.reservations.iter().position(|r| r.id == id) {
                    self.scheduler.reservations[index].blocked_by = Some("superseded".into());
                }
                break;
            }
            changed = true;
        }
        changed
    }

    /// The scheduler's own queue position for a thread-bound reservation.
    pub(super) fn admission_value_for(&self, thread_id: usize) -> Value {
        let Some(reservation) = self.live_reservation_for(thread_id) else {
            return Value::Null;
        };
        let position = (reservation.state == ReservationState::Queued)
            .then(|| self.scheduler.queue_position(&reservation.id))
            .flatten();
        json!({
            "reservationId": reservation.id,
            "state": reservation.state.id(),
            "position": position,
            "blockedBy": reservation.blocked_by,
            "queuedAt": reservation.queued_at,
            "grantedAt": reservation.granted_at,
            "uncertain": reservation.uncertain,
            "scope": {
                "coordinator": reservation.coordinator,
                "executor": reservation.executor,
                "provider": reservation.provider,
                "workspace": reservation.workspace,
            },
        })
    }

    /// The reservation a task attempt holds, for task_status.
    pub(super) fn admission_value_for_task(&self, task: &Task) -> Value {
        let Some(reservation) = self.scheduler.reservations.iter().find(|r| {
            r.state != ReservationState::Released
                && matches!(
                    &r.subject,
                    Subject::Task { task_id, attempt_id }
                        if *task_id == task.id && *attempt_id == task.attempt_id
                )
        }) else {
            return Value::Null;
        };
        let position = (reservation.state == ReservationState::Queued)
            .then(|| self.scheduler.queue_position(&reservation.id))
            .flatten();
        json!({
            "reservationId": reservation.id,
            "state": reservation.state.id(),
            "position": position,
            "blockedBy": reservation.blocked_by,
            "queuedAt": reservation.queued_at,
            "grantedAt": reservation.granted_at,
            "uncertain": reservation.uncertain,
            "scope": {
                "coordinator": reservation.coordinator,
                "executor": reservation.executor,
                "provider": reservation.provider,
                "workspace": reservation.workspace,
            },
        })
    }

    /// The authoritative scheduling view the `scheduling.status` RPC serves:
    /// effective limits, per-scope usage, the ordered queue with reasons,
    /// held reservations with uncertainty, and provider backoff.
    pub fn scheduling_status_value(&self) -> Value {
        let counts = self.scheduler.held_counts();
        let order = self.scheduler.grant_order(&counts);
        let now = lifecycle::now();
        let queued: Vec<Value> = order
            .iter()
            .enumerate()
            .map(|(position, index)| self.scheduler.reservations[*index].value(Some(position + 1)))
            .collect();
        let held: Vec<Value> = self
            .scheduler
            .reservations
            .iter()
            .filter(|r| r.state == ReservationState::Held)
            .map(|r| r.value(None))
            .collect();
        let coordinators: Vec<Value> = counts
            .coordinator
            .iter()
            .map(|(home, running)| {
                json!({
                    "scope": "coordinator",
                    "id": home,
                    "running": running,
                    "queued": self.scheduler.reservations.iter().filter(|r| {
                        r.state == ReservationState::Queued && r.coordinator == Some(*home)
                    }).count(),
                    "limit": self.scheduler.limits.coordinator_running,
                })
            })
            .collect();
        let executors: Vec<Value> = counts
            .executor
            .iter()
            .map(|(executor, running)| {
                json!({
                    "scope": "executor",
                    "id": executor,
                    "running": running,
                    "limit": self.scheduler.limits.executor_running,
                })
            })
            .collect();
        let mut providers: Vec<Value> = counts
            .provider
            .iter()
            .map(|(provider, running)| {
                json!({
                    "scope": "provider",
                    "id": provider,
                    "running": running,
                    "limit": self.scheduler.limits.provider_running,
                    "backoffUntil": self.scheduler.provider_backoff.get(provider).filter(|until| **until > now),
                })
            })
            .collect();
        for (provider, until) in &self.scheduler.provider_backoff {
            if *until > now && !counts.provider.contains_key(provider) {
                providers.push(json!({
                    "scope": "provider",
                    "id": provider,
                    "running": 0,
                    "limit": self.scheduler.limits.provider_running,
                    "backoffUntil": until,
                }));
            }
        }
        let workspaces: Vec<Value> = counts
            .workspace
            .iter()
            .map(|(workspace, running)| {
                json!({
                    "scope": "workspace",
                    "id": workspace,
                    "running": running,
                    "limit": self.scheduler.limits.workspace_running,
                })
            })
            .collect();
        json!({
            "limits": self.scheduler.limits.value(),
            "usage": {
                "account": {
                    "running": counts.account,
                    "queued": queued.len(),
                    "limit": self.scheduler.limits.account_running,
                    "pendingLimit": self.scheduler.limits.account_pending,
                },
                "coordinators": coordinators,
                "executors": executors,
                "providers": providers,
                "workspaces": workspaces,
            },
            "queue": queued,
            "held": held,
            "providerBackoff": self.scheduler.provider_backoff
                .iter()
                .filter(|(_, until)| **until > now)
                .map(|(provider, until)| json!({"provider":provider,"until":until}))
                .collect::<Vec<_>>(),
        })
    }

    /// The caller-scoped slice orchestrator_capabilities reports: effective
    /// limits, this coordinator's usage and its queue entries. A caller with
    /// no coordinator sees the account view only.
    pub(super) fn scheduling_scoped_value(&self, coordinator: Option<usize>) -> Value {
        let counts = self.scheduler.held_counts();
        let order = self.scheduler.grant_order(&counts);
        let entries: Vec<Value> = order
            .iter()
            .enumerate()
            .filter_map(|(position, index)| {
                let reservation = &self.scheduler.reservations[*index];
                (coordinator.is_none() || reservation.coordinator == coordinator)
                    .then(|| reservation.value(Some(position + 1)))
            })
            .collect();
        json!({
            "limits": self.scheduler.limits.value(),
            "coordinator": coordinator.map(|home| json!({
                "id": home,
                "running": counts.coordinator.get(&home).copied().unwrap_or(0),
                "limit": self.scheduler.limits.coordinator_running,
            })),
            "account": {
                "running": counts.account,
                "limit": self.scheduler.limits.account_running,
                "pending": self.scheduler.reservations.iter()
                    .filter(|r| r.state == ReservationState::Queued).count(),
                "pendingLimit": self.scheduler.limits.account_pending,
            },
            "queue": entries,
        })
    }

    /// `scheduling.configure`: merges the given limits and returns the new
    /// authoritative status. `null` means unlimited; every positive number
    /// is a cap. Unknown keys fail rather than being silently ignored.
    pub(super) fn scheduling_configure(&mut self, args: &Value) -> Result<Value, String> {
        let limits = args
            .get("limits")
            .and_then(|value| value.as_object())
            .ok_or("limits must be an object of scope limits.")?;
        let fields: &[(&str, fn(&mut Limits) -> &mut Option<usize>)] = &[
            ("coordinatorRunning", |limits| {
                &mut limits.coordinator_running
            }),
            ("coordinatorUnfinished", |limits| {
                &mut limits.coordinator_unfinished
            }),
            ("accountRunning", |limits| &mut limits.account_running),
            ("accountPending", |limits| &mut limits.account_pending),
            ("executorRunning", |limits| &mut limits.executor_running),
            ("providerRunning", |limits| &mut limits.provider_running),
            ("workspaceRunning", |limits| &mut limits.workspace_running),
        ];
        for (name, value) in limits {
            let Some(field) = fields.iter().find(|(field, _)| field == name) else {
                return Err(format!("Unknown scheduler limit {name}."));
            };
            let parsed = if value.is_null() {
                None
            } else {
                let limit = value
                    .as_u64()
                    .filter(|limit| *limit >= 1 && *limit <= usize::MAX as u64)
                    .ok_or(format!("{name} must be null or a positive integer."))?;
                Some(usize::try_from(limit).map_err(|_| format!("{name} is too large."))?)
            };
            *field.1(&mut self.scheduler.limits) = parsed;
        }
        self.admission_pump();
        Ok(self.scheduling_status_value())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn demo() -> State {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        state
    }

    /// A managed worker thread with live input so `admission_active` holds.
    fn worker(state: &mut State, text: &str) -> usize {
        let id = state.next_id;
        state.next_id += 1;
        let mut thread = Thread::new(id);
        thread.model_provider = "cpa".into();
        thread.pending_injections.push_back(Injection {
            text: text.into(),
            shown: false,
            steer: None,
        });
        state.threads.push(thread);
        let index = state.threads.len() - 1;
        state.ensure_thread_admission(index, false, false);
        index
    }

    fn reservation_for(state: &State, thread_id: usize) -> &Reservation {
        let id = state
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .and_then(|thread| thread.admission.as_deref())
            .expect("live admission id");
        state
            .scheduler
            .reservations
            .iter()
            .find(|reservation| reservation.id == id)
            .expect("reservation")
    }

    #[test]
    fn independent_roots_share_the_account_limit() {
        let mut state = demo();
        state.scheduler.limits.account_running = Some(1);
        let first = worker(&mut state, "first");
        let second = worker(&mut state, "second");
        state.admission_pump();
        assert_eq!(
            reservation_for(&state, state.threads[first].id).state,
            ReservationState::Held
        );
        let queued = reservation_for(&state, state.threads[second].id);
        assert_eq!(queued.state, ReservationState::Queued);
        assert_eq!(queued.blocked_by.as_deref(), Some("account"));
    }

    #[test]
    fn a_released_slot_admits_the_next_in_line() {
        let mut state = demo();
        state.scheduler.limits.account_running = Some(1);
        let first = worker(&mut state, "first");
        let second = worker(&mut state, "second");
        state.admission_pump();
        // The first worker settles: nothing left for a provider to do.
        state.threads[first].pending_injections.clear();
        state.admission_pump();
        let released = reservation_for(&state, state.threads[first].id);
        assert_eq!(released.state, ReservationState::Released);
        assert_eq!(released.release_reason.as_deref(), Some("settled"));
        assert_eq!(
            reservation_for(&state, state.threads[second].id).state,
            ReservationState::Held
        );
    }

    #[test]
    fn release_is_idempotent() {
        let mut state = demo();
        let index = worker(&mut state, "only");
        state.admission_pump();
        let id = state.threads[index].admission.clone().unwrap();
        let position = state
            .scheduler
            .reservations
            .iter()
            .position(|reservation| reservation.id == id)
            .unwrap();
        state.release_reservation_at(position, "cancelled");
        let released_at = state.scheduler.reservations[position].released_at;
        state.release_reservation_at(position, "failed");
        assert_eq!(
            state.scheduler.reservations[position].released_at,
            released_at
        );
        assert_eq!(
            state.scheduler.reservations[position]
                .release_reason
                .as_deref(),
            Some("cancelled")
        );
    }

    #[test]
    fn queued_reservations_survive_a_restart() {
        let mut state = demo();
        state.scheduler.limits.account_running = Some(1);
        let first = worker(&mut state, "first");
        let second = worker(&mut state, "second");
        // A pending placement is durable work the queued reservation keeps.
        state.threads[second].placement = Some(placement::ThreadPlacement {
            requested: placement::PlacementRequest::Default,
            mode: placement::PlacementMode::Isolated,
            phase: placement::PlacementPhase::Pending,
            executor: "local".into(),
            repository: None,
            base_branch: None,
            base_commit: None,
            upstream_thread_id: None,
            branch: None,
            checkout: None,
            start_from_origin: false,
            primary_checkout: false,
            new_worktree: true,
            capability: None,
            error: None,
        });
        state.admission_pump();
        let snapshot = state.snapshot();
        let mut restored = State::new(PathBuf::new(), false);
        restored.demo = true;
        restored.restore_global(&snapshot).unwrap();
        assert_eq!(restored.scheduler.reservations.len(), 2);
        let first_id = state.threads[first].id;
        let second_id = state.threads[second].id;
        assert_eq!(
            reservation_for(&restored, first_id).state,
            ReservationState::Held
        );
        assert_eq!(
            reservation_for(&restored, second_id).state,
            ReservationState::Queued
        );
        // Reconcile settles the held worker whose transient input did not
        // survive the restart. The queued worker's durable placement keeps
        // its reservation live behind the placement fence.
        restored.admission_pump();
        assert_eq!(
            reservation_for(&restored, first_id).state,
            ReservationState::Released
        );
        let queued = reservation_for(&restored, second_id);
        assert_eq!(queued.state, ReservationState::Queued);
        assert_eq!(queued.blocked_by.as_deref(), Some("placement"));
    }

    #[test]
    fn a_refused_start_requeues_with_backoff_then_retries() {
        let mut state = demo();
        state
            .mcp_call(1, "delegate_task", &json!({"prompt":"A bounded task"}))
            .unwrap();
        assert_eq!(state.tasks[0].state, TaskState::Starting);
        // Demo never spawns; a real runtime would have refused this start.
        state.demo = false;
        state.runtime_enabled = true;
        state.admission_pump();
        assert_eq!(state.tasks[0].state, TaskState::Queued);
        let reservation_id = state
            .scheduler
            .reservations
            .iter()
            .find(|reservation| matches!(&reservation.subject, Subject::Task { .. }))
            .unwrap()
            .id
            .clone();
        let reservation = &state.scheduler.reservations[0];
        assert_eq!(reservation.state, ReservationState::Queued);
        assert_eq!(reservation.blocked_by.as_deref(), Some("backoff"));
        assert!(reservation.not_before > lifecycle::now());
        assert_eq!(state.scheduler.backoff_strikes["cpa"], 1);
        // Once the hold elapses the same attempt is granted again.
        let position = state
            .scheduler
            .reservations
            .iter()
            .position(|reservation| reservation.id == reservation_id)
            .unwrap();
        state.scheduler.reservations[position].not_before = 0;
        state.demo = true;
        state.runtime_enabled = false;
        state.admission_pump();
        assert_eq!(state.tasks[0].state, TaskState::Starting);
    }

    #[test]
    fn an_offline_remote_attempt_stays_held_and_uncertain() {
        let mut state = demo();
        state
            .mcp_call(1, "delegate_task", &json!({"prompt":"A bounded task"}))
            .unwrap();
        state.tasks[0].state = TaskState::Running;
        state.tasks[0].remote = Some(RemoteTask {
            server_id: "peer-1".into(),
            project: json!({}),
            label: "Peer".into(),
            thread_id: Some(41),
            launch_command: crate::protocol::Command::new("peer.thread.launch", json!({})),
            stop_owed: false,
            stop_command_id: None,
            stop_attempt_id: None,
            last_error: None,
            next_retry_at: 0,
            retry_attempt: 0,
            state: RemoteState::TargetOffline,
            needs_you: false,
            request: None,
            wait_revision: None,
            parent_stop_serial: 0,
            child_stop_serial: 0,
            client_request_id: None,
            send_command: None,
            document_replies: VecDeque::new(),
            placement: None,
        });
        state.admission_pump();
        let reservation = &state.scheduler.reservations[0];
        assert_eq!(reservation.state, ReservationState::Held);
        assert!(reservation.uncertain);
        // No duplicate attempt may start while the outcome is unproven.
        assert_eq!(state.tasks[0].state, TaskState::Running);
        // A proven terminal result releases the slot exactly once.
        state.tasks[0].remote.as_mut().unwrap().state = RemoteState::Done;
        state.tasks[0].finish(TaskState::Completed, "done", "");
        state.admission_pump();
        assert_eq!(
            state.scheduler.reservations[0].state,
            ReservationState::Released
        );
        assert_eq!(
            state.scheduler.reservations[0].release_reason.as_deref(),
            Some("completed")
        );
    }

    #[test]
    fn a_provider_rate_limit_backs_off_its_scope() {
        let mut state = demo();
        state.scheduler_note_failure("cpa", "429 rate limit exceeded");
        let index = worker(&mut state, "queued behind backoff");
        state.admission_pump();
        let reservation = reservation_for(&state, state.threads[index].id);
        assert_eq!(reservation.state, ReservationState::Queued);
        assert_eq!(reservation.blocked_by.as_deref(), Some("provider_backoff"));
    }

    #[test]
    fn terminal_transitions_release_and_completion_clears_backoff() {
        let mut state = demo();
        state.scheduler.backoff_strikes.insert("cpa".into(), 3);
        state
            .mcp_call(1, "delegate_task", &json!({"prompt":"A bounded task"}))
            .unwrap();
        state
            .mcp_call(1, "delegate_task", &json!({"prompt":"Another task"}))
            .unwrap();
        state.tasks[0].finish(TaskState::Completed, "done", "");
        state.tasks[1].finish(TaskState::Cancelled, "", "cancelled");
        state.admission_pump();
        let reasons: Vec<Option<String>> = state
            .scheduler
            .reservations
            .iter()
            .map(|reservation| reservation.release_reason.clone())
            .collect();
        assert_eq!(
            reasons,
            vec![Some("completed".into()), Some("cancelled".into())]
        );
        assert!(state.scheduler.backoff_strikes.is_empty());
    }

    #[test]
    fn a_pending_placement_blocks_the_grant_until_ready() {
        let mut state = demo();
        let index = worker(&mut state, "waiting on placement");
        state.threads[index].placement = Some(placement::ThreadPlacement {
            requested: placement::PlacementRequest::Default,
            mode: placement::PlacementMode::Isolated,
            phase: placement::PlacementPhase::Pending,
            executor: "local".into(),
            repository: None,
            base_branch: None,
            base_commit: None,
            upstream_thread_id: None,
            branch: None,
            checkout: None,
            start_from_origin: false,
            primary_checkout: false,
            new_worktree: true,
            capability: None,
            error: None,
        });
        state.admission_pump();
        let id = state.threads[index].id;
        assert_eq!(
            reservation_for(&state, id).blocked_by.as_deref(),
            Some("placement")
        );
        state
            .threads
            .iter_mut()
            .find(|thread| thread.id == id)
            .unwrap()
            .placement
            .as_mut()
            .unwrap()
            .phase = placement::PlacementPhase::Ready;
        state.admission_pump();
        assert_eq!(reservation_for(&state, id).state, ReservationState::Held);
    }

    #[test]
    fn the_pending_cap_refuses_new_admissions() {
        let mut state = demo();
        state.scheduler.limits.account_pending = Some(1);
        let _held = worker(&mut state, "occupies the queue");
        state.scheduler.reservations[0].state = ReservationState::Queued;
        let error = state
            .mcp_call(1, "delegate_task", &json!({"prompt":"One too many"}))
            .unwrap_err();
        assert!(error.contains("admission queue is full"), "{error}");
    }

    #[test]
    fn configure_merges_limits_and_rejects_unknown_keys() {
        let mut state = demo();
        let status = state
            .scheduling_configure(&json!({"limits":{"accountRunning":2,"providerRunning":null}}))
            .unwrap();
        assert_eq!(status["limits"]["accountRunning"], 2);
        assert_eq!(status["limits"]["providerRunning"], Value::Null);
        assert!(
            state
                .scheduling_configure(&json!({"limits":{"bogus":1}}))
                .is_err()
        );
        assert!(
            state
                .scheduling_configure(&json!({"limits":{"accountRunning":0}}))
                .is_err()
        );
    }
}
