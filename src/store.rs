//! One app-wide index. Legacy workspace files are read, never changed or deleted.
use rune::Waker;
use serde_json::Value;
use std::{fs, path::PathBuf, sync::mpsc, thread};

pub struct Store {
    sender: Option<mpsc::Sender<String>>,
    pub errors: mpsc::Receiver<String>,
    last: String,
    worker: Option<thread::JoinHandle<()>>,
}

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
    if let Some(index) = load(&directory.join("index.json"))? {
        return Ok(Some(index));
    }
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
        if let Some((_, aliases)) = unique.iter_mut().find(|(p, _)| *p == canonical) {
            if !aliases.contains(&path) {
                aliases.push(path);
            }
        } else {
            let mut aliases = vec![path];
            if !aliases.contains(&canonical) {
                aliases.push(canonical.clone());
            }
            unique.push((canonical, aliases));
        }
    }
    if unique.is_empty() {
        return Ok(None);
    }
    let mut projects = Vec::new();
    let mut threads = Vec::new();
    let mut captains: Vec<Value> = crate::captains::Captain::defaults()
        .iter()
        .map(crate::captains::Captain::to_value)
        .collect();
    let mut used = std::collections::HashSet::new();
    let mut preferences = serde_json::json!({});
    for (i, (path, aliases)) in unique.into_iter().enumerate() {
        let project_id = i + 1;
        let mut project = crate::projects::Project::new(project_id, path.clone());
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
                    crate::captains::Captain::defaults()
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
        projects.push(project.to_value());
    }
    Ok(Some(serde_json::json!({
        "version": 3, "projects": projects, "threads": threads, "captains": captains,
        "dark": preferences["dark"], "zoom": preferences["zoom"],
        "cloneParent": recents.as_ref().map(|r| &r["cloneParent"]),
    })))
}

pub fn load(path: &std::path::Path) -> Result<Option<Value>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e|
            format!("Could not read the saved thread index: {e}. The existing file has not been overwritten.")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not open the saved thread index: {e}.")),
    }
}

impl Store {
    pub fn new(path: PathBuf, wake: Waker) -> Self {
        let (sender, receiver) = mpsc::channel::<String>();
        let (errors, error_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            while let Ok(mut snapshot) = receiver.recv() {
                while let Ok(newer) = receiver.try_recv() {
                    snapshot = newer;
                }
                let result = (|| -> std::io::Result<()> {
                    fs::create_dir_all(path.parent().expect("index directory"))?;
                    let temporary = path.with_extension("json.tmp");
                    use std::io::Write;
                    let mut staged = fs::File::create(&temporary)?;
                    staged.write_all(snapshot.as_bytes())?;
                    staged.sync_all()?;
                    drop(staged);
                    fs::rename(temporary, &path)?;
                    Ok(())
                })();
                if let Err(error) = result {
                    let _ = errors.send(format!(
                        "Could not save the thread index: {error}. Pi still owns its session files."
                    ));
                    wake();
                }
            }
        });
        Self {
            sender: Some(sender),
            errors: error_receiver,
            last: String::new(),
            worker: Some(worker),
        }
    }

    pub fn save(&mut self, value: Value) {
        let serialized = value.to_string();
        if self.last != serialized
            && self
                .sender
                .as_ref()
                .is_some_and(|sender| sender.send(serialized.clone()).is_ok())
        {
            self.last = serialized;
        }
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
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
}
