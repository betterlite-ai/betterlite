use betterlite_foundry::embeddings::error::FoundryEmbeddingError;
use betterlite_oracle_helper_api::error::HelperError;

// the Python class a failure lands in, with the status LiteLLM sets on the raised object
pub(super) struct Failure {
    pub(super) class: &'static str,
    pub(super) status: Option<u16>,
}

pub(super) fn failure(error: &FoundryEmbeddingError) -> Option<Failure> {
    let (class, status) = match error {
        FoundryEmbeddingError::ApiBaseMissing
        | FoundryEmbeddingError::ApiKeyMissing
        | FoundryEmbeddingError::TokenCount { .. } => ("ValueError", None),
        FoundryEmbeddingError::ImageDataMissing | FoundryEmbeddingError::TextDataMissing => {
            ("Exception", None)
        }
        FoundryEmbeddingError::Provider { status } => ("HTTPStatusError", Some(*status)),
        FoundryEmbeddingError::ReplyError { status } => ("Exception", Some(*status)),
        FoundryEmbeddingError::Json { source } if source.is_data() => {
            ("ResponseValidationError", None)
        }
        FoundryEmbeddingError::Json { .. } => ("JSONDecodeError", None),
        FoundryEmbeddingError::DataMissing => ("KeyError", None),
        FoundryEmbeddingError::TooFewEntries { .. } => ("IndexError", None),
        FoundryEmbeddingError::Transport { .. } => ("TransportError", None),
        FoundryEmbeddingError::FieldType { .. }
        | FoundryEmbeddingError::Encode { .. }
        | FoundryEmbeddingError::Header { .. }
        | FoundryEmbeddingError::Text(_) => return None,
    };
    Some(Failure { class, status })
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
