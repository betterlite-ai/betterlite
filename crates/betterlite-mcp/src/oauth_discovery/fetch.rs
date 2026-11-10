use std::future::Future;
use std::time::Duration;

use http::{HeaderMap, StatusCode};
use thiserror::Error;

use super::document::{MetadataDocument, decode_metadata_document};
use super::outcome::DiscoveryStatus;
use super::redaction::sanitized_error_text;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct FetchError {
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct FetchedDocument {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub trait MetadataFetcher: Send + Sync {
    fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        timeout: Duration,
    ) -> impl Future<Output = Result<FetchedDocument, FetchError>> + Send;
}

#[must_use]
pub const fn classify_status(status: StatusCode) -> DiscoveryStatus {
    match status.as_u16() {
        404 | 405 | 410 => DiscoveryStatus::NotFound,
        _ => DiscoveryStatus::HttpError(status),
    }
}

pub async fn fetch_oauth_discovery_url<F>(
    fetcher: &F,
    url: &str,
    timeout: Duration,
) -> Result<MetadataDocument, DiscoveryStatus>
where
    F: MetadataFetcher,
{
    let response = fetcher
        .get_json(url, &[], timeout)
        .await
        .map_err(|error| DiscoveryStatus::Transport(sanitized_error_text(&error.message)))?;
    if !response.status.is_success() {
        return Err(classify_status(response.status));
    }
    decode_metadata_document(&response.body).map_err(|error| {
        tracing::debug!(
            reason = %error,
            "the MCP OAuth metadata document was rejected"
        );
        DiscoveryStatus::Invalid
    })
}
