use thiserror::Error;

// the failure facts of the LiteLLM exception each sentinel raises, before the proxy turns them into a body
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MockFailure {
    pub status: Option<u16>,
    pub error_type: Option<&'static str>,
    pub code: Option<&'static str>,
}

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
    #[error("the mock embedding answer is an error")]
    EmbeddingFailure,
}

impl MockError {
    #[must_use]
    pub const fn failure(self) -> MockFailure {
        match self {
            Self::RateLimit => MockFailure {
                status: Some(429),
                error_type: Some("throttling_error"),
                code: Some("429"),
            },
            Self::ContextWindowExceeded | Self::ContentFilter | Self::ObjectStream => MockFailure {
                status: Some(400),
                error_type: None,
                code: None,
            },
            Self::InternalServerError | Self::WrongType { .. } | Self::ToolCallsWithoutChoice => {
                MockFailure {
                    status: Some(500),
                    error_type: Some("internal_server_error"),
                    code: None,
                }
            }
            Self::Streaming => MockFailure {
                status: Some(529),
                error_type: None,
                code: None,
            },
            Self::Timeout => MockFailure {
                status: Some(408),
                error_type: None,
                code: None,
            },
            Self::EmbeddingFailure => MockFailure {
                status: None,
                error_type: None,
                code: None,
            },
        }
    }
}
