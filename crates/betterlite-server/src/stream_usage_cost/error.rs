#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum StreamCostError {
    #[error("the cost frame is not a JSON object")]
    NotAnObject,
    #[error("the stream usage field {field} must be an object")]
    DetailsNotAnObject { field: &'static str },
    #[error("the stream usage field {field} must be an integer")]
    NotAnInteger { field: &'static str },
    #[error("the stream usage field {field} must be a number")]
    NotANumber { field: &'static str },
    #[error("the token count {field} does not fit a signed 64-bit integer")]
    Overflow { field: &'static str },
    #[error("the stream usage could not be read: {reason}")]
    Usage { reason: String },
    #[error("the stream usage could not be serialized")]
    Serialize,
}
