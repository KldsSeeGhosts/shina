//! KSG serverOwnership.ts: a never-unlinked SQLite file, held before any writable state.
use std::path::Path;

pub struct Owner {
    _connection: rusqlite::Connection,
}

impl Owner {
    pub fn acquire(directory: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let connection = rusqlite::Connection::open(directory.join("server-owner.sqlite"))
            .map_err(|e| e.to_string())?;
        // Allow the winner of simultaneous first opens to finish its lock upgrade.
        // A live owner still produces a bounded refusal, never a stale-file takeover.
        connection
            .busy_timeout(std::time::Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        connection.execute_batch("BEGIN EXCLUSIVE;").map_err(|_| {
            "Another Shina runtime owns this profile. Use --server-client to attach, or stop its owner. Do not delete server-owner.sqlite.".to_owned()
        })?;
        Ok(Self {
            _connection: connection,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_is_exclusive_and_released_without_unlinking() {
        let dir = crate::store::directory().unwrap();
        let owner = Owner::acquire(&dir).unwrap();
        assert!(Owner::acquire(&dir).is_err());
        drop(owner);
        assert!(dir.join("server-owner.sqlite").exists());
        assert!(Owner::acquire(&dir).is_ok());
    }

    #[test]
    fn simultaneous_owners_have_exactly_one_winner() {
        let directory = crate::store::directory().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let release = std::sync::Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let directory = directory.clone();
                let barrier = barrier.clone();
                let release = release.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let owner = Owner::acquire(&directory);
                    release.wait();
                    owner.is_ok()
                })
            })
            .collect();
        barrier.wait();
        release.wait();
        assert_eq!(
            workers
                .into_iter()
                .filter_map(|worker| worker.join().ok())
                .filter(|won| *won)
                .count(),
            1
        );
    }
}
