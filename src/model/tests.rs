use super::*;

#[test]
fn nav_history_walks_back_and_forward_and_a_new_open_drops_the_forward_entries() {
    let mut state = State::new(PathBuf::new(), false);
    state.tabs_enabled = false;
    let first = state.current().id;
    // Open two more real threads: a blank draft's "new thread" reuses the
    // same slot, so these push their own ids.
    let mut ids = vec![first];
    for title in ["second thread", "third thread"] {
        let mut thread = Thread::new(state.next_id);
        state.next_id += 1;
        thread.title = title.into();
        state.threads.push(thread);
        let index = state.threads.len() - 1;
        state.show_thread(index);
        ids.push(state.threads[index].id);
    }
    let second = ids[1];
    let third = ids[2];
    assert_eq!(state.current().id, third);
    state.nav_back();
    assert_eq!(state.current().id, second);
    state.nav_back();
    assert_eq!(state.current().id, first);
    assert!(state.nav_history.back().is_none());
    state.nav_forward();
    assert_eq!(state.current().id, second);
    state.nav_forward();
    assert_eq!(state.current().id, third);
    assert!(state.nav_history.forward().is_none());
    // Back once, then a fresh open drops the third thread's entry.
    state.nav_back();
    let mut thread = Thread::new(state.next_id);
    state.next_id += 1;
    thread.title = "fourth thread".into();
    state.threads.push(thread);
    let index = state.threads.len() - 1;
    state.show_thread(index);
    assert!(state.nav_history.forward().is_none());
    assert_ne!(state.current().id, third);
}

#[test]
fn message_text_drops_terminal_escape_sequences() {
    let content = json!(
        "\u{1b}[38;2;144;144;144m✻ Worked for 6s\u{1b}[0m \u{1b}]8;;http://x\u{7}link\u{1b}]8;;\u{1b}\\ end"
    );
    assert_eq!(content_text(&content), "✻ Worked for 6s link end");
    assert_eq!(content_text(&json!("plain [0m text")), "plain [0m text");
}

#[test]
fn restore_rejects_invalid_relationships_and_wrong_typed_session_fields() {
    let state = State::new(PathBuf::new(), false);
    let baseline = state.snapshot();
    for (field, value) in [
        ("captainHome", json!(true)),
        ("delegated", json!(true)),
        ("session", json!(123)),
        ("model", json!(false)),
        ("thinking", json!(4)),
        ("updated", json!("bad")),
    ] {
        let mut index = baseline.clone();
        index["threads"][0][field] = value;
        let mut restored = State::new(PathBuf::new(), false);
        assert!(restored.restore_global(&index).is_err(), "{field}");
    }
}

#[test]
fn diagnostics_are_bounded_redacted_and_truncated_at_ingestion() {
    let mut thread = Thread::new(1);
    for _ in 0..1000 {
        thread.diagnostic(&format!("ghp_secret {}", "x".repeat(9000)));
    }
    assert_eq!(thread.diagnostics.len(), 128);
    assert!(
        thread
            .diagnostics
            .iter()
            .all(|line| line.len() < 2100 && !line.contains("ghp_secret"))
    );
}

#[test]
fn editing_sidebar_search_clears_old_hits_and_rejects_a_late_reply() {
    let mut state = State::headless(crate::store::directory().unwrap());
    state.client = Some(crate::client::Client::new(state.data_dir.clone()));
    state.current_mut().title = "Zebra notes".into();
    state.search.replace_text("fix");
    state.sync_sidebar_search_input();
    let old_seq = state.sidebar_search.request_seq;
    state.sidebar_search.settled_query = "fix".into();
    state.sidebar_search.pending_servers.insert("local".into());
    let hit = crate::thread_search::ThreadSearchHit {
        thread_id: state.current().id,
        project_id: None,
        entry_id: "old".into(),
        source: "user".into(),
        snippet: "fix".into(),
        quality: 1.0,
        hits: 1,
        updated: 0,
    };
    state.apply_sidebar_search_result(old_seq, "local", Ok(vec![hit.clone()]));
    assert_eq!(state.sidebar_search.visible_query, "fix");

    state.search.replace_text("Zebra");
    assert!(state.sync_sidebar_search_input());
    assert!(state.sidebar_search.pending);
    assert!(state.thread_title_search_matches(state.current()));
    assert!(state.sidebar_search.visible_matches.is_empty());
    assert!(state.sidebar_search.shown_order.is_empty());
    assert!(state.sidebar_search.visible_query.is_empty());
    state.apply_sidebar_search_result(old_seq, "local", Ok(vec![hit.clone()]));
    assert!(state.sidebar_search.visible_matches.is_empty());

    state.search.replace_text("");
    state.apply_sidebar_search_result(old_seq, "local", Ok(vec![hit]));
    assert!(!state.sidebar_search.pending);
    assert!(state.sidebar_search.visible_query.is_empty());
}

