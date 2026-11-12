use super::*;

#[test]
fn dispatch_uses_provider_capability_before_the_user_preference() {
    let mut t = Thread::new(1);
    t.status = Status::Running;
    for harness in ["pi", "codex"] {
        t.harness = harness.into();
        t.native.follow_up = false;
        assert!(!t.follow_up_queues());
        t.native.follow_up = true;
        assert!(t.follow_up_queues());
    }
    for harness in ["claude", "generic-acp"] {
        t.harness = harness.into();
        t.native.follow_up = false;
        assert!(t.follow_up_queues());
    }
    t.harness = "claude".into();
    t.harness_state["turnCapabilities"] = json!({"supportsInterrupt":true,"supportsActiveSteering":true,"supportsQueuedMessages":true});
    assert!(!t.follow_up_queues());
    t.context.begin_compaction();
    assert!(t.follow_up_queues());
}

#[test]
fn stop_pending_lands_or_times_out_without_changing_its_target_or_killing_the_process() {
    let (pi, _commands, _) = crate::backend::Pi::test_connection();
    let mut t = Thread::new(1);
    t.pi = Some(pi);
    t.status = Status::Running;
    t.run_id = Some("run-a".into());
    t.stop();
    assert_eq!(t.stop_control.phase(), Some(StopPhase::Pending));
    assert_eq!(t.stop_control.target(), Some("run-a"));
    assert!(t.stop_control.disabled());
    let now = Instant::now() + Duration::from_secs(10);
    t.tick_stop(now);
    assert_eq!(t.stop_control.phase(), Some(StopPhase::TimedOut));
    assert!(!t.stop_control.disabled());
    assert!(t.operation_deadline.is_none());
    assert!(t.pi.is_some());
    t.apply(json!({"type":"agent_settled","aborted":true}));
    t.tick_stop(now);
    assert_eq!(t.stop_control.phase(), Some(StopPhase::Landed));
    assert_eq!(t.resumable_run_id.as_deref(), Some("run-a"));
}

#[test]
fn stopping_holds_the_queue_and_explicit_resume_continues_the_exact_run_first() {
    let (pi, commands, _) = crate::backend::Pi::test_connection();
    let mut t = Thread::new(1);
    t.pi = Some(pi);
    t.model_provider = "cpa".into();
    t.status = Status::Running;
    t.run_id = Some("interrupted".into());
    t.native.follow_up = true;
    t.draft.replace_text("Held follow-up");
    t.send_during_run();
    t.stop();
    assert_eq!(t.native.queue.len(), 1);
    assert!(t.native.queue_held);
    assert!(t.draft.text.is_empty());
    t.apply(json!({"type":"agent_settled","aborted":true}));
    assert_eq!(t.native.queue.len(), 1);
    assert!(t.resume_run(Some("stale"), true).is_err());
    t.resume_run(Some("interrupted"), true).unwrap();
    assert!(!t.native.queue_held);
    assert_eq!(t.native.queue.len(), 1);
    assert_eq!(
        t.messages.last().unwrap().text,
        "Continue where you left off."
    );
    assert!(
        commands
            .try_iter()
            .any(|r| r["type"] == "prompt" && r["message"] == "Continue where you left off.")
    );
}

#[test]
fn held_queue_only_resume_does_not_invent_a_continuation_message() {
    let mut t = Thread::new(1);
    t.native.queue_held = true;
    t.native.queue.push_back(pi_session::QueuedMessage {
        id: 5,
        prompt: Prompt::new("Held".into(), vec![]),
    });
    t.resume_run(None, true).unwrap();
    assert!(!t.native.queue_held);
    assert!(t.messages.is_empty());
}

#[test]
fn held_queue_and_interrupted_identity_survive_restart_without_replaying() {
    let mut t = Thread::new(1);
    t.native.queue_held = true;
    t.native.queue.push_back(pi_session::QueuedMessage {
        id: 19,
        prompt: Prompt::new("Held after restart".into(), vec![]),
    });
    let saved = t.pending_inputs_value();
    let mut restored = Thread::new(1);
    restored.restore_pending_inputs(&saved).unwrap();
    restored.restore_pending_inputs(&saved).unwrap();
    assert!(restored.native.queue_held);
    assert_eq!(
        restored.native.queue.len(),
        1,
        "duplicate snapshots do not duplicate the queue"
    );
    assert_eq!(restored.native.queue[0].id, 19);
    assert!(restored.draft.text.is_empty());
    assert!(!restored.drive_native());
}

#[test]
fn a_new_run_resets_stop_and_older_peers_cannot_resume() {
    let mut t = Thread::new(1);
    t.run_id = Some("old".into());
    t.begin_stop();
    t.run_id = Some("new".into());
    t.tick_stop(Instant::now());
    assert!(t.stop_control.phase().is_none());
    t.run_control_supported = false;
    assert!(
        t.resume_run(None, true)
            .unwrap_err()
            .contains("Update required")
    );
    let capabilities: TurnCapabilities = serde_json::from_value(json!({
        "supportsActiveSteering":true,"futureCapability":{"kind":"unknown"}
    }))
    .unwrap();
    assert!(capabilities.supports_active_steering);
    assert!(!capabilities.supports_interrupt);
}

#[test]
fn normal_completion_after_stop_does_not_offer_an_interrupted_run() {
    let (pi, _commands, _) = crate::backend::Pi::test_connection();
    let mut t = Thread::new(1);
    t.pi = Some(pi);
    t.status = Status::Running;
    t.run_id = Some("completed".into());
    t.stop();
    t.apply(json!({"type":"agent_settled","aborted":false}));
    assert!(t.resumable_run_id.is_none());
    assert!(!t.can_resume());
    assert!(t.resume_run(Some("completed"), true).is_err());
}

#[test]
fn a_stop_captured_without_a_run_cannot_stop_a_run_that_started_later() {
    let mut state = State::new(PathBuf::new(), false);
    state.current_mut().run_id = Some("new-run".into());
    state.current_mut().status = Status::Running;
    let command = crate::protocol::Command::new(
        "thread.stop",
        json!({"threadId":state.current().id,"runId":null}),
    );
    assert!(
        state
            .execute_command(&command)
            .unwrap_err()
            .contains("no longer active")
    );
    assert_eq!(state.current().stop_serial, 0);
    assert!(state.current().status.busy());
}
