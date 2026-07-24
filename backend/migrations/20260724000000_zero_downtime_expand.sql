-- Phase 1: Expand Phase for Zero-Downtime Schema Upgrade
-- Example: Upgrading audit_logs table by expanding metadata into structured columns without locking or breaking existing API versions.

ALTER TABLE audit_logs ADD COLUMN IF NOT EXISTS event_category TEXT DEFAULT 'general';
ALTER TABLE audit_logs ADD COLUMN IF NOT EXISTS client_ip TEXT;

-- Create dual-write trigger function for zero-downtime backward compatibility
CREATE OR REPLACE FUNCTION sync_audit_logs_event_category()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.event_category IS NULL THEN
        NEW.event_category := 'general';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_sync_audit_logs_category ON audit_logs;
CREATE TRIGGER trg_sync_audit_logs_category
    BEFORE INSERT OR UPDATE ON audit_logs
    FOR EACH ROW
    EXECUTE FUNCTION sync_audit_logs_event_category();
