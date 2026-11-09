use std::str;

use http::HeaderMap;
use serde::de::DeserializeOwned;
use serde_json::error::Category;

use super::error::TritonError;

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
    serde_json::from_str(text).map_err(|source| match source.classify() {
        Category::Syntax | Category::Eof => TritonError::Api {
            status_code,
            message: text.to_owned(),
            headers: HeaderMap::new(),
        },
        Category::Data | Category::Io => TritonError::ReplyShape { source },
    })
}
