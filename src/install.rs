//! A per-user install with a stable path, so Start pins, the Caelestia launcher, and in-app
//! updates all point at the same executable. Nothing here needs administrator rights.
//!
//! Each platform has the same two update steps. Prepare copies a verified download next to the
//! installed app before the update is offered. Commit runs once this process and Pi are gone:
//! it renames the installed app aside, moves the prepared one into place, and restores the old
//! one if the move fails. The previous app stays as a recovery copy.
use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

/// Windows identifies the installed app by this ID, not by its path, so pins survive moves.
#[cfg(windows)]
pub const APP_ID: &str = "KldsSeeGhosts.Shina";
#[cfg(windows)]
const SET_APP_ID: &str = include_str!("../assets/windows/set-app-id.ps1");
pub const MACOS_BUNDLE_ID: &str = "com.kidsseeghosts.shina";
pub const MACOS_APP_NAME: &str = "Shina.app";

/// Gives this process the installed app's ID, so its taskbar button matches the Start
/// shortcut and anything pinned from the running window points at the installed copy, and
/// refreshes its shortcut and Apps & Features entry after an update.
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
        // Best effort: a failed shortcut refresh must not stop an already-replaced app.
        let _ = refresh_windows_registration();
    }
}

#[cfg(windows)]
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Shina";

/// `%LOCALAPPDATA%\Programs\Shina`, where the installed copy lives.
pub fn install_dir() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("Programs").join("Shina"))
}

/// Where an installed copy lives: the executable on Windows, the `.app` bundle on macOS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstalledTarget {
    Windows { exe: PathBuf },
    MacBundle { app: PathBuf },
}

impl InstalledTarget {
    /// The Windows executable or the macOS bundle directory.
    pub fn path(&self) -> &Path {
        match self {
            Self::Windows { exe } => exe,
            Self::MacBundle { app } => app,
        }
    }
    /// The release asset this build installs from, for the running architecture.
    pub fn asset(&self) -> &'static str {
        #[cfg(all(not(windows), target_arch = "x86_64"))]
        if matches!(self, Self::MacBundle { .. }) {
            return "Shina-macos-x86_64.zip";
        }
        if matches!(self, Self::Windows { .. }) {
            "shina.exe"
        } else {
            "Shina-macos-aarch64.zip"
        }
    }
    /// The format of the real installation on this operating system.
    fn format(executable: &Path) -> Self {
        if cfg!(windows) {
            Self::Windows {
                exe: executable.to_path_buf(),
            }
        } else {
            let app = bundle_root(executable)
                .unwrap_or_else(|| executable.parent().unwrap_or(executable).join("Shina.app"));
            Self::MacBundle { app }
        }
    }
}

/// The installed app this process runs from, or `None` for a development build. Development
/// executables and unsupported platforms never update themselves.
pub fn installed_target() -> Option<InstalledTarget> {
    installed_target_for(
        &env::current_exe().ok()?,
        cfg!(windows).then(install_dir).flatten(),
        cfg!(target_os = "macos")
            .then(macos_app_directory)
            .flatten(),
    )
}

/// The installed copy this process runs from, or `None` for a development build.
pub fn is_installed() -> bool {
    installed_target().is_some()
}

/// `installed_target` against explicit locations, so each platform's rules can be tested on
/// any host.
fn installed_target_for(
    current: &Path,
    windows_dir: Option<PathBuf>,
    macos_dir: Option<PathBuf>,
) -> Option<InstalledTarget> {
    if let Some(dir) = windows_dir {
        let target = target_for_executable(current, &dir.join("shina.exe"))?;
        return matches!(target, InstalledTarget::Windows { .. }).then_some(target);
    }
    let directory = macos_dir?;
    let app = bundle_root(current)?;
    let target = target_for_executable(current, &directory.join(MACOS_APP_NAME))?;
    if !matches!(target, InstalledTarget::MacBundle { .. })
        || !same_file(&app, target.path())
        || mac_bundle_version(&app).is_none()
        || mac_bundle_id(&app).as_deref() != Some(MACOS_BUNDLE_ID)
        || !app.join("Contents/MacOS").join(binary_name()).is_file()
    {
        return None;
    }
    Some(target)
}

