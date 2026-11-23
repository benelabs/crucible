//! WebSocket handler for real-time dashboard updates.
//!
//! Clients connect to `/api/v1/ws/dashboard` and receive a JSON push every
//! `PUSH_INTERVAL_SECS` seconds containing the latest dashboard metrics.
//! The connection is kept alive with periodic pings.
//!
//! Outbound message queuing enforces a strict backpressure buffer limit
//! and frame send timeout to protect against slow consumers and stalled TCP ACKs.
//!
//! Handshake upgrade requires authentication via bearer token, session token header,
//! or query parameters (`token`, `session_token`, `access_token`, `jwt`).

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::api::middleware::auth::AdminAuthState;
use crate::services::{
    error_recovery::ErrorManager,
    sys_metrics::{MetricsExporter, SystemMetrics},
};

const PUSH_INTERVAL_SECS: u64 = 5;
const PING_INTERVAL_SECS: u64 = 30;
const PONG_TIMEOUT_SECS: u64 = 60;

/// Default buffer limit for outbound messages per WebSocket client.
pub const DEFAULT_MAX_OUTBOUND_BUFFER: usize = 32;

/// Default timeout allowed for a single frame send before disconnecting a slow consumer.
pub const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(3);

/// Shared state required by the WebSocket handler.
#[derive(Clone)]
pub struct WsState {
    pub metrics_exporter: Arc<MetricsExporter>,
    pub error_manager: Arc<ErrorManager>,
    pub outbound_buffer_limit: usize,
    pub send_timeout: Duration,
    pub auth_state: Option<Arc<AdminAuthState>>,
    pub jwt_secret: Option<String>,
}

impl WsState {
    pub fn new(metrics_exporter: Arc<MetricsExporter>, error_manager: Arc<ErrorManager>) -> Self {
        Self {
            metrics_exporter,
            error_manager,
            outbound_buffer_limit: DEFAULT_MAX_OUTBOUND_BUFFER,
            send_timeout: DEFAULT_SEND_TIMEOUT,
            auth_state: None,
            jwt_secret: None,
        }
    }

    pub fn with_outbound_buffer_limit(mut self, limit: usize) -> Self {
        self.outbound_buffer_limit = limit;
        self
    }

    pub fn with_send_timeout(mut self, timeout: Duration) -> Self {
        self.send_timeout = timeout;
        self
    }

    pub fn with_auth_state(mut self, auth_state: Arc<AdminAuthState>) -> Self {
        self.auth_state = Some(auth_state);
        self
    }

    pub fn with_jwt_secret(mut self, secret: impl Into<String>) -> Self {
        self.jwt_secret = Some(secret.into());
        self
    }

    /// Check whether a candidate token is valid.
    pub async fn is_token_valid(&self, token: &str) -> bool {
        let token = token.trim();
        if token.is_empty() {
            return false;
        }

        if let Some(auth) = &self.auth_state {
            if auth.is_token_valid(token).await {
                return true;
            }
        }

        if let Some(secret) = self.jwt_secret.as_deref() {
            if crate::api::middleware::auth::verify_hs256_jwt(token, secret, None, None).is_some() {
                return true;
            }
        }

        // Fallback to environment variables
        if let Ok(admin_token) = std::env::var("ADMIN_API_TOKEN") {
            if !admin_token.trim().is_empty() && token == admin_token.trim() {
                return true;
            }
        }
        if let Ok(op_token) = std::env::var("OPERATOR_API_TOKEN") {
            if !op_token.trim().is_empty() && token == op_token.trim() {
                return true;
            }
        }
        if let Ok(ws_token) = std::env::var("WS_AUTH_TOKEN") {
            if !ws_token.trim().is_empty() && token == ws_token.trim() {
                return true;
            }
        }
        if let Ok(secret) = std::env::var("JWT_SECRET") {
            if !secret.trim().is_empty()
                && crate::api::middleware::auth::verify_hs256_jwt(token, secret.trim(), None, None)
                    .is_some()
            {
                return true;
            }
        }

        false
    }
}

