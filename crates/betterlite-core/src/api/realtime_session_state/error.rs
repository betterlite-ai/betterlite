use crate::api::realtime_contracts::error::RealtimeContractError;

#[derive(Debug, thiserror::Error)]
pub enum SessionStateError {
    #[error("the realtime frame is not a valid event: {0}")]
    Frame(#[from] RealtimeContractError),
    #[error("the realtime frame must be a JSON object holding an event")]
    NotAnEvent,
    #[error("the transcription usage could not be encoded: {0}")]
    Usage(#[from] serde_json::Error),
}
