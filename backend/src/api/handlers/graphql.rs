use axum::{extract::Json, response::IntoResponse};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Default maximum query depth allowed before rejecting execution.
pub const DEFAULT_MAX_QUERY_DEPTH: usize = 6;

/// Default maximum query complexity score allowed before rejecting execution.
pub const DEFAULT_MAX_QUERY_COMPLEXITY: usize = 100;

/// Representation of GraphQL Request payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLRequest {
    pub query: String,
    pub operation_name: Option<String>,
    pub variables: Option<Value>,
}

/// Representation of GraphQL Response payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLResponse {
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQLError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<String>>,
}

/// Microservice Subgraph identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SubgraphDomain {
    Contracts,
    Governance,
    Indexing,
    Auth,
}

/// Analyzes a GraphQL query string to determine maximum selection depth and complexity score.
pub fn analyze_graphql_query(query: &str) -> Result<(usize, usize), String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("GraphQL query cannot be empty".to_string());
    }

    let mut current_depth: usize = 0;
    let mut max_depth: usize = 0;
    let mut complexity: usize = 0;

    let mut in_string = false;
    let mut escape = false;
    let mut in_comment = false;
    let mut in_arguments = 0;

    let chars: Vec<char> = query.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let ch = chars[i];

        if in_comment {
            if ch == '\n' || ch == '\r' {
                in_comment = false;
            }
            i += 1;
            continue;
        }

        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if ch == '#' {
            in_comment = true;
            i += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            i += 1;
            continue;
        }

        if ch == '(' {
            in_arguments += 1;
            i += 1;
            continue;
        } else if ch == ')' {
            if in_arguments > 0 {
                in_arguments -= 1;
            }
            i += 1;
            continue;
        }

        if in_arguments > 0 {
            i += 1;
            continue;
        }

        if ch == '{' {
            current_depth += 1;
            if current_depth > max_depth {
                max_depth = current_depth;
            }
            i += 1;
            continue;
        } else if ch == '}' {
            if current_depth > 0 {
                current_depth -= 1;
            } else {
                return Err("Syntax Error: Unmatched closing brace in GraphQL query".to_string());
            }
            i += 1;
            continue;
        }

        // Field identifier identification
        if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < n && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            let is_keyword = matches!(
                ident.to_ascii_lowercase().as_str(),
                "query"
                    | "mutation"
                    | "subscription"
                    | "fragment"
                    | "on"
                    | "true"
                    | "false"
                    | "null"
            );
            if current_depth > 0 && !is_keyword {
                complexity += 1;
            }
            continue;
        }

        i += 1;
    }

    if current_depth != 0 {
        return Err("Syntax Error: Unclosed brace in GraphQL query".to_string());
    }

    Ok((max_depth, complexity))
}

/// GraphQL Federation Gateway Manager
#[derive(Debug, Clone)]
pub struct GraphQLFederationGateway {
    #[allow(dead_code)]
    subgraphs: HashMap<SubgraphDomain, String>,
    pub max_depth: usize,
    pub max_complexity: usize,
}

impl Default for GraphQLFederationGateway {
    fn default() -> Self {
        let mut subgraphs = HashMap::new();
        subgraphs.insert(
            SubgraphDomain::Contracts,
            "http://localhost:8081/graphql".to_string(),
        );
        subgraphs.insert(
            SubgraphDomain::Governance,
            "http://localhost:8082/graphql".to_string(),
        );
        subgraphs.insert(
            SubgraphDomain::Indexing,
            "http://localhost:8083/graphql".to_string(),
        );
        subgraphs.insert(
            SubgraphDomain::Auth,
            "http://localhost:8084/graphql".to_string(),
        );
        Self {
            subgraphs,
            max_depth: DEFAULT_MAX_QUERY_DEPTH,
            max_complexity: DEFAULT_MAX_QUERY_COMPLEXITY,
        }
    }
}

impl GraphQLFederationGateway {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(max_depth: usize, max_complexity: usize) -> Self {
        Self {
            max_depth,
            max_complexity,
            ..Self::default()
        }
    }

    /// Process federated GraphQL query across microservice subgraphs
    pub async fn execute_query(&self, req: GraphQLRequest) -> GraphQLResponse {
        let q = req.query.trim();

        // Enforce query depth and complexity limits to protect against DoS
        let (depth, complexity) = match analyze_graphql_query(q) {
            Ok(metrics) => metrics,
            Err(err_msg) => {
                return GraphQLResponse {
                    data: None,
                    errors: Some(vec![GraphQLError {
                        message: err_msg,
                        path: None,
                    }]),
                };
            }
        };

        if depth > self.max_depth {
            return GraphQLResponse {
                data: None,
                errors: Some(vec![GraphQLError {
                    message: format!(
                        "Query depth limit of {} exceeded (actual: {})",
                        self.max_depth, depth
                    ),
                    path: None,
                }]),
            };
        }

        if complexity > self.max_complexity {
            return GraphQLResponse {
                data: None,
                errors: Some(vec![GraphQLError {
                    message: format!(
                        "Query complexity limit of {} exceeded (actual: {})",
                        self.max_complexity, complexity
                    ),
                    path: None,
                }]),
            };
        }

        // Schema Introspection Query
        if q.contains("__schema") || q.contains("__type") {
            let app_env = std::env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());
            if app_env.eq_ignore_ascii_case("production") {
                return GraphQLResponse {
                    data: None,
                    errors: Some(vec![GraphQLError {
                        message: "GraphQL introspection is disabled in production environment"
                            .to_string(),
                        path: None,
                    }]),
                };
            }

            return GraphQLResponse {
                data: Some(json!({
                    "__schema": {
                        "types": [
                            { "name": "Query", "kind": "OBJECT" },
                            { "name": "Contract", "kind": "OBJECT" },
                            { "name": "Proposal", "kind": "OBJECT" },
                            { "name": "User", "kind": "OBJECT" },
                            { "name": "IndexEvent", "kind": "OBJECT" }
                        ],
                        "queryType": { "name": "Query" }
                    }
                })),
                errors: None,
            };
        }

