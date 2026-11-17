//! Durable execution leases for logical Captain threads.
//!
//! The controller owns intent, Captain membership and the logical thread. An
//! executor owns only one physical thread alias and provider session at a time.
//! Control calls name the exact lease and generation they observed. Expiry or a
//! lost connection makes a lease uncertain; it never authorizes a replacement.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const SCHEMA: u32 = 1;
pub const LEASE_MILLIS: u64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Prepared,
    Dispatched,
    ExecutorAccepted,
    ProviderAccepted,
    Running,
    Terminal,
    Uncertain,
    Fenced,
}

impl Phase {
    pub fn word(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Dispatched => "dispatched",
            Self::ExecutorAccepted => "executorAccepted",
            Self::ProviderAccepted => "providerAccepted",
            Self::Running => "running",
            Self::Terminal => "terminal",
            Self::Uncertain => "uncertain",
            Self::Fenced => "fenced",
        }
    }

    pub fn blocks_replacement(self) -> bool {
        matches!(
            self,
            Self::Prepared
                | Self::Dispatched
                | Self::ExecutorAccepted
                | Self::ProviderAccepted
                | Self::Running
                | Self::Uncertain
        )
    }

    pub fn in_flight(self) -> bool {
        matches!(
            self,
            Self::Prepared
                | Self::Dispatched
                | Self::ExecutorAccepted
                | Self::ProviderAccepted
                | Self::Running
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub workspace_id: String,
    pub owner_id: String,
    pub run_as_owner_id: String,
    pub controller_id: String,
    pub initiating_device_id: String,
    pub executor_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub lease_id: String,
    pub generation: u64,
    pub workspace_id: String,
    pub owner_id: String,
    pub run_as_owner_id: String,
    pub controller_id: String,
    pub initiating_device_id: String,
    pub executor_id: String,
    pub logical_thread_uid: String,
    #[serde(default)]
    pub logical_captain_uid: Option<String>,
    pub run_id: String,
    #[serde(default)]
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub delivery_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub schema: u32,
    pub lease_id: String,
    pub generation: u64,
    pub workspace_id: String,
    pub owner_id: String,
    pub run_as_owner_id: String,
    pub controller_id: String,
    pub initiating_device_id: String,
    pub executor_id: String,
    #[serde(default)]
    pub logical_captain_uid: Option<String>,
    pub logical_thread_uid: String,
    #[serde(default)]
    pub executor_thread_alias: Option<u64>,
    #[serde(default)]
    pub executor_thread_uid: Option<String>,
    pub run_id: String,
    #[serde(default)]
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub delivery_id: Option<String>,
    #[serde(default)]
    pub request_key: Option<String>,
    #[serde(default)]
    pub queued_behind_current: bool,
    pub phase: Phase,
    pub prepared_at: u64,
    pub saved_at: u64,
    #[serde(default)]
    pub dispatched_at: Option<u64>,
    #[serde(default)]
    pub executor_accepted_at: Option<u64>,
    #[serde(default)]
    pub provider_accepted_at: Option<u64>,
    #[serde(default)]
    pub running_at: Option<u64>,
    #[serde(default)]
    pub terminal_at: Option<u64>,
    #[serde(default)]
    pub terminal_status: Option<String>,
    #[serde(default)]
    pub uncertain_at: Option<u64>,
    #[serde(default)]
    pub uncertain_reason: Option<String>,
    #[serde(default)]
    pub fenced_at: Option<u64>,
    #[serde(default)]
    pub fence_evidence: Option<String>,
    pub last_observed_at: u64,
    #[serde(default)]
    pub lease_expires_at: u64,
    #[serde(default)]
    pub evidence: Vec<String>,
}

impl Lease {
    fn scope(&self) -> Scope {
        Scope {
            workspace_id: self.workspace_id.clone(),
            owner_id: self.owner_id.clone(),
            run_as_owner_id: self.run_as_owner_id.clone(),
            controller_id: self.controller_id.clone(),
            initiating_device_id: self.initiating_device_id.clone(),
            executor_id: self.executor_id.clone(),
        }
    }

    fn same_ownership(&self, scope: &Scope, captain: &Option<String>) -> bool {
        self.workspace_id == scope.workspace_id
            && self.owner_id == scope.owner_id
            && self.run_as_owner_id == scope.run_as_owner_id
            && self.controller_id == scope.controller_id
            && self.logical_captain_uid == *captain
    }

    pub fn binding(&self) -> Binding {
        Binding {
            lease_id: self.lease_id.clone(),
            generation: self.generation,
            workspace_id: self.workspace_id.clone(),
            owner_id: self.owner_id.clone(),
            run_as_owner_id: self.run_as_owner_id.clone(),
            controller_id: self.controller_id.clone(),
            initiating_device_id: self.initiating_device_id.clone(),
            executor_id: self.executor_id.clone(),
            logical_thread_uid: self.logical_thread_uid.clone(),
            logical_captain_uid: self.logical_captain_uid.clone(),
            run_id: self.run_id.clone(),
            attempt_id: self.attempt_id.clone(),
            delivery_id: self.delivery_id.clone(),
        }
    }

    pub fn value(&self) -> Value {
        serde_json::to_value(self).expect("execution lease serialization")
    }

    pub fn control(&self) -> Value {
        json!({
            "leaseId":self.lease_id,
            "generation":self.generation,
            "workspaceId":self.workspace_id,
            "ownerId":self.owner_id,
            "controllerId":self.controller_id,
            "logicalThreadUid":self.logical_thread_uid,
            "runId":self.run_id,
            "attemptId":self.attempt_id
        })
    }
}

pub fn lease_from_assignment(
    assignment: &Value,
    executor_id: &str,
    executor_thread_alias: Option<u64>,
    executor_thread_uid: Option<String>,
    now: u64,
) -> Result<Lease, String> {
    let text = |field: &str| -> Result<String, String> {
        assignment[field]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("Execution assignment is missing {field}."))
    };
    let generation = assignment["generation"]
        .as_u64()
        .filter(|generation| *generation > 0)
        .ok_or("Execution assignment has an invalid generation.")?;
    let prepared_at = assignment["preparedAt"].as_u64().unwrap_or(now);
    let controller_id = text("controllerId")?;
    if assignment["executorId"].as_str() != Some(executor_id) {
        return Err("Execution assignment names a different executor.".into());
    }
    let lease = Lease {
        schema: SCHEMA,
        lease_id: text("leaseId")?,
        generation,
        workspace_id: text("workspaceId")?,
        owner_id: text("ownerId")?,
        run_as_owner_id: text("runAsOwnerId")?,
        controller_id,
        initiating_device_id: text("initiatingDeviceId")?,
        executor_id: executor_id.into(),
        logical_captain_uid: assignment["logicalCaptainUid"].as_str().map(str::to_owned),
        logical_thread_uid: text("logicalThreadUid")?,
        executor_thread_alias,
        executor_thread_uid,
        run_id: text("runId")?,
        attempt_id: assignment["attemptId"].as_str().map(str::to_owned),
        delivery_id: assignment["deliveryId"].as_str().map(str::to_owned),
        request_key: assignment["requestKey"].as_str().map(str::to_owned),
        queued_behind_current: assignment["queuesBehindCurrent"] == true,
        phase: Phase::Dispatched,
        prepared_at,
        saved_at: now,
        dispatched_at: Some(now),
        executor_accepted_at: None,
        provider_accepted_at: None,
        running_at: None,
        terminal_at: None,
        terminal_status: None,
        uncertain_at: None,
        uncertain_reason: None,
        fenced_at: None,
        fence_evidence: None,
        last_observed_at: now,
        lease_expires_at: now + LEASE_MILLIS,
        evidence: vec!["controller assignment received".into()],
    };
    validate_lease(&lease)?;
    Ok(lease)
}

