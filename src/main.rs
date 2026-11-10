#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod attachments;
mod backend;
mod brand;
mod captains;
mod context;
mod install;
mod lifecycle;
mod mcp;
mod model;
mod projects;
mod session;
mod store;
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
    if *snapshot && !demo {
        state.load_snapshot();
    }
    state
}

fn key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
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
        let order = state.sidebar_navigation();
        if !order.is_empty() {
            let i = order
                .iter()
                .position(|i| state.threads[*i].id == state.current().id)
                .unwrap_or(0);
            let next = if event.is_command('[') || event.is_command('{') {
                (i + order.len() - 1) % order.len()
            } else {
                (i + 1) % order.len()
            };
            state.select_thread(order[next]);
        }
    } else if matches!(event.key, Key::Command('1'..='9')) && !event.mods.shift && !event.mods.alt {
        let Key::Command(n) = event.key else {
            unreachable!()
        };
        if let Some(i) = state.sidebar_navigation().get(n as usize - '1' as usize) {
            state.select_thread(*i);
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
        state.search_open = true;
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
    state.next_id = 16;
    state.tabs = vec![7, 1, 11];
    state.select_thread(home);
}

/// The same work with a standalone thread open, so both crews are collapsed.
fn captain_crews_demo(state: &mut State) {
    captain_workspace_demo(state);
    let solo = state.threads.iter().position(|t| t.id == 11).unwrap();
    state.select_thread(solo);
}

fn main() {
    if let Err(error) =
        rune::set_app_icon(include_bytes!(concat!(env!("OUT_DIR"), "/shina-icon.png")).to_vec())
    {
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
            eprintln!("Could not uninstall Shina: {error}");
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
    OPTIONS.set((workspace, demo, snapshot)).ok();
    rune::App::new(AppSpec {
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
            ("sidebar-narrow", |state| {
                state.populate_demo();
                state.compact_sidebar = true;
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
            ("captain-goal-empty", |state| {
                captain_workspace_demo(state);
                let home = state.current_mut();
                home.title = "Polish the Shina workspace and keyboard navigation".into();
                home.messages.clear();
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
                state.search_open = true;
                state.search.replace_text("sidebar");
            }),
            ("thread-menu", |state| {
                state.populate_demo();
                state.dark = true;
                state.thread_menu = true;
            }),
            ("shelves-expanded", |state| {
                state.populate_demo();
                state.shelves = [true; 3];
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
                state.open_project_menu();
            }),
            ("projects-dark", |state| {
                state.populate_demo();
                state.dark = true;
                state.open_project_menu();
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
            ("picker", |state| {
                state.populate_demo();
                state.captain_picker = true;
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
    .on_close(|state, _| close_hook(state))
    .on_quit(|state, _| quit_hook(state))
    .titlebar(rune::Titlebar::new(44.0))
    .run();
    backend::shutdown_all();
    install_recorded_update();
}

/// This app does not enable Rune's keep_running. Closing its window terminates the host,
/// which Rune removes before running quit hooks. Save here too, including session end.
fn close_hook(state: &mut State) {
    quit_hook(state);
}

/// Always save on each quit attempt. Windows can cancel session end and call this again.
/// Normal quits never approve an install, so cancellation cannot leave a stale approval.
fn quit_hook(state: &mut State) {
    if let Err(error) = state.checkpoint_and_flush() {
        state.cancel_update_exit();
        eprintln!("Could not save Shina before quitting: {error}");
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
