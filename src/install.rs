//! A per-user install with a stable path, so Start pins, the Caelestia launcher, and in-app
//! updates all point at the same executable. Nothing here needs administrator rights.
//!
//! Each platform has the same two update steps. Prepare copies a verified download next to the
//! installed app before the update is offered. Commit runs after Rune and Pi shut down.
//! Windows waits for the original process to exit, then uses ReplaceFileW with a backup.
//! macOS exchanges whole bundles atomically. Neither removes the installed path first.
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
        // Registration belongs to install/update, never the window's startup path.
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
    Linux { exe: PathBuf },
}

impl InstalledTarget {
    /// The Windows executable or the macOS bundle directory.
    pub fn path(&self) -> &Path {
        match self {
            Self::Windows { exe } | Self::Linux { exe } => exe,
            Self::MacBundle { app } => app,
        }
    }
    /// The release asset this build installs from, for the running architecture.
    pub fn asset(&self) -> &'static str {
        if matches!(self, Self::Linux { .. }) {
            return "Shina-linux-x86_64.tar.gz";
        }
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
        InstalledTarget::Windows { exe } | InstalledTarget::Linux { exe } => exe.clone(),
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

#[cfg(test)]
type Rename = dyn Fn(&Path) -> std::io::Result<()>;

#[cfg(test)]
thread_local! {
    /// Fails the atomic replacement before it changes either path.
    static FINAL_RENAME: std::cell::RefCell<Option<Box<Rename>>> =
        const { std::cell::RefCell::new(None) };
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
    fs::OpenOptions::new()
        .write(true)
        .open(&staged)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("Could not flush the prepared Shina: {e}"))?;
    Ok(staged)
}

/// Replace the executable without removing its launch path first. On Windows a helper
/// runs this after the UI process exits, so the mapped executable does not block replacement.
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
    #[cfg(test)]
    FINAL_RENAME
        .with(|rename| {
            rename
                .borrow()
                .as_ref()
                .map_or(Ok(()), |fail| fail(prepared))
        })
        .map_err(|e| format!("Could not install Shina: {e}"))?;
    replace_exe(prepared, &target, &old).map_err(|e| format!("Could not install Shina: {e}"))?;
    sync_directory(dir);
    Ok(target)
}

#[cfg(windows)]
fn replace_exe(source: &Path, target: &Path, backup: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }
    if !target.exists() {
        return fs::rename(source, target);
    }
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let (source, target, backup) = (wide(source), wide(target), wide(backup));
    // ReplaceFileW preserves a backup and does not expose a two-rename launch-path gap.
    let result = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            source.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_exe(source: &Path, target: &Path, backup: &Path) -> std::io::Result<()> {
    if target.exists() {
        fs::copy(target, backup)?;
    }
    fs::rename(source, target)
}

/// Restore by replacing the failed executable atomically, preserving it separately.
pub fn restore_exe(dir: &Path, _running: &Path) -> Result<(), String> {
    let _lock = InstallLock::acquire(dir)?;
    let target = dir.join("shina.exe");
    let old = previous_exe(dir);
    if !old.is_file() {
        return Err("There is no previous Shina executable to restore.".into());
    }
    replace_exe(&old, &target, &dir.join("shina.failed.exe"))
        .map_err(|e| format!("Could not restore the previous Shina: {e}"))?;
    sync_directory(dir);
    Ok(())
}

/// Best-effort directory flush. Some Windows filesystems do not support flushing directory
/// handles; a successful executable replacement is not a power-loss durability guarantee.
pub fn sync_directory(dir: &Path) {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x02000000); // FILE_FLAG_BACKUP_SEMANTICS
    }
    if let Ok(directory) = options.open(dir) {
        let _ = directory.sync_all();
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
            return b.read(&mut right[..1]).is_ok_and(|read| read == 0);
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
    launch_child(exe, args).map(drop)
}

pub fn launch_child(exe: &Path, args: &[&str]) -> Result<std::process::Child, String> {
    let mut command = Command::new(exe);
    #[cfg(windows)]
    hidden(&mut command);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", exe.display()))
}

pub fn report_notice(message: &str) {
    if let Some(directory) = crate::store::directory() {
        let _ = fs::create_dir_all(&directory);
        let _ = fs::write(
            directory.join("startup-warning.txt"),
            crate::update::redact(message),
        );
    }
}

