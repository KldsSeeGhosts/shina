#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backend;
mod captains;
mod install;
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

static OPTIONS: OnceLock<(PathBuf, bool)> = OnceLock::new();

fn init() -> State {
    let (path, demo) = OPTIONS.get().expect("startup options");
    State::new(path.clone(), *demo)
}

fn key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if event.is_command('n') {
        state.new_thread();
        state.current_mut().draft.focused = true;
    } else if event.is_command('o') {
        projects::pick_folder(state, tx);
    } else if event.is_command('b') {
        state.sidebar = !state.sidebar;
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
    // The installed app reopens the last workspace; development builds use the current folder.
    let mut workspace = install::is_installed()
        .then(install::last_workspace)
        .flatten()
        .unwrap_or_else(|| std::env::current_dir().expect("current directory"));
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
    let workspace = workspace.canonicalize().unwrap_or_else(|error| {
        eprintln!("Could not open workspace {}: {error}", workspace.display());
        std::process::exit(2);
    });
    if !workspace.is_dir() {
        eprintln!("The workspace must be a directory.");
        std::process::exit(2);
    }
    OPTIONS.set((workspace, demo)).ok();
    rune::App::new(AppSpec {
        fonts: &[include_bytes!("../assets/fonts/InterVariable.ttf")],
        families: ("Inter Variable", "Cascadia Mono, Consolas"),
        min_size: Some((620.0, 480.0)),
        setup: Some(|state: &mut State, wake| state.setup(wake)),
        poll: Some(model::poll),
        on_key: Some(key),
        scenes: &[
            ("example", |state| state.populate_demo()),
            ("dark", |state| {
                state.populate_demo();
                state.dark = true;
            }),
            ("zoomed", |state| {
                state.populate_demo();
                state.zoom = 1.25;
            }),
            ("captain", |state| {
                state.populate_demo();
                state.open_captain(0);
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
    .titlebar(rune::Titlebar::new(44.0))
    .run();
    backend::shutdown_all();
}
