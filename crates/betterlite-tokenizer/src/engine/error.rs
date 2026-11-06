use kitoken::convert::ConversionError;
use kitoken::{DecodeError, EncodeError, InitializationError, RegexError};
use thiserror::Error;
use tokio::task::JoinError;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("the tiktoken vocabulary is invalid")]
    Vocabulary(#[source] ConversionError),
    #[error("the tiktoken split pattern is invalid")]
    Pattern(#[source] RegexError),
    #[error("the tiktoken tokenizer could not be initialized")]
    Initialize(#[source] InitializationError),
    #[error("the text could not be encoded")]
    TiktokenEncode(#[source] EncodeError),
    #[error("the token ids could not be decoded")]
    TiktokenDecode(#[source] DecodeError),
    #[error("the hugging face tokenizer could not be loaded")]
    HuggingFaceLoad(#[source] tokenizers::Error),
    #[error("the text could not be encoded")]
    HuggingFaceEncode(#[source] tokenizers::Error),
    #[error("the token ids could not be decoded")]
    HuggingFaceDecode(#[source] tokenizers::Error),
    #[error("the {encoding} encoding is not bundled and has to be fetched")]
    NotBundled { encoding: &'static str },
    #[error("the token counting pool is closed")]
    PoolClosed,
    #[error("the token counting task failed")]
    Task(#[source] Box<JoinError>),
    #[error("the token counting task returned no result")]
    NoResult,
}
