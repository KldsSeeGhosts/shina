#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod acp;
mod attachments;
mod backend;
mod brand;
mod captains;
mod client;
mod codex;
mod context;
mod crisp;
mod database;
mod harness;
mod horizon;
mod install;
mod lifecycle;
mod maintenance;
mod mcp;
mod model;
mod outbox;
mod peer;
mod portraits;
mod power;
mod process;
mod projects;
mod protocol;
mod routines;
mod runtime;
mod server;
mod server_auth;
mod server_lock;
mod service;
mod session;
mod store;
mod tray;
mod ui;
mod update;
mod visuals;

use model::State;
use rune::{AppSpec, Key, KeyEvent, Tx};
use std::{path::PathBuf, sync::OnceLock};

static OPTIONS: OnceLock<(PathBuf, bool, bool)> = OnceLock::new();

fn init() -> State {
    let (path, demo, snapshot) = OPTIONS.get().expect("startup options");
    let mut state = State::new(path.clone(), *demo);
    if std::env::args().any(|arg| arg == "--server-client") {
        state.client = Some(client::Client::new(state.data_dir.clone()));
    }
    if *snapshot && !demo {
        state.load_snapshot();
    }
    state
}

fn key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if ui::questionnaire_key(state, event) {
        return true;
    }
    if state.undo_key(event, false) {
        return true;
    }
    if event.mods.shift && event.is_command('s') {
        state.settle_thread(state.current().id);
    } else if event.mods.shift && event.is_command('p') {
        state.pin_thread(state.current().id);
    } else if event.mods.shift
        && (event.is_command('[')
            || event.is_command(']')
            || event.is_command('{')
            || event.is_command('}'))
    {
        let order = ui::sidebar_navigation(state);
        if !order.is_empty() {
            let i = order
                .iter()
                .position(|(server, id)| {
                    server == &state.machines.selected && *id == state.current().id
                })
                .unwrap_or(0);
            let next = if event.is_command('[') || event.is_command('{') {
                (i + order.len() - 1) % order.len()
            } else {
                (i + 1) % order.len()
            };
            let (server, id) = &order[next];
            if state.client.is_some() {
                state.open_machine_thread(server, *id);
            } else if let Some(index) = state.threads.iter().position(|thread| thread.id == *id) {
                state.select_thread(index);
            }
        }
    } else if matches!(event.key, Key::Command('1'..='9')) && !event.mods.shift && !event.mods.alt {
        let Key::Command(n) = event.key else {
            unreachable!()
        };
        if let Some((server, id)) = ui::sidebar_navigation(state).get(n as usize - '1' as usize) {
            if state.client.is_some() {
                state.open_machine_thread(server, *id);
            } else if let Some(index) = state.threads.iter().position(|thread| thread.id == *id) {
                state.select_thread(index);
            }
        }
    } else if event.is_command('n') {
        if event.mods.alt {
            state.new_thread_in(None);
        } else {
            state.new_thread();
        }
        state.current_mut().draft.focused = true;
    } else if event.mods.shift && event.is_command('o') {
        state.new_thread();
        state.current_mut().draft.focused = true;
    } else if event.is_command('o') {
        projects::pick_folder(state, tx);
    } else if event.is_command('b') {
        if state.viewport_width < 760.0 {
            state.compact_sidebar = !state.compact_sidebar;
        } else {
            state.sidebar = !state.sidebar;
        }
        state.checkpoint();
    } else if event.is_command('l') {
        state.current_mut().draft.focused = true;
    } else if event.is_command('k') {
        state.search.focused = true;
    } else if event.is_command('=') || event.is_command('+') {
        state.step_zoom(1);
    } else if event.is_command('-') || event.is_command('_') {
        state.step_zoom(-1);
    } else if event.is_command('0') {
        state.step_zoom(0);
    } else if event.key == Key::Escape && state.current().status.busy() {
        state.current_mut().stop();
    } else {
        return false;
    }
    true
}

fn running_demo(state: &mut State) {
    state.populate_demo();
    let thread = state.current_mut();
    thread.status = model::Status::Running;
    thread.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(24));
    thread.elapsed = None;
    thread.phase = "Running powershell".into();
    thread.messages.truncate(3);
    for tool in &mut thread.tools[2..] {
        tool.anchor.message = 2;
        tool.anchor.offset = thread.messages[2].text.len();
    }
    let tool = &mut thread.tools[3];
    tool.done = false;
    tool.duration_ms = None;
    tool.started = Some(std::time::Instant::now());
}

