use std::io::Error as IoError;
use std::num::TryFromIntError;

use http::header::InvalidHeaderValue;

pub mod aggregation;
pub mod clock;
pub mod delivery;
pub mod drain;
pub mod flush;
pub mod logger;
pub mod queue;
pub mod records;
pub mod region;
pub mod transport;
pub mod wire;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum NewRelicMetricsError {
    #[error("the cost sum overflowed the decimal range")]
    CostSum,
    #[error("a token count sum overflowed")]
    TokenSum,
    #[error("the remaining budget overflowed the decimal range")]
    BudgetSum,
    #[error("the window start does not fit in milliseconds")]
    WindowStart(#[source] TryFromIntError),
    #[error("the window length does not fit in milliseconds")]
    WindowLength(#[source] TryFromIntError),
    #[error(
        "newrelic_api_key is required for NewRelicMetricsLogger; team-scoped metrics never fall back to environment credentials"
    )]
    MissingApiKey,
    #[error("the Metric API payload could not be written as JSON")]
    Encode(#[source] serde_json::Error),
    #[error("the Metric API payload could not be gzipped")]
    Compress(#[source] IoError),
    #[error("the team ingest key cannot be carried in the Api-Key header")]
    ApiKeyHeader(#[source] InvalidHeaderValue),
}
