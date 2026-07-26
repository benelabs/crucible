use backend::services::audit::{AuditDomainEvent, AuditEventRequest, compute_hash};
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
        hash: "abc123".to_string(),
        previous_hash: "".to_string(),
    };

    let serialized = serde_json::to_string(&event).expect("Should serialize domain event");
    let deserialized: AuditDomainEvent = serde_json::from_str(&serialized).expect("Should deserialize domain event");

    assert_eq!(deserialized.event_id, event_id);
    assert_eq!(deserialized.event_type, "USER_LOGIN");
    assert_eq!(deserialized.hash, "abc123");
    assert_eq!(deserialized.previous_hash, "");
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

#[test]
fn test_compute_hash_genesis_entry() {
    let event = AuditDomainEvent {
        event_id: Uuid::new_v4(),
        aggregate_id: "genesis".to_string(),
        event_type: "GENESIS".to_string(),
        user_id: None,
        details: json!({"init": true}),
        sequence_number: 1,
        timestamp: chrono::Utc::now(),
        hash: String::new(),
        previous_hash: String::new(),
    };

    let hash = compute_hash("", &event);
    // Genesis hash should be a 64-char hex string (SHA-256).
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_compute_hash_chaining() {
    let timestamp = chrono::Utc::now();

    let event1 = AuditDomainEvent {
        event_id: Uuid::new_v4(),
        aggregate_id: "a1".to_string(),
        event_type: "ACTION_A".to_string(),
        user_id: Some("user1".to_string()),
        details: json!({"step": 1}),
        sequence_number: 1,
        timestamp,
        hash: String::new(),
        previous_hash: String::new(),
    };

    let hash1 = compute_hash("", &event1);
    assert_eq!(hash1.len(), 64);

    let event2 = AuditDomainEvent {
        event_id: Uuid::new_v4(),
        aggregate_id: "a2".to_string(),
        event_type: "ACTION_B".to_string(),
        user_id: Some("user2".to_string()),
        details: json!({"step": 2}),
        sequence_number: 2,
        timestamp,
        hash: String::new(),
        previous_hash: hash1.clone(),
    };

    let hash2 = compute_hash(&hash1, &event2);
    assert_eq!(hash2.len(), 64);
    // hash2 must differ from hash1.
    assert_ne!(hash2, hash1);
}

#[test]
fn test_compute_hash_chain_deterministic() {
    let timestamp = chrono::Utc::now();

    let event = AuditDomainEvent {
        event_id: Uuid::new_v4(),
        aggregate_id: "det".to_string(),
        event_type: "DETERMINISTIC".to_string(),
        user_id: None,
        details: json!({"val": 42}),
        sequence_number: 1,
        timestamp,
        hash: String::new(),
        previous_hash: "prevhash123".to_string(),
    };

    let hash_a = compute_hash("prevhash123", &event);
    let hash_b = compute_hash("prevhash123", &event);

    // Same inputs must produce the same hash.
    assert_eq!(hash_a, hash_b);
}

#[test]
fn test_compute_hash_different_prev_hash_changes_result() {
    let timestamp = chrono::Utc::now();

    let event = AuditDomainEvent {
        event_id: Uuid::new_v4(),
        aggregate_id: "diff".to_string(),
        event_type: "DIFF".to_string(),
        user_id: None,
        details: json!({"x": 1}),
        sequence_number: 1,
        timestamp,
        hash: String::new(),
        previous_hash: String::new(),
    };

    let hash1 = compute_hash("aaaa", &event);
    let hash2 = compute_hash("bbbb", &event);

    // Different previous_hash must yield different chain hash.
    assert_ne!(hash1, hash2);
}
