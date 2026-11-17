//! Real-Git coverage for the delegated-worker placement contract (#20).
//! Provisioned checkouts are actual `git worktree` directories under the
//! state's data dir; providers are the fake `Pi::test_connection`.
use super::*;
use crate::protocol::Command;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
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

    /// A delegating parent on its own data dir, linked to a peer named
    /// `Target` advertising `contracts` and a project entry matching this
    /// fixture's repo by its `origin` remote identity.
    fn parent_state(&self, contracts: &[&str]) -> State {
        let dir = self.root.join("parent-data");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("server-id"), uuid::Uuid::new_v4().to_string()).unwrap();
        let mut state = State::headless(dir);
        state.demo = true;
        state
            .projects
            .entries
            .push(Project::new(1, self.repo.clone()));
        state.projects.next_id = 2;
        state.threads[0].project = Some(1);
        // Automatic scoping needs the caller's repo identity to resolve on
        // the project's normalized path (Project::new canonicalizes it).
        let normalized = state.projects.entries[0].path.clone();
        let deadline = Instant::now() + Duration::from_secs(10);
        while remote_tasks::repository(&normalized).is_none() {
            assert!(Instant::now() < deadline, "repository identity unresolved");
            std::thread::sleep(Duration::from_millis(20));
        }
        state.peers.push(crate::peer::Peer {
            server_id: uuid::Uuid::new_v4().to_string(),
            host: "127.0.0.1:1".into(),
            label: "Target".into(),
            linked_at: 0,
            capabilities: json!({
                "contracts": contracts,
                "projects": [{"id": 1, "path": self.repo.to_string_lossy(),
                              "name": "Repo",
                              "gitRemote": "example.com/team/repo"}],
                "harnesses": [{"id": "pi"}],
                "models": [{"provider": "cpa", "id": "fixture",
                            "reasoning_levels": ["off"]}]}),
            last_reached_at: Some(0),
            last_error: None,
        });
        state
    }

    fn add_origin(&self) {
        self.git(
            &self.repo,
            &[
                "remote",
                "add",
                "origin",
                "https://example.com/team/repo.git",
            ],
        );
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
        state.durable_task_controls = true;
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
fn a_shared_reader_uses_its_parents_dirty_worktree_without_write_access() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("parent-worktree");
    let parent_checkout = fixture.root.join("parent-checkout");
    fixture.git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "-b",
            "parent-branch",
            &parent_checkout.to_string_lossy(),
            "main",
        ],
    );
    std::fs::write(
        parent_checkout.join("README.md"),
        "parent's unsaved changes\n",
    )
    .unwrap();
    let (mut state, connects) = fixture.state();
    state.threads[0].worktree_path = Some(parent_checkout.to_string_lossy().into_owned());
    state.threads[0].branch = Some("parent-branch".into());
    state.threads[0].env_mode = Some(ThreadEnvMode::Worktree);
    state
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Read my current changes", "placement":{"mode":"shared"}
            }),
        )
        .unwrap();
    let placement = child_placement(&state, 1);
    assert_eq!(placement.branch.as_deref(), Some("parent-branch"));
    assert!(!placement.primary_checkout);
    assert!(placement.read_only());
    assert_eq!(state.threads[1].native.access, AccessMode::ReadOnly);
    let connected = connects.lock().unwrap();
    assert_eq!(connected.len(), 1);
    assert_eq!(
        crate::vcs::real_location(Path::new(&connected[0].1)),
        crate::vcs::real_location(&parent_checkout)
    );
    assert_eq!(
        std::fs::read_to_string(Path::new(&connected[0].1).join("README.md")).unwrap(),
        "parent's unsaved changes\n"
    );
    assert_eq!(
        fixture
            .git(&parent_checkout, &["status", "--porcelain"])
            .trim(),
        "M README.md"
    );
    assert_eq!(
        fixture
            .git(&fixture.repo, &["status", "--porcelain"])
            .trim(),
        ""
    );
}

#[test]
fn the_reserved_base_commit_does_not_follow_a_branch_that_moves_before_provisioning() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("pinned-base");
    let (mut state, _) = fixture.state();
    let base = fixture
        .git(&fixture.repo, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    state
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Build from this base", "placement":{"mode":"isolated","baseBranch":"main"}
            }),
        )
        .unwrap();
    assert_eq!(
        child_placement(&state, 1).base_commit.as_deref(),
        Some(base.as_str())
    );
    fixture.git(
        &fixture.repo,
        &["commit", "--allow-empty", "-m", "move main", "--quiet"],
    );
    let next = fixture
        .git(&fixture.repo, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    assert_ne!(base, next);
    poll_until_ready(&mut state, 15);
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(placement.base_commit.as_deref(), Some(base.as_str()));
    let checkout = Path::new(placement.checkout.as_ref().unwrap());
    assert_eq!(fixture.git(checkout, &["rev-parse", "HEAD"]).trim(), base);
}

