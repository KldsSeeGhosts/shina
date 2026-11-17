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

-- A target stores every accepted peer input or stop intent before it may
-- touch a provider. The payload carries the exact command, attempt, request,
-- runtime generation, requested and realized delivery, queue state and
-- provider observations. Accepted entries survive restart until their
-- provider outcome is reconciled.
CREATE TABLE task_input_intents (
    thread_id INTEGER NOT NULL,
    command_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    attempt_id TEXT,
    request_id TEXT,
    runtime_generation TEXT,
    updated_at INTEGER NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY(thread_id, command_id)
);
CREATE INDEX task_input_intents_attempt
    ON task_input_intents(thread_id, attempt_id);
PRAGMA user_version=9;
