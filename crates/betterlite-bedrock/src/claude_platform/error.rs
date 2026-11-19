use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_anthropic::request::error::RequestError;
use betterlite_core::api::anthropic_beta_filter::error::BetaFilterError;
use betterlite_core::api::messages_native_transform::error::TransformError;

use crate::claude_platform_inputs::error::PlatformInputError;

// The AuthenticationError both validate methods raise when _get_workspace_id answers None.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error(
        "Missing workspace ID for Claude Platform on AWS. Pass `workspace_id` or configure the provider workspace setting."
    )]
    Missing { model: String },
    #[error(transparent)]
    Input(#[from] PlatformInputError),
}

impl WorkspaceError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Missing { .. } => Some(401),
            Self::Input(error) => error.status_code(),
        }
    }

    // LiteLLM passes the provider and the model to the AuthenticationError it raises
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        match self {
            Self::Missing { model } => Some(model.as_str()),
            Self::Input(_) => None,
        }
    }
}

// The chat validation walks the header predicates after the workspace check.
#[derive(Debug, thiserror::Error)]
pub enum ChatValidationError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Features(#[from] FeatureFlagsError),
    #[error("the request does not round-trip as JSON")]
    Json(#[from] serde_json::Error),
}

impl ChatValidationError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Workspace(error) => error.status_code(),
            Self::Features(error) => error.status_code(),
            Self::Json(_) => None,
        }
    }
}

// The native header preparation only adds headers; the beta updater is the one other failure.
#[derive(Debug, thiserror::Error)]
pub enum NativeHeaderError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Beta(#[from] BetaFilterError),
    #[error("the request does not round-trip as JSON")]
    Json(#[from] serde_json::Error),
}

impl NativeHeaderError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            // LiteLLM raises a plain ValueError from the beta updater, which no status carries
            Self::Workspace(error) => error.status_code(),
            Self::Beta(_) | Self::Json(_) => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NativeTransformError {
    #[error(transparent)]
    Transform(#[from] TransformError),
    #[error("the request does not round-trip as JSON")]
    Json(#[from] serde_json::Error),
}

impl NativeTransformError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Transform(error) => Some(error.status_code()),
            Self::Json(_) => Some(500),
        }
    }
}

// The chat transform only fails where the inherited Anthropic mapper fails.
pub type ChatTransformError<FileFailure> = RequestError<FileFailure>;
