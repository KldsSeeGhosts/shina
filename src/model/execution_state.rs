use super::*;

impl State {
    pub(crate) fn execution_thread_quiescent(&self, thread_id: usize) -> bool {
        self.threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .is_some_and(Thread::execution_quiescent)
            && !self.peer_task_intents.iter().any(|intent| {
                intent.thread_id == thread_id
                    && intent.queue_state != "settled"
                    && intent.provider_state != "rejected"
            })
    }

    pub(crate) fn execution_baseline(&self) -> ExecutionBaseline {
        ExecutionBaseline {
            runs: self
                .threads
                .iter()
                .map(|thread| (thread.id, thread.run_id.clone()))
                .collect(),
            tasks: self
                .tasks
                .iter()
                .filter_map(|task| {
                    task.remote.as_ref().map(|remote| {
                        (
                            task.id.clone(),
                            (
                                remote.launch_command.command_id,
                                remote
                                    .send_command
                                    .as_ref()
                                    .map(|command| command.command_id),
                                remote.thread_id,
                            ),
                        )
                    })
                })
                .collect(),
        }
    }

    /// Install controller leases after the reducer accepted a command but
    /// before the runtime commits or dispatches any provider/peer effect.
    pub(crate) fn prepare_execution_after_command(
        &mut self,
        baseline: &ExecutionBaseline,
        device: &str,
    ) -> Result<(), String> {
        let remote_children: std::collections::BTreeSet<usize> = self
            .tasks
            .iter()
            .filter(|task| task.remote.is_some())
            .map(|task| task.child_thread_id)
            .collect();
        let changed_threads: Vec<(usize, bool)> = self
            .threads
            .iter()
            .filter(|thread| {
                thread.run_id.is_some()
                    && baseline.runs.get(&thread.id) != Some(&thread.run_id)
                    && !remote_children.contains(&thread.id)
                    && thread
                        .execution
                        .as_ref()
                        .is_none_or(|binding| Some(&binding.run_id) != thread.run_id.as_ref())
            })
            .map(|thread| {
                let uid = self
                    .snapshot_identity()
                    .thread_uid(thread.id)
                    .map(str::to_owned);
                let queues = thread.status.busy()
                    && uid
                        .as_deref()
                        .and_then(|uid| self.execution.active_for_thread(uid))
                        .is_some();
                (thread.id, queues)
            })
            .collect();
        for (id, queues) in changed_threads {
            self.allocate_local_execution(id, device, queues)?;
        }

        let task_ids: Vec<String> = self.tasks.iter().map(|task| task.id.clone()).collect();
        for task_id in task_ids {
            let Some(index) = self.tasks.iter().position(|task| task.id == task_id) else {
                continue;
            };
            let Some(remote) = self.tasks[index].remote.as_ref() else {
                continue;
            };
            let prior = baseline.tasks.get(&task_id);
            let launch_is_new =
                prior.is_none_or(|(launch, _, _)| *launch != remote.launch_command.command_id);
            let send_is_new = remote.send_command.as_ref().is_some_and(|command| {
                prior.and_then(|(_, send, _)| *send) != Some(command.command_id)
            });
            if launch_is_new && remote.execution.is_none() {
                self.allocate_remote_execution(index, device, None)?;
            } else if send_is_new {
                let command = remote.send_command.as_ref().expect("new send command");
                let delivery = if command.params.get("requestId").is_some() {
                    "steer"
                } else {
                    command.params["delivery"].as_str().unwrap_or("follow_up")
                }
                .to_owned();
                self.allocate_remote_execution(index, device, Some(&delivery))?;
            }
        }
        Ok(())
    }

    pub(super) fn execution_scope(&self, device: &str, executor: &str) -> crate::execution::Scope {
        crate::execution::Scope {
            workspace_id: self.identity.workspace_id.clone(),
            owner_id: self.identity.owner_id.clone(),
            run_as_owner_id: self.identity.owner_id.clone(),
            controller_id: self.identity.controller_id.clone(),
            initiating_device_id: device.into(),
            executor_id: executor.into(),
        }
    }

