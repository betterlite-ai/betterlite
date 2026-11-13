use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::multimodal_embedding_records::error::MultimodalEmbeddingError;
use crate::token_cache::error::CredentialCacheError;
use crate::vertex_text_embeddings::error::VertexTextEmbeddingError;

use super::post::PostError;

#[derive(Debug, Error)]
pub enum VertexEmbeddingCallError {
    #[error("the Vertex embedding credentials could not be read: {source}")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error("the Vertex embedding endpoint could not be built: {source}")]
    Target {
        #[source]
        source: EndpointTargetError,
    },
    #[error("the Vertex embedding request is not valid: {source}")]
    TextRequest {
        #[source]
        source: VertexTextEmbeddingError,
    },
    #[error("the Vertex embedding answer is not valid: {source}")]
    TextAnswer {
        #[source]
        source: VertexTextEmbeddingError,
    },
    #[error("the {field} value must be true or false")]
    Flag { field: &'static str },
    #[error(transparent)]
    Multimodal(#[from] MultimodalEmbeddingError),
    #[error("the Vertex embedding request could not be written: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Vertex embedding response is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the Vertex embedding response is not valid JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Vertex endpoint answered with status {status}")]
    Status { status: u16, body: Vec<u8> },
    #[error("Timeout error occurred.")]
    Timeout,
    #[error("the Vertex endpoint could not be reached: {source}")]
    Transport {
        #[source]
        source: PostError,
    },
}

// The text and async multimodal branches turn a timeout into Vertex's legacy 408; the sync multimodal branch maps its own errors
impl From<PostError> for VertexEmbeddingCallError {
    fn from(source: PostError) -> Self {
        match source {
            PostError::Timeout => Self::Timeout,
            other @ (PostError::Url(_)
            | PostError::HeaderName { .. }
            | PostError::HeaderValue { .. }
            | PostError::Send { .. }) => Self::Transport { source: other },
        }
    }
}

impl VertexEmbeddingCallError {
    // LiteLLM keeps an upstream status on the text and async multimodal branches; the sync multimodal reply goes through the transformation's 500
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Multimodal(error) => error.status_code(),
            Self::Status { status, .. } => *status,
            Self::Timeout => 408,
            Self::TextRequest { .. } | Self::Flag { .. } => 400,
            Self::Credentials { .. }
            | Self::Target { .. }
            | Self::TextAnswer { .. }
            | Self::Encode { .. }
            | Self::ResponseTooLarge { .. }
            | Self::Decode { .. }
            | Self::Transport { .. } => 500,
        }
    }
}
