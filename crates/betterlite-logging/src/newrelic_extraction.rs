use std::num::ParseFloatError;

pub mod content;
pub mod ids;
pub mod messages;
pub mod request_params;
pub mod response_fields;
pub mod sources;
pub mod timing;
pub mod trace_context;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum ExtractionError {
    #[error("a timestamp cannot be read as a number of seconds")]
    Timestamp(#[source] ParseFloatError),
    #[error("response_time cannot be read as a float")]
    ResponseTime(#[source] ParseFloatError),
    #[error("a timestamp is outside the millisecond range")]
    TimestampRange,
    #[error("a request message is not an object")]
    RequestMessageShape,
    #[error("a response choice is not an object or carries no message object")]
    ChoiceShape,
}
