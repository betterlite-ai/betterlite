use std::str;

use http::HeaderMap;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::value::RawValue;

use super::error::TritonError;

// a derived struct also reads a JSON array by position, so the first byte decides, as LiteLLM's key lookups need an object
pub(super) fn parse_object<T>(raw: &RawValue) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
{
    if !raw.get().starts_with('{') {
        return Err(serde_json::Error::custom("expected a JSON object"));
    }
    serde_json::from_str(raw.get())
}

// an unreadable body carries the response status, while a JSON value of the wrong shape ends as LiteLLM's generic 500
pub(super) fn decode_reply<T>(status_code: u16, body: &[u8]) -> Result<T, TritonError>
where
    T: DeserializeOwned,
{
    let Ok(text) = str::from_utf8(body) else {
        return Err(TritonError::Api {
            status_code,
            message: format!("{} bytes that are not UTF-8", body.len()),
            headers: HeaderMap::new(),
        });
    };
    let raw = serde_json::from_str::<Box<RawValue>>(text).map_err(|_error| TritonError::Api {
        status_code,
        message: text.to_owned(),
        headers: HeaderMap::new(),
    })?;
    parse_object(&raw).map_err(|source| TritonError::ReplyShape { source })
}
