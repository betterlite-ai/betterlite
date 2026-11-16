use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::decisions::openai::response::ResponseConversionError;
use betterlite_openai::decisions::transformation::requests::RequestConversionError;
use betterlite_openai::realtime::error::RealtimeError;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_redact::error::RedactionError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpenAiAssistantError {
    #[error(transparent)]
    Assistants(#[from] AssistantsError),
    #[error(transparent)]
    Threads(#[from] ThreadsError),
    #[error(transparent)]
    Runs(#[from] ThreadRunsError),
    #[error(transparent)]
    DecisionsRequest(#[from] RequestConversionError),
    #[error(transparent)]
    DecisionsPayload(#[from] serde_json::Error),
    #[error(transparent)]
    DecisionsResponse(#[from] ResponseConversionError),
    #[error(transparent)]
    Realtime(#[from] RealtimeError),
    #[error(transparent)]
    Redaction(#[from] RedactionError),
    #[error("the openai assistant composition runs only assistant and session operations")]
    UnsupportedOperation,
}
