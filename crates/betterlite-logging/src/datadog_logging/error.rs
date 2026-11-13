use std::io;

use betterlite_env::EnvError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

use crate::datadog::payloads::error::DatadogPayloadError;
use crate::private_json::error::PrivateJsonError;

#[derive(Debug, Error)]
pub enum DatadogError {
    #[error("the Datadog environment could not be read")]
    Environment(#[from] EnvError),
    #[error("DD_API_KEY is not set, set 'DD_API_KEY=<>'")]
    MissingApiKey,
    #[error("DD_SITE is not set in .env, set 'DD_SITE=<>'")]
    MissingSite,
    #[error("the Datadog payload could not be built")]
    Payload(#[from] DatadogPayloadError),
    #[error("the Datadog payload could not be serialized")]
    Json(#[from] PrivateJsonError),
    #[error("the DD-API-KEY header value is not valid")]
    ApiKeyHeader(#[source] InvalidHeaderValue),
    #[error("the Datadog payload could not be compressed")]
    Compress(#[source] io::Error),
    #[error("standard_logging_object not found in kwargs")]
    MissingStandardLoggingObject,
    #[error("the legacy v1 payload needs the call's kwargs")]
    MissingLegacyKwargs,
}
