use serde::Serialize;

use crate::fields::status::LoggingStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum DataDogStatus {
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "warning")]
    Warn,
    #[serde(rename = "error")]
    Error,
}

impl DataDogStatus {
    #[must_use]
    pub const fn from_logging_status(status: LoggingStatus) -> Self {
        match status {
            LoggingStatus::Success => Self::Info,
            LoggingStatus::Failure => Self::Error,
        }
    }
}
