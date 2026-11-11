use betterlite_models::usage_sum::UsageSumError;
use rust_decimal::Error as DecimalError;
use thiserror::Error;

use crate::api::responses_usage::ResponsesUsageError;

#[derive(Debug, Error)]
pub enum RealtimeCostError {
    #[error("the realtime event has no {field} field")]
    MissingField { field: &'static str },
    #[error("the realtime event field {field} must be {expected}")]
    Shape {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the realtime usage could not be converted: {source}")]
    Conversion {
        #[from]
        source: ResponsesUsageError,
    },
    #[error("the realtime usage could not be aggregated: {source}")]
    Aggregation {
        #[from]
        source: UsageSumError,
    },
    #[error("the realtime usage JSON could not be built: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the price field {field} is not a number")]
    NotANumber { field: &'static str },
    #[error("the price field {field} does not parse as a decimal: {source}")]
    Rate {
        field: &'static str,
        #[source]
        source: DecimalError,
    },
    #[error("the realtime cost arithmetic overflowed")]
    Overflow,
}
