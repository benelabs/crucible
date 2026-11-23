-- Migration: Add is_active column and index to users table for administrative suspension
ALTER TABLE users ADD COLUMN IF NOT EXISTS is_active BOOLEAN NOT NULL DEFAULT true;
CREATE INDEX IF NOT EXISTS idx_users_is_active ON users(is_active);
