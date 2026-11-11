//! In-app updates from GitHub Releases. The repository is private, so downloads go through
//! the user's existing `gh` CLI login; Shina never reads or stores a GitHub token.
//!
//! An installed copy checks in the background, downloads and verifies a newer build into its
//! own staging area, and offers one restart. A plain quit keeps the prepared update.
//! Failures stay quiet: the status returns to idle and the next interval
//! retries.
use rune::Waker;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::install::{self, InstalledTarget};

pub const REPOSITORY: &str = "KldsSeeGhosts/shina";
const SHA256SUMS: &str = "SHA256SUMS";
/// The first check runs after this delay. Then a finished check schedules the next one.
pub const FIRST_CHECK: Duration = Duration::from_secs(2);
pub const INTERVAL: Duration = Duration::from_secs(30);
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_DIAGNOSTICS: usize = 10;
/// The most an archive listing may take before the archive is refused, so a listing that
/// `run_bounded` would cut short can never hide an entry from validation.
const MAX_ARCHIVE_LISTING: usize = 64 * 1024 * 1024;

/// What an installed copy shows, and what it waits to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// This build never updates itself: a development run, an unsupported platform, or a demo.
    Disabled,
    /// No update yet, no newer release, or the last attempt failed. The next check is scheduled.
    Idle,
    /// Asking GitHub for the latest release.
    Checking,
    /// Downloading, verifying and preparing a newer release.
    Downloading { tag: String },
    /// A verified replacement waits for an explicit restart.
    Ready { update: PreparedUpdate },
}

/// A verified replacement. Its manifest is written only after verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedUpdate {
    pub tag: String,
    pub target: InstalledTarget,
    pub asset: String,
    pub size: u64,
    pub sha256: String,
    /// The downloaded asset, reverified before the install.
    pub download: PathBuf,
    /// The verified replacement on the target filesystem.
    pub prepared: PathBuf,
}

impl PreparedUpdate {
    fn to_value(&self) -> Value {
        serde_json::json!({
            "version": 1, "tag": self.tag, "asset": self.asset, "size": self.size,
            "sha256": self.sha256, "download": self.download, "prepared": self.prepared,
            "target": match &self.target {
                InstalledTarget::Windows { exe } => serde_json::json!({"windows": exe}),
                InstalledTarget::MacBundle { app } => serde_json::json!({"macos": app}),
            },
        })
    }

    fn from_value(value: &Value) -> Option<Self> {
        if value["version"] != 1 {
            return None;
        }
        let target = if let Some(exe) = value["target"]["windows"].as_str() {
            InstalledTarget::Windows {
                exe: PathBuf::from(exe),
            }
        } else {
            InstalledTarget::MacBundle {
                app: PathBuf::from(value["target"]["macos"].as_str()?),
            }
        };
        Some(Self {
            tag: value["tag"].as_str()?.to_owned(),
            target,
            asset: value["asset"].as_str()?.to_owned(),
            size: value["size"].as_u64().filter(|size| *size > 0)?,
            sha256: value["sha256"].as_str()?.to_owned(),
            download: PathBuf::from(value["download"].as_str()?),
            prepared: PathBuf::from(value["prepared"].as_str()?),
        })
    }

    /// The executable, or the bundle an explicit restart opens.
    pub fn launcher(&self) -> PathBuf {
        match &self.target {
            InstalledTarget::Windows { exe } => exe.clone(),
            InstalledTarget::MacBundle { app } => app.clone(),
        }
    }
}

/// Why a manifest's update cannot install itself here, or `None` when it can. A manifest
/// from a test folder, another app folder or another platform is never installed.
fn update_reaches_the_installed_app(update: &PreparedUpdate, target: &InstalledTarget) -> bool {
    update.target == *target
        && update.asset == target.asset()
        && hex_sha256(&update.sha256).is_some()
        && is_newer(&update.tag, env!("CARGO_PKG_VERSION"))
        && update.prepared.exists()
        && update.download.is_file()
}

/// A newer release tag and the asset this build installs from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub asset: ReleaseAsset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub size: u64,
    /// SHA-256 as GitHub reports it (`sha256:<hex>`), when it reports one.
    pub digest: Option<String>,
}

/// Why an attempt failed. Quiet failures only become a diagnostic. An authentication denial
/// stops networked attempts for the rest of this app session.
#[derive(Clone, Debug, PartialEq)]
enum Failure {
    Quiet(String),
    Auth(String),
}

impl Failure {
    fn quiet(message: impl Into<String>) -> Self {
        Self::Quiet(message.into())
    }
}

/// A finished worker operation. Typed, so the UI never sees a half-built status.
enum Event {
    /// A manifest from a previous run, revalidated before it is exposed.
    Recovered {
        update: PreparedUpdate,
        outcome: Result<(), String>,
    },
    Checked(Result<Option<Release>, Failure>),
    Downloaded {
        /// The attempt's private directory, removed when preparation fails.
        attempt: PathBuf,
        outcome: Result<PreparedUpdate, Failure>,
    },
}

/// Where updates download, prepare and persist. Tests point this at a temporary folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    pub root: PathBuf,
}

impl Storage {
    /// The updater's data area, outside the repository.
    pub fn default_storage() -> Self {
        let base = crate::store::directory().unwrap_or_else(|| std::env::temp_dir().join("Shina"));
        #[cfg(test)]
        {
            static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|time| time.as_nanos())
                .unwrap_or(0);
            Self::at(
                base.join("updates")
                    .join(format!("profile-{}-{nanos}-{serial}", std::process::id())),
            )
        }
        #[cfg(not(test))]
        Self::at(base.join("updates"))
    }

    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn manifest(&self) -> PathBuf {
        self.root.join("ready.json")
    }

    /// A unique directory for one attempt, so no partial file is ever reused.
    fn attempt(&self) -> PathBuf {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_nanos())
            .unwrap_or(0);
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.root
            .join(format!("attempt-{}-{nanos}-{serial}", std::process::id()))
    }

    /// Removes downloads an interrupted run left behind, keeping the one a prepared update
    /// still verifies against.
    fn discard_attempts(&self, keep: &Option<PreparedUpdate>) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let keep = keep
                .as_ref()
                .is_some_and(|update| update.download.starts_with(&path));
            if !keep && entry.file_name().to_string_lossy().starts_with("attempt-") {
                let _ = fs::remove_dir_all(path);
            }
        }
    }

    /// Removes one attempt's files after preparation failed.
    fn discard(&self, attempt: &Path) {
        if attempt.starts_with(&self.root) {
            let _ = fs::remove_dir_all(attempt);
        }
    }
}

/// An update to install after Rune exits. `relaunch` is true only for an explicit restart.
pub struct ExitUpdate {
    pub update: PreparedUpdate,
    pub relaunch: bool,
}

static EXIT: std::sync::Mutex<Option<ExitUpdate>> = std::sync::Mutex::new(None);

/// Every test that touches EXIT must hold this single process-wide guard.
#[cfg(test)]
pub fn test_exit_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: Mutex<()> = Mutex::new(());
    let guard = GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    take_exit();
    guard
}

fn exit_slot() -> std::sync::MutexGuard<'static, Option<ExitUpdate>> {
    EXIT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Clean only the updater's fixed staging paths, and only after reading the manifest.
fn cleanup_prepared(target: &InstalledTarget, keep: Option<&PreparedUpdate>) {
    match target {
        InstalledTarget::Windows { exe } => {
            if let Some(directory) = exe.parent() {
                let staged = directory.join("shina.new.exe");
                if !keep.is_some_and(|update| update.prepared == staged) {
                    let _ = fs::remove_file(staged);
                }
            }
        }
        InstalledTarget::MacBundle { app } => {
            let staged = install::bundle_staging(app);
            if !keep.is_some_and(|update| update.prepared.starts_with(&staged)) {
                install::discard_prepared(app);
            }
        }
    }
}

/// The helper runs the previous known-working executable, not the untested download.
/// It waits for this process to exit before touching a mapped Windows executable.
#[cfg(windows)]
pub fn start_windows_helper(update: &PreparedUpdate) -> Result<(), String> {
    if update.target.path()
        != install::install_dir()
            .ok_or("No install folder.")?
            .join("shina.exe")
    {
        return Err("The update target is not the installed Shina.".into());
    }
    let storage = Storage::default_storage();
    let helper = storage.root.join("update-helper.exe");
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    fs::create_dir_all(&storage.root).map_err(|e| e.to_string())?;
    fs::copy(&current, &helper).map_err(|e| format!("Could not prepare update helper: {e}"))?;
    if file_sha256(&helper)? != file_sha256(&current)? {
        return Err("The update helper copy is incomplete.".into());
    }
    // The helper can open this process before the parent returns from main.
    let parent = std::process::id().to_string();
    let handshake = storage.root.join("helper-started");
    match fs::remove_file(&handshake) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not reset the helper handshake: {error}")),
    }
    let mut child = install::launch_child(&helper, &["--apply-update", &parent])?;
    install::wait_for_marker(&mut child, &handshake, METADATA_TIMEOUT)
}

#[cfg(windows)]
pub fn run_windows_helper() {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, id: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let result = (|| -> Result<(), String> {
        let storage = Storage::default_storage();
        if std::env::current_exe().map_err(|e| e.to_string())?
            != storage.root.join("update-helper.exe")
        {
            return Err("Only the staged helper may apply an update.".into());
        }
        let parent = std::env::args()
            .nth(2)
            .and_then(|arg| arg.parse::<u32>().ok())
            .ok_or("The helper has no parent process.")?;
        // Validate before acknowledging. On failure the parent retains its old executable
        // and handles reopening it. After acknowledgment the helper owns the handoff.
        let bytes = fs::read(storage.manifest()).map_err(|e| e.to_string())?;
        let update = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| PreparedUpdate::from_value(&value))
            .ok_or("Invalid update manifest.")?;
        let target = InstalledTarget::Windows {
            exe: install::install_dir()
                .ok_or("No install folder.")?
                .join("shina.exe"),
        };
        if !update_reaches_the_installed_app(&update, &target)
            || update.prepared != target.path().with_file_name("shina.new.exe")
        {
            return Err("The update does not belong to this install.".into());
        }
        Gh.reverify(&update)?;
        let handle = unsafe { OpenProcess(0x00100000, 0, parent) }; // SYNCHRONIZE
        if handle.is_null() {
            return Err("Could not wait for Shina to exit.".into());
        }
        let acknowledged = fs::write(storage.root.join("helper-started"), b"ready");
        let waited = if acknowledged.is_ok() {
            unsafe { WaitForSingleObject(handle, 30_000) }
        } else {
            u32::MAX
        };
        unsafe {
            CloseHandle(handle);
        }
        if waited != 0 {
            return Err("Shina did not exit; no update was installed.".into());
        }
        match apply_prepared_update(&update) {
            Ok(installed) => {
                if let Err(error) = install::refresh_windows_registration() {
                    install::report_notice(&format!(
                        "Update installed, but Windows registration failed: {error}"
                    ));
                }
                let outcome = relaunch_or_restore(
                    &update,
                    &installed,
                    &std::env::current_exe().map_err(|e| e.to_string())?,
                );
                let _ = fs::remove_file(storage.manifest());
                outcome
            }
            Err(error) => {
                let reopened = relaunch(&update.launcher(), &target);
                Err(format!("{error}. Reopening the previous app: {reopened:?}"))
            }
        }
    })();
    if let Err(error) = result {
        install::report_notice(&format!("Shina update helper failed: {error}"));
        eprintln!("Shina update helper: {error}");
        std::process::exit(1);
    }
}

