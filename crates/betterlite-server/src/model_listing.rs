pub mod access;
pub mod available;
pub mod info;
pub mod response;

mod tests;

use axum::http::StatusCode;
use betterlite_auth::auth_model_lists::wildcard::WildcardError;

// The failures a listing reports, each the HTTPException LiteLLM raises for it on the listing routes.
#[derive(Debug, thiserror::Error)]
pub enum ModelListingError {
    #[error(transparent)]
    TeamNotFound(#[from] access::TeamMissing),
    #[error(transparent)]
    NotAMember(#[from] access::NotAMember),
    #[error("the key's model grants cannot be read: {0}")]
    KeyFields(#[source] serde_json::Error),
    #[error("the model list could not be expanded: {0}")]
    Wildcard(#[from] WildcardError),
    #[error("The following model(s) do not exist or are not accessible: {}", .models.join(", "))]
    BatchInaccessible { models: Vec<String> },
    #[error("The model `{model_id}` does not exist or is not accessible")]
    ModelInaccessible { model_id: String },
    #[error(
        "Invalid fallback_type. Must be one of: ['general', 'context_window', 'content_policy']"
    )]
    InvalidFallbackType,
}

impl ModelListingError {
    // the status LiteLLM's HTTPException carries for this failure
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::TeamNotFound(_)
            | Self::BatchInaccessible { .. }
            | Self::ModelInaccessible { .. } => StatusCode::NOT_FOUND,
            Self::NotAMember(_) => StatusCode::FORBIDDEN,
            Self::InvalidFallbackType => StatusCode::BAD_REQUEST,
            Self::KeyFields(_) | Self::Wildcard(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
