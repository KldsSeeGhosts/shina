//! Real-Git coverage for the delegated-worker placement contract (#20).
//! Provisioned checkouts are actual `git worktree` directories under the
//! state's data dir; providers are the fake `Pi::test_connection`.
use super::*;
use crate::protocol::Command;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    empty_cfg: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("shina-placement-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let empty_cfg = root.join("empty.gitconfig");
        std::fs::write(&empty_cfg, "").unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let mut cmd = std::process::Command::new("git");
            cmd.arg("-c")
                .arg("user.name=Test User")
                .arg("-c")
                .arg("user.email=test@example.com")
                .args(args)
                .current_dir(&repo)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_CONFIG_GLOBAL", &empty_cfg)
                .env("GIT_CONFIG_SYSTEM", &empty_cfg);
            let out = crate::process::output(&mut cmd, Duration::from_secs(10), 1024 * 1024)
                .expect("spawn git");
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        git(&["init", "-b", "main", "--quiet"]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        git(&["add", "README.md"]);
        git(&["commit", "-m", "initial", "--quiet"]);
        Fixture {
            root,
            repo,
            empty_cfg,
        }
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-c")
            .arg("user.name=Test User")
            .arg("-c")
            .arg("user.email=test@example.com")
            .args(args)
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_GLOBAL", &self.empty_cfg)
            .env("GIT_CONFIG_SYSTEM", &self.empty_cfg);
        let out = crate::process::output(&mut cmd, Duration::from_secs(10), 1024 * 1024)
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn worktree_dirs(&self) -> Vec<PathBuf> {
        let out = self.git(&self.repo, &["worktree", "list", "--porcelain"]);
        out.lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(PathBuf::from)
            .collect()
    }

    fn state(
        &self,
    ) -> (
        State,
        std::sync::Arc<std::sync::Mutex<Vec<(usize, String)>>>,
    ) {
        let mut state = State::new(PathBuf::new(), false);
        state.data_dir = self.root.join("data");
        std::fs::create_dir_all(&state.data_dir).unwrap();
        state.load_global_for_test();
        state.runtime_enabled = true;
        state.test_git_context = Some(crate::vcs::GitContext {
            git_config_global: Some(self.empty_cfg.clone()),
            git_config_system: Some(self.empty_cfg.clone()),
        });
        state
            .projects
            .entries
            .push(Project::new(1, self.repo.clone()));
        state.projects.next_id = 2;
        state.threads[0].project = Some(1);
        let connects: std::sync::Arc<std::sync::Mutex<Vec<(usize, String)>>> =
            std::sync::Arc::default();
        let hook = connects.clone();
        state.test_connect_pi = Some(Box::new(move |tid, path| {
            hook.lock()
                .unwrap()
                .push((tid, path.to_string_lossy().into_owned()));
            let (pi, _commands, _events) = Pi::test_connection();
            Some(pi)
        }));
        (state, connects)
    }
}

fn poll_until_ready(state: &mut State, deadline_secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(deadline_secs);
    loop {
        state.poll_placements();
        state.poll_worktree_setup();
        let pending = state.threads.iter().any(|thread| {
            thread
                .placement
                .as_ref()
                .is_some_and(|p| p.phase == placement::PlacementPhase::Pending)
        });
        if !pending || Instant::now() > deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn child_placement(state: &State, index: usize) -> &placement::ThreadPlacement {
    state.threads[index].placement.as_ref().expect("placement")
}

#[test]
fn delegated_writers_get_separate_worktrees_and_checkouts() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("writers");
    let (mut state, connects) = fixture.state();
    for brief in ["Write file one", "Write file two"] {
        state
            .mcp_call(1, "delegate_task", &json!({"prompt": brief}))
            .unwrap();
    }
    assert_eq!(state.threads.len(), 3);
    assert_eq!(fixture.worktree_dirs().len(), 1);
    poll_until_ready(&mut state, 15);
    let one = child_placement(&state, 1);
    let two = child_placement(&state, 2);
    assert_eq!(
        one.phase,
        placement::PlacementPhase::Ready,
        "{:?}",
        one.error
    );
    assert_eq!(two.phase, placement::PlacementPhase::Ready);
    assert_eq!(one.mode, placement::PlacementMode::Isolated);
    assert!(one.new_worktree && two.new_worktree);
    assert_ne!(one.checkout, two.checkout);
    assert_ne!(one.branch, two.branch);
    for index in [1, 2] {
        let placement = child_placement(&state, index);
        let checkout = Path::new(placement.checkout.as_ref().unwrap());
        assert!(checkout.starts_with(state.data_dir.join("worktrees")));
        assert!(checkout.join("README.md").exists());
        assert_eq!(
            state.threads[index].worktree_path.as_deref(),
            placement.checkout.as_deref()
        );
        assert_eq!(state.threads[index].env_mode, Some(ThreadEnvMode::Worktree));
        let checkout = placement.checkout.clone();
        let base_commit = placement.base_commit.clone();
        let status = state
            .mcp_call(
                1,
                "task_status",
                &json!({"thread_id": state.threads[index].id}),
            )
            .unwrap();
        assert_eq!(status["placement"]["checkout"], json!(checkout));
        assert_eq!(status["placement"]["baseCommit"], json!(base_commit));
    }
    assert_eq!(fixture.worktree_dirs().len(), 3);
    // The parent checkout and branch were never touched.
    assert_eq!(
        fixture
            .git(&fixture.repo, &["symbolic-ref", "--short", "HEAD"])
            .trim(),
        "main"
    );
    // Each provider started inside its own worktree, never in the project.
    let connected = connects.lock().unwrap().clone();
    assert_eq!(connected.len(), 2);
    for index in [1, 2] {
        let placement = child_placement(&state, index);
        assert!(connected.iter().any(|(tid, path)| {
            *tid == state.threads[index].id
                && Path::new(path) == Path::new(placement.checkout.as_ref().unwrap())
        }));
    }
}

#[test]
fn shared_reader_gets_parent_checkout_and_stays_read_only() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("shared");
    std::fs::write(fixture.repo.join("dirty.txt"), "uncommitted\n").unwrap();
    let (mut state, connects) = fixture.state();
    state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt": "Read the tree", "placement": {"mode": "shared"}}),
        )
        .unwrap();
    poll_until_ready(&mut state, 5);
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(placement.mode, placement::PlacementMode::Shared);
    assert_eq!(
        crate::vcs::real_location(Path::new(placement.checkout.as_ref().unwrap())),
        crate::vcs::real_location(&fixture.repo)
    );
    assert_eq!(placement.branch.as_deref(), Some("main"));
    assert!(!placement.new_worktree);
    assert!(placement.primary_checkout);
    assert_eq!(state.threads[1].native.access, AccessMode::ReadOnly);
    assert_eq!(fixture.worktree_dirs().len(), 1);
    // The reader ran in the parent's dirty checkout.
    let connected = connects.lock().unwrap().clone();
    assert_eq!(connected.len(), 1);
    assert_eq!(
        crate::vcs::real_location(Path::new(&connected[0].1)),
        crate::vcs::real_location(&fixture.repo)
    );
    // The parent's uncommitted work was never disturbed.
    assert_eq!(
        std::fs::read_to_string(fixture.repo.join("dirty.txt")).unwrap(),
        "uncommitted\n"
    );
}

