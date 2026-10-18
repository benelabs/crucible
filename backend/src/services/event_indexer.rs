//! Contract event indexing service.
//!
//! Indexes Soroban contract events from the Stellar network into a queryable
//! local store backed by PostgreSQL, with Redis caching for hot queries.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

/// A single indexed contract event.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IndexedEvent {
    pub id: Uuid,
    pub contract_id: String,
    pub ledger_sequence: i64,
    pub transaction_hash: String,
    pub event_index: i32,
    pub event_type: String,
    pub topics: serde_json::Value,
    pub data: serde_json::Value,
    pub indexed_at: DateTime<Utc>,
}

/// Query parameters for filtering indexed events.
#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    pub contract_id: Option<String>,
    pub event_type: Option<String>,
    pub from_ledger: Option<i64>,
    pub to_ledger: Option<i64>,
    pub limit: Option<i64>,
}

/// Service responsible for ingesting and querying contract events.
pub struct EventIndexer {
    db: PgPool,
}

impl EventIndexer {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Ingest a batch of raw events from the Stellar network.
    #[instrument(skip(self, events), fields(count = events.len()))]
    pub async fn ingest(&self, events: Vec<RawEvent>) -> Result<usize, sqlx::Error> {
        if events.is_empty() {
            return Ok(0);
        }

        let mut inserted = 0;
        for event in events {
            let result = sqlx::query(
                r#"
                INSERT INTO contract_events
                    (id, contract_id, ledger_sequence, transaction_hash, event_index, event_type, topics, data, indexed_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT (transaction_hash, event_index) DO NOTHING
                "#,
            )
            .bind(Uuid::new_v4())
            .bind(&event.contract_id)
            .bind(event.ledger_sequence)
            .bind(&event.transaction_hash)
            .bind(event.event_index)
            .bind(&event.event_type)
            .bind(&event.topics)
            .bind(&event.data)
            .bind(Utc::now())
            .execute(&self.db)
            .await?;

            if result.rows_affected() > 0 {
                inserted += 1;
                debug!(contract_id = %event.contract_id, event_type = %event.event_type, "Indexed event");
            }
        }

        info!(inserted, "Event ingestion complete");
        Ok(inserted)
    }

    /// Query indexed events with optional filters.
    #[instrument(skip(self))]
    pub async fn query(&self, q: EventQuery) -> Result<Vec<IndexedEvent>, sqlx::Error> {
        // Build a dynamic query using a base + optional WHERE clauses.
        // sqlx doesn't support fully dynamic queries, so we use a fixed
        // parameterised form that covers all filter combinations.
        let limit = q.limit.unwrap_or(100).min(1000);

        let rows = sqlx::query_as::<_, IndexedEvent>(
            r#"
                 SELECT id, contract_id, ledger_sequence, transaction_hash, event_index,
                   event_type, topics, data, indexed_at
            FROM contract_events
            WHERE ($1::text IS NULL OR contract_id = $1)
              AND ($2::text IS NULL OR event_type  = $2)
              AND ($3::bigint IS NULL OR ledger_sequence >= $3)
              AND ($4::bigint IS NULL OR ledger_sequence <= $4)
            ORDER BY ledger_sequence DESC, indexed_at DESC
            LIMIT $5
            "#,
        )
        .bind(q.contract_id)
        .bind(q.event_type)
        .bind(q.from_ledger)
        .bind(q.to_ledger)
        .bind(limit)
        .fetch_all(&self.db)
        .await?;

        Ok(rows)
    }

    /// Return the highest ledger sequence that has been indexed.
    pub async fn latest_ledger(&self) -> Result<Option<i64>, sqlx::Error> {
        let row: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT MAX(ledger_sequence) FROM contract_events")
                .fetch_optional(&self.db)
                .await?;
        Ok(row.and_then(|(v,)| v))
    }

    /// Delete events older than `keep_ledgers` ledgers behind the latest.
    #[instrument(skip(self))]
    pub async fn prune(&self, keep_ledgers: i64) -> Result<u64, sqlx::Error> {
        let latest = self.latest_ledger().await?.unwrap_or(0);
        let cutoff = latest.saturating_sub(keep_ledgers);
        let result = sqlx::query("DELETE FROM contract_events WHERE ledger_sequence < $1")
            .bind(cutoff)
            .execute(&self.db)
            .await?;
        let deleted = result.rows_affected();
        if deleted > 0 {
            warn!(deleted, cutoff, "Pruned old contract events");
        }
        Ok(deleted)
    }
}

/// A raw event as received from the Stellar network / horizon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEvent {
    pub contract_id: String,
    pub ledger_sequence: i64,
    pub transaction_hash: String,
    pub event_index: i32,
    pub event_type: String,
    pub topics: serde_json::Value,
    pub data: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    fn lazy_pool() -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://postgres:postgres@localhost/crucible_test")
            .unwrap()
    }

    fn make_event(contract_id: &str, ledger: i64, tx: &str, kind: &str) -> RawEvent {
        RawEvent {
            contract_id: contract_id.to_string(),
            ledger_sequence: ledger,
            transaction_hash: tx.to_string(),
            event_index: 0,
            event_type: kind.to_string(),
            topics: serde_json::json!(["transfer"]),
            data: serde_json::json!({"amount": 100}),
        }
    }

    #[tokio::test]
    async fn test_ingest_empty_batch_returns_zero() {
        let indexer = EventIndexer::new(lazy_pool());
        let result = indexer.ingest(vec![]).await.unwrap();
        assert_eq!(result, 0);
    }

    #[tokio::test]
    async fn test_event_query_defaults() {
        let q = EventQuery::default();
        assert!(q.contract_id.is_none());
        assert!(q.limit.is_none());
    }

    #[test]
    fn test_raw_event_serialization() {
        let ev = make_event("CABC", 100, "txhash1", "transfer");
        let json = serde_json::to_string(&ev).unwrap();
        let back: RawEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.contract_id, "CABC");
        assert_eq!(back.ledger_sequence, 100);
        assert_eq!(back.event_index, 0);
    }
}
