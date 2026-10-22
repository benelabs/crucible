use backend::services::audit::{compute_hash, AuditDomainEvent, AuditEventRequest};
use chrono::{DateTime, TimeZone, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

/// Fixed timestamp makes hash-related tests reproducible.
fn test_timestamp() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 15, 12, 30, 45)
        .single()
        .expect("Test timestamp should be valid")
}

/// Creates a consistent audit event for testing.
fn make_event(
    event_id: Uuid,
    aggregate_id: &str,
    event_type: &str,
    user_id: Option<&str>,
    details: Value,
    sequence_number: u64,
    previous_hash: &str,
) -> AuditDomainEvent {
    AuditDomainEvent {
        event_id,
        aggregate_id: aggregate_id.to_string(),
        event_type: event_type.to_string(),
        user_id: user_id.map(str::to_string),
        details,
        sequence_number,
        timestamp: test_timestamp(),
        hash: String::new(),
        previous_hash: previous_hash.to_string(),
    }
}

#[test]
fn test_audit_domain_event_serialization_round_trip() {
    let event_id = Uuid::from_u128(1);

    let event = AuditDomainEvent {
        event_id,
        aggregate_id: "user-123".to_string(),
        event_type: "USER_LOGIN".to_string(),
        user_id: Some("user-123".to_string()),
        details: json!({"ip": "127.0.0.1"}),
        sequence_number: 1,
        timestamp: test_timestamp(),
        hash: "abc123".to_string(),
        previous_hash: String::new(),
    };

    let serialized =
        serde_json::to_string(&event).expect("Domain event should serialize");

    let deserialized: AuditDomainEvent =
        serde_json::from_str(&serialized).expect("Domain event should deserialize");

    assert_eq!(deserialized.event_id, event.event_id);
    assert_eq!(deserialized.aggregate_id, event.aggregate_id);
    assert_eq!(deserialized.event_type, event.event_type);
    assert_eq!(deserialized.user_id, event.user_id);
    assert_eq!(deserialized.details, event.details);
    assert_eq!(deserialized.sequence_number, event.sequence_number);
    assert_eq!(deserialized.timestamp, event.timestamp);
    assert_eq!(deserialized.hash, event.hash);
    assert_eq!(deserialized.previous_hash, event.previous_hash);
}

#[test]
fn test_audit_event_request_structure() {
    let request = AuditEventRequest {
        aggregate_id: Some("contract-456".to_string()),
        event_type: "CONTRACT_DEPLOYED".to_string(),
        user_id: Some("deployer-789".to_string()),
        details: json!({"tx": "0xabc"}),
    };

    assert_eq!(request.aggregate_id.as_deref(), Some("contract-456"));
    assert_eq!(request.event_type, "CONTRACT_DEPLOYED");
    assert_eq!(request.user_id.as_deref(), Some("deployer-789"));
    assert_eq!(request.details, json!({"tx": "0xabc"}));
}

#[test]
fn test_audit_event_request_supports_optional_fields() {
    let request = AuditEventRequest {
        aggregate_id: None,
        event_type: "SYSTEM_EVENT".to_string(),
        user_id: None,
        details: json!({}),
    };

    assert_eq!(request.aggregate_id, None);
    assert_eq!(request.user_id, None);
    assert_eq!(request.event_type, "SYSTEM_EVENT");
    assert_eq!(request.details, json!({}));
}

#[test]
fn test_compute_hash_genesis_entry() {
    let event = make_event(
        Uuid::from_u128(1),
        "genesis",
        "GENESIS",
        None,
        json!({"init": true}),
        1,
        "",
    );

    let hash = compute_hash("", &event);

    // SHA-256 hashes must contain exactly 64 hexadecimal characters.
    assert_eq!(hash.len(), 64);
    assert!(
        hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Hash must be hexadecimal"
    );
}

#[test]
fn test_compute_hash_is_deterministic() {
    let event = make_event(
        Uuid::from_u128(2),
        "deterministic",
        "DETERMINISTIC",
        Some("user-1"),
        json!({"value": 42}),
        1,
        "previous-hash",
    );

    let first = compute_hash("previous-hash", &event);
    let second = compute_hash("previous-hash", &event);

    assert_eq!(first, second);
}

#[test]
fn test_compute_hash_changes_when_previous_hash_changes() {
    let event = make_event(
        Uuid::from_u128(3),
        "aggregate-1",
        "UPDATE",
        None,
        json!({"value": 1}),
        1,
        "",
    );

    let first = compute_hash("previous-a", &event);
    let second = compute_hash("previous-b", &event);

    assert_ne!(first, second);
}

