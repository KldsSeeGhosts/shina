-- Stable logical identities are additive. Existing numeric keys, command
-- hashes, task/delivery ids and provider session strings remain untouched.
CREATE TABLE identity_workspace (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    workspace_id TEXT NOT NULL UNIQUE,
    owner_id TEXT NOT NULL UNIQUE,
    controller_id TEXT NOT NULL,
    executor_id TEXT NOT NULL,
    identity_schema INTEGER NOT NULL CHECK(identity_schema > 0),
    backup_path TEXT,
    source_index_sha256 TEXT,
    backup_sha256 TEXT,
    migrated_at INTEGER NOT NULL,
    index_version INTEGER NOT NULL
);

CREATE TABLE identity_entities (
    controller_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN
        ('captain','thread','project_checkout','task','routine')),
    logical_id TEXT NOT NULL,
    canonical_key TEXT NOT NULL,
    PRIMARY KEY(controller_id,kind,logical_id),
    UNIQUE(controller_id,kind,canonical_key)
);

-- A duplicate local project path can have several legacy numeric aliases that
-- resolve to one checkout identity. The alias key is never rewritten.
CREATE TABLE identity_aliases (
    controller_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN
        ('captain','thread','project_checkout','task','routine')),
    legacy_key TEXT NOT NULL,
    logical_id TEXT NOT NULL,
    PRIMARY KEY(controller_id,kind,legacy_key),
    FOREIGN KEY(controller_id,kind,logical_id)
        REFERENCES identity_entities(controller_id,kind,logical_id)
);

-- Internal MCP receipt owners are audit subjects, never credentials or devices.
CREATE TABLE identity_audit_subjects (
    subject TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('captain','thread')),
    legacy_key TEXT NOT NULL,
    logical_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    controller_id TEXT NOT NULL,
    FOREIGN KEY(controller_id,kind,logical_id)
        REFERENCES identity_entities(controller_id,kind,logical_id)
);

CREATE TRIGGER identity_audit_subject_is_not_device
BEFORE INSERT ON identity_audit_subjects
WHEN EXISTS(SELECT 1 FROM devices WHERE id=NEW.subject)
BEGIN
    SELECT RAISE(ABORT,'An identity audit subject is already a registered device.');
END;

CREATE TRIGGER identity_device_is_not_audit_subject
BEFORE INSERT ON devices
WHEN NEW.id LIKE 'mcp/%' OR EXISTS(
    SELECT 1 FROM identity_audit_subjects WHERE subject=NEW.id
)
BEGIN
    SELECT RAISE(ABORT,'An internal audit subject cannot become a device.');
END;

PRAGMA user_version=8;
