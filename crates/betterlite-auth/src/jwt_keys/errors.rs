use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use thiserror::Error;

use super::DecodeError;
use super::http::FetchError;
use crate::auth_cache::AuthCacheError;

#[derive(Debug, Error)]
pub enum KeyError {
    #[error("Missing JWT Public Key URL from environment.")]
    MissingUrl,
    #[error("No matching public key found. keys={keys}, kid={kid}", kid = kid.as_deref().unwrap_or("None"))]
    NoMatchingKey { keys: String, kid: Option<String> },
    #[error("{message}")]
    Unreachable { message: String },
    #[error("JWT Auth: OIDC discovery endpoint {url} returned status {status}: {body}")]
    DiscoveryStatus {
        url: String,
        status: u16,
        body: String,
    },
    #[error("JWT Auth: Failed to parse OIDC discovery document at {url}: {source}")]
    DiscoveryParse {
        url: String,
        #[source]
        source: DecodeError,
    },
    #[error("JWT Auth: OIDC discovery document at {url} does not contain a 'jwks_uri' field.")]
    DiscoveryMissingUri { url: String },
    #[error("JWT Auth: JWKS endpoint {url} returned status {status}: {body}")]
    JwksStatus {
        url: String,
        status: u16,
        body: String,
    },
    #[error("Error parsing response: {source}. Check server logs for original response.")]
    JwksParse {
        #[source]
        source: DecodeError,
    },
    // a port failure that is not an outage: the request never left, or its body was over the byte bound
    #[error("JWT Auth: fetching {url} failed: {source}")]
    Fetch {
        url: String,
        #[source]
        source: FetchError,
    },
    #[error(transparent)]
    Cache(#[from] AuthCacheError),
}

// jwks_unavailable_exception: an unreachable IdP gets the retryable 503; every other key failure is a 401.
#[must_use]
pub fn exception_for(error: &KeyError) -> ProxyException {
    match error {
        KeyError::Unreachable { message } => ProxyException::new(ProxyExceptionInit {
            message: format!(
                "Service Unavailable, the identity provider's JWKS endpoint is temporarily unreachable, \
                 so the JWT signature could not be verified. Please retry shortly. Error: {message}"
            ),
            error_type: "auth_provider_unavailable".to_owned(),
            param: Some("None".to_owned()),
            code: ExceptionCode::Status(503),
            headers: BTreeMap::new(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        }),
        KeyError::MissingUrl
        | KeyError::NoMatchingKey { .. }
        | KeyError::DiscoveryStatus { .. }
        | KeyError::DiscoveryParse { .. }
        | KeyError::DiscoveryMissingUri { .. }
        | KeyError::JwksStatus { .. }
        | KeyError::JwksParse { .. }
        | KeyError::Fetch { .. }
        | KeyError::Cache(_) => ProxyException::new(ProxyExceptionInit {
            message: format!("Authentication Error, {error}"),
            error_type: "auth_error".to_owned(),
            param: Some("None".to_owned()),
            code: ExceptionCode::Status(401),
            headers: BTreeMap::new(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        }),
    }
}
