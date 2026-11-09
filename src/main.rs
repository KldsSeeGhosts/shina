#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backend;
mod captains;
mod model;
mod store;
mod ui;
mod visuals;

use model::State;
use rune::{AppSpec, Key, KeyEvent, Tx};
use std::{path::PathBuf, sync::OnceLock};

static OPTIONS: OnceLock<(PathBuf, bool)> = OnceLock::new();

fn init() -> State {
    let (path, demo) = OPTIONS.get().expect("startup options");
    State::new(path.clone(), *demo)
}

fn key(state: &mut State, event: &KeyEvent, _: &mut Tx) -> bool {
    if event.is_command('n') {
        state.new_thread();
        state.current_mut().draft.focused = true;
    } else if event.is_command('b') {
        state.sidebar = !state.sidebar;
    } else if event.is_command('l') {
        state.current_mut().draft.focused = true;
    } else if event.is_command('k') {
        state.search_open = true;
        state.search.focused = true;
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
    let mut workspace = std::env::current_dir().expect("current directory");
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
            ("captain", |state| {
                state.populate_demo();
                state.open_captain(0);
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
