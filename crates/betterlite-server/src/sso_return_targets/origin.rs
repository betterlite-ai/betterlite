use betterlite_http::user_url::error::UrlParseError;
use betterlite_http::user_url::split::{Parsed, urlparse};

use super::SsoReturnTargetError;

const HTTPS_PORT: u16 = 443;
const HTTP_PORT: u16 = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

// SSOAuthenticationHandler._origin: (scheme, hostname, parsed.port or the scheme's default), both lowercased
pub fn origin(url: &str) -> Result<Origin, UrlParseError> {
    let parsed: Parsed = urlparse(url, "")?;
    let host = parsed.hostname().unwrap_or_default();
    let port = parsed.port()?.unwrap_or(if parsed.scheme == "https" {
        HTTPS_PORT
    } else {
        HTTP_PORT
    });
    let scheme = parsed.scheme;
    Ok(Origin { scheme, host, port })
}

// _validate_return_to: an absent control plane ignores the absolute target, a mismatch raises, a match is honored
pub fn validate_return_to(
    return_to: &str,
    control_plane_url: Option<&str>,
) -> Result<bool, SsoReturnTargetError> {
    let Some(configured) = control_plane_url else {
        return Ok(false);
    };
    if origin(return_to)? != origin(configured)? {
        return Err(SsoReturnTargetError::OriginMismatch);
    }
    Ok(true)
}
