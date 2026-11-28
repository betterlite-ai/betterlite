use betterlite_http::user_url::error::UrlParseError;
use betterlite_http::user_url::split::urlparse;
use thiserror::Error;

const HTTPS_DEFAULT_PORT: u16 = 443;
const OTHER_DEFAULT_PORT: u16 = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Origin {
    scheme: String,
    hostname: String,
    port: u16,
}

#[derive(Debug, Error)]
pub enum ReturnToError {
    #[error("return_to does not match the configured control_plane_url")]
    MismatchedOrigin,
    #[error(transparent)]
    Parse(#[from] UrlParseError),
}

impl ReturnToError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::MismatchedOrigin => Some(400),
            Self::Parse(_) => None,
        }
    }
}

// _validate_return_to: absent control plane ignores the target, a matching origin accepts, else a 400.
pub fn validate_return_to(
    return_to: &str,
    control_plane_url: Option<&str>,
) -> Result<bool, ReturnToError> {
    let Some(configured_url) = control_plane_url else {
        return Ok(false);
    };
    let requested = origin(return_to)?;
    let configured = origin(configured_url)?;
    if requested == configured {
        return Ok(true);
    }
    Err(ReturnToError::MismatchedOrigin)
}

// the handler's _origin closure: lowercase scheme and hostname, with 443 for https and 80 otherwise
fn origin(url: &str) -> Result<Origin, UrlParseError> {
    let parsed = urlparse(url, "")?;
    let scheme = parsed.scheme.to_lowercase();
    let default_port = if scheme == "https" {
        HTTPS_DEFAULT_PORT
    } else {
        OTHER_DEFAULT_PORT
    };
    let port = parsed.port()?.unwrap_or(default_port);
    Ok(Origin {
        scheme,
        hostname: parsed.hostname().unwrap_or_default(),
        port,
    })
}
