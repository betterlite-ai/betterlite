mod tests;

use betterlite_http::user_url::split::{Parsed, UrlParseError, urlparse};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use thiserror::Error;

use crate::gateway_authorize::oauth_error;
use crate::gateway_dcr::contracts::CONNECT_FLOW_COOKIE_PREFIX;
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};

pub const UNREADABLE_URL_DESCRIPTION: &str = "the gateway could not read the redirect URI";

const FORM_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

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

pub fn cookie_path_and_secure(base_url: &str) -> Result<(String, bool), UrlPartsError> {
    let parsed = urlparse(base_url, "")?;
    let secure = parsed.scheme == "https";
    let path = if parsed.path.is_empty() {
        "/".to_owned()
    } else {
        parsed.path
    };
    Ok((path, secure))
}

pub fn append_query_params(url: &str, extra: &[(String, String)]) -> Result<String, UrlPartsError> {
    let parsed = urlparse(url, "")?;
    let mut pairs = parse_qsl(&parsed.query);
    pairs.extend(extra.iter().cloned());
    let query = urlencode(&pairs);
    Ok(Parsed { query, ..parsed }.unparse())
}

#[must_use]
pub fn relative_request_url(path: &str, query: Option<&str>) -> String {
    let Some(query) = query.filter(|query| !query.is_empty()) else {
        return path.to_owned();
    };
    format!("{path}?{query}")
}

pub fn origin_only(url: &str) -> Result<String, UrlPartsError> {
    let parsed = urlparse(url, "")?;
    Ok(if parsed.netloc.is_empty() {
        String::new()
    } else {
        format!("{}://{}", parsed.scheme, parsed.netloc)
    })
}

pub(crate) fn urlencode(pairs: &[(String, String)]) -> String {
    let mut query = String::new();
    for (name, value) in pairs {
        if !query.is_empty() {
            query.push('&');
        }
        query.push_str(&form_encode(name));
        query.push('=');
        query.push_str(&form_encode(value));
    }
    query
}

fn form_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for (index, segment) in value.split(' ').enumerate() {
        if index > 0 {
            encoded.push('+');
        }
        encoded.extend(utf8_percent_encode(segment, FORM_SAFE));
    }
    encoded
}

fn parse_qsl(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|piece| !piece.is_empty())
        .map(|piece| {
            let (name, value) = piece.split_once('=').unwrap_or((piece, ""));
            (form_decode(name), form_decode(value))
        })
        .collect()
}

fn form_decode(text: &str) -> String {
    let mut decoded = String::new();
    for (index, segment) in text.split('+').enumerate() {
        if index > 0 {
            decoded.push(' ');
        }
        decoded.push_str(&percent_decode_str(segment).decode_utf8_lossy());
    }
    decoded
}
