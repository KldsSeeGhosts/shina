-- Opt-in Expo push delivery. One registration row per authenticated device; the
-- push token lives only in this database. Generation increments on every
-- re-registration so queued work for a rotated or unregistered token is fenced.
CREATE TABLE mobile_push_registrations (
    device_id TEXT PRIMARY KEY REFERENCES devices(id),
    platform TEXT NOT NULL CHECK(platform IN ('ios','android')),
    token TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK(generation > 0),
    active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)),
    registered_at INTEGER NOT NULL,
    unregistered_at INTEGER,
    deactivated_at INTEGER,
    last_error TEXT
);
-- The durable outbox. One job per (device, event); jobs are created inside the
-- projection commit that records their event. Statuses: pending -> leased ->
-- submitted -> delivered; failed and cancelled are terminal. Expo ticket
-- acceptance is submission, never delivery; only a receipt proves delivery.
CREATE TABLE mobile_push_jobs (
    seq INTEGER PRIMARY KEY AUTOINCREMENT CHECK(seq > 0),
    event_seq INTEGER NOT NULL,
    event_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    device_id TEXT NOT NULL REFERENCES devices(id),
    generation INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN
        ('pending','leased','submitted','delivered','failed','cancelled')),
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL,
    lease_owner TEXT,
    lease_expires_at INTEGER,
    ticket_id TEXT,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(device_id, event_id)
);
CREATE INDEX mobile_push_jobs_due ON mobile_push_jobs(status, next_attempt_at);
CREATE INDEX mobile_push_jobs_device ON mobile_push_jobs(device_id, status, seq);
CREATE INDEX mobile_push_jobs_event ON mobile_push_jobs(event_seq);
PRAGMA user_version=7;
