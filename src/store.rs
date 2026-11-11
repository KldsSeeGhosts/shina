//! One app-wide index. Legacy workspace files are read, never changed or deleted.
use rune::Waker;
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

/// Exists for the lifetime of a run, including the interval between a cancelled quit
/// request and the next frame. Only an actual drop following a clean save removes it.
pub struct RunMarker {
    path: PathBuf,
    pub previous_unclean: bool,
    refreshed: std::time::Instant,
    pub clean: bool,
}

impl RunMarker {
    pub fn start(directory: &std::path::Path) -> Result<Self, String> {
        let path = directory.join("run-marker.json");
        let previous_unclean = match fs::metadata(&path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(format!("Could not inspect Shina's run marker: {error}")),
        };
        let mut marker = Self {
            path,
            previous_unclean,
            refreshed: std::time::Instant::now(),
            clean: false,
        };
        marker.write()?;
        Ok(marker)
    }

    fn write(&mut self) -> Result<(), String> {
        fs::write(
            &self.path,
            serde_json::json!({"pid":std::process::id(),"refreshed_at":crate::lifecycle::now()})
                .to_string(),
        )
        .map_err(|error| format!("Could not write Shina's run marker: {error}"))?;
        self.refreshed = std::time::Instant::now();
        Ok(())
    }

    pub fn tick(&mut self) -> Result<(), String> {
        self.clean = false;
        if self.refreshed.elapsed() >= Duration::from_secs(60) {
            self.write()?;
        }
        Ok(())
    }
}

impl Drop for RunMarker {
    fn drop(&mut self) {
        if self.clean {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub struct Store {
    database: Option<(crate::database::Database, String)>,
    database_error: Option<String>,
    sender: Option<mpsc::Sender<Message>>,
    pub errors: mpsc::Receiver<String>,
    last: String,
    worker: Option<thread::JoinHandle<()>>,
    healthy: Arc<AtomicBool>,
}

/// Synchronous durable replacement for small ledgers that must commit before dispatch.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(database) = crate::database::for_path(path) {
        let value = serde_json::from_slice(bytes)?;
        return database
            .write(
                path.file_name()
                    .expect("database document")
                    .to_str()
                    .expect("UTF-8"),
                value,
            )
            .map_err(std::io::Error::other);
    }
    use std::io::Write;
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| std::io::Error::other("Missing parent"))?,
    )?;
    let temporary = path.with_extension("json.tmp");
    let mut staged = fs::File::create(&temporary)?;
    staged.write_all(bytes)?;
    staged.sync_all()?;
    drop(staged);
    fs::rename(&temporary, path)?;
    #[cfg(unix)]
    fs::File::open(path.parent().expect("checked parent"))?.sync_all()?;
    Ok(())
}

/// A queued write, or a barrier that is acknowledged after every earlier write finished.
enum Message {
    Save(String),
    Flush(mpsc::Sender<Result<(), String>>),
}

#[cfg(not(test))]
pub fn directory() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }?;
    Some(base.join("Shina"))
}

// Tests never resolve Shina storage through the user's environment. Each test thread
// gets an owned temporary profile, including any MCP bridge files or scratch chats.
#[cfg(test)]
thread_local! {
    static TEST_DIRECTORY: TestDirectory = TestDirectory::new();
    static TEST_OVERRIDE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
struct TestDirectory(PathBuf);

#[cfg(test)]
impl TestDirectory {
    fn new() -> Self {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "shina-test-profile-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos(),
            SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        fs::create_dir(&directory).expect("isolated test profile");
        Self(directory)
    }
}

#[cfg(test)]
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
pub fn directory() -> Option<PathBuf> {
    TEST_OVERRIDE
        .with(|directory| directory.borrow().clone())
        .or_else(|| Some(TEST_DIRECTORY.with(|directory| directory.0.clone())))
}

#[cfg(test)]
pub fn with_directory<T>(directory: PathBuf, action: impl FnOnce() -> T) -> T {
    struct Reset(Option<PathBuf>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_OVERRIDE.with(|directory| *directory.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(TEST_OVERRIDE.with(|slot| slot.replace(Some(directory))));
    action()
}

pub fn legacy_path(directory: &std::path::Path, workspace: &std::path::Path) -> PathBuf {
    // A stable, non-cryptographic filename. This is not a security boundary.
    let hash = workspace
        .to_string_lossy()
        .bytes()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ byte as u64).wrapping_mul(0x100000001b3)
        });
    let name = format!("workspace-{hash:016x}.json");
    directory.join(name)
}

