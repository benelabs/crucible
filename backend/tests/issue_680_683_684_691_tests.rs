use backend::api::{
    handlers::graphql::{GraphQLFederationGateway, GraphQLRequest},
    middleware::rate_limit::{RateLimitConfig, TokenBucketRateLimiter},
};
use serde_json::json;

#[tokio::test]
async fn test_graphql_federation_introspection() {
    let gateway = GraphQLFederationGateway::new();
    let req = GraphQLRequest {
        query: "{ __schema { types { name } } }".to_string(),
        operation_name: None,
        variables: None,
    };

    let resp = gateway.execute_query(req).await;
    assert!(resp.data.is_some());
    let data = resp.data.unwrap();
    assert!(data.get("__schema").is_some());
}

#[tokio::test]
async fn test_graphql_federation_subgraph_routing() {
    let gateway = GraphQLFederationGateway::new();
    let req = GraphQLRequest {
        query: "query { contracts { id name address } }".to_string(),
        operation_name: None,
        variables: None,
    };

    let resp = gateway.execute_query(req).await;
    assert!(resp.data.is_some());
    let data = resp.data.unwrap();
    assert!(data.get("contracts").is_some());
}

#[tokio::test]
async fn test_token_bucket_rate_limiter() {
    let config = RateLimitConfig {
        capacity: 2,
        refill_rate_per_sec: 1,
        ttl: std::time::Duration::from_secs(60),
    };

    let limiter = TokenBucketRateLimiter::new(None, config);

    // First request - allowed
    let res1 = limiter.check_and_consume("client_1").await.unwrap();
    assert!(res1.allowed);
    assert_eq!(res1.remaining, 1);

    // Second request - allowed
    let res2 = limiter.check_and_consume("client_1").await.unwrap();
    assert!(res2.allowed);
    assert_eq!(res2.remaining, 0);

    // Third request - rate limited
    let res3 = limiter.check_and_consume("client_1").await.unwrap();
    assert!(!res3.allowed);
}