#[test]
fn remote_upstream_selection_maps_the_home_task_to_its_acknowledged_target() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("upstream-map");
    fixture.add_origin();
    let mut home = fixture.parent_state(&[
        crate::protocol::PEER_TASKS,
        crate::protocol::THREAD_READ,
        crate::protocol::PLACEMENT,
    ]);
    let upstream = home
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Build upstream", "target":{"machine":"Target","model":"fixture"}
            }),
        )
        .unwrap()["thread_id"]
        .as_u64()
        .unwrap() as usize;
    let target_id = 999;
    let remote = home.tasks[0].remote.as_mut().unwrap();
    remote.thread_id = Some(target_id);
    remote.placement.as_mut().unwrap()["phase"] = json!("ready");
    let dependent = home
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Build dependent", "target":{"machine":"Target","model":"fixture"},
                "placement":{"mode":"isolated","upstreamThreadId":upstream}
            }),
        )
        .unwrap();
    assert!(dependent["thread_id"].is_number());
    assert_eq!(
        home.tasks[1].remote.as_ref().unwrap().launch_command.params["placement"]["upstreamThreadId"],
        target_id
    );
    let unrelated = home
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Not a home task", "target":{"machine":"Target","model":"fixture"},
                "placement":{"mode":"isolated","upstreamThreadId":target_id}
            }),
        )
        .unwrap_err();
    assert!(unrelated.contains("same peer project"), "{unrelated}");
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
            &json!({"thread_id": child_id, "message": "continue", "client_request_id":"retry-placement"}),
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
    restored.durable_task_controls = true;
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
fn a_placement_runs_setup_once_without_reprovisioning_during_setup() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("setup-once");
    std::fs::write(
        fixture.repo.join(crate::vcs::T3_PROJECT_FILE_NAME),
        json!({"scripts":[{"name":"setup","command":"exit 0","runOnWorktreeCreate":true}]})
            .to_string(),
    )
    .unwrap();
    fixture.git(&fixture.repo, &["add", crate::vcs::T3_PROJECT_FILE_NAME]);
    fixture.git(&fixture.repo, &["commit", "-m", "setup", "--quiet"]);
    let (mut state, connects) = fixture.state();
    state
        .mcp_call(1, "delegate_task", &json!({"prompt":"Write it"}))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while state.threads[1].worktree_setup.is_none() {
        state.poll_placements();
        assert!(Instant::now() < deadline, "setup never started");
        std::thread::sleep(Duration::from_millis(10));
    }
    // Do not drain setup yet. Its runner may already have finished, but the
    // placement is still pending and setup still owns it until we read that result.
    for _ in 0..3 {
        state.poll_placements();
        assert!(state.placement_rxs.is_empty());
        assert_eq!(state.threads[1].worktree_setup_seq, 1);
    }
    assert!(connects.lock().unwrap().is_empty());
    poll_until_ready(&mut state, 15);
    assert_eq!(
        child_placement(&state, 1).phase,
        placement::PlacementPhase::Ready
    );
    assert_eq!(state.threads[1].worktree_setup_seq, 1);
    assert_eq!(connects.lock().unwrap().len(), 1);
}

#[test]
fn interrupted_placement_setup_waits_for_an_explicit_retry_after_restart() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("interrupted-setup");
    let (mut state, _) = fixture.state();
    state
        .mcp_call(1, "delegate_task", &json!({"prompt":"Write it"}))
        .unwrap();
    state.threads[1].worktree_setup = Some(WorktreeSetupState {
        phase: WorktreeSetupPhase::Running,
        script_name: Some("setup".into()),
        script_command: Some("exit 0".into()),
        error: None,
        output: String::new(),
        exit_code: None,
        timed_out: false,
        duration_ms: 0,
        skipped: false,
    });
    let saved = state.snapshot();
    let (mut restored, connects) = fixture.state();
    restored.restore_global(&saved).unwrap();
    assert_eq!(
        child_placement(&restored, 1).phase,
        placement::PlacementPhase::Failed
    );
    assert!(
        child_placement(&restored, 1)
            .error
            .as_deref()
            .unwrap()
            .contains("interrupted")
    );
    restored.poll_placements();
    assert!(restored.placement_rxs.is_empty());
    assert!(connects.lock().unwrap().is_empty());
    assert_eq!(fixture.worktree_dirs().len(), 1);
}

