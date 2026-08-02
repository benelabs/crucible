use axum::{
    body::Body,
    http::{HeaderName, HeaderValue, Request},
    middleware::Next,
    response::IntoResponse,
};
use tracing::Instrument;
use uuid::Uuid;

pub const REQUEST_ID_HEADER: &str = "x-request-id";

#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// Middleware that generates or propagates a unique Request ID for each incoming HTTP request.
///
/// It ensures that:
/// 1. An X-Request-ID header exists on the incoming request. If not, it generates a new UUID.
/// 2. The Request ID is stored in request Extensions so other handlers/middlewares can access it.
/// 3. The request processing is wrapped in a tracing span tagged with request_id.
/// 4. The response contains the X-Request-ID header.
pub async fn request_id_middleware(
    mut request: Request<Body>,
    next: Next,
) -> impl IntoResponse {
    let header_name = HeaderName::from_static(REQUEST_ID_HEADER);

    let request_id = request
        .headers()
        .get(&header_name)
        .and_then(|val| val.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let header_val = HeaderValue::from_str(&request_id).unwrap_or_else(|_| HeaderValue::from_static(""));
    request.headers_mut().insert(header_name.clone(), header_val.clone());

    request.extensions_mut().insert(RequestId(request_id.clone()));

    let span = tracing::info_span!("request", request_id = %request_id);

    let mut response = async move {
        next.run(request).await
    }
    .instrument(span)
    .await
    .into_response();

    response.headers_mut().insert(header_name, header_val);

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_request_id_middleware_generates_id() {
        let app = Router::new()
            .route("/", get(|req: Request<Body>| async move {
                let req_id = req.extensions().get::<RequestId>().cloned();
                assert!(req_id.is_some());
                "OK"
            }))
            .layer(axum::middleware::from_fn(request_id_middleware));

        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert!(response.headers().contains_key(REQUEST_ID_HEADER));
        let header_val = response.headers().get(REQUEST_ID_HEADER).unwrap().to_str().unwrap();
        assert!(Uuid::parse_str(header_val).is_ok());
    }

    #[tokio::test]
    async fn test_request_id_middleware_propagates_existing_id() {
        let existing_id = "test-req-id-12345";
        let app = Router::new()
            .route("/", get(move |req: Request<Body>| async move {
                let req_id = req.extensions().get::<RequestId>().cloned().unwrap();
                assert_eq!(req_id.0, existing_id);
                "OK"
            }))
            .layer(axum::middleware::from_fn(request_id_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(REQUEST_ID_HEADER, existing_id)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.headers().get(REQUEST_ID_HEADER).unwrap().to_str().unwrap(),
            existing_id
        );
    }
}
