use std::str::from_utf8;

use reqwest::Response;
use reqwest::header::{AUTHORIZATION, COOKIE, HOST, HeaderMap, HeaderValue, LOCATION};

use crate::user_url::error::UserUrlError;
use crate::user_url::httpx_url::{HttpxUrl, validate_path};
use crate::user_url::join::join;

// httpx Response.has_redirect_location and is_redirect: one of five statuses, with a Location header
pub fn has_redirect_location(response: &Response) -> bool {
    matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308)
        && response.headers().contains_key(LOCATION)
}

// httpx Headers.get: repeated values joined with ", ", decoded as UTF-8 when every header value is, else as Latin-1
pub fn location(headers: &HeaderMap) -> Option<String> {
    let values: Vec<&[u8]> = headers
        .get_all(LOCATION)
        .iter()
        .map(HeaderValue::as_bytes)
        .collect();
    if values.is_empty() {
        return None;
    }
    let utf8 = headers
        .values()
        .all(|value| from_utf8(value.as_bytes()).is_ok());
    let decoded: Vec<String> = values
        .into_iter()
        .map(|bytes| match from_utf8(bytes) {
            Ok(text) if utf8 => text.to_owned(),
            _ => bytes.iter().copied().map(char::from).collect(),
        })
        .collect();
    Some(decoded.join(", "))
}

// httpx Client._redirect_url
pub fn redirect_url(request: &HttpxUrl, location: &str) -> Result<HttpxUrl, UserUrlError> {
    let mut target = HttpxUrl::parse(location).map_err(UserUrlError::LocationHeader)?;
    if !target.scheme.is_empty() && target.host.is_empty() {
        target.host.clone_from(&request.host);
        validate_path(&target.path, &target.scheme, true)?;
    }
    if target.scheme.is_empty() || target.host.is_empty() {
        target = join(request, location)?;
    }
    if request.fragment.is_some() && target.fragment.is_none() {
        target.fragment.clone_from(&request.fragment);
    }
    Ok(target)
}

fn same_origin(url: &HttpxUrl, other: &HttpxUrl) -> bool {
    url.scheme == other.scheme
        && url.host == other.host
        && url.port_or_default() == other.port_or_default()
}

fn is_https_redirect(url: &HttpxUrl, location: &HttpxUrl) -> bool {
    url.host == location.host
        && url.scheme == "http"
        && url.port_or_default() == Some(80)
        && location.scheme == "https"
        && location.port_or_default() == Some(443)
}

// httpx Client._redirect_headers for a GET that stays a GET
pub fn redirect_headers(headers: &HeaderMap, from: &HttpxUrl, to: &HttpxUrl) -> HeaderMap {
    let mut next = headers.clone();
    if !same_origin(from, to) {
        if !is_https_redirect(from, to) {
            next.remove(AUTHORIZATION);
        }
        next.remove(HOST);
    }
    next.remove(COOKIE);
    next
}
