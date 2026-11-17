use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use betterlite_openai::chat_content::ChatContentError;
use betterlite_openai::cohere_chat_v1::error::CohereV1ConfigError;
use betterlite_openai::cohere_chat_v1::reply::ReplyError as CohereV1ReplyError;
use betterlite_openai::cohere_chat_v2::CohereV2ConfigError;
use betterlite_openai::cohere_chat_v2::reply::ReplyError as CohereV2ReplyError;
use betterlite_openai::cohere_embedding::CohereEmbeddingError;
use betterlite_openai::cohere_rerank::CohereRerankError;
use betterlite_openai::embedding::EmbeddingError;
use betterlite_openai::host_rerank::error::HostRerankError;
use betterlite_openai::infinity_embedding::error::InfinityEmbeddingError;
use betterlite_openai::voyage_embedding::VoyageEmbeddingError;
use betterlite_openai::voyage_rerank::VoyageRerankError;

use crate::operation::OperationError;

#[derive(Debug, Error)]
pub enum FamilyError {
    #[error("the {provider} provider is not one of the embedding and rerank families")]
    Unsupported { provider: String },
    #[error("the request could not be written as JSON: {source}")]
    Encode {
        #[from]
        source: serde_json::Error,
    },
    #[error("the {family} endpoint {value:?} is not a URL: {source}")]
    Url {
        family: &'static str,
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header holds a value HTTP cannot carry: {source}")]
    Header {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("{name} is not a header name: {source}")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the bundled cl100k vocabulary that counts Cohere input is missing")]
    Vocabulary,
    #[error("the Voyage rerank document is not an object with a text string")]
    VoyageDocument,
    #[error(transparent)]
    CohereV1(#[from] CohereV1ConfigError),
    #[error(transparent)]
    CohereV1Reply(#[from] CohereV1ReplyError),
    #[error(transparent)]
    CohereV2(#[from] CohereV2ConfigError),
    #[error(transparent)]
    CohereV2Reply(#[from] CohereV2ReplyError),
    #[error(transparent)]
    ChatBody(#[from] ChatContentError),
    #[error(transparent)]
    CohereEmbedding(#[from] CohereEmbeddingError),
    #[error(transparent)]
    VoyageEmbedding(#[from] VoyageEmbeddingError),
    #[error(transparent)]
    JinaEmbedding(#[from] EmbeddingError),
    #[error(transparent)]
    InfinityEmbedding(#[from] InfinityEmbeddingError),
    #[error(transparent)]
    CohereRerank(#[from] CohereRerankError),
    #[error(transparent)]
    HostRerank(#[from] HostRerankError),
    #[error(transparent)]
    VoyageRerank(#[from] VoyageRerankError),
}

impl From<FamilyError> for OperationError {
    fn from(error: FamilyError) -> Self {
        match error {
            FamilyError::Encode { source } => Self::Encode { source },
            FamilyError::Header { name, source } => Self::InvalidHeader { name, source },
            // the framework has no variant for a family's own preparation failure, so it names the stage
            FamilyError::Unsupported { .. }
            | FamilyError::Url { .. }
            | FamilyError::HeaderName { .. }
            | FamilyError::Vocabulary
            | FamilyError::VoyageDocument
            | FamilyError::CohereV1(_)
            | FamilyError::CohereV1Reply(_)
            | FamilyError::CohereV2(_)
            | FamilyError::CohereV2Reply(_)
            | FamilyError::ChatBody(_)
            | FamilyError::CohereEmbedding(_)
            | FamilyError::VoyageEmbedding(_)
            | FamilyError::JinaEmbedding(_)
            | FamilyError::InfinityEmbedding(_)
            | FamilyError::CohereRerank(_)
            | FamilyError::HostRerank(_)
            | FamilyError::VoyageRerank(_) => Self::UnsupportedOperation {
                provider: String::from("embedding_rerank"),
                operation: String::from("prepare"),
            },
        }
    }
}
