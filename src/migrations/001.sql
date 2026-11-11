CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE documents (name TEXT PRIMARY KEY, payload TEXT NOT NULL CHECK(json_valid(payload)));
CREATE TABLE events (
    seq INTEGER PRIMARY KEY, schema INTEGER NOT NULL DEFAULT 1,
    command_id TEXT, payload TEXT NOT NULL CHECK(json_valid(payload))
);
CREATE TABLE projection (id INTEGER PRIMARY KEY CHECK(id=1), seq INTEGER NOT NULL, payload TEXT NOT NULL);
CREATE TABLE command_receipts (
    device TEXT NOT NULL, command_id TEXT NOT NULL, hash TEXT NOT NULL,
    payload TEXT NOT NULL, PRIMARY KEY(device, command_id)
);
CREATE TABLE effect_outbox (
    id TEXT PRIMARY KEY, command_id TEXT, device TEXT, kind TEXT NOT NULL,
    payload TEXT NOT NULL, status TEXT NOT NULL
        CHECK(status IN ('pending','dispatching','completed','interrupted')),
    error TEXT
);
CREATE TABLE devices (
    id TEXT PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL, revoked_at INTEGER
);
PRAGMA user_version=1;
