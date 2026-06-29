use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use backend::api::handlers::contracts::create_upgrade_plan;
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn create_upgrade_plan_returns_ready_plan() {
    let app = Router::new().route("/api/v1/contracts/upgrade-plan", post(create_upgrade_plan));

    let payload = json!({
        "contractId": "CCONTRACT123",
        "currentVersion": "1.2.3",
        "targetVersion": "1.2.4",
        "currentWasmHash": "wasm-old",
        "targetWasmHash": "wasm-new",
        "requestedBy": "GADMIN123",
        "strategy": "inPlace",
        "migrationRequired": false,
        "stateMigrationHash": null,
        "compatibilityChecks": [
            { "name": "storage_layout", "passed": true, "notes": null },
            { "name": "public_interface", "passed": true, "notes": null },
            { "name": "authorization", "passed": true, "notes": null }
        ]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/contracts/upgrade-plan")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "success");
    assert_eq!(json["data"]["status"], "ready");
    assert_eq!(json["data"]["riskLevel"], "low");
    assert_eq!(json["data"]["approvalsRequired"], 1);
}

#[tokio::test]
async fn create_upgrade_plan_rejects_invalid_version_order() {
    let app = Router::new().route("/api/v1/contracts/upgrade-plan", post(create_upgrade_plan));

    let payload = json!({
        "contractId": "CCONTRACT123",
        "currentVersion": "1.2.3",
        "targetVersion": "1.2.3",
        "currentWasmHash": "wasm-old",
        "targetWasmHash": "wasm-new",
        "requestedBy": "GADMIN123",
        "strategy": "inPlace",
        "migrationRequired": false,
        "stateMigrationHash": null,
        "compatibilityChecks": [
            { "name": "storage_layout", "passed": true, "notes": null },
            { "name": "public_interface", "passed": true, "notes": null },
            { "name": "authorization", "passed": true, "notes": null }
        ]
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/contracts/upgrade-plan")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
