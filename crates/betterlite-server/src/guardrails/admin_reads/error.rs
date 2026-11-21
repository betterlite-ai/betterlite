use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use betterlite_guardrails::guardrail_registry::error::RegistryError;
use serde::Serialize;
use thiserror::Error;

use crate::guardrails::provider_forms::error::FormError;
use crate::tracing_routes::router::unauthenticated;

#[derive(Debug, Error)]
pub(super) enum ReadError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("Guardrail with ID {guardrail_id} not found")]
    NotFound { guardrail_id: String },
    // LiteLLM's _extract_fields_recursive raises this exact text
    #[error(
        "Max depth of {limit} exceeded while processing model fields. Please check the model structure for excessive nesting."
    )]
    TooDeep { limit: usize },
    #[error("{detail}")]
    Internal { detail: String },
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

impl ReadError {
    const fn status(&self) -> StatusCode {
        match self {
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::TooDeep { .. } => StatusCode::BAD_REQUEST,
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<GuardrailParamsError> for ReadError {
    fn from(error: GuardrailParamsError) -> Self {
        Self::Internal {
            detail: error.to_string(),
        }
    }
}

impl From<RegistryError> for ReadError {
    fn from(error: RegistryError) -> Self {
        Self::Internal {
            detail: error.to_string(),
        }
    }
}

impl From<FormError> for ReadError {
    fn from(error: FormError) -> Self {
        match error {
            FormError::TooDeep { limit } => Self::TooDeep { limit },
        }
    }
}

impl IntoResponse for ReadError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::NotFound { .. } | Self::TooDeep { .. } | Self::Internal { .. } => {
                let detail = self.to_string();
                (self.status(), Json(DetailBody { detail })).into_response()
            }
        }
    }
}
