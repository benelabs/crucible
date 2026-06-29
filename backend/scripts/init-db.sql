-- =============================================================================
-- Crucible Backend — Database Initialization Script
-- =============================================================================
-- This script runs automatically when the PostgreSQL container starts for the
-- first time (mounted into /docker-entrypoint-initdb.d/).
--
-- It sets up:
--   1. Required PostgreSQL extensions
--   2. Core application schema
--   3. Indexes for query performance
--   4. Seed data for development
-- =============================================================================

-- ---------------------------------------------------------------------------
-- Extensions
-- ---------------------------------------------------------------------------
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";     -- UUID generation
CREATE EXTENSION IF NOT EXISTS "pgcrypto";      -- Cryptographic functions
CREATE EXTENSION IF NOT EXISTS "citext";         -- Case-insensitive text

-- ---------------------------------------------------------------------------
-- Schema: Core Tables
-- ---------------------------------------------------------------------------

-- Contracts: stores deployed smart contract metadata
CREATE TABLE IF NOT EXISTS contracts (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name            CITEXT NOT NULL,
    address         VARCHAR(128) NOT NULL UNIQUE,
    network         VARCHAR(32) NOT NULL DEFAULT 'testnet',
    wasm_hash       VARCHAR(128),
    deployer        VARCHAR(128),
    description     TEXT,
    abi_json        JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Test runs: stores test execution results
CREATE TABLE IF NOT EXISTS test_runs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    contract_id     UUID NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
    status          VARCHAR(32) NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'running', 'passed', 'failed', 'error')),
    total_tests     INTEGER NOT NULL DEFAULT 0,
    passed_tests    INTEGER NOT NULL DEFAULT 0,
    failed_tests    INTEGER NOT NULL DEFAULT 0,
    duration_ms     BIGINT,
    error_message   TEXT,
    metadata        JSONB DEFAULT '{}',
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Test cases: individual test results within a run
CREATE TABLE IF NOT EXISTS test_cases (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    test_run_id     UUID NOT NULL REFERENCES test_runs(id) ON DELETE CASCADE,
    name            VARCHAR(512) NOT NULL,
    status          VARCHAR(32) NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'running', 'passed', 'failed', 'skipped')),
    duration_ms     BIGINT,
    gas_used        BIGINT,
    error_message   TEXT,
    stack_trace     TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Job queue: background job tracking
CREATE TABLE IF NOT EXISTS jobs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    job_type        VARCHAR(128) NOT NULL,
    payload         JSONB NOT NULL DEFAULT '{}',
    status          VARCHAR(32) NOT NULL DEFAULT 'queued'
                    CHECK (status IN ('queued', 'running', 'completed', 'failed', 'retrying')),
    attempts        INTEGER NOT NULL DEFAULT 0,
    max_attempts    INTEGER NOT NULL DEFAULT 3,
    last_error      TEXT,
    scheduled_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- Indexes
-- ---------------------------------------------------------------------------
CREATE INDEX IF NOT EXISTS idx_contracts_network ON contracts(network);
CREATE INDEX IF NOT EXISTS idx_contracts_created_at ON contracts(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_test_runs_contract_id ON test_runs(contract_id);
CREATE INDEX IF NOT EXISTS idx_test_runs_status ON test_runs(status);
CREATE INDEX IF NOT EXISTS idx_test_runs_created_at ON test_runs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_test_cases_test_run_id ON test_cases(test_run_id);
CREATE INDEX IF NOT EXISTS idx_test_cases_status ON test_cases(status);
CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
CREATE INDEX IF NOT EXISTS idx_jobs_job_type ON jobs(job_type);
CREATE INDEX IF NOT EXISTS idx_jobs_scheduled_at ON jobs(scheduled_at);

-- Contract storage optimization reports
CREATE TABLE IF NOT EXISTS contract_storage_optimizations (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
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

-- Contract versions and artifact hashes
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

-- Deployment automation jobs
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

-- Stored contract test results
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

-- ---------------------------------------------------------------------------
-- Functions: Auto-update updated_at timestamp
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Apply the trigger to tables with updated_at
DO $$ BEGIN
    CREATE TRIGGER set_updated_at_contracts
        BEFORE UPDATE ON contracts
        FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
    CREATE TRIGGER set_updated_at_jobs
        BEFORE UPDATE ON jobs
        FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

-- ---------------------------------------------------------------------------
-- Seed Data (development only)
-- ---------------------------------------------------------------------------
INSERT INTO contracts (name, address, network, description)
VALUES
    ('Counter', 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC', 'testnet',
     'Simple counter contract for testing basic increment/decrement operations.'),
    ('Token',   'CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM',  'testnet',
     'Standard token contract implementing SEP-41 interface.'),
    ('Escrow',  'CBIJHCAIAP5BO4V4L5LRIA3XDCGAVSGAYLNAHVRMHMXBGIGMXSQKIBXKE', 'testnet',
     'Multi-party escrow contract with arbiter-based dispute resolution.')
ON CONFLICT (address) DO NOTHING;

-- =============================================================================
-- Initialization complete
-- =============================================================================