#[derive(Clone, Debug)]
pub struct Prepare {
    pub scope: Scope,
    pub logical_captain_uid: Option<String>,
    pub logical_thread_uid: String,
    pub executor_thread_alias: Option<u64>,
    pub executor_thread_uid: Option<String>,
    pub run_id: String,
    pub attempt_id: Option<String>,
    pub delivery_id: Option<String>,
    pub request_key: Option<String>,
    pub now: u64,
    /// A follow-up may queue behind the current provider turn. A replacement
    /// (retry, resume or executor change) must wait for terminal or fenced
    /// evidence from the previous generation.
    pub queues_behind_current: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    #[serde(default = "schema")]
    pub schema: u32,
    #[serde(default)]
    pub next_generations: BTreeMap<String, u64>,
    #[serde(default)]
    pub leases: Vec<Lease>,
    #[serde(default)]
    pub assignments: BTreeMap<String, String>,
}

fn schema() -> u32 {
    SCHEMA
}

impl Default for Book {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            next_generations: BTreeMap::new(),
            leases: Vec::new(),
            assignments: BTreeMap::new(),
        }
    }
}

impl Book {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err("Execution leases require a newer Shina binary.".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut generations = BTreeMap::<&str, u64>::new();
        for lease in &self.leases {
            validate_lease(lease)?;
            if !ids.insert(lease.lease_id.as_str()) {
                return Err("Duplicate execution lease identity.".into());
            }
            let previous = generations
                .entry(lease.logical_thread_uid.as_str())
                .or_default();
            if lease.generation == 0 || lease.generation <= *previous {
                return Err("Execution generations must increase monotonically.".into());
            }
            *previous = lease.generation;
            if let Some(next) = self.next_generations.get(&lease.logical_thread_uid)
                && *next < lease.generation
            {
                return Err("Execution generation cursor is behind a saved lease.".into());
            }
        }
        Ok(())
    }

