#[derive(Debug, Clone, Copy, Eq, PartialEq, thiserror::Error)]
pub enum BoundsError {
    #[error("adaptive limiter bounds must satisfy 1 <= floor <= ceiling, got {floor}..{ceiling}")]
    Invalid { floor: u64, ceiling: u64 },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, thiserror::Error)]
pub enum AcquireError {
    #[error("the adaptive concurrency limiter already holds the maximum number of waiting callers")]
    Overloaded,
    #[error("the caller deadline passed before an adaptive concurrency permit became free")]
    DeadlineExceeded,
}