/// A failed import leaves global saving disabled, so no partial migration can replace data.
pub fn load_global(
    directory: &std::path::Path,
    launch: Option<&std::path::Path>,
) -> Result<Option<Value>, String> {
    let global_path = directory.join("index.json");
    if let Some(database) = crate::database::for_path(&global_path) {
        return database.read("index.json");
    }
    if let Some(mut index) = load(&global_path)? {
        let projects = index["projects"].as_array().cloned().unwrap_or_default();
        if let Some(threads) = index["threads"].as_array_mut() {
            for record in threads {
                if !record.is_object() {
                    return Err("The saved index contains a non-object thread.".into());
                }
                for field in ["updated", "session"] {
                    if record.get(field).is_some_and(|value| {
                        !value.is_null()
                            && if field == "updated" {
                                !value.is_u64()
                            } else {
                                !value.is_string()
                            }
                    }) {
                        return Err(format!("The saved index contains an invalid {field}."));
                    }
                }
                let workspace = projects
                    .iter()
                    .find(|p| p["id"] == record["project"])
                    .and_then(|p| p["path"].as_str())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| directory.join("chats").join(record["id"].to_string()));
                restore_activity(record, &workspace, modified(&global_path));
            }
        }
        return Ok(Some(index));
    }
    migrate_legacy(directory, launch, &std::env::temp_dir())
}

