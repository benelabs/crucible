-- Backend contract service tables for storage optimization, versioning,
-- deployment automation, and test result storage.

CREATE TABLE IF NOT EXISTS contract_storage_optimizations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_id TEXT NOT NULL,
    target_network TEXT NOT NULL,
    storage_entries_estimate BIGINT NOT NULL,
    estimated_rent_savings_percent DOUBLE PRECISION NOT NULL,
    ttl_strategy TEXT NOT NULL,
    recommendations JSONB NOT NULL DEFAULT '[]',
    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_contract_storage_optimizations_contract_id
    ON contract_storage_optimizations (contract_id);

CREATE TABLE IF NOT EXISTS contract_versions (
    id TEXT PRIMARY KEY,
    contract_id TEXT NOT NULL,
    version TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    wasm_hash TEXT,
    changelog TEXT,
    created_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_contract_versions_contract_version UNIQUE (contract_id, version)
);

CREATE INDEX IF NOT EXISTS idx_contract_versions_contract_id
    ON contract_versions (contract_id);

CREATE TABLE IF NOT EXISTS contract_deployments (
    id TEXT PRIMARY KEY,
    contract_id TEXT NOT NULL,
    version TEXT NOT NULL,
    network TEXT NOT NULL,
    deployer TEXT NOT NULL,
    wasm_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('planned', 'queued', 'running', 'succeeded', 'failed')),
    transaction_envelope TEXT,
    steps JSONB NOT NULL DEFAULT '[]',
    checks JSONB NOT NULL DEFAULT '[]',
    dry_run BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_contract_deployments_contract_id
    ON contract_deployments (contract_id);
CREATE INDEX IF NOT EXISTS idx_contract_deployments_network_status
    ON contract_deployments (network, status);

CREATE TABLE IF NOT EXISTS contract_test_runs (
    id TEXT PRIMARY KEY,
    contract_id TEXT NOT NULL,
    build_id TEXT,
    status TEXT NOT NULL CHECK (status IN ('passed', 'failed', 'error', 'running')),
    total_tests BIGINT NOT NULL,
    passed_tests BIGINT NOT NULL,
    failed_tests BIGINT NOT NULL,
    skipped_tests BIGINT NOT NULL,
    duration_ms BIGINT,
    metadata JSONB NOT NULL DEFAULT '{}',
    completed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_contract_test_runs_contract_id
    ON contract_test_runs (contract_id);
CREATE INDEX IF NOT EXISTS idx_contract_test_runs_status
    ON contract_test_runs (status);

CREATE TABLE IF NOT EXISTS contract_test_cases (
    id TEXT PRIMARY KEY,
    test_run_id TEXT NOT NULL REFERENCES contract_test_runs(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('passed', 'failed', 'skipped', 'running')),
    duration_ms BIGINT,
    gas_used BIGINT,
    error_message TEXT,
    stack_trace TEXT
);

CREATE INDEX IF NOT EXISTS idx_contract_test_cases_run_id
    ON contract_test_cases (test_run_id);
