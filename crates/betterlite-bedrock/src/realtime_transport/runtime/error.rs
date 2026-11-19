use std::error::Error as StdError;
use std::fmt::Debug;

use aws_sdk_bedrockruntime::error::{DisplayErrorContext, ProvideErrorMetadata, SdkError};
use thiserror::Error;

use crate::realtime_events::stream::StreamError;
use crate::realtime_transport::error::ProviderClass;

#[derive(Debug, Error)]
pub enum SdkStreamError {
    #[error("{message}")]
    Provider { class_name: String, message: String },
    #[error("the Bedrock SDK client is closed")]
    Closed,
}

impl ProviderClass for SdkStreamError {
    fn provider_class_name(&self) -> Option<&str> {
        match self {
            Self::Provider { class_name, .. } => Some(class_name),
            Self::Closed => None,
        }
    }

    fn provider_message(&self) -> Option<&str> {
        match self {
            Self::Provider { message, .. } => Some(message),
            Self::Closed => None,
        }
    }
}

// The SDK answers a service failure with its modeled code and message, and a transport failure with its display text
pub(super) fn provider_error<E, R>(error: &SdkError<E, R>) -> StreamError<SdkStreamError>
where
    E: ProvideErrorMetadata + StdError + 'static,
    R: Debug,
{
    let failure = error.as_service_error().map_or_else(
        || SdkStreamError::Provider {
            class_name: "SdkError".to_owned(),
            message: DisplayErrorContext(error).to_string(),
        },
        |service| SdkStreamError::Provider {
            class_name: service.code().unwrap_or("SdkError").to_owned(),
            message: service.message().unwrap_or_default().to_owned(),
        },
    );
    StreamError {
        message: failure.to_string(),
        source: Some(failure),
    }
}

pub(super) fn closed_stream() -> StreamError<SdkStreamError> {
    StreamError {
        message: "the Bedrock SDK client is closed".to_owned(),
        source: Some(SdkStreamError::Closed),
    }
}
