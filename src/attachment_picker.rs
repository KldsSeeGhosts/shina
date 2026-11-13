//! Native multi-file selection, ported from KSG's ComposerAttachButton.
use crate::model::State;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawWindowHandle, WindowHandle,
};
use std::sync::Mutex;

static WINDOW: Mutex<Option<rune::WindowHandle>> = Mutex::new(None);

pub fn set_window(window: &rune::WindowHandle) {
    *WINDOW.lock().expect("attachment picker window") = Some(window.clone());
}

struct Parent<'a>(WindowHandle<'a>);
impl HasWindowHandle for Parent<'_> {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Ok(self.0)
    }
}
impl HasDisplayHandle for Parent<'_> {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        match self.0.as_raw() {
            RawWindowHandle::AppKit(_) => Ok(DisplayHandle::appkit()),
            RawWindowHandle::Win32(_) => Ok(DisplayHandle::windows()),
            _ => Err(HandleError::NotSupported),
        }
    }
}

pub fn open(state: &mut State, thread: usize, options: rune::FileDialogOptions) {
    let machine = state.machines.selected.clone();
    let handle = state.background.handle.clone();
    let window = WINDOW.lock().expect("attachment picker window").clone();
    let result = window
        .ok_or_else(|| "The attachment picker has no parent window.".to_owned())
        .and_then(|window| {
            window.with_raw_handle(|raw| {
                let mut dialog = rfd::AsyncFileDialog::new().set_title(options.title);
                if let Some(directory) = options.directory {
                    dialog = dialog.set_directory(directory);
                }
                if let Some(name) = options.file_name {
                    dialog = dialog.set_file_name(name);
                }
                for (name, extensions) in options.filters {
                    dialog = dialog.add_filter(name, &extensions);
                }
                // Bypass Rune's dialog because FileDialogResult holds one path.
                // Construct on the UI thread, like Rune, then await without blocking Cocoa.
                dialog.set_parent(&Parent(raw)).pick_files()
            })
        });
    match (result, handle) {
        (Ok(future), Some(handle)) => {
            std::thread::spawn(move || {
                let paths = pollster::block_on(future).map(|files| {
                    files
                        .into_iter()
                        .map(|file| file.path().to_owned())
                        .collect::<Vec<_>>()
                });
                handle.update(move |state, _| {
                    if let Some(paths) = paths {
                        state.attach_paths_on_to(&machine, thread, &paths);
                    }
                    state.checkpoint();
                });
            });
        }
        (Err(error), _) => {
            state.set_attachment_target_error(
                &machine,
                thread,
                format!("Could not open the attachment picker: {error}"),
            );
        }
        _ => {}
    }
}