#[cfg(windows)]
pub fn show_uninstall_error(message: &str) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            window: *mut std::ffi::c_void,
            text: *const u16,
            title: *const u16,
            flags: u32,
        ) -> i32;
    }
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Shina uninstall failed"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10);
    }
}

#[cfg(windows)]
fn registration_command(command: &mut Command) -> Result<std::process::ExitStatus, String> {
    crate::process::output(command, std::time::Duration::from_secs(15), 64 * 1024)
        .map(|output| output.status)
        .map_err(|e| e.to_string())
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
/// Runs only during install or update, never before opening the window.
#[cfg(windows)]
pub fn refresh_windows_registration() -> Result<(), String> {
    let target = install_dir()
        .map(|dir| dir.join("shina.exe"))
        .ok_or("LOCALAPPDATA is not set.")?;
    let shortcut = start_menu_shortcut().ok_or("APPDATA is not set.")?;
    let staged_shortcut = shortcut.with_file_name("Shina.new.lnk");
    let home = env::var("USERPROFILE").unwrap_or_default();
    let script = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut({}); $s.TargetPath = {}; \
         $s.WorkingDirectory = {}; $s.IconLocation = {}; $s.Description = 'Shina'; $s.Save()",
        powershell_quote(&staged_shortcut.to_string_lossy()),
        powershell_quote(&target.to_string_lossy()),
        powershell_quote(&home),
        powershell_quote(&format!("{},0", target.to_string_lossy())),
    );
    let status = registration_command(Command::new("powershell").args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]))
    .map_err(|e| format!("Could not create the Start Menu shortcut: {e}"))?;
    if !status.success() {
        return Err("Could not create the Start Menu shortcut.".into());
    }
    let helper = target.with_file_name("set-app-id.ps1");
    fs::write(&helper, SET_APP_ID)
        .map_err(|e| format!("Could not write {}: {e}", helper.display()))?;
    let status = registration_command(
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&helper)
            .arg("-Shortcut")
            .arg(&staged_shortcut)
            .args(["-AppId", APP_ID]),
    )
    .map_err(|e| format!("Could not set the shortcut's app ID: {e}"))?;
    if !status.success() {
        return Err("Could not set the shortcut's app ID.".into());
    }
    fs::rename(&staged_shortcut, &shortcut)
        .map_err(|e| format!("Could not replace the Start Menu shortcut: {e}"))?;
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
        let status = registration_command(Command::new("reg").args([
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
    if let Err(error) = refresh_windows_registration() {
        report_notice(&format!(
            "Shina installed, but Windows registration failed: {error}"
        ));
    }
    Ok(target)
}

/// `shina --uninstall`: remove the shortcut and registration, then delete the install folder
/// once this process has exited. Thread indexes in `%LOCALAPPDATA%\Shina` are kept.
#[cfg(windows)]
pub fn uninstall() -> Result<(), String> {
    let root = crate::store::directory().ok_or("No Shina data directory.")?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let helper = root.join("uninstall-helper.exe");
    fs::copy(env::current_exe().map_err(|e| e.to_string())?, &helper).map_err(|e| e.to_string())?;
    let marker = root.join("uninstall-started");
    match fs::remove_file(&marker) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not reset the uninstall handshake: {error}")),
    }
    let mut child = launch_child(
        &helper,
        &["--finish-uninstall", &std::process::id().to_string()],
    )?;
    wait_for_marker(&mut child, &marker, std::time::Duration::from_secs(10))
}

#[cfg(windows)]
pub fn wait_for_marker(
    child: &mut std::process::Child,
    marker: &Path,
    timeout: std::time::Duration,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if marker.is_file() {
            return Ok(());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let mut details = String::new();
            if let Some(directory) = crate::store::directory()
                && let Ok(file) = fs::File::open(directory.join("startup-warning.txt"))
            {
                let _ = file.take(4096).read_to_string(&mut details);
            }
            return Err(format!(
                "The Shina helper exited before acknowledging: {status}. {details}"
            ));
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = crate::process::wait_child(child, std::time::Duration::from_secs(1));
            return Err("The Shina helper did not acknowledge in time.".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(windows)]
pub fn finish_uninstall() -> Result<(), String> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, id: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let root = crate::store::directory().ok_or("No Shina data directory.")?;
    if env::current_exe().map_err(|e| e.to_string())? != root.join("uninstall-helper.exe") {
        return Err("Only the staged helper may finish uninstall.".into());
    }
    let target = install_dir().ok_or("No install folder.")?;
    // Never recurse into a redirected, relative, or unexpected directory.
    let metadata = fs::symlink_metadata(&target).map_err(|e| e.to_string())?;
    if !target.is_absolute()
        || metadata.file_type().is_symlink()
        || target.file_name().and_then(|name| name.to_str()) != Some("Shina")
        || target
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("Programs")
    {
        return Err("Unsafe uninstall target.".into());
    }
    let base = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("No local app directory.")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let expected = crate::projects::clean_path(&base.join("Programs").join("Shina"));
    let resolved = crate::projects::clean_path(&target.canonicalize().map_err(|e| e.to_string())?);
    if !resolved
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected.to_string_lossy())
    {
        return Err(
            "The install directory resolves outside its expected location. No files were removed."
                .into(),
        );
    }
    let parent = env::args()
        .nth(2)
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|&id| id != 0 && id != std::process::id())
        .ok_or("Invalid uninstall parent.")?;
    let handle = unsafe { OpenProcess(0x00100000, 0, parent) };
    if handle.is_null() {
        return Err("Could not open the uninstall parent.".into());
    }
    let acknowledged = fs::write(root.join("uninstall-started"), b"ready");
    let result = if acknowledged.is_ok() {
        unsafe { WaitForSingleObject(handle, 30_000) }
    } else {
        u32::MAX
    };
    unsafe {
        CloseHandle(handle);
    }
    if result != 0 {
        return Err("Shina did not exit. Registration was preserved.".into());
    }
    fs::remove_dir_all(&resolved).map_err(|e| {
        format!(
            "Could not remove {}: {e}. Registration was preserved. Close all Shina windows and retry uninstall.",
            target.display()
        )
    })?;
    if target.exists() {
        return Err("The install folder still exists. Retry uninstall.".into());
    }
    if let Some(shortcut) = start_menu_shortcut() {
        let _ = fs::remove_file(shortcut);
    }
    let status = registration_command(Command::new("reg").args(["delete", UNINSTALL_KEY, "/f"]))?;
    if !status.success() {
        return Err("The files were removed but Windows registration could not be removed.".into());
    }
    let _ = fs::remove_file(root.join("uninstall-started"));
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
    sync_prepared_bundle(&prepared)?;
    Ok(prepared)
}