#[test]
fn retry_after_provisioning_failure_reuses_the_same_reservation() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("retry");
    let (mut state, _connects) = fixture.state();
    state
        .mcp_call(1, "delegate_task", &json!({"prompt": "Write it"}))
        .unwrap();
    let planned = PathBuf::from(child_placement(&state, 1).checkout.clone().unwrap());
    let branch = child_placement(&state, 1).branch.clone().unwrap();
    // A stale directory blocks the recorded checkout: report it, never delete it.
    std::fs::create_dir_all(&planned).unwrap();
    poll_until_ready(&mut state, 10);
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Failed);
    assert!(
        placement
            .error
            .as_deref()
            .unwrap_or("")
            .contains("will not delete"),
        "{}",
        placement.error.as_deref().unwrap_or("")
    );
    assert_eq!(state.tasks[0].state, tasks::TaskState::Queued);
    assert!(state.threads[1].pi.is_none());
    std::fs::remove_dir_all(&planned).unwrap();
    // The next attempt requeues the same task and reuses the reservation.
    let child_id = state.threads[1].id;
    state
        .mcp_call(
            1,
            "task_send",
            &json!({"thread_id": child_id, "message": "continue"}),
        )
        .unwrap();
    poll_until_ready(&mut state, 15);
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(
        placement.checkout.as_deref(),
        Some(planned.to_string_lossy().as_ref())
    );
    assert_eq!(placement.branch.as_deref(), Some(branch.as_str()));
    assert_eq!(fixture.worktree_dirs().len(), 2);
}

