use axum::{
    extract::Request,
    http::{header, Method},
    middleware::Next,
    response::Response,
};

use crate::error::AppError;

const BODYLESS_MUTATION_ROUTES: [&str; 2] = ["/api/config/reload", "/api/v1/auth/rotate"];

/// Helper to validate if a Content-Type header string represents `application/json`.
///
/// Splits off any optional parameters (e.g. `; charset=utf-8`) and compares the primary
/// media type case-insensitively. A plain `starts_with` check would wrongly accept
/// values such as `application/json5` or `application/json_evil`.
pub fn is_json_content_type(header_val: &str) -> bool {
    let media_type = header_val.split(';').next().unwrap_or("").trim();
    media_type.eq_ignore_ascii_case("application/json")
}

/// Middleware that validates the Content-Type header on mutating requests.
///
/// For requests with JSON bodies, this middleware ensures the client sends a valid
/// `Content-Type: application/json` header, matched case-insensitively according to MIME
/// specifications (RFC 2045, RFC 7231 §3.1.1.1). Parameters such as `; charset=utf-8` are
/// ignored. Safe methods (GET, HEAD, OPTIONS) and the explicitly bodyless mutation routes
/// below are allowed through without validation.
///
/// Returns `415 Unsupported Media Type` if the Content-Type is missing, malformed, or
/// does not evaluate to `application/json`.
pub async fn require_json_content_type(req: Request, next: Next) -> Result<Response, AppError> {
    let method = req.method();
    if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
        return Ok(next.run(req).await);
    }

    if method == Method::POST && BODYLESS_MUTATION_ROUTES.contains(&req.uri().path()) {
        return Ok(next.run(req).await);
    }

    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if is_json_content_type(content_type) {
        Ok(next.run(req).await)
    } else {
        Err(AppError::UnsupportedMediaType(
            "Expected Content-Type: application/json".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::post,
        Router,
    };
    use tower::ServiceExt;

    fn test_app() -> Router {
        Router::new()
            .route(
                "/test",
                post(|| async { "ok" })
                    .get(|| async { "get_ok" })
                    .head(|| async { "head_ok" })
                    .options(|| async { "options_ok" }),
            )
            .layer(middleware::from_fn(require_json_content_type))
    }

    #[tokio::test]
    async fn accepts_standard_application_json() {
        let app = test_app();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Content-Type", "application/json")
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn accepts_application_json_with_charset_parameter() {
        let app = test_app();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Content-Type", "application/json; charset=utf-8")
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn accepts_application_json_with_whitespace_and_multiple_parameters() {
        let app = test_app();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header(
                "Content-Type",
                "application/json ; charset=UTF-8; boundary=something",
            )
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn accepts_uppercase_and_mixed_case_content_type() {
        let app = test_app();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Content-Type", "APPLICATION/JSON")
            .body(Body::empty())
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let req_mixed = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Content-Type", "Application/Json; Charset=utf-8")
            .body(Body::empty())
            .unwrap();

        let res_mixed = app.oneshot(req_mixed).await.unwrap();
        assert_eq!(res_mixed.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_non_json_subtypes_like_json_patch_or_xml() {
        let app = test_app();
        let req_patch = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Content-Type", "application/json-patch+json")
            .body(Body::empty())
            .unwrap();

        let res_patch = app.clone().oneshot(req_patch).await.unwrap();
        assert_eq!(res_patch.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let req_xml = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Content-Type", "application/xml")
            .body(Body::empty())
            .unwrap();

        let res_xml = app.oneshot(req_xml).await.unwrap();
        assert_eq!(res_xml.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn rejects_text_plain_and_text_html() {
        let app = test_app();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Content-Type", "text/plain")
            .body(Body::empty())
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let req_html = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Content-Type", "text/html; charset=utf-8")
            .body(Body::empty())
            .unwrap();

        let res_html = app.oneshot(req_html).await.unwrap();
        assert_eq!(res_html.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn rejects_missing_content_type_on_post() {
        let app = test_app();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn rejects_malformed_content_type_header() {
        let app = test_app();
        let req = Request::builder()
            .method("POST")
            .uri("/test")
            .header("Content-Type", "invalid-content-type-no-slash")
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn allows_safe_methods_without_content_type() {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            let req = Request::builder()
                .method(method)
                .uri("/test")
                .body(Body::empty())
                .unwrap();

            let res = test_app().oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn allows_bodyless_mutation_routes_without_content_type() {
        let app = Router::new()
            .route("/api/config/reload", post(|| async { "reloaded" }))
            .route("/api/v1/auth/rotate", post(|| async { "rotated" }))
            .layer(middleware::from_fn(require_json_content_type));

        for path in BODYLESS_MUTATION_ROUTES {
            let req = Request::builder()
                .method(Method::POST)
                .uri(path)
                .body(Body::empty())
                .unwrap();

            let res = app.clone().oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "route {path}");
        }
    }

    #[test]
    fn test_is_json_content_type_valid() {
        assert!(is_json_content_type("application/json"));
        assert!(is_json_content_type("application/json; charset=utf-8"));
        assert!(is_json_content_type("application/json;charset=utf-8"));
        assert!(is_json_content_type("APPLICATION/JSON"));
        assert!(is_json_content_type("APPLICATION/JSON; charset=utf-8"));
        assert!(is_json_content_type(
            "  application/json  ; boundary=something "
        ));
    }

    #[test]
    fn test_is_json_content_type_invalid_and_bypass_prevention() {
        // Bypass attempts that a starts_with check would incorrectly accept
        assert!(!is_json_content_type("application/json_evil"));
        assert!(!is_json_content_type("application/json5"));
        assert!(!is_json_content_type("application/json-patch+json"));

        assert!(!is_json_content_type("text/plain"));
        assert!(!is_json_content_type("application/xml"));
        assert!(!is_json_content_type("multipart/form-data"));
        assert!(!is_json_content_type(""));
    }
}
