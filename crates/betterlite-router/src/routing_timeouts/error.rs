use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum TimeoutError {
    #[error("{field} is not a finite number of seconds: {seconds}")]
    NotFinite { field: String, seconds: f64 },
    #[error("{field} is a negative number of seconds: {seconds}")]
    Negative { field: String, seconds: f64 },
    #[error("{field} is too large to represent as a duration: {seconds}")]
    OutOfRange { field: String, seconds: f64 },
    #[error("the routing deadline has expired")]
    DeadlineExpired,
    #[error("the attempt timeout has expired")]
    AttemptExpired,
    #[error("the routing was cancelled")]
    Cancelled,
    #[error("the routing sources select no timeout")]
    NoTimeout,
}

impl TimeoutError {
    // LiteLLM answers an expired request timeout with 408, and its proxy writes the status again as the error code
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::DeadlineExpired | Self::AttemptExpired => Some(408),
            Self::NotFinite { .. }
            | Self::Negative { .. }
            | Self::OutOfRange { .. }
            | Self::Cancelled
            | Self::NoTimeout => None,
        }
    }

    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        match self {
            Self::DeadlineExpired | Self::AttemptExpired => Some("408"),
            Self::NotFinite { .. }
            | Self::Negative { .. }
            | Self::OutOfRange { .. }
            | Self::Cancelled
            | Self::NoTimeout => None,
        }
    }
}
