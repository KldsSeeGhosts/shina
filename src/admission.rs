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

/// Per-id caps layered over a scope's class limit. An id absent from its map
/// uses the class limit; ids are the same keys the scope counts under
/// ("captain:<id>" or "home:<thread>" for coordinator, decimal ids for
/// captain and workspace, server ids for executor, provider ids).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScopeOverrides {
    pub coordinator: BTreeMap<String, usize>,
    pub captain: BTreeMap<String, usize>,
    pub executor: BTreeMap<String, usize>,
    pub provider: BTreeMap<String, usize>,
    pub workspace: BTreeMap<String, usize>,
}

impl ScopeOverrides {
    const MAX_ENTRIES: usize = 256;

    fn maps(&self) -> [&BTreeMap<String, usize>; 5] {
        [
            &self.coordinator,
            &self.captain,
            &self.executor,
            &self.provider,
            &self.workspace,
        ]
    }

    fn valid(&self) -> bool {
        self.maps().iter().all(|map| {
            map.len() <= Self::MAX_ENTRIES
                && map
                    .iter()
                    .all(|(id, cap)| !id.is_empty() && id.len() <= 64 && *cap >= 1)
        })
    }

    fn value(&self) -> Value {
        json!({
            "coordinator": self.coordinator,
            "captain": self.captain,
            "executor": self.executor,
            "provider": self.provider,
            "workspace": self.workspace,
        })
    }
}

