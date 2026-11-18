use std::io;

use thiserror::Error;

use betterlite_bedrock::bedrock_embedding_io::error::EmbeddingIoError;
use betterlite_bedrock::signing::error::SigningError;
use betterlite_oracle_helper_api::error::HelperError;

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