#[test]
fn restart_preserves_and_reuses_the_recorded_reservation() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("restart");
    let (mut state, _connects) = fixture.state();
    let status = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt": "Write it", "client_request_id": "wt-1"}),
        )
        .unwrap();
    assert_eq!(
        state.threads.len(),
        2,
        "delegate accepted without a child: {status}"
    );
    let planned_checkout = child_placement(&state, 1).checkout.clone().unwrap();
    let planned_branch = child_placement(&state, 1).branch.clone().unwrap();
    // Restart while the reservation is still pending, then provision once.
    let snapshot = state.snapshot();
    let mut restored = State::new(PathBuf::new(), false);
    restored.data_dir = state.data_dir.clone();
    restored.restore_global(&snapshot).unwrap();
    restored.runtime_enabled = true;
    restored.test_git_context = state.test_git_context.clone();
    poll_until_ready(&mut restored, 15);
    let placement = child_placement(&restored, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(
        placement.checkout.as_deref(),
        Some(planned_checkout.as_str())
    );
    assert_eq!(placement.branch.as_deref(), Some(planned_branch.as_str()));
    assert_eq!(fixture.worktree_dirs().len(), 2);
    // A second restart keeps the ready reservation; nothing re-provisions.
    let snapshot = restored.snapshot();
    let mut again = State::new(PathBuf::new(), false);
    again.data_dir = restored.data_dir.clone();
    again.restore_global(&snapshot).unwrap();
    let placement = again.threads[1].placement.as_ref().unwrap();
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(
        placement.checkout.as_deref(),
        Some(planned_checkout.as_str())
    );
    again.poll_placements();
    assert_eq!(fixture.worktree_dirs().len(), 2);
    // A same-key retry after the restart replays the original task, not a new one.
    let replay = again
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt": "Write it", "client_request_id": "wt-1"}),
        )
        .unwrap();
    assert_eq!(replay["thread_id"], json!(again.threads[1].id));
    assert_eq!(replay["replayed"], true);
}

#[test]
fn setup_failure_starts_no_provider_and_leaves_the_parent_intact() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("setup-fail");
    std::fs::write(
        fixture.repo.join(crate::vcs::T3_PROJECT_FILE_NAME),
        json!({"scripts":[{"name":"boom","command":"exit 3","runOnWorktreeCreate":true}]})
            .to_string(),
    )
    .unwrap();
    fixture.git(&fixture.repo, &["add", crate::vcs::T3_PROJECT_FILE_NAME]);
    fixture.git(&fixture.repo, &["commit", "-m", "script", "--quiet"]);
    std::fs::write(fixture.repo.join("staged.txt"), "staged\n").unwrap();
    fixture.git(&fixture.repo, &["add", "staged.txt"]);
    let (mut state, connects) = fixture.state();
    state
        .mcp_call(1, "delegate_task", &json!({"prompt": "Write it"}))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        state.poll_placements();
        state.poll_worktree_setup();
        if matches!(
            child_placement(&state, 1).phase,
            placement::PlacementPhase::Ready | placement::PlacementPhase::Failed
        ) || Instant::now() > deadline
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Failed);
    assert_eq!(
        state.threads[1]
            .worktree_setup
            .as_ref()
            .map(|setup| setup.phase),
        Some(WorktreeSetupPhase::Failed)
    );
    assert!(state.threads[1].pi.is_none());
    assert_eq!(state.tasks[0].state, tasks::TaskState::Queued);
    assert!(connects.lock().unwrap().is_empty());
    assert_eq!(
        fixture
            .git(&fixture.repo, &["symbolic-ref", "--short", "HEAD"])
            .trim(),
        "main"
    );
    assert_eq!(
        fixture
            .git(&fixture.repo, &["status", "--porcelain"])
            .trim(),
        "A  staged.txt"
    );
}

