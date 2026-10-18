//! Webhook Event Delivery Engine with Cryptographic Signatures
//!
//! Provides asynchronous webhook dispatching, HMAC-SHA256 signing, retry backoff,
//! delivery logging history, and manual re-trigger capability.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tracing::{error, info, warn};
use url::Url;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

/// Webhook endpoint registration model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEndpoint {
    pub id: Uuid,
    pub url: String,
    pub secret: String,
    pub event_types: Vec<String>,
    pub enabled: bool,
}

/// Webhook event payload structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEvent {
    pub id: Uuid,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub timestamp: i64,
}

/// Record of a webhook delivery attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDeliveryLog {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub event_id: Uuid,
    pub attempt: u32,
    pub status_code: Option<u16>,
    pub success: bool,
    pub response_body: Option<String>,
    pub delivered_at: i64,
}

/// Asynchronous webhook dispatcher worker.
#[derive(Debug, Clone)]
pub struct WebhookDispatcherWorker {
    pub max_retries: u32,
    pub base_delay_secs: u64,
}

impl Default for WebhookDispatcherWorker {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_secs: 2,
        }
    }
}

impl WebhookDispatcherWorker {
    pub fn new(max_retries: u32, base_delay_secs: u64) -> Self {
        Self {
            max_retries,
            base_delay_secs,
        }
    }

    /// Sign payload using HMAC-SHA256 with secret key.
    /// Returns formatted signature string: `sha256=<hex_digest>`.
    pub fn sign_payload(secret: &str, payload: &str) -> Result<String, anyhow::Error> {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|e| anyhow::anyhow!("HMAC key error: {}", e))?;
        mac.update(payload.as_bytes());
        let result = mac.finalize();
        let hex_signature = hex::encode(result.into_bytes());
        Ok(format!("sha256={}", hex_signature))
    }

    /// Verify webhook signature using constant-time comparison to prevent timing analysis attacks.
    pub fn verify_signature(secret: &str, payload: &str, expected_signature: &str) -> bool {
        use subtle::ConstantTimeEq;

        let computed = match Self::sign_payload(secret, payload) {
            Ok(sig) => sig,
            Err(_) => return false,
        };

        if computed.len() != expected_signature.len() {
            return false;
        }

        computed.as_bytes().ct_eq(expected_signature.as_bytes()).into()
    }

    /// Calculate exponential retry backoff duration with full jitter for a given attempt.
    pub fn calculate_retry_delay(&self, attempt: u32) -> Duration {
        let factor = 2u64.saturating_pow(attempt.saturating_sub(1));
        let max_delay_ms = self.base_delay_secs.saturating_mul(factor).saturating_mul(1000);
        if max_delay_ms == 0 {
            return Duration::ZERO;
        }

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as u64;
        let mixed = nanos.wrapping_add((attempt as u64).wrapping_mul(6_364_136_223_846_793_005));
        let jitter_ms = mixed % max_delay_ms;

        Duration::from_millis(jitter_ms)
    }

    /// Dispatch a webhook event to an endpoint with HMAC signature.
    pub async fn dispatch_event(
        &self,
        endpoint: &WebhookEndpoint,
        event: &WebhookEvent,
        attempt: u32,
    ) -> WebhookDeliveryLog {
        let serialized_body = serde_json::to_string(&event).unwrap_or_default();
        let signature = Self::sign_payload(&endpoint.secret, &serialized_body)
            .unwrap_or_else(|_| "sha256=invalid".to_string());

        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        headers.insert("X-Crucible-Signature".to_string(), signature);
        headers.insert("X-Crucible-Event-Id".to_string(), event.id.to_string());
        headers.insert("X-Crucible-Event-Type".to_string(), event.event_type.clone());

        info!(
            endpoint_id = %endpoint.id,
            event_id = %event.id,
            attempt = attempt,
            "Dispatching webhook event"
        );

        let (status_code, success, response_body) =
            match tokio::time::timeout(WEBHOOK_TIMEOUT, async {
                let url = validate_webhook_url(&endpoint.url)?;
                let addresses = resolve_public_addresses(&url).await?;
                let host = url
                    .host_str()
                    .ok_or_else(|| anyhow::anyhow!("webhook URL has no host"))?;
                let client = reqwest::Client::builder()
                    .connect_timeout(WEBHOOK_TIMEOUT)
                    .timeout(WEBHOOK_TIMEOUT)
                    .redirect(reqwest::redirect::Policy::none())
                    .resolve_to_addrs(host, &addresses)
                    .build()?;

                send_webhook_request(&client, &url, &headers, &serialized_body).await
            })
            .await
            {
                Ok(Ok((status, success, body))) => (Some(status), success, body),
                Ok(Err(dispatch_error)) => {
                    warn!(
                        endpoint_id = %endpoint.id,
                        event_id = %event.id,
                        attempt = attempt,
                        error = %dispatch_error,
                        "Webhook delivery failed"
                    );
                    (None, false, None)
                }
                Err(_) => {
                    warn!(
                        endpoint_id = %endpoint.id,
                        event_id = %event.id,
                        attempt = attempt,
                        "Webhook delivery timed out"
                    );
                    (None, false, None)
                }
            };

        if let Some(status) = status_code {
            if !success {
                warn!(
                    endpoint_id = %endpoint.id,
                    event_id = %event.id,
                    attempt = attempt,
                    status_code = status,
                    "Webhook endpoint returned a non-success status"
                );
            }
        }

        WebhookDeliveryLog {
            id: Uuid::new_v4(),
            endpoint_id: endpoint.id,
            event_id: event.id,
            attempt,
            status_code,
            success,
            response_body,
            delivered_at: chrono::Utc::now().timestamp(),
        }
    }

    /// Re-trigger a failed webhook delivery manually.
    pub async fn retry_webhook_delivery(
        &self,
        endpoint: &WebhookEndpoint,
        event: &WebhookEvent,
    ) -> WebhookDeliveryLog {
        info!(
            endpoint_id = %endpoint.id,
            event_id = %event.id,
            "Manual webhook re-trigger initiated"
        );
        self.dispatch_event(endpoint, event, 1).await
    }
}

