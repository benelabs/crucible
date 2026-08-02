use backend::workers::dlq::DeadLetterJob;
use serde_json::json;

#[test]
fn test_dead_letter_job_serialization() {
    let now = chrono::Utc::now();
    let job = DeadLetterJob {
        id: "job-999".to_string(),
        job_name: "CONTRACT_VERIFY".to_string(),
        payload: json!({"contract_id": "C123"}),
        failure_reason: "Max retries (5) exceeded: database lock timeout".to_string(),
        attempts: 5,
        first_failed_at: now,
        failed_at: now,
    };

    let json_str = serde_json::to_string(&job).expect("Should serialize DeadLetterJob");
    let deserialized: DeadLetterJob =
        serde_json::from_str(&json_str).expect("Should deserialize DeadLetterJob");

    assert_eq!(deserialized.id, "job-999");
    assert_eq!(deserialized.attempts, 5);
    assert_eq!(deserialized.job_name, "CONTRACT_VERIFY");
}
