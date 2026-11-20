use std::collections::btree_map::Entry;
use std::str::FromStr as _;
use std::str::from_utf8;

use betterlite_http::interactions::base::{RequestFailure, RequestHeaders, ResponseHeaders};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::{MultipartPart, encode};
use betterlite_http::proxy::outbound::merge_query;
use betterlite_http::proxy::transport_error::TransportError;
use reqwest::Url;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

use super::super::clients::CharacterBody;

// httpx's response.text: UTF-8 with errors="replace", so each invalid sequence reads as one U+FFFD
pub(crate) fn body_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}

// a header value reads as UTF-8 or, failing that, as iso-8859-1, the way httpx decodes one
fn header_text(value: &HeaderValue) -> String {
    from_utf8(value.as_bytes()).map_or_else(
        |_invalid| {
            value
                .as_bytes()
                .iter()
                .map(|byte| char::from(*byte))
                .collect()
        },
        str::to_owned,
    )
}

// a repeated response header joins with ", ", the way HTTP spells the same value twice
pub(crate) fn response_headers(headers: &HeaderMap) -> ResponseHeaders {
    let mut map = ResponseHeaders::new();
    for (name, value) in headers {
        let text = header_text(value);
        match map.entry(name.as_str().to_owned()) {
            Entry::Occupied(mut entry) => {
                let joined = entry.get_mut();
                joined.push_str(", ");
                joined.push_str(&text);
            }
            Entry::Vacant(entry) => {
                entry.insert(text);
            }
        }
    }
    map
}

// what httpx's raise_for_status turns into an HTTPStatusError: its status, its headers and its text
pub(crate) fn status_failure(reply: &UpstreamResponse) -> RequestFailure {
    RequestFailure::Status {
        status_code: reply.status,
        headers: response_headers(&reply.headers),
        body: body_text(&reply.body),
    }
}

pub(crate) fn header_map(headers: &RequestHeaders) -> Result<HeaderMap, RequestFailure> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_str(name).map_err(RequestFailure::HeaderName)?;
        let header_value =
            HeaderValue::from_str(value.expose_secret()).map_err(RequestFailure::HeaderValue)?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

// the GET target: the transform's params join the URL's own, and a key already in the URL wins
pub(crate) fn request_url(text: &str, query: &[(String, String)]) -> Result<Url, RequestFailure> {
    let mut url = Url::parse(text).map_err(RequestFailure::Url)?;
    merge_query(&mut url, query);
    Ok(url)
}

// the transform's files list becomes one multipart body, marked with the boundary it was encoded against
pub(crate) fn multipart_body(
    parts: &[MultipartPart],
    boundary: &str,
) -> Result<CharacterBody, TransportError> {
    let data = encode(boundary, parts)?;
    Ok(CharacterBody::Multipart {
        content_type: format!("multipart/form-data; boundary={boundary}"),
        data,
    })
}