#[test]
fn all_captain_homes_restore_without_demoting_or_settling_and_keep_their_crew() {
    let mut old = State::new(".".into(), false);
    // Rosters start empty; this test restores homes for the earlier named presets.
    old.captains = crate::captains::Captain::presets();
    // Text keeps the first thread's tab, so it is saved and restored with the homes.
    old.current_mut().draft.replace_text("Saved draft");
    for captain in 0..4 {
        old.open_captain(captain);
    }
    let homes: Vec<_> = old
        .threads
        .iter()
        .filter(|t| t.captain_home)
        .map(|t| t.id)
        .collect();
    for id in homes {
        let home = old.threads.iter_mut().find(|t| t.id == id).unwrap();
        home.session = Some(format!("original-home-{id}.jsonl"));
        let mut crew = Thread::new(old.next_id);
        old.next_id += 1;
        crew.project = home.project;
        crew.captain = home.captain;
        crew.crew_captain = home.captain;
        crew.parent = Some(id);
        crew.delegated = true;
        crew.session = Some(format!("original-crew-{id}.jsonl"));
        old.threads.push(crew);
    }
    let mut index = old.snapshot();
    index.as_object_mut().unwrap().remove("identity");
    index["version"] = 3.into();
    for (field, uid) in [
        ("captains", "captainUid"),
        ("projects", "checkoutUid"),
        ("threads", "threadUid"),
    ] {
        for record in index[field].as_array_mut().into_iter().flatten() {
            record.as_object_mut().unwrap().remove(uid);
        }
    }
    for t in index["threads"].as_array_mut().unwrap() {
        t.as_object_mut().unwrap().remove("lifecycle");
        t["pinned"] = true.into();
        t["unread"] = true.into();
    }
    let mut restored = State::new(PathBuf::new(), false);
    restored.restore_global(&index).unwrap();
    assert_eq!(restored.threads.len(), old.threads.len());
    assert_eq!(
        restored.threads.iter().filter(|t| t.captain_home).count(),
        4
    );
    for (before, after) in old.threads.iter().zip(&restored.threads) {
        assert_eq!(
            (before.id, &before.session, before.parent),
            (after.id, &after.session, after.parent)
        );
        if after.captain_home {
            assert!(!after.life.settled());
            assert!(
                !restored.sidebar_threads().contains(
                    &restored
                        .threads
                        .iter()
                        .position(|t| t.id == after.id)
                        .unwrap()
                )
            );
            assert!(!after.pinned());
        }
    }
    restored.search.replace_text("Pip");
    assert!(restored.visible_sidebar_threads().is_empty());
    restored.sweep_lifecycle(lifecycle::now() + 100 * 86_400_000);
    assert!(
        restored
            .threads
            .iter()
            .filter(|t| t.captain_home)
            .all(|t| !t.life.settled())
    );
    // Opening an existing captain ignores a workspace hint. Scope changes are explicit.
    let original_home = restored.captain_home(0);
    restored.captain_project = Some(None);
    restored.open_captain(0);
    assert_eq!(Some(restored.current().id), original_home);
    restored.set_captain_scope(0, vec![]).unwrap();
    assert!(restored.current().project.is_none());
    let mut again = State::new(PathBuf::new(), false);
    again.restore_global(&restored.snapshot()).unwrap();
}

