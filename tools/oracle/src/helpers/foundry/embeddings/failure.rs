use betterlite_foundry::embeddings::error::FoundryEmbeddingError;
use betterlite_oracle_helper_api::error::HelperError;

// the Python class a failure lands in, with the status LiteLLM sets on the raised object
pub(super) struct Failure {
    pub(super) class: &'static str,
    pub(super) status: Option<u16>,
}

pub(super) fn failure(error: &FoundryEmbeddingError) -> Option<Failure> {
    match error {
        FoundryEmbeddingError::ApiBaseMissing
        | FoundryEmbeddingError::ApiKeyMissing
        | FoundryEmbeddingError::TokenCount { .. } => Some(Failure {
            class: "ValueError",
            status: None,
        }),
        FoundryEmbeddingError::ImageDataMissing | FoundryEmbeddingError::TextDataMissing => {
            Some(Failure {
                class: "Exception",
                status: None,
            })
        }
        FoundryEmbeddingError::Provider { status } => Some(Failure {
            class: "HTTPStatusError",
            status: Some(*status),
        }),
        FoundryEmbeddingError::ReplyError { status } => Some(Failure {
            class: "Exception",
            status: Some(*status),
        }),
        FoundryEmbeddingError::Json { source } if source.is_data() => Some(Failure {
            class: "ResponseValidationError",
            status: None,
        }),
        FoundryEmbeddingError::Json { .. } => Some(Failure {
            class: "JSONDecodeError",
            status: None,
        }),
        FoundryEmbeddingError::DataMissing => Some(Failure {
            class: "KeyError",
            status: None,
        }),
        FoundryEmbeddingError::TooFewEntries { .. } => Some(Failure {
            class: "IndexError",
            status: None,
        }),
        FoundryEmbeddingError::Transport { .. } => Some(Failure {
            class: "TransportError",
            status: None,
        }),
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
