use betterlite_context::local_ttl_cache::CacheError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DatabricksOauthError {
    #[error("Databricks App OAuth config is missing required field(s): {}", .0.join(", "))]
    MissingFields(Vec<&'static str>),
    #[error("'databricks_oauth' must be a mapping of OAuth settings")]
    BlockNotMapping,
    #[error("Databricks App OAuth token URL is not a valid URL")]
    TokenUrl(#[source] url::ParseError),
    #[error("the Databricks App OAuth client credentials cannot be carried in a header")]
    CredentialHeader(#[source] InvalidHeaderValue),
    #[error("Databricks App OAuth token request failed with status {status}")]
    TokenStatus { status: u16 },
    #[error("Databricks App OAuth token request failed")]
    TokenRequest(#[source] TransportError),
    #[error("Databricks App OAuth token response is not a JSON object")]
    TokenResponse(#[source] serde_json::Error),
    #[error("Databricks App OAuth token response missing 'access_token'")]
    MissingAccessToken,
    #[error("the Databricks App OAuth token cache failed")]
    Cache(#[source] CacheError),
}
