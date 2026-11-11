//! Server-only SQLite ownership. Legacy files remain an immutable import source.
use crate::{outbox::Effect, protocol::Receipt};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, Weak},
};

const FILES: [&str; 6] = [
    "index.json",
    "server-inputs.json",
    "ui-state.json",
    "memory.json",
    "routines.json",
    "honcho-outbox.json",
];
type Registry = HashMap<PathBuf, Weak<Mutex<Inner>>>;
static OPEN: LazyLock<Mutex<Registry>> = LazyLock::new(|| Mutex::new(HashMap::new()));

struct Inner {
    connection: Connection,
    batch: Option<std::thread::ThreadId>,
    staged: BTreeMap<String, Value>,
    revocations: Vec<String>,
    failure: Option<String>,
    dispatch_fenced: bool,
    claimed_dispatches: usize,
}

#[derive(Clone)]
pub struct Database(Arc<Mutex<Inner>>);

/// A claimed external effect must settle before maintenance can prepare a restart.
pub struct Dispatch(Database);
impl Drop for Dispatch {
    fn drop(&mut self) {
        self.0.0.lock().expect("database").claimed_dispatches -= 1;
    }
}

fn error(error: impl std::fmt::Display) -> String {
    format!("Shina database is unavailable. Writes are blocked; no JSON fallback was used. {error}")
}

pub fn for_path(path: &Path) -> Option<Database> {
    let name = path.file_name()?.to_str()?;
    if !FILES.contains(&name) {
        return None;
    }
    OPEN.lock()
        .expect("database registry")
        .get(path.parent()?)
        .and_then(Weak::upgrade)
        .map(Database)
}

impl Database {
    pub fn claim_dispatch(&self) -> Result<Option<Dispatch>, String> {
        let mut inner = self.0.lock().expect("database");
        if let Some(failure) = &inner.failure {
            return Err(failure.clone());
        }
        if inner.dispatch_fenced {
            return Ok(None);
        }
        inner.claimed_dispatches += 1;
        Ok(Some(Dispatch(self.clone())))
    }

    /// Shares the runtime's maintenance/validation fence with asynchronous workers.
    /// The lock makes fencing and claiming mutually exclusive.
    pub fn fence_dispatch(&self, fenced: bool) -> usize {
        let mut inner = self.0.lock().expect("database");
        inner.dispatch_fenced = fenced;
        inner.claimed_dispatches
    }

