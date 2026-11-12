use std::error::Error;

use crate::credential_sources::error::CredentialSourceError;

pub const REAUTHENTICATION_MARKER: &str = "Reauthentication is needed";

#[derive(Debug, thiserror::Error)]
pub enum CredentialCacheError {
    #[error("the vertex credential could not be loaded")]
    Load(#[source] CredentialSourceError),
    #[error("the vertex credential could not be refreshed")]
    Refresh(#[source] CredentialSourceError),
    #[error("Could not resolve credentials")]
    MissingCredentials,
    #[error("Could not resolve credentials token")]
    MissingToken,
    #[error("Could not resolve project_id")]
    MissingProject,
    #[error("the vertex credential refresh lease is closed")]
    LeaseClosed,
}

// LiteLLM retries only when the refresh failure carries this text somewhere in its cause chain
#[must_use]
pub fn needs_reauthentication(error: &CredentialCacheError) -> bool {
    let mut source: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(current) = source {
        if current.to_string().contains(REAUTHENTICATION_MARKER) {
            return true;
        }
        source = current.source();
    }
    false
}