#[test]
fn a_late_peer_placement_result_cannot_revive_a_cancelled_worker() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("cancel-provisioning");
    std::fs::write(
        fixture.repo.join(crate::vcs::T3_PROJECT_FILE_NAME),
        json!({"scripts":[{"name":"setup","command":"exit 0","runOnWorktreeCreate":true}]})
            .to_string(),
    )
    .unwrap();
    fixture.git(&fixture.repo, &["add", crate::vcs::T3_PROJECT_FILE_NAME]);
    fixture.git(&fixture.repo, &["commit", "-m", "setup", "--quiet"]);
    let (mut target, connects) = fixture.state();
    target.catalog.default_model = Some(crate::session::AgentModel {
        provider: "cpa".into(),
        id: "fixture".into(),
        name: "Fixture".into(),
        ..Default::default()
    });
    let result = target
        .execute_peer_command(
            "parent-device",
            &Command::new(
                "peer.thread.launch",
                json!({
                    "projectId":1, "harness":"pi", "provider":"cpa", "model":"fixture",
                    "brief":"Do not revive this work", "origin":peer_origin(),
                    "placement":{"mode":"isolated"}
                }),
            ),
        )
        .unwrap();
    let child_id = result["threadId"].as_u64().unwrap() as usize;
    target.poll_placements();
    assert_eq!(target.placement_rxs.len(), 1);
    target
        .execute_peer_command(
            "parent-device",
            &Command::new("peer.thread.stop", json!({"threadId":child_id})),
        )
        .unwrap();
    target.reconcile_peer_task_intents();
    assert_eq!(target.threads[1].task_result, Some("cancelled"));
    let deadline = Instant::now() + Duration::from_secs(20);
    while !target.placement_rxs.is_empty() {
        target.poll_placements();
        assert!(Instant::now() < deadline, "provisioning never finished");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        child_placement(&target, 1).phase,
        placement::PlacementPhase::Pending
    );
    assert_eq!(fixture.worktree_dirs().len(), 2);
    assert!(target.threads[1].worktree_setup.is_none());
    assert!(target.threads[1].pending_injections.is_empty());
    assert!(target.threads[1].pi.is_none());
    assert!(connects.lock().unwrap().is_empty());
    target
        .execute_peer_command(
            "parent-device",
            &Command::new(
                "peer.thread.send",
                json!({
                    "threadId":child_id, "message":"Only the new attempt",
                    "delivery":"attempt", "attemptId":"new-attempt"
                }),
            ),
        )
        .unwrap();
    poll_until_ready(&mut target, 20);
    assert_eq!(
        child_placement(&target, 1).phase,
        placement::PlacementPhase::Ready
    );
    assert_eq!(
        target.threads[1].pending_injections.len(),
        1,
        "{:?}",
        target.threads[1]
            .pending_injections
            .iter()
            .map(|injection| injection.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        target.threads[1].pending_injections[0]
            .text
            .contains("Only the new attempt")
    );
    assert!(
        !target.threads[1].pending_injections[0]
            .text
            .contains("Do not revive this work")
    );
    assert_eq!(connects.lock().unwrap().len(), 1);
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
        attempt_id: None,
        captain: "Captain".into(),
        memories: Vec::new(),
        result: None,
        acknowledged: false,
        attempt_message: String::new(),
        cancelled_commands: Vec::new(),
        reply_after_position: 0,
        document_reads: false,
        inputs: Vec::new(),
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
    let sent = target
        .execute_peer_command(
            "fixture-peer",
            &Command::new(
                "peer.thread.send",
                json!({
                    "threadId":child_id, "message":"Second follow-up"
                }),
            ),
        )
        .unwrap();
    assert_eq!(sent["queued"], true);
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
    assert_eq!(injections.len(), 3);
    assert!(injections[0].text.contains("Do the delegated work"));
    assert!(injections[1].text.contains("Follow-up work"));
    assert!(injections[2].text.contains("Second follow-up"));
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

#[test]
fn explicit_isolated_rejects_on_unborn_and_projectless_workspaces() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("reject");
    // Unborn repository: a Git dir with no commit cannot host a worktree.
    let unborn = fixture.root.join("unborn");
    fixture.git(&fixture.root, &["init", "-b", "main", "--quiet", "unborn"]);
    assert!(unborn.join(".git").exists());
    let (mut state, _connects) = fixture.state();
    state.projects.entries.push(Project::new(2, unborn));
    state.threads[0].project = Some(2);
    let error = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt": "Write", "placement": {"mode": "isolated"}}),
        )
        .unwrap_err();
    assert!(error.contains("no commits"), "{error}");
    assert_eq!(state.threads.len(), 1);
    assert!(state.tasks.is_empty());
    // A project-less caller has no workspace; explicit isolated refuses, the
    // writer default alone may fall back to a private scratch directory.
    state.threads[0].project = None;
    let error = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt": "Write", "placement": {"mode": "isolated"}}),
        )
        .unwrap_err();
    assert!(error.contains("needs a project workspace"), "{error}");
    assert_eq!(state.threads.len(), 1);
    state
        .mcp_call(1, "delegate_task", &json!({"prompt": "Write"}))
        .unwrap();
    let placement = child_placement(&state, 1);
    assert_eq!(placement.mode, placement::PlacementMode::Shared);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert!(!placement.primary_checkout);
    assert!(
        placement
            .capability
            .as_deref()
            .unwrap_or("")
            .contains("no project workspace"),
        "{:?}",
        placement.capability
    );
}

#[test]
fn the_default_mode_word_selects_the_writer_default() {
    let spec: placement::PlacementSpec =
        serde_json::from_value(json!({"mode": "default"})).unwrap();
    assert_eq!(spec.mode, None);
    assert_eq!(placement::requested_mode(&spec), None);
    // An explicit "default" with a base is an isolated intent.
    let spec: placement::PlacementSpec =
        serde_json::from_value(json!({"mode": "default", "baseBranch": "main"})).unwrap();
    assert_eq!(
        placement::requested_mode(&spec),
        Some(placement::PlacementMode::Isolated)
    );
    // And it survives end to end in the args parser.
    let spec = placement::spec_from_args(&json!({"placement": {"mode": "default"}})).unwrap();
    assert!(spec.is_some());
}

#[test]
fn provisioning_adopts_a_worktree_left_by_a_lost_attempt() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("adopt");
    let (mut state, connects) = fixture.state();
    state
        .mcp_call(1, "delegate_task", &json!({"prompt": "Write it"}))
        .unwrap();
    let planned = PathBuf::from(child_placement(&state, 1).checkout.clone().unwrap());
    let branch = child_placement(&state, 1).branch.clone().unwrap();
    // Simulate a crash after `git worktree add` but before the outcome was
    // committed: the registered worktree exists at the recorded path.
    fixture.git(
        &fixture.repo,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &planned.to_string_lossy(),
            "main",
        ],
    );
    poll_until_ready(&mut state, 15);
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(
        crate::vcs::real_location(Path::new(placement.checkout.as_ref().unwrap())),
        crate::vcs::real_location(&planned)
    );
    // Adopted, never duplicated: the repo still has exactly two worktrees.
    assert_eq!(fixture.worktree_dirs().len(), 2);
    assert_eq!(connects.lock().unwrap().len(), 1);
}

