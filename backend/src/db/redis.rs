//! Redis connection manager setup with timeout and retry strategy.

use crate::config::redis::RedisConfig;
use redis::Client;

/// Establish a connection manager with exponential backoff and timeout logic.
pub async fn connect_with_retry(
    client: &Client,
    config: &RedisConfig,
) -> Result<redis::aio::ConnectionManager, redis::RedisError> {
    config.connect_manager_with_retry(client).await
}

/// Role of a Redis node in replication topology.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RedisNodeRole {
    Master,
    Slave,
    Unknown(String),
}

/// Detailed health status of a Redis node inspected during readiness checks.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RedisHealthReport {
    pub is_healthy: bool,
    pub is_loading: bool,
    pub role: RedisNodeRole,
    pub is_read_only: bool,
    pub details: Option<String>,
}

/// Parses Redis `INFO` command string output for replication and persistence fields.
pub fn parse_redis_info(info_str: &str) -> (bool, RedisNodeRole, bool) {
    let mut is_loading = false;
    let mut role = RedisNodeRole::Master;
    let mut is_read_only = false;

    for line in info_str.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            match k.trim() {
                "loading" => {
                    is_loading = v.trim() == "1";
                }
                "role" => {
                    let r = v.trim();
                    if r == "slave" || r == "replica" {
                        role = RedisNodeRole::Slave;
                        is_read_only = true;
                    } else if r == "master" {
                        role = RedisNodeRole::Master;
                    } else {
                        role = RedisNodeRole::Unknown(r.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    (is_loading, role, is_read_only)
}

/// Evaluates node health report based on parsed info flags and readiness requirements.
pub fn evaluate_redis_readiness(info_str: &str, require_write: bool) -> RedisHealthReport {
    let (is_loading, role, is_read_only) = parse_redis_info(info_str);

    if is_loading {
        return RedisHealthReport {
            is_healthy: false,
            is_loading: true,
            role,
            is_read_only,
            details: Some("Redis node is in loading state during cluster recovery".to_string()),
        };
    }

    if require_write && is_read_only {
        return RedisHealthReport {
            is_healthy: false,
            is_loading: false,
            role,
            is_read_only: true,
            details: Some(
                "Redis node is in read-only replica mode; write operations unavailable".to_string(),
            ),
        };
    }

    RedisHealthReport {
        is_healthy: true,
        is_loading: false,
        role,
        is_read_only,
        details: None,
    }
}

/// Checks whether the Redis connection is ready and capable of accepting traffic.
///
/// Unlike a bare `PING`, this probe verifies that the node:
/// 1. Responds to commands (basic liveness via PING).
/// 2. Is not currently loading dataset into memory (`loading:0` in `INFO persistence`).
/// 3. Is not in an unexpected read-only replica state when write capability is required.
pub async fn check_redis_readiness(
    conn: &mut redis::aio::ConnectionManager,
    require_write: bool,
) -> Result<RedisHealthReport, redis::RedisError> {
    let _: String = redis::cmd("PING").query_async(conn).await?;

    let info_str: String = redis::cmd("INFO")
        .query_async(conn)
        .await
        .unwrap_or_default();

    let report = evaluate_redis_readiness(&info_str, require_write);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_healthy_master_node() {
        let info = r#"
# Server
redis_version:7.0.0
# Replication
role:master
connected_slaves:2
# Persistence
loading:0
rdb_last_bgsave_status:ok
"#;
        let report = evaluate_redis_readiness(info, true);
        assert!(report.is_healthy);
        assert!(!report.is_loading);
        assert_eq!(report.role, RedisNodeRole::Master);
        assert!(!report.is_read_only);
        assert!(report.details.is_none());
    }

    #[test]
    fn test_rejects_node_in_loading_state() {
        let info = r#"
# Replication
role:master
# Persistence
loading:1
loading_total_bytes:10485760
"#;
        let report = evaluate_redis_readiness(info, false);
        assert!(!report.is_healthy);
        assert!(report.is_loading);
        assert!(report.details.unwrap().contains("loading state"));
    }

    #[test]
    fn test_rejects_read_only_replica_when_write_required() {
        let info = r#"
# Replication
role:slave
master_host:127.0.0.1
master_port:6379
# Persistence
loading:0
"#;
        let report = evaluate_redis_readiness(info, true);
        assert!(!report.is_healthy);
        assert_eq!(report.role, RedisNodeRole::Slave);
        assert!(report.is_read_only);
        assert!(report.details.unwrap().contains("read-only replica mode"));
    }

    #[test]
    fn test_allows_replica_when_write_not_required() {
        let info = r#"
# Replication
role:replica
# Persistence
loading:0
"#;
        let report = evaluate_redis_readiness(info, false);
        assert!(report.is_healthy);
        assert_eq!(report.role, RedisNodeRole::Slave);
        assert!(report.is_read_only);
        assert!(report.details.is_none());
    }

    #[test]
    fn test_parses_unknown_role() {
        let info = "role:sentinel\nloading:0";
        let (loading, role, read_only) = parse_redis_info(info);
        assert!(!loading);
        assert_eq!(role, RedisNodeRole::Unknown("sentinel".to_string()));
        assert!(!read_only);
    }
}
