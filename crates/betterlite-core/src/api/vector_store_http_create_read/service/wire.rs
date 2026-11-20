use std::collections::BTreeMap;

use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::vector_store_http_write::error::WriteError;

// a caller header spelled in another case goes first as its own line, since httpx keeps both values of a name
pub(super) fn wire_headers(
    headers: &OpenAiHeaders,
    caller: Option<&BTreeMap<String, String>>,
) -> Result<HeaderMap, WriteError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        if caller.is_some_and(|extra| extra.contains_key(name)) {
            append_header(&mut map, name, value)?;
        }
    }
    for (name, value) in headers {
        if !caller.is_some_and(|extra| extra.contains_key(name)) {
            append_header(&mut map, name, value)?;
        }
    }
    Ok(map)
}

fn append_header(map: &mut HeaderMap, name: &str, value: &SecretString) -> Result<(), WriteError> {
    let header_name =
        HeaderName::from_bytes(name.as_bytes()).map_err(|source| WriteError::HeaderName {
            name: name.to_owned(),
            source,
        })?;
    let header_value =
        HeaderValue::from_str(value.expose_secret()).map_err(|source| WriteError::HeaderValue {
            name: name.to_owned(),
            source,
        })?;
    map.append(header_name, header_value);
    Ok(())
}
