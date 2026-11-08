use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MockError {
    #[error("the mock answer is a rate limit error")]
    RateLimit,
    #[error("the mock answer is a context window exceeded error")]
    ContextWindowExceeded,
    #[error("the mock answer is an internal server error")]
    InternalServerError,
    #[error("the mock answer is a content filter rejection")]
    ContentFilter,
    #[error("the mock answer fails after the stream starts")]
    Streaming,
    #[error("the mock answer is a timeout")]
    Timeout,
    #[error("the mock answer does not fit the {field} field")]
    WrongType { field: &'static str },
    #[error("a mock response object cannot be streamed")]
    ObjectStream,
    #[error("mock_tool_calls needs at least one choice")]
    ToolCallsWithoutChoice,
    #[error("a mock call asks for {count} choices, more than the {limit} betterlite builds")]
    TooManyChoices { count: u32, limit: u32 },
    #[error("the mock embedding answer is an error")]
    EmbeddingFailure,
}

impl MockError {
    // the HTTP status LiteLLM's exception carries; the shared classifier turns it into type and code
    #[must_use]
    pub const fn status(self) -> Option<u16> {
        match self {
            Self::RateLimit => Some(429),
            Self::ContextWindowExceeded
            | Self::ContentFilter
            | Self::ObjectStream
            | Self::TooManyChoices { .. } => Some(400),
            Self::InternalServerError | Self::WrongType { .. } | Self::ToolCallsWithoutChoice => {
                Some(500)
            }
            Self::Streaming => Some(529),
            Self::Timeout => Some(408),
            Self::EmbeddingFailure => None,
        }
    }
}