#[test]
fn test_compute_hash_changes_when_event_content_changes() {
    let event_a = make_event(
        Uuid::from_u128(4),
        "aggregate-1",
        "BALANCE_UPDATED",
        Some("user-1"),
        json!({"balance": 100}),
        1,
        "",
    );

    let mut event_b = event_a.clone();
    event_b.details = json!({"balance": 200});

    assert_ne!(
        compute_hash("", &event_a),
        compute_hash("", &event_b),
        "Changing event details should change the hash"
    );
}

#[test]
fn test_compute_hash_changes_when_sequence_number_changes() {
    let event_a = make_event(
        Uuid::from_u128(5),
        "aggregate-1",
        "TRANSFER",
        None,
        json!({"amount": 10}),
        1,
        "",
    );

    let mut event_b = event_a.clone();
    event_b.sequence_number = 2;

    assert_ne!(
        compute_hash("", &event_a),
        compute_hash("", &event_b),
        "Changing the sequence number should change the hash"
    );
}

#[test]
fn test_compute_hash_changes_when_event_identity_changes() {
    let event_a = make_event(
        Uuid::from_u128(6),
        "aggregate-1",
        "USER_LOGIN",
        None,
        json!({}),
        1,
        "",
    );

    let mut event_b = event_a.clone();
    event_b.event_id = Uuid::from_u128(7);

    assert_ne!(
        compute_hash("", &event_a),
        compute_hash("", &event_b),
        "Changing the event ID should change the hash"
    );
}

#[test]
fn test_compute_hash_changes_when_timestamp_changes() {
    let event_a = make_event(
        Uuid::from_u128(8),
        "aggregate-1",
        "USER_LOGIN",
        None,
        json!({}),
        1,
        "",
    );

    let mut event_b = event_a.clone();
    event_b.timestamp = Utc
        .with_ymd_and_hms(2026, 1, 15, 12, 31, 45)
        .single()
        .expect("Test timestamp should be valid");

    assert_ne!(
        compute_hash("", &event_a),
        compute_hash("", &event_b),
        "Changing the timestamp should change the hash"
    );
}

#[test]
fn test_compute_hash_chain_links_consecutive_events() {
    let event_1 = make_event(
        Uuid::from_u128(9),
        "aggregate-1",
        "CREATED",
        Some("user-1"),
        json!({"step": 1}),
        1,
        "",
    );

    let hash_1 = compute_hash("", &event_1);

    let event_2 = make_event(
        Uuid::from_u128(10),
        "aggregate-1",
        "UPDATED",
        Some("user-1"),
        json!({"step": 2}),
        2,
        &hash_1,
    );

    let hash_2 = compute_hash(&hash_1, &event_2);

    assert_eq!(hash_1.len(), 64);
    assert_eq!(hash_2.len(), 64);
    assert_ne!(hash_1, hash_2);
    assert_eq!(event_2.previous_hash, hash_1);
}

#[test]
fn test_compute_hash_chain_is_sensitive_to_event_order() {
    let event_a = make_event(
        Uuid::from_u128(11),
        "aggregate-1",
        "ACTION_A",
        None,
        json!({"step": 1}),
        1,
        "",
    );

    let event_b = make_event(
        Uuid::from_u128(12),
        "aggregate-1",
        "ACTION_B",
        None,
        json!({"step": 2}),
        2,
        "",
    );

    let hash_a = compute_hash("", &event_a);
    let hash_b_after_a = compute_hash(&hash_a, &event_b);

    let hash_b = compute_hash("", &event_b);
    let hash_a_after_b = compute_hash(&hash_b, &event_a);

    assert_ne!(
        hash_b_after_a, hash_a_after_b,
        "Changing event order should change the chain hashes"
    );
}

#[test]
fn test_compute_hash_ignores_stored_hash_fields_if_not_part_of_payload() {
    let event = make_event(
        Uuid::from_u128(13),
        "aggregate-1",
        "HASH_TEST",
        None,
        json!({"value": true}),
        1,
        "",
    );

    let mut modified = event.clone();
    modified.hash = "tampered-stored-hash".to_string();
    modified.previous_hash = "different-previous-hash".to_string();

    // The explicit previous-hash argument is the chain input. The stored
    // hash field should not recursively hash itself.
    assert_eq!(
        compute_hash("fixed-previous-hash", &event),
        compute_hash("fixed-previous-hash", &modified)
    );
}
