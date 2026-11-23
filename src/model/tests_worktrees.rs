use super::*;

fn init_temp_git_repo(root: &std::path::Path, empty_config: &std::path::Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let run = |cwd: &std::path::Path, args: &[&str]| {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-c")
            .arg("user.name=Test User")
            .arg("-c")
            .arg("user.email=test@example.com")
            .args(args)
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_GLOBAL", empty_config)
            .env("GIT_CONFIG_SYSTEM", empty_config);
        let out = crate::process::output(&mut cmd, Duration::from_secs(10), 1024 * 1024)
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&repo, &["init", "-b", "main", "--quiet"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    run(&repo, &["add", "README.md"]);
    run(&repo, &["commit", "-m", "initial", "--quiet"]);
    repo
}

#[test]
fn worktree_bootstrap_creates_worktree_starts_harness_and_queues_second_send() {
    if !crate::vcs::git_available() {
        return;
    }
    let root = std::env::temp_dir().join(format!("shina-wt-boot-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let empty_cfg = root.join("empty.gitconfig");
    std::fs::write(&empty_cfg, "").unwrap();
    let repo = init_temp_git_repo(&root, &empty_cfg);
    let git_ctx = crate::vcs::GitContext {
        git_config_global: Some(empty_cfg.clone()),
        git_config_system: Some(empty_cfg.clone()),
    };

    let mut state = State::new(repo.clone(), false);
    state.data_dir = root.join("data");
    std::fs::create_dir_all(&state.data_dir).unwrap();
    state.runtime_enabled = true;
    state.test_git_context = Some(git_ctx);

    let commands_slot: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<Value>>>> =
        std::sync::Arc::default();
    let commands_slot_hook = commands_slot.clone();
    state.test_connect_pi = Some(Box::new(move |_tid, _path| {
        let (pi, commands, _events) = Pi::test_connection();
        *commands_slot_hook.lock().unwrap() = Some(commands);
        Some(pi)
    }));

    {
        let t = state.current_mut();
        t.harness = "claude".into();
        t.model_provider = "claude".into();
        t.env_mode = Some(ThreadEnvMode::Worktree);
        t.base_branch = Some("main".into());
        t.start_from_origin = Some(true);
        t.draft.replace_text("First turn in worktree");
    }
    state.send();
    assert!(state.current().worktree_bootstrapping());
    assert!(state.current().native.queue_held);
    assert_eq!(state.current().native.queue.len(), 1);
    assert!(state.current().draft.text.is_empty());

    // Second send while bootstrap is in progress queues behind the first message.
    state.current_mut().draft.replace_text("Second queued turn");
    state.send();
    assert_eq!(state.current().native.queue.len(), 2);
    assert!(state.current().native.queue_held);

    let deadline = Instant::now() + Duration::from_secs(10);
    while state.current().worktree_bootstrapping() && Instant::now() < deadline {
        state.poll_worktree_bootstraps();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !state.current().worktree_bootstrapping(),
        "bootstrap did not finish in time"
    );

    let wt_path = state
        .current()
        .worktree_path
        .clone()
        .expect("worktreePath recorded");
    assert!(std::path::Path::new(&wt_path).exists());
    assert!(
        state
            .current()
            .branch
            .as_deref()
            .is_some_and(|b| b.starts_with("t3/"))
    );
    assert_eq!(state.current().base_branch.as_deref(), Some("main"));
    assert_eq!(state.current().env_mode, Some(ThreadEnvMode::Worktree));
    assert_eq!(state.test_connect_workspaces.len(), 1);
    assert_eq!(state.test_connect_workspaces[0].1, PathBuf::from(&wt_path));
    assert_eq!(
        state.thread_directory(state.active).unwrap(),
        PathBuf::from(&wt_path)
    );

    // First prompt was released and sent to the harness; second stays queued behind it.
    assert_eq!(state.current().native.queue.len(), 1);
    assert_eq!(
        state.current().native.queue[0].prompt.text,
        "Second queued turn"
    );
    let commands = commands_slot.lock().unwrap().take().expect("pi commands");
    let first_cmd = commands
        .try_iter()
        .find(|c| c["type"] == "prompt")
        .expect("first prompt dispatched");
    assert_eq!(first_cmd["message"], "First turn in worktree");

    // Settling turn 1 releases the queued second turn in the same worktree.
    state.current_mut().apply(json!({
        "type": "response",
        "id": first_cmd["id"],
        "command": "prompt",
        "success": true,
        "data": {}
    }));
    state
        .current_mut()
        .apply(json!({"type": "agent_settled", "aborted": false}));
    assert!(state.current().native.queue.is_empty());
    let second_cmd = commands
        .try_iter()
        .find(|c| c["type"] == "prompt")
        .expect("second prompt dispatched");
    assert_eq!(second_cmd["message"], "Second queued turn");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn worktree_bootstrap_failure_restores_draft_and_records_nothing() {
    if !crate::vcs::git_available() {
        return;
    }
    let root = std::env::temp_dir().join(format!("shina-wt-fail-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let empty_cfg = root.join("empty.gitconfig");
    std::fs::write(&empty_cfg, "").unwrap();
    let repo = init_temp_git_repo(&root, &empty_cfg);
    let git_ctx = crate::vcs::GitContext {
        git_config_global: Some(empty_cfg.clone()),
        git_config_system: Some(empty_cfg.clone()),
    };

    let mut state = State::new(repo, false);
    state.data_dir = root.join("data");
    std::fs::create_dir_all(&state.data_dir).unwrap();
    state.runtime_enabled = true;
    state.test_git_context = Some(git_ctx);

    {
        let t = state.current_mut();
        t.harness = "claude".into();
        t.model_provider = "claude".into();
        t.env_mode = Some(ThreadEnvMode::Worktree);
        t.base_branch = Some("missing-branch-does-not-exist".into());
        t.start_from_origin = Some(false);
        t.draft.replace_text("Keep this draft on failure");
    }
    state.send();
    assert!(state.current().worktree_bootstrapping());

    let deadline = Instant::now() + Duration::from_secs(10);
    while state.current().worktree_bootstrapping() && Instant::now() < deadline {
        state.poll_worktree_bootstraps();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!state.current().worktree_bootstrapping());
    assert_eq!(state.current().draft.text, "Keep this draft on failure");
    assert_eq!(state.current().env_mode, Some(ThreadEnvMode::Worktree));
    assert!(state.current().worktree_path.is_none());
    assert!(state.current().branch.is_none());
    assert!(state.current().native.queue.is_empty());
    assert!(!state.current().native.queue_held);
    assert!(state.test_connect_workspaces.is_empty());
    assert!(
        state
            .current()
            .submission_error
            .as_deref()
            .is_some_and(|e| e.contains("Git command failed")),
        "expected git error, got {:?}",
        state.current().submission_error
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn worktree_setup_script_succeeds_fails_and_retries() {
    if !crate::vcs::git_available() {
        return;
    }

    // 1. Setup script that succeeds holds the first message until setup completes, then runs it.
    let root_ok = std::env::temp_dir().join(format!("shina-setup-ok-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root_ok).unwrap();
    let empty_cfg_ok = root_ok.join("empty.gitconfig");
    std::fs::write(&empty_cfg_ok, "").unwrap();
    let repo_ok = init_temp_git_repo(&root_ok, &empty_cfg_ok);
    let setup_command = if cfg!(windows) {
        "Write-Output line1; Set-Content -LiteralPath setup.marker -Value setup-done"
    } else {
        "echo line1 && echo setup-done > setup.marker"
    };
    std::fs::write(
        repo_ok.join("t3.json"),
        json!({"scripts":[{
            "name":"Setup", "command":setup_command, "runOnWorktreeCreate":true
        }]})
        .to_string(),
    )
    .unwrap();
    {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-c")
            .arg("user.name=Test User")
            .arg("-c")
            .arg("user.email=test@example.com")
            .args(["add", "t3.json"])
            .current_dir(&repo_ok)
            .env("GIT_CONFIG_GLOBAL", &empty_cfg_ok)
            .env("GIT_CONFIG_SYSTEM", &empty_cfg_ok);
        assert!(cmd.status().unwrap().success());
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-c")
            .arg("user.name=Test User")
            .arg("-c")
            .arg("user.email=test@example.com")
            .args(["commit", "-m", "add t3.json", "--quiet"])
            .current_dir(&repo_ok)
            .env("GIT_CONFIG_GLOBAL", &empty_cfg_ok)
            .env("GIT_CONFIG_SYSTEM", &empty_cfg_ok);
        assert!(cmd.status().unwrap().success());
    }

    let mut state_ok = State::new(repo_ok, false);
    state_ok.data_dir = root_ok.join("data");
    std::fs::create_dir_all(&state_ok.data_dir).unwrap();
    state_ok.runtime_enabled = true;
    state_ok.test_git_context = Some(crate::vcs::GitContext {
        git_config_global: Some(empty_cfg_ok.clone()),
        git_config_system: Some(empty_cfg_ok.clone()),
    });
    let cmds_ok: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<Value>>>> =
        std::sync::Arc::default();
    let cmds_ok_hook = cmds_ok.clone();
    state_ok.test_connect_pi = Some(Box::new(move |_tid, _path| {
        let (pi, commands, _events) = Pi::test_connection();
        *cmds_ok_hook.lock().unwrap() = Some(commands);
        Some(pi)
    }));
    {
        let t = state_ok.current_mut();
        t.harness = "claude".into();
        t.model_provider = "claude".into();
        t.env_mode = Some(ThreadEnvMode::Worktree);
        t.base_branch = Some("main".into());
        t.start_from_origin = Some(false);
        t.draft.replace_text("After setup succeeds");
    }
    state_ok.send();

    let deadline = Instant::now() + Duration::from_secs(10);
    while state_ok.current().worktree_bootstrapping() && Instant::now() < deadline {
        state_ok.poll_worktree_bootstraps();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        state_ok.current().worktree_setup.as_ref().map(|s| s.phase),
        Some(WorktreeSetupPhase::Running)
    );
    assert!(state_ok.current().native.queue_held);
    assert!(state_ok.test_connect_workspaces.is_empty());

    let deadline = Instant::now() + Duration::from_secs(10);
    while state_ok
        .current()
        .worktree_setup
        .as_ref()
        .is_some_and(|s| s.phase == WorktreeSetupPhase::Running)
        && Instant::now() < deadline
    {
        state_ok.poll_worktree_setup();
        std::thread::sleep(Duration::from_millis(10));
    }
    let setup = state_ok.current().worktree_setup.as_ref().unwrap();
    assert_eq!(setup.phase, WorktreeSetupPhase::Succeeded);
    assert!(setup.output.contains("line1"));
    assert!(!state_ok.current().native.queue_held);
    let wt_path = state_ok.current().worktree_path.clone().unwrap();
    assert!(std::path::Path::new(&wt_path).join("setup.marker").exists());
    let sent = cmds_ok
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .try_iter()
        .find(|c| c["type"] == "prompt")
        .expect("first message released after setup succeeded");
    assert_eq!(sent["message"], "After setup succeeds");

    // Snapshot + restore preserves setup card state.
    let snap = state_ok.snapshot();
    let mut restored = State::new(".".into(), false);
    restored.restore_global(&snap).unwrap();
    assert_eq!(
        restored.current().worktree_setup.as_ref().map(|s| s.phase),
        Some(WorktreeSetupPhase::Succeeded)
    );
    let _ = std::fs::remove_dir_all(&root_ok);

    // 2. Setup script that fails holds the first message; retry succeeds and releases it.
    let root_fail = std::env::temp_dir().join(format!("shina-setup-fail-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root_fail).unwrap();
    let empty_cfg_fail = root_fail.join("empty.gitconfig");
    std::fs::write(&empty_cfg_fail, "").unwrap();
    let repo_fail = init_temp_git_repo(&root_fail, &empty_cfg_fail);
    let gate_file = root_fail.join("allow_pass");
    let cmd_str = if cfg!(windows) {
        format!(
            "if (Test-Path -LiteralPath '{}') {{ Write-Output retry-ok }} else {{ Write-Output failing-tail; exit 2 }}",
            gate_file.to_string_lossy().replace('\'', "''")
        )
    } else {
        format!(
            "if [ -f '{}' ]; then echo retry-ok; else echo failing-tail; exit 2; fi",
            gate_file.display()
        )
    };
    std::fs::write(
        repo_fail.join("t3.json"),
        json!({
            "scripts": [{
                "name": "Setup",
                "command": cmd_str,
                "runOnWorktreeCreate": true
            }]
        })
        .to_string(),
    )
    .unwrap();
    {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-c")
            .arg("user.name=Test User")
            .arg("-c")
            .arg("user.email=test@example.com")
            .args(["add", "t3.json"])
            .current_dir(&repo_fail)
            .env("GIT_CONFIG_GLOBAL", &empty_cfg_fail)
            .env("GIT_CONFIG_SYSTEM", &empty_cfg_fail);
        assert!(cmd.status().unwrap().success());
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-c")
            .arg("user.name=Test User")
            .arg("-c")
            .arg("user.email=test@example.com")
            .args(["commit", "-m", "add t3.json", "--quiet"])
            .current_dir(&repo_fail)
            .env("GIT_CONFIG_GLOBAL", &empty_cfg_fail)
            .env("GIT_CONFIG_SYSTEM", &empty_cfg_fail);
        assert!(cmd.status().unwrap().success());
    }

    let mut state_fail = State::new(repo_fail, false);
    state_fail.data_dir = root_fail.join("data");
    std::fs::create_dir_all(&state_fail.data_dir).unwrap();
    state_fail.runtime_enabled = true;
    state_fail.test_git_context = Some(crate::vcs::GitContext {
        git_config_global: Some(empty_cfg_fail.clone()),
        git_config_system: Some(empty_cfg_fail.clone()),
    });
    let cmds_fail: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<Value>>>> =
        std::sync::Arc::default();
    let cmds_fail_hook = cmds_fail.clone();
    state_fail.test_connect_pi = Some(Box::new(move |_tid, _path| {
        let (pi, commands, _events) = Pi::test_connection();
        *cmds_fail_hook.lock().unwrap() = Some(commands);
        Some(pi)
    }));
    {
        let t = state_fail.current_mut();
        t.harness = "claude".into();
        t.model_provider = "claude".into();
        t.env_mode = Some(ThreadEnvMode::Worktree);
        t.base_branch = Some("main".into());
        t.start_from_origin = Some(false);
        t.draft.replace_text("After retry succeeds");
    }
    state_fail.send();

    let deadline = Instant::now() + Duration::from_secs(10);
    while state_fail.current().worktree_bootstrapping() && Instant::now() < deadline {
        state_fail.poll_worktree_bootstraps();
        std::thread::sleep(Duration::from_millis(10));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while state_fail
        .current()
        .worktree_setup
        .as_ref()
        .is_some_and(|s| s.phase == WorktreeSetupPhase::Running)
        && Instant::now() < deadline
    {
        state_fail.poll_worktree_setup();
        std::thread::sleep(Duration::from_millis(10));
    }
    let failed = state_fail.current().worktree_setup.as_ref().unwrap();
    assert_eq!(failed.phase, WorktreeSetupPhase::Failed);
    assert_eq!(failed.exit_code, Some(2));
    assert!(failed.output.contains("failing-tail"));
    assert!(state_fail.current().native.queue_held);
    assert!(state_fail.test_connect_workspaces.is_empty());

    // Fix condition and retry: setup succeeds and releases the queued message.
    std::fs::write(&gate_file, "ok").unwrap();
    state_fail.retry_worktree_setup();
    assert_eq!(
        state_fail
            .current()
            .worktree_setup
            .as_ref()
            .map(|s| s.phase),
        Some(WorktreeSetupPhase::Running)
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while state_fail
        .current()
        .worktree_setup
        .as_ref()
        .is_some_and(|s| s.phase == WorktreeSetupPhase::Running)
        && Instant::now() < deadline
    {
        state_fail.poll_worktree_setup();
        std::thread::sleep(Duration::from_millis(10));
    }
    let retried = state_fail.current().worktree_setup.as_ref().unwrap();
    assert_eq!(retried.phase, WorktreeSetupPhase::Succeeded);
    assert!(retried.output.contains("retry-ok"));
    assert!(!state_fail.current().native.queue_held);
    let sent_retry = cmds_fail
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .try_iter()
        .find(|c| c["type"] == "prompt")
        .expect("first message released after retry");
    assert_eq!(sent_retry["message"], "After retry succeeds");

    let _ = std::fs::remove_dir_all(&root_fail);
}

#[test]
fn draft_workspace_mode_and_base_branch_persist_across_thread_switch_and_reload() {
    let dir = std::env::temp_dir().join(format!("shina-draft-persist-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let index_path = dir.join("index.json");
    let mut store = crate::store::Store::new(index_path.clone(), Arc::new(|| {}));

    let mut state = State::new(".".into(), false);
    state.data_dir = dir.clone();
    state.current_mut().messages.push(Message {
        id: "u1".into(),
        role: "You",
        text: "First thread".into(),
        images: vec![],
        context: None,
    });
    state.thread_defaults.default_thread_env_mode = ThreadEnvMode::Worktree;
    state.new_thread();
    assert_eq!(state.threads.len(), 2);
    let t2_idx = state.active;
    assert_eq!(
        state.threads[t2_idx].env_mode,
        Some(ThreadEnvMode::Worktree)
    );
    state.set_draft_base_branch("main".into());
    state.set_draft_start_from_origin(false);

    // Switch away to thread 0 and switch back: draft choices persist.
    state.open_kept_tab(0);
    state.open_kept_tab(t2_idx);
    assert_eq!(state.current().env_mode, Some(ThreadEnvMode::Worktree));
    assert_eq!(state.current().base_branch.as_deref(), Some("main"));
    assert_eq!(state.current().start_from_origin, Some(false));
    assert!(!state.current().effective_start_from_origin());

    // Save to disk and reload from temporary data directory.
    let snapshot = state.snapshot();
    store.save(snapshot);
    store.flush().unwrap();
    let loaded = crate::store::load(&index_path)
        .unwrap()
        .expect("saved index");
    let mut reloaded = State::new(".".into(), false);
    reloaded.data_dir = dir.clone();
    reloaded.restore_global(&loaded).unwrap();
    assert_eq!(
        reloaded.thread_defaults.default_thread_env_mode,
        ThreadEnvMode::Worktree
    );
    let restored_t2 = reloaded
        .threads
        .iter()
        .find(|t| t.id == state.threads[t2_idx].id)
        .expect("restored draft thread");
    assert_eq!(restored_t2.env_mode, Some(ThreadEnvMode::Worktree));
    assert_eq!(restored_t2.base_branch.as_deref(), Some("main"));
    assert_eq!(restored_t2.start_from_origin, Some(false));

    let _ = std::fs::remove_dir_all(&dir);
}

fn setup_multi_model_repo_state(root: &std::path::Path, models: Vec<AgentModel>) -> State {
    let empty_cfg = root.join("empty.gitconfig");
    std::fs::write(&empty_cfg, "").unwrap();
    let repo = init_temp_git_repo(root, &empty_cfg);
    let git_ctx = crate::vcs::GitContext {
        git_config_global: Some(empty_cfg.clone()),
        git_config_system: Some(empty_cfg),
    };
    let mut state = State::new(repo.clone(), false);
    state.data_dir = root.join("data");
    std::fs::create_dir_all(&state.data_dir).unwrap();
    state.runtime_enabled = true;
    state.test_git_context = Some(git_ctx);
    let project_path = state
        .project(state.current().project)
        .unwrap()
        .path
        .to_string_lossy()
        .into_owned();
    state.vcs_branches.insert(
        ("local".into(), project_path),
        VcsBranchCacheEntry {
            is_repo: true,
            loaded: true,
            loading: false,
            error: None,
            refs: vec![crate::vcs::VcsRef {
                name: "main".into(),
                is_remote: Some(false),
                remote_name: None,
                upstream_ref: None,
                current: true,
                is_default: true,
                worktree_path: None,
            }],
        },
    );
    state.catalog.models = models.clone();
    state.catalog.default_model = models.first().cloned();
    state.default_harness = "pi".into();
    state.current_mut().harness = "pi".into();
    state.current_mut().models = models;
    if let Some(first) = state.catalog.models.first().cloned() {
        state.current_mut().set_model(first.provider, first.id);
    }
    state
}

#[test]
fn multi_model_send_three_models_creates_three_worktrees_and_clears_draft() {
    if !crate::vcs::git_available() {
        return;
    }
    let root = std::env::temp_dir().join(format!("shina-mm-three-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let models = vec![
        AgentModel {
            provider: "cpa".into(),
            id: "cpa/claude-sonnet-4-5".into(),
            name: "Claude Sonnet 4.5".into(),
            thinking_levels: vec!["low".into(), "high".into()],
            supports_images: Some(true),
            ..Default::default()
        },
        AgentModel {
            provider: "cpa".into(),
            id: "cpa/gpt-5".into(),
            name: "GPT-5".into(),
            thinking_levels: vec!["medium".into(), "high".into()],
            supports_images: Some(true),
            ..Default::default()
        },
        AgentModel {
            provider: "cpa".into(),
            id: "cpa/gemini-2-5-pro".into(),
            name: "Gemini 2.5 Pro".into(),
            supports_images: Some(true),
            ..Default::default()
        },
    ];
    let mut state = setup_multi_model_repo_state(&root, models);
    let captured: std::sync::Arc<std::sync::Mutex<Vec<(usize, std::sync::mpsc::Receiver<Value>)>>> =
        std::sync::Arc::default();
    let captured_hook = captured.clone();
    state.test_connect_pi = Some(Box::new(move |tid, _path| {
        let (pi, commands, _events) = Pi::test_connection();
        captured_hook.lock().unwrap().push((tid, commands));
        Some(pi)
    }));

    state.set_composer_thinking_sticky("high".into());
    state.toggle_model_in_multi_selection("pi", "cpa/gpt-5");
    state.toggle_model_in_multi_selection("pi", "cpa/gemini-2-5-pro");
    assert_eq!(state.multi_model_selections().len(), 3);

    let img = test_attachment("shared.png", 1);
    state
        .current_mut()
        .draft
        .replace_text("Compare these three models");
    state.current_mut().add_attachment(img.clone()).unwrap();
    let draft_id = state.current().id;

    state.send();

    let deadline = Instant::now() + Duration::from_secs(10);
    while state.threads.iter().any(|t| t.worktree_bootstrapping()) && Instant::now() < deadline {
        state.poll_worktree_bootstraps();
        std::thread::sleep(Duration::from_millis(10));
    }

    // User stays on the draft, and the draft + multi-selection are cleared.
    assert_eq!(state.current().id, draft_id);
    assert!(state.current().draft.text.is_empty());
    assert!(state.current().draft_images.is_empty());
    assert!(
        state
            .current()
            .draft_model_state
            .additional_models
            .is_empty()
    );
    assert_eq!(state.threads.len(), 4);

    let started: Vec<&Thread> = state.threads.iter().filter(|t| t.id != draft_id).collect();
    assert_eq!(started.len(), 3);
    let worktrees: std::collections::HashSet<&str> = started
        .iter()
        .map(|t| t.worktree_path.as_deref().expect("worktree path"))
        .collect();
    let branches: std::collections::HashSet<&str> = started
        .iter()
        .map(|t| t.branch.as_deref().expect("branch"))
        .collect();
    assert_eq!(worktrees.len(), 3);
    assert_eq!(branches.len(), 3);
    for wt in &worktrees {
        assert!(std::path::Path::new(wt).exists(), "missing worktree {wt}");
    }
    for b in &branches {
        assert!(b.starts_with("t3/"), "unexpected branch {b}");
    }
    assert_eq!(
        started
            .iter()
            .map(|t| t.model_id.as_str())
            .collect::<Vec<_>>(),
        vec!["cpa/claude-sonnet-4-5", "cpa/gpt-5", "cpa/gemini-2-5-pro"]
    );
    for t in &started {
        assert_eq!(t.title, "Compare these three models");
        assert_eq!(t.base_branch.as_deref(), Some("main"));
        assert_eq!(t.env_mode, Some(ThreadEnvMode::Worktree));
        assert_eq!(t.messages.len(), 1);
        assert_eq!(t.messages[0].text, "Compare these three models");
        assert_eq!(t.messages[0].images, vec![img.clone()]);
    }

    assert_eq!(state.toasts.len(), 1);
    assert_eq!(state.toasts[0].kind, ToastKind::Success);
    assert_eq!(state.toasts[0].title, "Started 3 threads in background");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn multi_model_send_partial_failure_keeps_two_threads_restores_failed_selection_and_prompt() {
    if !crate::vcs::git_available() {
        return;
    }
    let root = std::env::temp_dir().join(format!("shina-mm-partial-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let models = vec![
        AgentModel {
            provider: "cpa".into(),
            id: "cpa/claude-sonnet-4-5".into(),
            name: "Claude Sonnet 4.5".into(),
            supports_images: Some(true),
            ..Default::default()
        },
        AgentModel {
            provider: "cpa".into(),
            id: "cpa/text-only".into(),
            name: "Text Only Model".into(),
            supports_images: Some(false),
            ..Default::default()
        },
        AgentModel {
            provider: "cpa".into(),
            id: "cpa/gemini-2-5-pro".into(),
            name: "Gemini 2.5 Pro".into(),
            supports_images: Some(true),
            ..Default::default()
        },
    ];
    let mut state = setup_multi_model_repo_state(&root, models);
    let kept_channels: std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Receiver<Value>>>> =
        std::sync::Arc::default();
    let kept_hook = kept_channels.clone();
    state.test_connect_pi = Some(Box::new(move |_tid, _path| {
        let (pi, commands, _events) = Pi::test_connection();
        kept_hook.lock().unwrap().push(commands);
        Some(pi)
    }));

    state.toggle_model_in_multi_selection("pi", "cpa/text-only");
    state.toggle_model_in_multi_selection("pi", "cpa/gemini-2-5-pro");
    let img = test_attachment("keep-me.png", 1);
    state
        .current_mut()
        .draft
        .replace_text("Inspect this screenshot");
    state.current_mut().add_attachment(img.clone()).unwrap();
    let draft_id = state.current().id;

    state.send();

    let deadline = Instant::now() + Duration::from_secs(10);
    while state.threads.iter().any(|t| t.worktree_bootstrapping()) && Instant::now() < deadline {
        state.poll_worktree_bootstraps();
        std::thread::sleep(Duration::from_millis(10));
    }

    // Two started threads remain (draft + 2 started = 3 threads total).
    let started: Vec<&Thread> = state.threads.iter().filter(|t| t.id != draft_id).collect();
    assert_eq!(started.len(), 2);
    assert_eq!(
        started
            .iter()
            .map(|t| t.model_id.as_str())
            .collect::<Vec<_>>(),
        vec!["cpa/claude-sonnet-4-5", "cpa/gemini-2-5-pro"]
    );
    for t in &started {
        assert_eq!(t.messages[0].images, vec![img.clone()]);
    }

    // Failed middle selection stays selected and prompt + attachment are restored on draft.
    assert_eq!(state.current().id, draft_id);
    assert_eq!(state.current().draft.text, "Inspect this screenshot");
    assert_eq!(state.current().draft_images, vec![img]);
    assert_eq!(
        state
            .multi_model_selections()
            .iter()
            .map(|s| s.model.as_str())
            .collect::<Vec<_>>(),
        vec!["cpa/text-only"]
    );

    // One notification names the failed model.
    assert_eq!(state.toasts.len(), 1);
    assert_eq!(state.toasts[0].kind, ToastKind::Error);
    assert_eq!(state.toasts[0].title, "Could not start Text Only Model");

    let _ = std::fs::remove_dir_all(&root);
}

// `rightPanel.toggle`: a plain thread opens on Work, its first tab; an open panel hides
// and re-shows keeping its tab (KSG `toggleRightPanel`).
#[test]
fn right_panel_toggle_opens_work_then_hides_and_reshows() {
    let mut state = State::new(".".into(), false);

    // Empty thread: opens the Inspector on its default tab.
    state.right_panel_toggle();
    let panel = state.right_panel();
    assert!(panel.open);
    assert_eq!(panel.view, crate::right_panel::PanelView::Work);

    // Open panel: hides, keeping the tab selection.
    state.right_panel_toggle();
    let hidden = state.right_panel();
    assert!(!hidden.open);
    assert_eq!(hidden.view, crate::right_panel::PanelView::Work);

    // Hidden: re-shows on the same tab.
    state.right_panel_toggle();
    let shown = state.right_panel();
    assert!(shown.open);
    assert_eq!(shown.view, crate::right_panel::PanelView::Work);

    // Open on a document: hides too.
    let key = state.panel_key();
    state
        .right_panels
        .open_captain_document(&key, 0, "NOTES.md");
    state.right_panel_toggle();
    let hidden = state.right_panel();
    assert!(!hidden.open);
    assert_eq!(hidden.view, crate::right_panel::PanelView::Context);
}

// `diff.toggle` on the Context tab shows Diff without hiding; on Diff it hides.

// `rightPanel.toggleMaximized` flips a maximized flag, but only on an open panel
// (KSG's `canMaximizeRightPanel`).

// `rightPanel.close` closes the open viewer first, then the panel.

// Two threads keep separate panel state: machine plus thread id is the key.
#[test]
fn right_panel_state_is_per_thread() {
    let mut state = State::new(".".into(), false);
    let first_key = state.panel_key();
    state.right_panel_toggle();
    state.right_panel_toggle_maximized();

    // A second thread with its own key starts closed and empty.
    let other = Thread::new(state.current().id + 1);
    state.threads.push(other);
    state.open_kept_tab(1);
    let second_key = state.panel_key();
    assert_ne!(first_key, second_key);
    assert_eq!(
        state.right_panels.get(&second_key),
        crate::right_panel::ThreadPanel::default()
    );

    state.right_panel_toggle();
    let key = state.panel_key();
    state
        .right_panels
        .open_captain_document(&key, 0, "NOTES.md");

    // Back on the first thread its panel is still open and maximized.
    state.open_kept_tab(0);
    let panel = state.right_panel();
    assert!(panel.open);
    assert!(panel.maximized);
    assert_eq!(panel.view, crate::right_panel::PanelView::Work);

    // Deleting a thread forgets its panel state.
    let mut ids = std::collections::HashSet::new();
    ids.insert(state.threads[1].id);
    state.remove_threads(&ids);
    assert_eq!(
        state.right_panels.get(&second_key),
        crate::right_panel::ThreadPanel::default()
    );
}