#[test]
fn unsupported_placements_reject_before_any_allocation() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("invalid");
    let non_repo = fixture.root.join("plain");
    std::fs::create_dir_all(&non_repo).unwrap();
    let (mut state, _connects) = fixture.state();
    state.projects.entries.push(Project::new(2, non_repo));
    state.threads[0].project = Some(2);
    let error = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt": "Write", "placement": {"mode": "isolated"}}),
        )
        .unwrap_err();
    assert!(error.contains("not a Git repository"), "{error}");
    assert_eq!(state.threads.len(), 1);
    assert!(state.tasks.is_empty());
    let error = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt": "Write", "placement": {"mode": "shared", "branch": "x"}}),
        )
        .unwrap_err();
    assert!(error.contains("takes no base"), "{error}");
    state.threads[0].project = Some(1);
    let error = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt": "Write", "placement": {"mode": "existing", "worktree": "missing-branch"}}),
        )
        .unwrap_err();
    assert!(error.contains("No worktree"), "{error}");
    assert_eq!(state.threads.len(), 1);
    assert_eq!(fixture.worktree_dirs().len(), 1);
}

#[test]
fn non_git_project_defaults_to_shared_with_a_capability_note() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("nongit");
    let plain = fixture.root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let (mut state, _connects) = fixture.state();
    state.projects.entries.push(Project::new(2, plain.clone()));
    state.threads[0].project = Some(2);
    state
        .mcp_call(1, "delegate_task", &json!({"prompt": "Write it"}))
        .unwrap();
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(placement.mode, placement::PlacementMode::Shared);
    assert_eq!(
        crate::vcs::real_location(Path::new(placement.checkout.as_ref().unwrap())),
        crate::vcs::real_location(&plain)
    );
    assert!(
        placement
            .capability
            .as_deref()
            .unwrap_or("")
            .contains("not a Git repository"),
        "{:?}",
        placement.capability
    );
}

#[test]
fn existing_worktree_selection_runs_in_the_selected_checkout() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("existing");
    let selected = fixture.root.join("picked");
    fixture.git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "-b",
            "picked-branch",
            &selected.to_string_lossy(),
            "main",
        ],
    );
    let (mut state, connects) = fixture.state();
    state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt": "Work here", "placement": {"mode": "existing", "worktree": "picked-branch"}}),
        )
        .unwrap();
    poll_until_ready(&mut state, 5);
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(placement.mode, placement::PlacementMode::Existing);
    assert!(!placement.new_worktree);
    assert_eq!(placement.branch.as_deref(), Some("picked-branch"));
    assert_eq!(
        crate::vcs::real_location(Path::new(placement.checkout.as_ref().unwrap())),
        crate::vcs::real_location(&selected)
    );
    assert_eq!(state.threads[1].env_mode, Some(ThreadEnvMode::Worktree));
    let connected = connects.lock().unwrap().clone();
    assert_eq!(connected.len(), 1);
    assert_eq!(
        crate::vcs::real_location(Path::new(&connected[0].1)),
        crate::vcs::real_location(&selected)
    );
}

