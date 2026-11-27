use std::error::Error as StdError;

use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::resolution::ResolutionError;
use thiserror::Error;

// A request record LiteLLM's pydantic models reject before the provider is ever resolved.
#[derive(Debug, Error)]
pub enum RecordError {
    #[error("the {field} record does not match its shape: {source}")]
    Invalid {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    // RealtimeTranscriptionSessionRequest(model=model, **transcription_session) raises a TypeError on a duplicate
    #[error("the transcription session names a root model and the call passes model= too")]
    ConflictingModel,
}

// The SDK's failure surface: everything before the provider exchange is shared, and the exchange keeps the dispatcher's error.
#[derive(Debug, Error)]
pub enum RealtimeHttpCallsError<Error>
where
    Error: StdError + 'static,
{
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(transparent)]
    Record(#[from] RecordError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Dispatch(Error),
    #[error("the timeout of {seconds} seconds is not a duration")]
    Timeout { seconds: f64 },
}
