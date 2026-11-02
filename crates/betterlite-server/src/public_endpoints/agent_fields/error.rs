use std::sync::Arc;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::public_endpoints::providers::error::ProvidersError;

#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentFieldsError {
    #[error("agent catalog is {size} bytes, over the {limit} byte limit")]
    CatalogTooLarge { size: usize, limit: usize },
    #[error("agent catalog JSON error: {0}")]
    CatalogJson(#[source] Arc<serde_json::Error>),
    #[error("provider catalog error: {0}")]
    Providers(#[source] ProvidersError),
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
impl IntoResponse for AgentFieldsError {
    fn into_response(self) -> Response {
        tracing::warn!(error = %self, "agent fields metadata failed to load");
        let body = InternalErrorBody {
            error: InternalError {
                message: "Internal server error",
                error_type: "internal_server_error",
            },
        };
        (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}