#[test]
fn undo_restores_unpin_snooze_archive_and_discarded_draft() {
    let mut state = State::new(".".into(), false);
    let at = lifecycle::now();
    state.current_mut().life.pin(at, Some("nb".into()));
    state.current_mut().life.snooze(at, at + 3_600_000);
    state.pin_thread(1);
    state.undo_thread_action();
    assert_eq!(state.threads[0].life.pin_order_key.as_deref(), Some("nb"));
    assert_eq!(state.threads[0].life.snoozed_until, Some(at + 3_600_000));
    state.archive_thread(1);
    assert!(state.threads[0].archived());
    state.undo_thread_action();
    assert!(!state.threads[0].archived());
    assert_eq!(state.threads[0].life.pin_order_key.as_deref(), Some("nb"));
    let draft = state.current().id;
    state.current_mut().draft.replace_text("Keep the draft");
    state.discard_draft(draft);
    state.undo_thread_action();
    assert_eq!(
        state
            .threads
            .iter()
            .find(|t| t.id == draft)
            .unwrap()
            .draft
            .text,
        "Keep the draft"
    );
}

#[test]
fn crew_questions_reach_the_coordinator_once_and_task_send_answers_without_approving() {
    let mut state = State::new(".".into(), false);
    let (parent_commands, _parent_events) = fake_pi(&mut state.threads[0], Status::Ready);
    let (index, child_commands, events) = child(&mut state);
    let request = json!({"type":"extension_ui_request","id":"question","method":"select","title":"Which file?","options":["a.rs","b.rs"]});
    events.send(Event::Record(request.clone())).unwrap();
    events.send(Event::Record(request)).unwrap();
    poll(&mut state, 0.0, &mut Tx::default());
    assert_eq!(state.threads[0].pending_injections.len(), 1);
    assert!(
        parent_commands.try_recv().unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("crew #2 asks")
    );
    assert!(!state.row_pending(&state.threads[0]));
    let generation = state.threads[index].request_generation.clone();
    assert!(
        state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"invalid","client_request_id":"select-invalid",
                    "request_id":"question","runtime_generation":generation})
            )
            .is_err()
    );
    state
        .mcp_call(
            1,
            "task_send",
            &json!({"thread_id":2,"message":"b.rs","client_request_id":"select-b",
                "request_id":"question","runtime_generation":generation}),
        )
        .unwrap();
    let response = child_commands.try_recv().unwrap();
    assert_eq!(response["type"], "extension_ui_response");
    assert_eq!(response["value"], "b.rs");
    assert!(state.threads[index].request.is_none());
    state.threads[index].apply(json!({"type":"extension_ui_request","id":"approval","method":"confirm","title":"Run tool?"}));
    state
        .mcp_call(
            1,
            "task_send",
            &json!({"thread_id":2,"message":"yes","client_request_id":"confirm-yes",
                "request_id":"approval","runtime_generation":generation}),
        )
        .unwrap_err();
    assert!(
        state.threads[index].request.is_some(),
        "a coordinator cannot approve for the user"
    );
    assert!(
        child_commands
            .try_iter()
            .all(|r| r["type"] != "extension_ui_response")
    );
}

