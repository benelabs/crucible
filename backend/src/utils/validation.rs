use async_trait::async_trait;
use axum::{
    extract::{rejection::JsonRejection, FromRequest, Request},
    Json,
};
use serde::de::DeserializeOwned;
use validator::Validate;
use crate::error::AppError;

/// A wrapper for Axum's Json extractor that adds automatic validation.
pub struct ValidatedJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate + 'static,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state).await
            .map_err(|rejection| match rejection {
                JsonRejection::JsonDataError(e) => AppError::BadRequest(format!("JSON data error: {}", e)),
                JsonRejection::JsonSyntaxError(e) => AppError::BadRequest(format!("JSON syntax error: {}", e)),
                _ => AppError::BadRequest("Invalid JSON".to_string()),
            })?;
        
        value.validate().map_err(|e| AppError::BadRequest(format!("Validation error: {}", e)))?;
        
        Ok(ValidatedJson(value))
    }
}