fn peer_origin() -> remote_tasks::PeerOrigin {
    remote_tasks::PeerOrigin {
        device_id: String::new(),
        server_id: uuid::Uuid::new_v4().to_string(),
        server_label: "Home".into(),
        parent_thread_id: 1,
        task_id: "parent-task-1".into(),
        captain: "Captain".into(),
        memories: Vec::new(),
        result: None,
        acknowledged: false,
        attempt_message: String::new(),
        cancelled_commands: Vec::new(),
        reply_after_position: 0,
        document_reads: false,
    }
}

#[test]
fn peer_launch_provisions_on_the_target_executor() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("peer");
    let (mut target, connects) = fixture.state();
    target.catalog.default_model = Some(crate::session::AgentModel {
        provider: "cpa".into(),
        id: "fixture".into(),
        name: "Fixture".into(),
        thinking_levels: Vec::new(),
        ..Default::default()
    });
    let launch = Command::new(
        "peer.thread.launch",
        json!({
            "projectId": 1, "harness": "pi", "provider": "cpa", "model": "fixture",
            "brief": "Do the delegated work", "title": "Peer worker",
            "origin": serde_json::to_value(peer_origin()).unwrap(),
            "access": "auto",
            "placement": {"mode": "isolated", "baseBranch": "main"}
        }),
    );
    let accepted = target
        .execute_peer_command("fixture-peer", &launch)
        .unwrap();
    let child_id = accepted["threadId"].as_u64().unwrap() as usize;
    assert_eq!(accepted["placement"]["phase"], json!("pending"));
    assert_eq!(accepted["placement"]["executor"], json!("local"));
    // A follow-up while provisioning queues behind the recorded brief.
    let sent = target
        .execute_peer_command(
            "fixture-peer",
            &Command::new(
                "peer.thread.send",
                json!({"threadId": child_id, "message": "Follow-up work"}),
            ),
        )
        .unwrap();
    assert_eq!(sent["queued"], json!(true));
    poll_until_ready(&mut target, 15);
    let index = target
        .threads
        .iter()
        .position(|thread| thread.id == child_id)
        .unwrap();
    let placement = child_placement(&target, index);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(placement.mode, placement::PlacementMode::Isolated);
    assert_eq!(placement.base_branch.as_deref(), Some("main"));
    let checkout = PathBuf::from(placement.checkout.clone().unwrap());
    assert!(checkout.join("README.md").exists());
    assert_eq!(fixture.worktree_dirs().len(), 2);
    // Brief then follow-up, delivered in order once the provider started.
    let injections = &target.threads[index].pending_injections;
    assert_eq!(injections.len(), 2);
    assert!(injections[0].text.contains("Do the delegated work"));
    assert!(injections[1].text.contains("Follow-up work"));
    let connected = connects.lock().unwrap().clone();
    assert_eq!(connected.len(), 1);
    assert_eq!(connected[0].0, child_id);
    assert_eq!(
        crate::vcs::real_location(Path::new(&connected[0].1)),
        crate::vcs::real_location(&checkout)
    );
    // The placement travels with the stored thread record for peer reads.
    let record = target.threads[index].stored_metadata();
    assert_eq!(record["placement"]["checkout"], json!(placement.checkout));
}

#[test]
fn peer_launch_rejects_an_invalid_placement_before_any_work() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("peer-invalid");
    let (mut target, _connects) = fixture.state();
    target.catalog.default_model = Some(crate::session::AgentModel {
        provider: "cpa".into(),
        id: "fixture".into(),
        name: "Fixture".into(),
        thinking_levels: Vec::new(),
        ..Default::default()
    });
    let launch = Command::new(
        "peer.thread.launch",
        json!({
            "projectId": 1, "harness": "pi", "provider": "cpa", "model": "fixture",
            "brief": "Do the delegated work",
            "origin": serde_json::to_value(peer_origin()).unwrap(),
            "placement": {"mode": "existing", "worktree": "missing-branch"}
        }),
    );
    let error = target
        .execute_peer_command("fixture-peer", &launch)
        .unwrap_err();
    assert!(error.contains("No worktree"), "{error}");
    assert_eq!(target.threads.len(), 1);
    assert_eq!(fixture.worktree_dirs().len(), 1);
}
