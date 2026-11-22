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
        // The peer below advertises only the pi harness and cpa models.
        state.threads[0].harness = "pi".into();
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
        let mut contracts = contracts.to_vec();
        if !contracts.contains(&crate::protocol::EXECUTION_LEASES) {
            contracts.push(crate::protocol::EXECUTION_LEASES);
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
        captain_turn: false,
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
