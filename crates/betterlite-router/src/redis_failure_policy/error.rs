use thiserror::Error;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Redis circuit breaker is open — skipping {operation}")]
pub struct CircuitOpenError {
    pub operation: String,
}
