use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::token_cache::error::CredentialCacheError;

// LiteLLM sends these through exception_type, where _map_vertex_exception matches message text and status, so a caller classifies PostStatus and FileStatus by passing their status and Display text through betterlite_error::vertex_errors::map_vertex_exception the way model_policy::error::classify_vertex_error does; only variants it leaves unclassified fall back to 500
#[derive(Debug, Error)]
pub enum GeminiEmbeddingCallError {
    #[error("the Vertex AI access token could not be minted")]
    Mint(#[source] CredentialCacheError),
    #[error(transparent)]
    Target(#[from] EndpointTargetError),
    #[error(
        "An API key is required to resolve Gemini file references (files/...). Pass api_key= or set GEMINI_API_KEY."
    )]
    MissingApiKey,
    #[error("the {name} header cannot be sent")]
    Header {
        name: String,
        #[source]
        source: http::Error,
    },
    #[error("the request URL {url} cannot be parsed")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the request to {url} failed")]
    Transport {
        url: String,
        #[source]
        source: TransportError,
    },
    #[error("Error fetching file {file}: {status} {body}")]
    FileStatus {
        file: String,
        status: u16,
        body: String,
    },
    #[error("Error: {status} {body}")]
    PostStatus { status: u16, body: String },
    #[error("the reply is larger than {limit} bytes")]
    BodyTooLarge { limit: usize },
    #[error("the reply is not valid JSON")]
    Decode(#[source] serde_json::Error),
    #[error("the request cannot be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the metadata of {file} must be an object")]
    MetadataShape { file: String },
    #[error("the {field} of {file} must be text")]
    MetadataField { file: String, field: &'static str },
    #[error(transparent)]
    Records(#[from] GeminiEmbeddingError),
}
