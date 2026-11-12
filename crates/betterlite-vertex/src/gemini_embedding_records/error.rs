use betterlite_models::billing_text::error::BillingTextError;
use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeminiEmbeddingError {
    #[error("{reason}")]
    Rejected { reason: String },
    #[error("{reason}")]
    Reply { reason: &'static str },
    #[error("the token count {value} is outside the supported range")]
    TokenRange { value: i64 },
    #[error("the token counts overflow")]
    TokenOverflow,
    #[error("the embeddings reply holds more entries than the index can address")]
    IndexOverflow,
    #[error("the embeddings text could not be formatted: {source}")]
    Formatting {
        #[source]
        source: BillingTextError,
    },
    #[error("the embeddings text could not be counted: {source}")]
    Counting {
        #[source]
        source: CountError,
    },
}