/// Flush payload files before an atomic exchange can make this bundle the installed app.
pub fn sync_prepared_bundle(bundle: &Path) -> Result<(), String> {
    for entry in fs::read_dir(bundle).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err("The prepared bundle contains a link.".into());
        }
        if kind.is_dir() {
            sync_prepared_bundle(&entry.path())?;
        } else {
            fs::File::open(entry.path())
                .and_then(|file| file.sync_all())
                .map_err(|e| format!("Could not flush the prepared bundle: {e}"))?;
        }
    }
    fs::File::open(bundle)
        .and_then(|file| file.sync_all())
        .map_err(|e| format!("Could not flush the prepared bundle folder: {e}"))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|e| format!("Could not create {}: {e}", destination.display()))?;
    // Preserve directory permissions as well as the file permissions copied by fs::copy.
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

/// macOS commit keeps the installed path present throughout. Before the atomic exchange,
/// previous contains the new build; after it, previous contains the old build.
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
    if !installed.is_dir() {
        return Err("The installed Shina bundle is missing; no replacement was attempted.".into());
    }
    remove_tree(&previous)?;
    fs::rename(prepared, &previous)
        .map_err(|e| format!("Could not stage the replacement Shina: {e}"))?;
    match exchange_paths(&previous, installed) {
        Ok(()) => {
            sync_directory(&directory);
            Ok(())
        }
        Err(error) => {
            let retained = fs::rename(&previous, prepared);
            Err(format!(
                "Could not exchange Shina bundles: {error}. The installed app is unchanged.{}",
                if retained.is_err() {
                    " The prepared update remains at Shina-app.previous."
                } else {
                    ""
                }
            ))
        }
    }
}