#[test]
fn blank_drafts_never_hold_a_sidebar_row_or_accumulate() {
    let mut state = State::new(".".into(), false);
    let workspace = state.data_dir.join("blank-drafts");
    state.projects.entries.push(Project::new(2, workspace));
    assert!(state.sidebar_threads().is_empty());
    for _ in 0..3 {
        state.new_thread();
    }
    state.new_thread_in(Some(2));
    state.new_thread_in(None);
    assert_eq!(
        state.threads.len(),
        3,
        "one blank draft per project, including no project"
    );
    assert!(state.sidebar_threads().is_empty(), "no empty Drafts rows");
    state.current_mut().draft.replace_text("A first prompt");
    let active = state.active;
    assert!(
        state.sidebar_threads().is_empty(),
        "the open draft's row does not pop in while typing"
    );
    state.open_kept_tab((active + 1) % state.threads.len());
    assert_eq!(state.sidebar_threads(), vec![active]);
    state.open_kept_tab(active);
    state.current_mut().draft.replace_text("");
    assert_eq!(
        state.sidebar_threads(),
        vec![active],
        "the open draft keeps its row while its composer is cleared"
    );
    state.discard_draft(state.current().id);
    assert_eq!(state.threads.len(), 2, "the blank thread is gone");
    assert!(
        state.undo.is_some(),
        "discarding a blank draft can restore its tab"
    );
    assert!(state.sidebar_threads().is_empty());
    state.show_archive = true;
    assert!(state.sidebar_threads().is_empty(), "nothing was archived");
    assert!(state.tabs.contains(&state.current().id));
}

fn tab_fixture() -> State {
    let mut state = State::new(".".into(), false);
    state.threads = (1..=5)
        .map(|id| {
            let mut thread = Thread::new(id);
            thread.title = format!("Thread {id}");
            thread.renamed = true;
            thread.project = Some(1);
            thread
        })
        .collect();
    state.next_id = 6;
    state.tabs = vec![1, 2, 3, 4];
    state.active = 1;
    state
}

#[test]
fn sidebar_clicks_reuse_one_ghost_tab_instead_of_piling_up() {
    let mut state = tab_fixture();
    // Ten untabbed threads the user clicks through in the sidebar.
    for n in 0..10 {
        let mut thread = Thread::new(10 + n);
        thread.title = format!("Sidebar thread {n}");
        state.threads.push(thread);
    }
    // The fixture's four tabs are kept; a fresh navigation opens a ghost.
    state.show_thread(4);
    assert_eq!(state.tabs, [1, 2, 5, 3, 4]);
    assert_eq!(state.ghost_tab, Some(5));
    // Clicking further threads replaces the ghost in place, so the strip
    // stays the same size no matter how many rows the user clicks through.
    for index in 5..15 {
        state.show_thread(index);
        assert_eq!(state.tabs.len(), 5, "click {index}");
        assert_eq!(state.ghost_tab, Some(state.threads[index].id));
    }
    // An already-tabbed thread takes focus while the ghost slot keeps its
    // last preview.
    state.show_thread(0);
    assert_eq!(state.ghost_tab, Some(19));
    assert_eq!(state.current().id, 1);
    // A double-click or "Keep tab" makes the ghost permanent. The ghost
    // slot now names the tab the user most recently previewed.
    state.keep_tab(19);
    assert_eq!(state.ghost_tab, None);
    state.show_thread(4);
    // Thread 5's slot went to the last preview, so revisiting it opens a
    // fresh ghost beside the active tab.
    assert_eq!(state.tabs, [1, 5, 2, 19, 3, 4]);
    assert_eq!(state.ghost_tab, Some(5));
    assert_eq!(state.current().id, 5);
}

#[test]
fn closing_middle_rightmost_and_inactive_tabs_uses_neighbours() {
    let mut state = tab_fixture();
    state.close_tab(2);
    assert_eq!(state.current().id, 3);
    assert_eq!(state.tabs, [1, 3, 4]);
    state.close_tab(1);
    assert_eq!(state.current().id, 3);
    state.open_kept_tab(3);
    state.close_tab(4);
    assert_eq!(state.current().id, 3);
    assert_eq!(state.tabs, [3]);
    state.close_tab(3);
    assert_eq!(state.tabs, [3]);
}