/// Hands a prepared update from `State` to `main`, which installs it once Rune has exited.
pub fn record_exit(update: ExitUpdate) {
    *exit_slot() = Some(update);
}

/// Takes the recorded update. Idempotent: a second call installs nothing more.
pub fn take_exit() -> Option<ExitUpdate> {
    exit_slot().take()
}

/// Whether an exit recorded an update, without taking it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn exit_recorded() -> bool {
    exit_slot().is_some()
}

/// Installs a prepared update by replacing the installed app. The caller has already shut
/// Rune and Pi down. Returns the path an explicit restart should open.
pub fn apply_prepared_update(update: &PreparedUpdate) -> Result<PathBuf, String> {
    Gh.reverify(update)?;
    match &update.target {
        InstalledTarget::Windows { exe } => {
            let dir = exe
                .parent()
                .ok_or("The installed executable has no folder.")?;
            install::commit_exe(&update.prepared, dir)
        }
        InstalledTarget::MacBundle { app } => {
            install::commit_macos_bundle(&update.prepared, app)?;
            register_mac_bundle(app);
            Ok(app.clone())
        }
    }
}

/// Runs the same Launch Services registration the installer uses. Best effort.
fn register_mac_bundle(app: &Path) {
    let result = crate::process::output(std::process::Command::new(
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
    )
    .arg("-f")
    .arg(app)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null()), METADATA_TIMEOUT, 64 * 1024);
    if !result.is_ok_and(|output| output.status.success()) {
        install::report_notice(
            "The update installed, but macOS LaunchServices registration failed.",
        );
    }
}

/// Opens the replaced app after an explicit restart.
pub fn relaunch(installed: &Path, target: &InstalledTarget) -> Result<(), String> {
    match target {
        InstalledTarget::Windows { .. } => install::launch_with(installed, &[]),
        InstalledTarget::MacBundle { .. } => install::open_mac_bundle(installed),
    }
}

/// Relaunches into a just-installed update. A build that cannot start is put back in place
/// and the previous app is reopened, so a launch failure never leaves the user without Shina.
pub fn relaunch_or_restore(
    update: &PreparedUpdate,
    installed: &Path,
    running: &Path,
) -> Result<(), String> {
    relaunch_or_restore_with(update, installed, running, relaunch)
}

fn relaunch_or_restore_with(
    update: &PreparedUpdate,
    installed: &Path,
    running: &Path,
    mut launch: impl FnMut(&Path, &InstalledTarget) -> Result<(), String>,
) -> Result<(), String> {
    let launch_error = match launch(installed, &update.target) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if let Err(error) = restore_previous(update, running) {
        return Err(format!(
            "{launch_error}. Restoring the previous app failed: {error}"
        ));
    }
    // macOS reopens the restored bundle; Windows reopens the restored installed path.
    let reopened = update.launcher();
    match launch(&reopened, &update.target) {
        Ok(()) => Ok(()),
        Err(error) => Err(format!(
            "{launch_error}. The previous Shina was restored; reopen {} ({error}).",
            reopened.display()
        )),
    }
}

/// Puts the previous app back over a replacement that cannot start. Windows keeps the old
/// executable as `shina.old.exe`, macOS keeps the bundle as `Shina-app.previous`.
pub fn restore_previous(update: &PreparedUpdate, running: &Path) -> Result<(), String> {
    match &update.target {
        InstalledTarget::Windows { exe } => {
            let dir = exe
                .parent()
                .ok_or("The installed executable has no folder.")?;
            install::restore_exe(dir, running)
        }
        InstalledTarget::MacBundle { app } => install::restore_mac_bundle(app),
    }
}

/// The worker side: release metadata, the download, verification and preparation.
trait UpdateHost: Send + 'static {
    fn fetch_release(&mut self, target: &InstalledTarget) -> Result<Option<Release>, Failure>;
    /// Downloads `release` into `attempt` and verifies it. Returns the file and its digest.
    fn download_release(
        &mut self,
        release: &Release,
        _target: &InstalledTarget,
        attempt: &Path,
    ) -> Result<(PathBuf, String), Failure>;
    fn prepare(
        &mut self,
        release: &Release,
        _target: &InstalledTarget,
        download: &Path,
    ) -> Result<PathBuf, Failure>;
    fn reverify(&mut self, update: &PreparedUpdate) -> Result<(), String>;
}

/// The real host: `gh` for metadata and downloads, the filesystem for preparation.
struct Gh;

impl UpdateHost for Gh {
    fn fetch_release(&mut self, target: &InstalledTarget) -> Result<Option<Release>, Failure> {
        let output = run_gh(
            &[
                "release",
                "view",
                "--repo",
                REPOSITORY,
                "--json",
                "tagName,assets",
            ],
            METADATA_TIMEOUT,
        )?;
        let release: Value = serde_json::from_slice(&output)
            .map_err(|_| Failure::quiet("GitHub returned unreadable release metadata."))?;
        let Some(tag) = release["tagName"].as_str() else {
            return Err(Failure::quiet("The latest release has no tag."));
        };
        if !is_newer(tag, env!("CARGO_PKG_VERSION")) {
            return Ok(None);
        }
        Ok(select_asset(&release, target).map(|asset| Release {
            tag: tag.to_owned(),
            asset,
        }))
    }

    fn download_release(
        &mut self,
        release: &Release,
        _target: &InstalledTarget,
        attempt: &Path,
    ) -> Result<(PathBuf, String), Failure> {
        let dir = attempt.join("asset");
        let download = dir.join(&release.asset.name);
        let result = run_gh_download(&release.tag, &release.asset.name, &dir, &download)
            .and_then(|()| {
                verify_download(&release.tag, &release.asset, &download, fetch_checksums)
            })
            .map(|sha256| (download, sha256));
        if result.is_err() {
            let _ = fs::remove_dir_all(attempt);
        }
        result
    }

    fn prepare(
        &mut self,
        release: &Release,
        target: &InstalledTarget,
        download: &Path,
    ) -> Result<PathBuf, Failure> {
        match target {
            InstalledTarget::Windows { exe } => {
                install::prepare_exe(download, exe.parent().unwrap_or_else(|| Path::new(".")))
                    .map_err(Failure::quiet)
            }
            InstalledTarget::MacBundle { app } => {
                let directory = install::mac_bundle_directory(app)
                    .ok_or_else(|| Failure::quiet("The app bundle has no folder."))?;
                if !writable(&directory) {
                    return Err(Failure::quiet(format!(
                        "{} is not writable, so the update cannot be prepared.",
                        directory.display()
                    )));
                }
                let prepared = (|| {
                    let archive = download
                        .parent()
                        .ok_or_else(|| Failure::quiet("The download has no folder."))?;
                    let extracted = archive.join("bundle");
                    unpack_macos_zip(download, &extracted)?;
                    let bundle = extracted.join(install::MACOS_APP_NAME);
                    if let Some(problem) = mac_bundle_problem(&bundle, &release.tag) {
                        return Err(Failure::quiet(problem));
                    }
                    // Staged beside the installed app, so commit is a rename on one volume.
                    let staged =
                        install::prepare_macos_bundle(&bundle, &install::bundle_staging(app))
                            .map_err(Failure::quiet)?;
                    let mut sign = Command::new("/usr/bin/codesign");
                    sign.args(["--force", "--deep", "--sign", "-"])
                        .arg(&staged)
                        .stdin(Stdio::null());
                    if !run_bounded(sign, METADATA_TIMEOUT)?.0.success() {
                        return Err(Failure::quiet("Could not sign the prepared Shina bundle."));
                    }
                    if let Some(problem) = mac_bundle_problem(&staged, &release.tag) {
                        return Err(Failure::quiet(problem));
                    }
                    install::sync_prepared_bundle(&staged).map_err(Failure::quiet)?;
                    Ok(staged)
                })();
                if prepared.is_err() {
                    install::discard_prepared(app);
                }
                prepared
            }
        }
    }

    fn reverify(&mut self, update: &PreparedUpdate) -> Result<(), String> {
        let owned = match &update.target {
            InstalledTarget::Windows { exe } => exe.with_file_name("shina.new.exe"),
            InstalledTarget::MacBundle { app } => {
                install::bundle_staging(app).join(install::MACOS_APP_NAME)
            }
        };
        if update.prepared != owned || update.prepared.is_symlink() {
            return Err("The prepared update is outside its staging location.".into());
        }
        if fs::metadata(&update.download)
            .map_err(|e| e.to_string())?
            .len()
            != update.size
            || file_sha256(&update.download)? != update.sha256
        {
            return Err("The downloaded update no longer matches its checksum.".into());
        }
        match &update.target {
            InstalledTarget::Windows { .. } => {
                if file_sha256(&update.prepared)? != update.sha256 {
                    return Err("The prepared update no longer matches its download.".into());
                }
            }
            InstalledTarget::MacBundle { .. } => {
                if !update.prepared.join("Contents/MacOS/shina").is_file() {
                    return Err("The prepared app is missing.".into());
                }
                if let Some(problem) = mac_bundle_problem(&update.prepared, &update.tag) {
                    return Err(problem);
                }
            }
        }
        Ok(())
    }
}

pub struct Updater {
    status: Status,
    target: Option<InstalledTarget>,
    events: Option<Receiver<Event>>,
    next_check: Option<Instant>,
    /// Set after a real authentication denial: no networked attempts until the next launch.
    suspended: Option<String>,
    notice: Option<String>,
    diagnostics: VecDeque<String>,
    storage: Storage,
    /// Shared with the one operation that may use it at a time.
    host: Arc<Mutex<Box<dyn UpdateHost>>>,
    /// This process runs from an installed app, so it may update itself.
    installed: bool,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            target: None,
            events: None,
            next_check: None,
            suspended: None,
            notice: None,
            diagnostics: VecDeque::new(),
            storage: Storage::default_storage(),
            host: Arc::new(Mutex::new(Box::new(Gh))),
            installed: false,
        }
    }
}

