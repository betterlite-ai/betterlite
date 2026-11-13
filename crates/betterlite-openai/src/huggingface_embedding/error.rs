use thiserror::Error;

use crate::huggingface_chat::error::HuggingfaceError;

#[derive(Debug, Error)]
pub enum HuggingfaceEmbeddingError {
    #[error(transparent)]
    Huggingface(#[from] HuggingfaceError),
    #[error(
        "invalid task_type {task_type}; expected one of sentence-similarity, feature-extraction, rerank, embed, similarity"
    )]
    InvalidTaskType { task_type: String },
    #[error("the embedding response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the embedding response is not valid JSON")]
    ResponseDecode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the embedding request body could not be serialized")]
    RequestEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the optional parameter {section} does not hold a JSON object")]
    SectionNotObject { section: &'static str },
    #[error("the embedding at index {index} does not hold a vector")]
    EmbeddingShape { index: usize },
    #[error("the input token count overflows")]
    UsageOverflow,
}
