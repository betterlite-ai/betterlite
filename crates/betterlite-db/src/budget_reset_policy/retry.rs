use crate::database_faults::ErrorType;
use crate::database_faults::reconnect::RetryPolicy;

// DB_RETRY_SAFE_ERROR_TYPES = (httpx.ConnectError,): only a connect failure proves the statement never reached Postgres.
pub const DB_RETRY_SAFE_ERROR_TYPES: &[ErrorType] = &[ErrorType::Connect];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryMode {
    Read,
    Write,
}

impl RetryMode {
    // A read has nothing to double-apply, so it replays every transport error; a write replays only the safe class.
    #[must_use]
    pub const fn policy(self, reason: &str) -> RetryPolicy<'_> {
        match self {
            Self::Read => RetryPolicy {
                reason,
                retry_safe_error_types: None,
                timeout_seconds: None,
                lock_timeout_seconds: None,
            },
            Self::Write => RetryPolicy {
                reason,
                retry_safe_error_types: Some(DB_RETRY_SAFE_ERROR_TYPES),
                timeout_seconds: None,
                lock_timeout_seconds: None,
            },
        }
    }
}

mod tests;
