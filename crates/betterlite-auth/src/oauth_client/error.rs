use std::fmt;

use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;
use url::ParseError;
use url::Url;

use crate::oauth_client::OAuthClient;
use crate::oauth_client::document::OAuthDocumentError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthEndpoint {
    Discovery,
    Authorization,
    Token,
    Userinfo,
}

#[derive(Debug, Error)]
pub enum OAuthProviderFailure {
    #[error("the provider request failed: {0}")]
    Transport(#[from] TransportError),
    #[error("the provider endpoint answered with status {status}")]
    Status { status: u16 },
    #[error("the provider document could not be decoded: {0}")]
    Document(#[from] OAuthDocumentError),
    #[error("the token response carries no access token")]
    MissingAccessToken,
}

#[derive(Debug, Error)]
pub enum OAuthClientError {
    #[error("'code' parameter was not found in callback request")]
    MissingCode,
    #[error("Invalid state")]
    InvalidState,
    #[error("the authorization server returned error '{error}': {description:?}")]
    Provider {
        error: String,
        description: Option<String>,
    },
    #[error("the {endpoint} endpoint is not HTTPS and allow_insecure_http is off")]
    Insecure { endpoint: OAuthEndpoint },
    #[error("the {endpoint} endpoint is not a URL: {source}")]
    Endpoint {
        endpoint: OAuthEndpoint,
        #[source]
        source: ParseError,
    },
    #[error("redirect_uri must be provided, either at construction or request time")]
    MissingRedirectUri,
    #[error("the callback URL is not a URL: {0}")]
    CallbackUrl(#[source] ParseError),
    #[error("the client credentials cannot be carried in an Authorization header")]
    Credentials(#[source] InvalidHeaderValue),
    #[error("the discovery document could not be fetched: {0}")]
    Discovery(#[source] OAuthProviderFailure),
    #[error("the token request failed: {0}")]
    Token(#[source] OAuthProviderFailure),
    #[error("the userinfo request failed: {0}")]
    Userinfo(#[source] OAuthProviderFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OAuthErrorResponse {
    pub status: u16,
    pub error_type: ProxyErrorTypes,
    pub param: Option<&'static str>,
    pub code: &'static str,
}

impl fmt::Display for OAuthEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Discovery => "discovery",
            Self::Authorization => "authorization",
            Self::Token => "token",
            Self::Userinfo => "userinfo",
        };
        formatter.write_str(name)
    }
}

impl OAuthClientError {
    #[must_use]
    pub const fn response(&self) -> OAuthErrorResponse {
        let (status, error_type, param, code) = match self {
            Self::MissingCode => (400, ProxyErrorTypes::AuthError, Some("code"), "400"),
            Self::InvalidState => (401, ProxyErrorTypes::AuthError, Some("state"), "401"),
            Self::Provider { .. } | Self::Token(_) | Self::Userinfo(_) => {
                (401, ProxyErrorTypes::AuthError, None, "401")
            }
            Self::Endpoint {
                endpoint: OAuthEndpoint::Discovery,
                ..
            }
            | Self::Insecure { .. }
            | Self::MissingRedirectUri
            | Self::CallbackUrl(_)
            | Self::Credentials(_) => (500, ProxyErrorTypes::InternalServerError, None, "500"),
            Self::Endpoint { .. } | Self::Discovery(_) => {
                (503, ProxyErrorTypes::AuthProviderUnavailable, None, "503")
            }
        };
        OAuthErrorResponse {
            status,
            error_type,
            param,
            code,
        }
    }
}

pub(crate) fn endpoint_url(endpoint: &str, kind: OAuthEndpoint) -> Result<Url, OAuthClientError> {
    Url::parse(endpoint).map_err(|source| OAuthClientError::Endpoint {
        endpoint: kind,
        source,
    })
}

pub(crate) fn require_secure(
    client: &OAuthClient,
    url: &Url,
    endpoint: OAuthEndpoint,
) -> Result<(), OAuthClientError> {
    if client.allow_insecure_http || url.scheme() == "https" {
        return Ok(());
    }
    Err(OAuthClientError::Insecure { endpoint })
}
