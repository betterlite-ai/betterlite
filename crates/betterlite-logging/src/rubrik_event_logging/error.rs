use thiserror::Error;

use crate::rubrik_moderation::error::ModerationError;

#[derive(Debug, Error)]
pub enum EventPayloadError {
    #[error("the standard logging object is missing from the call details")]
    MissingStandardLoggingObject,
    #[error("the {field} value has the wrong type")]
    FieldType { field: &'static str },
    #[error("the call details could not be read: {source}")]
    CallDetails {
        #[from]
        source: ModerationError,
    },
    #[error("the start_time value is not an RFC 3339 timestamp")]
    StartTime {
        #[source]
        source: chrono::ParseError,
    },
}
