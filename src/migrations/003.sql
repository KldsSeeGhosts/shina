CREATE TABLE update_audit (
    update_id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id),
    requested_at INTEGER NOT NULL,
    accepted INTEGER NOT NULL,
    tag TEXT,
    outcome TEXT NOT NULL
);
CREATE INDEX update_audit_device_time ON update_audit(device_id, requested_at);
PRAGMA user_version=3;
