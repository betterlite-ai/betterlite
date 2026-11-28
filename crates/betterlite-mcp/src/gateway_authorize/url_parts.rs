mod tests;

use betterlite_http::user_url::error::UrlParseError;
use betterlite_http::user_url::split::urlparse;
use thiserror::Error;

use crate::gateway_authorize::oauth_error;
use crate::gateway_dcr::contracts::CONNECT_FLOW_COOKIE_PREFIX;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::oauth_state::query as state_query;
use crate::oauth_state::state_cookie;

pub const UNREADABLE_URL_DESCRIPTION: &str = "the gateway could not read the redirect URI";

#[derive(Debug, Error)]
#[error(transparent)]
pub struct UrlPartsError(#[from] UrlParseError);

impl From<UrlPartsError> for OauthError {
    fn from(_error: UrlPartsError) -> Self {
        oauth_error(500, OauthErrorCode::ServerError, UNREADABLE_URL_DESCRIPTION)
    }
}

#[must_use]
pub fn flow_cookie_name(handle: &str) -> String {
    let mut name = String::from(CONNECT_FLOW_COOKIE_PREFIX);
    name.push_str(handle);
    name
}

// oauth_state::state_cookie owns the path fallback and https test; this parse adds the typed error.
pub fn cookie_path_and_secure(base_url: &str) -> Result<(String, bool), UrlPartsError> {
    urlparse(base_url, "").map_err(UrlPartsError::from)?;
    Ok(state_cookie::cookie_path_and_secure(base_url))
}

// oauth_state::query owns the codec; this parse adds the typed error it cannot report.
pub fn append_query_params(url: &str, extra: &[(String, String)]) -> Result<String, UrlPartsError> {
    urlparse(url, "").map_err(UrlPartsError::from)?;
    Ok(state_query::append_query_params(url, extra))
}

#[must_use]
pub fn relative_request_url(path: &str, query: Option<&str>) -> String {
    let Some(present) = query.filter(|text| !text.is_empty()) else {
        return path.to_owned();
    };
    format!("{path}?{present}")
}

pub fn origin_only(url: &str) -> Result<String, UrlPartsError> {
    let parsed = urlparse(url, "")?;
    Ok(if parsed.netloc.is_empty() {
        String::new()
    } else {
        format!("{}://{}", parsed.scheme, parsed.netloc)
    })
}
