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

/// The installed target an executable belongs to, if it runs from `installed`.
fn target_for_executable(current: &Path, installed: &Path) -> Option<InstalledTarget> {
    let target = InstalledTarget::format(installed);
    if !same_file(current, &target.path().join(executable_path())) {
        return None;
    }
    Some(target)
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

/// Removes the executable an update or reinstall replaced. It could not be deleted while it
/// was running, so the next launch cleans it up.
#[cfg(windows)]
pub fn remove_replaced_exe() {
    remove_replaced_exe_from(env::current_exe().ok().as_deref());
}

#[cfg(not(windows))]
pub fn remove_replaced_exe() {}

#[cfg(windows)]
fn remove_replaced_exe_from(current: Option<&Path>) {
    if let Some(current) = current
        && let Some(dir) = current.parent()
    {
        let _ = fs::remove_file(dir.join("shina.old.exe"));
    }
}

/// Windows: the installed build renamed aside during a replacement.
fn previous_exe() -> PathBuf {
    install_dir()
        .unwrap_or_else(env::temp_dir)
        .join("shina.old.exe")
}

/// Puts `source` at the installed path now: prepare, then commit. `--install` uses this.
pub fn place_exe(source: &Path) -> Result<PathBuf, String> {
    let dir = install_dir().ok_or("LOCALAPPDATA is not set.")?;
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let staged = prepare_exe(source, &dir)?;
    commit_exe(&staged, &dir)
}

/// Windows prepare: copy the download beside the installed executable and verify the copy.
pub fn prepare_exe(source: &Path, dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let staged = dir.join("shina.new.exe");
    let _ = fs::remove_file(&staged);
    fs::copy(source, &staged).map_err(|e| format!("Could not copy Shina: {e}"))?;
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
/// but not overwriting it.
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
    let old = previous_exe();
    let installed = target.exists() || target.is_symlink();
    if installed {
        let _ = fs::remove_file(&old);
        fs::rename(&target, &old)
            .map_err(|e| format!("Could not move the previous Shina aside: {e}"))?;
    }
    match fs::rename(prepared, &target) {
        Ok(()) => Ok(target),
        Err(error) => {
            if installed {
                // Restore the previous build so the next launch still starts Shina.
                let _ = fs::rename(&old, &target);
            }
            Err(format!("Could not install Shina: {error}"))
        }
    }
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

/// macOS: where a prepared bundle waits until commit.
pub fn bundle_staging() -> PathBuf {
    install_dir()
        .unwrap_or_else(env::temp_dir)
        .join("macos-update")
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
    if !path.exists() && !path.is_symlink() {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(|e| format!("Could not remove {}: {e}", path.display()))
}

/// macOS commit: move the installed bundle aside and the prepared one into its place.
/// The previous bundle stays as `Shina-app.previous` for recovery. A failed move restores it.
pub fn commit_macos_bundle(prepared: &Path, installed: &Path) -> Result<(), String> {
    let directory =
        mac_bundle_directory(installed).ok_or("The installed app has no parent folder.")?;
    let _lock = InstallLock::acquire(&directory.join(".Shina-update"))?;
    if !prepared.is_dir() {
        return Err(
            "The prepared update is missing. Shina will install it on the next launch.".into(),
        );
    }
    let previous = directory.join("Shina-app.previous");
    remove_tree(&previous)?;
    if installed.exists() {
        fs::rename(installed, &previous)
            .map_err(|e| format!("Could not move the previous Shina aside: {e}"))?;
    }
    match fs::rename(prepared, installed) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::rename(&previous, installed);
            Err(format!("Could not install Shina: {error}"))
        }
    }
}

/// Opens the replaced bundle, as an explicit update restart does.
pub fn open_mac_bundle(app: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/bin/open")
            .arg("-n")
            .arg(app)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(drop)
            .map_err(|e| format!("Could not open {}: {e}", app.display()))
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
        let exe = installed.path().join(executable_path());
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
        assert_eq!(fs::read(previous_exe()).unwrap(), b"old");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_second_rename_restores_the_previous_build() {
        let dir = temporary("rollback");
        let target = dir.join("shina.exe");
        write(&target, b"old");
        // A directory where the prepared file should be makes the second rename fail.
        let prepared = dir.join("prepared");
        fs::create_dir_all(prepared.join("shina.exe")).unwrap();
        assert!(commit_exe(&prepared.join("shina.exe"), &dir).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
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
        assert_eq!(fs::read(previous_exe()).unwrap(), b"old");
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
        let staging = root.join("staging");
        let extracted = root.join("extracted/Shina.app");
        write(&extracted.join(executable_path()), b"new");
        let prepared = prepare_macos_bundle(&extracted, &staging).unwrap();
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
        fs::remove_dir_all(root).unwrap();
    }
}