/// The payload pushed to each connected client.
#[derive(Debug, Serialize, Deserialize)]
pub struct DashboardUpdate {
    pub r#type: String,
    pub metrics: SystemMetrics,
    pub active_errors: usize,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Extract authentication token from headers or query parameters.
pub fn extract_ws_token(headers: &HeaderMap, query: &HashMap<String, String>) -> Option<String> {
    // 1. Authorization: Bearer <token>
    if let Some(auth_val) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = auth_val.trim();
        if let Some(stripped) = trimmed
            .strip_prefix("Bearer ")
            .or_else(|| trimmed.strip_prefix("bearer "))
        {
            let t = stripped.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        } else if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    // 2. x-session-token header
    if let Some(session_val) = headers.get("x-session-token").and_then(|v| v.to_str().ok()) {
        let t = session_val.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }

    // 3. sec-websocket-protocol header
    if let Some(proto_val) = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
    {
        for part in proto_val.split(',') {
            let t = part.trim();
            if !t.is_empty() && !t.eq_ignore_ascii_case("websocket") {
                return Some(t.to_string());
            }
        }
    }

    // 4. Query parameters: token, session_token, access_token, jwt
    for key in &["token", "session_token", "access_token", "jwt"] {
        if let Some(val) = query.get(*key) {
            let t = val.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }

    None
}

/// Axum handler — validates auth credentials and upgrades HTTP connection to WebSocket.
pub async fn ws_dashboard_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(state): State<Arc<WsState>>,
) -> Response {
    let token = extract_ws_token(&headers, &query);
    let Some(token) = token else {
        warn!("WebSocket handshake rejected: missing authorization token");
        return crate::api::errors::make_error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Authentication required: provide a Bearer token or token query parameter",
        );
    };

