use std::str;

use http::HeaderMap;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::error::Category;

use super::error::TritonError;

// a derived struct also reads a JSON array by position, but a flattened field reads only a JSON object, as LiteLLM's key lookups do
#[derive(Debug, Deserialize)]
pub(super) struct ObjectOnly<T> {
    #[serde(flatten)]
    pub(super) inner: T,
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
    serde_json::from_str::<ObjectOnly<T>>(text)
        .map(|decoded| decoded.inner)
        .map_err(|source| match source.classify() {
            Category::Syntax | Category::Eof => TritonError::Api {
                status_code,
                message: text.to_owned(),
                headers: HeaderMap::new(),
            },
            Category::Data | Category::Io => TritonError::ReplyShape { source },
        })
}