#[cfg(test)]
fn gh() -> Command {
    panic!("Unit tests must inject GitHub responses, not run gh.");
}

#[cfg(not(test))]
fn gh() -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new("gh");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
}

/// `v0.2.10` or `0.2.10` as comparable numbers. Anything else is not a release version.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let version = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.split('-').next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(version)
}

pub fn is_newer(tag: &str, current: &str) -> bool {
    matches!((parse_version(tag), parse_version(current)), (Some(a), Some(b)) if a > b)
}

/// The exact asset for `target` with the size GitHub declares. A missing or sizeless asset
/// means this build cannot update from that release.
pub fn select_asset(release: &Value, target: &InstalledTarget) -> Option<ReleaseAsset> {
    named_asset(release, target.asset())
}

/// The asset called `name` with its declared size.
fn named_asset(release: &Value, name: &str) -> Option<ReleaseAsset> {
    let asset = release["assets"]
        .as_array()?
        .iter()
        .find(|asset| asset["name"] == name)?;
    let size = match &asset["size"] {
        Value::Number(number) => number.as_u64()?,
        Value::String(text) => text.parse().ok()?,
        _ => return None,
    };
    Some(ReleaseAsset {
        name: name.to_owned(),
        size,
        digest: match &asset["digest"] {
            Value::String(digest) if !digest.is_empty() => Some(digest.clone()),
            Value::Object(object) => object
                .get("value")
                .and_then(Value::as_str)
                .filter(|digest| !digest.is_empty())
                .map(str::to_owned),
            _ => None,
        },
    })
}

/// A `sha256:<hex>` value, or a bare hex digest, normalized to lowercase hex.
fn hex_sha256(text: &str) -> Option<String> {
    let text = text.trim();
    let hex = text.strip_prefix("sha256:").unwrap_or(text);
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

/// The checksum line for `name` in a `SHA256SUMS` file (`<hex>  <name>`).
pub fn sha256s_entry<'a>(checksums: &'a str, name: &str) -> Option<&'a str> {
    checksums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next(), parts.next()) {
            (Some(hex), Some(file), None)
                if file.trim_start_matches('*') == name && hex_sha256(hex).is_some() =>
            {
                Some(hex)
            }
            _ => None,
        }
    })
}

/// Streams a file through SHA-256 and returns its lowercase hex digest.
pub fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
fn bytes_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Checks a download against its declared size and checksum before preparation. An
/// authentication denial while fetching the checksums stays typed, so it suspends attempts.
fn verify_download(
    tag: &str,
    asset: &ReleaseAsset,
    download: &Path,
    checksums: impl FnOnce(&str) -> Result<String, Failure>,
) -> Result<String, Failure> {
    let quiet = |message: String| Err(Failure::Quiet(message));
    let size = match fs::metadata(download) {
        Ok(metadata) => metadata.len(),
        Err(_) => return quiet(format!("The {} download is incomplete.", asset.name)),
    };
    if size != asset.size {
        return quiet(format!(
            "The {} download is incomplete ({} of {} bytes).",
            asset.name, size, asset.size
        ));
    }
    let expected = expected_sha256(tag, asset, checksums)?;
    let actual = match file_sha256(download) {
        Ok(actual) => actual,
        Err(error) => return quiet(error),
    };
    if actual != expected {
        return quiet(format!(
            "The {} download does not match its SHA-256.",
            asset.name
        ));
    }
    Ok(actual)
}

/// The SHA-256 `asset` must match: its published digest, or its `SHA256SUMS` entry.
fn expected_sha256(
    tag: &str,
    asset: &ReleaseAsset,
    checksums: impl FnOnce(&str) -> Result<String, Failure>,
) -> Result<String, Failure> {
    if let Some(digest) = &asset.digest {
        return hex_sha256(digest).ok_or_else(|| {
            Failure::quiet(format!(
                "The {} release publishes an invalid SHA-256.",
                asset.name
            ))
        });
    }
    match sha256s_entry(&checksums(tag)?, &asset.name).and_then(hex_sha256) {
        Some(digest) => Ok(digest),
        None => Err(Failure::quiet(format!(
            "The {tag} release publishes no SHA-256 for {}.",
            asset.name
        ))),
    }
}

/// The release's `SHA256SUMS` file, for releases without per-asset digests.
fn fetch_checksums(tag: &str) -> Result<String, Failure> {
    let dir = std::env::temp_dir().join(format!(
        "shina-checksums-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_nanos())
            .unwrap_or(0)
    ));
    let file = dir.join(SHA256SUMS);
    let result = run_gh_download(tag, SHA256SUMS, &dir, &file).and_then(|()| {
        let file = fs::File::open(&file)
            .map_err(|_| Failure::quiet(format!("Could not read {SHA256SUMS}.")))?;
        let mut text = String::new();
        file.take(1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|_| Failure::quiet(format!("Could not read {SHA256SUMS}.")))?;
        if text.len() > 1024 * 1024 {
            return Err(Failure::quiet("The checksum file is too large."));
        }
        Ok(text)
    });
    let _ = fs::remove_dir_all(&dir);
    result
}

fn failure_text(failure: &Failure) -> String {
    match failure {
        Failure::Quiet(message) | Failure::Auth(message) => message.clone(),
    }
}

/// Runs `gh` with prompts off and a deadline, returning its stdout when it succeeds.
fn run_gh(args: &[&str], timeout: Duration) -> Result<Vec<u8>, Failure> {
    let mut command = gh();
    command
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    let (status, stdout, stderr) = run_bounded(command, timeout)?;
    if status.success() {
        return Ok(stdout);
    }
    Err(failure_from_stderr(&String::from_utf8_lossy(&stderr)))
}

fn run_gh_download(tag: &str, asset: &str, dir: &Path, file: &Path) -> Result<(), Failure> {
    fs::create_dir_all(dir)
        .map_err(|e| Failure::quiet(format!("Could not download {asset}: {e}")))?;
    // No --clobber: each attempt owns a fresh directory.
    let _ = fs::remove_file(file);
    let mut command = gh();
    command
        .args([
            "release",
            "download",
            tag,
            "--repo",
            REPOSITORY,
            "--pattern",
            asset,
            "--dir",
        ])
        .arg(dir)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    let (status, _, stderr) = run_bounded(command, DOWNLOAD_TIMEOUT)?;
    if status.success() && file.is_file() {
        return Ok(());
    }
    if status.success() {
        return Err(Failure::quiet(format!(
            "The {tag} download is missing {asset}."
        )));
    }
    Err(failure_from_stderr(&String::from_utf8_lossy(&stderr)))
}

/// A real authentication denial asks for a manual remedy; everything else stays quiet.
fn failure_from_stderr(stderr: &str) -> Failure {
    let lower = stderr.to_ascii_lowercase();
    if [
        "authentication",
        "not logged",
        "bad credentials",
        "http 401",
    ]
    .iter()
    .any(|text| lower.contains(text))
    {
        return Failure::Auth("Run `gh auth login` so Shina can download updates.".into());
    }
    Failure::quiet(bounded_line(stderr))
}

/// One sanitized, bounded diagnostic line.
fn bounded_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("GitHub request failed.");
    let text: String = line.chars().take(200).collect();
    format!("Update check failed: {}", redact(&text))
}

/// Removes anything shaped like a token from a diagnostic.
pub fn redact(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let secret = ["ghp_", "github_pat_", "gho_", "ghu_", "ghs_"]
                .iter()
                .any(|prefix| word.contains(prefix));
            if secret { "[redacted]" } else { word }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs a command to completion, or kills and reaps it at the deadline.
fn run_bounded(
    command: Command,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), Failure> {
    run_bounded_with_limit(command, timeout, 1024 * 1024)
}

fn run_bounded_with_limit(
    mut command: Command,
    timeout: Duration,
    output_limit: usize,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), Failure> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GH_PROMPT_DISABLED", "1");
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = crate::process::Process::spawn(&mut command).map_err(|error| {
        if program == "gh" && error.kind() == std::io::ErrorKind::NotFound {
            Failure::quiet("Install the GitHub CLI (gh) so Shina can check for updates.")
        } else {
            Failure::quiet(format!("Could not start update command {program}: {error}"))
        }
    })?;
    let stdout = read_on_thread(child.stdout.take().expect("piped stdout"), output_limit);
    let stderr = read_on_thread(child.stderr.take().expect("piped stderr"), 64 * 1024);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                child.terminate();
                let _ = child.reap(Duration::from_secs(1));
                return Err(Failure::quiet("The update command did not finish in time."));
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                child.terminate();
                let _ = child.reap(Duration::from_secs(1));
                return Err(Failure::quiet("Could not wait for the update command."));
            }
        }
    };
    child.terminate();
    let receive = |reader: Receiver<Result<Vec<u8>, Failure>>| {
        reader
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| Failure::quiet("The update command's output did not close in time."))?
    };
    let out = receive(stdout)?;
    let err = receive(stderr)?;
    Ok((status, out, err))
}

fn read_on_thread<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> Receiver<Result<Vec<u8>, Failure>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut out = Vec::new();
        let mut buffer = [0u8; 8192];
        let mut overflow = false;
        let result = loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    break if overflow {
                        Err(Failure::quiet("The update command's output is too large."))
                    } else {
                        Ok(out)
                    };
                }
                Ok(count) => {
                    let keep = count.min(limit.saturating_sub(out.len()));
                    out.extend_from_slice(&buffer[..keep]);
                    overflow |= keep < count;
                }
                Err(error) => {
                    break Err(Failure::quiet(format!(
                        "Could not read update output: {error}"
                    )));
                }
            }
        };
        let _ = sender.send(result);
    });
    receiver
}

/// Whether `dir` accepts new entries. An unwritable applications folder fails quietly.
fn writable(dir: &Path) -> bool {
    dir.is_dir()
        && fs::metadata(dir)
            .map(|meta| !meta.permissions().readonly())
            .unwrap_or(false)
        && fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(format!(".shina-write-{}", std::process::id())))
            .map(|file| {
                drop(file);
                let _ = fs::remove_file(dir.join(format!(".shina-write-{}", std::process::id())));
            })
            .is_ok()
}

/// Unpacks a `Shina.app` archive with the system tools after checking every entry and link.
fn unpack_macos_zip(zip: &Path, destination: &Path) -> Result<(), Failure> {
    let mut list = Command::new("/usr/bin/zipinfo");
    list.arg("-l").arg(zip);
    let (status, listing, _) = run_bounded_with_limit(list, METADATA_TIMEOUT, MAX_ARCHIVE_LISTING)?;
    if !status.success() {
        return Err(Failure::quiet("The update archive could not be read."));
    }
    let listing = String::from_utf8(listing)
        .map_err(|_| Failure::quiet("The update archive listing is not UTF-8."))?;
    if let Some(problem) = archive_entry_problem(&listing) {
        return Err(Failure::quiet(problem));
    }
    fs::create_dir_all(destination)
        .map_err(|e| Failure::quiet(format!("Could not unpack the update: {e}")))?;
    let mut extract = Command::new("/usr/bin/ditto");
    extract.args(["-x", "-k"]).arg(zip).arg(destination);
    let (status, _, _) = run_bounded(extract, DOWNLOAD_TIMEOUT)?;
    if !status.success() {
        return Err(Failure::quiet("The update archive could not be unpacked."));
    }
    Ok(())
}