    pub(crate) fn execution_initiator_for_thread(&self, thread: usize) -> Option<String> {
        let uid = self.snapshot_identity().thread_uid(thread)?.to_owned();
        self.execution
            .latest(&uid)
            .map(|lease| lease.initiating_device_id.clone())
    }

    pub(super) fn allocate_local_execution(
        &mut self,
        thread_id: usize,
        device: &str,
        queues_behind_current: bool,
    ) -> Result<crate::execution::Lease, String> {
        let identity = self.snapshot_identity();
        let thread = self
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .ok_or("Unknown execution thread.")?;
        let run_id = thread
            .run_id
            .clone()
            .ok_or("A managed execution needs a run identity.")?;
        let logical_thread_uid = identity
            .thread_uid(thread_id)
            .ok_or("The thread has no logical identity.")?
            .to_owned();
        let executor = self.identity.executor_id.clone();
        let (lease, created) = self.execution.prepare(crate::execution::Prepare {
            scope: self.execution_scope(device, &executor),
            logical_captain_uid: thread
                .captain
                .and_then(|captain| identity.captain_uid(captain).map(str::to_owned)),
            logical_thread_uid,
            executor_thread_alias: Some(thread_id as u64),
            executor_thread_uid: identity.thread_uid(thread_id).map(str::to_owned),
            run_id,
            attempt_id: None,
            delivery_id: None,
            request_key: None,
            now: lifecycle::now(),
            queues_behind_current,
        })?;
        let thread = self
            .threads
            .iter_mut()
            .find(|thread| thread.id == thread_id)
            .expect("execution thread");
        if thread.execution.as_ref().map(|binding| &binding.lease_id) != Some(&lease.lease_id) {
            thread.execution = Some(lease.binding());
        }
        if created {
            let _ = self.execution.transition(
                &lease.lease_id,
                crate::execution::Phase::Dispatched,
                lifecycle::now(),
                "controller committed local run intent",
            )?;
        }
        Ok(self
            .execution
            .get(&lease.lease_id)
            .cloned()
            .expect("prepared execution"))
    }

