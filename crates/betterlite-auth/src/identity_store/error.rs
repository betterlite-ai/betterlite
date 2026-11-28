use crate::auth_errors::conversion::AuthFailure;
use crate::auth_key_lookup::error::KeyLookupError;
use crate::resolved_key::auth::AuthError;

#[derive(Debug, thiserror::Error)]
pub enum IdentityStoreError {
    #[error(transparent)]
    KeyLookup(#[from] KeyLookupError),
    #[error(transparent)]
    Cache(#[from] AuthError),
    #[error("Principal carries no source key; it was not produced by IdentityStore.resolve")]
    PrincipalMissingSourceKey,
    #[error("the identity store is missing required ports: {ports}")]
    MissingPorts { ports: String },
}

impl IdentityStoreError {
    // B757's translation: the key lookup's own envelope, then the generic 401 an ordinary failure becomes.
    #[must_use]
    pub fn failure(&self) -> AuthFailure {
        match self {
            Self::KeyLookup(error) => error.failure(),
            Self::Cache(error) => AuthFailure::Other {
                message: error.to_string(),
                param: None,
            },
            Self::PrincipalMissingSourceKey | Self::MissingPorts { .. } => AuthFailure::Other {
                message: self.to_string(),
                param: None,
            },
        }
    }
}
