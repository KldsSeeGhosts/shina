//! A per-user install with a stable path, so Start pins, the Caelestia launcher, and in-app
//! updates all point at the same executable. Nothing here needs administrator rights.
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Windows identifies the installed app by this ID, not by its path, so pins survive moves.
pub const APP_ID: &str = "KldsSeeGhosts.Shina";
const SET_APP_ID: &str = include_str!("../assets/windows/set-app-id.ps1");

/// Gives this process the installed app's ID, so its taskbar button matches the Start
/// shortcut and anything pinned from the running window points at the installed copy.
pub fn adopt_app_id() {
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SetCurrentProcessExplicitAppUserModelID(id: *const u16) -> i32;
        }
        let id: Vec<u16> = APP_ID.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `id` is a NUL-terminated UTF-16 string that outlives the call.
        unsafe {
            SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
        }
    }
}

const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Shina";

/// `%LOCALAPPDATA%\Programs\Shina`, where the installed copy lives.
pub fn install_dir() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("Programs").join("Shina"))
}

pub fn installed_exe() -> Option<PathBuf> {
    install_dir().map(|dir| dir.join("shina.exe"))
}

/// Whether this process is the installed copy rather than a development build.
pub fn is_installed() -> bool {
    let (Ok(current), Some(installed)) = (env::current_exe(), installed_exe()) else {
        return false;
    };
    same_file(&current, &installed)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn start_menu_shortcut() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .map(|base| PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs\Shina.lnk"))
}

fn settings_dir() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("Shina"))
}

/// The workspace the installed app reopens when launched without `--workspace`.
pub fn last_workspace() -> Option<PathBuf> {
    let text = fs::read_to_string(settings_dir()?.join("last-workspace.txt")).ok()?;
    let path = PathBuf::from(text.trim());
    path.is_dir().then_some(path)
}

pub fn remember_workspace(path: &Path) {
    if let Some(dir) = settings_dir() {
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(
            dir.join("last-workspace.txt"),
            path.to_string_lossy().as_bytes(),
        );
    }
}

/// Removes the executable an update or reinstall replaced. It could not be deleted while it
/// was running, so the next launch cleans it up.
pub fn remove_replaced_exe() {
    if let Ok(current) = env::current_exe()
        && let Some(dir) = current.parent()
    {
        let _ = fs::remove_file(dir.join("shina.old.exe"));
    }
}

/// Puts `source` at the installed path. A running installed copy is renamed aside first,
/// because Windows allows renaming a running executable but not overwriting it.
pub fn place_exe(source: &Path) -> Result<PathBuf, String> {
    let dir = install_dir().ok_or("LOCALAPPDATA is not set.")?;
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let target = dir.join("shina.exe");
    if target.exists() && same_file(source, &target) {
        return Ok(target);
    }
    let staged = dir.join("shina.new.exe");
    fs::copy(source, &staged).map_err(|e| format!("Could not copy Shina: {e}"))?;
    if target.exists() {
        let old = dir.join("shina.old.exe");
        let _ = fs::remove_file(&old);
        fs::rename(&target, &old)
            .map_err(|e| format!("Could not move the previous Shina aside: {e}"))?;
    }
    fs::rename(&staged, &target).map_err(|e| format!("Could not install Shina: {e}"))?;
    Ok(target)
}

/// Starts Shina without sharing this process's console or pipes, so a script that ran
/// `--install` (or the app that relaunched for an update) is never held open by it.
pub fn launch_with(exe: &Path, args: &[&str]) -> Result<(), String> {
    Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not start {}: {e}", exe.display()))
}

fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

fn powershell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// `shina --install`: copy this build into place, add the Start Menu shortcut, and register
/// an Apps & Features entry. Safe to run again; it refreshes everything.
pub fn install() -> Result<PathBuf, String> {
    let current = env::current_exe().map_err(|e| e.to_string())?;
    let target = place_exe(&current)?;
    let shortcut = start_menu_shortcut().ok_or("APPDATA is not set.")?;
    let home = env::var("USERPROFILE").unwrap_or_default();
    let script = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut({}); $s.TargetPath = {}; \
         $s.WorkingDirectory = {}; $s.IconLocation = {}; $s.Description = 'Shina'; $s.Save()",
        powershell_quote(&shortcut.to_string_lossy()),
        powershell_quote(&target.to_string_lossy()),
        powershell_quote(&home),
        powershell_quote(&format!("{},0", target.to_string_lossy())),
    );
    let status = hidden(Command::new("powershell").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]))
    .status()
    .map_err(|e| format!("Could not create the Start Menu shortcut: {e}"))?;
    if !status.success() {
        return Err("Could not create the Start Menu shortcut.".into());
    }
    let helper = target.with_file_name("set-app-id.ps1");
    fs::write(&helper, SET_APP_ID)
        .map_err(|e| format!("Could not write {}: {e}", helper.display()))?;
    let status = hidden(Command::new("powershell").args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ]))
    .arg(&helper)
    .arg("-Shortcut")
    .arg(&shortcut)
    .args(["-AppId", APP_ID])
    .status()
    .map_err(|e| format!("Could not set the shortcut's app ID: {e}"))?;
    if !status.success() {
        return Err("Could not set the shortcut's app ID.".into());
    }
    let exe = target.to_string_lossy().into_owned();
    let dir = target
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    for (name, kind, value) in [
        ("DisplayName", "REG_SZ", "Shina".to_owned()),
        (
            "DisplayVersion",
            "REG_SZ",
            env!("CARGO_PKG_VERSION").to_owned(),
        ),
        ("Publisher", "REG_SZ", "KldsSeeGhosts".to_owned()),
        ("InstallLocation", "REG_SZ", dir),
        ("DisplayIcon", "REG_SZ", exe.clone()),
        (
            "UninstallString",
            "REG_SZ",
            format!("\"{exe}\" --uninstall"),
        ),
        ("NoModify", "REG_DWORD", "1".to_owned()),
        ("NoRepair", "REG_DWORD", "1".to_owned()),
    ] {
        let status = hidden(Command::new("reg").args([
            "add",
            UNINSTALL_KEY,
            "/v",
            name,
            "/t",
            kind,
            "/d",
            &value,
            "/f",
        ]))
        .status()
        .map_err(|e| format!("Could not register Shina with Windows: {e}"))?;
        if !status.success() {
            return Err(format!("Could not write the {name} uninstall value."));
        }
    }
    Ok(target)
}

/// `shina --uninstall`: remove the shortcut and registration, then delete the install folder
/// once this process has exited. Thread indexes in `%LOCALAPPDATA%\Shina` are kept.
pub fn uninstall() -> Result<(), String> {
    if let Some(shortcut) = start_menu_shortcut() {
        let _ = fs::remove_file(shortcut);
    }
    let _ = hidden(Command::new("reg").args(["delete", UNINSTALL_KEY, "/f"])).status();
    if let Some(dir) = install_dir().filter(|d| d.is_dir()) {
        let path = dir.to_string_lossy().into_owned();
        let mut command = Command::new("cmd");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // cmd parses its own quoting, so pass the line verbatim.
            command.raw_arg(format!(
                "/C \"timeout /t 2 /nobreak >nul & rmdir /s /q \"{path}\"\""
            ));
        }
        hidden(&mut command)
            .spawn()
            .map_err(|e| format!("Could not schedule removal of {path}: {e}"))?;
    }
    Ok(())
}
