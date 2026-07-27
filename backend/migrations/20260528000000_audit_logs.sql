-- Add audit_logs table for audit logging service with hash chain tamper protection
CREATE TABLE IF NOT EXISTS audit_logs (
    id SERIAL PRIMARY KEY,
    event_type TEXT NOT NULL,
    user_id TEXT,
    details JSONB NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    hash TEXT NOT NULL DEFAULT '',
    previous_hash TEXT NOT NULL DEFAULT ''
);

-- Index for chain verification queries (ordered by id for linear chain walk)
CREATE INDEX IF NOT EXISTS idx_audit_logs_id_asc ON audit_logs (id ASC);