#[test]
fn old_peer_refuses_isolated_and_keeps_only_an_explicit_shared() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("old-peer");
    fixture.add_origin();
    // A peer that never heard of placement-v1: it would ignore the launch's
    // placement argument and run the worker in its shared writable checkout.
    let mut parent =
        fixture.parent_state(&[crate::protocol::PEER_TASKS, crate::protocol::THREAD_READ]);
    let delegate = |extra: &Value| {
        let mut args = json!({"prompt": "Write it",
            "target": {"machine": "Target", "model": "fixture"}});
        if let Some(obj) = extra.as_object() {
            args.as_object_mut().unwrap().extend(obj.clone());
        }
        args
    };
    // The writer default is an isolation intent: it must refuse on an old
    // peer rather than silently run shared writable.
    let error = parent
        .mcp_call(1, "delegate_task", &delegate(&json!({})))
        .unwrap_err();
    assert!(
        error.contains("Update required") && error.contains("placement-v1"),
        "{error}"
    );
    assert_eq!(parent.threads.len(), 1);
    assert!(parent.tasks.is_empty());
    // Explicit isolated and existing refuse the same way.
    for spec in [
        json!({"mode": "isolated"}),
        json!({"mode": "existing", "worktree": "some-branch"}),
    ] {
        let error = parent
            .mcp_call(1, "delegate_task", &delegate(&json!({"placement": spec})))
            .unwrap_err();
        assert!(error.contains("Update required"), "{error}");
        assert!(parent.tasks.is_empty());
    }
    // An explicit shared reader is the one placement an old peer can honour:
    // it stays shared, goes out read-only, and is recorded with its
    // capability note.
    parent
        .mcp_call(
            1,
            "delegate_task",
            &delegate(&json!({"placement": {"mode": "shared"}})),
        )
        .unwrap();
    let remote = parent.tasks[0].remote.as_ref().unwrap();
    assert_eq!(remote.launch_command.params["access"], json!("read-only"));
    assert!(remote.launch_command.params["placement"].is_null());
    let placement = remote.placement.as_ref().unwrap();
    assert_eq!(placement["mode"], json!("shared"));
    assert_eq!(placement["phase"], json!("ready"));
    assert!(placement["primaryCheckout"] == json!(true));
    assert!(
        placement["capability"]
            .as_str()
            .unwrap_or("")
            .contains("placement-v1"),
        "{placement}"
    );
    std::fs::remove_dir_all(parent.data_dir).unwrap();
}

#[test]
fn two_delegated_writers_on_one_peer_get_distinct_provisioned_worktrees() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("two-server");
    fixture.add_origin();
    // Target executor: a second state on its own data dir that provisions for
    // real through the peer command path.
    let (mut target, connects) = fixture.state();
    target.catalog.default_model = Some(crate::session::AgentModel {
        provider: "cpa".into(),
        id: "fixture".into(),
        name: "Fixture".into(),
        thinking_levels: Vec::new(),
        ..Default::default()
    });
    let mut parent = fixture.parent_state(&[
        crate::protocol::PEER_TASKS,
        crate::protocol::THREAD_READ,
        crate::protocol::PLACEMENT,
    ]);
    // Two writers delegated to the same peer project.
    for brief in ["Write module A", "Write module B"] {
        parent
            .mcp_call(
                1,
                "delegate_task",
                &json!({"prompt": brief,
                    "target": {"machine": "Target", "model": "fixture"}}),
            )
            .unwrap();
    }
    assert_eq!(parent.tasks.len(), 2);
    // Carry each launch to the target and mirror its reply to the parent.
    let mut child_ids = Vec::new();
    for task in &parent.tasks {
        let launch = task.remote.as_ref().unwrap().launch_command.clone();
        assert_eq!(launch.method, "peer.thread.launch");
        let accepted = target
            .execute_peer_command("parent-device", &launch)
            .unwrap();
        assert_eq!(accepted["placement"]["mode"], json!("isolated"));
        assert_eq!(accepted["placement"]["phase"], json!("pending"));
        child_ids.push(accepted["threadId"].as_u64().unwrap() as usize);
        let (send, receive) = mpsc::channel();
        send.send(Ok(json!({"status": "accepted", "result": accepted})))
            .unwrap();
        parent.peer_runtime.inject_pending(
            task.id.clone(),
            task.remote.as_ref().unwrap(),
            "peer.thread.launch".into(),
            receive,
        );
    }
    assert!(parent.poll_remote_tasks());
    poll_until_ready(&mut target, 20);
    // Two distinct provisioned checkouts and branches; neither is the project.
    let mut seen = std::collections::HashSet::new();
    for &child_id in &child_ids {
        let index = target
            .threads
            .iter()
            .position(|t| t.id == child_id)
            .expect("worker thread");
        let placement = target.threads[index].placement.as_ref().unwrap();
        assert_eq!(placement.phase, placement::PlacementPhase::Ready);
        assert_eq!(placement.mode, placement::PlacementMode::Isolated);
        assert!(placement.new_worktree);
        assert!(!placement.primary_checkout);
        let checkout = placement.checkout.clone().unwrap();
        let branch = placement.branch.clone().unwrap();
        assert!(Path::new(&checkout).starts_with(target.data_dir.join("worktrees")));
        assert_ne!(
            crate::vcs::real_location(Path::new(&checkout)),
            crate::vcs::real_location(&fixture.repo)
        );
        assert!(seen.insert(checkout));
        assert!(seen.insert(branch));
    }
    assert_eq!(seen.len(), 4);
    assert_eq!(fixture.worktree_dirs().len(), 3);
    // Each provider started in its own worktree, never the shared checkout.
    let connected = connects.lock().unwrap().clone();
    assert_eq!(connected.len(), 2);
    for &child_id in &child_ids {
        let index = target
            .threads
            .iter()
            .position(|t| t.id == child_id)
            .unwrap();
        let placement = target.threads[index].placement.as_ref().unwrap();
        assert!(connected.iter().any(|(tid, path)| {
            *tid == child_id && Path::new(path) == Path::new(placement.checkout.as_ref().unwrap())
        }));
    }
    // The parent mirrors each resolved placement from the launch replies.
    for task in &parent.tasks {
        let placement = task.remote.as_ref().unwrap().placement.as_ref().unwrap();
        assert_eq!(placement["mode"], json!("isolated"));
        assert!(placement["checkout"].is_string());
    }
    // The project checkout was never touched.
    assert_eq!(
        fixture
            .git(&fixture.repo, &["symbolic-ref", "--short", "HEAD"])
            .trim(),
        "main"
    );
    std::fs::remove_dir_all(parent.data_dir).unwrap();
}

