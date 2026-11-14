use betterlite_vertex::credential_sources::error::CredentialSourceError;
use betterlite_vertex::token_cache::error::CredentialCacheError;

// the wrapped Vertex errors carry their own text: B1301's credential message and B1302's cache failure
#[derive(Debug, thiserror::Error)]
pub enum ArmorTokenError {
    #[error(transparent)]
    Credentials(#[from] CredentialSourceError),
    #[error(transparent)]
    Lookup(#[from] CredentialCacheError),
}