/// An offline, read-only fleet fixture for native navigation and sidebar checks.
fn fleet_demo(state: &mut State) {
    state.populate_demo();
    let mut projections = Vec::new();
    for (server, host, home, chat) in [
        ("local", "Local", 7, 1),
        ("example-studio", "Studio", 8, 3),
        ("example-laptop", "Laptop", 9, 2),
    ] {
        let mut fixture = State::new(PathBuf::new(), false);
        fixture.populate_demo();
        fixture
            .threads
            .retain(|thread| thread.id == home || thread.id == chat);
        fixture.active = fixture
            .threads
            .iter()
            .position(|thread| thread.id == chat)
            .unwrap();
        fixture.tabs = vec![chat];
        for project in &mut fixture.projects.entries {
            project.captain = None;
            project.captain_thread = None;
        }
        let projection = fixture.projection();
        projections.push((server, host, projection));
    }
    state
        .threads
        .retain(|thread| thread.id == 7 || thread.id == 1);
    state.tabs = vec![1];
    let mut client = client::Client::new(state.data_dir.clone());
    for (server, host, projection) in projections {
        if server != "local" {
            let mut remote = client::Client::new(state.data_dir.clone());
            remote.remote = Some(peer::Link {
                server_id: server.into(),
                host: host.into(),
                device_id: "example".into(),
                token: "unused-offline-fixture".into(),
            });
            client.linked.insert(server.into(), remote);
        }
        state.machines.views.insert(server.into(), projection);
    }
    state.client = Some(client);
}

fn markdown_demo(state: &mut State) {
    fleet_demo(state);
    let thread = state.current_mut();
    thread.tools.clear();
    thread.thoughts.clear();
    thread.work.clear();
    thread.captain = None;
    thread.messages = vec![
        model::Message { role: "You", text: "Keep the setup order:\nattach the listener\nopen the socket\n\n```rust\nlet ready = true;\n```".into(), images: vec![] },
        model::Message { role: "Pi", text: "Register the listener before opening the socket.\n\n```rust\nlet ready = true;\nif ready {\n    socket.open();\n}\n```\n\n```diff\n- socket.open();\n  socket.on(\"reconnect\", reconnect);\n+ socket.open();\n```\n\nBoth blocks have a code-only copy action.".into(), images: vec![] },
    ];
}

/// Example coordinator and worker chats. No agents run in snapshot scenes.
fn captain_workspace_demo(state: &mut State) {
    state.populate_demo();
    state.project_scope = model::ProjectScope::Project(1);
    let home = state
        .threads
        .iter()
        .position(|thread| thread.id == 7)
        .unwrap();
    state.threads[home].title = "Polish the Shina workspace".into();
    state.threads[home].renamed = true;
    state.threads[home].messages = vec![
        model::Message {
            role: "You",
            text: "Refine the sidebar and composer. Keep our captains and existing animations, but give each kind of work one clear place.".into(),
            images: vec![],
        },
        model::Message {
            role: "Pi",
            text: "The workspace filter now applies to every conversation.\n\nI'll keep the sidebar review and reconnect fix under this captain. Independent conversations stay in the thread list, with the same pin, snooze, and settle controls.\n\nThe sidebar review needs your approval. You can open the worker directly without leaving this body of work.".into(),
            images: vec![],
        },
    ];
    for id in [1, 2] {
        let worker = state
            .threads
            .iter_mut()
            .find(|thread| thread.id == id)
            .unwrap();
        worker.parent = Some(7);
        worker.delegated = true;
    }
    let review = &mut state.threads[1];
    review.status = model::Status::Waiting;
    review.request = Some(serde_json::json!({
        "id": "demo-approval",
        "method": "confirm",
        "title": "Apply the sidebar changes?",
        "message": "Example approval, not a real pending operation."
    }));
    let mut direct = model::Thread::new(11);
    direct.title = "Explore keyboard navigation".into();
    direct.project = Some(1);
    direct.status = model::Status::Completed;
    direct.life.latest_user_message_at = Some(lifecycle::now());
    state.threads.push(direct);
    // A second captain in the same workspace, with quiet, running and unread workers.
    let mut release = model::Thread::new(12);
    release.title = "Prepare the 0.11 release".into();
    release.renamed = true;
    release.captain = Some(2);
    release.captain_home = true;
    release.project = Some(1);
    state.threads.push(release);
    let now = lifecycle::now();
    for (id, title, status, unread) in [
        (13, "Draft the release notes", model::Status::Running, false),
        (14, "Check the update feed", model::Status::Completed, false),
        (
            15,
            "Verify the signed build",
            model::Status::Completed,
            true,
        ),
    ] {
        let mut worker = model::Thread::new(id);
        worker.title = title.into();
        worker.status = status;
        worker.parent = Some(12);
        worker.delegated = true;
        worker.captain = Some(2);
        worker.project = Some(1);
        if status == model::Status::Completed {
            worker.life.last_completed_at = Some(now - 60_000);
            worker.life.last_visited_at = Some(if unread { now - 120_000 } else { now });
        }
        state.threads.push(worker);
    }
    crew_tasks_demo(state);
    state.tabs = vec![7, 1, 11];
    state.select_thread(home);
}

