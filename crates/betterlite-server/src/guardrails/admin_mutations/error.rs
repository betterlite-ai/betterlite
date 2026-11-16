use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_guardrails::guardrail_registry::error::RegistryError;
use serde::Serialize;
use thiserror::Error;

use super::body::BodyError;
use super::review_rows::ReviewRowsError;
use crate::tracing_routes::router::unauthenticated;

pub(super) const MANAGE_ADMIN_REQUIRED: &str = "Admin access required to manage guardrails";
pub(super) const REVIEW_ADMIN_REQUIRED: &str = "Admin access required";

#[derive(Debug, Error)]
pub(super) enum MutationError {
    #[error("the request has no authenticated caller")]
    Unauthenticated,
    #[error("{0}")]
    Body(Box<BodyError>),
    #[error("{detail}")]
    Forbidden { detail: &'static str },
    #[error("Prisma client not initialized")]
    NoDatabase,
    #[error("Guardrail with ID {guardrail_id} not found")]
    NotFound { guardrail_id: String },
    #[error("Guardrail submission not found")]
    SubmissionNotFound,
    #[error("Guardrail is not pending review (status={status})")]
    NotPendingReview { status: String },
    #[error("Guardrail litellm_params is missing or invalid")]
    MissingParams,
    #[error("Guardrail configuration error: {source}")]
    Configuration { source: RegistryError },
    #[error("Invalid guardrail configuration, update rejected: {source}")]
    UpdateRejected { source: RegistryError },
    #[error("{source}")]
    Registry { source: RegistryError },
    #[error("{source}")]
    ReviewRows { source: ReviewRowsError },
    #[error("Guardrail with ID {guardrail_id} not found after the update")]
    MissingAfterUpdate { guardrail_id: String },
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

// LiteLLM's ValueError and TypeError from a guardrail constructor, and nothing else
pub(super) const fn is_configuration_error(error: &RegistryError) -> bool {
    matches!(
        error,
        RegistryError::Unsupported { .. }
            | RegistryError::MissingGuardrailType
            | RegistryError::ScanOnlyUnsupported { .. }
            | RegistryError::ScanOnlyWithSkipTool { .. }
            | RegistryError::Base(_)
            | RegistryError::Initializer(_)
            | RegistryError::InitFailed { .. }
    )
}

impl MutationError {
    const fn status(&self) -> StatusCode {
        match self {
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Body(_) | Self::UpdateRejected { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::NoDatabase
            | Self::MissingParams
            | Self::Registry { .. }
            | Self::ReviewRows { .. }
            | Self::MissingAfterUpdate { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotFound { .. } | Self::SubmissionNotFound => StatusCode::NOT_FOUND,
            Self::NotPendingReview { .. } | Self::Configuration { .. } => StatusCode::BAD_REQUEST,
        }
    }
}

// LiteLLM's eager 500s: verbose_proxy_logger.exception(<context>: %s, e) then str(e) as the detail
pub(super) fn logged(context: &str, error: MutationError) -> MutationError {
    tracing::error!("{context}: {error}");
    error
}

impl IntoResponse for MutationError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => unauthenticated(),
            Self::Body(error) => (*error).into_response(),
            Self::Forbidden { .. }
            | Self::NoDatabase
            | Self::NotFound { .. }
            | Self::SubmissionNotFound
            | Self::NotPendingReview { .. }
            | Self::MissingParams
            | Self::Configuration { .. }
            | Self::UpdateRejected { .. }
            | Self::Registry { .. }
            | Self::ReviewRows { .. }
            | Self::MissingAfterUpdate { .. } => {
                let detail = self.to_string();
                (self.status(), Json(DetailBody { detail })).into_response()
            }
        }
    }
}
