use betterlite_foundry::embeddings::error::FoundryEmbeddingError;
use betterlite_oracle_helper_api::error::HelperError;

// the Python class each failure corresponds to in LiteLLM's conversion path
pub(super) enum Failure {
    Class(&'static str),
    Status(u16),
    Reply(u16),
}

pub(super) fn failure(error: &FoundryEmbeddingError) -> Option<Failure> {
    match error {
        FoundryEmbeddingError::ApiBaseMissing
        | FoundryEmbeddingError::ApiKeyMissing
        | FoundryEmbeddingError::TokenCount { .. } => Some(Failure::Class("ValueError")),
        FoundryEmbeddingError::ImageDataMissing | FoundryEmbeddingError::TextDataMissing => {
            Some(Failure::Class("Exception"))
        }
        FoundryEmbeddingError::Provider { status } => Some(Failure::Status(*status)),
        FoundryEmbeddingError::ReplyError { status } => Some(Failure::Reply(*status)),
        FoundryEmbeddingError::Json { source } if source.is_data() => {
            Some(Failure::Class("ResponseValidationError"))
        }
        FoundryEmbeddingError::Json { .. } => Some(Failure::Class("JSONDecodeError")),
        FoundryEmbeddingError::DataMissing => Some(Failure::Class("KeyError")),
        FoundryEmbeddingError::TooFewEntries { .. } => Some(Failure::Class("IndexError")),
        FoundryEmbeddingError::Transport { .. } => Some(Failure::Class("TransportError")),
        FoundryEmbeddingError::FieldType { .. }
        | FoundryEmbeddingError::Encode { .. }
        | FoundryEmbeddingError::Header { .. }
        | FoundryEmbeddingError::Text(_) => None,
    }
}

pub(super) fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