fn modified(path: &std::path::Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn restore_activity(record: &mut Value, workspace: &std::path::Path, index_time: u64) {
    if record["updated"].as_u64().is_some_and(|time| time > 0) {
        return;
    }
    let session_time = record["session"]
        .as_str()
        .map(|session| {
            let session = crate::projects::clean_path(std::path::Path::new(session));
            modified(&if session.is_absolute() {
                session
            } else {
                workspace.join(session)
            })
        })
        .unwrap_or(0);
    record["updated"] = if session_time > 0 {
        session_time
    } else {
        index_time
    }
    .into();
}

fn under_directory(path: &std::path::Path, directory: &std::path::Path) -> bool {
    path.ancestors()
        .any(|ancestor| crate::projects::same_path(ancestor, directory))
}

fn migrate_legacy(
    directory: &std::path::Path,
    launch: Option<&std::path::Path>,
    temporary: &std::path::Path,
) -> Result<Option<Value>, String> {
    let mut paths = Vec::new();
    let recents = load(&directory.join("projects.json"))?;
    if let Some(recents) = &recents {
        for record in recents["projects"]
            .as_array()
            .ok_or("Invalid legacy project list.")?
        {
            let path = record["path"]
                .as_str()
                .ok_or("Invalid legacy project path.")?;
            paths.push(PathBuf::from(path));
        }
    }
    match fs::read_to_string(directory.join("last-workspace.txt")) {
        Ok(path) if !path.trim().is_empty() => paths.push(PathBuf::from(path.trim())),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Could not read last-workspace.txt: {e}")),
    }
    if let Some(launch) = launch {
        paths.push(launch.to_path_buf());
    }
    let mut unique: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for path in paths {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        let clean = crate::projects::normalize_path(&path);
        let variants = [path, canonical, clean.clone()];
        if let Some((_, aliases)) = unique
            .iter_mut()
            .find(|(p, _)| crate::projects::same_path(p, &clean))
        {
            for variant in variants {
                if !aliases.contains(&variant) {
                    aliases.push(variant);
                }
            }
        } else {
            let mut aliases = Vec::new();
            for variant in variants {
                if !aliases.contains(&variant) {
                    aliases.push(variant);
                }
            }
            unique.push((clean, aliases));
        }
    }
    if unique.is_empty() {
        return Ok(None);
    }
    let mut projects = Vec::new();
    let mut threads = Vec::new();
    let mut captains: Vec<Value> = crate::captains::Captain::presets()
        .iter()
        .map(crate::captains::Captain::to_value)
        .collect();
    let mut used = std::collections::HashSet::new();
    let mut preferences = serde_json::json!({});
    for (path, aliases) in unique {
        let missing = !path.is_dir();
        if missing && under_directory(&path, temporary) {
            continue;
        }
        let project_id = projects.len() + 1;
        let mut project = crate::projects::Project::new(project_id, path.clone());
        let first_thread = threads.len();
        let mut seen_indexes = std::collections::HashSet::new();
        for alias in aliases {
            let legacy = legacy_path(directory, &alias);
            let previous = directory
                .parent()
                .unwrap_or(directory)
                .join("Still")
                .join(legacy.file_name().expect("legacy filename"));
            let source = if legacy.exists() { &legacy } else { &previous };
            if !seen_indexes.insert(source.clone()) {
                continue;
            }
            let index = load(source)?;
            if let Some(index) = index {
                if index["version"] != 1 && index["version"] != 2 {
                    return Err(format!(
                        "Unsupported legacy index version in {}.",
                        legacy.display()
                    ));
                }
                let local_captains = if index["version"] == 2 {
                    index["captains"]
                        .as_array()
                        .filter(|p| !p.is_empty())
                        .ok_or("Invalid legacy captain roster.")?
                        .clone()
                } else {
                    crate::captains::Captain::presets()
                        .iter()
                        .map(crate::captains::Captain::to_value)
                        .collect()
                };
                let mut captain_ids = Vec::new();
                for profile in local_captains {
                    crate::captains::Captain::from_value(&profile)
                        .ok_or("Invalid legacy captain profile.")?;
                    let id = captains
                        .iter()
                        .position(|p| *p == profile)
                        .unwrap_or_else(|| {
                            captains.push(profile);
                            captains.len() - 1
                        });
                    captain_ids.push(id);
                }
                let records = index["threads"]
                    .as_array()
                    .ok_or("Invalid legacy thread list.")?;
                let mut remap = std::collections::HashMap::new();
                for record in records {
                    let old = record["id"]
                        .as_u64()
                        .filter(|&id| id > 0 && id < usize::MAX as u64)
                        .ok_or("Invalid legacy thread id.")?;
                    let mut id = old;
                    while used.contains(&id) {
                        id = id
                            .checked_add(1)
                            .filter(|&id| id < usize::MAX as u64)
                            .ok_or("Thread ids exhausted.")?;
                    }
                    if remap.insert(old, id).is_some() {
                        return Err("Duplicate thread ids in a legacy index.".into());
                    }
                    used.insert(id);
                }
                let mut imported = Vec::new();
                for record in records {
                    let mut record = record.clone();
                    let old = record["id"].as_u64().expect("validated id");
                    record["id"] = remap[&old].into();
                    record["project"] = project_id.into();
                    record["machine"] = "local".into();
                    restore_activity(&mut record, &path, modified(source));
                    if !record["parent"].is_null() {
                        let parent = record["parent"]
                            .as_u64()
                            .and_then(|id| remap.get(&id))
                            .ok_or("Invalid legacy parent link.")?;
                        record["parent"] = (*parent).into();
                    }
                    if !record["captain"].is_null() {
                        let captain = record["captain"]
                            .as_u64()
                            .and_then(|id| captain_ids.get(id as usize))
                            .ok_or("Invalid legacy captain link.")?;
                        record["captain"] = (*captain).into();
                    }
                    imported.push(record);
                }
                // Keep all historical home/persona flags. Only this reference designates the
                // project's coordinator; other former homes become ordinary history in the UI.
                if let Some(home) = imported
                    .iter()
                    .filter(|t| {
                        !t["captain"].is_null()
                            && t["delegated"] != true
                            && project.captain.is_none()
                    })
                    .min_by_key(|t| (t["archived"] == true, t["captainHome"] != true))
                {
                    project.captain = home["captain"].as_u64().map(|id| id as usize);
                    if home["captainHome"] == true {
                        project.captain_thread = home["id"].as_u64().map(|id| id as usize);
                    }
                }
                threads.extend(imported);
                preferences = index;
            }
        }
        if !missing || threads.len() > first_thread {
            project.updated = threads[first_thread..]
                .iter()
                .filter_map(|t| t["updated"].as_u64())
                .max()
                .unwrap_or(0);
            projects.push(project.to_value());
        }
    }
    Ok(Some(serde_json::json!({
        "version": 3, "projects": projects, "threads": threads, "captains": captains,
        "dark": preferences["dark"], "zoom": preferences["zoom"],
        "cloneParent": recents.as_ref().map(|r| &r["cloneParent"]),
    })))
}

pub fn load(path: &std::path::Path) -> Result<Option<Value>, String> {
    if let Some(database) = crate::database::for_path(path) {
        return database.read(
            path.file_name()
                .expect("database document")
                .to_str()
                .expect("UTF-8"),
        );
    }
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e|
            format!("Could not read the saved thread index: {e}. The existing file has not been overwritten.")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not open the saved thread index: {e}.")),
    }
}