fn validate_webhook_url(raw_url: &str) -> anyhow::Result<Url> {
    let url = Url::parse(raw_url)?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("webhook URL must use HTTP or HTTPS");
    }
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("webhook URL must not contain credentials");
    }
    if url.host_str().is_none() {
        anyhow::bail!("webhook URL has no host");
    }
    Ok(url)
}

async fn resolve_public_addresses(url: &Url) -> anyhow::Result<Vec<SocketAddr>> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("webhook URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("webhook URL has no port"))?;
    let addresses = tokio::net::lookup_host((host, port)).await?;
    let addresses: Vec<_> = addresses.collect();
    if addresses.is_empty() {
        anyhow::bail!("webhook host resolved to no addresses");
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        anyhow::bail!("webhook host resolves to a non-public IP address");
    }
    Ok(addresses)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let address = u32::from(ip);
    let blocked_ranges = [
        (u32::from(Ipv4Addr::new(0, 0, 0, 0)), 8),
        (u32::from(Ipv4Addr::new(10, 0, 0, 0)), 8),
        (u32::from(Ipv4Addr::new(100, 64, 0, 0)), 10),
        (u32::from(Ipv4Addr::new(127, 0, 0, 0)), 8),
        (u32::from(Ipv4Addr::new(169, 254, 0, 0)), 16),
        (u32::from(Ipv4Addr::new(172, 16, 0, 0)), 12),
        (u32::from(Ipv4Addr::new(192, 0, 0, 0)), 24),
        (u32::from(Ipv4Addr::new(192, 0, 2, 0)), 24),
        (u32::from(Ipv4Addr::new(192, 88, 99, 0)), 24),
        (u32::from(Ipv4Addr::new(192, 168, 0, 0)), 16),
        (u32::from(Ipv4Addr::new(198, 18, 0, 0)), 15),
        (u32::from(Ipv4Addr::new(198, 51, 100, 0)), 24),
        (u32::from(Ipv4Addr::new(203, 0, 113, 0)), 24),
        (u32::from(Ipv4Addr::new(224, 0, 0, 0)), 4),
        (u32::from(Ipv4Addr::new(240, 0, 0, 0)), 4),
    ];

    !blocked_ranges.iter().any(|&(network, prefix)| {
        let mask = u32::MAX << (32 - prefix);
        address & mask == network & mask
    })
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    let is_global_unicast = segments[0] & 0xe000 == 0x2000;
    let is_special_2001_range = segments[0] == 0x2001 && segments[1] & 0xff80 == 0;
    let is_documentation = segments[0] == 0x2001 && segments[1] == 0x0db8;
    let is_6to4 = segments[0] == 0x2002;

    is_global_unicast && !is_special_2001_range && !is_documentation && !is_6to4
}