/// Ember's delegations in the captain workspace: task records in each state, the calls that
/// started them, and one delivered result. Nothing here starts a process.
fn crew_tasks_demo(state: &mut State) {
    let now = lifecycle::now();
    for (id, title, status, phase) in [
        (
            16,
            "Tighten the composer spacing",
            model::Status::Running,
            "Editing src/ui.rs",
        ),
        (
            17,
            "Audit keyboard focus order",
            model::Status::Running,
            "Reading src/main.rs",
        ),
        (18, "Port the attention popover", model::Status::Ready, ""),
        (19, "Document the crew tray", model::Status::Ready, ""),
    ] {
        let mut worker = model::Thread::new(id);
        worker.title = title.into();
        worker.status = status;
        worker.phase = phase.into();
        worker.parent = Some(7);
        worker.delegated = true;
        worker.captain = Some(0);
        worker.project = Some(1);
        worker.life.latest_user_message_at = Some(now - 60_000);
        state.threads.push(worker);
    }
    for id in [1, 2, 16, 17, 18, 19] {
        let worker = state.threads.iter_mut().find(|t| t.id == id).unwrap();
        worker.set_model("cpa".into(), "gpt-5.5".into());
        worker.model = "GPT-5.5 (CPA → Codex)".into();
    }
    if let Some(review) = state.threads.iter_mut().find(|t| t.id == 2) {
        review.phase = "Waiting for approval".into();
    }
    state.next_id = 20;
    use model::TaskState::*;
    state.demo_task(
        1,
        Completed,
        Some(552_000),
        "Reconnect now retries with backoff and keeps the draft. Tests pass.",
        "",
    );
    state.demo_task(2, Running, Some(252_000), "", "");
    state.demo_task(16, Running, Some(65_000), "", "");
    state.demo_task(17, Running, Some(31_000), "", "");
    state.demo_task(19, Queued, None, "", "");
    state.demo_task(
        18,
        Interrupted,
        Some(1_250_000),
        "",
        "Shina restarted before this worker finished. Check its partial changes before resuming.",
    );
    let home = state.threads.iter_mut().find(|t| t.id == 7).unwrap();
    let plan = "I'll split this into six pieces of work. Each worker gets a full brief, and I'll keep the sidebar review with you.";
    let request = std::mem::take(&mut home.messages)
        .into_iter()
        .next()
        .unwrap();
    home.messages = vec![
        request,
        model::Message {
            role: "Pi",
            text: plan.into(),
            images: vec![],
        },
        model::Message {
            role: "Shina",
            text: "Shina: task #1 finished
Task id: demo
Title: Fix the reconnect listener
Status: completed
Last reply:
Reconnect now retries with backoff and keeps the draft. I added a regression test for a dropped socket, and `cargo test` passes.
Error: none".into(),
            images: vec![],
        },
        model::Message {
            role: "Pi",
            text: "The reconnect fix is in and tested. The sidebar review is waiting on your approval below, and the crew tray notes start when a slot frees up.".into(),
            images: vec![],
        },
    ];
    home.tools = [1, 2, 16, 17, 18, 19]
        .into_iter()
        .enumerate()
        .map(|(block, child)| model::Tool {
            id: format!("delegate-{child}"),
            name: "mcp__shina__delegate_task".into(),
            detail: String::new(),
            done: true,
            failed: false,
            output: format!("{{\"thread_id\":{child}}}"),
            images: vec![],
            structured: serde_json::Value::Null,
            parent_call: None,
            exit_code: None,
            subagents: vec![],
            anchor: model::ToolAnchor {
                message: 1,
                offset: plan.len(),
            },
            block,
            expanded: false,
            group_expanded: false,
            started: None,
            duration_ms: Some(400),
        })
        .collect();
}

