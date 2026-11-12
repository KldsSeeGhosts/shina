//! KSG Code's composerDispatch, ComposerPrimaryActions and useComposerResume contracts.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TurnCapabilities {
    pub supports_interrupt: bool,
    pub supports_active_steering: bool,
    pub supports_steering_by_interrupt_restart: bool,
    pub supports_queued_messages: bool,
    pub active_steering_during_compaction: bool,
}

impl TurnCapabilities {
    pub const fn new(steering: bool, during_compaction: bool) -> Self {
        Self {
            supports_interrupt: true,
            supports_active_steering: steering,
            supports_steering_by_interrupt_restart: false,
            supports_queued_messages: true,
            active_steering_during_compaction: during_compaction,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopPhase {
    Pending,
    TimedOut,
    Landed,
}

#[derive(Default)]
pub struct StopControl {
    target: Option<String>,
    phase: Option<StopPhase>,
    deadline: Option<Instant>,
}

impl StopControl {
    pub fn phase(&self) -> Option<StopPhase> {
        self.phase
    }
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }
    pub fn disabled(&self) -> bool {
        matches!(self.phase, Some(StopPhase::Pending | StopPhase::Landed))
    }
    pub fn label(&self) -> &'static str {
        match self.phase {
            Some(StopPhase::Pending) => "Stopping",
            Some(StopPhase::Landed) => "Stopped",
            _ => "Stop generation",
        }
    }
    fn begin(&mut self, target: Option<String>) {
        self.target = target;
        self.phase = Some(StopPhase::Pending);
        self.deadline = Some(Instant::now() + Duration::from_secs(10));
    }
}

impl Thread {
    pub fn resume_pending(&self) -> bool {
        self.resume_command.is_some()
    }
    pub fn turn_capabilities(&self) -> TurnCapabilities {
        if self.harness == "claude"
            && let Some(capabilities) = self.harness_state["capabilities"].as_array()
        {
            return TurnCapabilities::new(
                capabilities
                    .iter()
                    .any(|capability| capability == "interrupt_cancel_queued_v1"),
                false,
            );
        }
        if let Ok(capabilities) =
            serde_json::from_value(self.harness_state["turnCapabilities"].clone())
        {
            return capabilities;
        }
        match self.harness.as_str() {
            "pi" => pi_session::TURN_CAPABILITIES,
            "codex" => crate::codex::TURN_CAPABILITIES,
            _ => TurnCapabilities::new(false, false),
        }
    }

    pub fn can_steer(&self) -> bool {
        let capabilities = self.turn_capabilities();
        (capabilities.supports_active_steering
            || capabilities.supports_interrupt
                && capabilities.supports_steering_by_interrupt_restart)
            && (self.context.phase != crate::context::ContextPhase::Compacting
                || capabilities.active_steering_during_compaction && !self.native.operating())
            && self.phase != "Stopping"
    }

    pub fn follow_up_queues(&self) -> bool {
        self.native.follow_up || !self.can_steer()
    }

    pub fn can_resume(&self) -> bool {
        self.run_control_supported
            && !self.status.busy()
            && !self.configuring
            && !self.initializing
            && !self.archived()
            && !self.offline_blocked
            && self.status != Status::Paused
            && self.request.is_none()
            && (self.resumable_run_id.is_some()
                || self.native.queue_held && !self.native.queue.is_empty())
    }

    pub(crate) fn tick_stop(&mut self, now: Instant) -> bool {
        let before = self.stop_control.phase;
        if self.run_id.is_some()
            && self.stop_control.target.is_some()
            && self.run_id != self.stop_control.target
        {
            self.stop_control = StopControl::default();
        } else if matches!(before, Some(StopPhase::Pending | StopPhase::TimedOut))
            && !self.status.busy()
        {
            self.stop_control.phase = Some(StopPhase::Landed);
            self.stop_control.deadline = None;
        } else if before == Some(StopPhase::Pending)
            && self
                .stop_control
                .deadline
                .is_some_and(|deadline| now >= deadline)
        {
            self.stop_control.phase = Some(StopPhase::TimedOut);
            self.stop_control.deadline = None;
            if self.phase == "Stopping" {
                self.phase = "Working".into();
            }
            if self
                .operation_deadline
                .is_some_and(|(kind, _)| kind == "abort")
            {
                self.operation_deadline = None;
            }
        } else if before == Some(StopPhase::Landed) && self.status.busy() {
            self.stop_control = StopControl::default();
            if self
                .operation_deadline
                .is_some_and(|(kind, _)| kind == "abort")
            {
                self.operation_deadline = None;
            }
        }
        before != self.stop_control.phase
    }

    pub(super) fn stop_deadline(&self) -> Option<Instant> {
        self.stop_control.deadline
    }

    /// Continue a specific latest interrupted run. A queue-only resume adds no user message.
    pub(super) fn resume_run(
        &mut self,
        source: Option<&str>,
        release_queue: bool,
    ) -> Result<(), String> {
        if !self.run_control_supported {
            return Err(crate::protocol::RUN_CONTROL_UPDATE_REQUIRED.into());
        }
        if self.status.busy()
            || self.offline_blocked
            || self.status == Status::Paused
            || self.configuring
            || self.initializing
            || self.archived()
            || self.request.is_some()
        {
            return Err("This thread can no longer be resumed from that run.".into());
        }
        if let Some(source) = source {
            if self.resumable_run_id.as_deref() != Some(source)
                || self.run_id.as_deref() != Some(source)
            {
                return Err("This thread can no longer be resumed from that run.".into());
            }
            let source = source.to_owned();
            self.begin_prompt(Prompt::new("Continue where you left off.".into(), vec![]));
            self.continuation_of_run_id = Some(source);
            if self.status == Status::Failed {
                return Err(self
                    .error
                    .clone()
                    .unwrap_or("Could not resume thread.".into()));
            }
        } else if !release_queue || !self.native.queue_held || self.native.queue.is_empty() {
            return Err("This thread has no held queued messages to resume.".into());
        }
        if release_queue {
            self.native.queue_held = false;
        }
        self.drive_native();
        Ok(())
    }

    pub(super) fn begin_stop(&mut self) {
        self.stop_control.begin(self.run_id.clone());
    }
}

impl State {
    pub fn resume_thread(&mut self) {
        if self.ownership_denied || self.demo || self.network_blocked() {
            return;
        }
        if self.current().resume_pending()
            || self.current().queued_edit.is_some()
            || self.current().remote_send.is_some()
            || self.current().queued_edit_saving()
            || !self.current().can_resume()
        {
            return;
        }
        let source = self.current().resumable_run_id.clone();
        let release = self.current().native.queue_held;
        if let Some(remote) = &self.current().remote {
            let result = remote.send(
                "thread.resume",
                json!({"threadId":self.current().id,
            "generation":self.current().request_generation,
            "continuationOfRunId":source,"releaseQueue":release}),
            );
            match result {
                Ok(command) => self.current_mut().resume_command = Some(command),
                Err(error) => self.current_mut().error = Some(error),
            }
            return;
        }
        if let Err(error) = self.current_mut().resume_run(source.as_deref(), release) {
            self.current_mut().error = Some(error);
            return;
        }
        if self.current().pi.is_none() {
            self.connect_thread(self.active);
        }
        self.checkpoint();
    }
}
