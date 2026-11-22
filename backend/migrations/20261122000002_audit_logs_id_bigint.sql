-- Migration: widen audit_logs.id to BIGINT (#1490)
-- AuditEventRecord.id is an i64; the original SERIAL column is INT4, which
-- makes every read path fail with a SQLx decode error.

ALTER TABLE audit_logs ALTER COLUMN id TYPE BIGINT;
