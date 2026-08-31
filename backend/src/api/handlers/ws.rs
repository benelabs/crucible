//! WebSocket handler for real-time dashboard updates.
//!
//! Clients connect to `/api/v1/ws/dashboard` and receive a JSON push every
//! `PUSH_INTERVAL_SECS` seconds containing the latest dashboard metrics.
//! The connection is kept alive with periodic pings.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::services::{
    error_recovery::ErrorManager,
    sys_metrics::{MetricsExporter, SystemMetrics},
};

const PUSH_INTERVAL_SECS: u64 = 5;
const PING_INTERVAL_SECS: u64 = 30;

/// Shared state required by the WebSocket handler.
#[derive(Clone)]
pub struct WsState {
    pub metrics_exporter: Arc<MetricsExporter>,
    pub error_manager: Arc<ErrorManager>,
}

/// The payload pushed to each connected client.
#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardUpdate {
    pub r#type: String,
    pub metrics: SystemMetrics,
    pub active_errors: usize,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Axum handler — upgrades the HTTP connection to WebSocket.
pub async fn ws_dashboard_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WsState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<WsState>) {
    crate::services::sys_metrics::record_websocket_conn_change(1);
    let (mut sender, mut receiver) = socket.split();

    info!("WebSocket client connected for dashboard updates");

    let mut push_ticker = interval(Duration::from_secs(PUSH_INTERVAL_SECS));
    let mut ping_ticker = interval(Duration::from_secs(PING_INTERVAL_SECS));

    loop {
        tokio::select! {
            _ = push_ticker.tick() => {
                let metrics = state.metrics_exporter.get_metrics().await;
                let active_errors = state.error_manager.get_active_tasks().await.len();

                let update = DashboardUpdate {
                    r#type: "dashboard_update".to_string(),
                    metrics,
                    active_errors,
                    timestamp: chrono::Utc::now(),
                };

                match serde_json::to_string(&update) {
                    Ok(json) => {
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            debug!("WebSocket client disconnected (send failed)");
                            break;
                        }
                    }
                    Err(e) => warn!(error = %e, "Failed to serialize dashboard update"),
                }
            }

            _ = ping_ticker.tick() => {
                if sender.send(Message::Ping(vec![].into())).await.is_err() {
                    debug!("WebSocket client disconnected (ping failed)");
                    break;
                }
            }

            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        info!("WebSocket client disconnected");
                        break;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        debug!("Received pong from WebSocket client");
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "WebSocket receive error");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    crate::services::sys_metrics::record_websocket_conn_change(-1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_update_serialization() {
        let metrics = SystemMetrics::default();
        let update = DashboardUpdate {
            r#type: "dashboard_update".to_string(),
            metrics,
            active_errors: 2,
            timestamp: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&update).unwrap();
        assert!(json.contains("dashboard_update"));
        assert!(json.contains("active_errors"));
    }
}