impl Store {
    pub fn new(path: PathBuf, wake: Waker) -> Self {
        if let Some(database) = crate::database::for_path(&path) {
            let (_, errors) = mpsc::channel();
            return Self {
                database: Some((
                    database,
                    path.file_name().expect("document").to_string_lossy().into(),
                )),
                database_error: None,
                sender: None,
                errors,
                last: String::new(),
                worker: None,
                healthy: Arc::new(AtomicBool::new(true)),
            };
        }
        let (sender, receiver) = mpsc::channel::<Message>();
        let (errors, error_receiver) = mpsc::channel();
        let healthy = Arc::new(AtomicBool::new(true));
        let status = healthy.clone();
        let worker = thread::spawn(move || {
            let mut last: Result<(), String> = Ok(());
            let mut latest: Option<String> = None;
            loop {
                let message = match receiver.recv_timeout(Duration::from_secs(1)) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) if last.is_err() => {
                        Message::Save(latest.clone().expect("failed snapshot"))
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let mut barrier = None;
                let mut snapshot = match message {
                    Message::Save(snapshot) => snapshot,
                    Message::Flush(ack) => {
                        if last.is_ok() {
                            let _ = ack.send(last.clone());
                            continue;
                        }
                        // Retry the latest failed snapshot even when the caller's state has
                        // not changed. A flush is not just a replay of the previous error.
                        barrier = Some(ack);
                        latest.clone().expect("failed snapshot")
                    }
                };
                // Coalesce newer writes, but never across a barrier.
                while barrier.is_none() {
                    let Ok(newer) = receiver.try_recv() else {
                        break;
                    };
                    match newer {
                        Message::Save(newer) => snapshot = newer,
                        Message::Flush(ack) => {
                            barrier = Some(ack);
                            break;
                        }
                    }
                }
                latest = Some(snapshot.clone());
                let result = write_atomic(&path, snapshot.as_bytes());
                last = match result {
                    Ok(()) => {
                        if !status.swap(true, Ordering::AcqRel) {
                            wake();
                        }
                        Ok(())
                    }
                    Err(error) => {
                        status.store(false, Ordering::Release);
                        let message = format!(
                            "Could not save the thread index: {error}. Pi still owns its session files."
                        );
                        if last.as_ref().err() != Some(&message) {
                            let _ = errors.send(message.clone());
                            wake();
                        }
                        Err(message)
                    }
                };
                if let Some(ack) = barrier {
                    let _ = ack.send(last.clone());
                }
            }
        });
        Self {
            database: None,
            database_error: None,
            sender: Some(sender),
            errors: error_receiver,
            last: String::new(),
            worker: Some(worker),
            healthy,
        }
    }

    pub fn save(&mut self, value: Value) {
        if let Some((database, name)) = &self.database {
            let serialized = value.to_string();
            if serialized == self.last {
                return;
            }
            if let Err(error) = database.write(name, value) {
                self.database_error = Some(error);
                self.healthy.store(false, Ordering::Release);
            } else {
                self.last = serialized;
            }
            return;
        }
        let serialized = value.to_string();
        if self.last != serialized
            && self
                .sender
                .as_ref()
                .is_some_and(|sender| sender.send(Message::Save(serialized.clone())).is_ok())
        {
            self.last = serialized;
        }
    }

    pub fn healthy(&self) -> bool {
        self.healthy.load(Ordering::Acquire)
    }

