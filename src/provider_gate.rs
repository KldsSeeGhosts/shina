//! Provider executables cannot change while a turn or a discovery process owns them.
use super::*;

impl State {
    pub(crate) fn hold_provider(&mut self, driver: &str) -> Vec<crate::backend::ShutdownFence> {
        self.provider_update_drivers.insert(driver.into());
        let mut fences = Vec::new();
        for thread in self
            .threads
            .iter_mut()
            .filter(|thread| thread.machine == "local" && thread.harness == driver)
        {
            thread.provider_update_held = true;
            if let Some(pi) = &thread.pi {
                fences.push(pi.stop_fence());
                // Admission checked that no running or queued turn owns this session.
                thread.disconnect();
            }
        }
        let catalog = match driver {
            "codex" => Some(&mut self.codex_catalog),
            "claude" => Some(&mut self.claude_catalog),
            _ => None,
        };
        if let Some(fence) = catalog.and_then(|catalog| catalog.stop_for_update()) {
            fences.push(fence);
        }
        fences
    }

    pub(crate) fn release_provider(&mut self, driver: &str) {
        self.provider_update_drivers.remove(driver);
        for thread in self
            .threads
            .iter_mut()
            .filter(|thread| thread.machine == "local" && thread.harness == driver)
        {
            thread.provider_update_held = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_gate_keeps_a_new_prompt_until_the_provider_is_available() {
        let mut state = State::headless(crate::store::directory().unwrap());
        state.threads[0].harness = "codex".into();
        let (pi, commands, _events) = Pi::test_connection();
        state.threads[0].pi = Some(pi);
        state.hold_provider("codex");
        assert!(state.threads[0].pi.is_none());
        assert!(state.update_restart_blocker().unwrap().contains("provider"));
        // A send during maintenance stays in the normal durable queued-prompt path.
        state.threads[0].begin_prompt(Prompt::new("keep my work".into(), vec![]));
        assert!(state.threads[0].queued_prompt.is_some());
        assert!(commands.try_recv().is_err());
        state.connect_thread(0);
        assert!(state.threads[0].pi.is_none());
        state.release_provider("codex");
        let (pi, commands, _events) = Pi::test_connection();
        state.threads[0].pi = Some(pi);
        state.threads[0].supports_images = Some(true);
        state.threads[0].drive_session();
        assert_eq!(commands.try_recv().unwrap()["message"], "keep my work");
    }

    #[test]
    fn update_gate_applies_to_new_threads_and_only_its_driver() {
        let mut state = State::headless(crate::store::directory().unwrap());
        state.hold_provider("codex");
        let mut thread = Thread::new(2);
        thread.harness = "codex".into();
        state.threads.push(thread);
        state.connect_thread(1);
        assert!(state.threads[1].provider_update_held);
        state.threads[0].harness = "claude".into();
        assert!(!state.threads[0].provider_update_held);
        state.release_provider("codex");
        assert!(!state.threads[1].provider_update_held);
        assert!(!state.provider_update_drivers.contains("codex"));
    }
}