async fn send_webhook_request(
    client: &reqwest::Client,
    url: &Url,
    headers: &HashMap<String, String>,
    body: &str,
) -> Result<(u16, bool, Option<String>), reqwest::Error> {
    let mut request = client.post(url.as_str()).body(body.to_owned());
    for (name, value) in headers {
        request = request.header(name, value);
    }

    let response = request.send().await?;
    let status = response.status();
    let response_body = response.text().await.ok();
    Ok((status.as_u16(), status.is_success(), response_body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_sha256_signature() {
        let secret = "crucible_webhook_secret_key_123";
        let payload = r#"{"event_type":"contract_deployed","contract_id":"0x123"}"#;

        let signature = WebhookDispatcherWorker::sign_payload(secret, payload).unwrap();
        assert!(signature.starts_with("sha256="));
        assert_eq!(signature.len(), 7 + 64); // "sha256=" + 64 hex chars

        // Verify deterministic output
        let signature2 = WebhookDispatcherWorker::sign_payload(secret, payload).unwrap();
        assert_eq!(signature, signature2);
    }

    #[test]
    fn test_retry_backoff_calculation() {
        let worker = WebhookDispatcherWorker::new(5, 2);

        assert!(worker.calculate_retry_delay(1) <= Duration::from_secs(2));
        assert!(worker.calculate_retry_delay(2) <= Duration::from_secs(4));
        assert!(worker.calculate_retry_delay(3) <= Duration::from_secs(8));
        assert!(worker.calculate_retry_delay(4) <= Duration::from_secs(16));
    }

    #[tokio::test]
    async fn test_delivery_log_rejects_loopback_endpoint() {
        let worker = WebhookDispatcherWorker::default();
        let endpoint = WebhookEndpoint {
            id: Uuid::new_v4(),
            url: "http://127.0.0.1/webhook".to_string(),
            secret: "secret".to_string(),
            event_types: vec!["contract_deployed".to_string()],
            enabled: true,
        };
        let event = WebhookEvent {
            id: Uuid::new_v4(),
            event_type: "contract_deployed".to_string(),
            payload: serde_json::json!({ "contract_id": "0xabc" }),
            timestamp: chrono::Utc::now().timestamp(),
        };

        let log = worker.dispatch_event(&endpoint, &event, 1).await;
        assert!(!log.success);
        assert_eq!(log.status_code, None);
        assert_eq!(log.endpoint_id, endpoint.id);
        assert_eq!(log.event_id, event.id);
    }

    #[tokio::test]
    async fn test_dispatch_sends_signed_post_and_records_response() {
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let event = WebhookEvent {
            id: Uuid::new_v4(),
            event_type: "contract_deployed".to_string(),
            payload: serde_json::json!({ "contract_id": "0xabc" }),
            timestamp: chrono::Utc::now().timestamp(),
        };
        let serialized_body = serde_json::to_string(&event).unwrap();
        let signature = WebhookDispatcherWorker::sign_payload("secret", &serialized_body).unwrap();

        Mock::given(method("POST"))
            .and(path("/webhook"))
            .and(header("X-Crucible-Signature", signature))
            .and(header("X-Crucible-Event-Id", event.id.to_string()))
            .and(body_json(serde_json::to_value(&event).unwrap()))
            .respond_with(ResponseTemplate::new(202).set_body_string("accepted"))
            .expect(1)
            .mount(&server)
            .await;

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        headers.insert(
            "X-Crucible-Signature".to_string(),
            WebhookDispatcherWorker::sign_payload("secret", &serialized_body).unwrap(),
        );
        headers.insert("X-Crucible-Event-Id".to_string(), event.id.to_string());
        headers.insert(
            "X-Crucible-Event-Type".to_string(),
            event.event_type.clone(),
        );

        let url = Url::parse(&format!("{}/webhook", server.uri())).unwrap();
        let (status, success, response_body) =
            send_webhook_request(&client, &url, &headers, &serialized_body)
                .await
                .unwrap();

        assert_eq!(status, 202);
        assert!(success);
        assert_eq!(response_body.as_deref(), Some("accepted"));
    }

    #[test]
    fn test_rejects_private_and_non_http_addresses() {
        assert!(!is_public_ip("10.1.2.3".parse().unwrap()));
        assert!(!is_public_ip("100.64.0.1".parse().unwrap()));
        assert!(!is_public_ip("::1".parse().unwrap()));
        assert!(!is_public_ip("fd00::1".parse().unwrap()));
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
        assert!(validate_webhook_url("https://example.com/webhook").is_ok());
        assert!(validate_webhook_url("file:///etc/passwd").is_err());
        assert!(validate_webhook_url("http://user:pass@example.com/").is_err());
    }

    #[tokio::test]
    async fn test_manual_retrigger() {
        let worker = WebhookDispatcherWorker::default();
        let endpoint = WebhookEndpoint {
            id: Uuid::new_v4(),
            url: "http://127.0.0.1/webhook".to_string(),
            secret: "secret".to_string(),
            event_types: vec!["error".to_string()],
            enabled: true,
        };
        let event = WebhookEvent {
            id: Uuid::new_v4(),
            event_type: "error".to_string(),
            payload: serde_json::json!({ "message": "Simulation failed" }),
            timestamp: chrono::Utc::now().timestamp(),
        };

        let log = worker.retry_webhook_delivery(&endpoint, &event).await;
        assert!(!log.success);
        assert_eq!(log.attempt, 1);
    }
}