    pub fn prepare(&mut self, request: Prepare) -> Result<(Lease, bool), String> {
        validate_prepare(&request)?;
        if let Some(existing) = self.exact_run(&request.logical_thread_uid, &request.run_id) {
            if existing.scope() != request.scope
                || existing.logical_captain_uid != request.logical_captain_uid
                || request
                    .executor_thread_alias
                    .is_some_and(|alias| existing.executor_thread_alias != Some(alias))
                || request
                    .executor_thread_uid
                    .as_ref()
                    .is_some_and(|uid| existing.executor_thread_uid.as_ref() != Some(uid))
                || existing.queued_behind_current != request.queues_behind_current
                || existing.attempt_id != request.attempt_id
                || existing.delivery_id != request.delivery_id
                || existing.request_key != request.request_key
            {
                return Err(
                    "This run identity is already bound to a different execution request.".into(),
                );
            }
            return Ok((existing.clone(), true));
        }
        if let Some(previous) = self.latest(&request.logical_thread_uid)
            && !previous.same_ownership(&request.scope, &request.logical_captain_uid)
        {
            return Err(
                "The logical thread is owned by a different controller or workspace.".into(),
            );
        }
        if let Some(uncertain) = self.leases.iter().find(|lease| {
            lease.logical_thread_uid == request.logical_thread_uid
                && lease.phase == Phase::Uncertain
        }) {
            return Err(format!(
                "Execution {} is uncertain. A queued follow-up cannot replace an unverified writer.",
                uncertain.lease_id,
            ));
        }
        if let Some(previous) = self.latest(&request.logical_thread_uid)
            && previous.phase.blocks_replacement()
            && (!request.queues_behind_current || previous.executor_id != request.scope.executor_id)
        {
            return Err(format!(
                "Execution {} (generation {}) is {}. Observe a terminal result or verified executor fence before starting generation {}.",
                previous.lease_id,
                previous.generation,
                previous.phase.word(),
                previous.generation.saturating_add(1)
            ));
        }
        let generation = self
            .next_generations
            .get(&request.logical_thread_uid)
            .copied()
            .unwrap_or(0)
            .max(
                self.latest(&request.logical_thread_uid)
                    .map_or(0, |lease| lease.generation),
            )
            .checked_add(1)
            .ok_or("Execution generation exhausted.")?;
        let lease = Lease {
            schema: SCHEMA,
            lease_id: uuid::Uuid::new_v4().to_string(),
            generation,
            workspace_id: request.scope.workspace_id,
            owner_id: request.scope.owner_id,
            run_as_owner_id: request.scope.run_as_owner_id,
            controller_id: request.scope.controller_id,
            initiating_device_id: request.scope.initiating_device_id,
            executor_id: request.scope.executor_id,
            logical_captain_uid: request.logical_captain_uid,
            logical_thread_uid: request.logical_thread_uid.clone(),
            executor_thread_alias: request.executor_thread_alias,
            executor_thread_uid: request.executor_thread_uid,
            run_id: request.run_id,
            attempt_id: request.attempt_id,
            delivery_id: request.delivery_id,
            request_key: request.request_key,
            queued_behind_current: request.queues_behind_current,
            phase: Phase::Prepared,
            prepared_at: request.now,
            // The lease is written in the same transaction as the intent and
            // receipt. A commit failure therefore leaves no durable lease.
            saved_at: request.now,
            dispatched_at: None,
            executor_accepted_at: None,
            provider_accepted_at: None,
            running_at: None,
            terminal_at: None,
            terminal_status: None,
            uncertain_at: None,
            uncertain_reason: None,
            fenced_at: None,
            fence_evidence: None,
            last_observed_at: request.now,
            lease_expires_at: 0,
            evidence: vec!["controller prepared intent".into()],
        };
        self.next_generations
            .insert(request.logical_thread_uid, generation);
        self.assignments
            .insert(assignment_key(&lease), lease.lease_id.clone());
        self.leases.push(lease.clone());
        Ok((lease, false))
    }

