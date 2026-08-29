#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseRouteRole {
    Primary,
    Replica,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseRouteHint {
    pub role: DatabaseRouteRole,
    pub endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationDryRunReport {
    pub migration_name: String,
    pub statements_checked: usize,
    pub rollback_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsSessionSnapshot {
    pub session_id: String,
    pub heartbeat_interval_secs: u64,
    pub can_resume: bool,
}

pub fn default_database_routes() -> Vec<DatabaseRouteHint> {
    vec![
        DatabaseRouteHint {
            role: DatabaseRouteRole::Primary,
            endpoint: "postgres://writer.internal".to_string(),
        },
        DatabaseRouteHint {
            role: DatabaseRouteRole::Replica,
            endpoint: "postgres://reader.internal".to_string(),
        },
    ]
}

pub fn dry_run_report(name: impl Into<String>, statements_checked: usize) -> MigrationDryRunReport {
    MigrationDryRunReport {
        migration_name: name.into(),
        statements_checked,
        rollback_supported: true,
    }
}

pub fn resumable_session(session_id: impl Into<String>) -> WsSessionSnapshot {
    WsSessionSnapshot {
        session_id: session_id.into(),
        heartbeat_interval_secs: 30,
        can_resume: true,
    }
}
