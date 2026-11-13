use std::num::ParseFloatError;

use betterlite_env::EnvError;
use http::header::InvalidHeaderValue;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::datadog_logging::error::DatadogError;
use crate::datadog_logging::transport::TransportFailure;

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error(transparent)]
    Environment(#[from] EnvError),
    #[error("the request end time {seconds} does not fit a unix timestamp")]
    Timestamp { seconds: Decimal },
    #[error("the {duration} duration cannot be computed from its timestamps")]
    Duration { duration: &'static str },
    #[error("the metric value could not be read as a number")]
    Value(#[source] ParseFloatError),
    #[error("the Datadog metrics payload could not be serialized")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Gzip(#[from] DatadogError),
    #[error("the Datadog metric key cannot be carried in a header")]
    ApiKeyHeader(#[source] InvalidHeaderValue),
}

#[derive(Debug, Error)]
pub enum SendError {
    #[error(transparent)]
    Build(#[from] MetricsError),
    #[error(transparent)]
    Transport(#[from] TransportFailure),
    #[error("the Datadog metrics API answered with status {status}")]
    Status { status: u16 },
}