    pub fn open(directory: &Path) -> Result<Self, String> {
        if for_path(&directory.join("index.json")).is_some() {
            return Err(error("This profile already has an open database owner."));
        }
        std::fs::create_dir_all(directory).map_err(error)?;
        let mut connection = Connection::open(directory.join("state.sqlite")).map_err(error)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(error)?;
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(error)?;
        if integrity != "ok" {
            return Err(error(integrity));
        }
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(error)?;
        if version > 1 {
            return Err(error("This schema requires a newer Shina binary."));
        }
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
            )
            .map_err(error)?;
        if version == 0 {
            // VACUUM INTO includes committed WAL pages and never overwrites an older backup.
            let count: i64 = connection
                .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
                .map_err(error)?;
            if count != 0 {
                return Err(error(
                    "Unversioned, nonempty database. Restore a matched backup.",
                ));
            }
            // Also runs when the binary version is unchanged, before schema/import writes.
            crate::maintenance::backup_state(directory)?;
            let tx = connection.transaction().map_err(error)?;
            tx.execute_batch(include_str!("migrations/001.sql"))
                .map_err(error)?;
            tx.commit().map_err(error)?;
        }
        let imported: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='legacy-import-v1')",
                [],
                |row| row.get(0),
            )
            .map_err(error)?;
        if !imported {
            Self::import(&mut connection, directory)?;
        }
        // Process-bound effects, including committed but undispatched prompts, cannot
        // be retried safely. Honcho keeps its reconciled, stable-ID ledger separately.
        {
            let tx = connection.transaction().map_err(error)?;
            let receipts: Vec<String> = {
                let mut stmt = tx
                    .prepare(
                        "SELECT DISTINCT r.payload FROM command_receipts r JOIN effect_outbox e
                     ON e.command_id=r.command_id AND e.device=r.device
                     WHERE e.kind='harness' AND e.status IN ('pending','dispatching')",
                    )
                    .map_err(error)?;
                stmt.query_map([], |row| row.get(0))
                    .map_err(error)?
                    .collect::<Result<_, _>>()
                    .map_err(error)?
            };
            for payload in receipts {
                let mut receipt: Receipt = serde_json::from_str(&payload).map_err(error)?;
                receipt.status = crate::protocol::ReceiptStatus::Interrupted;
                receipt.result = json!({"error":"Server restarted before dispatch was confirmed. Review before retrying.","previousResult":receipt.result});
                put_receipt(&tx, &receipt)?;
            }
            tx.execute_batch(
                "UPDATE effect_outbox SET status='interrupted', error='Server restarted. Review before retrying.'
                 WHERE kind='harness' AND status IN ('pending','dispatching');",
            ).map_err(error)?;
            tx.commit().map_err(error)?;
        }
        let database = Self(Arc::new(Mutex::new(Inner {
            connection,
            batch: None,
            staged: BTreeMap::new(),
            revocations: Vec::new(),
            failure: None,
            dispatch_fenced: false,
            claimed_dispatches: 0,
        })));
        if !imported {
            let backup = directory.join("state.sqlite.import-v1.bak");
            if !backup.exists() {
                database.backup(&backup)?;
            }
        }
        OPEN.lock()
            .expect("database registry")
            .insert(directory.to_path_buf(), Arc::downgrade(&database.0));
        Ok(database)
    }

    fn import(connection: &mut Connection, directory: &Path) -> Result<(), String> {
        let mut sources = BTreeMap::new();
        let mut values = BTreeMap::new();
        for name in FILES {
            let path = directory.join(name);
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let value: Value = serde_json::from_slice(&bytes)
                        .map_err(|e| error(format!("Import {name}: {e}")))?;
                    if !value.is_object() {
                        return Err(error(format!("Import {name} must be an object.")));
                    }
                    sources.insert(name.to_owned(), format!("{:x}", Sha256::digest(&bytes)));
                    values.insert(name.to_owned(), value);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(error(format!("Import {name}: {e}"))),
            }
        }
        // Preserve store.rs's project alias/collision migration for pre-index profiles.
        if let Some(index) = crate::store::load_global(directory, None)? {
            values.insert("index.json".into(), index);
        }
        crate::model::State::validate_import(directory, &values).map_err(error)?;
        let ui = values
            .remove("server-inputs.json")
            .or_else(|| values.get("ui-state.json").cloned())
            .unwrap_or_else(|| json!({"version":1}));
        // Drafts, tabs, active selection and layout belong only to the original device.
        let mut server_ui = json!({"version":1});
        if let Some(value) = ui.get("pendingMessages") {
            server_ui["pendingMessages"] = value.clone();
        }
        if let Some(keep_awake) = ui["background"].get("keep_awake") {
            server_ui["background"] = json!({"keep_awake":keep_awake});
        }
        values.remove("ui-state.json");
        values.insert("server-inputs.json".into(), server_ui);
        let tx = connection.transaction().map_err(error)?;
        for (name, value) in &values {
            tx.execute(
                "INSERT INTO documents(name,payload) VALUES(?1,?2)",
                params![name, value.to_string()],
            )
            .map_err(error)?;
            if name == "honcho-outbox.json" {
                sync_honcho(&tx, value)?;
            }
            let verified: String = tx
                .query_row("SELECT payload FROM documents WHERE name=?1", [name], |r| {
                    r.get(0)
                })
                .map_err(error)?;
            if serde_json::from_str::<Value>(&verified).map_err(error)? != *value {
                return Err(error("Import verification mismatch."));
            }
        }
        if let Some(index) = values.get("index.json") {
            for receipt in index["commandReceipts"].as_array().into_iter().flatten() {
                let receipt: Receipt = serde_json::from_value(receipt.clone()).map_err(error)?;
                put_receipt(&tx, &receipt)?;
            }
        }
        for (name, hash) in &sources {
            let bytes = std::fs::read(directory.join(name)).map_err(error)?;
            if *hash != format!("{:x}", Sha256::digest(bytes)) {
                return Err(error("A legacy source changed during import."));
            }
        }
        tx.execute(
            "INSERT INTO metadata(key,value) VALUES('legacy-import-v1',?1)",
            [json!({"sources":sources,"documents":values.len(),"importedAt":crate::lifecycle::now()}).to_string()],
        ).map_err(error)?;
        tx.commit().map_err(error)
    }

    pub fn read(&self, name: &str) -> Result<Option<Value>, String> {
        let inner = self.0.lock().expect("database");
        if let Some(failure) = &inner.failure {
            return Err(failure.clone());
        }
        if inner.batch == Some(std::thread::current().id())
            && let Some(value) = inner.staged.get(name)
        {
            return Ok(Some(value.clone()));
        }
        let value: Option<String> = inner
            .connection
            .query_row("SELECT payload FROM documents WHERE name=?1", [name], |r| {
                r.get(0)
            })
            .optional()
            .map_err(error)?;
        value
            .map(|value| serde_json::from_str(&value).map_err(error))
            .transpose()
    }

    pub fn write(&self, name: &str, value: Value) -> Result<(), String> {
        let mut inner = self.0.lock().expect("database");
        if let Some(failure) = &inner.failure {
            return Err(failure.clone());
        }
        if inner.batch.is_some() {
            inner.staged.insert(name.into(), value);
            return Ok(());
        }
        let result = (|| {
            let tx = inner.connection.transaction().map_err(error)?;
            tx.execute(
                "INSERT INTO documents(name,payload) VALUES(?1,?2)
                 ON CONFLICT(name) DO UPDATE SET payload=excluded.payload",
                params![name, value.to_string()],
            )
            .map_err(error)?;
            if name == "honcho-outbox.json" {
                sync_honcho(&tx, &value)?;
            }
            tx.commit().map_err(error)
        })();
        if let Err(error) = &result {
            inner.failure = Some(error.clone());
        }
        result
    }

    pub fn begin(&self) -> Result<(), String> {
        let mut inner = self.0.lock().expect("database");
        if let Some(error) = &inner.failure {
            return Err(error.clone());
        }
        if inner.batch.is_some() {
            return Err(error("Nested runtime transaction."));
        }
        inner.batch = Some(std::thread::current().id());
        Ok(())
    }

    pub fn commit(
        &self,
        seq: u64,
        projection: &Value,
        patch: &Value,
        receipt: Option<&Receipt>,
        effects: &[Effect],
    ) -> Result<(), String> {
        let mut inner = self.0.lock().expect("database");
        if let Some(error) = &inner.failure {
            return Err(error.clone());
        }
        let staged = std::mem::take(&mut inner.staged);
        let revocations = std::mem::take(&mut inner.revocations);
        let result = (|| {
            let tx = inner.connection.transaction().map_err(error)?;
            for id in revocations {
                tx.execute(
                    "UPDATE devices SET revoked_at=COALESCE(revoked_at,?2) WHERE id=?1",
                    params![id, crate::lifecycle::now() as i64],
                )
                .map_err(error)?;
            }
            for (name, value) in staged {
                tx.execute(
                    "INSERT INTO documents(name,payload) VALUES(?1,?2)
                     ON CONFLICT(name) DO UPDATE SET payload=excluded.payload",
                    params![name, value.to_string()],
                )
                .map_err(error)?;
                if name == "honcho-outbox.json" {
                    sync_honcho(&tx, &value)?;
                }
            }
            if let Some(receipt) = receipt {
                put_receipt(&tx, receipt)?;
            }
            for effect in effects {
                tx.execute(
                    "INSERT INTO effect_outbox(id,command_id,device,kind,payload,status)
                     VALUES(?1,?2,?3,'harness',?4,'pending')",
                    params![
                        effect.id,
                        receipt.map(|r| r.command_id.to_string()),
                        receipt.map(|r| &r.device_id),
                        effect.payload.to_string()
                    ],
                )
                .map_err(error)?;
            }
            tx.execute(
                "INSERT INTO events(seq,command_id,payload) VALUES(?1,?2,?3)",
                params![
                    seq as i64,
                    receipt.map(|r| r.command_id.to_string()),
                    patch.to_string()
                ],
            )
            .map_err(error)?;
            tx.execute(
                "INSERT INTO projection(id,seq,payload) VALUES(1,?1,?2)
                 ON CONFLICT(id) DO UPDATE SET seq=excluded.seq,payload=excluded.payload",
                params![seq as i64, projection.to_string()],
            )
            .map_err(error)?;
            // Bound replay storage. Clients behind this range get a fresh paged snapshot.
            tx.execute(
                "DELETE FROM events WHERE seq <= ?1",
                [seq.saturating_sub(1024) as i64],
            )
            .map_err(error)?;
            tx.commit().map_err(error)
        })();
        inner.batch = None;
        if let Err(error) = &result {
            inner.failure = Some(error.clone());
        }
        result
    }

    pub fn receipt(&self, device: &str, id: uuid::Uuid) -> Result<Option<Receipt>, String> {
        let inner = self.0.lock().expect("database");
        let text: Option<String> = inner
            .connection
            .query_row(
                "SELECT payload FROM command_receipts WHERE device=?1 AND command_id=?2",
                params![device, id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(error)?;
        text.map(|text| serde_json::from_str(&text).map_err(error))
            .transpose()
    }

    pub fn replay(&self, after: u64, through: u64) -> Result<Option<Vec<Value>>, String> {
        if after > through || through - after > 128 {
            return Ok(None);
        }
        let inner = self.0.lock().expect("database");
        let mut statement = inner
            .connection
            .prepare("SELECT seq,payload FROM events WHERE seq>?1 AND seq<=?2 ORDER BY seq")
            .map_err(error)?;
        let rows = statement
            .query_map(params![after as i64, through as i64], |r| {
                Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
            })
            .map_err(error)?;
        let mut events = Vec::new();
        let mut bytes = 0;
        for row in rows {
            let (seq, payload) = row.map_err(error)?;
            if seq != after + events.len() as u64 + 1 {
                return Ok(None);
            }
            bytes += payload.len();
            if bytes > 1024 * 1024 {
                return Ok(None);
            }
            events.push(
                json!({"seq":seq,"patch":serde_json::from_str::<Value>(&payload).map_err(error)?}),
            );
        }
        Ok((events.len() as u64 == through - after).then_some(events))
    }

    pub fn effect_status(
        &self,
        id: &str,
        status: &str,
        detail: Option<&str>,
    ) -> Result<(), String> {
        let mut inner = self.0.lock().expect("database");
        if let Some(failure) = &inner.failure {
            return Err(failure.clone());
        }
        let result = inner
            .connection
            .execute(
                "UPDATE effect_outbox SET status=?2,error=?3 WHERE id=?1",
                params![id, status, detail],
            )
            .map(|_| ())
            .map_err(error);
        if let Err(error) = &result {
            inner.failure = Some(error.clone());
        }
        result
    }

    pub fn issue_device(&self, id: &str, token: &str) -> Result<(), String> {
        let inner = self.0.lock().expect("database");
        if let Some(failure) = &inner.failure {
            return Err(failure.clone());
        }
        let count: i64 = inner
            .connection
            .query_row(
                "SELECT count(*) FROM devices WHERE id != ?1 AND revoked_at IS NULL",
                [id],
                |r| r.get(0),
            )
            .map_err(error)?;
        if count >= 256 {
            return Err("Device limit reached.".into());
        }
        inner.connection.execute(
            "INSERT INTO devices(id,token_hash,created_at) VALUES(?1,?2,?3)
             ON CONFLICT(id) DO UPDATE SET token_hash=excluded.token_hash WHERE devices.revoked_at IS NULL",
            params![id, hash_token(token), crate::lifecycle::now() as i64],
        ).map_err(error)?;
        drop(inner);
        if self.device(token)?.is_none() {
            return Err("Device revoked. Enroll a new device with user approval.".into());
        }
        Ok(())
    }

    pub fn device(&self, token: &str) -> Result<Option<String>, String> {
        self.0
            .lock()
            .expect("database")
            .connection
            .query_row(
                "SELECT id FROM devices WHERE token_hash=?1 AND revoked_at IS NULL",
                [hash_token(token)],
                |row| row.get(0),
            )
            .optional()
            .map_err(error)
    }

    pub fn devices(&self) -> Result<Value, String> {
        let inner = self.0.lock().expect("database");
        let mut stmt = inner
            .connection
            .prepare("SELECT id,created_at,revoked_at FROM devices ORDER BY created_at,id")
            .map_err(error)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(json!({
                    "deviceId":r.get::<_,String>(0)?,"createdAt":r.get::<_,i64>(1)?,
                    "revokedAt":r.get::<_,Option<i64>>(2)?
                }))
            })
            .map_err(error)?;
        Ok(json!(rows.collect::<Result<Vec<_>, _>>().map_err(error)?))
    }

    pub fn revoke(&self, id: &str) -> Result<(), String> {
        let mut inner = self.0.lock().expect("database");
        if let Some(failure) = &inner.failure {
            return Err(failure.clone());
        }
        let exists: bool = inner
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1)",
                [id],
                |r| r.get(0),
            )
            .map_err(error)?;
        if !exists {
            return Err("Unknown device ID.".into());
        }
        if inner.batch.is_some() {
            inner.revocations.push(id.into());
            return Ok(());
        }
        inner
            .connection
            .execute(
                "UPDATE devices SET revoked_at=COALESCE(revoked_at,?2) WHERE id=?1",
                params![id, crate::lifecycle::now() as i64],
            )
            .map(|_| ())
            .map_err(error)
    }

    pub fn sequence(&self) -> Result<u64, String> {
        self.0
            .lock()
            .expect("database")
            .connection
            .query_row("SELECT seq FROM projection WHERE id=1", [], |r| {
                r.get::<_, i64>(0)
            })
            .optional()
            .map(|seq| seq.unwrap_or(0) as u64)
            .map_err(error)
    }

    pub fn projection(&self) -> Result<Option<Value>, String> {
        let text: Option<String> = self
            .0
            .lock()
            .expect("database")
            .connection
            .query_row("SELECT payload FROM projection WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(error)?;
        text.map(|text| serde_json::from_str(&text).map_err(error))
            .transpose()
    }

    /// SQLite creates a consistent, non-overwriting backup, including committed WAL pages.
    /// Call this under the profile owner lock before a future schema upgrade or update.
    pub fn backup(&self, destination: &Path) -> Result<(), String> {
        self.0
            .lock()
            .expect("database")
            .connection
            .execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])
            .map_err(error)?;
        let backup =
            Connection::open_with_flags(destination, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(error)?;
        let integrity: String = backup
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(error)?;
        if integrity != "ok" {
            return Err(error("Backup verification failed."));
        }
        Ok(())
    }

    pub fn failure(&self) -> Option<String> {
        self.0.lock().expect("database").failure.clone()
    }

    pub fn fail(&self, message: String) {
        let mut inner = self.0.lock().expect("database");
        inner.failure = Some(message);
        inner.batch = None;
        inner.staged.clear();
        inner.revocations.clear();
    }

    pub fn finish_if_unchanged(&self) -> Result<bool, String> {
        let mut inner = self.0.lock().expect("database");
        if !inner.revocations.is_empty() {
            return Ok(false);
        }
        for (name, value) in &inner.staged {
            let old: Option<String> = inner
                .connection
                .query_row("SELECT payload FROM documents WHERE name=?1", [name], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(error)?;
            if old.as_deref() != Some(value.to_string().as_str()) {
                return Ok(false);
            }
        }
        inner.batch = None;
        inner.staged.clear();
        Ok(true)
    }

    pub fn device_active(&self, id: &str) -> Result<bool, String> {
        self.0
            .lock()
            .expect("database")
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND revoked_at IS NULL)",
                [id],
                |r| r.get(0),
            )
            .map_err(error)
    }
}

fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn sync_honcho(connection: &Connection, value: &Value) -> Result<(), String> {
    for turn in value["pending"].as_array().into_iter().flatten() {
        let id = turn["id"].as_str().ok_or("Invalid Honcho turn ID.")?;
        connection
            .execute(
                "INSERT OR IGNORE INTO effect_outbox(id,kind,payload,status)
             VALUES(?1,'honcho',?2,'pending')",
                params![format!("honcho:{id}"), turn.to_string()],
            )
            .map_err(error)?;
    }
    Ok(())
}

fn put_receipt(connection: &Connection, receipt: &Receipt) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO command_receipts(device,command_id,hash,payload) VALUES(?1,?2,?3,?4)
         ON CONFLICT(device,command_id) DO UPDATE SET payload=excluded.payload",
            params![
                receipt.device_id,
                receipt.command_id.to_string(),
                receipt.hash,
                serde_json::to_string(receipt).map_err(error)?
            ],
        )
        .map(|_| ())
        .map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        outbox,
        protocol::{Command, ReceiptStatus},
    };

    fn directory(name: &str) -> PathBuf {
        let directory = crate::store::directory().unwrap().join(name);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn realistic_profile_import_is_verified_once_and_never_changes_legacy_sources() {
        let directory = directory("realistic-import");
        crate::model::State::write_realistic_legacy_fixture(&directory);
        crate::store::write_atomic(
            &directory.join("honcho-outbox.json"),
            json!({
                "version":1,"destination":"http://127.0.0.1/shina","instance":"fixture",
                "next":2,"captains":{"0":"captain-0"},"pending":[{
                    "id":"fixture-1","session":"fixture-thread-1","peer":"captain-0",
                    "worker":false,"user":"A previous prompt","reply":"A previous reply"
                }]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let original: BTreeMap<_, _> = FILES
            .iter()
            .filter_map(|name| {
                std::fs::read(directory.join(name))
                    .ok()
                    .map(|bytes| (*name, bytes))
            })
            .collect();
        let database = Database::open(&directory).unwrap();
        let index = database.read("index.json").unwrap().unwrap();
        assert_eq!(index["projects"].as_array().unwrap().len(), 1);
        assert!(
            index["captainHomes"]
                .as_object()
                .is_some_and(|homes| !homes.is_empty())
        );
        assert_eq!(index["tasks"].as_array().unwrap().len(), 1);
        assert!(
            index["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| !t["lifecycle"]["archivedAt"].is_null())
        );
        assert_eq!(
            database.read("memory.json").unwrap().unwrap()["captains"]["0"][0]["essential"],
            true
        );
        assert_eq!(
            database.read("routines.json").unwrap().unwrap()["routines"][0]["name"],
            "Check fixtures"
        );
        let ui = database.read("server-inputs.json").unwrap().unwrap();
        assert!(ui.get("drafts").is_none() && ui.get("sidebar").is_none());
        assert!(ui["pendingMessages"].is_object());
        assert!(directory.join("state.sqlite.import-v1.bak").exists());
        database
            .write("memory.json", json!({"changed":true}))
            .unwrap();
        drop(database);
        let database = Database::open(&directory).unwrap();
        assert_eq!(
            database.read("memory.json").unwrap().unwrap(),
            json!({"changed":true})
        );
        for (name, bytes) in original {
            assert_eq!(
                std::fs::read(directory.join(name)).unwrap(),
                bytes,
                "{name}"
            );
        }
        let marker: String = database
            .0
            .lock()
            .unwrap()
            .connection
            .query_row(
                "SELECT value FROM metadata WHERE key='legacy-import-v1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            serde_json::from_str::<Value>(&marker).unwrap()["sources"]["index.json"].is_string()
        );
    }

    #[test]
    fn corrupt_or_partial_import_has_no_documents_or_marker_and_can_be_repaired() {
        for name in [
            "index.json",
            "memory.json",
            "routines.json",
            "ui-state.json",
            "honcho-outbox.json",
        ] {
            let directory = directory(name);
            crate::store::write_atomic(&directory.join(name), b"{\"version\":").unwrap();
            assert!(
                Database::open(&directory)
                    .err()
                    .unwrap()
                    .contains("Writes are blocked")
            );
            let connection = Connection::open(directory.join("state.sqlite")).unwrap();
            let count: i64 = connection
                .query_row(
                    "SELECT (SELECT count(*) FROM documents)+(SELECT count(*) FROM metadata)",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
            drop(connection);
            // A valid JSON object with the wrong version also cannot complete import.
            crate::store::write_atomic(&directory.join(name), b"{\"version\":999}").unwrap();
            assert!(Database::open(&directory).is_err());
        }
    }

    #[test]
    fn damaged_database_never_falls_back_to_valid_json() {
        let directory = directory("damaged-database");
        crate::store::write_atomic(&directory.join("state.sqlite"), b"not sqlite").unwrap();
        let failure = Database::open(&directory).err().unwrap();
        assert!(failure.contains("Writes are blocked") && failure.contains("no JSON fallback"));
    }

    #[test]
    fn committed_prompt_is_interrupted_at_both_crash_boundaries_and_never_redispatched() {
        for dispatching in [false, true] {
            let directory = directory(if dispatching {
                "after-claim"
            } else {
                "before-claim"
            });
            let database = Database::open(&directory).unwrap();
            let command = Command::new("thread.send", json!({"text":"Only once"}));
            let receipt = Receipt {
                command_id: command.command_id,
                device_id: "device".into(),
                hash: command.hash(),
                status: ReceiptStatus::Accepted,
                result: json!({"threadId":1}),
                seq: 1,
            };
            database.begin().unwrap();
            database
                .write(
                    "index.json",
                    json!({"runtimeInputs":{"1":[{"text":"Only once"}]}}),
                )
                .unwrap();
            let capture = outbox::Capture::begin();
            let (sender, receiver) = std::sync::mpsc::sync_channel(8);
            outbox::send(&sender, json!({"type":"prompt","message":"Only once"})).unwrap();
            let effects = capture.finish();
            assert!(
                receiver.try_recv().is_err(),
                "dispatch must wait for commit"
            );
            database
                .commit(1, &json!({}), &json!([]), Some(&receipt), &effects)
                .unwrap();
            if dispatching {
                database
                    .effect_status(&effects[0].id, "dispatching", None)
                    .unwrap();
            }
            drop(effects);
            drop(database);
            let recovered = Database::open(&directory).unwrap();
            assert!(receiver.try_recv().is_err());
            assert!(matches!(
                recovered
                    .receipt("device", command.command_id)
                    .unwrap()
                    .unwrap()
                    .status,
                ReceiptStatus::Interrupted
            ));
            assert!(
                recovered.read("index.json").unwrap().unwrap()["runtimeInputs"]["1"].is_array()
            );
        }
    }

    #[test]
    fn a_failed_commit_publishes_nothing_and_blocks_later_writes() {
        let database = Database::open(&directory("failed-commit")).unwrap();
        database.begin().unwrap();
        database
            .write("memory.json", json!({"staged":true}))
            .unwrap();
        database.0.lock().unwrap().connection.execute_batch(
            "CREATE TRIGGER fail_commit BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'fixture disk error'); END;"
        ).unwrap();
        assert!(
            database
                .commit(1, &json!({}), &json!([]), None, &[])
                .is_err()
        );
        assert!(database.write("memory.json", json!({})).is_err());
        let inner = database.0.lock().unwrap();
        let count: i64 = inner
            .connection
            .query_row(
                "SELECT count(*) FROM documents WHERE name='memory.json'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn replay_is_contiguous_bounded_and_falls_back_for_trimmed_or_large_gaps() {
        let database = Database::open(&directory("replay")).unwrap();
        for seq in 1..=140 {
            database.begin().unwrap();
            database
                .commit(seq, &json!({}), &json!([]), None, &[])
                .unwrap();
        }
        assert!(database.replay(0, 140).unwrap().is_none());
        assert_eq!(database.replay(130, 140).unwrap().unwrap().len(), 10);
        assert!(database.replay(141, 140).unwrap().is_none());
        database
            .0
            .lock()
            .unwrap()
            .connection
            .execute("DELETE FROM events WHERE seq=135", [])
            .unwrap();
        assert!(database.replay(130, 140).unwrap().is_none());
        assert_eq!(database.replay(140, 140).unwrap().unwrap().len(), 0);
    }

    #[test]
    fn credentials_are_hashed_survive_restart_and_revocation_prevents_reenrollment() {
        let directory = directory("devices");
        let database = Database::open(&directory).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        database.issue_device(&id, "secret-token").unwrap();
        let hash: String = database
            .0
            .lock()
            .unwrap()
            .connection
            .query_row("SELECT token_hash FROM devices", [], |r| r.get(0))
            .unwrap();
        assert_ne!(hash, "secret-token");
        drop(database);
        let database = Database::open(&directory).unwrap();
        assert_eq!(
            database.device("secret-token").unwrap().as_deref(),
            Some(id.as_str())
        );
        database.revoke(&id).unwrap();
        assert!(database.device("secret-token").unwrap().is_none());
        assert!(database.issue_device(&id, "replacement-token").is_err());
        assert!(database.devices().unwrap()[0]["revokedAt"].is_number());
    }

    #[test]
    fn honcho_effects_are_not_visible_to_the_worker_until_the_state_commits() {
        let database = Database::open(&directory("honcho-atomic")).unwrap();
        database.begin().unwrap();
        database
            .write(
                "honcho-outbox.json",
                json!({"pending":[{"id":"stable-turn","user":"Hello","reply":"Reply"}]}),
            )
            .unwrap();
        let reader = database.clone();
        assert!(
            std::thread::spawn(move || reader.read("honcho-outbox.json").unwrap())
                .join()
                .unwrap()
                .is_none()
        );
        database
            .commit(1, &json!({}), &json!([]), None, &[])
            .unwrap();
        let reader = database.clone();
        assert!(
            std::thread::spawn(move || reader.read("honcho-outbox.json").unwrap())
                .join()
                .unwrap()
                .is_some()
        );
        let kind: String = database
            .0
            .lock()
            .unwrap()
            .connection
            .query_row(
                "SELECT kind FROM effect_outbox WHERE id='honcho:stable-turn'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "honcho");
    }

    #[test]
    fn backups_include_wal_and_never_overwrite_a_previous_backup() {
        let directory = directory("backup");
        let database = Database::open(&directory).unwrap();
        database
            .write("memory.json", json!({"verified":"committed"}))
            .unwrap();
        let path = directory.join("before-upgrade.sqlite");
        database.backup(&path).unwrap();
        assert!(database.backup(&path).is_err());
        let backup = Connection::open(path).unwrap();
        let saved: String = backup
            .query_row(
                "SELECT payload FROM documents WHERE name='memory.json'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&saved).unwrap(),
            json!({"verified":"committed"})
        );
    }
}
