use betterlite_openai::batches::error::{BatchesError, SdkError};
use betterlite_openai::xai_batches::error::XaiBatchesError;

use crate::batch_calls::error::DispatchFailure;

impl From<BatchesError> for DispatchFailure {
    fn from(error: BatchesError) -> Self {
        match &error {
            BatchesError::Sdk {
                source: SdkError::Status {
                    status, message, ..
                },
            } => Self::Status {
                status: *status,
                message: message.clone(),
            },
            BatchesError::NotInitialized
            | BatchesError::NotAsync
            | BatchesError::NotSync
            | BatchesError::Sdk { .. }
            | BatchesError::Invalid { .. } => Self::Connection(error.to_string()),
        }
    }
}

impl From<XaiBatchesError> for DispatchFailure {
    fn from(error: XaiBatchesError) -> Self {
        match &error {
            XaiBatchesError::Status {
                status, message, ..
            } => Self::Status {
                status: *status,
                message: message.clone(),
            },
            XaiBatchesError::MissingApiKey => Self::Status {
                status: 401,
                message: error.to_string(),
            },
            XaiBatchesError::InputFileRequired => Self::Status {
                status: 400,
                message: error.to_string(),
            },
            XaiBatchesError::Limit(_)
            | XaiBatchesError::Decode { .. }
            | XaiBatchesError::Transport { .. }
            | XaiBatchesError::Path { .. }
            | XaiBatchesError::Authorization { .. }
            | XaiBatchesError::BaseUrl { .. }
            | XaiBatchesError::Encode { .. } => Self::Connection(error.to_string()),
        }
    }
}
