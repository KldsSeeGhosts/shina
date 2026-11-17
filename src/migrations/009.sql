-- Feature-local task-operation evidence. The reducer projects a bounded view
-- into task_status and the index; this table keeps every keyed control result
-- so an old client_request_id replays its original outcome after restart.
CREATE TABLE task_operations (
    scope TEXT NOT NULL,
    request_key TEXT NOT NULL,
    command_id TEXT NOT NULL,
    task_id TEXT,
    target_thread_id INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY(scope, request_key)
);
CREATE INDEX task_operations_task ON task_operations(task_id);
CREATE INDEX task_operations_thread ON task_operations(target_thread_id);
PRAGMA user_version=9;
