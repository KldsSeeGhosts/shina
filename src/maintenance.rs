//! Leased restart fence. Preparation never stops a run or closes a client.
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};

pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Drain {
    id: String,
    deadline: Instant,
    clients: BTreeSet<u32>,
    ready: BTreeSet<u32>,
    committed: bool,
}

#[derive(Default)]
pub struct Maintenance {
    drain: Option<Drain>,
}

impl Maintenance {
    pub fn fenced(&self) -> bool {
        self.drain.is_some()
    }
    pub fn status(&self) -> Value {
        self.drain.as_ref().map_or(json!({"phase":"ready"}), |d| json!({
            "phase":if d.committed {"restarting"} else {"draining"},
            "updateId":d.id, "clientPids":d.clients, "clientsReady":d.clients.is_subset(&d.ready)
        }))
    }
    pub fn prepare(
        &mut self,
        blocker: Option<&str>,
        clients: BTreeSet<u32>,
        now: Instant,
    ) -> Result<Value, String> {
        if self.fenced() {
            return Err("Another restart is already prepared.".into());
        }
        if let Some(reason) = blocker {
            return Err(reason.into());
        }
        self.drain = Some(Drain {
            id: uuid::Uuid::new_v4().to_string(),
            deadline: now + DRAIN_TIMEOUT,
            clients,
            ready: BTreeSet::new(),
            committed: false,
        });
        Ok(self.status())
    }
    fn matching(&mut self, params: &Value) -> Result<&mut Drain, String> {
        self.drain
            .as_mut()
            .filter(|d| params["updateId"] == d.id)
            .ok_or("No matching prepared update.".into())
    }
    pub fn cancel(&mut self, params: &Value) -> Result<Value, String> {
        if self.matching(params)?.committed {
            return Err("Restart already committed.".into());
        }
        self.drain = None;
        Ok(self.status())
    }
    pub fn client_ready(&mut self, params: &Value) -> Result<Value, String> {
        let drain = self.matching(params)?;
        let pid = params["pid"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("Client PID required.")?;
        if drain.clients.contains(&pid) {
            drain.ready.insert(pid);
        }
        Ok(self.status())
    }
    pub fn commit(&mut self, params: &Value, now: Instant) -> Result<Value, String> {
        let drain = self.matching(params)?;
        if drain.committed {
            return Ok(self.status());
        }
        if now >= drain.deadline {
            self.drain = None;
            return Err("Drain timed out. Normal operation resumed.".into());
        }
        if !drain.clients.is_subset(&drain.ready) {
            return Err("Waiting for local clients to save drafts.".into());
        }
        drain.committed = true;
        // Give the response and server.restarting notification time to reach clients.
        drain.deadline = now + Duration::from_secs(1);
        Ok(self.status())
    }
    /// Returns true only after an explicit commit. An expired preparation resumes normally.
    pub fn advance(&mut self, now: Instant) -> bool {
        if let Some(drain) = &self.drain
            && now >= drain.deadline
        {
            if drain.committed {
                return true;
            }
            self.drain = None;
        }
        false
    }
}

pub fn negotiate(major: u32, minor: u32, schema: u32) -> Result<u32, String> {
    if major != crate::protocol::MAJOR || schema != 1 {
        return Err(format!(
            "Protocol major or projection schema mismatch. Upgrade both Shina clients and the host with `shina update --yes`. Client major {major}, server major {}.",
            crate::protocol::MAJOR
        ));
    }
    Ok(minor.min(crate::protocol::MINOR))
}

/// One authenticated maintenance request. It never spawns a server or changes authentication.
pub fn rpc(directory: &Path, method: &str, args: Value) -> Result<Value, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(crate::client::local_rpc(directory, method, args))
}

