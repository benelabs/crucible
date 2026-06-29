use crate::error::AppError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{info, instrument};

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ContractCallLog {
    pub id: Option<i32>,
    pub contract_id: String,
    pub function_name: String,
    pub arguments: serde_json::Value,
    pub caller: Option<String>,
    pub status: String,
    pub gas_used: f64,
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone)]
pub struct ContractCallLogger {
    pub db: PgPool,
}

impl ContractCallLogger {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    #[instrument(skip(self))]
    pub async fn log_call(&self, log: ContractCallLog) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO contract_call_logs (contract_id, function_name, arguments, caller, status, gas_used, timestamp) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        )
        .bind(&log.contract_id)
        .bind(&log.function_name)
        .bind(&log.arguments)
        .bind(&log.caller)
        .bind(&log.status)
        .bind(&log.gas_used)
        .bind(log.timestamp.unwrap_or_else(chrono::Utc::now))
        .execute(&self.db)
        .await
        .map_err(AppError::db)?;

        info!(contract_id = %log.contract_id, function_name = %log.function_name, "Contract call logged");
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn get_logs(
        &self,
        contract_id: Option<String>,
        limit: i64,
    ) -> Result<Vec<ContractCallLog>, AppError> {
        let logs = if let Some(cid) = contract_id {
            sqlx::query_as::<_, ContractCallLog>(
                "SELECT id, contract_id, function_name, arguments, caller, status, gas_used, timestamp \
                 FROM contract_call_logs \
                 WHERE contract_id = $1 \
                 ORDER BY timestamp DESC \
                 LIMIT $2"
            )
            .bind(cid)
            .bind(limit)
            .fetch_all(&self.db)
            .await
            .map_err(AppError::db)?
        } else {
            sqlx::query_as::<_, ContractCallLog>(
                "SELECT id, contract_id, function_name, arguments, caller, status, gas_used, timestamp \
                 FROM contract_call_logs \
                 ORDER BY timestamp DESC \
                 LIMIT $1"
            )
            .bind(limit)
            .fetch_all(&self.db)
            .await
            .map_err(AppError::db)?
        };

        Ok(logs)
    }
}
