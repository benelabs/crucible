-- Migration: multi-network deployment stages (#1490)
-- Required by ContractDeploymentService::create_multi_network_deployment.

CREATE TABLE IF NOT EXISTS multi_network_deployments (
    deployment_id TEXT NOT NULL,
    network       TEXT NOT NULL,
    contract_id   TEXT NOT NULL,
    version       TEXT NOT NULL,
    status        TEXT NOT NULL,
    tx_hash       TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (deployment_id, network)
);

CREATE INDEX IF NOT EXISTS idx_multi_network_deployments_contract
    ON multi_network_deployments (contract_id, version);