pub fn wait_unowned(directory: &Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(owner) = crate::server_lock::Owner::acquire(directory) {
            drop(owner);
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Server did not exit. No files were replaced.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Run under the server owner lock, before opening a newer schema. SQLite's own
/// VACUUM INTO includes committed WAL pages. Retain every backup for binary rollback.
pub fn backup_state(directory: &Path) -> Result<std::path::PathBuf, String> {
    let backup = directory
        .join("backups")
        .join(format!("before-update-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "server-owner.sqlite" || name == "server.json" {
            continue;
        }
        let target = backup.join(name.as_ref());
        if path
            .extension()
            .is_some_and(|ext| ext == "sqlite" || ext == "sqlite3" || ext == "db")
        {
            let connection = rusqlite::Connection::open_with_flags(
                &path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(|e| e.to_string())?;
            connection
                .execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
                .map_err(|e| e.to_string())?;
            let saved = rusqlite::Connection::open_with_flags(
                &target,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(|e| e.to_string())?;
            let integrity: String = saved
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if integrity != "ok" {
                return Err("SQLite backup verification failed. Update cancelled.".into());
            }
        } else if path.extension().is_some_and(|ext| ext == "json") && name != "service.json" {
            std::fs::copy(&path, &target).map_err(|e| e.to_string())?;
        }
        if target.is_file() {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&target)
                .and_then(|file| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
    }
    crate::install::sync_directory(&backup);
    Ok(backup)
}

/// Caller holds the owner lock and has stopped the failed binary. Preserve the failed
/// schema separately, including databases that did not exist before migration.
pub fn restore_backup(directory: &Path, backup: &Path) -> Result<(), String> {
    let backup_root = directory
        .join("backups")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let backup = backup.canonicalize().map_err(|e| e.to_string())?;
    if !backup.starts_with(&backup_root) || backup == backup_root {
        return Err("Rollback backup is outside this profile's backup directory.".into());
    }
    let failed = backup.join(format!("failed-upgrade-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&failed).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if text.starts_with("server-owner.sqlite")
            || matches!(text.as_ref(), "server.json" | "service.json")
        {
            continue;
        }
        let base = text
            .strip_suffix("-wal")
            .or_else(|| text.strip_suffix("-shm"))
            .unwrap_or(&text);
        if Path::new(base)
            .extension()
            .is_some_and(|ext| ext == "sqlite" || ext == "sqlite3" || ext == "db" || ext == "json")
        {
            std::fs::rename(entry.path(), failed.join(name)).map_err(|e| e.to_string())?;
        }
    }
    for entry in std::fs::read_dir(&backup).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_file() {
            crate::store::write_atomic(
                &directory.join(entry.file_name()),
                &std::fs::read(entry.path()).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    crate::install::sync_directory(directory);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn major_refuses_minor_negotiates() {
        assert!(negotiate(99, 0, 1).unwrap_err().contains("shina update"));
        assert!(negotiate(crate::protocol::MAJOR, 0, 2).is_err());
        assert_eq!(
            negotiate(crate::protocol::MAJOR, 999, 1).unwrap(),
            crate::protocol::MINOR
        );
        assert_eq!(negotiate(crate::protocol::MAJOR, 0, 1).unwrap(), 0);
    }
    #[test]
    fn refused_cancelled_and_expired_drains_never_stop_work() {
        let now = Instant::now();
        let mut m = Maintenance::default();
        for reason in [
            "active",
            "configuring",
            "queued",
            "approval",
            "clone",
            "routine",
        ] {
            assert_eq!(
                m.prepare(Some(reason), BTreeSet::new(), now).unwrap_err(),
                reason
            );
            assert!(!m.fenced());
        }
        let status = m.prepare(None, [42].into(), now).unwrap();
        assert!(m.commit(&status, now).is_err());
        assert!(!m.advance(now + DRAIN_TIMEOUT));
        assert!(!m.fenced());
        let status = m.prepare(None, [42].into(), now).unwrap();
        m.cancel(&status).unwrap();
        assert!(!m.fenced());
        let mut status = m.prepare(None, [42].into(), now).unwrap();
        status["pid"] = json!(42);
        m.client_ready(&status).unwrap();
        m.commit(&status, now).unwrap();
        assert!(!m.advance(now));
        assert!(m.advance(now + Duration::from_secs(1)));
    }
    #[test]
    fn backup_includes_committed_sqlite_and_keeps_source() {
        let directory = crate::store::directory().unwrap();
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("state.sqlite");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE example (value TEXT); INSERT INTO example VALUES ('kept');").unwrap();
        let backup = backup_state(&directory).unwrap();
        let saved = rusqlite::Connection::open(backup.join("state.sqlite")).unwrap();
        assert_eq!(
            saved
                .query_row("SELECT value FROM example", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "kept"
        );
        assert!(path.exists());
    }
    #[test]
    fn update_backup_includes_open_durable_database_and_preserves_source_documents() {
        let directory = crate::store::directory().unwrap();
        let database = crate::database::Database::open(&directory).unwrap();
        database
            .write("memory.json", json!({"committed":"in WAL"}))
            .unwrap();
        database.issue_device("fixture", "protected-token").unwrap();
        let backup = backup_state(&directory).unwrap();
        let saved = rusqlite::Connection::open(backup.join("state.sqlite")).unwrap();
        let value: String = saved
            .query_row(
                "SELECT payload FROM documents WHERE name='memory.json'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&value).unwrap(),
            json!({"committed":"in WAL"})
        );
        assert_eq!(
            saved
                .query_row("SELECT count(*) FROM devices", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(database.device_active("fixture").unwrap());
    }
    #[test]
    fn damaged_database_backup_cancels_without_returning_a_rollback_path() {
        let directory = crate::store::directory().unwrap();
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("state.sqlite"), b"not sqlite").unwrap();
        assert!(backup_state(&directory).is_err());
    }
    #[test]
    fn rollback_restores_the_matched_schema_and_quarantines_new_databases() {
        let directory = crate::store::directory().unwrap();
        std::fs::create_dir_all(&directory).unwrap();
        let original = directory.join("original.sqlite");
        let db = rusqlite::Connection::open(&original).unwrap();
        db.execute_batch("CREATE TABLE original (value TEXT);")
            .unwrap();
        drop(db);
        let backup = backup_state(&directory).unwrap();
        let db = rusqlite::Connection::open(&original).unwrap();
        db.execute_batch("DROP TABLE original; CREATE TABLE newer (value TEXT);")
            .unwrap();
        drop(db);
        std::fs::write(directory.join("new-schema.sqlite"), b"failed migration").unwrap();
        restore_backup(&directory, &backup).unwrap();
        let db = rusqlite::Connection::open(&original).unwrap();
        db.prepare("SELECT * FROM original").unwrap();
        assert!(!directory.join("new-schema.sqlite").exists());
        assert!(backup.join("original.sqlite").exists());
        assert!(
            std::fs::read_dir(&backup)
                .unwrap()
                .flatten()
                .any(|entry| entry.path().join("new-schema.sqlite").exists())
        );
    }
}