/// One entry of a `zipinfo -l` listing: the file-type letter of its mode, and its path.
///
/// ```text
/// -rwxr-xr-x  3.0 unx  1234 tx     567 defN 26-Nov-09 14:55 Shina.app/Contents/MacOS/shina
/// -rwxr-xr-x  3.0 unx  1234 tx     567 defN 26-Nov-09 14:55 Shina.app/My App
/// ```
///
/// The columns after the mode are the version, `unx`, the three sizes, the method, the date
/// and the time. Only the path holds spaces, so a spaced path is what follows the time
/// column, which always holds a `:`.
fn zipinfo_entry(line: &str) -> Option<(char, &str)> {
    if line.chars().any(char::is_control) {
        return None;
    }
    let mut rest = line;
    let mut fields = Vec::new();
    for _ in 0..9 {
        rest = rest.trim_start_matches(' ');
        let end = rest.find(' ')?;
        fields.push(&rest[..end]);
        rest = &rest[end..];
    }
    let mode = fields[0];
    if mode.len() != 10 || !mode.is_ascii() {
        return None;
    }
    let kind = mode.as_bytes()[0] as char;
    if !matches!(kind, '-' | 'd' | 'l' | 'p' | 'c' | 'b' | 's')
        || !mode[1..].chars().all(|c| "-rwxstST".contains(c))
        || fields[1].parse::<f32>().is_err()
        || fields[2] != "unx"
        || fields[3].parse::<u64>().is_err()
        || fields[5].parse::<u64>().is_err()
        || fields[7].split('-').count() != 3
        || fields[8].len() != 5
        || fields[8].as_bytes()[2] != b':'
        || !fields[8].chars().all(|c| c.is_ascii_digit() || c == ':')
    {
        return None;
    }
    let path = rest.trim_start_matches(' ');
    (!path.is_empty()).then_some((kind, path))
}

/// Parse the complete listing, including both independently reported entry counts.
fn archive_entries(listing: &str) -> Option<Vec<(char, &str)>> {
    if listing.len() > MAX_ARCHIVE_LISTING || listing.chars().any(|c| c.is_control() && c != '\n') {
        return None;
    }
    let lines: Vec<_> = listing.lines().collect();
    if lines.len() < 4 || !lines[0].starts_with("Archive:  ") {
        return None;
    }
    let (size, count) = lines[1]
        .strip_prefix("Zip file size: ")?
        .split_once(" bytes, number of entries: ")?;
    size.parse::<u64>().ok()?;
    let count = count.parse::<usize>().ok()?;
    let footer: Vec<_> = lines.last()?.split_whitespace().collect();
    if footer.len() != 9
        || footer[0].parse::<usize>().ok()? != count
        || !matches!(footer[1], "file," | "files,")
        || footer[2].parse::<u64>().is_err()
        || footer[3] != "bytes"
        || footer[4] != "uncompressed,"
        || footer[5].parse::<u64>().is_err()
        || footer[6] != "bytes"
        || footer[7] != "compressed:"
        || footer[8].strip_suffix('%')?.parse::<f64>().is_err()
        || lines.len().checked_sub(3)? != count
        || count == 0
    {
        return None;
    }
    lines[2..lines.len() - 1]
        .iter()
        .map(|line| zipinfo_entry(line))
        .collect()
}

#[cfg(test)]
fn zipinfo_complete(listing: &str) -> Option<bool> {
    Some(archive_entries(listing).is_some())
}

/// Rejects absolute paths, parent links, links and entries outside the bundle before
/// extraction. `listing` is a `zipinfo -l` listing, whose first column gives each entry type.
pub fn archive_entry_problem(listing: &str) -> Option<String> {
    let Some(entries) = archive_entries(listing) else {
        return Some("The update archive listing is incomplete or malformed.".into());
    };
    for (kind, entry) in entries {
        if kind == 'l' {
            return Some(format!("The update archive contains a link: {entry}"));
        }
        if kind != 'd' && kind != '-' {
            return Some(format!(
                "The update archive contains a special file: {entry}"
            ));
        }
        let path = entry.strip_prefix("./").unwrap_or(entry);
        // Only the Shina bundle itself: no other App bundle, no loose file.
        let inside_app = path
            .split('/')
            .next()
            .is_some_and(|first| first == install::MACOS_APP_NAME);
        let unsafe_path = path.starts_with('/')
            || path.starts_with('\\')
            || path.contains('\\')
            || path
                .split('/')
                .any(|part| part == ".." || part.is_empty() && path.contains("//"))
            || path.split(':').count() > 1;
        if unsafe_path || !inside_app {
            return Some(format!(
                "The update archive contains an unexpected path: {entry}"
            ));
        }
    }
    None
}

/// Why a downloaded Mac bundle may not be installed: identity, version, architecture, signing.
pub fn mac_bundle_problem(bundle: &Path, tag: &str) -> Option<String> {
    if install::mac_bundle_id(bundle).as_deref() != Some(install::MACOS_BUNDLE_ID) {
        return Some("The downloaded app is not Shina.".into());
    }
    let expected = tag.trim_start_matches('v');
    if install::mac_bundle_version(bundle).as_deref() != Some(expected) {
        return Some(format!("The downloaded app is not version {expected}."));
    }
    let executable = bundle.join("Contents/MacOS/shina");
    if !executable.is_file() || !bundle.join("Contents/MacOS/launch-shina").is_file() {
        return Some("The downloaded app is incomplete.".into());
    }
    let mut architecture = Command::new("/usr/bin/lipo");
    architecture
        .arg("-archs")
        .arg(&executable)
        .stdin(Stdio::null());
    let architectures = run_bounded(architecture, METADATA_TIMEOUT)
        .ok()
        .filter(|output| output.0.success())
        .map(|output| String::from_utf8_lossy(&output.1).trim().to_owned())
        .unwrap_or_default();
    if !architectures
        .split_whitespace()
        .any(|arch| arch == running_architecture())
    {
        return Some("The downloaded app is built for another processor.".into());
    }
    let mut signature = Command::new("/usr/bin/codesign");
    signature
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .stdin(Stdio::null());
    let signed = run_bounded(signature, METADATA_TIMEOUT)
        .map(|output| output.0.success())
        .unwrap_or(false);
    if !signed {
        return Some("The downloaded app is not signed correctly.".into());
    }
    None
}

fn running_architecture() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "arm64"
    }
}

impl Updater {
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Recent quiet failures, newest last. Available to the diagnostics views and tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn diagnostics(&self) -> impl Iterator<Item = &String> {
        self.diagnostics.iter()
    }

