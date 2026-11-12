CREATE TABLE projection_records (
    document TEXT NOT NULL,
    scope TEXT NOT NULL,
    key TEXT NOT NULL,
    schema INTEGER NOT NULL CHECK(schema > 0),
    payload TEXT NOT NULL,
    PRIMARY KEY(document,scope,key)
) WITHOUT ROWID;
CREATE TABLE thread_events (
    thread_id INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    schema INTEGER NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY(thread_id,seq)
) WITHOUT ROWID;
PRAGMA user_version=5;