#[test]
fn a_peer_launch_reservation_commits_before_any_provisioning_starts() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("commit-order");
    let mut state = State::headless(fixture.root.join("server-data"));
    std::fs::create_dir_all(&state.data_dir).unwrap();
    state.load_global_for_test();
    state.runtime_enabled = true;
    state.test_git_context = Some(crate::vcs::GitContext {
        git_config_global: Some(fixture.empty_cfg.clone()),
        git_config_system: Some(fixture.empty_cfg.clone()),
    });
    state
        .projects
        .entries
        .push(Project::new(1, fixture.repo.clone()));
    state.projects.next_id = 2;
    state.catalog.default_model = Some(crate::session::AgentModel {
        provider: "cpa".into(),
        id: "fixture".into(),
        name: "Fixture".into(),
        thinking_levels: Vec::new(),
        ..Default::default()
    });
    let mut runtime = crate::runtime::RuntimeState::from_state(state);
    let database = crate::database::Database::open(&runtime.state.data_dir).unwrap();
    database
        .issue_scoped_device("parent", "parent-token", &["read", "peer-delegate"])
        .unwrap();
    runtime.database = Some(database.clone());
    let launch = Command::new(
        "peer.thread.launch",
        json!({
            "projectId": 1, "harness": "pi", "provider": "cpa",
            "model": "fixture", "brief": "Do the delegated work",
            "title": "Worker", "origin": serde_json::to_value(peer_origin()).unwrap(),
        }),
    );
    let receipt = runtime.command("parent", launch).unwrap();
    assert_eq!(receipt["status"], json!("accepted"), "{receipt}");
    let child_id = receipt["result"]["threadId"].as_u64().unwrap() as usize;
    assert_eq!(receipt["result"]["placement"]["phase"], json!("pending"));
    // The command transaction committed the reservation durably.
    let committed = database.read("index.json").unwrap().unwrap();
    let record = committed["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == json!(child_id))
        .expect("committed worker thread");
    assert_eq!(record["placement"]["phase"], json!("pending"));
    assert_eq!(record["placement"]["mode"], json!("isolated"));
    assert!(record["placement"]["checkout"].is_string());
    // And nothing provisioned inside that transaction: the filesystem is
    // untouched until a later committed-state tick drives it.
    assert_eq!(fixture.worktree_dirs().len(), 1);
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        runtime.tick();
        let index = runtime
            .state
            .threads
            .iter()
            .position(|t| t.id == child_id)
            .unwrap();
        let placement = runtime.state.threads[index].placement.as_ref().unwrap();
        if placement.phase != placement::PlacementPhase::Pending {
            break;
        }
        assert!(Instant::now() < deadline, "placement never left pending");
        std::thread::sleep(Duration::from_millis(20));
    }
    let index = runtime
        .state
        .threads
        .iter()
        .position(|t| t.id == child_id)
        .unwrap();
    let placement = runtime.state.threads[index].placement.as_ref().unwrap();
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(fixture.worktree_dirs().len(), 2);
    // The provisioned checkout on disk matches the committed reservation.
    assert_eq!(
        crate::vcs::real_location(Path::new(placement.checkout.as_ref().unwrap())),
        crate::vcs::real_location(Path::new(record["placement"]["checkout"].as_str().unwrap()))
    );
    std::fs::remove_dir_all(runtime.state.data_dir).unwrap();
}

