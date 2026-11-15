use betterlite_models::decisions::text::TextError;
use thiserror::Error;

use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_options::error::GeminiOptionsError;
use crate::gemini_request::error::BodyError;

#[derive(Debug, Error)]
pub enum RowError<Mapped> {
    #[error("`{0}` must be {1}")]
    Field(&'static str, &'static str),
    #[error("`input` is required on /v1/embeddings batch requests, but was not provided")]
    MissingInput,
    #[error("`input` on /v1/embeddings batch requests must not be empty")]
    EmptyInput,
    #[error(transparent)]
    Embedding(#[from] GeminiEmbeddingError),
    #[error(transparent)]
    Options(#[from] GeminiOptionsError),
    #[error(transparent)]
    Body(#[from] BodyError),
    #[error("a Vertex batch row could not be written")]
    Dump(#[source] TextError),
    #[error("the injected row mapper failed")]
    Mapper(#[source] Mapped),
}
