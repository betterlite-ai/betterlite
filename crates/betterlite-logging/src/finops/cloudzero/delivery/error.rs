use chrono::NaiveDate;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TimestampError {
    #[error("Could not parse timestamp '{timestamp}': {source}")]
    Parse {
        timestamp: String,
        #[source]
        source: jiff::Error,
    },
    #[error(
        "Could not process timestamp '{timestamp}': time/usage_start must be text or a datetime"
    )]
    NotText { timestamp: String },
    #[error("Could not process timestamp '{timestamp}': outside the supported range")]
    OutOfRange { timestamp: String },
}

#[derive(Debug, Error)]
pub enum DeliveryError<TransportError> {
    #[error("sending the CloudZero billing drop for {date} failed")]
    Send {
        date: NaiveDate,
        #[source]
        source: TransportError,
    },
}
