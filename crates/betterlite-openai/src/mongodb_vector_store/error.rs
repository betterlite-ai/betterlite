use thiserror::Error;

// LiteLLM raises every MongoDB vector store failure with llm_provider="mongodb"; the caller puts it on the error body
pub const LLM_PROVIDER: &str = "mongodb";

const UNAVAILABLE: &str = "MongoDB sidecar is unavailable. Check its address, health, and logs.";

#[derive(Debug, Error)]
pub enum MongoDbError {
    #[error("{message}")]
    BadRequest { message: String },
    #[error("MongoDB sidecar rejected api_key.")]
    Authentication,
    #[error("{message}")]
    Timeout { message: String },
    #[error("{message}")]
    ServiceUnavailable { message: String },
}

// LiteLLM's get_error_class: the 400 and 408 branches pass the sidecar text through, the others replace it
#[must_use]
pub fn error_for_status(message: String, status: u16) -> MongoDbError {
    match status {
        400 => MongoDbError::BadRequest { message },
        401 => MongoDbError::Authentication,
        408 => MongoDbError::Timeout { message },
        _ => MongoDbError::ServiceUnavailable {
            message: UNAVAILABLE.to_owned(),
        },
    }
}