#[test]
fn new_thread_inherits_valid_composer_selection_and_rejects_stale_models() {
    let mut state = tab_fixture();
    let model = AgentModel {
        provider: "codex".into(),
        id: "chosen".into(),
        name: "Chosen".into(),
        thinking_levels: vec!["high".into()],
        context_window: Some(100),
        supports_images: Some(true),
        service_tiers: vec![],
        ..Default::default()
    };
    state.current_mut().harness = "codex".into();
    state.current_mut().models = vec![model];
    state.current_mut().selected_model = Some(("codex".into(), "chosen".into()));
    state.current_mut().selected_thinking = Some("high".into());
    state.new_thread_in(Some(1));
    let draft = state.current().id;
    assert_eq!(state.tabs, [1, 2, draft, 3, 4]);
    assert_eq!(state.current().harness, "codex");
    assert_eq!(
        state.current().selected_model,
        Some(("codex".into(), "chosen".into()))
    );
    assert_eq!(state.current().selected_thinking.as_deref(), Some("high"));
    state.current_mut().draft.replace_text("Occupied");
    state.current_mut().selected_model = Some(("codex".into(), "removed".into()));
    state.new_thread_in(Some(1));
    assert_eq!(state.current().harness, state.default_harness);
    assert_eq!(state.current().selected_model, None);
}

#[test]
fn discarding_a_draft_with_content_archives_it_and_undo_restores_it() {
    let mut state = State::new(".".into(), false);
    state.current_mut().draft.replace_text("Keep me");
    state
        .current_mut()
        .draft_images
        .push(test_attachment("kept.png", 1));
    assert_eq!(state.sidebar_threads(), vec![0]);
    state.discard_draft(1);
    assert!(
        state.threads.iter().any(|t| t.id == 1 && t.archived()),
        "a written draft stays as an archive record"
    );
    assert!(state.threads[0].archived());
    assert!(state.undo.is_some(), "a draft with content offers Undo");
    assert!(state.sidebar_threads().is_empty());
    assert!(state.current().draft_images.is_empty());
    state.undo_thread_action();
    assert!(!state.threads[0].archived());
    assert_eq!(state.threads[0].draft.text, "Keep me");
    assert_eq!(state.threads[0].draft_images.len(), 1);
    assert_eq!(state.sidebar_threads(), vec![0]);
}

