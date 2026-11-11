use betterlite_http::error::HttpError;
use betterlite_openai::embedding::EmbeddingError;
use betterlite_provider::operation::OperationError;

use super::CallError;

pub(crate) fn embedding_error(error: OperationError) -> CallError<EmbeddingError> {
    let route = match error {
        OperationError::Provider { response } => EmbeddingError::Provider { response },
        OperationError::MissingApiKey => EmbeddingError::MissingApiKey,
        OperationError::Encode { source } => EmbeddingError::Encode { source },
        OperationError::Decode { source, .. } => EmbeddingError::Decode { source },
        OperationError::UnavailableAdapter { provider }
        | OperationError::UnsupportedOperation { provider, .. } => {
            EmbeddingError::UnsupportedProvider { model: provider }
        }
        OperationError::Timeout => EmbeddingError::Http {
            source: HttpError::Timeout,
        },
        OperationError::TooLarge { limit } => EmbeddingError::Http {
            source: HttpError::TooLarge { limit },
        },
        kept @ (OperationError::Http { .. }
        | OperationError::InvalidHeader { .. }
        | OperationError::TransformRetry(_)) => {
            return CallError::Operation { source: kept };
        }
    };
    CallError::Route(route)
}