/// The installed target an executable belongs to, if it runs from `installed`: the
/// executable itself on Windows, the bundle's inner binary on macOS.
fn target_for_executable(current: &Path, installed: &Path) -> Option<InstalledTarget> {
    let target = InstalledTarget::format(installed);
    if !same_file(current, &executable_in(&target)) {
        return None;
    }
    Some(target)
}

/// The executable a target runs, which is the bundle's inner binary on macOS.
fn executable_in(target: &InstalledTarget) -> PathBuf {
    match target {
        InstalledTarget::Windows { exe } => exe.clone(),
        InstalledTarget::MacBundle { app } => app.join(executable_path()),
    }
}

/// Where the executable sits inside an installed target.
fn executable_path() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("shina.exe")
    } else {
        PathBuf::from("Contents").join("MacOS").join(binary_name())
    }
}

pub fn binary_name() -> &'static str {
    if cfg!(windows) { "shina.exe" } else { "shina" }
}

/// The directory an installed Mac app may be replaced in: `/Applications`, or the writable
/// directory `SHINA_MACOS_APP_DIR` names.
fn macos_app_directory() -> Option<PathBuf> {
    match env::var_os("SHINA_MACOS_APP_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => Some(PathBuf::from("/Applications")),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The outermost `*.app` directory holding `executable`.
fn bundle_root(executable: &Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        })
        .map(Path::to_path_buf)
}

