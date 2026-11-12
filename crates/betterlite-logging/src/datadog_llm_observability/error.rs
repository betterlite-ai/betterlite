use betterlite_env::EnvError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

use crate::datadog_logging::transport::TransportFailure;

#[derive(Debug, Error)]
pub enum DatadogLlmObsError {
    #[error("the Datadog environment could not be read: {0}")]
    Environment(#[from] EnvError),
    #[error("DD_API_KEY is not set, set 'DD_API_KEY=<>'")]
    MissingApiKey,
    #[error("DD_SITE is not set, set 'DD_SITE=<>', example site = `us5.datadoghq.com`")]
    MissingSite,
    #[error("the flush interval {seconds} is not a usable duration")]
    FlushInterval { seconds: f64 },
    #[error("the DD-API-KEY header value is not valid")]
    ApiKeyHeader(#[source] InvalidHeaderValue),
    #[error("the Datadog LLM Observability intake payload could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the Datadog LLM Observability intake request could not be completed")]
    Transport(#[from] TransportFailure),
    #[error("the Datadog LLM Observability intake answered with status {status}: {detail}")]
    UnexpectedStatus { status: u16, detail: String },
}
