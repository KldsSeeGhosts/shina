//! The per-thread provider event drain lifted out of `model::poll`. It stays a
//! child of `model` so private `State`/`Thread` access still works, but it takes
//! no `rune::Tx` and calls no UI: requested wakes come back in `Batch::wakes` and
//! delivery (connects, diff bumps, injections, failure notices) stays in `poll`.
use super::*;

/// One bounded pass over every thread's provider queue: what changed, which wakes
/// the drain asked for, and the work `poll` delivers on the drained threads' behalf.
pub(super) struct Batch {
    /// Any drained thread changed visible state.
    pub changed: bool,
    /// A run settled, errored or exited; `poll` also sweeps lifecycles on it.
    pub run_ended: bool,
    /// A thread hit the 256-event drain budget; `poll` schedules another frame.
    pub event_budget_exhausted: bool,
    /// Threads ready to connect an idle session for a queued prompt.
    pub connect_indices: Vec<usize>,
    /// Threads whose provider turn ended or reported file changes; `poll` marks
    /// their diff panels stale (KSG's workspace-mutation refresh bump).
    pub diff_notes: Vec<usize>,
    /// `(parent thread id, notice)` crew questions to inject into parents.
    pub crew_questions: Vec<(usize, String)>,
    /// `(thread id, error)` background sends whose session failed while pending.
    pub background_failures: Vec<(usize, String)>,
    /// Wake deadlines the drain asked for, in request order.
    pub wakes: Vec<Duration>,
}

