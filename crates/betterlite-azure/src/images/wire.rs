use betterlite_models::decisions::Json;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;

use crate::image_transport::json::{JsonMap, JsonValue};

use super::error::ImageError;

// B535 sends the body as its own JSON map, and the logging hooks and reply defaults read B004's Json
pub(super) fn wire_map(fields: &IndexMap<String, Json>) -> JsonMap {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), wire_value(value)))
        .collect()
}

fn wire_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(wire_value).collect()),
        Json::Object(fields) => JsonValue::Object(wire_map(fields)),
    }
}

pub(super) fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, ImageError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header =
            HeaderName::from_bytes(name.as_bytes()).map_err(|error| ImageError::Header {
                message: error.to_string(),
            })?;
        let text = HeaderValue::from_str(value).map_err(|error| ImageError::Header {
            message: error.to_string(),
        })?;
        map.insert(header, text);
    }
    Ok(map)
}

pub(super) fn text_value(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, ImageError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ImageError::Field { field }),
    }
}
