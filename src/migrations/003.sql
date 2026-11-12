CREATE TABLE peers (
    server_id TEXT PRIMARY KEY,
    host TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    linked_at INTEGER NOT NULL,
    capabilities TEXT NOT NULL DEFAULT '{}',
    last_reached_at INTEGER,
    last_error TEXT
);
PRAGMA user_version=3;