/// Drain each connected thread's provider events, bounded per thread per frame,
/// then run its deadline, idle-timeout and queued-prompt bookkeeping. Remote task
/// children are skipped: their records arrive over the peer link, not a session.
pub(super) fn poll_threads(state: &mut State) -> Batch {
    let mut batch = Batch {
        changed: false,
        run_ended: false,
        event_budget_exhausted: false,
        connect_indices: Vec::new(),
        diff_notes: Vec::new(),
        crew_questions: Vec::new(),
        background_failures: Vec::new(),
        wakes: Vec::new(),
    };
    let remote_children: std::collections::HashSet<_> = state
        .tasks
        .iter()
        .filter(|t| t.remote.is_some())
        .map(|t| t.child_thread_id)
        .collect();
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
                batch.event_budget_exhausted = true;
                batch.wakes.push(Duration::ZERO);
            }
            thread_changed = true;
            thread.updated = crate::projects::now();
            thread.life.updated_at = lifecycle::now();
            match event {
                Event::Record(record) => {
                    state.background.observe_record(&record);
                    let settled = record["type"] == "agent_settled";
                    // A tool ending can grow `changes`; remember where they stood.
                    let changes_before = (record["type"] == "tool_execution_end").then(|| {
                        thread
                            .tools
                            .iter()
                            .map(|tool| tool.changes.len())
                            .sum::<usize>()
                    });
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
                            thread.forwarded_question = Some(token.clone());
                            batch.crew_questions.push((parent, format!(
                            "Shina: crew #{} asks a question.\n{}\nAnswer with mcp__shina__task_send using thread_id {}, a fresh client_request_id, request_id {}, runtime_generation {}, and message containing your answer. Reuse the same key and arguments for retries. For select, use one listed option exactly. Approval requests still require the user.",
                            thread.id, record, thread.id, json!(token), json!(thread.request_generation)
                            )));
                        }
                    }
                    batch.run_ended |= settled;
                    let acp_catalog = state.acp_catalogs.get_mut(&thread.harness).filter(|_| {
                        record["command"] == "get_available_models" && record["success"] == true
                    });
                    thread.apply(record);
                    if let Some(catalog) = acp_catalog {
                        catalog.apply_live_acp_models(&thread.models);
                    }
                    if settled
                        || changes_before.is_some_and(|before| {
                            thread
                                .tools
                                .iter()
                                .map(|tool| tool.changes.len())
                                .sum::<usize>()
                                > before
                        })
                    {
                        batch.diff_notes.push(index);
                    }
                    if settled {
                        thread.command("get_state", json!({}));
                    }
                    if !state.headless && index == state.active {
                        thread.life.last_visited_at = Some(lifecycle::now());
                    }
                }
                Event::Error(error) => {
                    batch.run_ended = true;
                    batch.diff_notes.push(index);
                    thread.fail_session(error);
                }
                Event::Diagnostic(diagnostic) => thread.diagnostic(&diagnostic),
                Event::Exited => {
                    batch.run_ended = true;
                    batch.diff_notes.push(index);
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
            batch
                .wakes
                .push(deadline.saturating_duration_since(Instant::now()));
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
                batch.background_failures.push((
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
            batch.wakes.push(Duration::from_secs(1));
            batch.changed = true;
        }
        // A quiet session can become dispatchable without another provider record.
        // Its intent must join this batch even when a different thread streamed.
        thread_changed |= crate::outbox::captured_count() != effects_before;
        if thread_changed {
            batch.changed = true;
            if let Some(scope) = &mut state.persistence_scope {
                scope.insert(thread.id);
            }
        }
        if thread.pi.is_some() {
            if let Some((_, deadline)) = thread.operation_deadline {
                batch
                    .wakes
                    .push(deadline.saturating_duration_since(Instant::now()));
            }
            if !thread.offline_blocked
                && let Some(deadline) = thread.injection_retry_at
            {
                batch
                    .wakes
                    .push(deadline.saturating_duration_since(Instant::now()));
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
            batch.wakes.push(remaining);
        }
        if thread.pi.is_none()
            && thread.queued_prompt.is_some()
            && thread.status == Status::Running
            && !thread.offline_blocked
            && thread.session_started.is_none()
        {
            batch.connect_indices.push(index);
        }
    }
    batch
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_flooded_thread_spends_its_own_budget_without_starving_others() {
        let mut state = State::headless(crate::store::directory().unwrap());
        let (pi, _commands, flooded) = Pi::test_connection_with_capacity(257);
        state.threads[0].pi = Some(pi);
        state.threads[0].status = Status::Ready;
        let mut quiet = Thread::new(2);
        let (_quiet_commands, quiet_events) = fake_pi(&mut quiet, Status::Ready);
        state.threads.push(quiet);
        for _ in 0..256 {
            flooded
                .send(Event::Diagnostic("queued output".into()))
                .unwrap();
        }
        flooded
            .send(Event::Record(json!({"type":"agent_start"})))
            .unwrap();
        quiet_events
            .send(Event::Record(json!({"type":"agent_start"})))
            .unwrap();
        let batch = poll_threads(&mut state);
        // The flooded thread hit the 256-event budget: its last record waits, the
        // next frame is requested, and the quiet thread still drained this pass.
        assert!(batch.event_budget_exhausted);
        assert!(batch.wakes.contains(&Duration::ZERO));
        assert_eq!(state.threads[0].status, Status::Ready);
        assert_eq!(state.threads[1].status, Status::Running);
        let batch = poll_threads(&mut state);
        assert_eq!(state.threads[0].status, Status::Running);
        assert!(!batch.event_budget_exhausted);
    }

    #[test]
    fn a_settled_run_ends_and_asks_pi_for_fresh_state() {
        let mut state = State::headless(crate::store::directory().unwrap());
        state.persistence_scope = Some(Default::default());
        let (commands, events) = fake_pi(&mut state.threads[0], Status::Running);
        events
            .send(Event::Record(
                json!({"type":"agent_settled","aborted":false}),
            ))
            .unwrap();
        let batch = poll_threads(&mut state);
        assert!(batch.run_ended);
        assert_eq!(batch.diff_notes, vec![0]);
        assert!(batch.changed);
        assert!(commands.try_iter().any(|c| c["type"] == "get_state"));
        assert!(
            state
                .persistence_scope
                .as_ref()
                .unwrap()
                .contains(&state.threads[0].id)
        );
        assert!(!state.artwork.is_initialized());
        assert!(!state.portraits.is_initialized());
        assert!(!state.attachment_cache.is_initialized());
    }

    #[test]
    fn an_exited_session_fails_the_thread_and_ends_the_run() {
        let mut state = State::headless(crate::store::directory().unwrap());
        let (_commands, events) = fake_pi(&mut state.threads[0], Status::Running);
        events.send(Event::Exited).unwrap();
        let batch = poll_threads(&mut state);
        assert!(batch.run_ended);
        assert_eq!(batch.diff_notes, vec![0]);
        assert_eq!(state.threads[0].status, Status::Failed);
        assert!(
            state.threads[0]
                .error
                .as_deref()
                .unwrap()
                .contains("Pi stopped unexpectedly")
        );
        assert!(state.threads[0].pi.is_none());
    }

    #[test]
    fn session_deadlines_come_back_as_wakes_instead_of_touching_a_tx() {
        let mut state = State::headless(crate::store::directory().unwrap());
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Ready);
        state.threads[0].session_started = Some(Instant::now());
        let batch = poll_threads(&mut state);
        // The START_TIMEOUT countdown is the only wake: the caller schedules it.
        assert_eq!(batch.wakes.len(), 1);
        assert!(batch.wakes[0] > Duration::ZERO && batch.wakes[0] <= START_TIMEOUT);
        assert!(!batch.run_ended && !batch.event_budget_exhausted);
        assert!(batch.connect_indices.is_empty() && batch.diff_notes.is_empty());
    }

    #[test]
    fn repaint_wakes_are_only_requested_when_a_ui_exists() {
        for headless in [false, true] {
            let mut state = if headless {
                State::headless(crate::store::directory().unwrap())
            } else {
                State::new(PathBuf::new(), false)
            };
            let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
            let batch = poll_threads(&mut state);
            // The busy active thread asks for a 1 s repaint only with a UI; the
            // session keep-alive wake is requested either way.
            assert_eq!(batch.wakes.contains(&Duration::from_secs(1)), !headless);
            assert!(batch.wakes.contains(&IDLE_TIMEOUT));
            assert_eq!(batch.changed, !headless);
        }
    }

    #[test]
    fn a_child_question_is_batched_once_for_poll_to_deliver() {
        let mut state = State::headless(crate::store::directory().unwrap());
        let mut crew = Thread::new(2);
        crew.parent = Some(state.threads[0].id);
        let (_commands, events) = fake_pi(&mut crew, Status::Running);
        state.threads.push(crew);
        for _ in 0..2 {
            events
                .send(Event::Record(json!({
                    "type": "extension_ui_request",
                    "id": "q1",
                    "method": "select",
                    "options": ["a", "b"]
                })))
                .unwrap();
        }
        let batch = poll_threads(&mut state);
        // The same request token is forwarded once, and the drain itself never
        // injects into the parent: that delivery belongs to `poll`.
        assert_eq!(batch.crew_questions.len(), 1);
        assert_eq!(batch.crew_questions[0].0, state.threads[0].id);
        assert!(state.threads[0].pending_injections.is_empty());
    }

    #[test]
    fn a_background_send_failure_is_reported_once() {
        let mut state = State::headless(crate::store::directory().unwrap());
        let mut sender = Thread::new(2);
        sender.background_start_pending = true;
        let (_commands, events) = fake_pi(&mut sender, Status::Running);
        state.threads.push(sender);
        events.send(Event::Error("spawn failed".into())).unwrap();
        let batch = poll_threads(&mut state);
        assert_eq!(
            batch.background_failures,
            vec![(state.threads[1].id, "spawn failed".to_owned())]
        );
        assert!(!state.threads[1].background_start_pending);
        let batch = poll_threads(&mut state);
        assert!(batch.background_failures.is_empty());
    }

    #[test]
    fn a_queued_prompt_on_a_disconnected_thread_asks_for_a_reconnect() {
        let mut state = State::headless(crate::store::directory().unwrap());
        state.threads[0].status = Status::Running;
        state.threads[0].queued_prompt = Some((Prompt::new("queued".into(), vec![]), 0));
        let batch = poll_threads(&mut state);
        assert_eq!(batch.connect_indices, vec![0]);
        // `poll` owns the actual connect; the batch only reports the index.
        assert!(state.threads[0].pi.is_none());
    }

    #[test]
    fn an_accepted_correction_waits_for_its_own_turn_outcome() {
        for fail_in_same_batch in [false, true] {
            let mut state = State::headless(crate::store::directory().unwrap());
            state.demo = true;
            let (_parent_commands, _parent_events) = fake_pi(&mut state.threads[0], Status::Ready);
            let mut child = Thread::new(2);
            child.parent = Some(1);
            child.delegated = true;
            child.arm_task();
            let (commands, events) = fake_pi(&mut child, Status::Running);
            let mut task = tasks::Task::new(&child, "First brief", None);
            task.state = tasks::TaskState::Running;
            state.threads.push(child);
            state.tasks.push(task);
            state
                .prepare_task_control(1, 1, "task_send", &json!({"message":"Correction"}))
                .unwrap();
            assert!(state.threads[1].flush_injection());
            let prompt = commands
                .try_iter()
                .find(|command| command["type"] == "prompt")
                .unwrap();
            state.threads[1].apply(json!({
                "type":"response","id":prompt["id"],"command":"prompt",
                "success":true,"data":{"disposition":"queued"}
            }));
            assert!(state.threads[1].pending_injections.is_empty());
            state.threads[1].native.follow_ups.push("Correction".into());
            events
                .send(Event::Record(
                    json!({"type":"agent_settled","aborted":false}),
                ))
                .unwrap();
            if !fail_in_same_batch {
                poll_threads(&mut state);
                state.poll_tasks(Instant::now());
                assert_eq!(state.tasks[0].state, tasks::TaskState::Running);
                assert!(state.threads[0].pending_injections.is_empty());
            }
            events
                .send(Event::Record(
                    json!({"type":"queue_update","steering":[],"followUp":[]}),
                ))
                .unwrap();
            events
                .send(Event::Record(json!({"type":"agent_start"})))
                .unwrap();
            if fail_in_same_batch {
                events
                    .send(Event::Error("Correction failed".into()))
                    .unwrap();
            }
            poll_threads(&mut state);
            state.poll_tasks(Instant::now());
            if !fail_in_same_batch {
                assert_eq!(state.tasks[0].state, tasks::TaskState::Running);
                assert!(state.threads[0].pending_injections.is_empty());
                events
                    .send(Event::Error("Correction failed".into()))
                    .unwrap();
                poll_threads(&mut state);
                state.poll_tasks(Instant::now());
            }
            assert_eq!(state.tasks[0].state, tasks::TaskState::Failed);
            assert_eq!(state.tasks[0].error_excerpt, "Correction failed");
        }
    }
}