#[test]
fn a_peer_dependent_checkout_uses_the_target_upstream_branch_pinned_at_reservation() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("upstream-target");
    fixture.add_origin();
    let (mut target, connects) = fixture.state();
    target.next_id = 100;
    target.catalog.default_model = Some(crate::session::AgentModel {
        provider: "cpa".into(),
        id: "fixture".into(),
        name: "Fixture".into(),
        thinking_levels: Vec::new(),
        ..Default::default()
    });
    let mut parent = fixture.parent_state(&[
        crate::protocol::PEER_TASKS,
        crate::protocol::THREAD_READ,
        crate::protocol::PLACEMENT,
    ]);
    let upstream = parent
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Build upstream", "target":{"machine":"Target","model":"fixture"}
            }),
        )
        .unwrap()["thread_id"]
        .as_u64()
        .unwrap() as usize;
    let launch = parent.tasks[0]
        .remote
        .as_ref()
        .unwrap()
        .launch_command
        .clone();
    let accepted = target
        .execute_peer_command("parent-device", &launch)
        .unwrap();
    let target_upstream = accepted["threadId"].as_u64().unwrap() as usize;
    assert_ne!(target_upstream, upstream);
    let (send, receive) = mpsc::channel();
    send.send(Ok(json!({"status":"accepted","result":accepted})))
        .unwrap();
    parent.peer_runtime.inject_pending(
        parent.tasks[0].id.clone(),
        parent.tasks[0].remote.as_ref().unwrap(),
        "peer.thread.launch".into(),
        receive,
    );
    assert!(parent.poll_remote_tasks());
    assert_eq!(
        parent.tasks[0]
            .remote
            .as_ref()
            .and_then(|remote| remote.thread_id),
        Some(target_upstream)
    );
    poll_until_ready(&mut target, 20);
    let upstream_index = target
        .threads
        .iter()
        .position(|thread| thread.id == target_upstream)
        .expect("upstream worker");
    let upstream_placement = child_placement(&target, upstream_index).clone();
    assert_eq!(upstream_placement.phase, placement::PlacementPhase::Ready);
    let upstream_branch = upstream_placement.branch.clone().unwrap();
    let upstream_checkout = PathBuf::from(upstream_placement.checkout.as_ref().unwrap());
    std::fs::write(
        upstream_checkout.join("upstream-result.txt"),
        "upstream result\n",
    )
    .unwrap();
    fixture.git(&upstream_checkout, &["add", "upstream-result.txt"]);
    fixture.git(
        &upstream_checkout,
        &["commit", "-m", "upstream result", "--quiet"],
    );
    let upstream_head = fixture
        .git(&upstream_checkout, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    assert_ne!(
        upstream_head,
        upstream_placement.base_commit.as_deref().unwrap(),
        "the selected upstream output must differ from the default base"
    );
    // Carry the executor's ready status home the same way its peer wait does.
    let (send, receive) = mpsc::channel();
    send.send(Ok(json!({
        "status":"accepted",
        "result":{"threadId":target_upstream,"status":"Running",
            "revision":"upstream-ready","placement":upstream_placement.value()}
    })))
    .unwrap();
    parent.peer_runtime.inject_pending(
        parent.tasks[0].id.clone(),
        parent.tasks[0].remote.as_ref().unwrap(),
        "peer.thread.wait".into(),
        receive,
    );
    assert!(parent.poll_remote_tasks());
    assert_eq!(
        parent.tasks[0]
            .remote
            .as_ref()
            .unwrap()
            .placement
            .as_ref()
            .unwrap()["phase"],
        json!("ready")
    );

    let dependent = parent
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Build dependent", "target":{"machine":"Target","model":"fixture"},
                "placement":{"mode":"isolated","upstreamThreadId":upstream}
            }),
        )
        .unwrap()["thread_id"]
        .as_u64()
        .unwrap() as usize;
    assert_ne!(dependent, upstream);
    let dependent_launch = parent.tasks[1]
        .remote
        .as_ref()
        .unwrap()
        .launch_command
        .clone();
    assert_eq!(
        dependent_launch.params["placement"]["upstreamThreadId"],
        json!(target_upstream),
        "the home task id must map to the target's acknowledged worker"
    );
    let accepted = target
        .execute_peer_command("parent-device", &dependent_launch)
        .unwrap();
    let dependent_target_id = accepted["threadId"].as_u64().unwrap() as usize;
    assert_eq!(
        accepted["placement"]["upstreamThreadId"],
        json!(target_upstream)
    );
    assert_eq!(accepted["placement"]["baseBranch"], json!(upstream_branch));
    assert_eq!(accepted["placement"]["baseCommit"], json!(upstream_head));
    assert_eq!(accepted["placement"]["phase"], json!("pending"));
    // The reservation pinned the branch head it saw. Advance that branch
    // afterwards to prove provisioning still starts from the saved commit.
    std::fs::write(upstream_checkout.join("advanced.txt"), "newer upstream\n").unwrap();
    fixture.git(&upstream_checkout, &["add", "advanced.txt"]);
    fixture.git(
        &upstream_checkout,
        &["commit", "-m", "advance upstream", "--quiet"],
    );
    let advanced_head = fixture
        .git(&upstream_checkout, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    assert_ne!(advanced_head, upstream_head);
    let (send, receive) = mpsc::channel();
    send.send(Ok(json!({"status":"accepted","result":accepted})))
        .unwrap();
    parent.peer_runtime.inject_pending(
        parent.tasks[1].id.clone(),
        parent.tasks[1].remote.as_ref().unwrap(),
        "peer.thread.launch".into(),
        receive,
    );
    assert!(parent.poll_remote_tasks());
    poll_until_ready(&mut target, 20);
    let dependent_index = target
        .threads
        .iter()
        .position(|thread| thread.id == dependent_target_id)
        .expect("dependent worker");
    let dependent_placement = child_placement(&target, dependent_index).clone();
    assert_eq!(dependent_placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(dependent_placement.mode, placement::PlacementMode::Isolated);
    assert_eq!(
        dependent_placement.upstream_thread_id,
        Some(target_upstream)
    );
    assert_eq!(
        dependent_placement.base_branch.as_deref(),
        Some(upstream_branch.as_str())
    );
    assert_eq!(
        dependent_placement.base_commit.as_deref(),
        Some(upstream_head.as_str())
    );
    let dependent_checkout = PathBuf::from(dependent_placement.checkout.as_ref().unwrap());
    assert_eq!(
        fixture
            .git(&dependent_checkout, &["rev-parse", "HEAD"])
            .trim(),
        upstream_head,
        "the dependent worktree starts at the pinned base, not the advanced branch"
    );
    assert_eq!(
        std::fs::read_to_string(dependent_checkout.join("upstream-result.txt")).unwrap(),
        "upstream result\n"
    );
    assert!(!dependent_checkout.join("advanced.txt").exists());
    assert_eq!(
        fixture
            .git(
                &fixture.repo,
                &["rev-parse", &format!("refs/heads/{upstream_branch}")]
            )
            .trim(),
        advanced_head,
        "the upstream branch itself still advances"
    );
    let mirrored = parent.tasks[1]
        .remote
        .as_ref()
        .and_then(|remote| remote.placement.as_ref())
        .expect("mirrored dependent placement");
    assert_eq!(mirrored["upstreamThreadId"], json!(target_upstream));
    assert_eq!(mirrored["baseBranch"], json!(upstream_branch));
    assert_eq!(mirrored["baseCommit"], json!(upstream_head));
    assert_eq!(connects.lock().unwrap().len(), 2);
    std::fs::remove_dir_all(parent.data_dir).unwrap();
}

#[test]
fn non_force_cleanup_preserves_worker_work_and_unmerged_branch_commits() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("cleanup");
    let (mut state, _connects) = fixture.state();
    state
        .mcp_call(1, "delegate_task", &json!({"prompt": "Write it"}))
        .unwrap();
    poll_until_ready(&mut state, 15);
    let placement = child_placement(&state, 1).clone();
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    let checkout = PathBuf::from(placement.checkout.as_ref().unwrap());
    let branch = placement.branch.clone().unwrap();
    let ctx = state.test_git_context.clone().unwrap_or_default();
    // A worker commit that exists only on its worker branch.
    std::fs::write(checkout.join("worker.txt"), "worker commit\n").unwrap();
    fixture.git(&checkout, &["add", "worker.txt"]);
    fixture.git(
        &checkout,
        &["commit", "-m", "unmerged worker work", "--quiet"],
    );
    let head = fixture
        .git(&checkout, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    assert!(crate::vcs::branch_exists(&fixture.repo, &branch, &ctx));
    // Dirty tracked, staged new and untracked files, all in one checkout.
    std::fs::write(checkout.join("README.md"), "dirty tracked\n").unwrap();
    std::fs::write(checkout.join("staged.txt"), "staged new\n").unwrap();
    fixture.git(&checkout, &["add", "staged.txt"]);
    std::fs::write(checkout.join("untracked.txt"), "untracked\n").unwrap();
    for input in [
        crate::vcs::VcsRemoveWorktreeInput {
            cwd: fixture.repo.to_string_lossy().into_owned(),
            path: checkout.to_string_lossy().into_owned(),
            force: None,
        },
        crate::vcs::VcsRemoveWorktreeInput {
            cwd: fixture.repo.to_string_lossy().into_owned(),
            path: checkout.to_string_lossy().into_owned(),
            force: Some(false),
        },
    ] {
        let error = crate::vcs::remove_worktree_with(&input, &ctx).unwrap_err();
        assert!(error.contains("GitVcsDriver.removeWorktree"), "{error}");
        assert!(checkout.exists(), "non-force removal deleted the checkout");
        assert_eq!(
            std::fs::read_to_string(checkout.join("README.md")).unwrap(),
            "dirty tracked\n"
        );
        assert_eq!(
            std::fs::read_to_string(checkout.join("staged.txt")).unwrap(),
            "staged new\n"
        );
        assert_eq!(
            std::fs::read_to_string(checkout.join("untracked.txt")).unwrap(),
            "untracked\n"
        );
        assert_eq!(fixture.git(&checkout, &["rev-parse", "HEAD"]).trim(), head);
        assert!(crate::vcs::branch_exists(&fixture.repo, &branch, &ctx));
    }
    // Restore the fixture's own temporary checkout to clean, then prove a
    // non-force removal still keeps the unmerged branch and commit.
    std::fs::write(checkout.join("README.md"), "hello\n").unwrap();
    fixture.git(&checkout, &["reset", "--quiet"]);
    std::fs::remove_file(checkout.join("staged.txt")).unwrap();
    std::fs::remove_file(checkout.join("untracked.txt")).unwrap();
    assert_eq!(
        fixture.git(&checkout, &["status", "--porcelain"]).trim(),
        ""
    );
    crate::vcs::remove_worktree_with(
        &crate::vcs::VcsRemoveWorktreeInput {
            cwd: fixture.repo.to_string_lossy().into_owned(),
            path: checkout.to_string_lossy().into_owned(),
            force: None,
        },
        &ctx,
    )
    .unwrap();
    assert!(!checkout.exists());
    assert!(crate::vcs::branch_exists(&fixture.repo, &branch, &ctx));
    assert_eq!(
        fixture
            .git(
                &fixture.repo,
                &["rev-parse", &format!("refs/heads/{branch}")]
            )
            .trim(),
        head
    );
    assert_eq!(
        fixture
            .git(&fixture.repo, &["show", &format!("{branch}:worker.txt")])
            .trim(),
        "worker commit"
    );
}

#[test]
fn direct_worker_send_does_not_retry_a_failed_reservation_and_keeps_parent_recovery() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("direct-recovery");
    let (mut state, connects) = fixture.state();
    state.headless = true;
    state
        .mcp_call(1, "delegate_task", &json!({"prompt": "Write it"}))
        .unwrap();
    let child = state.threads[1].id;
    let placement = child_placement(&state, 1).clone();
    let planned = PathBuf::from(placement.checkout.as_ref().unwrap());
    let branch = placement.branch.clone().unwrap();
    std::fs::create_dir_all(&planned).unwrap();
    poll_until_ready(&mut state, 10);
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Failed);
    let error = placement.error.clone().unwrap();
    assert!(error.contains("will not delete"), "{error}");
    assert_eq!(state.tasks[0].state, tasks::TaskState::Queued);

    // A direct send to the worker is not the recovery route. It starts
    // nothing, keeps the failed reservation, and says where new work must
    // come from. It also must not leave the worker looking busy.
    let direct = state
        .execute_command(&Command::new(
            "thread.send",
            json!({"threadId":child,"text":"Direct follow-up","attachments":[],"mode":"prompt"}),
        ))
        .unwrap_err();
    assert!(direct.contains("Nothing started"), "{direct}");
    assert!(direct.contains("same checkout"), "{direct}");
    assert_eq!(
        child_placement(&state, 1).phase,
        placement::PlacementPhase::Failed
    );
    assert_eq!(
        child_placement(&state, 1).error.as_deref(),
        Some(error.as_str())
    );
    assert_eq!(state.threads[1].status, Status::Failed);
    assert_eq!(state.tasks[0].state, tasks::TaskState::Queued);
    assert!(state.threads[1].messages.is_empty());
    assert!(state.threads[1].queued_prompt.is_none());
    assert!(state.threads[1].native.queue.is_empty());
    assert!(state.threads[1].pending_injections.is_empty());
    assert!(state.threads[1].pi.is_none());
    assert!(state.placement_rxs.is_empty());
    assert!(connects.lock().unwrap().is_empty());
    assert!(planned.exists());
    assert_eq!(fixture.worktree_dirs().len(), 1);

    // Captain new work is the recovery route and reuses the reservation.
    std::fs::remove_dir_all(&planned).unwrap();
    let sent = state
        .mcp_call(
            1,
            "task_send",
            &json!({"thread_id":child,"message":"continue","client_request_id":"recover-direct"}),
        )
        .unwrap();
    assert_eq!(sent["control"]["saved"], true);
    assert_eq!(
        child_placement(&state, 1).phase,
        placement::PlacementPhase::Pending
    );
    poll_until_ready(&mut state, 15);
    let placement = child_placement(&state, 1);
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(
        placement.checkout.as_deref(),
        Some(planned.to_string_lossy().as_ref())
    );
    assert_eq!(placement.branch.as_deref(), Some(branch.as_str()));
    assert_eq!(connects.lock().unwrap().len(), 1);
    assert_eq!(fixture.worktree_dirs().len(), 2);
}

