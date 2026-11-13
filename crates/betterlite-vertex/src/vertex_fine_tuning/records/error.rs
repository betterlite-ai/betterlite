use betterlite_openai::fine_tuning::error::FineTuneError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TuningRecordError {
    #[error("the tuning job record is not valid JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the tuning job response has no state")]
    MissingState,
    #[error("the {field} hyperparameter is not a value the record can carry")]
    Hyperparameter { field: &'static str },
    #[error("the tuning job record does not match the OpenAI job contract: {source}")]
    Job {
        #[source]
        source: FineTuneError,
    },
}
