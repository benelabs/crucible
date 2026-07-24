-- Phase 2: Contract Phase for Zero-Downtime Schema Upgrade
-- Executed after all API instances are updated to read/write new structured columns.

-- Ensure index exists on newly expanded column
CREATE INDEX IF NOT EXISTS idx_audit_logs_event_category ON audit_logs(event_category);

-- Cleanup temporary triggers after deprecation window
DROP TRIGGER IF EXISTS trg_sync_audit_logs_category ON audit_logs;
DROP FUNCTION IF EXISTS sync_audit_logs_event_category();
