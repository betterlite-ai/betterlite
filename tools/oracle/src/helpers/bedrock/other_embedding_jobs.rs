use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::Value;
use thiserror::Error;

mod fixture;
mod input;
mod view;

pub(crate) struct Op;

#[derive(Debug, Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Status(#[from] betterlite_bedrock::embedding_jobs::EmbeddingJobsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the fixture is not usable")]
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
        let answer: Value =
            serde_json::to_value(view::render(&execution)).map_err(Failure::from)?;
        Ok(answer)
    }
}

mod tests;
