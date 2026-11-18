use betterlite_bedrock::sagemaker_runtime::error::RuntimeFailure;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::watsonx_audio::error::api_status;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_tokenizer::engine::error::EngineError;
use thiserror::Error;

use crate::data_cloud_dispatch::error::DispatchError;
use crate::data_cloud_dispatch::handler::HandlerError;
use crate::sap_auth::error::SapAuthError;
use crate::sap_chat::error::SapChatError;
use crate::sap_embedding::error::SapEmbeddingError;
use crate::sap_stream::SapOrchestrationError;
use crate::watsonx_embed::error::WatsonxEmbeddingError;
use crate::watsonx_rerank::error::WatsonxRerankError;

use super::sap::SapLookupError;

// the failures LiteLLM's sap, watsonx and sagemaker arms raise, with the status the exception carries
#[derive(Debug, Error)]
pub enum EnterpriseError {
    #[error(transparent)]
    Handler(#[from] HandlerError),
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
    #[error(transparent)]
    SapLookup(#[from] SapLookupError),
    #[error(transparent)]
    SapAuth(#[from] SapAuthError),
    #[error(transparent)]
    SapChat(#[from] SapChatError),
    #[error(transparent)]
    SapEmbedding(#[from] SapEmbeddingError),
    #[error(transparent)]
    SapOrchestration(#[from] SapOrchestrationError),
    #[error(transparent)]
    Watsonx(#[from] WatsonxError),
    #[error(transparent)]
    WatsonxEmbedding(#[from] WatsonxEmbeddingError),
    #[error(transparent)]
    WatsonxRerank(#[from] WatsonxRerankError),
    #[error(transparent)]
    Sagemaker(#[from] SagemakerError),
    #[error(transparent)]
    SagemakerFailure(#[from] RuntimeFailure),
    #[error("the sagemaker tokenizer vocabulary is not bundled")]
    MissingVocabulary,
    #[error("the sagemaker tokenizer could not be built: {source}")]
    Encoding {
        #[source]
        source: EngineError,
    },
    #[error("the {field} value must be a string")]
    FieldType { field: &'static str },
    #[error("the request body could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider answer could not be read as JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider could not be reached: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the {url} endpoint is not a valid URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the request headers cannot be carried")]
    Headers {
        #[source]
        source: http::Error,
    },
}

impl EnterpriseError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Handler(error) => error.status_code(),
            Self::SapChat(error) => error.status(),
            // LiteLLM raises these from the embedding config constructor, so its mapper has no status to keep
            Self::SapEmbedding(_) => None,
            Self::SapOrchestration(error) => Some(error.status_code),
            Self::Watsonx(error) => api_status(error),
            Self::WatsonxEmbedding(error) => error.status(),
            Self::WatsonxRerank(error) => error.status(),
            Self::Sagemaker(error) => Some(error.status_code),
            Self::SagemakerFailure(error) => error.status_code(),
            Self::Dispatch(_)
            | Self::SapLookup(_)
            | Self::SapAuth(_)
            | Self::MissingVocabulary
            | Self::Encoding { .. }
            | Self::FieldType { .. }
            | Self::Encode { .. }
            | Self::Decode { .. }
            | Self::Transport { .. }
            | Self::Url { .. }
            | Self::Headers { .. } => None,
        }
    }
}
