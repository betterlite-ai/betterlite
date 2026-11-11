use thiserror::Error;

mod tests;

// resolved once at the boundary; B198's deadline and B206's metadata stay on the call context
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiLimits {
    pub max_json_bytes: usize,
    pub max_event_bytes: usize,
    pub max_items: usize,
}

impl Default for ApiLimits {
    fn default() -> Self {
        // 16 MiB, 1 MiB and 64 Ki items; 0x0001_0000 is 65536
        Self {
            max_json_bytes: 16 * 1024 * 1024,
            max_event_bytes: 1024 * 1024,
            max_items: 0x0001_0000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonBoundary {
    pub known_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventBoundary {
    pub known_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemsBoundary {
    pub known_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiBoundariesInput {
    Json(JsonBoundary),
    Event(EventBoundary),
    Items(ItemsBoundary),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiBoundariesResult {
    pub observed: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ApiLimitError {
    #[error("the request JSON body is {observed} bytes; the maximum is {limit}")]
    JsonTooLarge { observed: usize, limit: usize },
    #[error("the provider event is {observed} bytes; the maximum is {limit}")]
    EventTooLarge { observed: usize, limit: usize },
    #[error("the request collected {observed} items; the maximum is {limit}")]
    TooManyItems { observed: usize, limit: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitErrorFields {
    pub status: u16,
    pub error_type: &'static str,
    pub param: &'static str,
    pub code: &'static str,
}

impl ApiLimitError {
    const fn rejection(input: ApiBoundariesInput, limit: usize) -> Self {
        match input {
            ApiBoundariesInput::Json(boundary) => Self::JsonTooLarge {
                observed: boundary.known_bytes,
                limit,
            },
            ApiBoundariesInput::Event(boundary) => Self::EventTooLarge {
                observed: boundary.known_bytes,
                limit,
            },
            ApiBoundariesInput::Items(boundary) => Self::TooManyItems {
                observed: boundary.known_count,
                limit,
            },
        }
    }

    #[must_use]
    pub const fn fields(self) -> LimitErrorFields {
        match self {
            Self::JsonTooLarge { .. } => LimitErrorFields {
                status: 413,
                error_type: "invalid_request_error",
                param: "body",
                code: "request_too_large",
            },
            Self::EventTooLarge { .. } => LimitErrorFields {
                status: 502,
                error_type: "bad_gateway",
                param: "stream",
                code: "event_too_large",
            },
            Self::TooManyItems { .. } => LimitErrorFields {
                status: 400,
                error_type: "invalid_request_error",
                param: "input",
                code: "too_many_items",
            },
        }
    }
}

impl ApiLimits {
    pub const fn check(
        &self,
        input: ApiBoundariesInput,
    ) -> Result<ApiBoundariesResult, ApiLimitError> {
        let (observed, limit) = match input {
            ApiBoundariesInput::Json(boundary) => (boundary.known_bytes, self.max_json_bytes),
            ApiBoundariesInput::Event(boundary) => (boundary.known_bytes, self.max_event_bytes),
            ApiBoundariesInput::Items(boundary) => (boundary.known_count, self.max_items),
        };
        if observed > limit {
            return Err(ApiLimitError::rejection(input, limit));
        }
        Ok(ApiBoundariesResult { observed, limit })
    }
}
