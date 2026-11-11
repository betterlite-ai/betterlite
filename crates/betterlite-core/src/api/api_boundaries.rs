use thiserror::Error;

mod tests;

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
    pub const fn check_json(
        &self,
        boundary: JsonBoundary,
    ) -> Result<ApiBoundariesResult, ApiLimitError> {
        if boundary.known_bytes > self.max_json_bytes {
            return Err(ApiLimitError::JsonTooLarge {
                observed: boundary.known_bytes,
                limit: self.max_json_bytes,
            });
        }
        Ok(ApiBoundariesResult {
            observed: boundary.known_bytes,
            limit: self.max_json_bytes,
        })
    }

    pub const fn check_event(
        &self,
        boundary: EventBoundary,
    ) -> Result<ApiBoundariesResult, ApiLimitError> {
        if boundary.known_bytes > self.max_event_bytes {
            return Err(ApiLimitError::EventTooLarge {
                observed: boundary.known_bytes,
                limit: self.max_event_bytes,
            });
        }
        Ok(ApiBoundariesResult {
            observed: boundary.known_bytes,
            limit: self.max_event_bytes,
        })
    }

    pub const fn check_items(
        &self,
        boundary: ItemsBoundary,
    ) -> Result<ApiBoundariesResult, ApiLimitError> {
        if boundary.known_count > self.max_items {
            return Err(ApiLimitError::TooManyItems {
                observed: boundary.known_count,
                limit: self.max_items,
            });
        }
        Ok(ApiBoundariesResult {
            observed: boundary.known_count,
            limit: self.max_items,
        })
    }

    pub const fn check(
        &self,
        input: ApiBoundariesInput,
    ) -> Result<ApiBoundariesResult, ApiLimitError> {
        match input {
            ApiBoundariesInput::Json(boundary) => self.check_json(boundary),
            ApiBoundariesInput::Event(boundary) => self.check_event(boundary),
            ApiBoundariesInput::Items(boundary) => self.check_items(boundary),
        }
    }
}
