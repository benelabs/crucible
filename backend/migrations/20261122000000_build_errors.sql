-- Migration: build error analytics storage (#1490)
-- The error analytics dashboard and its integration tests require a
-- persistent table for recorded build errors.

CREATE TABLE IF NOT EXISTS build_errors (
    id          BIGSERIAL PRIMARY KEY,
    error_type  TEXT NOT NULL,
    message     TEXT NOT NULL,
    occurred_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_build_errors_occurred_at
    ON build_errors (occurred_at DESC);
