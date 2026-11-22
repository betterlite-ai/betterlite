use std::collections::BTreeMap;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_models::decisions::Json;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;

use super::error::AzureMediaError;

// httpx sends a data field as its text form: booleans as true/false, numbers as their JSON digits
pub(super) fn form_parts(
    data: &IndexMap<String, Json>,
) -> Result<Vec<MultipartPart>, AzureMediaError> {
    let mut parts = Vec::with_capacity(data.len());
    for (field, value) in data {
        let text = match value {
            Json::Null => String::new(),
            Json::Bool(flag) => String::from(if *flag { "true" } else { "false" }),
            Json::Number(number) => number.to_string(),
            Json::Text(text) => text.clone(),
            Json::List(_) | Json::Object(_) => {
                return Err(AzureMediaError::FormField {
                    field: field.clone(),
                });
            }
        };
        parts.push(MultipartPart {
            name: field.clone(),
            file_name: None,
            content_type: None,
            data: text.into_bytes(),
        });
    }
    Ok(parts)
}

// httpx decodes a failure body with errors="replace", so the message keeps its U+FFFD text
pub(super) fn lossy_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for part in body.utf8_chunks() {
        text.push_str(part.valid());
        text.extend((!part.invalid().is_empty()).then_some('\u{fffd}'));
    }
    text
}

pub(super) fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, AzureMediaError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            AzureMediaError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| AzureMediaError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}
