//! Bounded, off-thread project inventories for the caret menu.
use rune::Waker;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const TTL: Duration = Duration::from_secs(30);
const MAX_PROJECTS: usize = 8;
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Debug)]
pub struct PathEntry {
    pub path: PathBuf,
    pub relative: String,
    pub directory: bool,
}

#[derive(Clone, Default)]
pub struct Inventory {
    pub entries: Arc<Vec<PathEntry>>,
    pub loading: bool,
}

struct Cached {
    inventory: Inventory,
    refreshed: Instant,
}

#[derive(Default)]
pub struct Sources {
    cache: Arc<Mutex<HashMap<PathBuf, Cached>>>,
    changed: Arc<AtomicBool>,
}

impl Sources {
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }

    pub fn paths(&self, root: &Path, wake: Waker) -> Inventory {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(root)
            && (entry.inventory.loading || entry.refreshed.elapsed() < TTL)
        {
            return entry.inventory.clone();
        }
        if cache.len() >= MAX_PROJECTS && !cache.contains_key(root) {
            let oldest = cache
                .iter()
                .filter(|(_, e)| !e.inventory.loading)
                .min_by_key(|(_, e)| e.refreshed)
                .map(|(p, _)| p.clone());
            if let Some(oldest) = oldest {
                cache.remove(&oldest);
            } else {
                return Inventory::default();
            }
        }
        let previous = cache
            .get(root)
            .map(|e| e.inventory.entries.clone())
            .unwrap_or_default();
        let inventory = Inventory {
            entries: previous,
            loading: true,
        };
        cache.insert(
            root.to_owned(),
            Cached {
                inventory: inventory.clone(),
                refreshed: Instant::now(),
            },
        );
        let root = root.to_owned();
        let shared = self.cache.clone();
        let changed = self.changed.clone();
        std::thread::spawn(move || {
            let entries = scan(&root);
            let mut cache = shared.lock().unwrap_or_else(|e| e.into_inner());
            cache.insert(
                root,
                Cached {
                    inventory: Inventory {
                        entries: Arc::new(entries),
                        loading: false,
                    },
                    refreshed: Instant::now(),
                },
            );
            changed.store(true, Ordering::Release);
            drop(cache);
            wake();
        });
        inventory
    }
}

pub fn scan(root: &Path) -> Vec<PathEntry> {
    let start = Instant::now();
    let mut entries = Vec::new();
    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .max_depth(Some(20))
        .require_git(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    for entry in walk.take(MAX_ENTRIES + 1) {
        if start.elapsed() > Duration::from_secs(2) {
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        let Some(kind) = entry.file_type() else {
            continue;
        };
        // Do not traverse or offer a symlink that could escape the project.
        if !kind.is_file() && !kind.is_dir() {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        if relative.len() > 1024 {
            continue;
        }
        entries.push(PathEntry {
            path: path.to_owned(),
            relative,
            directory: kind.is_dir(),
        });
    }
    entries.sort_by(|a, b| a.relative.cmp(&b.relative));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_respects_gitignore_including_nested_rules_and_folders() {
        let root = crate::store::directory().unwrap().join("mention-scan");
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::create_dir_all(root.join("ignored")).unwrap();
        std::fs::write(root.join(".gitignore"), "ignored/\n*.secret\n").unwrap();
        std::fs::write(root.join("src/.gitignore"), "hidden.txt\n").unwrap();
        for path in [
            "src/main.rs",
            "src/hidden.txt",
            "src/nested/a.secret",
            "ignored/a.rs",
        ] {
            std::fs::write(root.join(path), "test").unwrap();
        }
        let entries = scan(&root);
        assert!(entries.iter().any(|e| e.relative == "src/main.rs"));
        assert!(
            entries
                .iter()
                .any(|e| e.relative == "src/nested" && e.directory)
        );
        assert!(!entries.iter().any(|e| e.relative.ends_with("hidden.txt")
            || e.relative.contains("ignored")
            || e.relative.ends_with(".secret")));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.parent().unwrap(), root.join("escape")).unwrap();
            assert!(!scan(&root).iter().any(|e| e.relative.starts_with("escape")));
        }
    }

    #[test]
    fn cache_is_per_project_and_refreshes_stale_inventories_off_thread() {
        let root = crate::store::directory().unwrap().join("mention-cache");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("first.rs"), "test").unwrap();
        let sources = Sources::default();
        let (sent, received) = std::sync::mpsc::channel();
        let wake: Waker = Arc::new(move || {
            let _ = sent.send(());
        });
        assert!(sources.paths(&root, wake.clone()).loading);
        received.recv_timeout(Duration::from_secs(10)).unwrap();
        let loaded = sources.paths(&root, wake.clone());
        assert!(!loaded.loading);
        assert_eq!(loaded.entries.len(), 1);
        assert!(sources.take_changed());
        assert!(!sources.take_changed());
        std::fs::write(root.join("second.rs"), "test").unwrap();
        assert_eq!(sources.paths(&root, wake.clone()).entries.len(), 1);
        sources
            .cache
            .lock()
            .unwrap()
            .get_mut(&root)
            .unwrap()
            .refreshed = Instant::now() - TTL - Duration::from_secs(1);
        assert!(sources.paths(&root, wake.clone()).loading);
        received.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(sources.paths(&root, wake.clone()).entries.len(), 2);
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        assert!(sources.paths(&other, wake.clone()).loading);
        received.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(sources.paths(&other, wake).entries.is_empty());
    }
}
