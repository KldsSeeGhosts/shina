CREATE TABLE mobile_notification_events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT CHECK(seq > 0),
    id TEXT NOT NULL UNIQUE,
    schema INTEGER NOT NULL DEFAULT 1 CHECK(schema > 0),
    kind TEXT NOT NULL CHECK(kind IN ('approval','user_input','run_completed','run_failed')),
    thread_id INTEGER NOT NULL,
    run_id TEXT,
    request_id TEXT,
    generation TEXT,
    source_key TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    push_required INTEGER NOT NULL DEFAULT 0,
    delivered_at INTEGER
);
CREATE INDEX mobile_notification_events_thread ON mobile_notification_events(thread_id, seq);
PRAGMA user_version=6;
