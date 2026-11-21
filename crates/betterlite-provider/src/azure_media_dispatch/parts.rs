use std::collections::BTreeMap;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageFiles;
use betterlite_openai::image_edits::file::multipart_parts;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;

use super::MediaPart;
use super::error::AzureMediaError;

// httpx sends a data field as its text form: booleans as true/false, numbers as their JSON digits
pub(super) fn form_parts(data: &IndexMap<String, Json>) -> Result<Vec<MediaPart>, AzureMediaError> {
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
        parts.push(MediaPart {
            field: field.clone(),
            filename: None,
            content_type: None,
            body: text.into_bytes(),
        });
    }
    Ok(parts)
}

// the httpx files argument as the multipart body betterlite-http sends, in the order LiteLLM builds it
pub(super) fn file_parts(files: &ImageFiles) -> Vec<MediaPart> {
    multipart_parts(files)
        .into_iter()
        .map(|part| MediaPart {
            field: part.name,
            filename: part.file_name,
            content_type: part.content_type,
            body: part.data,
        })
        .collect()
}

// the encoder takes the http crate's own part record, so the prepared view is copied into it here
pub(super) fn wire_parts(parts: &[MediaPart]) -> Vec<MultipartPart> {
    parts
        .iter()
        .map(|part| MultipartPart {
            name: part.field.clone(),
            file_name: part.filename.clone(),
            content_type: part.content_type.clone(),
            data: part.body.clone(),
        })
        .collect()
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
