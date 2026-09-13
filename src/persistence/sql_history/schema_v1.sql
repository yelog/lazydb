CREATE TABLE IF NOT EXISTS history_executions (
    execution_id TEXT PRIMARY KEY NOT NULL,
    operation_id TEXT NOT NULL,
    transaction_id TEXT,
    requested_at INTEGER NOT NULL,
    sql TEXT NOT NULL,
    status TEXT NOT NULL,
    certainty TEXT NOT NULL,
    transaction_outcome TEXT NOT NULL,
    affected_rows INTEGER,
    returned_rows INTEGER,
    elapsed_millis INTEGER,
    profile_id TEXT,
    database_name TEXT,
    schema_name TEXT
);

CREATE INDEX IF NOT EXISTS history_executions_requested_at
    ON history_executions (requested_at DESC, execution_id DESC);
CREATE INDEX IF NOT EXISTS history_executions_status
    ON history_executions (status, requested_at DESC, execution_id DESC);
CREATE INDEX IF NOT EXISTS history_executions_transaction
    ON history_executions (transaction_id, requested_at DESC);
