//! KSG binaryPreview.worker.ts SQLite branch and BinaryViewer.
use crate::attachments::{self, Attachment};
use rusqlite::{Connection, OpenFlags};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

pub const PAGE_SIZE: usize = 50;
pub struct Table {
    pub name: String,
    pub count: u64,
}
pub struct Page {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}
pub struct Database {
    connection: Mutex<Connection>,
    pub tables: Vec<Table>,
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// KSG terminates its worker at the same deadlines. SQLite's interrupt prevents an
/// expensive count/generated expression from keeping Shina's UI or worker occupied.
struct Deadline(mpsc::Sender<()>);
impl Deadline {
    fn new(connection: &Connection, seconds: u64) -> Self {
        let interrupt = connection.get_interrupt_handle();
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            if receive.recv_timeout(Duration::from_secs(seconds)).is_err() {
                interrupt.interrupt();
            }
        });
        Self(send)
    }
}
impl Drop for Deadline {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

impl Database {
    pub fn open(attachment: &Attachment) -> Result<Self, String> {
        // SQL.js receives a byte snapshot. Use Shina's immutable blob, never open
        // the user's database read-write or attach its WAL.
        let file = attachments::stage_transfer_file(attachment)?;
        let path = attachments::path_for(&file).ok_or("This table is unavailable.")?;
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| e.to_string())?;
        let _deadline = Deadline::new(&connection, 30);
        connection
            .execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
            .map_err(|e| e.to_string())?;
        let names: Vec<String> = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND substr(name, 1, 7) != 'sqlite_' AND sql NOT LIKE 'CREATE VIRTUAL TABLE%' ORDER BY name LIMIT 101")
            .map_err(|e| e.to_string())?.query_map([], |row| row.get(0)).map_err(|e| e.to_string())?
            .collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        if names.len() > 100 {
            return Err("Database previews are limited to 100 tables.".into());
        }
        let mut tables = Vec::new();
        for name in names {
            let count = connection
                .query_row(
                    &format!("SELECT count(*) FROM {}", quote(&name)),
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())?;
            tables.push(Table {
                name,
                count: count.max(0) as u64,
            });
        }
        Ok(Self {
            connection: Mutex::new(connection),
            tables,
        })
    }
    pub fn page(&self, table: &str, page: usize) -> Result<Page, String> {
        if !self.tables.iter().any(|t| t.name == table) || page > 9_007_199_254_740_991 / PAGE_SIZE
        {
            return Err("This table is unavailable.".into());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "This table is unavailable.")?;
        let _deadline = Deadline::new(&connection, 15);
        let mut statement = connection
            .prepare(&format!("PRAGMA table_xinfo({})", quote(table)))
            .map_err(|e| e.to_string())?;
        let columns: Vec<String> = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(6)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| match r {
                Ok((_, 1)) => None,
                Ok((name, _)) => Some(Ok(name)),
                Err(e) => Some(Err(e)),
            })
            .take(50)
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        if columns.is_empty() {
            return Ok(Page {
                columns,
                rows: Vec::new(),
            });
        }
        let fields = columns.iter().map(|name| {
            let field = quote(name);
            format!("CASE WHEN typeof({field}) = 'blob' THEN '[blob: ' || length({field}) || ' bytes]' ELSE substr(CAST({field} AS TEXT), 1, 1024) END")
        }).collect::<Vec<_>>().join(",");
        let mut statement = connection
            .prepare(&format!(
                "SELECT {fields} FROM {} LIMIT {PAGE_SIZE} OFFSET {}",
                quote(table),
                page * PAGE_SIZE
            ))
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| {
                (0..columns.len())
                    .map(|i| {
                        row.get::<_, Option<String>>(i)
                            .map(|s| s.unwrap_or("NULL".into()))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(Page { columns, rows })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sqlite_preview_is_read_only_pages_fifty_rows_and_bounds_cells() {
        let path = crate::store::directory().unwrap().join("preview.sqlite");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE \"quoted\"\"table\" (n INTEGER, text TEXT, data BLOB, empty TEXT);",
        )
        .unwrap();
        for i in 0..55 {
            db.execute(
                "INSERT INTO \"quoted\"\"table\" VALUES (?1,?2,?3,NULL)",
                rusqlite::params![i, "猫".repeat(2000), vec![1u8; 500]],
            )
            .unwrap();
        }
        drop(db);
        let file = crate::attachments::attach_file(&path).unwrap();
        let database = Database::open(&file).unwrap();
        assert_eq!(database.tables[0].name, "quoted\"table");
        assert_eq!(database.tables[0].count, 55);
        let page = database.page("quoted\"table", 0).unwrap();
        assert_eq!(page.rows.len(), 50);
        assert_eq!(page.rows[0][1].chars().count(), 1024);
        assert_eq!(page.rows[0][2], "[blob: 500 bytes]");
        assert_eq!(page.rows[0][3], "NULL");
        assert_eq!(database.page("quoted\"table", 1).unwrap().rows.len(), 5);
        assert!(database.page("sqlite_schema", 0).is_err());
    }
}