    /// Waits until every write queued so far is on disk, and reports the last write's result.
    pub fn flush(&self) -> Result<(), String> {
        if self.database.is_some() {
            return self.database_error.clone().map_or(Ok(()), Err);
        }
        let Some(sender) = &self.sender else {
            return Err("The store is no longer running.".into());
        };
        let (ack, result) = mpsc::channel();
        sender
            .send(Message::Flush(ack))
            .map_err(|_| "The store is no longer running.".to_string())?;
        result
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "The store did not finish flushing within five seconds.".to_string())?
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            crate::process::join_for(worker, Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };
    #[test]
    fn run_markers_distinguish_unclean_exits_and_clear_only_on_clean_drop() {
        let root = directory().unwrap().join("markers");
        fs::create_dir_all(&root).unwrap();
        let marker_path = root.join("run-marker.json");
        {
            let mut marker = RunMarker::start(&root).unwrap();
            assert!(!marker.previous_unclean);
            marker.refreshed -= Duration::from_secs(61);
            marker.tick().unwrap();
            assert!(marker.refreshed.elapsed() < Duration::from_secs(1));
            assert!(load(&marker_path).unwrap().unwrap()["refreshed_at"].is_u64());
        }
        assert!(marker_path.exists(), "unclean drop keeps evidence");
        {
            let mut marker = RunMarker::start(&root).unwrap();
            assert!(marker.previous_unclean);
            marker.clean = true;
            assert!(
                marker_path.exists(),
                "quit intent alone cannot clear the marker"
            );
            marker.tick().unwrap();
            assert!(
                !marker.clean,
                "a cancelled quit returns to an unclean running state"
            );
            marker.clean = true;
        }
        assert!(!marker_path.exists());
        let mut marker = RunMarker::start(&root).unwrap();
        assert!(!marker.previous_unclean);
        marker.clean = true;
    }

    #[test]
    fn default_test_storage_and_mcp_bridge_never_use_the_user_profile() {
        let base = directory().unwrap();
        assert!(base.starts_with(std::env::temp_dir()));
        if let Some(profile) = std::env::var_os("LOCALAPPDATA") {
            assert!(!crate::projects::same_path(
                &base,
                &PathBuf::from(profile).join("Shina")
            ));
        }
        let mut state = crate::model::State::new(PathBuf::new(), false);
        assert_eq!(state.data_dir, base);
        state.new_thread_in(None);
        let chat = state.working_directory();
        assert!(chat.starts_with(&base));
        fs::create_dir_all(&chat).unwrap();
        let extension = crate::mcp::extension_path().unwrap();
        assert!(extension.starts_with(&base));
        {
            let mut store = Store::new(base.join("index.json"), Arc::new(|| {}));
            store.save(serde_json::json!({"test":true}));
        }
        assert!(base.join("index.json").is_file());
        let other = std::thread::spawn(|| directory().unwrap()).join().unwrap();
        assert_ne!(base, other);
        assert!(
            !other.exists(),
            "the test thread removes its own temporary profile"
        );
    }

    #[test]
    fn migration_skips_missing_junk_but_keeps_non_temp_history() {
        let root = directory().unwrap().join("migration");
        let data = root.join("data");
        let temporary = root.join("system-temp");
        let existing = root.join("existing");
        let history = root.join("offline-project");
        let missing = root.join("missing-without-index");
        let empty = root.join("missing-empty-index");
        let junk = temporary.join("shina-profile-stale");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&existing).unwrap();
        let recents = serde_json::json!({"projects":[
            {"path":junk},{"path":missing},{"path":empty},{"path":existing},{"path":history}
        ]});
        fs::write(data.join("projects.json"), recents.to_string()).unwrap();
        let threads = serde_json::json!({"version":1,"threads":[{"id":1,"title":"Retained history","session":"saved.jsonl"}]});
        fs::write(legacy_path(&data, &history), threads.to_string()).unwrap();
        fs::write(legacy_path(&data, &junk), threads.to_string()).unwrap();
        fs::write(
            legacy_path(&data, &empty),
            serde_json::json!({"version":1,"threads":[]}).to_string(),
        )
        .unwrap();
        fs::write(
            data.join("last-workspace.txt"),
            junk.to_string_lossy().as_bytes(),
        )
        .unwrap();
        let imported = migrate_legacy(&data, None, &temporary).unwrap().unwrap();
        let projects = imported["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0]["name"], "existing");
        assert_eq!(projects[1]["name"], "offline-project");
        assert_eq!(imported["threads"].as_array().unwrap().len(), 1);
        assert_eq!(imported["threads"][0]["project"], projects[1]["id"]);
        assert!(
            legacy_path(&data, &junk).is_file(),
            "ignored indexes are never deleted"
        );
        assert_eq!(load(&data.join("projects.json")).unwrap().unwrap(), recents);
    }

