#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod attachments;
mod backend;
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
        let order = state.visible_sidebar_threads();
        if !order.is_empty() {
            let i = order
                .iter()
                .position(|i| state.threads[*i].id == state.selected_row())
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
        if let Some(i) = state
            .visible_sidebar_threads()
            .get(n as usize - '1' as usize)
        {
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
    install::remove_replaced_exe();
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
        fonts: &[include_bytes!("../assets/fonts/InterVariable.ttf")],
        families: ("Inter Variable", "Cascadia Mono, Consolas"),
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
    .on_close(quit_hook)
    .on_quit(quit_hook)
    .titlebar(rune::Titlebar::new(44.0))
    .run();
    backend::shutdown_all();
    install_recorded_update();
}

/// Runs from both quit hooks and is idempotent, so whichever Rune calls first saves and
/// records. An install is recorded only for an explicit restart or a normal quit with a ready
/// update and no active work. A window that only hides never installs anything.
fn quit_hook(state: &mut State, _: &mut Tx) {
    let quit = rune::quitting();
    prepare_exit(state, quit);
}

fn prepare_exit(state: &mut State, quitting: bool) {
    let explicit = state.take_restart_request();
    if quitting && explicit {
        // The click's own quit already saved and recorded the restart.
        return;
    }
    if state.exit_prepared() {
        return;
    }
    state.mark_exit_prepared();
    if explicit {
        let mut reply = String::new();
        if state.prepared_exit(true, Some(&mut reply)).is_err() {
            state.report_restart_blocker(reply);
        }
        return;
    }
    state.checkpoint();
}

/// After Rune and Pi have exited, replace the installed app with a prepared update. A normal
/// quit installs it without reopening Shina.
fn install_recorded_update() {
    let Some(exit) = update::take_exit() else {
        return;
    };
    match update::apply_prepared_update(&exit.update) {
        Ok(installed) => {
            if exit.relaunch
                && let Err(error) = update::relaunch(&installed, &exit.update.target)
            {
                eprintln!("Could not reopen Shina: {error}");
            }
        }
        Err(error) => {
            // The previous app is back in place, and the download stays for the next quit.
            eprintln!("Could not install the Shina update: {error}");
            if exit.relaunch {
                let launcher = exit.update.launcher();
                let _ = update::relaunch(&launcher, &exit.update.target);
            }
        }
    }
}
