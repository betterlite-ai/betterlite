use betterlite_openai::fine_tuning::error::FineTuneError;
use thiserror::Error;

pub mod request;
pub mod response;
pub mod timestamp;

#[derive(Debug, Error)]
pub enum TuningRecordError {
    #[error("the tuning job record is not valid JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("the tuning job response has no state")]
    MissingState,
    #[error("the {field} hyperparameter is not a value the record can carry")]
    Hyperparameter { field: &'static str },
    #[error("the tuning job record does not match the OpenAI job contract: {0}")]
    Job(#[source] FineTuneError),
}

mod tests;
