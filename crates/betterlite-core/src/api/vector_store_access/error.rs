use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;

#[derive(Debug, Error)]
pub enum VectorStoreAccessError {
    #[error("the caller's credential did not project into the policy's fields")]
    Fields(#[source] serde_json::Error),
    #[error("the parsed litellm_params could not be written back as JSON")]
    Params(#[source] serde_json::Error),
    #[error(transparent)]
    Team(#[from] TeamLookupError),
    #[error(transparent)]
    Limits(#[from] ApiLimitError),
}

impl VectorStoreAccessError {
    // the HTTP status a route answers with, or None where the policy call raises no HTTPException
    #[must_use]
    pub fn http(&self) -> Option<u16> {
        match self {
            Self::Team(error) => error.http().map(|(status, _detail)| status),
            Self::Fields(_) | Self::Params(_) | Self::Limits(_) => None,
        }
    }
}
