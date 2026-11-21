use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

pub mod guard;
pub mod store;

mod tests;

pub const AGENTS_DISABLED: &str =
    "Access to agents is disabled for your role. Contact your proxy admin.";
pub const VECTOR_STORES_DISABLED: &str =
    "Access to vector_stores is disabled for your role. Contact your proxy admin.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureName {
    Agents,
    VectorStores,
}

// LiteLLM raises HTTPException(403, {"error": ...}); the route that mounts the guard sends this detail
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ErrorDetail {
    pub error: &'static str,
}

impl FeatureName {
    #[must_use]
    pub const fn denial(self) -> ErrorDetail {
        match self {
            Self::Agents => ErrorDetail {
                error: AGENTS_DISABLED,
            },
            Self::VectorStores => ErrorDetail {
                error: VECTOR_STORES_DISABLED,
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FeatureAccessError<Failure> {
    #[error("{}", .0.denial().error)]
    Disabled(FeatureName),
    // LiteLLM does not catch a failure of the exemption call, so it leaves the guard as the route's own error
    #[error("the admin privilege check failed")]
    Privileges(#[source] Failure),
}

impl<Failure> FeatureAccessError<Failure> {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Disabled(_) => StatusCode::FORBIDDEN,
            Self::Privileges(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
