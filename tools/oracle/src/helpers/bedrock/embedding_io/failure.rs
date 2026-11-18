use std::io;

use betterlite_bedrock::bedrock_embedding_io::error::EmbeddingIoError;
use betterlite_bedrock::signing::error::SigningError;
use betterlite_error::error::details::{TimeoutDetails, TimeoutStatus};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_oracle_helper_api::error::HelperError;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Embedding(#[from] EmbeddingIoError),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Runtime(#[from] io::Error),
    #[error("the fixture clock is out of range")]
    Clock,
}

pub(crate) fn timeout(asynchronous: bool) -> Box<HelperError> {
    let message = if asynchronous {
        "litellm.Timeout: Connection timed out. Timeout passed=None, time taken=<seconds> seconds"
    } else {
        "litellm.Timeout: Connection timed out after None seconds."
    };
    let error_class = ErrorClass::Timeout(Box::new(TimeoutDetails {
        context: ErrorContext {
            message: message.to_owned(),
            model: Some("default-model-name".to_owned()),
            provider: Some("litellm-httpx-handler".to_owned()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        },
        original_status: TimeoutStatus(408),
    }));
    Box::new(HelperError {
        class: "Timeout".to_owned(),
        status: Some(error_class.metadata().status_code),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_owned(),
    })
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(HelperError {
            class: "EmbeddingIoError".to_owned(),
            status: failure_status(&failure),
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

const fn failure_status(failure: &Failure) -> Option<u16> {
    match failure {
        Failure::Embedding(error) => error.status(),
        Failure::Signing(_) | Failure::Json(_) | Failure::Runtime(_) | Failure::Clock => None,
    }
}
