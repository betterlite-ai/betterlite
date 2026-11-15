use thiserror::Error;

use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum ImageIdentityError {
    // LiteLLM raises one ValueError for either missing value, which a proxy answers with a 500
    #[error("vertex_project and vertex_location are required for Vertex AI")]
    MissingTarget,
    #[error("the vertex location is not valid")]
    Location(#[source] VertexUrlError),
    // get_access_token's resolution failures reach image calls the same way
    #[error("the Vertex AI access token could not be minted")]
    Auth(#[source] CredentialCacheError),
}

impl ImageIdentityError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingTarget | Self::Location(_) | Self::Auth(_) => None,
        }
    }
}
