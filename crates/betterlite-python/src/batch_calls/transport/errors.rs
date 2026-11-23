use betterlite_openai::batches::error::{BatchesError, SdkError};
use betterlite_openai::xai_batches::error::XaiBatchesError;

use crate::batch_calls::error::DispatchFailure;

// one shape for both executors: a provider status keeps its own message, anything else is a connection failure
fn failure(status: Option<u16>, message: String) -> DispatchFailure {
    match status {
        Some(status) => DispatchFailure::Status { status, message },
        None => DispatchFailure::Connection(message),
    }
}

impl From<BatchesError> for DispatchFailure {
    fn from(error: BatchesError) -> Self {
        // an SDK status keeps the provider's message; every other SDK failure never reached it
        if let BatchesError::Sdk {
            source: SdkError::Status {
                status, message, ..
            },
        } = &error
        {
            return DispatchFailure::Status {
                status: *status,
                message: message.clone(),
            };
        }
        failure(None, error.to_string())
    }
}

impl From<XaiBatchesError> for DispatchFailure {
    fn from(error: XaiBatchesError) -> Self {
        match &error {
            XaiBatchesError::Status {
                status, message, ..
            } => DispatchFailure::Status {
                status: *status,
                message: message.clone(),
            },
            XaiBatchesError::MissingApiKey => failure(Some(401), error.to_string()),
            XaiBatchesError::InputFileRequired => failure(Some(400), error.to_string()),
            XaiBatchesError::Limit(_)
            | XaiBatchesError::Decode { .. }
            | XaiBatchesError::Transport { .. }
            | XaiBatchesError::Path { .. }
            | XaiBatchesError::Authorization { .. }
            | XaiBatchesError::BaseUrl { .. }
            | XaiBatchesError::Encode { .. } => failure(None, error.to_string()),
        }
    }
}
