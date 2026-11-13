use std::io;
use std::path::PathBuf;
use std::str::Utf8Error;

use reqwest::header::InvalidHeaderValue;
use thiserror::Error;
use url::ParseError;

use crate::hub_oauth::file_lock::LockError;
use crate::hub_token::HubTokenError;
use crate::proxy::transport_error::TransportError;

#[derive(Debug, Error)]
pub enum OidcError {
    #[error(
        "HF_OIDC_RESOURCE is set but no OIDC id token is available: not running in a supported CI provider (github) and HF_OIDC_ID_TOKEN is not set. Set HF_OIDC_ID_TOKEN to the id token minted by your CI provider, or unset HF_OIDC_RESOURCE."
    )]
    NoIdToken,
    #[error(
        "Cannot request an OIDC id token from GitHub Actions. Make sure the workflow job sets `permissions: id-token: write`."
    )]
    GithubRequest,
    #[error(
        "Cannot reach {url}: offline mode is enabled. To disable it, please unset the `HF_HUB_OFFLINE` environment variable."
    )]
    Offline { url: String },
    #[error("the GitHub Actions request token holds a character a header cannot carry")]
    Header(#[source] InvalidHeaderValue),
    #[error("{url} is not a valid URL")]
    Url {
        url: String,
        #[source]
        source: ParseError,
    },
    #[error("the request to {url} failed")]
    Transport {
        url: String,
        #[source]
        source: TransportError,
    },
    #[error("{url} answered with status {status}")]
    Status { url: String, status: u16 },
    #[error("the answer from {url} is not the expected JSON")]
    Body {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Error)]
pub enum StoredTokensError {
    #[error("the stored tokens file {path} is not UTF-8")]
    Utf8 {
        path: PathBuf,
        #[source]
        source: Utf8Error,
    },
}

#[derive(Debug, Error)]
pub enum RefreshError {
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Stored(#[from] StoredTokensError),
    #[error(transparent)]
    TokenFile(#[from] HubTokenError),
    #[error("{url} is not a valid URL")]
    Url {
        url: String,
        #[source]
        source: ParseError,
    },
    #[error(
        "Cannot reach {url}: offline mode is enabled. To disable it, please unset the `HF_HUB_OFFLINE` environment variable."
    )]
    Offline { url: String },
    #[error("Failed to refresh access token: {0}")]
    Request(#[source] TransportError),
    #[error("Failed to parse response from {url} (status {status})")]
    Unreadable {
        url: String,
        status: u16,
        #[source]
        source: serde_json::Error,
    },
    #[error("Failed to refresh access token: {} - {description}", .error.clone().unwrap_or_else(|| status.to_string()))]
    Rejected {
        status: u16,
        error: Option<String>,
        description: String,
    },
    #[error("the refreshed token could not be saved: {0}")]
    Save(#[source] io::Error),
}
