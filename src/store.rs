//! Small workspace index only. Pi owns its session transcripts and credentials.
use rune::Waker;
use serde_json::Value;
use std::{fs, path::PathBuf, sync::mpsc, thread};

pub struct Store {
    sender: Option<mpsc::Sender<String>>,
    pub errors: mpsc::Receiver<String>,
    last: String,
    worker: Option<thread::JoinHandle<()>>,
}

pub fn path(workspace: &std::path::Path) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }?;
    // A stable, non-cryptographic filename. This is not a security boundary.
    let hash = workspace
        .to_string_lossy()
        .bytes()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ byte as u64).wrapping_mul(0x100000001b3)
        });
    Some(
        base.join("Still")
            .join(format!("workspace-{hash:016x}.json")),
    )
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
                    fs::write(&temporary, snapshot)?;
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
    fn atomically_replaces_and_flushes_the_index_on_shutdown() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("still-store-{}-{unique}", std::process::id()));
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