    /// Install a controller-issued lease on an executor. The executor never
    /// allocates a second controller generation for the same logical thread.
    pub fn accept_assignment(&mut self, mut lease: Lease) -> Result<(Lease, bool), String> {
        validate_lease(&lease)?;
        if let Some(existing) = self.get(&lease.lease_id) {
            if existing.binding() != lease.binding()
                || existing.request_key != lease.request_key
                || existing.queued_behind_current != lease.queued_behind_current
                || existing.executor_thread_alias != lease.executor_thread_alias
                || existing.executor_thread_uid != lease.executor_thread_uid
                || existing.generation != lease.generation
            {
                return Err("Conflicting execution assignment for an existing lease.".into());
            }
            return Ok((existing.clone(), true));
        }
        if let Some(existing) = self.latest(&lease.logical_thread_uid) {
            if !existing.same_ownership(&lease.scope(), &lease.logical_captain_uid) {
                return Err(
                    "This logical thread already has a different authoritative controller.".into(),
                );
            }
            if existing.generation == lease.generation {
                return Err("Conflicting assignment for the same execution generation.".into());
            }
            if existing.generation > lease.generation {
                return Err("Execution generation is stale.".into());
            }
            if self.leases.iter().any(|previous| {
                previous.logical_thread_uid == lease.logical_thread_uid
                    && previous.phase == Phase::Uncertain
            }) {
                return Err(format!(
                    "Execution {} is uncertain. The executor requires observed terminal or verified fence evidence before any newer generation.",
                    existing.lease_id
                ));
            }
            if existing.phase.blocks_replacement()
                && (!lease.queued_behind_current
                    || existing.executor_id != lease.executor_id
                    || existing.executor_thread_alias != lease.executor_thread_alias
                    || existing.executor_thread_uid != lease.executor_thread_uid)
            {
                return Err(format!(
                    "Execution {} (generation {}) is {}. The executor requires observed terminal or verified fence evidence before accepting a replacement.",
                    existing.lease_id,
                    existing.generation,
                    existing.phase.word()
                ));
            }
        }
        lease.phase = Phase::ExecutorAccepted;
        lease.executor_accepted_at = Some(lease.last_observed_at);
        lease
            .evidence
            .push("authenticated paired controller assignment accepted".into());
        self.next_generations
            .insert(lease.logical_thread_uid.clone(), lease.generation);
        self.assignments
            .insert(assignment_key(&lease), lease.lease_id.clone());
        self.leases.push(lease.clone());
        Ok((lease, false))
    }

    pub fn exact_run(&self, thread: &str, run: &str) -> Option<&Lease> {
        self.leases
            .iter()
            .find(|lease| lease.logical_thread_uid == thread && lease.run_id == run)
    }

