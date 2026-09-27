-- Phase 2: Contract Phase for Zero-Downtime Schema Upgrade
-- Executed after all API instances are updated to read/write new structured columns.

-- Enforce DEFAULT and NOT NULL constraint after backfill completion
ALTER TABLE audit_logs ALTER COLUMN event_category SET DEFAULT 'general';
UPDATE audit_logs SET event_category = 'general' WHERE event_category IS NULL;
ALTER TABLE audit_logs ALTER COLUMN event_category SET NOT NULL;

-- Ensure index exists on newly expanded column
CREATE INDEX IF NOT EXISTS idx_audit_logs_event_category ON audit_logs(event_category);

-- Cleanup temporary triggers after deprecation window
DROP TRIGGER IF EXISTS trg_sync_audit_logs_category ON audit_logs;
DROP FUNCTION IF EXISTS sync_audit_logs_event_category();