    /// The manual remedy after an authentication denial this session, if any.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn suspended(&self) -> Option<&str> {
        self.suspended.as_deref()
    }

    pub fn notice(&self) -> Option<&str> {
        self.suspended().or(self.notice.as_deref())
    }

    pub fn retry(&mut self) {
        if matches!(self.status, Status::Idle) && self.events.is_none() && self.installed {
            self.suspended = None;
            self.notice = None;
            self.next_check = Some(Instant::now());
        }
    }

    pub fn prepared_update(&self) -> Option<&PreparedUpdate> {
        match &self.status {
            Status::Ready { update } => Some(update),
            _ => None,
        }
    }

    /// Test storage and a scripted host, so no test reaches GitHub.
    #[cfg(test)]
    fn configure(&mut self, storage: Storage, host: Box<dyn UpdateHost>) {
        self.storage = storage;
        self.host = Arc::new(Mutex::new(host));
    }

    fn record(&mut self, message: impl Into<String>) {
        if self.diagnostics.len() == MAX_DIAGNOSTICS {
            self.diagnostics.pop_front();
        }
        let message = redact(&message.into());
        self.diagnostics.push_back(
            message
                .chars()
                .take(300)
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect(),
        );
    }

    fn idle_from(&mut self, now: Instant) {
        self.status = Status::Idle;
        self.next_check = Some(now + INTERVAL);
    }

    /// Decides eligibility and schedules the first check. Demo and snapshot runs pass
    /// `enabled = false` and never start workers.
    pub fn start(&mut self, enabled: bool, now: Instant, wake: Waker) {
        self.next_check = None;
        if self.status == Status::Disabled {
            return;
        }
        if !enabled {
            self.status = Status::Disabled;
            return;
        }
        if let Some(directory) = crate::store::directory() {
            let warning = directory.join("startup-warning.txt");
            if let Ok(file) = fs::File::open(&warning) {
                let mut message = String::new();
                let _ = file.take(4096).read_to_string(&mut message);
                self.notice = Some(redact(&message));
                self.record(message);
                let _ = fs::remove_file(warning);
            }
        }
        if !self.installed {
            match install::installed_target() {
                Some(target) => {
                    self.target = Some(target);
                    self.installed = true;
                }
                None => {
                    self.status = Status::Disabled;
                    return;
                }
            }
        }
        let Some(target) = self.target.clone() else {
            self.status = Status::Disabled;
            return;
        };
        if self.events.is_some() {
            return;
        }
        // A prepared update from a previous run is revalidated before it shows as Ready. Its
        // download is kept, and Ready schedules no check.
        let update = fs::read(self.storage.manifest())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| PreparedUpdate::from_value(&value))
            .filter(|update| {
                update_reaches_the_installed_app(update, &target)
                    && update.download.starts_with(&self.storage.root)
            });
        if let Some(update) = update {
            self.spawn(wake, move |host| {
                let outcome = host.reverify(&update).map_err(|error| redact(&error));
                Event::Recovered { update, outcome }
            });
            return;
        }
        self.storage.discard_attempts(&None);
        cleanup_prepared(&target, None);
        self.clear_manifest();
        self.next_check = Some(now + FIRST_CHECK);
    }

    /// An updater holding a prepared update, for guards and UI tests.
    #[cfg(test)]
    pub fn test_ready(update: PreparedUpdate) -> Self {
        Self {
            status: Status::Ready { update },
            ..Self::default()
        }
    }

    /// An updater mid-download, for UI tests.
    #[cfg(test)]
    pub fn test_downloading(tag: &str) -> Self {
        Self {
            status: Status::Downloading { tag: tag.into() },
            ..Self::default()
        }
    }

    /// An updater that has finished its last attempt, for UI tests.
    #[cfg(test)]
    pub fn test_idle() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn test_suspended() -> Self {
        let mut updater = Self {
            installed: true,
            ..Self::default()
        };
        updater.failed(
            Failure::Auth("Run gh auth login manually to enable updates.".into()),
            Instant::now(),
        );
        updater
    }

    /// Turns updating off for this build. `start` leaves a disabled updater alone.
    #[cfg(test)]
    pub fn disable(&mut self) {
        self.status = Status::Disabled;
        self.next_check = None;
    }

    /// Time until the next scheduled check, for `Tx::wake_after`.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.next_check
            .map(|next| next.saturating_duration_since(now))
    }

    /// Consumes finished worker events and starts due work. Returns whether anything visible
    /// changed.
    pub fn poll(&mut self, now: Instant, wake: Waker) -> bool {
        let Some(event) = self
            .events
            .as_ref()
            .and_then(|events| events.try_recv().ok())
        else {
            self.start_due_work(now, wake);
            return false;
        };
        self.events = None;
        let changed = self.apply(event, now, wake.clone());
        self.start_due_work(now, wake);
        changed
    }

    fn start_due_work(&mut self, now: Instant, wake: Waker) {
        if self.events.is_some() {
            return;
        }
        if self.suspended.is_some() || !matches!(self.status, Status::Idle) {
            // A suspended updater and a ready update run no timer at all.
            self.next_check = None;
            return;
        }
        if !self.next_check.is_some_and(|next| now >= next) {
            return;
        }
        self.next_check = None;
        let Some(target) = self.target.clone() else {
            return;
        };
        self.status = Status::Checking;
        self.spawn(wake, move |host| {
            Event::Checked(host.fetch_release(&target))
        });
    }

    /// Downloads and prepares a release. Only one worker operation runs at a time.
    fn begin_download(&mut self, release: Release, wake: Waker) {
        let Some(target) = self.target.clone() else {
            return;
        };
        let attempt = self.storage.attempt();
        self.status = Status::Downloading {
            tag: release.tag.clone(),
        };
        let downloaded = release.clone();
        let staged = attempt.clone();
        self.spawn(wake, move |host| {
            let outcome = host
                .download_release(&downloaded, &target, &staged)
                .and_then(|(download, sha256)| {
                    let prepared = host.prepare(&downloaded, &target, &download)?;
                    Ok(PreparedUpdate {
                        tag: downloaded.tag.clone(),
                        target: target.clone(),
                        asset: downloaded.asset.name.clone(),
                        size: downloaded.asset.size,
                        sha256,
                        download,
                        prepared,
                    })
                });
            Event::Downloaded { attempt, outcome }
        });
    }

    /// Runs `work` on a worker thread, then wakes the app. Only one operation runs at a time,
    /// so the shared host is free while the worker holds it.
    fn spawn(
        &mut self,
        wake: Waker,
        work: impl FnOnce(&mut dyn UpdateHost) -> Event + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        let host = Arc::clone(&self.host);
        self.events = Some(receiver);
        thread::spawn(move || {
            let event = {
                let mut host = host.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                work(&mut **host)
            };
            let _ = sender.send(event);
            wake();
        });
    }

    fn apply(&mut self, event: Event, now: Instant, wake: Waker) -> bool {
        match event {
            Event::Recovered { update, outcome } => {
                match outcome {
                    Ok(()) => {
                        self.storage.discard_attempts(&Some(update.clone()));
                        cleanup_prepared(&update.target, Some(&update));
                        self.ready(update);
                    }
                    Err(error) => {
                        self.storage.discard_attempts(&None);
                        self.record(error);
                        self.clear_manifest();
                        cleanup_prepared(&update.target, None);
                        self.idle_from(now);
                    }
                }
                true
            }
            Event::Checked(Ok(Some(release))) => {
                self.notice = None;
                self.begin_download(release, wake);
                true
            }
            Event::Checked(Ok(None)) => {
                self.notice = None;
                self.idle_from(now);
                true
            }
            Event::Checked(Err(failure)) => {
                self.failed(failure, now);
                true
            }
            Event::Downloaded {
                outcome: Ok(update),
                ..
            } => {
                match self.write_manifest(&update) {
                    Ok(()) => self.ready(update),
                    Err(error) => {
                        self.storage.discard_attempts(&None);
                        cleanup_prepared(&update.target, None);
                        self.failed(Failure::quiet(error), now);
                    }
                }
                true
            }
            Event::Downloaded {
                attempt,
                outcome: Err(failure),
            } => {
                // Nothing of a failed attempt is kept, so no complete download piles up.
                self.storage.discard(&attempt);
                if let Some(target) = &self.target {
                    cleanup_prepared(target, None);
                }
                self.failed(failure, now);
                true
            }
        }
    }

    /// A verified replacement waits for an install, so no timer runs.
    fn ready(&mut self, update: PreparedUpdate) {
        self.next_check = None;
        self.status = Status::Ready { update };
    }

    fn failed(&mut self, failure: Failure, now: Instant) {
        self.notice = Some(failure_text(&failure));
        self.record(failure_text(&failure));
        if let Failure::Auth(remedy) = failure {
            // No networked attempt runs again this session, so no timer is scheduled.
            self.suspended = Some(remedy);
            self.status = Status::Idle;
            self.next_check = None;
            return;
        }
        self.idle_from(now);
    }

    fn write_manifest(&self, update: &PreparedUpdate) -> Result<(), String> {
        fs::create_dir_all(&self.storage.root).map_err(|e| e.to_string())?;
        let manifest = self.storage.manifest();
        let temporary = manifest.with_extension("json.tmp");
        let written = fs::File::create(&temporary)
            .and_then(|mut file| {
                file.write_all(update.to_value().to_string().as_bytes())?;
                file.sync_all()
            })
            .and_then(|()| fs::rename(&temporary, &manifest));
        written.map_err(|e| format!("Could not save the prepared update: {e}"))
    }

    fn clear_manifest(&self) {
        let _ = fs::remove_file(self.storage.manifest());
    }

    /// Drops `Ready` after a failed install, so this run does not retry a broken update.
    #[allow(dead_code)]
    pub fn discard_ready(&mut self) {
        if matches!(self.status, Status::Ready { .. }) {
            self.clear_manifest();
            self.status = Status::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_is_not_automatically_an_authentication_denial() {
        assert!(matches!(
            failure_from_stderr("HTTP 403: API rate limit exceeded"),
            Failure::Quiet(_)
        ));
        assert!(matches!(
            failure_from_stderr("HTTP 401: bad credentials"),
            Failure::Auth(_)
        ));
        let mut updater = Updater {
            installed: true,
            ..Updater::default()
        };
        updater.failed(
            Failure::Auth("Run gh auth login manually.".into()),
            Instant::now(),
        );
        assert!(updater.notice().unwrap().contains("manually"));
        updater.retry();
        assert!(updater.suspended().is_none());
        assert!(updater.next_check.is_some());
    }
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    type Check = dyn Fn() -> Result<Option<Release>, Failure> + Send + Sync;

    /// A host a test scripts, so no test reaches GitHub, and none runs `gh` or an archive
    /// tool: a real verification path takes its checksums from the fixture.
    struct Scripted {
        checks: Arc<AtomicUsize>,
        downloads: Arc<AtomicUsize>,
        check: Arc<Check>,
        /// Preparation fails with this message, or succeeds by copying the download.
        prepare: Option<String>,
        /// An authentication denial while fetching checksums.
        deny_checksums: Arc<AtomicBool>,
        /// Recovery finds the download damaged.
        reverify_fails: bool,
        /// Nothing publishes a digest, so verification asks for SHA256SUMS.
        sums: Option<String>,
    }

    impl Scripted {
        fn new(
            check: impl Fn() -> Result<Option<Release>, Failure> + Send + Sync + 'static,
        ) -> Self {
            Self {
                checks: Default::default(),
                downloads: Default::default(),
                check: Arc::new(check),
                prepare: None,
                deny_checksums: Default::default(),
                reverify_fails: false,
                sums: None,
            }
        }

        fn failing_prepare(self, message: &str) -> Self {
            Self {
                prepare: Some(message.to_owned()),
                ..self
            }
        }

        /// The real download and verification, with `sums` standing in for SHA256SUMS.
        fn with_sums(self, sums: String) -> Self {
            Self {
                sums: Some(sums),
                ..self
            }
        }

        fn deny_checksums(self) -> Self {
            Self {
                deny_checksums: Arc::new(AtomicBool::new(true)),
                ..self
            }
        }
    }

    impl UpdateHost for Scripted {
        fn fetch_release(&mut self, _: &InstalledTarget) -> Result<Option<Release>, Failure> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            (self.check)()
        }
        fn download_release(
            &mut self,
            release: &Release,
            _: &InstalledTarget,
            attempt: &Path,
        ) -> Result<(PathBuf, String), Failure> {
            self.downloads.fetch_add(1, Ordering::SeqCst);
            let dir = attempt.join("asset");
            fs::create_dir_all(&dir).unwrap();
            let download = dir.join(&release.asset.name);
            let Some(sums) = &self.sums else {
                // A scripted download with a published digest needs no checksum command.
                fs::write(&download, b"asset").unwrap();
                return Ok((download, bytes_sha256(b"asset")));
            };
            fs::write(&download, "x".repeat(release.asset.size as usize)).unwrap();
            // The fixture's SHA256SUMS stands in for the release's, so nothing is fetched.
            let denied = self.deny_checksums.load(Ordering::SeqCst);
            let sums = sums.clone();
            let sha256 = verify_download(&release.tag, &release.asset, &download, |_| {
                if denied {
                    return Err(Failure::Auth(
                        "Run `gh auth login` so Shina can download updates.".into(),
                    ));
                }
                Ok(sums.clone())
            })?;
            Ok((download, sha256))
        }
        fn prepare(
            &mut self,
            _: &Release,
            _: &InstalledTarget,
            download: &Path,
        ) -> Result<PathBuf, Failure> {
            if let Some(message) = &self.prepare {
                return Err(Failure::quiet(message.clone()));
            }
            let prepared = download.with_extension("prepared");
            fs::write(&prepared, fs::read(download).unwrap()).unwrap();
            Ok(prepared)
        }
        fn reverify(&mut self, update: &PreparedUpdate) -> Result<(), String> {
            if self.reverify_fails {
                return Err("The downloaded update no longer matches its checksum.".into());
            }
            (file_sha256(&update.download)? == update.sha256)
                .then_some(())
                .ok_or_else(|| "The downloaded update no longer matches its checksum.".into())
        }
    }

    fn release(tag: &str) -> Release {
        Release {
            tag: tag.into(),
            asset: ReleaseAsset {
                name: "shina.exe".into(),
                size: 5,
                digest: Some(format!("sha256:{}", bytes_sha256(b"asset"))),
            },
        }
    }

    fn target() -> InstalledTarget {
        InstalledTarget::Windows {
            exe: crate::store::directory()
                .unwrap()
                .join("fixture-Shina")
                .join("shina.exe"),
        }
    }

    fn wake() -> Waker {
        Arc::new(|| {})
    }

    /// A fixture-local updater folder, so no test writes outside its own profile.
    fn storage(name: &str) -> Storage {
        let root = crate::store::directory().unwrap().join(name);
        fs::create_dir_all(&root).unwrap();
        Storage::at(root)
    }

    /// An updater that believes it runs from an installed copy, with isolated storage.
    fn installed(host: Scripted) -> (Updater, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        // Each updater in a test gets a separate manifest and attempts.
        static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (checks, downloads) = (Arc::clone(&host.checks), Arc::clone(&host.downloads));
        let mut updater = Updater::default();
        updater.configure(storage(&format!("updater-{serial}")), Box::new(host));
        updater.target = Some(target());
        updater.installed = true;
        (updater, checks, downloads)
    }

    /// Polls as the Rune loop does on each wake until `done` holds.
    fn settle(updater: &mut Updater, now: Instant, done: impl Fn(&Updater) -> bool) {
        for step in 0..500 {
            updater.poll(now + Duration::from_millis(step), wake());
            if done(updater) && updater.events.is_none() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!(
            "the updater never settled: {:?} {:?}",
            updater.status(),
            updater.diagnostics().collect::<Vec<_>>()
        );
    }

    fn newer() -> Scripted {
        Scripted::new(|| Ok(Some(release("v99.0.0"))))
    }

    /// A manifest and its files, as a previous run of `updater` left them.
    fn saved_update(updater: &Updater, tag: &str) -> PreparedUpdate {
        let attempt = updater.storage.attempt();
        fs::create_dir_all(&attempt).unwrap();
        let download = attempt.join("shina.exe");
        fs::write(&download, b"asset").unwrap();
        let update = PreparedUpdate {
            tag: tag.into(),
            target: target(),
            asset: "shina.exe".into(),
            size: 5,
            sha256: bytes_sha256(b"asset"),
            prepared: download.clone(),
            download,
        };
        fs::create_dir_all(&updater.storage.root).unwrap();
        fs::write(updater.storage.manifest(), update.to_value().to_string()).unwrap();
        update
    }

    #[test]
    fn compares_release_tags_with_the_running_version() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
        assert_eq!(parse_version("v1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
    }

    #[test]
    fn the_first_check_waits_two_seconds_then_each_attempt_waits_thirty_seconds() {
        let (mut updater, checks, _) = installed(newer());
        let start = Instant::now();
        updater.start(true, start, wake());
        assert_eq!(updater.status(), &Status::Idle);
        assert_eq!(updater.remaining(start), Some(FIRST_CHECK));
        updater.poll(start + FIRST_CHECK - Duration::from_secs(1), wake());
        assert_eq!(updater.status(), &Status::Idle, "nothing runs early");
        updater.poll(start + FIRST_CHECK, wake());
        assert_eq!(updater.status(), &Status::Checking);
        settle(&mut updater, start + FIRST_CHECK, |u| {
            u.prepared_update().is_some()
        });
        assert_eq!(updater.prepared_update().unwrap().tag, "v99.0.0");
        assert_eq!(checks.load(Ordering::SeqCst), 1);
        assert!(
            updater.remaining(start).is_none(),
            "a ready update runs no timer"
        );
    }

    #[test]
    fn a_newer_release_downloads_and_becomes_ready_without_a_click() {
        let (mut updater, _, downloads) = installed(newer());
        let start = Instant::now();
        updater.start(true, start, wake());
        updater.poll(start + FIRST_CHECK, wake());
        settle(&mut updater, start + FIRST_CHECK, |u| {
            u.prepared_update().is_some()
        });
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
        let update = updater.prepared_update().unwrap();
        assert_eq!(update.tag, "v99.0.0");
        assert_eq!(update.sha256, bytes_sha256(b"asset"));
        assert_eq!(update.asset, "shina.exe");
        let manifest: Value =
            serde_json::from_slice(&fs::read(updater.storage.manifest()).unwrap()).unwrap();
        assert_eq!(manifest["tag"], "v99.0.0");
        assert!(
            updater.remaining(start).is_none(),
            "a ready update runs no timer"
        );
    }

    #[test]
    fn only_one_operation_runs_at_a_time() {
        // Every operation this host handles sleeps well past the polls below.
        let started = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&started);
        let host = Scripted::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(1000));
            Ok(Some(release("v99.0.0")))
        });
        let (mut updater, _, _) = installed(host);
        let start = Instant::now();
        updater.start(true, start, wake());
        updater.poll(start + FIRST_CHECK, wake());
        // Under a loaded test run the worker can take a while to begin; wait for it, not a clock.
        let deadline = Instant::now() + Duration::from_secs(10);
        while started.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        for tick in 0..10 {
            thread::sleep(Duration::from_millis(5));
            updater.poll(start + FIRST_CHECK + Duration::from_millis(tick), wake());
        }
        assert_eq!(
            started.load(Ordering::SeqCst),
            1,
            "a check in flight blocks another"
        );
        assert_eq!(updater.status(), &Status::Checking);
        settle(&mut updater, start + FIRST_CHECK, |u| {
            u.prepared_update().is_some()
        });
        assert_eq!(started.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn no_newer_release_returns_to_idle_and_schedules_the_next_check() {
        let (mut updater, _, _) = installed(Scripted::new(|| Ok(None)));
        let start = Instant::now();
        updater.start(true, start, wake());
        updater.poll(start + FIRST_CHECK, wake());
        settle(&mut updater, start + FIRST_CHECK, |u| {
            u.status() == &Status::Idle
        });
        assert!(updater.diagnostics().next().is_none());
        let remaining = updater.remaining(start + FIRST_CHECK).unwrap();
        assert!(
            remaining <= INTERVAL + Duration::from_secs(2)
                && remaining > INTERVAL - Duration::from_secs(1),
            "the next check waits the interval: {remaining:?}"
        );
    }

    #[test]
    fn a_failed_check_stays_quiet_and_retries_at_the_next_interval() {
        let (mut updater, checks, _) = installed(Scripted::new(|| {
            Err(Failure::quiet("Update check failed: gh is not installed"))
        }));
        let start = Instant::now();
        updater.start(true, start, wake());
        updater.poll(start + FIRST_CHECK, wake());
        settle(&mut updater, start + FIRST_CHECK, |u| {
            u.status() == &Status::Idle
        });
        assert_eq!(updater.diagnostics().count(), 1);
        let next = start + FIRST_CHECK + INTERVAL + Duration::from_secs(1);
        updater.poll(next, wake());
        assert_eq!(
            updater.status(),
            &Status::Checking,
            "the next interval tries again"
        );
        for _ in 0..200 {
            if checks.load(Ordering::SeqCst) == 2 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(checks.load(Ordering::SeqCst), 2, "no sticky error");
    }

    #[test]
    fn an_authentication_denial_stops_attempts_until_the_next_launch() {
        let (mut updater, checks, _) = installed(Scripted::new(|| {
            Err(Failure::Auth(
                "Run `gh auth login` so Shina can download updates.".into(),
            ))
        }));
        let start = Instant::now();
        updater.start(true, start, wake());
        updater.poll(start + FIRST_CHECK, wake());
        settle(&mut updater, start + FIRST_CHECK, |u| {
            u.status() == &Status::Idle
        });
        assert_eq!(checks.load(Ordering::SeqCst), 1);
        assert_eq!(
            updater.suspended(),
            Some("Run `gh auth login` so Shina can download updates.")
        );
        assert_eq!(
            updater.remaining(start + FIRST_CHECK),
            None,
            "a suspended updater schedules no deadline"
        );
        updater.poll(start + FIRST_CHECK + INTERVAL * 4, wake());
        assert_eq!(checks.load(Ordering::SeqCst), 1);
        assert_eq!(updater.status(), &Status::Idle);
        assert_eq!(
            updater.remaining(start + FIRST_CHECK + INTERVAL * 4),
            None,
            "the deadline never comes back"
        );
    }

    #[test]
    fn an_authentication_denial_fetching_checksums_suspends_attempts() {
        // The failure stays typed through verification, so the updater suspends instead of
        // retrying every interval. The latest release is newer but publishes no digest.
        let mut latest = release("v99.0.0");
        latest.asset.digest = None;
        let host = Scripted::new(move || Ok(Some(latest.clone())))
            .with_sums(String::new())
            .deny_checksums();
        let (mut updater, _, _) = installed(host);
        let start = Instant::now();
        updater.start(true, start, wake());
        updater.poll(start + FIRST_CHECK, wake());
        settle(&mut updater, start + FIRST_CHECK, |u| u.events.is_none());
        assert_eq!(
            updater.suspended(),
            Some("Run `gh auth login` so Shina can download updates.")
        );
        assert_eq!(updater.status(), &Status::Idle);
        assert_eq!(
            updater.remaining(start + FIRST_CHECK),
            None,
            "suspension schedules nothing"
        );
        assert!(updater.prepared_update().is_none());
    }

    #[test]
    fn diagnostics_are_bounded_and_tokens_are_redacted() {
        let line = bounded_line("error: HTTP 401: Bad credentials for token ghp_secretvalue");
        assert!(line.contains("[redacted]"));
        assert!(!line.contains("ghp_secretvalue"));
        let long = bounded_line(&"x".repeat(400));
        assert_eq!(long, format!("Update check failed: {}", "x".repeat(200)));
        assert!(matches!(
            failure_from_stderr("error: authentication required"),
            Failure::Auth(_)
        ));
        assert!(matches!(
            failure_from_stderr("error: connection reset"),
            Failure::Quiet(_)
        ));
        assert_eq!(redact("ghs_abc failed"), "[redacted] failed");
    }

    #[test]
    fn a_prepared_update_survives_later_checks() {
        let (mut updater, checks, downloads) = installed(newer());
        let start = Instant::now();
        updater.start(true, start, wake());
        updater.poll(start + FIRST_CHECK, wake());
        settle(&mut updater, start + FIRST_CHECK, |u| {
            u.prepared_update().is_some()
        });
        let ready = updater.prepared_update().cloned().unwrap();
        updater.next_check = Some(Instant::now());
        for tick in 0..20 {
            updater.poll(Instant::now() + Duration::from_secs(tick), wake());
        }
        assert_eq!(updater.status(), &Status::Ready { update: ready });
        assert_eq!(checks.load(Ordering::SeqCst), 1, "Ready keeps its download");
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_prepared_update_from_a_previous_run_is_revalidated_before_ready() {
        let (mut updater, checks, _) = installed(Scripted::new(|| Ok(None)));
        let saved = saved_update(&updater, "v99.0.0");
        let start = Instant::now();
        updater.start(true, start, wake());
        settle(&mut updater, start, |u| u.prepared_update().is_some());
        assert_eq!(updater.prepared_update().unwrap().tag, "v99.0.0");
        assert_eq!(checks.load(Ordering::SeqCst), 0, "recovery needs no check");
        assert!(saved.prepared.exists(), "the recovered download stays");
        assert_eq!(
            updater.remaining(start),
            None,
            "a recovered Ready keeps no startup timer"
        );
        updater.poll(start + FIRST_CHECK * 10, wake());
        assert_eq!(
            updater.remaining(start + FIRST_CHECK * 10),
            None,
            "the startup deadline never comes back"
        );
        assert_eq!(checks.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn startup_recovery_keeps_a_verified_prepared_exe_and_deletes_only_orphans() {
        let (mut updater, _, _) = installed(Scripted::new(|| panic!("recovery must not fetch")));
        updater.host = Arc::new(Mutex::new(Box::new(Gh)));
        let mut saved = saved_update(&updater, "v99.0.0");
        let InstalledTarget::Windows { exe } = &saved.target else {
            unreachable!()
        };
        saved.prepared = install::prepare_exe(&saved.download, exe.parent().unwrap()).unwrap();
        updater.write_manifest(&saved).unwrap();
        let orphan = updater.storage.root.join("attempt-orphan");
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("partial"), b"partial").unwrap();
        let start = Instant::now();
        updater.start(true, start, wake());
        assert!(
            saved.download.is_file(),
            "read the manifest before deleting downloads"
        );
        assert!(
            saved.prepared.is_file(),
            "read the manifest before deleting shina.new.exe"
        );
        settle(&mut updater, start, |u| u.prepared_update().is_some());
        assert!(!orphan.exists());
        assert_eq!(updater.prepared_update(), Some(&saved));
        assert_eq!(updater.remaining(start + INTERVAL), None);
    }

    #[test]
    fn startup_removes_an_unreferenced_prepared_bundle() {
        let root = storage("orphan-bundle").root;
        let app = root.join("Applications/Shina.app");
        let staging = install::bundle_staging(&app);
        fs::create_dir_all(staging.join("Shina.app/Contents")).unwrap();
        let mut updater = Updater {
            installed: true,
            target: Some(InstalledTarget::MacBundle { app }),
            storage: Storage::at(root.join("updates")),
            ..Updater::default()
        };
        updater.start(true, Instant::now(), wake());
        assert!(!staging.exists());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_failed_mac_launch_restores_and_reopens_the_installed_bundle_not_the_failed_backup() {
        let root = storage("launch-restore").root;
        let app = root.join("Applications/Shina.app");
        let executable = |path: &Path| path.join("Contents/MacOS/shina");
        fs::create_dir_all(executable(&app).parent().unwrap()).unwrap();
        fs::write(executable(&app), b"old").unwrap();
        let source = root.join("incoming/Shina.app");
        fs::create_dir_all(executable(&source).parent().unwrap()).unwrap();
        fs::write(executable(&source), b"broken").unwrap();
        let prepared =
            install::prepare_macos_bundle(&source, &install::bundle_staging(&app)).unwrap();
        install::commit_macos_bundle(&prepared, &app).unwrap();
        let update = PreparedUpdate {
            tag: "v99.0.0".into(),
            target: InstalledTarget::MacBundle { app: app.clone() },
            asset: "fixture.zip".into(),
            size: 1,
            sha256: "0".repeat(64),
            download: root.join("fixture.zip"),
            prepared,
        };
        let mut launches = Vec::new();
        relaunch_or_restore_with(&update, &app, &executable(&app), |path, _| {
            launches.push(path.to_path_buf());
            let contents = fs::read(executable(path)).unwrap();
            if launches.len() == 1 {
                assert_eq!(contents, b"broken");
                Err("Launch Services rejected the build".into())
            } else {
                assert_eq!(contents, b"old");
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(launches, vec![app.clone(), app]);
    }

    #[test]
    fn a_failed_exe_launch_restores_and_reopens_the_original_build() {
        let root = storage("exe-launch-restore").root;
        let exe = root.join("install/shina.exe");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"old").unwrap();
        let download = root.join("download");
        fs::write(&download, b"broken").unwrap();
        let prepared = install::prepare_exe(&download, exe.parent().unwrap()).unwrap();
        install::commit_exe(&prepared, exe.parent().unwrap()).unwrap();
        let update = PreparedUpdate {
            tag: "v99.0.0".into(),
            target: InstalledTarget::Windows { exe: exe.clone() },
            asset: "shina.exe".into(),
            size: 6,
            sha256: bytes_sha256(b"broken"),
            download,
            prepared,
        };
        let mut launches = 0;
        relaunch_or_restore_with(&update, &exe, &root.join("helper.exe"), |path, _| {
            launches += 1;
            assert_eq!(path, exe);
            if launches == 1 {
                assert_eq!(fs::read(path).unwrap(), b"broken");
                Err("The executable cannot start".into())
            } else {
                assert_eq!(fs::read(path).unwrap(), b"old");
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(launches, 2);
        assert!(update.download.is_file(), "keep the downloaded asset");
    }

    #[test]
    fn a_failed_launch_without_a_backup_does_not_retry_the_failed_build() {
        let root = storage("missing-relaunch-backup").root;
        let exe = root.join("shina.exe");
        fs::write(&exe, b"broken").unwrap();
        let update = PreparedUpdate {
            tag: "v99.0.0".into(),
            target: InstalledTarget::Windows { exe: exe.clone() },
            asset: "shina.exe".into(),
            size: 6,
            sha256: bytes_sha256(b"broken"),
            download: root.join("download"),
            prepared: root.join("prepared"),
        };
        let mut launches = 0;
        let error = relaunch_or_restore_with(&update, &exe, &root.join("helper.exe"), |_, _| {
            launches += 1;
            Err("launch failed".into())
        })
        .unwrap_err();
        assert_eq!(launches, 1);
        assert!(error.contains("no previous"), "{error}");
        assert_eq!(fs::read(exe).unwrap(), b"broken");
    }

    #[test]
    fn a_damaged_prepared_update_is_dropped_instead_of_offered() {
        let mut host = Scripted::new(|| Ok(None));
        host.reverify_fails = true;
        let (mut updater, _, _) = installed(host);
        let saved = saved_update(&updater, "v99.0.0");
        let start = Instant::now();
        updater.start(true, start, wake());
        settle(&mut updater, start, |u| u.diagnostics().next().is_some());
        assert!(updater.prepared_update().is_none());
        assert_eq!(updater.status(), &Status::Idle);
        assert!(
            !updater.storage.manifest().exists(),
            "the manifest is cleared"
        );
        assert_eq!(updater.diagnostics().count(), 1);
        assert!(
            !saved.prepared.exists(),
            "the damaged download is removed with its attempt"
        );
        // A failed recovery schedules the next check a full interval after it.
        let next = updater.next_check.expect("a failed recovery retries later");
        assert!(next >= start + INTERVAL, "the retry waits the interval");
    }

    #[test]
    fn a_manifest_for_another_app_folder_is_never_installed() {
        let (mut updater, _, _) = installed(Scripted::new(|| Ok(None)));
        let mut saved = saved_update(&updater, "v99.0.0");
        saved.target = InstalledTarget::Windows {
            exe: updater.storage.root.join("elsewhere").join("shina.exe"),
        };
        fs::write(updater.storage.manifest(), saved.to_value().to_string()).unwrap();
        let start = Instant::now();
        updater.start(true, start, wake());
        assert!(updater.prepared_update().is_none());
        assert_eq!(
            updater.remaining(start),
            Some(FIRST_CHECK),
            "another folder's manifest is ignored and the first check is kept"
        );
    }

    #[test]
    fn a_failed_preparation_keeps_no_download_and_retries_later() {
        let host = newer().failing_prepare("The applications folder is not writable.");
        let (mut updater, _, downloads) = installed(host);
        let start = Instant::now();
        updater.start(true, start, wake());
        updater.poll(start + FIRST_CHECK, wake());
        settle(&mut updater, start + FIRST_CHECK, |u| u.events.is_none());
        assert_eq!(updater.status(), &Status::Idle);
        assert_eq!(downloads.load(Ordering::SeqCst), 1);
        let attempts: Vec<_> = fs::read_dir(&updater.storage.root)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("attempt-"))
            })
            .collect();
        assert!(attempts.is_empty(), "the failed attempt is removed");
        assert!(
            updater
                .diagnostics()
                .any(|line| line.contains("not writable")),
            "the failure is recorded quietly"
        );
        assert!(updater.prepared_update().is_none());
        assert!(
            updater.remaining(start + FIRST_CHECK).is_some(),
            "the next interval retries"
        );
    }

    #[test]
    fn a_development_build_and_demo_runs_are_disabled() {
        let start = Instant::now();
        let mut dev = Updater::default();
        dev.disable();
        dev.start(true, start, wake());
        assert_eq!(dev.status(), &Status::Disabled);
        assert_eq!(dev.remaining(start), None);
        assert!(!dev.poll(start + INTERVAL * 10, wake()));
        let mut demo = Updater::default();
        demo.start(false, start, wake());
        assert_eq!(demo.status(), &Status::Disabled);
        assert_eq!(demo.remaining(start), None);
    }

    #[test]
    fn platform_asset_selection_requires_the_exact_name_and_a_declared_size() {
        let release = serde_json::json!({"tagName":"v1.0.0","assets":[
            {"name":"shina.exe","size":2048,"digest":"sha256:ABCDEF"},
            {"name":"Shina-macos-aarch64.zip","size":4096},
            {"name":"Shina-macos-x86_64.zip","size":"8192","digest":{"value":"sha256:ff"}}
        ]});
        let windows = select_asset(
            &release,
            &InstalledTarget::Windows {
                exe: PathBuf::from("shina.exe"),
            },
        )
        .unwrap();
        assert_eq!(windows.size, 2048);
        assert_eq!(windows.digest.as_deref(), Some("sha256:ABCDEF"));
        let mac = select_asset(
            &release,
            &InstalledTarget::MacBundle {
                app: PathBuf::from("Shina.app"),
            },
        )
        .unwrap();
        assert_eq!(
            mac.name,
            if cfg!(all(not(windows), target_arch = "x86_64")) {
                "Shina-macos-x86_64.zip"
            } else {
                "Shina-macos-aarch64.zip"
            }
        );
        let without_size = serde_json::json!({"assets":[{"name":"shina.exe"}]});
        assert!(
            select_asset(
                &without_size,
                &InstalledTarget::Windows {
                    exe: PathBuf::from("shina.exe")
                }
            )
            .is_none()
        );
        let missing = serde_json::json!({"assets":[{"name":"shina","size":1}]});
        assert!(
            select_asset(
                &missing,
                &InstalledTarget::Windows {
                    exe: PathBuf::from("shina.exe")
                }
            )
            .is_none()
        );
    }

    #[test]
    fn partial_files_a_size_mismatch_and_a_digest_mismatch_are_refused() {
        let file = storage("verify").root.join("download.exe");
        fs::write(&file, b"asset").unwrap();
        let mut asset = ReleaseAsset {
            name: "shina.exe".into(),
            size: 5,
            digest: Some(format!("sha256:{}", bytes_sha256(b"asset"))),
        };
        let offline = |_: &str| -> Result<String, Failure> { panic!("unexpected checksum fetch") };
        assert!(verify_download("v1", &asset, &file, offline).is_ok());
        asset.size = 99;
        assert!(matches!(
            verify_download("v1", &asset, &file, offline),
            Err(Failure::Quiet(_))
        ));
        asset.size = 5;
        asset.digest = Some(format!("sha256:{}", "0".repeat(64)));
        assert!(verify_download("v1", &asset, &file, offline).is_err());
        asset.digest = Some("sha256:nothex".into());
        assert!(verify_download("v1", &asset, &file, offline).is_err());
        fs::write(&file, b"ass").unwrap();
        assert!(
            verify_download("v1", &asset, &file, offline).is_err(),
            "a partial file fails its size"
        );
    }

    #[test]
    fn a_release_without_digests_verifies_against_its_sha256sums_file() {
        // The checksums come from a fixture file, so no gh process runs: offline.
        let root = storage("sums").root;
        let file = root.join("shina.exe");
        fs::write(&file, b"asset").unwrap();
        let asset = ReleaseAsset {
            name: "shina.exe".into(),
            size: 5,
            digest: None,
        };
        let digest = bytes_sha256(b"asset");
        let sums = root.join(SHA256SUMS);
        let serve = |text: &str| {
            fs::write(&sums, text).unwrap();
            verify_download("v1", &asset, &file, |_| {
                fs::read_to_string(&sums)
                    .map_err(|_| Failure::quiet(format!("Could not read {SHA256SUMS}.")))
            })
        };
        let matching = format!(
            "{digest}  shina.exe\n{} *Shina-macos-aarch64.zip\n",
            "0".repeat(64)
        );
        assert_eq!(serve(&matching).unwrap(), digest);
        let other = format!("{}  other.exe\n", "0".repeat(64));
        assert!(
            matches!(serve(&other), Err(Failure::Quiet(_))),
            "a mismatched checksum is quiet"
        );
        assert!(matches!(serve(""), Err(Failure::Quiet(_))));
        // A published digest needs no checksums file at all.
        let with_digest = ReleaseAsset {
            digest: Some(format!("sha256:{digest}")),
            ..asset
        };
        let never_fetched = |_: &str| Err::<String, Failure>(Failure::quiet("never fetched"));
        assert_eq!(
            expected_sha256("v1", &with_digest, never_fetched).unwrap(),
            digest
        );
    }

    #[test]
    fn a_sha256sums_file_covers_releases_without_asset_digests() {
        let one = "1".repeat(64);
        let two = "2".repeat(64);
        let checksums = format!("{one}  shina.exe\n{two} *Shina-macos-aarch64.zip\n");
        assert_eq!(sha256s_entry(&checksums, "shina.exe"), Some(one.as_str()));
        assert_eq!(
            sha256s_entry(&checksums, "Shina-macos-aarch64.zip"),
            Some(two.as_str())
        );
        assert_eq!(sha256s_entry(&checksums, "missing.exe"), None);
        assert_eq!(sha256s_entry("garbage\n", "shina.exe"), None);
        assert_eq!(
            named_asset(
                &serde_json::json!({"assets":[{"name":"shina.exe","size":5}]}),
                "shina.exe"
            )
            .map(|asset| asset.size),
            Some(5)
        );
    }

    /// Entry lines as `zipinfo -l` prints them.
    const DIR: &str = "drwxr-xr-x  3.0 unx        0 bx        0 stor 26-Nov-09 14:55 Shina.app/";
    const MACOS_DIR: &str =
        "drwxr-xr-x  3.0 unx        0 bx        0 stor 26-Nov-09 14:55 Shina.app/Contents/MacOS/";
    const BIN: &str =
        "-rwxr-xr-x  3.0 unx  1234 tx     567 defN 26-Nov-09 14:55 Shina.app/Contents/MacOS/shina";
    const LINK: &str = "lrwxr-xr-x  3.0 unx       11 bx       11 stor 26-Nov-09 14:55 Shina.app/lib -> /etc/passwd";
    fn listing(entries: &[&str]) -> String {
        format!(
            "Archive:  Shina-macos-aarch64.zip\nZip file size: 900 bytes, number of entries: {}\n{}\n{} files, 1234 bytes uncompressed, 567 bytes compressed:  53.8%\n",
            entries.len(),
            entries.join("\n"),
            entries.len()
        )
    }

    #[test]
    fn archive_entries_outside_the_bundle_and_links_are_rejected() {
        assert!(archive_entry_problem(&listing(&[DIR, MACOS_DIR, BIN])).is_none());
        assert!(archive_entry_problem("").is_some());
        let outside = |path: &str| {
            let line = format!("-rw-r--r--  3.0 unx  1 tx  1 stor 26-Nov-09 14:55 {path}");
            archive_entry_problem(&listing(&[line.as_str()]))
        };
        assert!(outside("/etc/passwd").is_some());
        assert!(outside("Shina.app/../evil").is_some());
        assert!(outside("Other.app/Contents/Info.plist").is_some());
        assert!(outside("README.txt").is_some());
        // A symlink's name looks like any other path, so its mode has to reject it.
        let problem = archive_entry_problem(&listing(&[DIR, LINK])).expect("a link is refused");
        assert!(problem.contains("link"), "{problem}");
        let fifo =
            "prw-r--r--  3.0 unx  0 bx  0 stor 26-Nov-09 14:55 Shina.app/Contents/MacOS/pipe";
        assert!(archive_entry_problem(&listing(&[fifo])).is_some());
    }

    #[test]
    fn an_incomplete_archive_listing_fails_closed() {
        assert_eq!(
            zipinfo_complete(&listing(&[DIR, MACOS_DIR, BIN])),
            Some(true)
        );
        // A listing cut off before its footer may hide entries, so it is never accepted.
        let full = listing(&[DIR, MACOS_DIR, BIN]);
        let cut_off = &full[..full.find("Shina.app/Contents/MacOS/shina").unwrap()];
        assert_eq!(zipinfo_complete(cut_off), Some(false));
        assert_eq!(
            zipinfo_complete(&listing(&[DIR, MACOS_DIR, BIN]).replace(" uncompressed, ", " ")),
            Some(false)
        );
        assert_eq!(
            zipinfo_complete("Archive:  empty.zip\nEmpty zipfile.\n"),
            Some(false)
        );
    }

    #[test]
    fn a_path_with_spaces_is_read_whole_from_its_column() {
        let spaced = "-rwxr-xr-x  3.0 unx  1234 tx     567 defN 26-Nov-09 14:55 Shina.app/My App";
        assert_eq!(
            zipinfo_entry(spaced),
            Some(('-', "Shina.app/My App")),
            "the path ends the line"
        );
        assert_eq!(zipinfo_entry(DIR), Some(('d', "Shina.app/")));
        assert_eq!(zipinfo_entry(MACOS_DIR).map(|(kind, _)| kind), Some('d'));
        assert_eq!(zipinfo_entry("Archive:  Shina-macos-aarch64.zip"), None);
        assert_eq!(zipinfo_entry("3 files, 1234 bytes uncompressed"), None);
        let timestamp_in_name = format!("{BIN} 14:55 extra");
        assert_eq!(
            zipinfo_entry(&timestamp_in_name).unwrap().1,
            "Shina.app/Contents/MacOS/shina 14:55 extra"
        );
    }

    #[test]
    fn archive_validation_rejects_unparsed_records_wrong_counts_trailing_data_and_controls() {
        let valid = listing(&[DIR, MACOS_DIR, BIN]);
        for invalid in [
            valid.replace(BIN, "not an archive entry"),
            valid.replace("3 files,", "2 files,"),
            valid.replace("number of entries: 3", "number of entries: 4"),
            format!("{valid}{BIN}\n"),
            valid.replace("shina\n", "shina\tunsafe\n"),
            valid.replace("shina\n", "shina\u{1b}unsafe\n"),
        ] {
            assert!(archive_entry_problem(&invalid).is_some(), "{invalid:?}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn command_output_limits_fail_instead_of_silently_truncating_metadata() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf '0123456789abcdef'"]);
        assert!(
            matches!(run_bounded_with_limit(command, Duration::from_secs(2), 8),
            Err(Failure::Quiet(message)) if message.contains("too large"))
        );
        let read = read_on_thread(std::io::Cursor::new(vec![b'x'; 1024 * 1024]), 32);
        assert!(matches!(read.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(Failure::Quiet(message)) if message.contains("too large")));
    }

    #[test]
    #[cfg(unix)]
    fn a_timed_out_command_is_killed_and_reaped_without_waiting_for_its_sleep() {
        let pid_file = storage("timeout-process").root.join("pid");
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "printf '%s' \"$$\" > \"$1\"; exec /bin/sleep 10",
                "shina-timeout-fixture",
            ])
            .arg(&pid_file);
        let start = Instant::now();
        assert!(matches!(run_bounded(command, Duration::from_millis(500)),
            Err(Failure::Quiet(message)) if message.contains("did not finish in time")));
        assert!(start.elapsed() < Duration::from_secs(2));
        let pid = fs::read_to_string(pid_file)
            .unwrap()
            .parse::<u32>()
            .unwrap();
        // Signal zero changes nothing. It also finds zombies, so a nonzero result proves
        // the child neither kept running nor remained as an unreaped child.
        let alive = Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!alive.success(), "the timed-out child still exists");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_real_system_zip_listing_passes_preflight_and_extracts_only_the_fixture_bundle() {
        let root = storage("native-archive").root;
        let stage = root.join("stage");
        let binary = stage.join("Shina.app/Contents/MacOS/shina");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"offline fixture").unwrap();
        let zip = root.join("fixture.zip");
        let status = Command::new("/usr/bin/ditto")
            .args([
                "--norsrc",
                "--noextattr",
                "--noqtn",
                "-c",
                "-k",
                "--keepParent",
            ])
            .arg(stage.join("Shina.app"))
            .arg(&zip)
            .status()
            .unwrap();
        assert!(status.success());
        let destination = root.join("extracted");
        unpack_macos_zip(&zip, &destination).unwrap();
        assert_eq!(
            fs::read(destination.join("Shina.app/Contents/MacOS/shina")).unwrap(),
            b"offline fixture"
        );
    }
}
