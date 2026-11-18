use std::io;

use betterlite_bedrock::bedrock_embedding_io::error::EmbeddingIoError;
use betterlite_bedrock::embedding_jobs::error::EmbeddingJobsError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::error::RedactionError;
use thiserror::Error;

mod env;
mod fixture;
mod input;
mod view;

pub(crate) struct Op;

#[derive(Debug, Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Status(#[from] EmbeddingJobsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Credentials(#[from] EmbeddingIoError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Redaction(#[from] RedactionError),
    #[error("the fixture clock is out of range")]
    Fixture,
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let call = input.call();
        let execution = fixture::execute(call).map_err(Box::<HelperError>::from)?;
        Ok(view::render(&execution))
    }
}

mod tests;