/// The same work with a standalone thread open, so both crews are collapsed.
/// A captain's Brief with labelled example scope, memory, routines and an earlier
/// conversation. Routines save to a throwaway temporary folder, never Shina's data folder.
fn captain_brief_demo(state: &mut State) {
    captain_workspace_demo(state);
    let Some(captain) = state.current().captain else {
        return;
    };
    let home = state.current().id;
    let _ = state.set_captain_scope(captain, vec![1, 2]);
    for (text, essential) in [
        (
            "Example memory: prefers small pull requests, each with a regression test.",
            true,
        ),
        (
            "Example memory: Shina ships on Windows first. Check macOS before a release.",
            true,
        ),
        (
            "Example memory: port UI from KSG Code instead of inventing new components.",
            true,
        ),
        (
            "Example memory: the release branch is cut on Thursday afternoons.",
            false,
        ),
        (
            "Example memory: Rune lands drag commits at once, so set a motion curve per phase.",
            false,
        ),
    ] {
        let _ = state.add_memory(captain, text.into(), essential);
    }
    let folder = std::env::temp_dir().join(format!("shina-demo-{}", std::process::id()));
    if std::fs::create_dir_all(&folder).is_ok() {
        let mut routines = routines::Routines::load(folder.join("routines.json"));
        let now = chrono::Local::now();
        let made = [
            (
                "Example: Morning review",
                routines::Schedule::Weekdays { at: "09:00".into() },
            ),
            (
                "Example: End-of-day summary",
                routines::Schedule::Daily { at: "18:30".into() },
            ),
            (
                "Example: Dependency check",
                routines::Schedule::Every { minutes: 240 },
            ),
        ]
        .into_iter()
        .filter_map(|(name, schedule)| {
            routines
                .create(
                    captain,
                    home,
                    name.into(),
                    "Example routine prompt.".into(),
                    schedule,
                    routines::MissedPolicy::Skip,
                    now,
                )
                .ok()
        })
        .collect::<Vec<_>>();
        if let Some(&last) = made.last() {
            let _ = routines.set_paused(last, true);
        }
        state.background.routines = routines;
    }
    let mut earlier = model::Thread::new(40);
    earlier.title = "Example: earlier conversation".into();
    earlier.captain = Some(captain);
    earlier.captain_home = true;
    earlier.project = Some(1);
    earlier.updated = projects::now().saturating_sub(3 * 86_400);
    earlier.life.created_at = earlier.updated * 1000;
    earlier.life.updated_at = earlier.life.created_at;
    earlier.messages = vec![model::Message {
        role: "You",
        text: "Example message in an earlier conversation.".into(),
        images: vec![],
    }];
    state.threads.push(earlier);
    state.brief.demo_status = Some(model::BackgroundStatus {
        routines_paused_while_away: true,
        ..Default::default()
    });
    state.show_activity = true;
}

/// The "You" profile editor with a labelled example profile.
fn settings_profile_demo(state: &mut State) {
    state.populate_demo();
    state.brief.profile_open = true;
    state.brief.profile_name.replace_text("Example: Sam");
    state.brief.profile_zone.replace_text("Europe/Berlin");
    state.brief.profile_preferences.replace_text(
        "Example preferences: short answers first, then detail. Ask before large refactors. Prefer British spelling in docs.",
    );
    state.brief.profile_name.focused = true;
}

/// A captain's home with its crew: the needs-you card and the crew tray above the composer.
fn captain_crews_demo(state: &mut State) {
    captain_workspace_demo(state);
    let home = state.threads.iter().position(|t| t.id == 7).unwrap();
    state.select_thread(home);
}