    pub(super) fn allocate_remote_execution(
        &mut self,
        task_index: usize,
        device: &str,
        delivery: Option<&str>,
    ) -> Result<crate::execution::Lease, String> {
        let task = self.tasks.get(task_index).ok_or("Unknown remote task.")?;
        let initiating_device = if task.captain_turn() {
            device.to_owned()
        } else {
            self.execution_initiator_for_thread(task.parent_thread_id)
                .unwrap_or_else(|| device.to_owned())
        };
        let remote = task.remote.as_ref().ok_or("Task is not remote.")?;
        let already_bound = if delivery.is_some() {
            remote
                .send_command
                .as_ref()
                .is_some_and(|command| command.params["execution"].is_object())
        } else {
            remote.launch_command.params["execution"].is_object()
        };
        if already_bound && let Some(lease) = &remote.execution {
            return Ok(lease.clone());
        }
        let identity = self.snapshot_identity();
        let child = self
            .threads
            .iter()
            .find(|thread| thread.id == task.child_thread_id)
            .ok_or("Remote task has no logical controller thread.")?;
        let logical_thread_uid = identity
            .thread_uid(child.id)
            .ok_or("Remote task thread has no logical identity.")?
            .to_owned();
        let steer = delivery == Some("steer");
        let current = if steer {
            remote.execution.clone()
        } else {
            None
        };
        let run_id = current
            .as_ref()
            .map(|lease| lease.run_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let attempt_id = if steer {
            current.as_ref().and_then(|lease| lease.attempt_id.clone())
        } else {
            Some(task.attempt_id.clone())
        };
        let request_key = remote.client_request_id.clone();
        let (lease, created) = if let Some(mut current) = current {
            current.last_observed_at = lifecycle::now();
            let _ = self.execution.prepare(crate::execution::Prepare {
                scope: self.execution_scope(&initiating_device, &remote.server_id),
                logical_captain_uid: child
                    .captain
                    .and_then(|captain| identity.captain_uid(captain).map(str::to_owned)),
                logical_thread_uid: logical_thread_uid.clone(),
                executor_thread_alias: remote.thread_id.map(|id| id as u64),
                executor_thread_uid: None,
                run_id: run_id.clone(),
                attempt_id: attempt_id.clone(),
                delivery_id: Some(task.delivery_id.clone()),
                request_key,
                now: lifecycle::now(),
                queues_behind_current: current.queued_behind_current,
            })?;
            (current, false)
        } else {
            self.execution.prepare(crate::execution::Prepare {
                scope: self.execution_scope(&initiating_device, &remote.server_id),
                logical_captain_uid: child
                    .captain
                    .and_then(|captain| identity.captain_uid(captain).map(str::to_owned)),
                logical_thread_uid: logical_thread_uid.clone(),
                executor_thread_alias: remote.thread_id.map(|id| id as u64),
                executor_thread_uid: None,
                run_id,
                attempt_id,
                delivery_id: Some(task.delivery_id.clone()),
                request_key,
                now: lifecycle::now(),
                queues_behind_current: delivery.is_some(),
            })?
        };
        if created {
            let _ = self.execution.transition(
                &lease.lease_id,
                crate::execution::Phase::Dispatched,
                lifecycle::now(),
                "controller committed peer assignment for dispatch",
            )?;
        }
        let lease = self
            .execution
            .get(&lease.lease_id)
            .cloned()
            .expect("remote lease");
        let assignment = lease.assignment();
        let remote = self.tasks[task_index].remote.as_mut().expect("remote task");
        remote.execution = Some(lease.clone());
        if remote.thread_id.is_none() {
            remote.launch_command.params["execution"] = assignment;
        } else if let Some(command) = &mut remote.send_command {
            command.params["execution"] = assignment;
            remote.execution_inputs.retain(|id, _| {
                self.execution.get(id).is_none_or(|lease| {
                    !matches!(
                        lease.phase,
                        crate::execution::Phase::Terminal | crate::execution::Phase::Fenced
                    )
                })
            });
            remote
                .execution_inputs
                .insert(lease.lease_id.clone(), command.clone());
        }
        if self.tasks[task_index].captain_turn()
            && let Some(home) = self
                .threads
                .iter_mut()
                .find(|thread| thread.id == self.tasks[task_index].child_thread_id)
        {
            home.run_id = Some(lease.run_id.clone());
        }
        Ok(lease)
    }

    pub(super) fn prepared_local_attempt(
        &self,
        task: &Task,
        thread: &Thread,
    ) -> Option<&crate::execution::Lease> {
        let binding = thread.next_execution.as_ref()?;
        let lease = self.execution.get(&binding.lease_id)?;
        (lease.phase == crate::execution::Phase::Prepared
            && lease.executor_id == self.identity.executor_id
            && lease.executor_thread_alias == Some(thread.id as u64)
            && lease.attempt_id.as_deref() == Some(task.attempt_id.as_str())
            && lease.delivery_id.as_deref() == Some(task.delivery_id.as_str())
            && lease.binding() == *binding)
            .then_some(lease)
    }

    pub(super) fn allocate_local_task_execution(
        &mut self,
        task_index: usize,
        initiating_device: &str,
    ) -> Result<(), String> {
        let task = self.tasks.get(task_index).ok_or("Unknown local task.")?;
        let thread_id = task.child_thread_id;
        let identity = self.snapshot_identity();
        let thread = self
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .ok_or("Local task has no child thread.")?;
        if self.prepared_local_attempt(task, thread).is_some() {
            return Ok(());
        }
        let logical_thread_uid = identity
            .thread_uid(thread_id)
            .ok_or("Local task thread has no logical identity.")?
            .to_owned();
        let executor = self.identity.executor_id.clone();
        let (lease, _) = self.execution.prepare(crate::execution::Prepare {
            scope: self.execution_scope(initiating_device, &executor),
            logical_captain_uid: thread
                .captain
                .and_then(|captain| identity.captain_uid(captain).map(str::to_owned)),
            logical_thread_uid,
            executor_thread_alias: Some(thread_id as u64),
            executor_thread_uid: identity.thread_uid(thread_id).map(str::to_owned),
            run_id: uuid::Uuid::new_v4().to_string(),
            attempt_id: Some(task.attempt_id.clone()),
            delivery_id: Some(task.delivery_id.clone()),
            request_key: task.request_key.as_ref().map(|key| key.key.clone()),
            now: lifecycle::now(),
            queues_behind_current: false,
        })?;
        if let Some(thread) = self
            .threads
            .iter_mut()
            .find(|thread| thread.id == thread_id)
        {
            thread.next_execution = Some(lease.binding());
        }
        Ok(())
    }

    /// Move persisted provider observations into the lease book. A queued peer
    /// acknowledgement is handled separately and never calls this with a
    /// provider-accepted phase.
    pub(crate) fn refresh_execution(&mut self, now: u64) -> bool {
        let mut changed = false;
        for thread in &mut self.threads {
            if let Some(binding) = &thread.execution
                && let Some(lease) = self.execution.get_mut(&binding.lease_id)
                && !lease.restart_continuation_blocked
                && (thread.stop_control.target() == Some(lease.run_id.as_str())
                    || thread.request.is_some())
            {
                lease.restart_continuation_blocked = true;
                changed = true;
            }
            for binding in std::mem::take(&mut thread.execution_acceptances) {
                if self.execution.get(&binding.lease_id).is_some_and(|lease| {
                    lease.binding() == binding
                        && lease.phase < crate::execution::Phase::ProviderAccepted
                }) {
                    changed |= self
                        .execution
                        .transition(
                            &binding.lease_id,
                            crate::execution::Phase::ProviderAccepted,
                            now,
                            "provider acknowledged this exact queued input, not writer activation",
                        )
                        .is_ok();
                }
            }
            for (binding, status) in std::mem::take(&mut thread.execution_settlements) {
                if self.execution.get(&binding.lease_id).is_some_and(|lease| {
                    lease.binding() == binding && lease.phase != crate::execution::Phase::Terminal
                }) {
                    changed |= self
                        .execution
                        .terminal(
                            &binding.lease_id,
                            &status,
                            now,
                            "executor observed provider terminal event for this exact turn",
                        )
                        .is_ok();
                }
            }
        }
        let dangling: Vec<_> = self
            .threads
            .iter()
            .filter(|thread| {
                matches!(thread.status, Status::Completed | Status::Failed)
                    && !thread.queued()
                    && thread.pending_prompt.is_none()
                    && thread.queued_prompt.is_none()
            })
            .flat_map(|thread| {
                thread
                    .next_execution
                    .iter()
                    .chain(&thread.execution)
                    .filter_map(|binding| {
                        let lease = self.execution.get(&binding.lease_id)?;
                        (lease.phase == crate::execution::Phase::Prepared
                            && lease.executor_id == self.identity.executor_id
                            && lease.controller_id == self.identity.controller_id)
                            .then(|| {
                                (
                                    thread.id,
                                    binding.lease_id.clone(),
                                    status_word(thread.status),
                                )
                            })
                    })
            })
            .collect();
        for (thread_id, lease_id, status) in dangling {
            let _ = self.execution.terminal(
                &lease_id,
                status,
                now,
                "thread reached a terminal state before the prepared generation dispatched",
            );
            if let Some(thread) = self
                .threads
                .iter_mut()
                .find(|thread| thread.id == thread_id)
                && thread
                    .next_execution
                    .as_ref()
                    .is_some_and(|binding| binding.lease_id == lease_id)
            {
                thread.next_execution = None;
            }
            changed = true;
        }
        let observations: Vec<_> = self
            .threads
            .iter()
            .filter_map(|thread| {
                thread.execution.as_ref().map(|binding| {
                    (
                        thread.id,
                        binding.clone(),
                        thread.provider_accepted_run.clone(),
                        thread.provider_running_run.clone(),
                        thread.provider_terminal_run.clone(),
                        thread.provider_terminal_status.clone(),
                    )
                })
            })
            .collect();
        for (thread_id, binding, accepted, running, terminal, terminal_status) in observations {
            let Some(lease) = self.execution.get(&binding.lease_id).cloned() else {
                continue;
            };
            let target = if terminal.as_deref() == Some(lease.run_id.as_str()) {
                Some((
                    crate::execution::Phase::Terminal,
                    terminal_status.unwrap_or_else(|| "terminal".into()),
                ))
            } else if running.as_deref() == Some(lease.run_id.as_str()) {
                Some((crate::execution::Phase::Running, String::new()))
            } else if accepted.as_deref() == Some(lease.run_id.as_str()) {
                Some((crate::execution::Phase::ProviderAccepted, String::new()))
            } else {
                None
            };
            if let Some((phase, status)) = target {
                let should_apply = match phase {
                    crate::execution::Phase::Terminal => {
                        lease.phase != crate::execution::Phase::Terminal
                    }
                    crate::execution::Phase::Uncertain => !matches!(
                        lease.phase,
                        crate::execution::Phase::Terminal
                            | crate::execution::Phase::Fenced
                            | crate::execution::Phase::Uncertain
                    ),
                    _ => lease.phase < phase,
                };
                if should_apply {
                    let before = self.execution.get(&lease.lease_id).cloned();
                    let evidence = match phase {
                        crate::execution::Phase::Running => {
                            "executor observed provider turn running"
                        }
                        crate::execution::Phase::ProviderAccepted => {
                            "executor observed provider accepted the prompt"
                        }
                        crate::execution::Phase::Terminal => {
                            "executor observed provider terminal event"
                        }
                        crate::execution::Phase::Uncertain => {
                            "executor state settled without matching provider evidence"
                        }
                        _ => "execution observation",
                    };
                    let _ = if phase == crate::execution::Phase::Terminal {
                        self.execution
                            .terminal(&lease.lease_id, &status, now, evidence)
                    } else if phase == crate::execution::Phase::Uncertain {
                        self.execution.uncertain(&lease.lease_id, now, status)
                    } else {
                        self.execution
                            .transition(&lease.lease_id, phase, now, evidence)
                    };
                    changed |= before.as_ref().map(|lease| &lease.phase)
                        != self
                            .execution
                            .get(&lease.lease_id)
                            .map(|lease| &lease.phase);
                }
            }
            let executor_thread_uid = self
                .snapshot_identity()
                .thread_uid(thread_id)
                .map(str::to_owned);
            if let Some(lease) = self.execution.get_mut(&binding.lease_id) {
                let alias = Some(thread_id as u64);
                if lease.executor_thread_alias != alias {
                    lease.executor_thread_alias = alias;
                    changed = true;
                }
                if lease.executor_thread_uid.is_none()
                    && let Some(uid) = executor_thread_uid
                {
                    lease.executor_thread_uid = Some(uid);
                    changed = true;
                }
            }
        }
        let offline: Vec<String> = self
            .peers
            .iter()
            .filter(|peer| {
                peer.last_error
                    .as_deref()
                    .is_some_and(|error| error == "Target offline.")
            })
            .map(|peer| peer.server_id.clone())
            .collect();
        for executor in offline {
            changed |= self.execution.mark_executor_uncertain(
                &executor,
                now,
                "controller lost the executor link before terminal or verified fence evidence",
            );
        }
        changed |= self.execution.expire(now);
        changed
    }

    pub(crate) fn mark_execution_restart_uncertain(&mut self) -> bool {
        self.execution.mark_restart_uncertain(lifecycle::now())
    }

    pub(crate) fn fence_exited_provider_jobs(&mut self) -> bool {
        self.execution
            .fence_exited_provider_jobs(&self.identity.executor_id, lifecycle::now())
    }

    pub(crate) fn record_shutdown_interruptions(&mut self) {
        for thread in &self.threads {
            if let Some(binding) = &thread.execution
                && let Some(lease) = self.execution.get_mut(&binding.lease_id)
                && lease.phase.in_flight()
                && lease.provider_job.is_some()
                && !lease.restart_continuation_blocked
                && thread.request.is_none()
                && thread.stop_control.target() != Some(lease.run_id.as_str())
            {
                lease.shutdown_interrupted = true;
            }
        }
    }

    pub(crate) fn settle_pending_prompt_effects(
        &mut self,
        undispatched: &std::collections::HashSet<String>,
    ) {
        let leases: Vec<_> = self
            .execution
            .leases
            .iter()
            .filter(|lease| {
                lease.executor_id == self.identity.executor_id
                    && lease.controller_id == self.identity.controller_id
                    && lease.provider_accepted_at.is_none()
                    && lease.running_at.is_none()
                    && lease.phase.blocks_replacement()
                    && undispatched.contains(&lease.run_id)
            })
            .map(|lease| lease.lease_id.clone())
            .collect();
        for id in leases {
            let _ = self.execution.terminal(
                &id,
                "not_dispatched",
                lifecycle::now(),
                "all committed prompt effects for this run were still pending at restart",
            );
        }
    }

    pub(crate) fn settle_prepared_admission_failure(&mut self, child: usize) {
        let Some(binding) = self.threads[child].next_execution.as_ref() else {
            return;
        };
        let lease_id = &binding.lease_id;
        if self.execution.get(lease_id).is_some_and(|lease| {
            lease.phase == crate::execution::Phase::Prepared && lease.binding() == *binding
        }) {
            let _ = self.execution.terminal(
                lease_id,
                "not_dispatched",
                lifecycle::now(),
                "admission commit failed before queuing the provider input",
            );
            self.threads[child].next_execution = None;
        }
    }

    pub(crate) fn mark_execution_prompts_dispatched(&mut self, effects: &[crate::outbox::Effect]) {
        let now = lifecycle::now();
        for effect in effects {
            let Some((run, command)) = effect.prompt_run() else {
                continue;
            };
            if let Some(text) = effect.prompt_text() {
                self.record_task_result_dispatch(command, text);
            }
            let Some(lease) = self
                .execution
                .leases
                .iter()
                .find(|lease| lease.run_id == run && lease.phase.in_flight())
                .cloned()
            else {
                continue;
            };
            if lease.phase == crate::execution::Phase::Prepared {
                let _ = self.execution.transition(
                    &lease.lease_id,
                    crate::execution::Phase::Dispatched,
                    now,
                    format!("prompt {command} committed for provider dispatch"),
                );
            }
            if lease.executor_id == self.identity.executor_id
                && lease.controller_id == self.identity.controller_id
                && let Some(stored) = self.execution.get_mut(&lease.lease_id)
            {
                if lease.executor_accepted_at.is_none() {
                    stored.provider_job = effect.provider_job.clone();
                } else if lease.provider_job != effect.provider_job {
                    // A later write to a different or untracked process
                    // invalidates this run's single-job evidence.
                    stored.provider_job = None;
                }
            }
            let _ = self.execution.transition(
                &lease.lease_id,
                crate::execution::Phase::ExecutorAccepted,
                now,
                format!("executor accepted provider command {command}"),
            );
        }
    }

    /// Refuse a replacement when durable evidence cannot distinguish a stopped
    /// old provider from one still running. The exact-generation stop/follow-up
    /// checks in peer commands handle non-replacement controls.
    pub(crate) fn require_execution_recovery(&self, thread_id: usize) -> Result<(), String> {
        let Some(uid) = self
            .snapshot_identity()
            .thread_uid(thread_id)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        if let Some(lease) = self.execution.active_for_thread(&uid) {
            return Err(format!(
                "Execution {} (generation {}) is {}. Observe its terminal result or reconcile an executor-verified fence before starting a replacement.",
                lease.lease_id,
                lease.generation,
                lease.phase.word()
            ));
        }
        Ok(())
    }

    pub(crate) fn require_execution_certain(&self, thread_id: usize) -> Result<(), String> {
        let Some(uid) = self
            .snapshot_identity()
            .thread_uid(thread_id)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        if let Some(lease) = self.execution.active_for_thread(&uid)
            && lease.phase == crate::execution::Phase::Uncertain
        {
            return Err(format!(
                "Execution {} is uncertain. Reconcile observed terminal or verified fence evidence before continuing it.",
                lease.lease_id
            ));
        }
        Ok(())
    }

    pub(crate) fn is_remote_task_thread(&self, thread_id: usize) -> bool {
        self.tasks
            .iter()
            .any(|task| task.child_thread_id == thread_id && task.remote.is_some())
    }

    pub(crate) fn repair_execution_bindings(&mut self) {
        for thread in &mut self.threads {
            if let Some(binding) = &thread.execution {
                if thread.run_id.as_deref() != Some(binding.run_id.as_str()) {
                    thread.execution = None;
                }
            }
        }
    }
}
