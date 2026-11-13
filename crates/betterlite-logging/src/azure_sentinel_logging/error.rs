use http::header::InvalidHeaderValue;
use thiserror::Error;

use crate::batch::logger::SettingsError;
use crate::datadog_logging::transport::TransportFailure;
use crate::private_json::error::PrivateJsonError;

#[derive(Debug, Error)]
pub enum SentinelError {
    #[error(
        "AZURE_SENTINEL_DCR_IMMUTABLE_ID is required. Set it as an environment variable or pass dcr_immutable_id parameter."
    )]
    MissingDcrImmutableId,
    #[error(
        "AZURE_SENTINEL_ENDPOINT is required. Set it as an environment variable or pass endpoint parameter."
    )]
    MissingEndpoint,
    #[error(
        "AZURE_SENTINEL_TENANT_ID or AZURE_TENANT_ID is required. Set it as an environment variable or pass tenant_id parameter."
    )]
    MissingTenantId,
    #[error(
        "AZURE_SENTINEL_CLIENT_ID or AZURE_CLIENT_ID is required. Set it as an environment variable or pass client_id parameter."
    )]
    MissingClientId,
    #[error(
        "AZURE_SENTINEL_CLIENT_SECRET or AZURE_CLIENT_SECRET is required. Set it as an environment variable or pass client_secret parameter."
    )]
    MissingClientSecret,
    #[error("the batch settings could not be read")]
    Batch(#[from] SettingsError),
    #[error("Failed to get OAuth2 token: {status} - {detail}")]
    TokenStatus { status: u16, detail: String },
    #[error("the OAuth2 token response is not a token document")]
    TokenBody(#[source] serde_json::Error),
    #[error("OAuth2 token response did not contain access_token")]
    MissingAccessToken,
    #[error("the Azure Sentinel request could not be completed")]
    Transport(#[from] TransportFailure),
    #[error("the Azure Sentinel batch could not be serialized")]
    Json(#[from] PrivateJsonError),
    #[error("the OAuth2 token is not a valid header value")]
    Authorization(#[source] InvalidHeaderValue),
}
