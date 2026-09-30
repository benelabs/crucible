ALTER TABLE contract_events
    ADD COLUMN event_index INTEGER;

WITH ranked_events AS (
    SELECT
        id,
        (
            ROW_NUMBER() OVER (
                PARTITION BY transaction_hash
                ORDER BY indexed_at, id
            ) - 1
        )::INTEGER AS event_index
    FROM contract_events
)
UPDATE contract_events AS events
SET event_index = ranked_events.event_index
FROM ranked_events
WHERE events.id = ranked_events.id;

ALTER TABLE contract_events
    ALTER COLUMN event_index SET NOT NULL;

ALTER TABLE contract_events
    DROP CONSTRAINT uq_contract_events_tx_type;

ALTER TABLE contract_events
    ADD CONSTRAINT uq_contract_events_tx_index
    UNIQUE (transaction_hash, event_index);