use betterlite_http::error::HttpError;
use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum VertexInteractionsError {
    #[error(
        "a Vertex AI project is required; set vertex_project or litellm_settings.vertex_project"
    )]
    MissingProject,
    #[error("the Vertex AI access token could not be minted")]
    Mint(#[source] CredentialCacheError),
    #[error("the vertex location is not valid")]
    Location(#[source] VertexUrlError),
    #[error("the Vertex AI base URL is not valid")]
    ApiBase(#[source] EndpointTargetError),
    #[error("the interaction_id cannot be used as a URL segment")]
    InteractionId(#[source] HttpError),
}
