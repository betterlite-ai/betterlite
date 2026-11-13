use std::num::{ParseIntError, TryFromIntError};

#[derive(Debug, thiserror::Error)]
pub enum DurationError {
    #[error("invalid duration format: {spec}")]
    InvalidFormat { spec: String },
    #[error("unsupported duration unit, passed duration: {spec}")]
    UnsupportedUnit { spec: String },
    #[error("the duration has more digits than a 64-bit quantity holds: {spec}")]
    QuantityOverflow {
        spec: String,
        #[source]
        source: ParseIntError,
    },
    #[error("the duration does not fit in signed 64-bit seconds: {spec}")]
    SecondsOverflow {
        spec: String,
        #[source]
        source: TryFromIntError,
    },
    #[error("monthly resets only support one month, got {quantity}")]
    UnsupportedMonthInterval { quantity: u64 },
    #[error("a computed calendar value overflowed")]
    CalendarOverflow,
    #[error("a computed calendar field does not fit its range: {source}")]
    CalendarField {
        #[source]
        source: TryFromIntError,
    },
    #[error("the calendar value is out of range: {source}")]
    Calendar {
        #[source]
        source: jiff::Error,
    },
    #[error("the timestamp is out of range: {source}")]
    Timestamp {
        #[source]
        source: jiff::Error,
    },
    #[error("the computed instant is outside the supported timestamp range")]
    TimestampOutOfRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurationWarning {
    TrailingInput { rest: String },
    UnrecognizedResetDuration { spec: String },
    UnknownResetUnit { unit: String },
    InvalidTimezone { name: String },
}