fn main() {
    let cli: Vec<String> = std::env::args().skip(1).collect();
    let result = match cli.first().map(String::as_str) {
        Some("service") => Some(service::cli(&cli[1..])),
        Some("update") => Some(update::cli_update(&cli[1..])),
        Some("pair" | "link" | "links" | "unlink" | "devices") => Some(peer::cli(&cli)),
        Some("--service-run") => Some(
            cli.get(1)
                .ok_or("Service descriptor required.".to_string())
                .and_then(|path| service::launch(std::path::Path::new(path))),
        ),
        Some("--coordinated-update") => Some(update::coordinated_helper()),
        _ => None,
    };
    if let Some(result) = result {
        if let Err(error) = result {
            eprintln!("Shina: {error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("serve") {
        if let Err(error) = server::serve() {
            eprintln!("Shina server: {error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().any(|arg| arg == "--mcp-bridge") {
        if let Err(error) = acp::mcp_bridge() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    // The window icon follows the saved theme: the night sail for dark, the day sail for light.
    let saved_dark = store::directory()
        .filter(|dir| !dir.join("state.sqlite").exists())
        .and_then(|dir| std::fs::read_to_string(dir.join("index.json")).ok())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|index| index["dark"] == true);
    let icon: &[u8] = if saved_dark {
        include_bytes!(concat!(env!("OUT_DIR"), "/shina-icon.png"))
    } else {
        include_bytes!(concat!(env!("OUT_DIR"), "/shina-icon-day.png"))
    };
    if let Err(error) = rune::set_app_icon(icon.to_vec()) {
        eprintln!("Could not set the Shina window icon: {error}");
    }
    let args = rune::extra_args();
    if args.iter().any(|a| a == "--install") {
        // Install this build, then hand over to the installed copy.
        match install::install() {
            Ok(installed) => {
                let _ = install::launch_with(&installed, &[]);
            }
            Err(error) => {
                eprintln!("Could not install Shina: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--uninstall") {
        if let Err(error) = install::uninstall() {
            #[cfg(windows)]
            install::show_uninstall_error(&error);
            eprintln!("Could not uninstall Shina: {error}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(windows)]
    if args.iter().any(|a| a == "--finish-uninstall") {
        if let Err(error) = install::finish_uninstall() {
            install::report_notice(&error);
            if store::directory()
                .is_some_and(|directory| directory.join("uninstall-started").is_file())
            {
                install::show_uninstall_error(&error);
            }
            std::process::exit(1);
        }
        return;
    }
    #[cfg(windows)]
    if args.iter().any(|a| a == "--apply-update") {
        update::run_windows_helper();
        return;
    }
    if install::is_installed() {
        install::adopt_app_id();
    }
    // Installed launches restore the whole global index. Development launches also focus cwd.
    let mut workspace = if install::is_installed() {
        PathBuf::new()
    } else {
        std::env::current_dir().expect("current directory")
    };
    let mut demo = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--demo" => demo = true,
            "--server-client" => {}
            "--workspace" => {
                let Some(path) = iter.next() else {
                    eprintln!("--workspace needs a directory path.");
                    std::process::exit(2);
                };
                workspace = PathBuf::from(path);
            }
            other => {
                eprintln!("Unknown Shina argument: {other}");
                std::process::exit(2);
            }
        }
    }
    let workspace = if workspace.as_os_str().is_empty() {
        workspace
    } else {
        workspace.canonicalize().unwrap_or_else(|error| {
            eprintln!("Could not open workspace {}: {error}", workspace.display());
            std::process::exit(2);
        })
    };
    if !workspace.as_os_str().is_empty() && !workspace.is_dir() {
        eprintln!("The workspace must be a directory.");
        std::process::exit(2);
    }
    let snapshot = std::env::args().any(|arg| arg == "--snap");
    if args.iter().any(|arg| arg == "--server-client") && (demo || snapshot) {
        eprintln!("--server-client cannot be combined with --demo or --snap.");
        std::process::exit(2);
    }
    OPTIONS.set((workspace, demo, snapshot)).ok();
    let app = rune::App::new(AppSpec {
        fonts: ui::FONTS,
        families: (ui::SANS_FAMILY, "Cascadia Mono, Consolas"),
        min_size: Some((620.0, 480.0)),
        setup: Some(|state: &mut State, wake| {
            if !OPTIONS.get().expect("startup options").2 { state.setup(wake); }
        }),
        poll: Some(model::poll),
        on_key: Some(key),
        scenes: &[
            ("example", |state| state.populate_demo()),
            ("fleet", fleet_demo),
            ("fleet-sidebar", |state| { fleet_demo(state); state.compact_sidebar = true; }),
            ("markdown", markdown_demo),
            ("markdown-dark", |state| { markdown_demo(state); state.dark = true; }),
            ("fleet-dark", |state| { fleet_demo(state); state.dark = true; }),
            ("dark", |state| {
                state.populate_demo();
                state.dark = true;
            }),
            ("empty", |state| {
                state.populate_demo();
                state.new_thread_in(None);
            }),
            ("empty-dark", |state| {
                state.populate_demo();
                state.new_thread_in(None);
                state.dark = true;
            }),
            // The narrowest docked sidebar, 200 pt, with the attention badge beside the wordmark.
            ("sidebar-narrow", |state| {
                captain_workspace_demo(state);
                state.sidebar_width = 200.0;
            }),
            ("collapsed", |state| {
                state.populate_demo();
                state.sidebar = false;
            }),
            ("work-running", running_demo),
            ("work-running-dark", |state| {
                running_demo(state);
                state.dark = true;
            }),
            ("work-collapsed", |state| {
                state.populate_demo();
                state.current_mut().work[0].expanded = false;
            }),
            ("thinking-output", |state| {
                state.populate_demo();
                state.current_mut().thoughts[0].expanded = true;
            }),
            ("tool-output", |state| {
                state.populate_demo();
                state.current_mut().tools[3].expanded = true;
            }),
            ("session-details", |state| {
                state.populate_demo();
                state.dark = true;
                state.show_activity = true;
            }),
            ("zoomed", |state| {
                state.populate_demo();
                state.zoom = 1.25;
            }),
            ("captain", |state| {
                state.populate_demo();
                state.open_captain(0);
            }),
            ("captain-workspace", captain_workspace_demo),
            ("captain-workspace-dark", |state| {
                captain_workspace_demo(state);
                state.dark = true;
            }),
            ("captain-worker", |state| {
                captain_workspace_demo(state);
                let worker = state.threads.iter().position(|t| t.id == 16).unwrap();
                state.select_thread(worker);
                let t = state.current_mut();
                t.status = model::Status::Running;
                t.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(65));
                t.messages = vec![
                    model::Message {
                        role: "Shina",
                        text: "Shina: task from parent #7

Tighten the composer's spacing to match KSG Code's ComposerSurface.".into(),
                        images: vec![],
                    },
                ];
            }),
            ("captain-attention", |state| {
                captain_workspace_demo(state);
                state.recovery_notice = Some("Shina was closed while 2 tasks were running. Check their partial work, then resume or dismiss them.".into());
                state.attention_menu = true;
            }),
            ("captain-goal-empty", |state| {
                captain_workspace_demo(state);
                let home = state.current_mut();
                home.title = "Polish the Shina workspace and keyboard navigation".into();
                home.messages.clear();
            }),
            ("captain-brief", captain_brief_demo),
            ("captain-brief-dark", |state| {
                captain_brief_demo(state);
                state.dark = true;
            }),
            ("settings-profile", settings_profile_demo),
            ("settings-menu", |state| {
                state.populate_demo();
                state.settings_menu = true;
            }),
            ("captain-crews", captain_crews_demo),
            ("captain-crews-dark", |state| {
                captain_crews_demo(state);
                state.dark = true;
            }),
            ("captain-crews-narrow", |state| {
                captain_crews_demo(state);
                state.compact_sidebar = true;
            }),
            ("captain-search", |state| {
                captain_workspace_demo(state);
                        state.search.replace_text("sidebar");
            }),
            ("thread-menu", |state| {
                state.populate_demo();
                state.dark = true;
                state.thread_menu = true;
            }),
            ("shelves-expanded", |state| {
                state.populate_demo();
                state.shelves = [true; 2];
            }),
            ("snooze-menu", |state| {
                state.populate_demo();
                state.open_snooze(1);
            }),
            ("undo-notice", |state| {
                state.populate_demo();
                state.settle_thread(3);
            }),
            ("projects", |state| {
                state.populate_demo();
                state.open_picker(model::ProjectPicker::AddProject);
            }),
            ("projects-dark", |state| {
                state.populate_demo();
                state.dark = true;
                state.open_picker(model::ProjectPicker::AddProject);
            }),
            ("clone", |state| {
                state.populate_demo();
                state.open_clone_dialog();
                state.projects.clone_source.replace_text("anomalyco/rune");
                state.projects.clone_progress = "Receiving objects:  42% (1234/2938), 3.10 MiB | 4.20 MiB/s".into();
            }),
            ("clone-dark", |state| {
                state.populate_demo();
                state.dark = true;
                state.open_clone_dialog();
                state.projects.clone_source.replace_text("https://github.com/anomalyco/rune");
                state.projects.clone_error = Some(format!(
                    "{} already exists and is not empty. Choose another destination.",
                    state.projects.clone_parent.join("rune").display()
                ));
            }),
            ("context-popover", |state| {
                state.populate_demo();
                let t = state.current_mut();
                t.context_window = Some(200_000);
                t.context.absorb(&serde_json::json!({
                    "contextUsage": {"tokens": 61_400, "contextWindow": 200_000, "percent": 30.7}
                }));
                t.native.stats = serde_json::json!({"tokens": {"total": 1_284_000}, "cost": 0.4183});
                t.context.details_open = true;
            }),
            ("picker", |state| {
                state.populate_demo();
                state.captain_picker = true;
            }),
            ("harness-picker", |state| {
                state.populate_demo();
                let fixture = |id: &str, name: &str| harness::Agent {
                    id: id.into(),
                    name: name.into(),
                    command: String::new(),
                    args: vec![],
                    env: vec![],
                };
                state.harnesses.agents = vec![
                    fixture("claude-code", "Claude Code"),
                    fixture("codex", "Codex"),
                    fixture("goose", "Goose"),
                ];
                state.current_mut().model = "GPT-5.5 (CPA → Codex)".into();
                state.current_mut().models = vec![];
                state.settings_picker = Some(model::SettingsPicker::Model);
            }),
            ("browse-folders", |state| {
                state.populate_demo();
                state.dark = true;
                state.browse_for_project();
            }),
            ("workspace-picker", |state| {
                state.populate_demo();
                state.dark = true;
                state.open_picker(model::ProjectPicker::NewThread);
            }),
            ("new-captain", |state| {
                state.populate_demo();
                state.begin_new_captain();
                let editor = state.captain_editor.as_mut().unwrap();
                editor.name.replace_text("Atlas");
                editor.role.replace_text("Code review");
                editor.instructions.replace_text("Review changes for correctness and regressions. Cite the files and tests that support each finding.");
                editor.portrait = 2;
            }),
        ],
        ..AppSpec::new("Shina", (1280.0, 840.0), init, ui::view)
    })
    // Portraits rest while the window is in the background or minimized.
    .on_focus(|state, focused, _| state.portraits.set_focused(focused))
    .keep_running()
    .on_close(|state, tx| {
        if !state.close_in_background() {
            close_hook(state);
            tx.quit();
        }
    })
    .on_quit(|state, _| quit_hook(state))
    .titlebar(rune::Titlebar::new(44.0));
    let handle = app.handle();
    let shared = handle.clone();
    handle.update(move |state, _| state.background.handle = Some(shared));
    app.run();
    client::shutdown_all();
    backend::shutdown_all();
    install_recorded_update();
}

/// Save on actual close-to-quit too, including session end.
fn close_hook(state: &mut State) {
    quit_hook(state);
}

/// Always save on each quit attempt. Windows can cancel session end and call this again.
/// Normal quits never approve an install, so cancellation cannot leave a stale approval.
fn quit_hook(state: &mut State) {
    // Kill the owned clone tree before native host teardown drops its worker threads.
    state.projects.clone_job.take();
    if let Err(error) = state.checkpoint_and_flush() {
        state.cancel_update_exit();
        eprintln!("Could not save Shina before quitting: {error}");
    } else {
        state.mark_clean_quit();
    }
}

/// After Rune and Pi shut down, apply only an explicitly approved update restart.
fn install_recorded_update() {
    let Some(exit) = update::take_exit().filter(|exit| exit.relaunch) else {
        return;
    };
    #[cfg(windows)]
    {
        if let Err(error) = update::start_windows_helper(&exit.update) {
            install::report_notice(&format!("Could not start the update helper: {error}"));
            eprintln!("Could not start the update helper: {error}");
            let _ = update::relaunch(&exit.update.launcher(), &exit.update.target);
        }
    }
    #[cfg(not(windows))]
    install_exit_update(exit);
}

#[cfg(not(windows))]
fn install_exit_update(exit: update::ExitUpdate) {
    // This exited process, which a failed relaunch may replace with the previous build.
    let running = std::env::current_exe().unwrap_or_else(|_| exit.update.launcher());
    match update::apply_prepared_update(&exit.update) {
        Ok(installed) => {
            let _ = std::fs::remove_file(update::Storage::default_storage().manifest());
            if exit.relaunch
                && let Err(error) = update::relaunch_or_restore(&exit.update, &installed, &running)
            {
                eprintln!("{error}");
            }
        }
        Err(error) => {
            // The previous app was restored where possible, and the download stays staged.
            eprintln!("Could not install the Shina update: {error}");
            if exit.relaunch
                && let Err(error) = update::relaunch(&exit.update.launcher(), &exit.update.target)
            {
                eprintln!("Could not reopen Shina: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::InstalledTarget;

    /// A verified update for an app in a fixture folder.
    fn prepared(root: &std::path::Path, target: InstalledTarget) -> update::PreparedUpdate {
        update::PreparedUpdate {
            tag: "v0.15.0".into(),
            target,
            asset: "shina.exe".into(),
            size: 1,
            sha256: "0".repeat(64),
            download: root.join("download.exe"),
            prepared: root.join("prepared.exe"),
        }
    }

    #[test]
    fn close_and_quit_flush_actual_drafts_without_an_update_or_while_busy() {
        let _quit = update::test_exit_guard();
        for close in [true, false] {
            for busy in [true, false] {
                let root = crate::store::directory().unwrap();
                let mut state = State::new(PathBuf::new(), false);
                state.load_global_for_test();
                if busy {
                    state.updater = update::Updater::test_ready(prepared(
                        &root,
                        InstalledTarget::Windows {
                            exe: root.join("install/shina.exe"),
                        },
                    ));
                    state.current_mut().status = model::Status::Running;
                }
                let draft = format!("final unsent draft, close={close}, busy={busy}");
                state.current_mut().draft.replace_text(&draft);
                if close {
                    close_hook(&mut state);
                } else {
                    quit_hook(&mut state);
                }
                let ui = store::load(&root.join("ui-state.json")).unwrap().unwrap();
                let id = state.current().id.to_string();
                assert_eq!(ui["drafts"][&id], draft);
                assert!(root.join("index.json").is_file());
                assert!(!update::exit_recorded());
            }
        }
    }

    #[test]
    fn a_quit_after_an_approved_restart_keeps_the_restart_record() {
        let _quit = update::test_exit_guard();
        let _ = update::take_exit();
        let mut state = State::new(PathBuf::new(), false);
        state.load_global_for_test();
        state.updater = update::Updater::test_ready(prepared(
            &crate::store::directory().unwrap(),
            InstalledTarget::Windows {
                exe: PathBuf::from("C:/fixture/Shina/shina.exe"),
            },
        ));
        // The click's poll saved and recorded the restart; the quit hook must not undo it.
        state.prepared_exit(true, None).unwrap();
        close_hook(&mut state);
        quit_hook(&mut state);
        let exit = update::take_exit().expect("the restart stays recorded");
        assert!(exit.relaunch);
    }

    #[test]
    fn a_cancelled_session_end_does_not_reuse_an_approval_or_skip_a_later_save() {
        let _quit = update::test_exit_guard();
        let root = store::directory().unwrap();
        let mut state = State::new(PathBuf::new(), false);
        state.load_global_for_test();
        state.updater = update::Updater::test_ready(prepared(
            &root,
            InstalledTarget::Windows {
                exe: root.join("install/shina.exe"),
            },
        ));
        // WM_QUERYENDSESSION saves, then WM_ENDSESSION(false) leaves the app running.
        state
            .current_mut()
            .draft
            .replace_text("before cancelled logoff");
        quit_hook(&mut state);
        assert!(!update::exit_recorded());
        model::poll(&mut state, 0.0, &mut Tx::default());
        state
            .current_mut()
            .draft
            .replace_text("after cancelled logoff");
        quit_hook(&mut state);
        let ui = store::load(&root.join("ui-state.json")).unwrap().unwrap();
        assert_eq!(
            ui["drafts"][state.current().id.to_string()],
            "after cancelled logoff"
        );
        assert!(!update::exit_recorded());
    }
}