        // Entity routing mock resolution for federated domain subgraphs
        if q.contains("contracts") || q.contains("contract") {
            return GraphQLResponse {
                data: Some(json!({
                    "contracts": [
                        { "id": "c-101", "name": "Soroban Token Contract", "address": "CC...101" },
                        { "id": "c-102", "name": "Governance DAO", "address": "CC...102" }
                    ]
                })),
                errors: None,
            };
        }

        if q.contains("proposal") || q.contains("governance") {
            return GraphQLResponse {
                data: Some(json!({
                    "proposals": [
                        { "id": 1, "title": "Upgrade Oracle Whitelist", "status": "ACTIVE", "votesFor": 15000 }
                    ]
                })),
                errors: None,
            };
        }

        if q.contains("me") || q.contains("user") {
            return GraphQLResponse {
                data: Some(json!({
                    "me": { "id": "u-001", "username": "ACodehunter", "role": "ADMIN" }
                })),
                errors: None,
            };
        }

        // Fallback default Query response
        GraphQLResponse {
            data: Some(json!({
                "serviceHealth": { "status": "UP", "subgraphsConnected": 4 }
            })),
            errors: None,
        }
    }
}

/// Axum GraphQL POST handler endpoint
pub async fn graphql_handler(Json(payload): Json<GraphQLRequest>) -> impl IntoResponse {
    let gateway = GraphQLFederationGateway::new();
    let response = gateway.execute_query(payload).await;
    Json(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_query_depth_limit_exceeded() {
        let gateway = GraphQLFederationGateway::new(); // default max_depth = 6
        let deep_query = r#"
            query DeepQuery {
                level1 {
                    level2 {
                        level3 {
                            level4 {
                                level5 {
                                    level6 {
                                        level7 {
                                            id
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        "#;

        let req = GraphQLRequest {
            query: deep_query.to_string(),
            operation_name: None,
            variables: None,
        };

        let resp = gateway.execute_query(req).await;
        assert!(resp.data.is_none());
        assert!(resp.errors.is_some());
        let errors = resp.errors.unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0]
            .message
            .contains("Query depth limit of 6 exceeded"));
    }

    #[tokio::test]
    async fn test_query_depth_allowed_within_limit() {
        let gateway = GraphQLFederationGateway::new();
        let shallow_query = r#"
            query ShallowQuery {
                level1 {
                    level2 {
                        id
                    }
                }
            }
        "#;

        let req = GraphQLRequest {
            query: shallow_query.to_string(),
            operation_name: None,
            variables: None,
        };

        let resp = gateway.execute_query(req).await;
        assert!(resp.data.is_some());
        assert!(resp.errors.is_none());
    }

    #[tokio::test]
    async fn test_query_complexity_limit_exceeded() {
        let gateway = GraphQLFederationGateway::with_limits(10, 5); // limit complexity to 5
        let complex_query = r#"
            query HighComplexity {
                contracts {
                    id
                    name
                    address
                    status
                    owner
                    created_at
                }
            }
        "#;

        let req = GraphQLRequest {
            query: complex_query.to_string(),
            operation_name: None,
            variables: None,
        };

        let resp = gateway.execute_query(req).await;
        assert!(resp.data.is_none());
        assert!(resp.errors.is_some());
        let errors = resp.errors.unwrap();
        assert!(errors[0]
            .message
            .contains("Query complexity limit of 5 exceeded"));
    }

    #[tokio::test]
    async fn test_query_complexity_allowed_within_limit() {
        let gateway = GraphQLFederationGateway::with_limits(10, 50);
        let normal_query = r#"
            query Normal {
                contracts {
                    id
                    name
                }
            }
        "#;

        let req = GraphQLRequest {
            query: normal_query.to_string(),
            operation_name: None,
            variables: None,
        };

        let resp = gateway.execute_query(req).await;
        assert!(resp.data.is_some());
        assert!(resp.errors.is_none());
    }

    #[tokio::test]
    async fn test_unmatched_braces_syntax_error() {
        let gateway = GraphQLFederationGateway::new();
        let broken_query = "{ contracts { id name }";

        let req = GraphQLRequest {
            query: broken_query.to_string(),
            operation_name: None,
            variables: None,
        };

        let resp = gateway.execute_query(req).await;
        assert!(resp.data.is_none());
        assert!(resp.errors.is_some());
        let errors = resp.errors.unwrap();
        assert!(errors[0].message.contains("Syntax Error"));
    }
}