    if !state.is_token_valid(&token).await {
        warn!("WebSocket handshake rejected: invalid or expired authentication token");
        return crate::api::errors::make_error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid authentication credentials",
        );
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<WsState>) {
    crate::services::sys_metrics::record_websocket_conn_change(1);
    let (mut sender, mut receiver) = socket.split();

    info!("WebSocket client connected for dashboard updates");

    let buffer_size = if state.outbound_buffer_limit > 0 {
        state.outbound_buffer_limit
    } else {
        DEFAULT_MAX_OUTBOUND_BUFFER
    };
    let send_timeout = state.send_timeout;

    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel::<Message>(buffer_size);

    let mut push_ticker = interval(Duration::from_secs(PUSH_INTERVAL_SECS));
    push_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ping_ticker = interval(Duration::from_secs(PING_INTERVAL_SECS));
    ping_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut last_pong_received = Instant::now();

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
                        match outbound_tx.try_send(Message::Text(json.into())) {
                            Ok(()) => {}
                            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                warn!(
                                    buffer_capacity = buffer_size,
                                    "WebSocket client dropped: outbound queue buffer limit exceeded (slow consumer)"
                                );
                                break;
                            }
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                debug!("WebSocket outbound channel closed");
                                break;
                            }
                        }
                    }
                    Err(e) => warn!(error = %e, "Failed to serialize dashboard update"),
                }
            }

            _ = ping_ticker.tick() => {
                // Enforce heartbeat deadline: terminate zombie connections that stop answering pings.
                if last_pong_received.elapsed() > Duration::from_secs(PONG_TIMEOUT_SECS) {
                    warn!(
                        elapsed_secs = last_pong_received.elapsed().as_secs(),
                        timeout_secs = PONG_TIMEOUT_SECS,
                        "WebSocket client heartbeat deadline exceeded (missing pong), tearing down zombie connection"
                    );
                    break;
                }

                match outbound_tx.try_send(Message::Ping(vec![].into())) {
                    Ok(()) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        warn!(
                            buffer_capacity = buffer_size,
                            "WebSocket client dropped: cannot queue ping, buffer full (slow consumer)"
                        );
                        break;
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        break;
                    }
                }
            }

            outbound_msg = outbound_rx.recv() => {
                match outbound_msg {
                    Some(msg) => {
                        match tokio::time::timeout(send_timeout, sender.send(msg)).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => {
                                debug!("WebSocket client disconnected (send failed), dropping stream");
                                break;
                            }
                            Err(_) => {
                                warn!(
                                    timeout_secs = send_timeout.as_secs(),
                                    "WebSocket client dropped: send timed out (slow consumer stalled TCP ACKs)"
                                );
                                break;
                            }
                        }
                    }
                    None => {
                        debug!("Outbound queue terminated, closing socket");
                        break;
                    }
                }
            }

            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        info!("WebSocket client explicitly disconnected or channel closed");
                        break;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        debug!("Received pong from WebSocket client");
                        last_pong_received = Instant::now();
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "WebSocket receive error, tearing down connection");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    // Teardown and decrement connection count immediately upon loop termination
    crate::services::sys_metrics::record_websocket_conn_change(-1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::middleware::auth::Principal;
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    fn create_test_jwt(secret: &str, sub: &str, exp: i64) -> String {
        let header = "{\"alg\":\"HS256\",\"typ\":\"JWT\"}";
        let payload = serde_json::json!({
            "sub": sub,
            "role": "admin",
            "exp": exp,
            "jti": "ws-test-jti"
        })
        .to_string();

        let b64 = |b: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        let signing_input = format!("{}.{}", b64(header.as_bytes()), b64(payload.as_bytes()));

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signing_input.as_bytes());
        let signature = b64(&mac.finalize().into_bytes());

        format!("{signing_input}.{signature}")
    }

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

    #[test]
    fn test_ws_state_outbound_buffer_configuration() {
        let metrics = Arc::new(MetricsExporter::new());
        let error_manager = Arc::new(ErrorManager::new());
        let state = WsState::new(metrics.clone(), error_manager.clone());

        assert_eq!(state.outbound_buffer_limit, DEFAULT_MAX_OUTBOUND_BUFFER);
        assert_eq!(state.send_timeout, DEFAULT_SEND_TIMEOUT);

        let custom_state = state
            .with_outbound_buffer_limit(16)
            .with_send_timeout(Duration::from_millis(500));
        assert_eq!(custom_state.outbound_buffer_limit, 16);
        assert_eq!(custom_state.send_timeout, Duration::from_millis(500));
    }

    #[test]
    fn test_slow_consumer_buffer_overflow_detection() {
        let buffer_size = 2;
        let (tx, _rx) = tokio::sync::mpsc::channel::<Message>(buffer_size);

        assert!(tx.try_send(Message::Text("msg1".into())).is_ok());
        assert!(tx.try_send(Message::Text("msg2".into())).is_ok());

        let overflow_res = tx.try_send(Message::Text("msg3".into()));
        assert!(matches!(
            overflow_res,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_))
        ));
    }

    #[test]
    fn test_extract_ws_token_from_authorization_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer secret_token_123".parse().unwrap(),
        );
        let query = HashMap::new();

        let token = extract_ws_token(&headers, &query);
        assert_eq!(token.as_deref(), Some("secret_token_123"));
    }

    #[test]
    fn test_extract_ws_token_from_query_params() {
        let headers = HeaderMap::new();
        let mut query = HashMap::new();
        query.insert("token".to_string(), "query_token_abc".to_string());

        let token = extract_ws_token(&headers, &query);
        assert_eq!(token.as_deref(), Some("query_token_abc"));

        let mut session_query = HashMap::new();
        session_query.insert("session_token".to_string(), "sess_xyz".to_string());
        assert_eq!(
            extract_ws_token(&headers, &session_query).as_deref(),
            Some("sess_xyz")
        );
    }

    #[test]
    fn test_extract_ws_token_from_custom_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-session-token", "custom_session_456".parse().unwrap());
        let query = HashMap::new();

        assert_eq!(
            extract_ws_token(&headers, &query).as_deref(),
            Some("custom_session_456")
        );

        let mut proto_headers = HeaderMap::new();
        proto_headers.insert(
            "sec-websocket-protocol",
            "websocket, proto_token_789".parse().unwrap(),
        );
        assert_eq!(
            extract_ws_token(&proto_headers, &query).as_deref(),
            Some("proto_token_789")
        );
    }

    #[tokio::test]
    async fn test_ws_state_validates_static_token() {
        let metrics = Arc::new(MetricsExporter::new());
        let error_manager = Arc::new(ErrorManager::new());
        let mut auth = AdminAuthState::new();
        auth.insert_token("valid_static_key", Principal::admin(1, "admin"));

        let state = WsState::new(metrics, error_manager).with_auth_state(Arc::new(auth));

        assert!(state.is_token_valid("valid_static_key").await);
        assert!(!state.is_token_valid("wrong_key").await);
    }

    #[tokio::test]
    async fn test_ws_state_validates_jwt() {
        let metrics = Arc::new(MetricsExporter::new());
        let error_manager = Arc::new(ErrorManager::new());
        let secret = "super-secret-jwt-key";
        let state = WsState::new(metrics, error_manager).with_jwt_secret(secret);

        let future_exp = chrono::Utc::now().timestamp() + 3600;
        let valid_jwt = create_test_jwt(secret, "admin-user", future_exp);
        assert!(state.is_token_valid(&valid_jwt).await);

        let past_exp = chrono::Utc::now().timestamp() - 3600;
        let expired_jwt = create_test_jwt(secret, "admin-user", past_exp);
        assert!(!state.is_token_valid(&expired_jwt).await);

        let tampered_jwt = format!("{}.tampered", &valid_jwt[..valid_jwt.rfind('.').unwrap()]);
        assert!(!state.is_token_valid(&tampered_jwt).await);
    }

    #[test]
    fn test_pong_timeout_configuration() {
        const { assert!(PONG_TIMEOUT_SECS > PING_INTERVAL_SECS) };
    }

    #[test]
    fn test_pong_heartbeat_deadline_detection() {
        let last_pong = Instant::now() - Duration::from_secs(PONG_TIMEOUT_SECS + 5);
        assert!(last_pong.elapsed() > Duration::from_secs(PONG_TIMEOUT_SECS));

        let recent_pong = Instant::now();
        assert!(recent_pong.elapsed() <= Duration::from_secs(PONG_TIMEOUT_SECS));
    }
}