    pub fn get(&self, id: &str) -> Option<&Lease> {
        self.leases.iter().find(|lease| lease.lease_id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Lease> {
        self.leases.iter_mut().find(|lease| lease.lease_id == id)
    }

    pub fn latest(&self, thread: &str) -> Option<&Lease> {
        self.leases
            .iter()
            .filter(|lease| lease.logical_thread_uid == thread)
            .max_by_key(|lease| lease.generation)
    }

    pub fn active_for_thread(&self, thread: &str) -> Option<&Lease> {
        self.leases
            .iter()
            .filter(|lease| lease.logical_thread_uid == thread && lease.phase.blocks_replacement())
            .max_by_key(|lease| lease.generation)
    }

    pub fn for_executor(&self, executor: &str) -> Vec<Lease> {
        self.leases
            .iter()
            .filter(|lease| lease.executor_id == executor)
            .cloned()
            .collect()
    }

    pub fn transition(
        &mut self,
        lease_id: &str,
        phase: Phase,
        now: u64,
        evidence: impl Into<String>,
    ) -> Result<Lease, String> {
        let evidence = evidence.into();
        let lease = self.get_mut(lease_id).ok_or("Unknown execution lease.")?;
        if matches!(lease.phase, Phase::Terminal | Phase::Fenced) {
            if lease.phase != phase {
                return Err(format!(
                    "Execution lease is already {} and cannot become {}.",
                    lease.phase.word(),
                    phase.word()
                ));
            }
            return Ok(lease.clone());
        }
        if lease.phase == Phase::Uncertain
            && !matches!(phase, Phase::Uncertain | Phase::Terminal | Phase::Fenced)
        {
            return Err("An uncertain execution needs explicit reconciliation.".into());
        }
        if phase != Phase::Uncertain
            && phase != Phase::Terminal
            && phase != Phase::Fenced
            && phase < lease.phase
        {
            return Err(format!(
                "Execution phase cannot move backwards from {} to {}.",
                lease.phase.word(),
                phase.word()
            ));
        }
        lease.phase = phase;
        lease.last_observed_at = now;
        lease.evidence.push(evidence);
        match phase {
            Phase::Dispatched => {
                lease.dispatched_at.get_or_insert(now);
                lease.lease_expires_at = lease.lease_expires_at.max(now + LEASE_MILLIS);
            }
            Phase::ExecutorAccepted => {
                lease.executor_accepted_at.get_or_insert(now);
                lease.lease_expires_at = now + LEASE_MILLIS;
            }
            Phase::ProviderAccepted => {
                lease.provider_accepted_at.get_or_insert(now);
                lease.lease_expires_at = now + LEASE_MILLIS;
            }
            Phase::Running => {
                lease.running_at.get_or_insert(now);
                lease.lease_expires_at = now + LEASE_MILLIS;
            }
            Phase::Terminal => {
                lease.terminal_at.get_or_insert(now);
                lease.lease_expires_at = 0;
            }
            Phase::Uncertain => {
                lease.uncertain_at.get_or_insert(now);
            }
            Phase::Fenced => {
                lease.fenced_at.get_or_insert(now);
                lease.lease_expires_at = 0;
            }
            Phase::Prepared => {}
        }
        Ok(lease.clone())
    }

    pub fn terminal(
        &mut self,
        lease_id: &str,
        status: &str,
        now: u64,
        evidence: impl Into<String>,
    ) -> Result<Lease, String> {
        self.transition(lease_id, Phase::Terminal, now, evidence)?;
        let lease = self.get_mut(lease_id).expect("transitioned lease");
        lease.terminal_status = Some(status.into());
        Ok(lease.clone())
    }

    pub fn uncertain(
        &mut self,
        lease_id: &str,
        now: u64,
        reason: impl Into<String>,
    ) -> Result<Lease, String> {
        let reason = reason.into();
        let lease = self.get_mut(lease_id).ok_or("Unknown execution lease.")?;
        if matches!(
            lease.phase,
            Phase::Terminal | Phase::Fenced | Phase::Uncertain
        ) {
            return Ok(lease.clone());
        }
        lease.phase = Phase::Uncertain;
        lease.uncertain_at.get_or_insert(now);
        lease.uncertain_reason = Some(reason.clone());
        lease.last_observed_at = now;
        lease.evidence.push(reason);
        Ok(lease.clone())
    }

    pub fn fence(
        &mut self,
        lease_id: &str,
        now: u64,
        evidence: impl Into<String>,
    ) -> Result<Lease, String> {
        let evidence = evidence.into();
        self.transition(lease_id, Phase::Fenced, now, evidence.clone())?;
        let lease = self.get_mut(lease_id).expect("fenced lease");
        lease.fence_evidence = Some(evidence);
        Ok(lease.clone())
    }

    pub fn expire(&mut self, now: u64) -> bool {
        let expired: Vec<String> = self
            .leases
            .iter()
            .filter(|lease| {
                lease.phase.in_flight()
                    && lease.lease_expires_at > 0
                    && lease.lease_expires_at <= now
            })
            .map(|lease| lease.lease_id.clone())
            .collect();
        for id in &expired {
            let _ = self.uncertain(
                id,
                now,
                "lease expired without observed terminal or verified fence evidence",
            );
        }
        !expired.is_empty()
    }

    pub fn mark_executor_uncertain(&mut self, executor: &str, now: u64, reason: &str) -> bool {
        let ids: Vec<String> = self
            .leases
            .iter()
            .filter(|lease| lease.executor_id == executor && lease.phase.in_flight())
            .map(|lease| lease.lease_id.clone())
            .collect();
        for id in &ids {
            let _ = self.uncertain(id, now, reason);
        }
        !ids.is_empty()
    }

    pub fn mark_restart_uncertain(&mut self, now: u64) -> bool {
        let ids: Vec<String> = self
            .leases
            .iter()
            .filter(|lease| lease.phase.in_flight())
            .map(|lease| lease.lease_id.clone())
            .collect();
        for id in &ids {
            let _ = self.uncertain(
                id,
                now,
                "controller or executor restarted before terminal or verified fence evidence",
            );
        }
        !ids.is_empty()
    }

    /// Control calls must name exactly the generation they observed.
    pub fn control(
        &self,
        thread: &str,
        lease_id: &str,
        generation: u64,
        run_id: Option<&str>,
    ) -> Result<&Lease, String> {
        let lease = self.get(lease_id).ok_or("Unknown execution lease.")?;
        if lease.logical_thread_uid != thread
            || lease.generation != generation
            || run_id.is_some_and(|run| lease.run_id != run)
        {
            return Err(format!(
                "Execution generation changed: {lease_id}/{generation} is no longer current. The newer execution was not affected."
            ));
        }
        let latest = self
            .latest(thread)
            .ok_or("Unknown logical execution thread.")?;
        if latest.lease_id != lease.lease_id && latest.generation > lease.generation {
            return Err(format!(
                "Execution generation changed: generation {} replaced generation {generation}. The newer execution was not affected.",
                latest.generation
            ));
        }
        Ok(lease)
    }

    pub fn projection(&self) -> Value {
        json!({
            "schema": self.schema,
            "nextGenerations": self.next_generations,
            "leases": self.leases
        })
    }

    pub fn inspect_for_thread(&self, thread: &str) -> Value {
        json!({
            "logicalThreadUid":thread,
            "nextGeneration":self.next_generations.get(thread).copied().unwrap_or(0),
            "active":self.active_for_thread(thread).map(Lease::value),
            "leases":self.leases.iter().filter(|lease| lease.logical_thread_uid == thread)
                .map(Lease::value).collect::<Vec<_>>()
        })
    }
}

fn assignment_key(lease: &Lease) -> String {
    format!(
        "{}:{}:{}:{}",
        lease.controller_id,
        lease.logical_thread_uid,
        lease.run_id,
        lease.attempt_id.as_deref().unwrap_or("")
    )
}

fn validate_prepare(request: &Prepare) -> Result<(), String> {
    for (name, id) in [
        ("workspaceId", request.scope.workspace_id.as_str()),
        ("ownerId", request.scope.owner_id.as_str()),
        ("runAsOwnerId", request.scope.run_as_owner_id.as_str()),
        ("controllerId", request.scope.controller_id.as_str()),
        ("executorId", request.scope.executor_id.as_str()),
        ("logicalThreadUid", request.logical_thread_uid.as_str()),
    ] {
        uuid::Uuid::parse_str(id).map_err(|_| format!("Invalid execution {name}."))?;
    }
    if request.run_id.is_empty() || request.run_id.len() > 256 {
        return Err("Invalid execution runId.".into());
    }
    if request.scope.initiating_device_id.is_empty()
        || request.scope.initiating_device_id.len() > 256
    {
        return Err("Invalid execution initiatingDeviceId.".into());
    }
    if request.scope.owner_id == request.scope.controller_id {
        return Err("Owner and controller identities must stay distinct.".into());
    }
    if request.scope.run_as_owner_id != request.scope.owner_id {
        return Err("Run-as owner must be the workspace owner in this version.".into());
    }
    if let Some(captain) = &request.logical_captain_uid {
        uuid::Uuid::parse_str(captain).map_err(|_| "Invalid logical Captain identity.")?;
    }
    if let Some(thread) = &request.executor_thread_uid {
        uuid::Uuid::parse_str(thread).map_err(|_| "Invalid executor thread identity.")?;
    }
    if let Some(attempt) = &request.attempt_id
        && (attempt.is_empty() || attempt.len() > 256)
    {
        return Err("Invalid execution attempt identity.".into());
    }
    Ok(())
}

fn validate_lease(lease: &Lease) -> Result<(), String> {
    if lease.schema != SCHEMA {
        return Err("Execution lease schema is unsupported.".into());
    }
    for (name, id) in [
        ("leaseId", lease.lease_id.as_str()),
        ("workspaceId", lease.workspace_id.as_str()),
        ("ownerId", lease.owner_id.as_str()),
        ("runAsOwnerId", lease.run_as_owner_id.as_str()),
        ("controllerId", lease.controller_id.as_str()),
        ("executorId", lease.executor_id.as_str()),
        ("logicalThreadUid", lease.logical_thread_uid.as_str()),
    ] {
        uuid::Uuid::parse_str(id).map_err(|_| format!("Invalid saved execution {name}."))?;
    }
    if lease.run_id.is_empty() || lease.run_id.len() > 256 {
        return Err("Invalid saved execution runId.".into());
    }
    if lease.initiating_device_id.is_empty() || lease.initiating_device_id.len() > 256 {
        return Err("Invalid saved execution initiatingDeviceId.".into());
    }
    if lease.generation == 0 {
        return Err("Saved execution generation must be positive.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(executor: &str) -> Scope {
        Scope {
            workspace_id: "11111111-1111-4111-8111-111111111111".into(),
            owner_id: "22222222-2222-4222-8222-222222222222".into(),
            run_as_owner_id: "22222222-2222-4222-8222-222222222222".into(),
            controller_id: "33333333-3333-4333-8333-333333333333".into(),
            initiating_device_id: "44444444-4444-4444-8444-444444444444".into(),
            executor_id: executor.into(),
        }
    }

    fn prepare(run: &str, now: u64) -> Prepare {
        Prepare {
            scope: scope("55555555-5555-4555-8555-555555555555"),
            logical_captain_uid: Some("66666666-6666-4666-8666-666666666666".into()),
            logical_thread_uid: "77777777-7777-4777-8777-777777777777".into(),
            executor_thread_alias: Some(9),
            executor_thread_uid: None,
            run_id: run.into(),
            attempt_id: Some("attempt-1".into()),
            delivery_id: Some("delivery-1".into()),
            request_key: Some("request-1".into()),
            now,
            queues_behind_current: false,
        }
    }

    #[test]
    fn a_queued_followup_cannot_bypass_an_uncertain_execution() {
        for uncertainty_on_older_generation in [false, true] {
            let mut book = Book::default();
            let (first, _) = book.prepare(prepare("first-run", 10)).unwrap();
            book.transition(&first.lease_id, Phase::Dispatched, 10, "sent")
                .unwrap();
            if uncertainty_on_older_generation {
                let mut second = prepare("queued-run", 11);
                second.queues_behind_current = true;
                book.prepare(second).unwrap();
            }
            book.uncertain(&first.lease_id, 12, "executor disconnected")
                .unwrap();
            let before = serde_json::to_value(&book).unwrap();
            let mut next = prepare("replacement-run", 13);
            next.queues_behind_current = true;
            assert!(
                book.prepare(next).is_err(),
                "queueing must not authorize a generation while any prior writer is uncertain"
            );
            assert_eq!(serde_json::to_value(&book).unwrap(), before);
        }
    }

    #[test]
    fn keyed_prepare_replay_requires_the_original_controller_and_executor() {
        for field in ["controller", "executor", "workspace", "captain"] {
            let mut book = Book::default();
            book.prepare(prepare("same-run", 10)).unwrap();
            let mut replay = prepare("same-run", 11);
            let other = "99999999-9999-4999-8999-999999999999".to_string();
            match field {
                "controller" => replay.scope.controller_id = other,
                "executor" => replay.scope.executor_id = other,
                "workspace" => replay.scope.workspace_id = other,
                "captain" => replay.logical_captain_uid = Some(other),
                _ => unreachable!(),
            }
            let before = serde_json::to_value(&book).unwrap();
            assert!(book.prepare(replay).is_err(), "replay changed {field}");
            assert_eq!(serde_json::to_value(&book).unwrap(), before);
        }
    }

    #[test]
    fn assignment_replay_requires_all_immutable_assignment_bindings() {
        for field in ["executor", "owner", "attempt", "delivery"] {
            let mut controller = Book::default();
            let (lease, _) = controller.prepare(prepare("same-run", 10)).unwrap();
            let mut executor = Book::default();
            executor.accept_assignment(lease.clone()).unwrap();
            let mut replay = lease;
            let other = "99999999-9999-4999-8999-999999999999".to_string();
            match field {
                "executor" => replay.executor_id = other,
                "owner" => {
                    replay.owner_id = other.clone();
                    replay.run_as_owner_id = other;
                }
                "attempt" => replay.attempt_id = Some("another-attempt".into()),
                "delivery" => replay.delivery_id = Some("another-delivery".into()),
                _ => unreachable!(),
            }
            let before = serde_json::to_value(&executor).unwrap();
            assert!(
                executor.accept_assignment(replay).is_err(),
                "replay changed {field}"
            );
            assert_eq!(serde_json::to_value(&executor).unwrap(), before);
        }
    }

    #[test]
    fn queueing_cannot_move_an_unsettled_writer_to_another_executor() {
        let mut book = Book::default();
        let (first, _) = book.prepare(prepare("first-run", 10)).unwrap();
        book.transition(&first.lease_id, Phase::Dispatched, 10, "sent")
            .unwrap();
        let mut next = prepare("another-executor", 11);
        next.scope.executor_id = "99999999-9999-4999-8999-999999999999".into();
        next.queues_behind_current = true;
        assert!(book.prepare(next).is_err());
        book.terminal(&first.lease_id, "completed", 12, "provider settled")
            .unwrap();
        let mut next = prepare("another-executor", 13);
        next.scope.executor_id = "99999999-9999-4999-8999-999999999999".into();
        assert!(
            book.prepare(next).is_ok(),
            "settled work may deliberately change executor"
        );
    }

    #[test]
    fn executor_refuses_queued_assignments_when_an_older_writer_is_uncertain() {
        let mut controller = Book::default();
        let (first, _) = controller.prepare(prepare("first-run", 10)).unwrap();
        let mut second = prepare("second-run", 11);
        second.queues_behind_current = true;
        let (second, _) = controller.prepare(second).unwrap();
        let mut third = prepare("third-run", 12);
        third.queues_behind_current = true;
        let (third, _) = controller.prepare(third).unwrap();
        let mut executor = Book::default();
        executor.accept_assignment(first.clone()).unwrap();
        executor.accept_assignment(second).unwrap();
        executor
            .uncertain(&first.lease_id, 13, "provider link lost")
            .unwrap();
        let before = serde_json::to_value(&executor).unwrap();
        assert!(executor.accept_assignment(third).is_err());
        assert_eq!(serde_json::to_value(&executor).unwrap(), before);
    }

    #[test]
    fn generations_are_monotonic_and_keyed_prepare_replays_exactly() {
        let mut book = Book::default();
        let first = book
            .prepare(prepare("88888888-8888-4888-8888-888888888888", 10))
            .unwrap();
        assert_eq!(first.0.generation, 1);
        let replay = book
            .prepare(prepare("88888888-8888-4888-8888-888888888888", 11))
            .unwrap();
        assert!(replay.1);
        assert_eq!(replay.0.lease_id, first.0.lease_id);
        assert_eq!(book.leases.len(), 1);
        book.terminal(&first.0.lease_id, "completed", 12, "provider settled")
            .unwrap();
        let second = book
            .prepare(prepare("99999999-9999-4999-8999-999999999999", 13))
            .unwrap();
        assert_eq!(second.0.generation, 2);
    }

    #[test]
    fn uncertain_and_expired_executions_block_replacement_until_fencing() {
        let mut book = Book::default();
        let first = book
            .prepare(prepare("88888888-8888-4888-8888-888888888888", 10))
            .unwrap();
        book.transition(&first.0.lease_id, Phase::Dispatched, 10, "committed intent")
            .unwrap();
        assert_eq!(book.leases[0].lease_expires_at, 10 + LEASE_MILLIS);
        book.expire(10 + LEASE_MILLIS);
        assert_eq!(
            book.latest("77777777-7777-4777-8777-777777777777")
                .unwrap()
                .phase,
            Phase::Uncertain
        );
        let blocked = book
            .prepare(prepare("99999999-9999-4999-8999-999999999999", 20))
            .unwrap_err();
        assert!(blocked.contains("uncertain"), "{blocked}");
        book.fence(
            &first.0.lease_id,
            21,
            "executor verified no provider process",
        )
        .unwrap();
        let second = book
            .prepare(prepare("99999999-9999-4999-8999-999999999999", 22))
            .unwrap();
        assert_eq!(second.0.generation, 2);
    }

    #[test]
    fn stale_control_names_never_match_a_newer_generation() {
        let mut book = Book::default();
        let first = book
            .prepare(prepare("88888888-8888-4888-8888-888888888888", 10))
            .unwrap();
        book.terminal(&first.0.lease_id, "completed", 11, "provider settled")
            .unwrap();
        let second = book
            .prepare(prepare("99999999-9999-4999-8999-999999999999", 12))
            .unwrap();
        let thread = "77777777-7777-4777-8777-777777777777";
        assert!(
            book.control(
                thread,
                &first.0.lease_id,
                first.0.generation,
                Some(&first.0.run_id)
            )
            .unwrap_err()
            .contains("replaced generation")
        );
        assert_eq!(
            book.control(
                thread,
                &second.0.lease_id,
                second.0.generation,
                Some(&second.0.run_id)
            )
            .unwrap()
            .lease_id,
            second.0.lease_id
        );
    }

    #[test]
    fn a_queued_follow_up_may_be_prepared_without_replacing_the_live_writer() {
        let mut book = Book::default();
        let first = book
            .prepare(prepare("88888888-8888-4888-8888-888888888888", 10))
            .unwrap();
        book.transition(&first.0.lease_id, Phase::Dispatched, 10, "sent")
            .unwrap();
        let mut next = prepare("99999999-9999-4999-8999-999999999999", 11);
        next.queues_behind_current = true;
        let second = book.prepare(next).unwrap();
        assert_eq!(second.0.generation, 2);
        assert_eq!(
            book.active_for_thread("77777777-7777-4777-8777-777777777777")
                .unwrap()
                .lease_id,
            second.0.lease_id,
            "the newest generation fences old controls as soon as it is accepted"
        );
    }

    #[test]
    fn restart_after_dispatch_is_uncertain_and_cannot_be_replaced_automatically() {
        let mut book = Book::default();
        let lease = book
            .prepare(prepare("88888888-8888-4888-8888-888888888888", 10))
            .unwrap();
        book.transition(&lease.0.lease_id, Phase::Dispatched, 10, "sent")
            .unwrap();
        assert!(book.mark_restart_uncertain(11));
        let saved = book.latest("77777777-7777-4777-8777-777777777777").unwrap();
        assert_eq!(saved.phase, Phase::Uncertain);
        assert!(
            saved
                .uncertain_reason
                .as_deref()
                .unwrap()
                .contains("restarted")
        );
        let error = book
            .prepare(prepare("99999999-9999-4999-8999-999999999999", 12))
            .unwrap_err();
        assert!(error.contains("uncertain"), "{error}");
        assert_eq!(book.leases.len(), 1, "no replacement was allocated");
    }
}