    #[test]
    fn legacy_activity_uses_session_mtime_then_index_mtime() {
        let base = directory().unwrap();
        let workspace = base.join("project");
        fs::create_dir(&workspace).unwrap();
        let session = workspace.join("session.jsonl");
        let session_time = 1_700_000_000;
        let index_time = session_time + 600;
        fs::File::create(&session)
            .unwrap()
            .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(session_time))
            .unwrap();
        let legacy = legacy_path(&base, &workspace);
        fs::write(
            &legacy,
            serde_json::json!({"version":1,"threads":[
                {"id":1,"title":"Session timestamp","session":"session.jsonl"},
                {"id":2,"title":"Index timestamp","session":"missing.jsonl","updated":0},
                {"id":3,"title":"Saved timestamp","updated":1700000100}
            ]})
            .to_string(),
        )
        .unwrap();
        fs::File::options()
            .write(true)
            .open(&legacy)
            .unwrap()
            .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(index_time))
            .unwrap();
        let imported = load_global(&base, Some(&workspace)).unwrap().unwrap();
        assert_eq!(imported["threads"][0]["updated"], session_time);
        assert_eq!(imported["threads"][1]["updated"], index_time);
        assert_eq!(imported["threads"][2]["updated"], 1_700_000_100u64);
        let mut unknown = serde_json::json!({"session":"unavailable"});
        restore_activity(&mut unknown, &workspace, 0);
        assert_eq!(unknown["updated"], 0);
    }

    #[test]
    #[cfg(windows)]
    fn verbatim_and_plain_legacy_paths_import_as_one_clean_project() {
        let base = directory().unwrap();
        let workspace = base.join("project");
        fs::create_dir(&workspace).unwrap();
        let verbatim = workspace.canonicalize().unwrap();
        let clean = crate::projects::normalize_path(&workspace);
        fs::write(
            base.join("projects.json"),
            serde_json::json!({"projects":[
                {"path":verbatim},{"path":clean}
            ]})
            .to_string(),
        )
        .unwrap();
        fs::write(
            legacy_path(&base, &verbatim),
            serde_json::json!({"version":1,"threads":[
                {"id":1,"title":"Existing history","session":"preserved-session"}
            ]})
            .to_string(),
        )
        .unwrap();
        let imported = load_global(&base, Some(&clean)).unwrap().unwrap();
        assert_eq!(imported["projects"].as_array().unwrap().len(), 1);
        assert_eq!(
            imported["projects"][0]["path"],
            clean.to_string_lossy().as_ref()
        );
        assert_eq!(imported["threads"].as_array().unwrap().len(), 1);
        assert_eq!(imported["threads"][0]["session"], "preserved-session");
    }

    #[test]
    fn damaged_legacy_or_global_indexes_never_start_a_partial_migration() {
        let directory = std::env::temp_dir().join(format!(
            "shina-bad-index-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let project = directory.join("missing-project");
        fs::create_dir(&project).unwrap();
        fs::write(
            directory.join("projects.json"),
            serde_json::json!({"projects":[{"path":project}]}).to_string(),
        )
        .unwrap();
        let legacy = legacy_path(&directory, &project);
        fs::write(&legacy, "{broken").unwrap();
        assert!(load_global(&directory, None).is_err());
        assert!(!directory.join("index.json").exists());
        assert_eq!(fs::read_to_string(&legacy).unwrap(), "{broken");
        fs::write(directory.join("index.json"), "{global-broken").unwrap();
        assert!(load_global(&directory, None).is_err());
        assert_eq!(
            fs::read_to_string(directory.join("index.json")).unwrap(),
            "{global-broken"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn version_one_legacy_index_and_empty_projects_migrate_once() {
        let directory = std::env::temp_dir().join(format!(
            "shina-v1-index-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let a = directory.join("old");
        let b = directory.join("empty");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(
            directory.join("projects.json"),
            serde_json::json!({"projects":[{"path":a},{"path":b}]}).to_string(),
        )
        .unwrap();
        let legacy = legacy_path(&directory, &a);
        fs::write(&legacy, serde_json::json!({"version":1,"threads":[{"id":1,"title":"Legacy","session":"keep.jsonl","captain":2,"captainHome":true}]}).to_string()).unwrap();
        let global = load_global(&directory, None).unwrap().unwrap();
        assert_eq!(global["projects"].as_array().unwrap().len(), 2);
        assert_eq!(global["threads"][0]["session"], "keep.jsonl");
        assert_eq!(global["threads"][0]["captain"], 2);
        fs::write(directory.join("index.json"), global.to_string()).unwrap();
        fs::write(&legacy, "{bad later").unwrap();
        assert_eq!(load_global(&directory, None).unwrap().unwrap(), global);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn flush_waits_for_queued_writes_reports_failure_and_is_never_coalesced() {
        let directory = directory().unwrap().join("flush");
        let file = directory.join("index.json");
        let mut store = Store::new(file.clone(), Arc::new(|| {}));
        store.save(serde_json::json!({"value":"first"}));
        store.flush().unwrap();
        assert_eq!(load(&file).unwrap().unwrap()["value"], "first");
        store.save(serde_json::json!({"value":"second"}));
        store.save(serde_json::json!({"value":"third"}));
        store.flush().unwrap();
        assert_eq!(load(&file).unwrap().unwrap()["value"], "third");
        // A save after a barrier is written for the next barrier, not lost with it.
        store.save(serde_json::json!({"value":"fourth"}));
        store.flush().unwrap();
        assert_eq!(load(&file).unwrap().unwrap()["value"], "fourth");
        // A blocked target turns the barrier into the write's failure.
        let blocked = directory.join("blocked");
        fs::create_dir_all(blocked.join("index.json")).unwrap();
        let mut broken = Store::new(blocked.join("index.json"), Arc::new(|| {}));
        broken.save(serde_json::json!({"value":"lost"}));
        assert!(broken.flush().is_err());
        assert!(
            broken.errors.try_recv().is_ok(),
            "the failure is also surfaced"
        );
        drop(broken);
        drop(store);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn atomically_replaces_and_flushes_the_index_on_shutdown() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("shina-store-{}-{unique}", std::process::id()));
        let file = directory.join("index.json");
        {
            let mut store = Store::new(file.clone(), Arc::new(|| {}));
            store.save(serde_json::json!({"version":1,"value":"first"}));
        }
        {
            let mut store = Store::new(file.clone(), Arc::new(|| {}));
            store.save(serde_json::json!({"version":1,"value":"second"}));
        }
        assert_eq!(load(&file).unwrap().unwrap()["value"], "second");
        fs::remove_file(&file).unwrap();
        fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn malformed_thread_entries_and_activity_types_fail_without_panicking() {
        let root = directory().unwrap();
        for record in [
            serde_json::json!(1),
            serde_json::json!("x"),
            serde_json::json!(null),
            serde_json::json!({"id":1,"updated":"wrong"}),
            serde_json::json!({"id":1,"session":123}),
        ] {
            fs::write(
                root.join("index.json"),
                serde_json::json!({"version":4,"projects":[],"threads":[record]}).to_string(),
            )
            .unwrap();
            assert!(load_global(&root, None).is_err());
        }
    }

    #[test]
    fn flush_retries_an_unchanged_snapshot_after_a_transient_failure() {
        let root = directory().unwrap().join("retry");
        let file = root.join("index.json");
        fs::create_dir_all(&file).unwrap();
        let mut store = Store::new(file.clone(), Arc::new(|| {}));
        let snapshot = serde_json::json!({"draft":"must survive"});
        store.save(snapshot.clone());
        assert!(store.flush().is_err());
        assert!(!store.healthy());
        fs::remove_dir(&file).unwrap();
        store.save(snapshot.clone()); // Deduplicated at the caller, but flush must retry.
        store.flush().unwrap();
        assert!(store.healthy());
        assert_eq!(load(&file).unwrap().unwrap(), snapshot);
    }

    #[test]
    fn a_failed_snapshot_retries_while_idle_and_wakes_after_recovery() {
        let file = directory().unwrap().join("idle-retry/index.json");
        fs::create_dir_all(&file).unwrap();
        let (wake, events) = mpsc::channel();
        let mut store = Store::new(
            file.clone(),
            Arc::new(move || {
                let _ = wake.send(());
            }),
        );
        let snapshot = serde_json::json!({"draft":"idle"});
        store.save(snapshot.clone());
        assert!(store.flush().is_err());
        events.try_iter().for_each(drop);
        fs::remove_dir(&file).unwrap();
        events.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(store.healthy());
        assert_eq!(load(&file).unwrap().unwrap(), snapshot);
    }
}