#[cfg(target_os = "macos")]
fn exchange_paths(left: &Path, right: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let left = CString::new(left.as_os_str().as_bytes())?;
    let right = CString::new(right.as_os_str().as_bytes())?;
    // Fail closed on filesystems without atomic exchange. Never fall back to two renames.
    let result = unsafe { libc::renamex_np(left.as_ptr(), right.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn exchange_paths(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Atomic bundle exchange requires macOS",
    ))
}

/// Atomically puts the previous bundle back, keeping the failed build at previous.
pub fn restore_mac_bundle(app: &Path) -> Result<(), String> {
    let directory = mac_bundle_directory(app).ok_or("The installed app has no parent folder.")?;
    let _lock = InstallLock::acquire(&directory.join(MACOS_STAGING))?;
    let previous = directory.join(MACOS_PREVIOUS);
    if !previous.is_dir() {
        return Err("There is no previous Shina to restore.".into());
    }
    exchange_paths(&previous, app).map_err(|e| format!("Could not restore the previous Shina: {e}"))
}

/// Opens the replaced bundle, as an explicit update restart does. `open` exits nonzero when
/// Launch Services cannot start the app, so its status is checked rather than the spawn.
pub fn open_mac_bundle(app: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // This launcher intentionally starts a long-lived app, so it must not own a
        // kill-on-drop process tree. Only the launcher's wait is bounded.
        let mut child = Command::new("/usr/bin/open")
            .arg("-n")
            .arg(app)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not open {}: {e}", app.display()))?;
        let status =
            match crate::process::wait_child(&mut child, std::time::Duration::from_secs(30)) {
                Ok(status) => status,
                Err(error) => {
                    let _ = child.kill();
                    let _ =
                        crate::process::wait_child(&mut child, std::time::Duration::from_secs(1));
                    return Err(format!(
                        "Could not finish opening {}: {error}",
                        app.display()
                    ));
                }
            };
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

    #[cfg(windows)]
    #[test]
    fn helper_exit_is_detected_without_waiting_for_the_handshake_timeout() {
        let root = crate::store::directory().unwrap();
        let mut command = Command::new("cmd");
        command.args(["/C", "exit", "1"]);
        let mut child = crate::process::Process::spawn(&mut command).unwrap();
        let start = std::time::Instant::now();
        assert!(
            wait_for_marker(
                &mut child,
                &root.join("no-marker"),
                std::time::Duration::from_secs(20)
            )
            .is_err()
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
    }
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
    fn a_failed_atomic_replacement_keeps_the_launch_path_and_prepared_build() {
        let root = temporary("rollback");
        let dir = root.join("install");
        let target = dir.join("shina.exe");
        write(&target, b"old");
        let download = root.join("download");
        write(&download, b"new");
        let staged = prepare_exe(&download, &dir).unwrap();
        // Fail the actual commit operation after preparation, before any path changes.
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
        assert_eq!(
            fs::read(&target).unwrap(),
            b"old",
            "the previous build never left its launch path"
        );
        assert!(
            !dir.join("shina.old.exe").exists(),
            "no rename-aside operation occurred"
        );
        assert!(
            staged.is_file(),
            "the prepared copy is kept for another restart attempt"
        );
        assert_eq!(commit_exe(&staged, &dir).unwrap(), target);
        assert_eq!(fs::read(&target).unwrap(), b"new");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restoring_without_a_backup_fails_and_leaves_the_current_executable_unchanged() {
        let dir = temporary("missing-backup");
        let target = dir.join("shina.exe");
        write(&target, b"current");
        let error = restore_exe(&dir, &target).unwrap_err();
        assert!(error.contains("no previous"), "{error}");
        assert_eq!(fs::read(&target).unwrap(), b"current");
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
    #[cfg(target_os = "macos")]
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
        // Simulate interruption after staging the incoming bundle at previous, before
        // atomic exchange. The original app must still be directly launchable.
        let next = root.join("next/Shina.app");
        write(&next.join(executable_path()), b"newer");
        let prepared = prepare_macos_bundle(&next, &bundle_staging(&installed)).unwrap();
        fs::remove_dir_all(applications.join("Shina-app.previous")).unwrap();
        fs::rename(&prepared, applications.join("Shina-app.previous")).unwrap();
        assert_eq!(fs::read(installed.join(executable_path())).unwrap(), b"new");
        exchange_paths(&applications.join("Shina-app.previous"), &installed).unwrap();
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
    #[cfg(target_os = "macos")]
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
