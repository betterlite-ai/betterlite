use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PrePostError {
    #[error("the raw request was captured instead of sent")]
    RawRequestCaptured,
    #[error("{message}")]
    CallbackFailed { message: String },
}
