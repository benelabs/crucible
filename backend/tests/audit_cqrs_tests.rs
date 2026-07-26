use backend::services::audit::{AuditDomainEvent, AuditEventRequest};
use serde_json::json;
use uuid::Uuid;

#[test]
fn test_audit_domain_event_serialization() {
    let event_id = Uuid::new_v4();
    let event = AuditDomainEvent {
        event_id,
        aggregate_id: "user-123".to_string(),
        event_type: "USER_LOGIN".to_string(),
        user_id: Some("user-123".to_string()),
        details: json!({"ip": "127.0.0.1"}),
        sequence_number: 1,
        timestamp: chrono::Utc::now(),
    };

    let serialized = serde_json::to_string(&event).expect("Should serialize domain event");
    let deserialized: AuditDomainEvent = serde_json::from_str(&serialized).expect("Should deserialize domain event");

    assert_eq!(deserialized.event_id, event_id);
    assert_eq!(deserialized.event_type, "USER_LOGIN");
}

#[test]
fn test_audit_event_request_structure() {
    let req = AuditEventRequest {
        aggregate_id: Some("contract-456".to_string()),
        event_type: "CONTRACT_DEPLOYED".to_string(),
        user_id: Some("deployer-789".to_string()),
        details: json!({"tx": "0xabc"}),
    };

    assert_eq!(req.event_type, "CONTRACT_DEPLOYED");
}
