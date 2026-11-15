use thiserror::Error;

use crate::gemini_history::error::HistoryError;
use crate::google_count_tokens::error::GoogleCountError;
use crate::model_policy::error::VertexAiError;

#[derive(Debug, Error)]
pub enum CountTokensError {
    #[error(transparent)]
    Google(#[from] GoogleCountError),
    #[error(transparent)]
    Partner(#[from] VertexAiError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error("a count message cannot be encoded: {source}")]
    Message {
        #[source]
        source: serde_json::Error,
    },
    #[error("the count reply names more tokens than a signed count can carry")]
    Overflow,
}

#[derive(Debug, Error)]
pub enum ModelInfoError {
    #[error("Vertex AI models are not supported yet")]
    NotSupported,
}
