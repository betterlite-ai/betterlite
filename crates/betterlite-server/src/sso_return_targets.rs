pub mod callback;
pub mod cookie;
pub mod decode;
pub mod login_code;
pub mod origin;
pub mod redirect;
pub mod return_path;

mod tests;

use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_http::user_url::error::UrlParseError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

use self::login_code::CacheWriteError;

#[derive(Debug, Error)]
pub enum SsoReturnTargetError {
    #[error("return_to does not match the configured control_plane_url")]
    OriginMismatch,
    #[error("return_to is not a valid URL: {0}")]
    Parse(#[from] UrlParseError),
    #[error("the cookie value contains a control character")]
    CookieValue,
    #[error("could not draw random bytes for the login code")]
    Entropy(#[source] getrandom::Error),
    #[error("the login code cache write failed")]
    CacheWrite(#[from] CacheWriteError),
    #[error("the redirect response could not be built")]
    Response(#[source] http::Error),
    #[error("the response header could not be built")]
    Header(#[source] InvalidHeaderValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SsoReturnTargetResponse {
    pub status: u16,
    pub error_type: ProxyErrorTypes,
    pub param: Option<&'static str>,
    pub code: &'static str,
}

impl SsoReturnTargetError {
    // The origin mismatch is LiteLLM's HTTPException(400); a return_to its parser rejects is the same 400 here
    #[must_use]
    pub const fn response(&self) -> SsoReturnTargetResponse {
        match self {
            Self::OriginMismatch | Self::Parse(_) | Self::CookieValue => SsoReturnTargetResponse {
                status: 400,
                error_type: ProxyErrorTypes::BadRequestError,
                param: None,
                code: "400",
            },
            Self::Entropy(_) | Self::CacheWrite(_) | Self::Response(_) | Self::Header(_) => {
                SsoReturnTargetResponse {
                    status: 500,
                    error_type: ProxyErrorTypes::InternalServerError,
                    param: None,
                    code: "500",
                }
            }
        }
    }
}
