-- Add role enum type
CREATE TYPE user_role AS ENUM ('admin', 'user', 'guest');

-- Add role column to users table
ALTER TABLE users ADD COLUMN IF NOT EXISTS role user_role NOT NULL DEFAULT 'guest';

-- Create permissions table
CREATE TABLE IF NOT EXISTS permissions (
    id SERIAL PRIMARY KEY,
    resource TEXT NOT NULL,
    action TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(resource, action)
);

-- Create user_permissions junction table
CREATE TABLE IF NOT EXISTS user_permissions (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    permission_id INTEGER NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
    granted_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    granted_by INTEGER REFERENCES users(id),
    PRIMARY KEY (user_id, permission_id)
);

-- Create indexes for performance
CREATE INDEX IF NOT EXISTS idx_user_permissions_user_id ON user_permissions(user_id);
CREATE INDEX IF NOT EXISTS idx_user_permissions_permission_id ON user_permissions(permission_id);
CREATE INDEX IF NOT EXISTS idx_permissions_resource_action ON permissions(resource, action);

-- Insert default permissions
INSERT INTO permissions (resource, action, description) VALUES
    ('contracts', 'read', 'View contract details'),
    ('contracts', 'write', 'Create or update contracts'),
    ('contracts', 'delete', 'Delete contracts'),
    ('test_runs', 'read', 'View test run results'),
    ('test_runs', 'write', 'Create test runs'),
    ('test_runs', 'delete', 'Delete test runs'),
    ('users', 'read', 'View user information'),
    ('users', 'write', 'Create or update users'),
    ('users', 'delete', 'Delete users'),
    ('permissions', 'manage', 'Manage user permissions')
ON CONFLICT (resource, action) DO NOTHING;
