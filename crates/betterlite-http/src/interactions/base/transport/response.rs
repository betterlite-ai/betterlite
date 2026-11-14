use std::collections::btree_map::Entry;
use std::io::Read;

use reqwest::header::{HeaderMap, HeaderValue};

use crate::interactions::base::{RequestFailure, ResponseHeaders};

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

// httpx reads header bytes as ascii then utf-8, and falls back to iso-8859-1, so none of them fails
fn header_text(value: &HeaderValue) -> String {
    match std::str::from_utf8(value.as_bytes()) {
        Ok(text) => text.to_owned(),
        Err(_) => value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect(),
    }
}

// the response body, read up to the limit so a provider answer cannot grow without bound
pub(crate) fn read_body<R>(reader: R, limit: usize) -> Result<Vec<u8>, RequestFailure>
where
    R: Read,
{
    let cap = u64::try_from(limit.saturating_add(1)).unwrap_or(u64::MAX);
    let mut body = Vec::new();
    reader
        .take(cap)
        .read_to_end(&mut body)
        .map_err(RequestFailure::Body)?;
    if body.len() > limit {
        return Err(RequestFailure::BodyTooLarge { limit });
    }
    Ok(body)
}
