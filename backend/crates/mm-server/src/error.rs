//! API error type: every failure leaves the server as
//! `{"error":{"code":"snake_code","message":"...","details":null}}`.

use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{message}")]
    ValidationFailed {
        message: String,
        details: Option<Value>,
    },
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Internal(String),
    #[error("{0}")]
    UpstreamUnavailable(String),
    #[error("{0}")]
    NotImplemented(String),
}

impl ApiError {
    pub fn code(&self) -> &'static str {
        match self {
            ApiError::BadRequest(_) => "bad_request",
            ApiError::NotFound(_) => "not_found",
            ApiError::ValidationFailed { .. } => "validation_failed",
            ApiError::Conflict(_) => "conflict",
            ApiError::Internal(_) => "internal",
            ApiError::UpstreamUnavailable(_) => "upstream_unavailable",
            ApiError::NotImplemented(_) => "not_implemented",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::ValidationFailed { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::UpstreamUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
        }
    }

    pub fn details(&self) -> Value {
        match self {
            ApiError::ValidationFailed { details, .. } => details.clone().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({
            "error": {
                "code": self.code(),
                "message": self.to_string(),
                "details": self.details(),
            }
        });
        (self.status(), Json(body)).into_response()
    }
}

/// `Json` extractor whose rejections use the error envelope.
pub struct ApiJson<T>(pub T);

#[axum::async_trait]
impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(ApiJson(v)),
            Err(e) => Err(ApiError::BadRequest(e.body_text())),
        }
    }
}
