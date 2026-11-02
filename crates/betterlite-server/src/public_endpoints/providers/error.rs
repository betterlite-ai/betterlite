use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug, Clone, thiserror::Error)]
pub enum ProvidersError {
    #[error("provider catalog fragment is {size} bytes, over the {limit} byte limit")]
    FragmentTooLarge { size: usize, limit: usize },
    #[error("provider catalog fragment JSON error: {0}")]
    FragmentJson(String),
    #[error("endpoint support file is {size} bytes, over the {limit} byte limit")]
    EndpointsTooLarge { size: usize, limit: usize },
    #[error("endpoint support file JSON error: {0}")]
    EndpointsJson(String),
}

#[derive(Serialize)]
struct InternalError {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

#[derive(Serialize)]
struct InternalErrorBody {
    error: InternalError,
}

// matches otel_unhandled_exception_handler, the body LiteLLM sends for this uncaught exception.
impl IntoResponse for ProvidersError {
    fn into_response(self) -> Response {
        tracing::warn!(error = %self, "public endpoints metadata failed to load");
        let body = InternalErrorBody {
            error: InternalError {
                message: "Internal server error",
                error_type: "internal_server_error",
            },
        };
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}