#[test]
fn captain_crew_send_retries_a_failed_reservation_in_the_same_checkout() {
    if !crate::vcs::git_available() {
        return;
    }
    let fixture = Fixture::new("crew-recovery");
    let (mut state, _connects) = fixture.state();
    state.headless = true;
    state.captains = crate::captains::Captain::presets();
    state.open_captain(0);
    let home = state.captain_home(0).unwrap();
    let model = crate::session::AgentModel {
        provider: "cpa".into(),
        id: "fixture".into(),
        name: "Fixture".into(),
        thinking_levels: Vec::new(),
        ..Default::default()
    };
    state.catalog.default_model = Some(model.clone());
    let home_index = state
        .threads
        .iter()
        .position(|thread| thread.id == home)
        .unwrap();
    state.threads[home_index].models = vec![model];
    state.threads[home_index].set_model("cpa".into(), "fixture".into());
    // Keep the fake provider's command channel alive so a real prompt send
    // does not fail as a dropped test transport.
    let channels: std::sync::Arc<
        std::sync::Mutex<Vec<(usize, mpsc::Receiver<Value>, mpsc::SyncSender<Event>)>>,
    > = std::sync::Arc::default();
    let paths: std::sync::Arc<std::sync::Mutex<Vec<(usize, String)>>> = std::sync::Arc::default();
    let kept = channels.clone();
    let seen = paths.clone();
    state.test_connect_pi = Some(Box::new(move |tid, path| {
        seen.lock()
            .unwrap()
            .push((tid, path.to_string_lossy().into_owned()));
        let (pi, commands, events) = Pi::test_connection();
        kept.lock().unwrap().push((tid, commands, events));
        Some(pi)
    }));
    let database = crate::database::Database::open(&state.data_dir).unwrap();
    database.begin().unwrap();
    let created = state
        .mcp_call(
            home,
            "captain_crew_create",
            &json!({
                "requestId":uuid::Uuid::new_v4(), "prompt":"Crew first input",
                "project":1, "placement":{"mode":"isolated"}
            }),
        )
        .unwrap();
    let crew = created["threadId"].as_u64().unwrap() as usize;
    let index = state
        .threads
        .iter()
        .position(|thread| thread.id == crew)
        .unwrap();
    let placement = state.threads[index].placement.clone().unwrap();
    let planned = PathBuf::from(placement.checkout.as_ref().unwrap());
    let branch = placement.branch.clone().unwrap();
    std::fs::create_dir_all(&planned).unwrap();
    poll_until_ready(&mut state, 10);
    let placement = state.threads[index].placement.as_ref().unwrap();
    assert_eq!(placement.phase, placement::PlacementPhase::Failed);
    assert!(
        placement
            .error
            .as_deref()
            .unwrap()
            .contains("will not delete")
    );
    assert_eq!(state.threads[index].status, Status::Failed);
    assert!(state.threads[index].native.queue_held);
    assert!(state.threads[index].pi.is_none());
    assert!(paths.lock().unwrap().is_empty());
    // The first input stays available for review instead of being lost.
    assert!(state.threads[index].draft.text.contains("Crew first input"));

    // The Captain's send is the recovery route even though the failed
    // placement left the worker's native queue held.
    let sent = state
        .mcp_call(
            home,
            "captain_crew_send",
            &json!({"requestId":uuid::Uuid::new_v4(), "threadId":crew,
                "message":"Retry after fixing the checkout"}),
        )
        .unwrap();
    assert_eq!(sent["saved"], true);
    let placement = state.threads[index].placement.as_ref().unwrap();
    assert_eq!(placement.phase, placement::PlacementPhase::Pending);
    assert!(!state.threads[index].native.queue_held);
    assert_eq!(state.threads[index].native.queue.len(), 1);
    assert!(state.threads[index].pi.is_none());
    std::fs::remove_dir_all(&planned).unwrap();
    poll_until_ready(&mut state, 15);
    let placement = state.threads[index].placement.as_ref().unwrap();
    assert_eq!(placement.phase, placement::PlacementPhase::Ready);
    assert_eq!(
        placement.checkout.as_deref(),
        Some(planned.to_string_lossy().as_ref())
    );
    assert_eq!(placement.branch.as_deref(), Some(branch.as_str()));
    assert!(
        state.threads[index].pi.is_some(),
        "status={:?} error={:?} queue={} pending={}",
        state.threads[index].status,
        state.threads[index].error,
        state.threads[index].native.queue.len(),
        state.threads[index].pending_prompt.is_some()
    );
    let paths = paths.lock().unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(
        crate::vcs::real_location(Path::new(&paths[0].1)),
        crate::vcs::real_location(&planned)
    );
    drop(paths);
    assert_eq!(channels.lock().unwrap().len(), 1);
    assert_eq!(fixture.worktree_dirs().len(), 2);
    assert!(state.threads[index].draft.text.contains("Crew first input"));
}