/// Effective limits. `None` means unlimited; `Some(n)` requires n >= 1, so
/// `Some(0)` cannot accidentally mean "pause everything" or "unlimited".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Limits {
    /// Providers running under one coordinator: a Captain (stable across
    /// home resets) or a parent thread.
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
    /// Providers per Captain id, counting that Captain's crew roots and
    /// delegated descendants together. `None` by default: the coordinator
    /// class already covers the single-user envelope.
    #[serde(default)]
    pub captain_running: Option<usize>,
    /// Providers per local project. Remote work does not map onto local
    /// project ids, so it does not count here.
    #[serde(default)]
    pub workspace_running: Option<usize>,
    /// Per-id caps that take precedence over the class limit for that scope.
    #[serde(default)]
    pub overrides: ScopeOverrides,
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
            captain_running: None,
            workspace_running: None,
            overrides: ScopeOverrides::default(),
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
            self.captain_running,
            self.workspace_running,
        ]
        .iter()
        .all(|limit| limit.is_none_or(|limit| limit >= 1))
            && self.overrides.valid()
    }

    /// The cap one scope id runs under: its override when configured, else
    /// the class limit.
    fn effective(
        &self,
        class: Option<usize>,
        overrides: &BTreeMap<String, usize>,
        id: &str,
    ) -> Option<usize> {
        overrides.get(id).copied().or(class)
    }

    pub fn value(&self) -> Value {
        json!({
            "coordinatorRunning": self.coordinator_running,
            "coordinatorUnfinished": self.coordinator_unfinished,
            "accountRunning": self.account_running,
            "accountPending": self.account_pending,
            "executorRunning": self.executor_running,
            "providerRunning": self.provider_running,
            "captainRunning": self.captain_running,
            "workspaceRunning": self.workspace_running,
            "overrides": self.overrides.value(),
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
    /// worker_busy, backoff, provider_backoff, or a scope name.
    #[serde(default)]
    pub blocked_by: Option<String>,
    #[serde(default)]
    pub not_before: u64,
    /// Held while its outcome is unproven (restart or target unreachable).
    #[serde(default)]
    pub uncertain: bool,
}

impl Reservation {
    /// The stable coordinator scope id: a Captain outlives every home it
    /// resets, so captain-owned work counts under "captain:<id>"; other
    /// local work counts under its home thread.
    pub fn coordinator_key(&self) -> String {
        self.captain
            .map(|captain| format!("captain:{captain}"))
            .or_else(|| self.coordinator.map(|home| format!("home:{home}")))
            .unwrap_or_default()
    }

    fn value(&self, position: Option<usize>) -> Value {
        json!({
            "reservationId": self.id,
            "subject": self.subject.value(),
            "state": self.state.id(),
            "position": position,
            "coordinator": self.coordinator,
            "coordinatorKey": self.coordinator_key(),
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
            "retryAt": (self.not_before > 0).then_some(self.not_before),
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
    /// Durable change counter. Every committed scheduler mutation bumps it,
    /// so `scheduling.configure` can compare-and-set and subscribers can
    /// read one consistent projection instead of polling.
    #[serde(default)]
    pub revision: u64,
}

/// Held reservations tallied per scope.
#[derive(Default)]
struct HeldCounts {
    account: usize,
    coordinator: HashMap<String, usize>,
    captain: HashMap<usize, usize>,
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
            let coordinator = reservation.coordinator_key();
            if !coordinator.is_empty() {
                *counts.coordinator.entry(coordinator).or_insert(0) += 1;
            }
            if let Some(captain) = reservation.captain {
                *counts.captain.entry(captain).or_insert(0) += 1;
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
        if !self.limits.valid() {
            return Err("The saved scheduler has invalid scope overrides.".into());
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

    /// Whether a held reservation's subject is still in the startup window
    /// between grant and a confirmed provider turn.
    fn reservation_starting(&self, reservation: &Reservation) -> bool {
        if reservation.state != ReservationState::Held {
            return false;
        }
        match &reservation.subject {
            Subject::Task { task_id, .. } => self
                .tasks
                .iter()
                .any(|task| task.id == *task_id && task.state == TaskState::Starting),
            Subject::Crew { thread_id, .. } | Subject::PeerWorker { thread_id, .. } => self
                .threads
                .iter()
                .any(|thread| thread.id == *thread_id && thread.pi.is_none()),
        }
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
    /// a pre-start correction keeps queue position. A stale held reservation
    /// is superseded only when its occupancy is proven gone — an unacked
    /// remote stop keeps it held and uncertain — and the current attempt
    /// always ends the call with its own live reservation: the durable
    /// fence has no gap.
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
        // Collect ids first: releasing prunes, so positions are unstable.
        let stale: Vec<String> = self
            .scheduler
            .reservations
            .iter()
            .filter(|r| {
                r.state != ReservationState::Released
                    && matches!(&r.subject, Subject::Task { task_id: id, .. } if *id == task_id)
            })
            .map(|r| r.id.clone())
            .collect();
        let mut adopted = false;
        for id in stale {
            let Some(position) = self.scheduler.reservations.iter().position(|r| r.id == id) else {
                continue;
            };
            let reservation = self.scheduler.reservations[position].clone();
            let attempt = match &reservation.subject {
                Subject::Task { attempt_id, .. } => attempt_id.clone(),
                _ => continue,
            };
            if attempt == self.tasks[task_index].attempt_id {
                adopted = true;
                continue;
            }
            match reservation.state {
                ReservationState::Queued if !adopted => {
                    // Adopt the queue slot for the corrected attempt.
                    let reservation = &mut self.scheduler.reservations[position];
                    reservation.subject = subject.clone();
                    reservation.coordinator = coordinator;
                    reservation.fair = fair.clone();
                    reservation.executor = executor.clone();
                    reservation.provider = provider.clone();
                    reservation.workspace = workspace;
                    reservation.captain = captain;
                    reservation.blocked_by = None;
                    reservation.uncertain = false;
                    self.scheduler.revision += 1;
                    adopted = true;
                }
                ReservationState::Held
                    if Self::remote_occupancy_unproven(&self.tasks[task_index], &attempt) =>
                {
                    // The older attempt's remote process may still run; its
                    // occupancy stays until the owed stop is acknowledged.
                    if !reservation.uncertain {
                        self.scheduler.reservations[position].uncertain = true;
                        self.scheduler.revision += 1;
                    }
                }
                _ => self.release_reservation_at(position, "superseded"),
            }
        }
        // The attempt reservation governs the child thread; a thread-scoped
        // reservation left from an earlier crew life or a backfill race must
        // not count the same provider occupancy a second time.
        let child = self.tasks[task_index].child_thread_id;
        let stray: Vec<String> = self
            .scheduler
            .reservations
            .iter()
            .filter(|r| {
                r.state != ReservationState::Released
                    && r.subject
                        .thread_id()
                        .is_some_and(|thread_id| thread_id == child)
            })
            .map(|r| r.id.clone())
            .collect();
        for id in stray {
            if let Some(position) = self.scheduler.reservations.iter().position(|r| r.id == id) {
                self.release_reservation_at(position, "superseded");
            }
        }
        if adopted {
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
        // A backfilled attempt that already runs occupies its slot
        // truthfully; only a queued attempt waits in line.
        let occupying = matches!(
            self.tasks[task_index].state,
            TaskState::Starting | TaskState::Running
        );
        self.scheduler.reservations.push(Reservation {
            id,
            subject,
            state: if occupying {
                ReservationState::Held
            } else {
                ReservationState::Queued
            },
            coordinator,
            fair,
            executor,
            provider,
            workspace,
            captain,
            priority: 0,
            queued_at: lifecycle::now(),
            granted_at: occupying.then(lifecycle::now),
            released_at: None,
            release_reason: None,
            blocked_by: None,
            not_before: 0,
            uncertain: false,
        });
        self.scheduler.revision += 1;
    }

    /// A stop receipt, disconnection, revocation or protocol error cannot
    /// prove provider settlement. Keep occupancy until a terminal result
    /// reaches the home, and also while this attempt still owes its stop.
    fn remote_occupancy_unproven(task: &Task, attempt: &str) -> bool {
        task.remote.as_ref().is_some_and(|remote| {
            remote.stop_owed
                && remote
                    .stop_attempt_id
                    .as_deref()
                    .is_none_or(|named| named == attempt)
                || !matches!(
                    remote.state,
                    RemoteState::Done | RemoteState::ResultPendingDelivery
                )
        })
    }

    /// A crew member or peer worker occupies admission while work or a live
    /// provider turn exists. `new_attempt` supersedes the live reservation's
    /// run identity in place, so the ledger never loses occupancy between
    /// runs and a re-ensure cannot free a slot a still-running process holds.
    /// A fresh reservation for a thread whose provider sits connected but
    /// idle queues like any other work: an open CLI connection is not a run.
    ///
    /// A delegated child's occupancy lives on its task's attempt
    /// reservation; it never holds a crew or peer-worker subject of its own.
    fn thread_has_task(&self, thread_id: usize) -> bool {
        self.tasks
            .iter()
            .any(|task| task.child_thread_id == thread_id && !task.state.finished())
    }

    pub(super) fn ensure_thread_admission(
        &mut self,
        index: usize,
        peer_worker: bool,
        new_attempt: bool,
    ) {
        let thread_id = self.threads[index].id;
        if self.thread_has_task(thread_id) {
            // The task reservation governs this thread; a second subject
            // would count the same provider occupancy twice.
            self.sync_admission_flag(index);
            return;
        }
        if let Some(current) = self.threads[index].admission.clone() {
            if let Some(position) = self
                .scheduler
                .reservations
                .iter()
                .position(|r| r.id == current && r.state != ReservationState::Released)
            {
                if !new_attempt {
                    self.sync_admission_flag(index);
                    return;
                }
                // Supersede in place: the reservation keeps its queue
                // position and never drops out of the ledger, so capacity
                // counting cannot see a gap where the old run was already
                // released but the new one not yet issued.
                self.threads[index].admission_run += 1;
                let run = self.threads[index].admission_run;
                let captain = if peer_worker {
                    None
                } else {
                    self.threads[index].crew_captain
                };
                let coordinator = captain.and_then(|captain| self.captain_home(captain));
                let fair = if let Some(origin) = self.threads[index].peer_origin.as_ref() {
                    format!("origin:{}", origin.server_id)
                } else if let Some(home) = coordinator {
                    format!("coordinator:{home}")
                } else {
                    format!("thread:{thread_id}")
                };
                let provider = Self::thread_provider(&self.threads[index]);
                let workspace = self.threads[index].project;
                let reservation = &mut self.scheduler.reservations[position];
                reservation.subject = if peer_worker {
                    Subject::PeerWorker { thread_id, run }
                } else {
                    Subject::Crew { thread_id, run }
                };
                reservation.coordinator = coordinator;
                reservation.fair = fair;
                reservation.provider = provider;
                reservation.workspace = workspace;
                reservation.captain = captain;
                self.scheduler.revision += 1;
                self.sync_admission_flag(index);
                return;
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
        // Only a provider mid-turn already occupies the slot truthfully;
        // everything else waits for a grant, pending input included.
        let running = self.threads[index].pi.is_some() && self.threads[index].status.busy();
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
        self.scheduler.revision += 1;
        self.threads[index].admission = Some(id);
        self.sync_admission_flag(index);
    }

    /// Whether a crew member or peer worker still has something a live
    /// provider slot should cover: a busy or queued turn, an unacknowledged
    /// result, or a checkout still being provisioned. An open provider
    /// connection alone is not work — an idle CLI holds no slot.
    fn admission_active(&self, index: usize) -> bool {
        let thread = &self.threads[index];
        thread.status.busy()
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

    /// The runtime gate provider delivery checks on every managed thread:
    /// set while a covering reservation is queued, and for a managed thread
    /// still waiting for lazy admission. Unmanaged interactive threads never
    /// set it, so their sends behave exactly as before.
    fn sync_admission_flag(&mut self, index: usize) {
        let thread_id = self.threads[index].id;
        let managed =
            self.threads[index].peer_origin.is_some() || self.threads[index].crew_captain.is_some();
        let blocked = match self.reservation_covering_thread(thread_id) {
            Some(reservation) => reservation.state == ReservationState::Queued,
            None => managed,
        };
        self.threads[index].admission_blocked = blocked;
    }

    /// Idempotent release: the first call decides; later calls are no-ops.
    fn release_reservation_at(&mut self, index: usize, reason: &'static str) {
        let reservation = &mut self.scheduler.reservations[index];
        if reservation.state == ReservationState::Released {
            return;
        }
        let provider = reservation.provider.clone();
        let subject = reservation.subject.clone();
        reservation.state = ReservationState::Released;
        reservation.released_at = Some(lifecycle::now());
        reservation.release_reason = Some(reason.into());
        reservation.blocked_by = None;
        reservation.uncertain = false;
        if reason == "completed" || reason == "settled" {
            self.scheduler.backoff_strikes.remove(&provider);
            self.scheduler.provider_backoff.remove(&provider);
        }
        self.scheduler.revision += 1;
        self.scheduler.prune_released();
        if let Subject::Crew { thread_id, .. } | Subject::PeerWorker { thread_id, .. } = subject
            && let Some(index) = self
                .threads
                .iter()
                .position(|thread| thread.id == thread_id)
        {
            self.sync_admission_flag(index);
        }
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
        self.scheduler.revision += 1;
    }

    /// Whether `connect_thread` could spawn a provider at all. In demo, headless
    /// client, or disabled-runtime states it returns without spawning, which
    /// must not read as a refused start.
    fn spawn_possible(&self) -> bool {
        !self.demo && self.runtime_enabled && self.client.is_none() && !self.ownership_denied
    }

    /// Release reservations whose subject reached an explicit terminal
    /// transition, was superseded, or vanished. Uncertain subjects keep
    /// their slot until the outcome is proven. Releases are collected by
    /// reservation id, never by position: `release_reservation_at` prunes
    /// the released audit tail, so any index saved earlier can go stale.
    fn scheduler_reconcile(&mut self) -> bool {
        let mut releases: Vec<(String, &'static str)> = Vec::new();
        let mut failures: Vec<(String, String)> = Vec::new();
        let mut uncertain: Vec<(String, bool)> = Vec::new();
        let mut requeues: Vec<String> = Vec::new();
        let mut adoptions: Vec<(String, usize)> = Vec::new();
        for index in 0..self.scheduler.reservations.len() {
            let reservation = self.scheduler.reservations[index].clone();
            if reservation.state == ReservationState::Released {
                continue;
            }
            match &reservation.subject {
                Subject::Task {
                    task_id,
                    attempt_id,
                } => {
                    let Some(task) = self.tasks.iter().find(|task| task.id == *task_id) else {
                        releases.push((reservation.id.clone(), "subject_gone"));
                        continue;
                    };
                    if task.attempt_id != *attempt_id {
                        if Self::remote_occupancy_unproven(task, attempt_id) {
                            uncertain.push((reservation.id.clone(), true));
                        } else {
                            releases.push((reservation.id.clone(), "superseded"));
                        }
                        continue;
                    }
                    if task.recovering {
                        uncertain.push((reservation.id.clone(), true));
                        continue;
                    }
                    if task.state.finished() {
                        if Self::remote_occupancy_unproven(task, attempt_id) {
                            // Home marked the task done before an owed remote
                            // stop landed; the target may still occupy a
                            // worker, so the slot stays uncertain.
                            uncertain.push((reservation.id.clone(), true));
                            continue;
                        }
                        let reason: &'static str = match task.state {
                            TaskState::Completed => "completed",
                            TaskState::Cancelled => "cancelled",
                            _ => "failed",
                        };
                        if reason == "failed" && !task.error_excerpt.is_empty() {
                            failures
                                .push((reservation.provider.clone(), task.error_excerpt.clone()));
                        }
                        releases.push((reservation.id.clone(), reason));
                        continue;
                    }
                    uncertain.push((
                        reservation.id.clone(),
                        task.remote
                            .as_ref()
                            .is_some_and(|remote| remote.state == RemoteState::TargetOffline),
                    ));
                    if task.state == TaskState::Interrupted {
                        releases.push((reservation.id.clone(), "interrupted"));
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
                                requeues.push(reservation.id.clone());
                            }
                            Some(child) if child.status == Status::Failed => {
                                if let Some(error) = child.error.clone() {
                                    failures.push((reservation.provider.clone(), error));
                                }
                                releases.push((reservation.id.clone(), "failed"));
                            }
                            None => releases.push((reservation.id.clone(), "subject_gone")),
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
                        releases.push((reservation.id.clone(), "thread_gone"));
                        continue;
                    };
                    let thread = &self.threads[position];
                    if thread.archived() {
                        releases.push((reservation.id.clone(), "thread_gone"));
                        continue;
                    }
                    if thread.admission.as_deref() != Some(reservation.id.as_str()) {
                        // A restored ledger still holds this thread's
                        // reservation while the thread lost its recorded id;
                        // adopt the sole live reservation rather than
                        // releasing capacity the work still needs.
                        let recorded_live = thread.admission.as_deref().is_some_and(|id| {
                            self.scheduler
                                .reservations
                                .iter()
                                .any(|r| r.id == id && r.state != ReservationState::Released)
                        });
                        let siblings = self
                            .scheduler
                            .reservations
                            .iter()
                            .filter(|r| {
                                r.state != ReservationState::Released
                                    && r.subject.thread_id() == Some(*thread_id)
                            })
                            .count();
                        if !recorded_live && siblings == 1 {
                            adoptions.push((reservation.id.clone(), position));
                        } else {
                            releases.push((reservation.id.clone(), "superseded"));
                            continue;
                        }
                    }
                    if !peer_worker && thread.crew_captain != reservation.captain {
                        releases.push((reservation.id.clone(), "left_crew"));
                        continue;
                    }
                    if thread.recovering_task {
                        uncertain.push((reservation.id.clone(), true));
                        continue;
                    }
                    uncertain.push((reservation.id.clone(), false));
                    if !self.admission_active(position) {
                        if thread.status == Status::Failed
                            && let Some(error) = thread.error.clone()
                        {
                            failures.push((reservation.provider.clone(), error));
                        }
                        releases.push((
                            reservation.id.clone(),
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
                            releases.push((reservation.id.clone(), "failed"));
                        } else {
                            requeues.push(reservation.id.clone());
                        }
                    }
                }
            }
        }
        let mut changed = !releases.is_empty() || !adoptions.is_empty();
        for (id, value) in uncertain {
            if let Some(reservation) = self.scheduler.reservations.iter_mut().find(|r| r.id == id) {
                if reservation.uncertain != value {
                    reservation.uncertain = value;
                    changed = true;
                }
            }
        }
        for (id, position) in adoptions {
            let run = self
                .scheduler
                .reservations
                .iter()
                .find(|r| r.id == id)
                .and_then(|r| match r.subject {
                    Subject::Crew { run, .. } | Subject::PeerWorker { run, .. } => Some(run),
                    _ => None,
                })
                .unwrap_or(0);
            self.threads[position].admission = Some(id);
            self.threads[position].admission_run = self.threads[position].admission_run.max(run);
            self.sync_admission_flag(position);
        }
        for (provider, error) in failures {
            self.scheduler_note_failure(&provider, &error);
            changed = true;
        }
        for (id, reason) in releases {
            if let Some(position) = self.scheduler.reservations.iter().position(|r| r.id == id) {
                self.release_reservation_at(position, reason);
            }
        }
        for id in requeues {
            let Some(position) = self.scheduler.reservations.iter().position(|r| r.id == id) else {
                continue;
            };
            let reservation = &mut self.scheduler.reservations[position];
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
            self.scheduler.revision += 1;
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
        let coordinator_key = reservation.coordinator_key();
        if !coordinator_key.is_empty()
            && let Some(limit) = limits.effective(
                limits.coordinator_running,
                &limits.overrides.coordinator,
                &coordinator_key,
            )
            && counts
                .coordinator
                .get(&coordinator_key)
                .copied()
                .unwrap_or(0)
                >= limit
        {
            return Some("coordinator".into());
        }
        if let Some(captain) = reservation.captain
            && let Some(limit) = limits.effective(
                limits.captain_running,
                &limits.overrides.captain,
                &captain.to_string(),
            )
            && counts.captain.get(&captain).copied().unwrap_or(0) >= limit
        {
            return Some("captain".into());
        }
        if let Some(workspace) = reservation.workspace
            && let Some(limit) = limits.effective(
                limits.workspace_running,
                &limits.overrides.workspace,
                &workspace.to_string(),
            )
            && counts.workspace.get(&workspace).copied().unwrap_or(0) >= limit
        {
            return Some("workspace".into());
        }
        if let Some(limit) = limits.effective(
            limits.provider_running,
            &limits.overrides.provider,
            &reservation.provider,
        ) && counts
            .provider
            .get(&reservation.provider)
            .copied()
            .unwrap_or(0)
            >= limit
        {
            return Some("provider".into());
        }
        if let Some(limit) = limits.effective(
            limits.executor_running,
            &limits.overrides.executor,
            &reservation.executor,
        ) && counts
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
                self.scheduler.revision += 1;
                self.tasks[task_index].state = TaskState::Starting;
                self.tasks[task_index].started_at = Some(lifecycle::now());
                self.sync_admission_flag(child);
                if self.tasks[task_index].remote.is_some() {
                    // Committed state only: dispatch_peer_work sends the
                    // launch command on the next tick.
                    return true;
                }
                // Persist intent before any prompt can reach the provider.
                if self.runtime_enabled
                    && let Err(error) = self.admission_commit()
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
                    peer_input: None,
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
                self.scheduler.revision += 1;
                self.sync_admission_flag(index);
                self.connect_thread(index);
                true
            }
        }
    }

    /// The live reservation, if any, that covers this thread: its own crew
    /// or peer-worker reservation, or a delegated attempt reservation whose
    /// task names it as the child. A delegated child carrying a Captain
    /// scope must not double-count under two reservations.
    pub(super) fn reservation_covering_thread(&self, thread_id: usize) -> Option<&Reservation> {
        self.scheduler.reservations.iter().find(|r| {
            r.state != ReservationState::Released
                && match &r.subject {
                    Subject::Crew { thread_id: id, .. }
                    | Subject::PeerWorker { thread_id: id, .. } => *id == thread_id,
                    Subject::Task { task_id, .. } => self
                        .tasks
                        .iter()
                        .any(|task| task.id == *task_id && task.child_thread_id == thread_id),
                }
        })
    }

    /// Reconcile every live reservation, refresh queue reasons, then grant
    /// in fairness order until no candidate can start. Replaces the old
    /// per-home running-count loop in `start_queued_tasks`.
    pub(super) fn admission_pump(&mut self) -> bool {
        // Lazily admit subjects that predate the scheduler or lost their
        // reservation to a crash: delegated attempts without a live
        // reservation and crew or peer threads still holding live work.
        let mut changed = false;
        for index in 0..self.tasks.len() {
            let uncovered = !self.scheduler.reservations.iter().any(|r| {
                r.state != ReservationState::Released
                    && matches!(&r.subject, Subject::Task { task_id, .. }
                        if *task_id == self.tasks[index].id)
            });
            match self.tasks[index].state {
                // A queued attempt joins the line; an attempt already
                // starting or running backfills a held reservation so its
                // occupancy is counted rather than silently uncapped.
                TaskState::Queued | TaskState::Starting | TaskState::Running if uncovered => {
                    self.admit_task_index(index);
                    changed = true;
                }
                _ => {}
            }
        }
        for index in 0..self.threads.len() {
            // Backfill only threads that never held a reservation: crew or
            // peer workers restored from an index that predates the
            // scheduler. A thread whose reservation released still carries
            // its id on `admission`, and re-admitting it here would mint a
            // phantom subject for work that is merely winding down. Its next
            // run enters through an explicit send, not the pump.
            // A delegated child is covered by its task reservation even
            // after that task finishes; admitting it again as crew would
            // double-count it. Any other managed thread with a released
            // line re-admits when fresh input arrives.
            let managed = (self.threads[index].peer_origin.is_some()
                || self.threads[index].crew_captain.is_some())
                && !self
                    .tasks
                    .iter()
                    .any(|task| task.child_thread_id == self.threads[index].id);
            let covered = self
                .reservation_covering_thread(self.threads[index].id)
                .is_some();
            if managed && !covered && self.admission_active(index) {
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
                    self.scheduler.revision += 1;
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
                    self.scheduler.revision += 1;
                }
                break;
            }
            changed = true;
        }
        // Runtime delivery gates mirror the final ledger state.
        for index in 0..self.threads.len() {
            let blocked = self.threads[index].admission_blocked;
            self.sync_admission_flag(index);
            changed |= blocked != self.threads[index].admission_blocked;
        }
        changed
    }

    /// The scheduler's own queue position for a thread-bound reservation.
    pub(super) fn admission_value_for(&self, thread_id: usize) -> Value {
        let Some(reservation) = self.reservation_covering_thread(thread_id) else {
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

    /// Per-id usage rows for one scope class, including ids that only
    /// queue: a scope the queue waits on is as visible as one that runs.
    fn scope_usage_rows(
        &self,
        scope: &str,
        key_of: impl Fn(&Reservation) -> Option<String>,
        class: Option<usize>,
        overrides: &BTreeMap<String, usize>,
    ) -> Vec<Value> {
        let mut rows: BTreeMap<String, [usize; 4]> = BTreeMap::new();
        for reservation in self
            .scheduler
            .reservations
            .iter()
            .filter(|r| r.state != ReservationState::Released)
        {
            let Some(id) = key_of(reservation) else {
                continue;
            };
            let row = rows.entry(id).or_default();
            match reservation.state {
                ReservationState::Queued => row[3] += 1,
                ReservationState::Held => {
                    row[0] += 1;
                    if self.reservation_starting(reservation) {
                        row[1] += 1;
                    }
                    if reservation.uncertain {
                        row[2] += 1;
                    }
                }
                ReservationState::Released => {}
            }
        }
        rows.into_iter()
            .map(|(id, [held, starting, uncertain, queued])| {
                json!({
                    "scope": scope,
                    "id": id,
                    "held": held,
                    "starting": starting,
                    "uncertain": uncertain,
                    "queued": queued,
                    "limit": overrides.get(&id).copied().or(class),
                    "classLimit": class,
                })
            })
            .collect()
    }

    /// The authoritative scheduling view the `scheduling.status` RPC serves:
    /// effective limits with defaults and editability, per-scope usage
    /// (held, starting, uncertain, queued), the ordered queue with reasons
    /// and retry times, held reservations with uncertainty, provider
    /// backoff, and the durable revision subscribers compare against.
    pub fn scheduling_status_value(&self) -> Value {
        let order = self.scheduler.grant_order(&self.scheduler.held_counts());
        let now = lifecycle::now();
        let limits = &self.scheduler.limits;
        let defaults = Limits::default();
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
        let mut providers = self.scope_usage_rows(
            "provider",
            |r| Some(r.provider.clone()),
            limits.provider_running,
            &limits.overrides.provider,
        );
        for row in &mut providers {
            let id = row["id"].as_str().unwrap_or_default();
            row["backoffUntil"] = self
                .scheduler
                .provider_backoff
                .get(id)
                .filter(|until| **until > now)
                .map(|until| json!(until))
                .unwrap_or(Value::Null);
        }
        for (provider, until) in &self.scheduler.provider_backoff {
            if *until > now
                && !providers
                    .iter()
                    .any(|row| row["id"].as_str() == Some(provider.as_str()))
            {
                providers.push(json!({
                    "scope": "provider",
                    "id": provider,
                    "held": 0,
                    "starting": 0,
                    "uncertain": 0,
                    "queued": 0,
                    "limit": limits.effective(
                        limits.provider_running,
                        &limits.overrides.provider,
                        provider,
                    ),
                    "classLimit": limits.provider_running,
                    "backoffUntil": until,
                }));
            }
        }
        let coordinators = self.scope_usage_rows(
            "coordinator",
            |r| (!r.coordinator_key().is_empty()).then(|| r.coordinator_key()),
            limits.coordinator_running,
            &limits.overrides.coordinator,
        );
        let captains = self.scope_usage_rows(
            "captain",
            |r| r.captain.map(|captain| captain.to_string()),
            limits.captain_running,
            &limits.overrides.captain,
        );
        let executors = self.scope_usage_rows(
            "executor",
            |r| Some(r.executor.clone()),
            limits.executor_running,
            &limits.overrides.executor,
        );
        let workspaces = self.scope_usage_rows(
            "workspace",
            |r| r.workspace.map(|workspace| workspace.to_string()),
            limits.workspace_running,
            &limits.overrides.workspace,
        );
        let pending = self
            .scheduler
            .reservations
            .iter()
            .filter(|r| r.state == ReservationState::Queued)
            .count();
        let scope_meta = |scope: &str, limit: Option<usize>, default: Option<usize>| json!({"scope":scope,"limit":limit,"default":default,"editable":true});
        json!({
            "revision": self.scheduler.revision,
            "limits": limits.value(),
            "defaults": defaults.value(),
            "scopes": [
                scope_meta("account", limits.account_running, defaults.account_running),
                scope_meta("coordinator", limits.coordinator_running, defaults.coordinator_running),
                scope_meta("coordinatorUnfinished", limits.coordinator_unfinished, defaults.coordinator_unfinished),
                scope_meta("captain", limits.captain_running, defaults.captain_running),
                scope_meta("executor", limits.executor_running, defaults.executor_running),
                scope_meta("provider", limits.provider_running, defaults.provider_running),
                scope_meta("workspace", limits.workspace_running, defaults.workspace_running),
                scope_meta("pending", limits.account_pending, defaults.account_pending),
            ],
            "usage": {
                "account": {
                    "held": held.len(),
                    "starting": self.scheduler.reservations.iter()
                        .filter(|r| self.reservation_starting(r)).count(),
                    "uncertain": self.scheduler.reservations.iter()
                        .filter(|r| r.state == ReservationState::Held && r.uncertain).count(),
                    "queued": pending,
                    "limit": limits.account_running,
                    "pendingLimit": limits.account_pending,
                },
                "coordinators": coordinators,
                "captains": captains,
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
        // The caller's stable scope key: a Captain home counts under
        // "captain:<id>", any other thread under "home:<id>".
        let key = coordinator.map(|home| {
            let captain = self
                .threads
                .iter()
                .find(|thread| thread.id == home && thread.captain_home)
                .and_then(|thread| thread.captain);
            captain
                .map(|captain| format!("captain:{captain}"))
                .unwrap_or_else(|| format!("home:{home}"))
        });
        json!({
            "revision": self.scheduler.revision,
            "limits": self.scheduler.limits.value(),
            "coordinator": coordinator.map(|home| {
                let key = key.clone().unwrap_or_default();
                json!({
                    "id": home,
                    "key": key,
                    "running": counts.coordinator.get(&key).copied().unwrap_or(0),
                    "uncertain": self.scheduler.reservations.iter().filter(|r| {
                        r.state == ReservationState::Held && r.uncertain
                            && r.coordinator_key() == key
                    }).count(),
                    "limit": self.scheduler.limits.effective(
                        self.scheduler.limits.coordinator_running,
                        &self.scheduler.limits.overrides.coordinator,
                        &key,
                    ),
                })
            }),
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

    /// One configure outcome for the UI contract: accepted, stale, or
    /// rejected, always with the authoritative revision and full status so
    /// the caller lands on the same projection the server persists.
    fn configure_receipt(&self, outcome: &str, reason: Option<String>) -> Value {
        json!({
            "outcome": outcome,
            "reason": reason,
            "revision": self.scheduler.revision,
            "status": self.scheduling_status_value(),
        })
    }

    /// `scheduling.configure`: parses limits and per-id overrides onto a
    /// clone, validates the whole result, then commits it in one write. A
    /// rejected or stale call changes nothing, so a later invalid field can
    /// never leave the earlier ones half-applied. `expectRevision` is the
    /// compare-and-set fence: a caller holding an older revision gets a
    /// stale receipt instead of silently overwriting a newer config.
    /// Lowering a cap only queues future grants; live work is never killed
    /// to satisfy a new limit.
    pub(super) fn scheduling_configure(&mut self, args: &Value) -> Result<Value, String> {
        if let Some(expect) = args.get("expectRevision") {
            let expect = expect
                .as_u64()
                .ok_or("expectRevision must be the revision from scheduling.status.")?;
            if expect != self.scheduler.revision {
                return Ok(self.configure_receipt(
                    "stale",
                    Some(format!(
                        "The scheduler is at revision {}, not {expect}.",
                        self.scheduler.revision
                    )),
                ));
            }
        }
        let mut limits = self.scheduler.limits.clone();
        if let Some(value) = args.get("limits") {
            let fields = value
                .as_object()
                .ok_or("limits must be an object of scope limits.")?;
            let setters: &[(&str, fn(&mut Limits) -> &mut Option<usize>)] = &[
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
                ("captainRunning", |limits| &mut limits.captain_running),
                ("workspaceRunning", |limits| &mut limits.workspace_running),
            ];
            for (name, value) in fields {
                let Some(setter) = setters.iter().find(|(field, _)| field == name) else {
                    return Ok(self.configure_receipt(
                        "rejected",
                        Some(format!("Unknown scheduler limit {name}.")),
                    ));
                };
                let parsed = if value.is_null() {
                    None
                } else {
                    match value
                        .as_u64()
                        .filter(|limit| *limit >= 1 && *limit <= usize::MAX as u64)
                        .and_then(|limit| usize::try_from(limit).ok())
                    {
                        Some(limit) => Some(limit),
                        None => {
                            return Ok(self.configure_receipt(
                                "rejected",
                                Some(format!("{name} must be null or a positive integer.")),
                            ));
                        }
                    }
                };
                *setter.1(&mut limits) = parsed;
            }
        }
        if let Some(value) = args.get("overrides") {
            let scopes = value
                .as_object()
                .ok_or("overrides must be an object of scope override maps.")?;
            for (scope, entries) in scopes {
                let map = match scope.as_str() {
                    "coordinator" => &mut limits.overrides.coordinator,
                    "captain" => &mut limits.overrides.captain,
                    "executor" => &mut limits.overrides.executor,
                    "provider" => &mut limits.overrides.provider,
                    "workspace" => &mut limits.overrides.workspace,
                    _ => {
                        return Ok(self.configure_receipt(
                            "rejected",
                            Some(format!("Unknown override scope {scope}.")),
                        ));
                    }
                };
                let Some(entries) = entries.as_object() else {
                    return Ok(self.configure_receipt(
                        "rejected",
                        Some(format!("overrides.{scope} must map scope ids to caps.")),
                    ));
                };
                for (id, cap) in entries {
                    if cap.is_null() {
                        map.remove(id);
                        continue;
                    }
                    match cap
                        .as_u64()
                        .filter(|cap| *cap >= 1 && *cap <= usize::MAX as u64)
                        .and_then(|cap| usize::try_from(cap).ok())
                    {
                        Some(cap) => {
                            map.insert(id.clone(), cap);
                        }
                        None => {
                            return Ok(self.configure_receipt(
                                "rejected",
                                Some(format!(
                                    "overrides.{scope}.{id} must be null or a positive integer."
                                )),
                            ));
                        }
                    }
                }
            }
        }
        if !limits.valid() {
            return Ok(self.configure_receipt(
                "rejected",
                Some("The resulting scheduler limits are invalid.".into()),
            ));
        }
        self.scheduler.limits = limits;
        self.scheduler.revision += 1;
        self.admission_pump();
        Ok(self.configure_receipt("accepted", None))
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
            peer_input: None,
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
            evidence_supported: false,
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
    fn configure_receipts_are_atomic_and_compare_on_revision() {
        let mut state = demo();
        let receipt = state
            .scheduling_configure(&json!({"limits":{"accountRunning":2,"providerRunning":null}}))
            .unwrap();
        assert_eq!(receipt["outcome"], "accepted");
        assert_eq!(receipt["status"]["limits"]["accountRunning"], 2);
        assert_eq!(receipt["status"]["limits"]["providerRunning"], Value::Null);
        let revision = receipt["revision"].as_u64().unwrap();
        // A rejected call applies nothing, not even the valid earlier field.
        let rejected = state
            .scheduling_configure(&json!({"limits":{"executorRunning":4,"accountRunning":0}}))
            .unwrap();
        assert_eq!(rejected["outcome"], "rejected");
        assert_eq!(
            state.scheduler.limits.executor_running,
            Some(EXECUTOR_RUNNING)
        );
        assert_eq!(state.scheduler.limits.account_running, Some(2));
        assert_eq!(rejected["revision"].as_u64().unwrap(), revision);
        // An unknown key rejects the same way.
        let rejected = state
            .scheduling_configure(&json!({"limits":{"bogus":1}}))
            .unwrap();
        assert_eq!(rejected["outcome"], "rejected");
        // A caller holding an old revision gets a stale receipt, not a write.
        let stale = state
            .scheduling_configure(&json!({"expectRevision":0,"limits":{"accountRunning":9}}))
            .unwrap();
        assert_eq!(stale["outcome"], "stale");
        assert_eq!(state.scheduler.limits.account_running, Some(2));
        // The current revision commits and returns the new projection.
        let accepted = state
            .scheduling_configure(&json!({"expectRevision":revision,"limits":{"accountRunning":9}}))
            .unwrap();
        assert_eq!(accepted["outcome"], "accepted");
        assert!(accepted["revision"].as_u64().unwrap() > revision);
        assert_eq!(state.scheduler.limits.account_running, Some(9));
    }

    #[test]
    fn configure_scopes_per_id_overrides() {
        let mut state = demo();
        captain(&mut state, 0, "Lead");
        captain(&mut state, 1, "Scout");
        let receipt = state
            .scheduling_configure(&json!({"overrides":{"coordinator":{"captain:0":1}}}))
            .unwrap();
        assert_eq!(receipt["outcome"], "accepted");
        // Captain 0 runs one provider; captain 1 keeps the class limit.
        let first = crew_member(&mut state, 0, "first");
        let second = crew_member(&mut state, 0, "second");
        let other = crew_member(&mut state, 1, "other captain");
        state.admission_pump();
        assert_eq!(
            reservation_for(&state, state.threads[first].id).state,
            ReservationState::Held
        );
        let queued = reservation_for(&state, state.threads[second].id);
        assert_eq!(queued.state, ReservationState::Queued);
        assert_eq!(queued.blocked_by.as_deref(), Some("coordinator"));
        assert_eq!(queued.coordinator_key(), "captain:0");
        assert_eq!(
            reservation_for(&state, state.threads[other].id).state,
            ReservationState::Held
        );
        // A bad override map rejects without touching the committed one.
        let rejected = state
            .scheduling_configure(&json!({"overrides":{"provider":{"cpa":0}}}))
            .unwrap();
        assert_eq!(rejected["outcome"], "rejected");
        assert_eq!(state.scheduler.limits.overrides.coordinator["captain:0"], 1);
        assert!(state.scheduler.limits.overrides.provider.is_empty());
    }

    /// Registers an active Captain with a home thread, the minimum state the
    /// owner and scope checks need.
    fn captain(state: &mut State, id: usize, name: &str) {
        state.captains.push(crate::captains::Captain {
            name: name.into(),
            role: String::new(),
            instructions: String::new(),
            portrait: 0,
        });
        state
            .captain_lifecycles
            .insert(id, captain_lifecycle::CaptainLifecycle::default());
        let home_id = state.next_id;
        state.next_id += 1;
        let mut home = Thread::new(home_id);
        home.captain = Some(id);
        home.captain_home = true;
        state.threads.push(home);
        state.captain_homes.insert(id, home_id);
    }

    /// A managed crew member of an opened Captain with live input staged.
    fn crew_member(state: &mut State, captain: usize, text: &str) -> usize {
        let id = state.next_id;
        state.next_id += 1;
        let mut thread = Thread::new(id);
        thread.model_provider = "cpa".into();
        thread.crew_captain = Some(captain);
        thread.pending_injections.push_back(Injection {
            text: text.into(),
            shown: false,
            steer: None,
            peer_input: None,
        });
        state.threads.push(thread);
        let index = state.threads.len() - 1;
        state.ensure_thread_admission(index, false, false);
        index
    }

    /// A released audit record for history-pressure tests.
    fn released_record(sequence: u64) -> Reservation {
        Reservation {
            id: format!("adm-hist-{sequence}"),
            subject: Subject::PeerWorker {
                thread_id: usize::MAX - sequence as usize,
                run: 1,
            },
            state: ReservationState::Released,
            coordinator: None,
            fair: "origin:hist".into(),
            executor: "local".into(),
            provider: "cpa".into(),
            workspace: None,
            captain: None,
            priority: 0,
            queued_at: 1,
            granted_at: Some(1),
            released_at: Some(2),
            release_reason: Some("settled".into()),
            blocked_by: None,
            not_before: 0,
            uncertain: false,
        }
    }

    #[test]
    fn a_long_released_history_does_not_corrupt_reconcile() {
        let mut state = demo();
        // More than RELEASED_KEEP released records plus two live workers that
        // settle in one reconcile pass. Releasing by saved index would hit a
        // stale position after the first release prunes; ids stay valid.
        for sequence in 0..205 {
            state.scheduler.reservations.push(released_record(sequence));
        }
        let first = worker(&mut state, "first");
        let second = worker(&mut state, "second");
        state.admission_pump();
        state.threads[first].pending_injections.clear();
        state.threads[second].pending_injections.clear();
        state.admission_pump();
        let first_id = state.threads[first].id;
        let second_id = state.threads[second].id;
        let released: Vec<&Reservation> = state
            .scheduler
            .reservations
            .iter()
            .filter(|r| r.state == ReservationState::Released)
            .collect();
        assert!(
            released
                .iter()
                .any(|r| r.subject.thread_id() == Some(first_id)
                    && r.release_reason.as_deref() == Some("settled")),
            "the first worker's reservation released"
        );
        assert!(
            released
                .iter()
                .any(|r| r.subject.thread_id() == Some(second_id)
                    && r.release_reason.as_deref() == Some("settled")),
            "the second worker's reservation released after pruning"
        );
    }

    #[test]
    fn a_new_attempt_supersedes_in_place_without_a_gap() {
        let mut state = demo();
        let index = worker(&mut state, "first run");
        state.admission_pump();
        let id = state.threads[index].admission.clone().unwrap();
        // A new attempt never releases the reservation first: the ledger
        // keeps exactly one live record throughout the supersede.
        state.ensure_thread_admission(index, false, true);
        assert_eq!(state.threads[index].admission.as_deref(), Some(id.as_str()));
        let reservation = reservation_for(&state, state.threads[index].id);
        assert_eq!(reservation.state, ReservationState::Held);
        assert!(matches!(reservation.subject, Subject::Crew { run: 2, .. }));
        let live = state
            .scheduler
            .reservations
            .iter()
            .filter(|r| {
                r.state != ReservationState::Released
                    && r.subject.thread_id() == Some(state.threads[index].id)
            })
            .count();
        assert_eq!(live, 1);
    }

    #[test]
    fn a_ready_crew_placement_still_waits_for_account_capacity() {
        let mut state = demo();
        state.scheduler.limits.account_running = Some(1);
        captain(&mut state, 0, "Cap");
        let first = worker(&mut state, "occupying capacity");
        let second = crew_member(&mut state, 0, "waiting for placement");
        state.admission_pump();
        state.finish_placement_ready(second);
        let queued = reservation_for(&state, state.threads[second].id);
        assert_eq!(queued.state, ReservationState::Queued);
        assert_eq!(queued.blocked_by.as_deref(), Some("account"));
        assert!(state.threads[second].admission_blocked);

        state.threads[first].pending_injections.clear();
        state.finish_placement_ready(second);
        assert_eq!(
            reservation_for(&state, state.threads[second].id).state,
            ReservationState::Held
        );
        assert!(!state.threads[second].admission_blocked);
    }

    #[test]
    fn restart_keeps_only_proven_never_granted_tasks_in_the_queue() {
        for previously_granted in [false, true] {
            let mut state = demo();
            state.scheduler.limits.account_running = Some(1);
            worker(&mut state, "occupying capacity");
            state.admission_pump();
            state
                .mcp_call(1, "delegate_task", &json!({"prompt":"queued brief"}))
                .unwrap();
            assert_eq!(state.tasks[0].state, TaskState::Queued);
            let task_id = state.tasks[0].id.clone();
            let reservation = state.scheduler.reservations.iter_mut().find(|row| {
                matches!(&row.subject, Subject::Task { task_id: id, .. } if *id == task_id)
            }).unwrap();
            assert_eq!(reservation.state, ReservationState::Queued);
            assert!(reservation.granted_at.is_none());
            if previously_granted {
                reservation.granted_at = Some(lifecycle::now());
            }
            state.prepare_task_recovery();
            assert_eq!(
                state.tasks[0].state,
                if previously_granted {
                    TaskState::Interrupted
                } else {
                    TaskState::Queued
                }
            );
            assert!(!state.tasks[0].recovering);
        }
    }

    #[test]
    fn a_cancelled_remote_attempt_stays_uncertain_until_the_stop_lands() {
        let mut state = demo();
        state.scheduler.limits.account_running = Some(1);
        state
            .mcp_call(1, "delegate_task", &json!({"prompt":"Remote work"}))
            .unwrap();
        state.tasks[0].state = TaskState::Running;
        let attempt = state.tasks[0].attempt_id.clone();
        // Home cancelled before the target acknowledged the stop: the
        // process may still occupy a worker there.
        state.tasks[0].remote = Some(remote_fixture("peer-1", true, Some(&attempt)));
        state.tasks[0].finish(TaskState::Cancelled, "", "cancelled");
        state.admission_pump();
        let reservation = &state.scheduler.reservations[0];
        assert_eq!(reservation.state, ReservationState::Held);
        assert!(reservation.uncertain);
        state
            .mcp_call(
                1,
                "delegate_task",
                &json!({"prompt":"Waiting for capacity"}),
            )
            .unwrap();
        assert_eq!(state.tasks[1].state, TaskState::Queued);

        // The actual peer receipt clears stop_owed but proves transport only.
        let (send, receive) = std::sync::mpsc::channel();
        send.send(Ok(json!({"status":"accepted","result":{"stopped":true}})))
            .unwrap();
        state.peer_runtime.inject_pending(
            state.tasks[0].id.clone(),
            state.tasks[0].remote.as_ref().unwrap(),
            "peer.thread.stop".into(),
            receive,
        );
        assert!(state.poll_remote_tasks_for_test());
        let remote = state.tasks[0].remote.as_ref().unwrap();
        assert!(!remote.stop_owed);
        assert_eq!(remote.state, RemoteState::Running);
        state.admission_pump();
        assert_eq!(
            state.scheduler.reservations[0].state,
            ReservationState::Held
        );
        assert!(state.scheduler.reservations[0].uncertain);
        assert_eq!(state.tasks[1].state, TaskState::Queued);

        // Only a later provider-terminal observation permits another grant.
        let (send, receive) = std::sync::mpsc::channel();
        send.send(Ok(json!({"status":"accepted","result":{
            "revision":"provider-settled","finished":true,"attemptId":attempt,
            "result":{"status":"cancelled","attemptId":attempt}}})))
            .unwrap();
        state.peer_runtime.inject_pending(
            state.tasks[0].id.clone(),
            state.tasks[0].remote.as_ref().unwrap(),
            "peer.thread.wait".into(),
            receive,
        );
        assert!(state.poll_remote_tasks_for_test());
        state.admission_pump();
        assert_eq!(
            state.scheduler.reservations[0].state,
            ReservationState::Released
        );
        assert_eq!(
            state.scheduler.reservations[0].release_reason.as_deref(),
            Some("cancelled")
        );
        assert_eq!(state.tasks[1].state, TaskState::Starting);
        let revision = state.scheduler.revision;
        state.admission_pump();
        assert_eq!(state.scheduler.revision, revision, "release is idempotent");
    }

    #[test]
    fn a_superseded_remote_attempt_keeps_unproven_occupancy() {
        let mut state = demo();
        state
            .mcp_call(1, "delegate_task", &json!({"prompt":"Remote work"}))
            .unwrap();
        state.tasks[0].state = TaskState::Running;
        let attempt_a = state.tasks[0].attempt_id.clone();
        state.tasks[0].remote = Some(remote_fixture("peer-1", true, Some(&attempt_a)));
        // A retry arrives while the old attempt's stop is still owed: the
        // new attempt gets its own reservation, and the old one keeps its
        // slot as uncertain rather than freeing it for a replacement.
        state.tasks[0].attempt_id = "attempt-b".into();
        state.tasks[0].state = TaskState::Queued;
        state.admit_task_index(0);
        state.admission_pump();
        let old = state
            .scheduler
            .reservations
            .iter()
            .find(|r| {
                matches!(&r.subject, Subject::Task { attempt_id, .. } if *attempt_id == attempt_a)
            })
            .unwrap();
        assert_eq!(old.state, ReservationState::Held);
        assert!(old.uncertain);
        let current = state
            .scheduler
            .reservations
            .iter()
            .find(|r| {
                matches!(&r.subject, Subject::Task { attempt_id, .. } if attempt_id == "attempt-b")
            })
            .unwrap();
        assert_eq!(current.state, ReservationState::Held);
    }

    #[test]
    fn remote_transport_failures_do_not_prove_cancelled_provider_settlement() {
        for remote_state in [
            RemoteState::Launching,
            RemoteState::Accepted,
            RemoteState::Running,
            RemoteState::TargetOffline,
            RemoteState::Revoked,
            RemoteState::Failed,
            RemoteState::UpdateRequired,
        ] {
            let mut state = demo();
            state
                .mcp_call(1, "delegate_task", &json!({"prompt":"Remote work"}))
                .unwrap();
            let mut remote = remote_fixture("peer-1", false, None);
            remote.state = remote_state;
            state.tasks[0].remote = Some(remote);
            state.tasks[0].finish(TaskState::Cancelled, "", "cancelled");
            state.admission_pump();
            assert_eq!(
                state.scheduler.reservations[0].state,
                ReservationState::Held,
                "{remote_state:?} is not provider-terminal evidence"
            );
            assert!(state.scheduler.reservations[0].uncertain);
        }
    }

    #[test]
    fn a_running_task_backfills_a_held_reservation() {
        let mut state = demo();
        state
            .mcp_call(1, "delegate_task", &json!({"prompt":"Legacy running work"}))
            .unwrap();
        // The ledger entry was lost (e.g. an older build); the attempt is
        // already running, so its occupancy is counted, not queued.
        state.scheduler.reservations.clear();
        state.tasks[0].state = TaskState::Running;
        state.admission_pump();
        assert_eq!(state.scheduler.reservations.len(), 1);
        let reservation = &state.scheduler.reservations[0];
        assert_eq!(reservation.state, ReservationState::Held);
        assert!(reservation.granted_at.is_some());
    }

    #[test]
    fn a_restored_ledger_reservation_is_adopted_by_its_thread() {
        let mut state = demo();
        let index = worker(&mut state, "survived restore");
        state.admission_pump();
        let id = state.threads[index].admission.clone().unwrap();
        // The thread record lost its reservation id (partial restore), but
        // the ledger still holds it; reconcile adopts it back instead of
        // releasing the capacity the live work needs.
        state.threads[index].admission = Some("adm-lost".into());
        state.admission_pump();
        assert_eq!(state.threads[index].admission.as_deref(), Some(id.as_str()));
        assert_eq!(
            reservation_for(&state, state.threads[index].id).state,
            ReservationState::Held
        );
    }

    #[test]
    fn settled_crew_input_queues_a_new_run_and_the_flag_gates_delivery() {
        let mut state = demo();
        captain(&mut state, 0, "Lead");
        state.scheduler.limits.account_running = Some(1);
        let busy = crew_member(&mut state, 0, "occupies the slot");
        let idle = crew_member(&mut state, 0, "first run");
        state.admission_pump();
        assert_eq!(
            reservation_for(&state, state.threads[busy].id).state,
            ReservationState::Held
        );
        // The idle member's turn finished and released; its flag blocks
        // delivery until a new reservation grants.
        state.threads[idle].pending_injections.clear();
        state.admission_pump();
        assert_eq!(
            reservation_for(&state, state.threads[idle].id).state,
            ReservationState::Released
        );
        assert!(state.threads[idle].admission_blocked);
        // Fresh input on the managed thread re-admits and waits its turn.
        state.threads[idle].pending_injections.push_back(Injection {
            text: "second run".into(),
            shown: false,
            steer: None,
            peer_input: None,
        });
        state.admission_pump();
        let queued = reservation_for(&state, state.threads[idle].id);
        assert_eq!(queued.state, ReservationState::Queued);
        assert_eq!(queued.blocked_by.as_deref(), Some("account"));
        assert!(state.threads[idle].admission_blocked);
        // The busy member settling frees the account slot; the queued input
        // grants and the delivery gate opens.
        state.threads[busy].pending_injections.clear();
        state.admission_pump();
        assert_eq!(
            reservation_for(&state, state.threads[idle].id).state,
            ReservationState::Held
        );
        assert!(!state.threads[idle].admission_blocked);
    }

    fn remote_fixture(server: &str, stop_owed: bool, stop_attempt: Option<&str>) -> RemoteTask {
        RemoteTask {
            server_id: server.into(),
            project: json!({}),
            label: "Peer".into(),
            thread_id: Some(41),
            launch_command: crate::protocol::Command::new("peer.thread.launch", json!({})),
            stop_owed,
            stop_command_id: stop_owed.then(uuid::Uuid::new_v4),
            stop_attempt_id: stop_attempt.map(str::to_owned),
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
            evidence_supported: false,
        }
    }
    #[test]
    fn a_queued_crew_roots_unsent_prompt_survives_a_restart() {
        let mut state = demo();
        state.scheduler.limits.account_running = Some(1);
        captain(&mut state, 0, "Cap");
        let _first = worker(&mut state, "first");
        let second = crew_member(&mut state, 0, "second");
        state.threads[second].captain = Some(0);
        state.threads[second].queued_prompt = Some((
            crate::attachments::Prompt::new("crew-b-fixture".into(), vec![]),
            0,
        ));
        state.threads[second].pending_injections.clear();
        state.admission_pump();
        let reservation = reservation_for(&state, state.threads[second].id);
        eprintln!(
            "state={:?} reason={:?} active={}",
            reservation.state,
            reservation.release_reason,
            state.admission_active(second)
        );
        assert_eq!(
            reservation_for(&state, state.threads[second].id).state,
            ReservationState::Queued
        );
        let snapshot = state.snapshot();
        let mut restored = State::new(PathBuf::new(), false);
        restored.demo = true;
        restored.restore_global(&snapshot).unwrap();
        let second_id = state.threads[second].id;
        let position = restored
            .threads
            .iter()
            .position(|thread| thread.id == second_id)
            .expect("restored crew root");
        assert!(
            restored.threads[position].queued_prompt.is_some(),
            "the accepted-but-unsent input must be restored with the thread"
        );
        // The first root's live work died with the process: its runtime
        // injections are not persisted, so the surviving queue line grants
        // the restored crew root exactly once.
        restored.admission_pump();
        assert_eq!(
            reservation_for(&restored, second_id).state,
            ReservationState::Held
        );
    }
}