#[cfg(windows)]
fn start_menu_shortcut() -> Option<PathBuf> {
    env::var_os("APPDATA")
        .map(|base| PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs\Shina.lnk"))
}

/// Repairs an install a crash between an update's two renames left without its executable.
/// Runs before anything reads the installed path, so the recovery copy is back in place for
/// the rest of the launch. It starts nothing when this process already runs from the
/// installed path, so a repaired app does not spawn a second copy.
#[cfg(windows)]
pub fn recover_install() {
    restore_replaced_exe(env::current_exe().ok().as_deref());
}

#[cfg(not(windows))]
pub fn recover_install() {}

/// Removes the previous executable a replacement kept, once the installed path is healthy.
#[cfg(windows)]
pub fn remove_replaced_exe() {
    if let Some(dir) = env::current_exe().ok().as_deref().and_then(Path::parent) {
        let _ = fs::remove_file(previous_exe(dir));
    }
}

#[cfg(not(windows))]
pub fn remove_replaced_exe() {}

#[cfg(windows)]
fn restore_replaced_exe(current: Option<&Path>) {
    let Some(current) = current else { return };
    let Some(dir) = current.parent() else { return };
    let target = dir.join("shina.exe");
    let old = previous_exe(dir);
    if (target.exists() || target.is_symlink()) || !old.is_file() {
        return;
    }
    // This run itself may hold the lock, so a held lock only skips this launch.
    let Ok(_lock) = InstallLock::acquire(dir) else {
        return;
    };
    if fs::rename(&old, &target).is_ok() && !same_file(current, &target) {
        // Restored while another copy runs: -install hands over to the installed app.
        let _ = launch_with(&target, &["-install"]);
    }
}

/// Unsupported platforms never replace a running app.
#[cfg(not(any(windows, target_os = "macos")))]
fn restore_replaced_exe(_: Option<&Path>) {}

#[cfg(test)]
type Restore = dyn Fn(Option<&Path>, &Path) -> Result<(), PathBuf>;
#[cfg(test)]
type Rename = dyn Fn(&Path) -> std::io::Result<()>;

#[cfg(test)]
thread_local! {
    /// Replaces the second rename of a commit, so a test can make it fail.
    static FINAL_RENAME: std::cell::RefCell<Option<Box<Rename>>> =
        const { std::cell::RefCell::new(None) };
    /// Replaces the rollback's restore, so a test can check the error that reports it.
    static RESTORE: std::cell::RefCell<Option<Box<Restore>>> = const { std::cell::RefCell::new(None) };
}

/// Windows: the installed build renamed aside during a replacement, in the same folder as
/// the installed executable.
fn previous_exe(dir: &Path) -> PathBuf {
    dir.join("shina.old.exe")
}

/// Puts `source` at the installed path now: prepare, then commit. `--install` uses this.
pub fn place_exe(source: &Path) -> Result<PathBuf, String> {
    let dir = install_dir().ok_or("LOCALAPPDATA is not set.")?;
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let staged = prepare_exe(source, &dir)?;
    commit_exe(&staged, &dir)?;
    Ok(dir.join("shina.exe"))
}

/// Windows prepare: copy the download beside the installed executable and verify the copy.
pub fn prepare_exe(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let staged = dir.join("shina.new.exe");
    let _ = fs::remove_file(&staged);
    fs::copy(source, &staged).map_err(|e| format!("Could not copy Shina: {e}"))?;
    // NTFS ignores mode bits, but a copy elsewhere has to stay executable.
    if let Ok(metadata) = fs::metadata(source) {
        let _ = fs::set_permissions(&staged, metadata.permissions());
    }
    let copied = fs::metadata(&staged).map_err(|e| format!("Could not read Shina: {e}"))?;
    let expected = fs::metadata(source).map_err(|e| format!("Could not read Shina: {e}"))?;
    if copied.len() != expected.len() || !same_contents(&staged, source) {
        let _ = fs::remove_file(&staged);
        return Err("The prepared Shina copy does not match its download.".into());
    }
    Ok(staged)
}

/// Windows commit: rename the running build aside, then move the prepared one into place.
/// A failed move puts the previous build back. Windows allows renaming a running executable
/// but not overwriting it. A crash between the two renames leaves the installed path empty
/// and `shina.old.exe` in place; the next launch's `remove_replaced_exe` restores it.
pub fn commit_exe(prepared: &Path, dir: &Path) -> Result<PathBuf, String> {
    let target = dir.join("shina.exe");
    if same_file(prepared, &target) {
        return Ok(target);
    }
    let _lock = InstallLock::acquire(dir)?;
    if !prepared.is_file() {
        return Err(
            "The prepared update is missing. Shina will install it on the next launch.".into(),
        );
    }
    let old = previous_exe(dir);
    let installed = target.exists() || target.is_symlink();
    if installed {
        let _ = fs::remove_file(&old);
        fs::rename(&target, &old)
            .map_err(|e| format!("Could not move the previous Shina aside: {e}"))?;
        // The moved name is visible at once, but its contents reach the disk only later.
        sync_directory(dir);
    }
    // Tests replace this move, so the rollback below runs for a real second-rename failure.
    #[cfg(test)]
    let installed_by = FINAL_RENAME.with(|rename| {
        rename
            .borrow()
            .as_ref()
            .map_or_else(|| fs::rename(prepared, &target), |rename| rename(prepared))
    });
    #[cfg(not(test))]
    let installed_by = fs::rename(prepared, &target);
    match installed_by {
        Ok(()) => Ok(target),
        Err(error) => {
            let detail = format!("Could not install Shina: {error}");
            let restored = installed.then_some(old.as_path());
            #[cfg(test)]
            let restored = RESTORE.with(|restore| {
                let restore = restore.borrow();
                match restore.as_ref() {
                    Some(fail) => match fail(restored, &target) {
                        Ok(()) => Ok(()),
                        Err(path) => Err(path),
                    },
                    None => restore_prepared(restored, &target),
                }
            });
            #[cfg(not(test))]
            let restored = restore_prepared(restored, &target);
            match restored {
                Ok(()) => Err(detail),
                Err(still) => Err(format!(
                    "{detail}. The previous Shina is still at {}",
                    still.display()
                )),
            }
        }
    }
}

/// Windows: puts the previous executable back after a replacement that could not start. The
/// process that relaunched has exited, so nothing holds `shina.exe` open.
pub fn restore_exe(dir: &Path, running: &Path) -> Result<(), String> {
    let _lock = InstallLock::acquire(dir)?;
    let target = dir.join("shina.exe");
    let old = previous_exe(dir);
    // A copy that did not start becomes the recovery copy when possible. Whatever runs from
    // the installed path keeps `shina.exe` in place, and a restore below overwrites it.
    if target.exists() && !same_file(running, &target) && fs::rename(&target, &old).is_err() {
        let _ = fs::remove_file(&target);
    }
    // A rename only reaches the disk with this, and a build that did not start may be gone.
    sync_directory(dir);
    let restored = if old.is_file() {
        restore_prepared(Some(&old), &target)
    } else {
        Ok(())
    };
    sync_directory(dir);
    restored.map_err(|path| {
        format!(
            "Could not put the previous Shina back; it is still at {}",
            path.display()
        )
    })
}

/// Makes a folder's renames durable. Without it a rename survives in memory but not after a
/// crash, and a reopened app can find a name whose contents never reached the disk.
pub fn sync_directory(dir: &Path) {
    if let Ok(directory) = fs::File::open(dir) {
        let _ = directory.sync_all();
    }
}

/// Replaces `target` with `source`, when there is one. Returns the last path either side
/// ended at when the move fails, so the caller can say where to find it.
fn restore_prepared(source: Option<&Path>, target: &Path) -> Result<(), PathBuf> {
    let Some(source) = source.filter(|source| source.is_file()) else {
        return Ok(());
    };
    if target.exists() && fs::remove_file(target).is_err() {
        return Err(source.to_path_buf());
    }
    fs::rename(source, target).map_err(|_| source.to_path_buf())
}

fn same_contents(a: &Path, b: &Path) -> bool {
    let (Ok(mut a), Ok(mut b)) = (fs::File::open(a), fs::File::open(b)) else {
        return false;
    };
    let (mut left, mut right) = ([0u8; 64 * 1024], [0u8; 64 * 1024]);
    loop {
        let read = match a.read(&mut left) {
            Ok(read) => read,
            Err(_) => return false,
        };
        if b.read_exact(&mut right[..read]).is_err() {
            return false;
        }
        if read == 0 {
            return true;
        }
        if left[..read] != right[..read] {
            return false;
        }
    }
}

/// An exclusive lock over one directory of prepared files. A second update process fails to
/// take it instead of replacing the app twice.
pub struct InstallLock(#[allow(dead_code)] fs::File);

impl InstallLock {
    pub fn acquire(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        let lock = dir.join("install.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock)
            .map_err(|e| format!("Could not open {}: {e}", lock.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Self(file)),
            Err(_) => Err("Another Shina install is already running.".into()),
        }
    }
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

#[cfg(windows)]
fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

#[cfg(windows)]
fn powershell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Recreates the Start Menu shortcut and the Apps & Features entry for the installed build.
/// Runs on launch, so an update replaced the executable without repeating the copy-and-launch.
#[cfg(windows)]
pub fn refresh_windows_registration() -> Result<(), String> {
    let target = install_dir()
        .map(|dir| dir.join("shina.exe"))
        .ok_or("LOCALAPPDATA is not set.")?;
    let shortcut = start_menu_shortcut().ok_or("APPDATA is not set.")?;
    // Recreate rather than overwrite: launchers such as Caelestia watch the Start Menu folder
    // for added and removed files, and an in-place rewrite would leave them with a stale entry.
    let _ = fs::remove_file(&shortcut);
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
    Ok(())
}

#[cfg(not(windows))]
pub fn refresh_windows_registration() -> Result<(), String> {
    Ok(())
}

/// `shina --install`: copy this build into place, add the Start Menu shortcut, and register
/// an Apps & Features entry. Safe to run again; it refreshes everything.
pub fn install() -> Result<PathBuf, String> {
    let current = env::current_exe().map_err(|e| e.to_string())?;
    let target = place_exe(&current)?;
    refresh_windows_registration()?;
    Ok(target)
}

/// `shina --uninstall`: remove the shortcut and registration, then delete the install folder
/// once this process has exited. Thread indexes in `%LOCALAPPDATA%\Shina` are kept.
#[cfg(windows)]
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

#[cfg(not(windows))]
pub fn uninstall() -> Result<(), String> {
    Err("Shina installs as an app bundle on this platform.".into())
}

// ---- macOS bundles ----

/// The bundle's short version (`0.9.0`), read without parsing XML links or DTDs.
pub fn mac_bundle_version(bundle: &Path) -> Option<String> {
    read_plist_value(bundle, "CFBundleShortVersionString")
}

pub fn mac_bundle_id(bundle: &Path) -> Option<String> {
    read_plist_value(bundle, "CFBundleIdentifier")
}

fn read_plist_value(bundle: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(bundle.join("Contents").join("Info.plist")).ok()?;
    let after = text.split_once(&format!("<key>{key}</key>"))?.1;
    let value = after.split_once("<string>")?.1.split_once("</string>")?.0;
    Some(value.trim().to_owned())
}

/// The name of a prepared bundle's staging folder, so a later run can remove it.
const MACOS_STAGING: &str = ".Shina-update";

/// macOS: where a prepared bundle waits until commit. A sibling of the installed bundle,
/// so the final rename stays on one filesystem and nothing is prepared in the temp dir.
pub fn bundle_staging(app: &Path) -> PathBuf {
    let mut name = app
        .file_name()
        .map(|name| name.to_owned())
        .unwrap_or_default();
    name.push(format!("{MACOS_STAGING}.prepared"));
    app.with_file_name(name)
}

/// Removes a preparation that failed, so no partial bundle is left beside the app.
pub fn discard_prepared(app: &Path) {
    let _ = remove_tree(&bundle_staging(app));
}

/// The parent folder of an installed bundle, which an update may replace within.
pub fn mac_bundle_directory(app: &Path) -> Option<PathBuf> {
    Some(app.parent()?.to_path_buf())
}

/// Moves a verified `Shina.app` into `staging`, a sibling folder of the installed bundle so
/// commit is one rename. The bundle is copied, never moved, because the download may live on
/// another volume.
pub fn prepare_macos_bundle(source: &Path, staging: &Path) -> Result<PathBuf, String> {
    let _lock = InstallLock::acquire(staging)?;
    let prepared = staging.join(MACOS_APP_NAME);
    remove_tree(&prepared)?;
    let incoming = staging.join("Shina.app.incoming");
    remove_tree(&incoming)?;
    copy_tree(source, &incoming)?;
    fs::rename(&incoming, &prepared)
        .map_err(|e| format!("Could not prepare {}: {e}", prepared.display()))?;
    Ok(prepared)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|e| format!("Could not create {}: {e}", destination.display()))?;
    // A copy has to stay executable: fs::copy alone would keep only the read and write bits.
    let mode_of = |path: &Path| fs::metadata(path).map(|meta| meta.permissions());
    let set_mode = |permissions, path: &Path| fs::set_permissions(path, permissions);
    if let Ok(permissions) = mode_of(source) {
        let _ = set_mode(permissions, destination);
    }
    for entry in
        fs::read_dir(source).map_err(|e| format!("Could not read {}: {e}", source.display()))?
    {
        let entry = entry.map_err(|e| format!("Could not read {}: {e}", source.display()))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let kind = entry
            .file_type()
            .map_err(|e| format!("Could not read {}: {e}", from.display()))?;
        if kind.is_symlink() {
            return Err("The update bundle contains a link Shina will not follow.".into());
        }
        if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|e| format!("Could not copy {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

fn remove_tree(path: &Path) -> Result<(), String> {
    if path.is_symlink() || path.is_file() {
        return fs::remove_file(path)
            .map_err(|e| format!("Could not remove {}: {e}", path.display()));
    }
    if !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(|e| format!("Could not remove {}: {e}", path.display()))
}

/// The name of the previous bundle a macOS commit keeps beside the installed app.
const MACOS_PREVIOUS: &str = "Shina-app.previous";

/// macOS commit: move the installed bundle aside and the prepared one into its place.
/// The previous bundle stays as `Shina-app.previous` for recovery. A failed move restores it,
/// and a crash between the two moves is repaired by the next commit.
pub fn commit_macos_bundle(prepared: &Path, installed: &Path) -> Result<(), String> {
    let directory =
        mac_bundle_directory(installed).ok_or("The installed app has no parent folder.")?;
    let _lock = InstallLock::acquire(&directory.join(MACOS_STAGING))?;
    if !prepared.is_dir() {
        return Err(
            "The prepared update is missing. Shina will install it on the next launch.".into(),
        );
    }
    let previous = directory.join(MACOS_PREVIOUS);
    if !installed.exists() && previous.is_dir() {
        // An interrupted commit left the app moved aside: put it back first.
        fs::rename(&previous, installed)
            .map_err(|e| format!("Could not restore the previous Shina: {e}"))?;
    }
    let existing = installed.exists();
    if existing {
        remove_tree(&previous)?;
        fs::rename(installed, &previous)
            .map_err(|e| format!("Could not move the previous Shina aside: {e}"))?;
    }
    match fs::rename(prepared, installed) {
        Ok(()) => Ok(()),
        Err(error) => {
            let restored = !existing || fs::rename(&previous, installed).is_ok();
            let detail = format!("Could not install Shina: {error}");
            Err(if restored {
                detail
            } else {
                format!(
                    "{detail}. The new app is {}. Move {} back to {} by hand.",
                    prepared.display(),
                    previous.display(),
                    installed.display()
                )
            })
        }
    }
}

/// macOS: puts the previous bundle back when a replacement could not start. The new bundle
/// takes the previous one's place, so the recovery copy is never lost.
pub fn restore_mac_bundle(app: &Path) -> Result<(), String> {
    let directory = mac_bundle_directory(app).ok_or("The installed app has no parent folder.")?;
    let _lock = InstallLock::acquire(&directory.join(MACOS_STAGING))?;
    let previous = directory.join(MACOS_PREVIOUS);
    if !previous.is_dir() {
        return Err("There is no previous Shina to restore.".into());
    }
    let failed = directory.join(".Shina-update.failed");
    remove_tree(&failed)?;
    if app.exists() && fs::rename(app, &failed).is_err() {
        return Err("Could not move the new Shina aside.".into());
    }
    if let Err(error) = fs::rename(&previous, app) {
        let _ = fs::rename(&failed, app);
        return Err(format!("Could not restore the previous Shina: {error}"));
    }
    let kept = fs::rename(&failed, &previous);
    if kept.is_err() {
        remove_tree(&failed)?;
    }
    Ok(())
}

/// Opens the replaced bundle, as an explicit update restart does. `open` exits nonzero when
/// Launch Services cannot start the app, so its status is checked rather than the spawn.
pub fn open_mac_bundle(app: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("/usr/bin/open")
            .arg("-n")
            .arg(app)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("Could not open {}: {e}", app.display()))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("Could not open {}.", app.display()))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Opening a Mac app is only supported on macOS.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "shina-install-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn the_installed_target_is_the_app_the_executable_runs_from() {
        let root = temporary("target");
        let installed = InstalledTarget::format(&root.join("installed").join(binary_name()));
        let exe = executable_in(&installed);
        write(&exe, b"app");
        assert_eq!(
            target_for_executable(&exe, installed.path()),
            Some(installed.clone())
        );
        // A development build elsewhere never updates itself.
        let dev = root.join("target/release").join(executable_path());
        write(&dev, b"dev");
        assert_ne!(
            target_for_executable(&dev, installed.path()),
            Some(installed)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_target_for_accepts_the_installer_bundle_and_rejects_a_dev_build() {
        let root = temporary("bundle");
        let applications = root.join("Applications");
        let app = applications.join(MACOS_APP_NAME);
        let plist = |id: &str| {
            format!(
                "<dict><key>CFBundleIdentifier</key><string>{id}</string>\
                 <key>CFBundleShortVersionString</key><string>0.9.0</string></dict>"
            )
        };
        write(
            &app.join("Contents/Info.plist"),
            plist(MACOS_BUNDLE_ID).as_bytes(),
        );
        let exe = app.join(executable_path());
        write(&exe, b"app");
        let found = installed_target_for(&exe, None, Some(applications.clone()));
        if cfg!(windows) {
            assert!(found.is_none(), "a Windows host has no Mac bundle target");
        } else {
            assert_eq!(found, Some(InstalledTarget::MacBundle { app: app.clone() }));
        }
        assert_eq!(
            installed_target_for(
                &root.join("target/release").join(executable_path()),
                None,
                Some(applications.clone())
            ),
            None,
            "a development executable is not installed"
        );
        write(
            &app.join("Contents/Info.plist"),
            plist("com.example.other").as_bytes(),
        );
        assert_eq!(installed_target_for(&exe, None, Some(applications)), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn asset_names_follow_the_platform_and_architecture() {
        let windows = InstalledTarget::Windows {
            exe: PathBuf::from("shina.exe"),
        };
        assert_eq!(windows.asset(), "shina.exe");
        let mac = InstalledTarget::MacBundle {
            app: PathBuf::from("/Applications/Shina.app"),
        };
        let expected = if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
            "Shina-macos-x86_64.zip"
        } else {
            "Shina-macos-aarch64.zip"
        };
        assert_eq!(mac.asset(), expected);
    }

    #[test]
    fn replacement_swaps_the_executable_and_keeps_the_previous_one() {
        let dir = temporary("replace");
        let target = dir.join("shina.exe");
        write(&target, b"old");
        let download = dir.join("download");
        write(&download, b"new build");
        let staged = prepare_exe(&download, &dir).unwrap();
        assert_eq!(
            fs::read(&target).unwrap(),
            b"old",
            "prepare never touches the install"
        );
        assert_eq!(commit_exe(&staged, &dir).unwrap(), target);
        assert_eq!(fs::read(&target).unwrap(), b"new build");
        assert_eq!(fs::read(previous_exe(&dir)).unwrap(), b"old");
        assert!(!staged.exists(), "the prepared copy is moved into place");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_second_rename_restores_the_previous_build() {
        let root = temporary("rollback");
        let dir = root.join("install");
        let target = dir.join("shina.exe");
        write(&target, b"old");
        let download = root.join("download");
        write(&download, b"new");
        let staged = prepare_exe(&download, &dir).unwrap();
        // The second rename fails after the previous build is aside, as a locked or full
        // folder makes it fail. The restore runs for real and puts the old build back.
        let locked = |_: &Path| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "shina.exe is locked",
            ))
        };
        FINAL_RENAME.with(|rename| *rename.borrow_mut() = Some(Box::new(locked)));
        let error = commit_exe(&staged, &dir).unwrap_err();
        FINAL_RENAME.with(|rename| *rename.borrow_mut() = None);
        assert!(error.contains("Could not install Shina"), "{error}");
        assert!(!error.contains("by hand"), "the restore succeeded: {error}");
        assert_eq!(
            fs::read(&target).unwrap(),
            b"old",
            "the previous build is back"
        );
        assert!(
            !dir.join("shina.old.exe").exists(),
            "the restore moved it back into place"
        );
        assert!(
            staged.is_file(),
            "the prepared copy is kept for the next quit"
        );
        assert_eq!(commit_exe(&staged, &dir).unwrap(), target);
        assert_eq!(fs::read(&target).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_rollback_that_also_fails_says_where_the_previous_build_is() {
        let dir = temporary("rollback-failed");
        let target = dir.join("shina.exe");
        write(&target, b"old");
        write(&dir.join("download"), b"new");
        let staged = prepare_exe(&dir.join("download"), &dir).unwrap();
        FINAL_RENAME.with(|rename| {
            *rename.borrow_mut() = Some(Box::new(|_: &Path| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "locked",
                ))
            }));
        });
        // Both moves are denied, so the error has to name the recovery copy.
        RESTORE.with(|restore| {
            *restore.borrow_mut() = Some(Box::new(|_: Option<&Path>, _: &Path| {
                Err(PathBuf::from("C:/fixture/Shina/shina.old.exe"))
            }));
        });
        let error = commit_exe(&staged, &dir).unwrap_err();
        FINAL_RENAME.with(|rename| *rename.borrow_mut() = None);
        RESTORE.with(|restore| *restore.borrow_mut() = None);
        assert!(error.contains("Could not install Shina"), "{error}");
        assert!(error.contains("shina.old.exe"), "{error}");
        assert!(staged.is_file());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_locked_install_keeps_the_prepared_update_and_commits_once() {
        let dir = temporary("locked");
        let target = dir.join("shina.exe");
        write(&target, b"old");
        let download = dir.join("download");
        write(&download, b"build");
        let staged = prepare_exe(&download, &dir).unwrap();
        let held = InstallLock::acquire(&dir).unwrap();
        assert!(commit_exe(&staged, &dir).is_err(), "a second install waits");
        assert!(staged.is_file(), "the download is retained");
        assert_eq!(fs::read(&target).unwrap(), b"old");
        drop(held);
        commit_exe(&staged, &dir).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"build");
        assert_eq!(fs::read(dir.join("shina.old.exe")).unwrap(), b"old");
        // Nothing prepared is left, so a second commit cannot repeat the replacement.
        assert!(commit_exe(&staged, &dir).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"build");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_prepare_mismatch_is_not_kept() {
        let dir = temporary("copy");
        let source = dir.join("source.exe");
        write(&source, b"one");
        let staged = prepare_exe(&source, &dir).unwrap();
        assert!(same_contents(&staged, &source));
        write(&source, b"two");
        assert!(!same_contents(&staged, &source));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mac_bundles_swap_as_whole_directories_and_keep_the_previous_one() {
        let root = temporary("mac");
        let applications = root.join("Applications");
        let installed = applications.join(MACOS_APP_NAME);
        write(&installed.join(executable_path()), b"old");
        let staging = bundle_staging(&installed);
        assert_eq!(staging.parent(), Some(applications.as_path()));
        let extracted = root.join("extracted/Shina.app");
        write(&extracted.join(executable_path()), b"new");
        let prepared = prepare_macos_bundle(&extracted, &bundle_staging(&installed)).unwrap();
        assert_eq!(fs::read(installed.join(executable_path())).unwrap(), b"old");
        commit_macos_bundle(&prepared, &installed).unwrap();
        assert_eq!(fs::read(installed.join(executable_path())).unwrap(), b"new");
        assert_eq!(
            fs::read(
                applications
                    .join("Shina-app.previous")
                    .join(executable_path())
            )
            .unwrap(),
            b"old"
        );
        // The next commit repairs an install a crash left moved aside.
        write(
            &applications.join("Shina-app.previous/Contents/stale"),
            b"old",
        );
        let next = root.join("next/Shina.app");
        write(&next.join(executable_path()), b"newer");
        let prepared = prepare_macos_bundle(&next, &bundle_staging(&installed)).unwrap();
        fs::remove_dir_all(applications.join("Shina-app.previous")).unwrap();
        commit_macos_bundle(&prepared, &installed).unwrap();
        assert_eq!(
            fs::read(installed.join(executable_path())).unwrap(),
            b"newer"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_exe_that_cannot_start_is_replaced_by_the_previous_one() {
        let root = temporary("restore-exe");
        let dir = root.join("install");
        let installed = dir.join("shina.exe");
        write(&installed, b"broken");
        write(&dir.join("shina.old.exe"), b"old");
        // The relaunched process is gone, so nothing holds the broken copy open.
        restore_exe(&dir, &installed).unwrap();
        assert_eq!(
            fs::read(&installed).ok(),
            Some(b"old".to_vec()),
            "the previous build is back at the installed path"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_broken_mac_bundle_is_replaced_by_the_previous_one() {
        let root = temporary("restore-mac");
        let applications = root.join("Applications");
        let installed = applications.join(MACOS_APP_NAME);
        write(&installed.join(executable_path()), b"broken");
        write(
            &applications
                .join("Shina-app.previous")
                .join(executable_path()),
            b"old",
        );
        restore_mac_bundle(&installed).unwrap();
        assert_eq!(fs::read(installed.join(executable_path())).unwrap(), b"old");
        assert_eq!(
            fs::read(
                applications
                    .join("Shina-app.previous")
                    .join(executable_path())
            )
            .unwrap(),
            b"broken",
            "the broken build becomes the recovery copy"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
