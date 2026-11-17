use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum TranscribeLoggingError {
    #[error("the Transcribe logging state was taken by another dispatcher")]
    StateTaken,
    #[error("the standard logging payload could not be built")]
    Payload,
    #[error("the standard logging payload could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
