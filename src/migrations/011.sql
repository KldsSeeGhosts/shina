CREATE INDEX IF NOT EXISTS thread_events_seq_thread_idx ON thread_events(seq, thread_id);
PRAGMA user_version=11;