#[test]
fn multi_project_scope_persists_and_opening_a_hidden_thread_widens_it() {
    let mut state = State::new(".".into(), false);
    state
        .projects
        .entries
        .push(Project::new(2, state.data_dir.join("other-project")));
    state.current_mut().draft.replace_text("Work in one");
    state.new_thread_in(Some(2));
    state.current_mut().draft.replace_text("Work in two");
    state.new_thread_in(None);
    state.set_project_scope(Some(1), false);
    state.set_project_scope(Some(2), true);
    assert_eq!(state.sidebar_threads().len(), 2);
    let mut restored = State::new(PathBuf::new(), false);
    restored.restore_global(&state.snapshot()).unwrap();
    restored.restore_ui(&state.ui_snapshot()).unwrap();
    assert_eq!(restored.sidebar_threads().len(), 2);
    restored.open_kept_tab(2);
    assert_eq!(restored.project_scope, ProjectScope::All);
    assert!(restored.scope_additional.is_empty());
    for (i, t) in restored.threads.iter_mut().enumerate() {
        t.life.created_at = (3 - i) as u64;
    }
    restored.click_thread(
        0,
        rune::Mods {
            cmd: true,
            ..Default::default()
        },
    );
    restored.click_thread(
        1,
        rune::Mods {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(restored.selected_threads.len(), 2);
}

use crate::update::test_exit_guard as exit_record_guard;

/// A verified update, as the updater would have prepared it.
fn prepared(tag: &str) -> crate::update::PreparedUpdate {
    let root = crate::store::directory().unwrap().join("prepared");
    crate::update::PreparedUpdate {
        tag: tag.into(),
        target: crate::install::InstalledTarget::Windows {
            exe: root.join("shina.exe"),
        },
        asset: "shina.exe".into(),
        size: 1,
        sha256: "0".repeat(64),
        download: root.join("download.exe"),
        prepared: root.join("prepared.exe"),
    }
}

fn ready_state() -> State {
    let mut state = State::new(PathBuf::new(), false);
    state.runtime_enabled = true;
    state
}

#[test]
fn running_waiting_queued_and_cloning_threads_block_an_update_restart() {
    let mut state = ready_state();
    assert_eq!(state.update_restart_blocker(), None);
    for status in [Status::Running, Status::Waiting] {
        state.threads[0].status = status;
        assert!(
            state.update_restart_blocker().is_some(),
            "{status:?} blocks"
        );
    }
    state.threads[0].status = Status::Ready;
    state.threads[0].queued_prompt = Some((
        Prompt {
            text: "queued".into(),
            draft: None,
            context: None,
            images: Default::default(),
        },
        0,
    ));
    assert!(state.update_restart_blocker().is_some());
    state.threads[0].queued_prompt = None;
    state.threads[0].request = Some(json!({"id":"1","method":"confirm"}));
    assert!(state.update_restart_blocker().is_some());
    state.threads[0].request = None;
    // Delegated and hidden threads count too.
    let mut hidden = Thread::new(2);
    hidden.status = Status::Running;
    hidden.delegated = true;
    hidden.life.settle(lifecycle::now());
    state.threads.push(hidden);
    assert!(state.update_restart_blocker().is_some());
    state.threads.pop();
    // A clone in progress is active work.
    state.projects.clone_job = Some(crate::projects::CloneJob::test_idle());
    assert!(state.update_restart_blocker().is_some());
    state.projects.clone_job = None;
    state.rename_thread = Some(1);
    assert!(state.update_restart_blocker().is_some());
    state.rename_thread = None;
    // A composer draft is saved, so it never blocks.
    state.threads[0].draft.replace_text("unsent");
    assert_eq!(state.update_restart_blocker(), None);
}

#[test]
fn thread_defaults_survive_the_index_and_only_apply_without_inheritance() {
    let mut state = State::new(PathBuf::new(), false);
    state.default_harness = "codex".into();
    state.thread_defaults.access = "approval-required".into();
    state.thread_defaults.snooze_minutes = vec![30, 90];
    state.thread_defaults.models.insert(
        "codex".into(),
        crate::ui::settings_ui::ModelDefault {
            provider: "openai".into(),
            model: "fixture-model".into(),
            effort: "high".into(),
        },
    );
    state.create_thread_in(None, &[], &Value::Null);
    assert_eq!(state.current().harness, "codex");
    assert_eq!(state.current().native.access, AccessMode::Supervised);
    assert_eq!(
        state.current().selected_model,
        Some(("openai".into(), "fixture-model".into()))
    );
    assert_eq!(state.current().selected_thinking.as_deref(), Some("high"));
    let saved = state.snapshot();
    let mut restored = State::new(PathBuf::new(), false);
    restored.restore_global(&saved).unwrap();
    assert_eq!(restored.thread_defaults.snooze_minutes, vec![30, 90]);
    restored.threads.push(Thread::new(99));
    restored.threads[0].harness = "claude".into();
    restored.threads[0].models = vec![AgentModel {
        provider: "claude".into(),
        id: "inherited".into(),
        name: "Inherited".into(),
        thinking_levels: vec!["high".into()],
        context_window: None,
        supports_images: None,
        service_tiers: vec![],
        ..Default::default()
    }];
    let index = restored.threads.len() - 1;
    restored.inherit_composer(
        index,
        &json!({"harness":"claude","access":"read-only","model":null}),
    );
    assert_eq!(restored.threads[index].native.access, AccessMode::ReadOnly);
    restored.inherit_composer(
        index,
        &json!({"harness":"claude","access":"read-only","model":["claude","inherited"],"thinking":"high"}),
    );
    assert_eq!(restored.threads[index].harness, "claude");
    assert_eq!(restored.threads[index].native.access, AccessMode::ReadOnly);
    assert_eq!(
        restored.threads[index].selected_model,
        Some(("claude".into(), "inherited".into()))
    );
    restored.preferences.confirm_archive = true;
    restored.preferences.confirm_delete = false;
    let ui = restored.ui_snapshot();
    state.restore_ui(&ui).unwrap();
    assert!(state.preferences.confirm_archive);
    assert!(!state.preferences.confirm_delete);
}

#[test]
fn a_restart_needs_a_successful_save_and_a_normal_quit_only_saves() {
    let _exit_record = exit_record_guard();
    let _ = crate::update::take_exit();
    let mut state = State::new(PathBuf::new(), false);
    // No stores were started, so the save fails and nothing is recorded.
    state.updater = crate::update::Updater::test_ready(prepared("v0.10.0"));
    assert!(state.prepared_exit(true, None).is_err());
    assert!(crate::update::take_exit().is_none());
    // A normal quit saves but never approves installation.
    let mut state = ready_state();
    state.load_global_for_test();
    let update = prepared("v0.10.1");
    state.updater = crate::update::Updater::test_ready(update);
    state
        .prepared_exit(false, None)
        .expect("a ready update with no work saves");
    assert!(crate::update::take_exit().is_none());
    // A busy normal quit also saves without approving an install.
    state.threads[0].status = Status::Running;
    state.updater = crate::update::Updater::test_ready(prepared("v0.10.2"));
    state.prepared_exit(false, None).unwrap();
    assert!(crate::update::take_exit().is_none());
    // A normal quit with no ready update still saves and flushes, and records nothing.
    state.threads[0].status = Status::Ready;
    state.updater = crate::update::Updater::test_idle();
    state
        .current_mut()
        .draft
        .replace_text("saved without an update");
    state
        .prepared_exit(false, None)
        .expect("a normal quit flushes even without an update");
    assert!(crate::update::take_exit().is_none());
    let saved = crate::store::load(&crate::store::directory().unwrap().join("ui-state.json"))
        .unwrap()
        .unwrap();
    assert_eq!(
        saved["drafts"][state.current().id.to_string()],
        "saved without an update"
    );
    // Approval records one restart, independent of the repeated quit saves.
    state.updater = crate::update::Updater::test_ready(prepared("v0.10.3"));
    state.prepared_exit(true, None).unwrap();
    assert!(state.take_restart_request());
    assert!(!state.take_restart_request());
    let _ = crate::update::take_exit();
}

#[test]
fn saved_state_round_trips_drafts_images_tabs_projects_captains_and_sessions() {
    let mut state = State::new(PathBuf::new(), false);
    // Rosters start empty; these tests exercise the earlier named presets.
    state.captains = crate::captains::Captain::presets();
    let base = crate::store::directory().unwrap().join("round-trip");
    std::fs::create_dir_all(&base).unwrap();
    // Two projects, three threads, two drafts, one image reference and two tabs.
    state.projects.entries = vec![
        Project::new(1, base.join("one")),
        Project::new(2, base.join("two")),
    ];
    state.projects.next_id = 3;
    let mut first = Thread::new(1);
    first.title = "First".into();
    first.project = Some(1);
    first.captain = Some(1);
    first.session = Some("one.jsonl".into());
    first.draft.replace_text("unsent draft");
    // Draft images are references to the deduplicated attachment store.
    let attachments = crate::attachments::directory().unwrap();
    std::fs::create_dir_all(&attachments).unwrap();
    std::fs::write(attachments.join("image-1.png"), b"png").unwrap();
    first.draft_images.push(Attachment {
        asset_id: None,
        uploaded: None,
        source: None,
        id: "image-1.png".into(),
        name: "shot.png".into(),
        mime: "image/png".into(),
        width: 4,
        height: 4,
        bytes: 3,
        path: None,
    });
    let mut second = Thread::new(2);
    second.title = "Second".into();
    second.project = Some(2);
    second.session = Some("two.jsonl".into());
    second.draft.replace_text("second draft");
    let mut third = Thread::new(3);
    third.title = "Third".into();
    third.delegated = true;
    third.project = Some(1);
    third.parent = Some(1);
    third.session = Some("three.jsonl".into());
    state.threads = vec![first, second, third];
    state.tabs = vec![1, 2];
    state.active = 1;
    state.captains[0].name = "Renamed captain".into();
    for (project, name) in [
        ("one", "one.jsonl"),
        ("two", "two.jsonl"),
        ("one", "three.jsonl"),
    ] {
        std::fs::create_dir_all(base.join(project)).unwrap();
        std::fs::write(
            base.join(project).join(name),
            format!("saved session {name}\n"),
        )
        .unwrap();
    }
    state.load_global_for_test();
    state.checkpoint_and_flush().unwrap();
    drop(state);
    let mut restored = State::new(PathBuf::new(), false);
    restored.load_global_for_test();
    assert!(
        restored.storage_error.is_none(),
        "{:?}",
        restored.storage_error
    );
    let draft = |state: &State, id: usize| {
        let thread = state.threads.iter().find(|t| t.id == id).unwrap();
        (
            thread.draft.text.clone(),
            thread.draft_images.len(),
            thread.session.clone(),
            thread.project,
            thread.title.clone(),
        )
    };
    assert_eq!(
        draft(&restored, 1),
        (
            "unsent draft".to_owned(),
            1,
            Some("one.jsonl".into()),
            Some(1),
            "First".into()
        )
    );
    assert_eq!(
        draft(&restored, 2),
        (
            "second draft".to_owned(),
            0,
            Some("two.jsonl".into()),
            Some(2),
            "Second".into()
        )
    );
    assert_eq!(draft(&restored, 3).2, Some("three.jsonl".into()));
    assert_eq!(restored.tabs, vec![1, 2]);
    assert_eq!(restored.current().id, 2);
    assert_eq!(restored.projects.entries.len(), 2);
    assert_eq!(restored.captains[0].name, "Renamed captain");
    assert_eq!(
        std::fs::read(attachments.join("image-1.png")).unwrap(),
        b"png"
    );
    for (project, name) in [
        ("one", "one.jsonl"),
        ("two", "two.jsonl"),
        ("one", "three.jsonl"),
    ] {
        assert_eq!(
            std::fs::read_to_string(base.join(project).join(name)).unwrap(),
            format!("saved session {name}\n")
        );
    }
    assert!(
        restored
            .threads
            .iter()
            .any(|t| t.delegated && t.parent == Some(1))
    );
}

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

fn model_with_levels(id: &str, levels: &[&str]) -> AgentModel {
    let mut map = serde_json::Map::new();
    for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
        map.insert(
            level.into(),
            if levels.contains(&level) {
                json!(level)
            } else {
                Value::Null
            },
        );
    }
    let mut value = json!({
        "provider": "cpa",
        "id": id,
        "name": format!("{id} model"),
        "reasoning": true,
    });
    value["thinkingLevelMap"] = Value::Object(map);
    AgentModel::from_value(&value).unwrap()
}

fn preparing_pi(thread: &mut Thread) -> (mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
    let result = fake_pi(thread, Status::Ready);
    thread.initializing = true;
    thread.state_loaded = false;
    thread.history_loaded = false;
    result
}

fn child(state: &mut State) -> (usize, mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
    let index = state.threads.len();
    let mut thread = Thread::new(state.next_id);
    state.next_id += 1;
    thread.title = format!("Child {}", thread.id);
    thread.delegated = true;
    thread.parent = Some(state.threads[0].id);
    thread.project = state.threads[0].project;
    thread.machine = state.threads[0].machine.clone();
    thread.arm_task();
    let (commands, events) = fake_pi(&mut thread, Status::Running);
    state.threads.push(thread);
    (index, commands, events)
}

fn acknowledge(thread: &mut Thread, prompt: &Value) {
    thread.apply(json!({"type":"response","id":prompt["id"],"command":"prompt","success":true,"data":{"disposition":"queued"}}));
}

#[path = "tests_delegation.rs"]
mod tests_delegation;
#[path = "tests_sessions.rs"]
mod tests_sessions;
#[path = "tests_worktrees.rs"]
mod tests_worktrees;

fn test_png(width: u32) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        width,
        1,
        image::Rgba([20, 40, 60, 255]),
    ))
    .write_to(&mut bytes, image::ImageFormat::Png)
    .unwrap();
    bytes.into_inner()
}

fn test_attachment(name: &str, width: u32) -> Attachment {
    crate::attachments::attach_bytes(name, test_png(width)).unwrap()
}
