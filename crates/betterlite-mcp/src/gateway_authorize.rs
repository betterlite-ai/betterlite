pub mod authorize;
pub mod connect;
pub mod contract;
pub mod reply;
pub mod scope;
#[cfg(test)]
pub(crate) mod tests;
pub mod url_parts;

use betterlite_http::user_url::split::urlparse;

use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::oauth_resource::challenge::TOKEN_NO_CACHE_HEADERS;
use crate::redirect_trust::shape::is_loopback_redirect_host;

#[must_use]
pub(crate) fn oauth_error(status: u16, error: OauthErrorCode, description: &str) -> OauthError {
    OauthError {
        status,
        error,
        description: description.to_owned(),
    }
}

#[must_use]
pub(crate) fn no_cache_headers() -> Vec<(String, String)> {
    TOKEN_NO_CACHE_HEADERS
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[must_use]
pub(crate) fn is_loopback_uri(redirect_uri: &str) -> bool {
    urlparse(redirect_uri, "").is_ok_and(|parsed| is_loopback_redirect_host(&parsed))
}
