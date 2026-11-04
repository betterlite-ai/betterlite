use serde_json::{Map, Value};

use super::ContentError;
use super::debug::{display_text_value, format_diagnostic_value};

pub(super) fn input_file(file: Option<&Value>) -> Value {
    let mut output = Map::new();
    output.insert("type".to_owned(), Value::String("input_file".to_owned()));
    if let Some(Value::Object(fields)) = file {
        for key in ["file_id", "file_data", "filename"] {
            if let Some(value) = fields.get(key) {
                output.insert(key.to_owned(), value.clone());
            }
        }
    }
    Value::Object(output)
}

pub fn image_and_file_fields(item: &Map<String, Value>) -> Result<Value, ContentError> {
    if item.get("type").and_then(Value::as_str) == Some("file") {
        return Ok(input_file(item.get("file")));
    }
    let image_url = item
        .get("image_url")
        .and_then(|value| match value {
            Value::String(url) => Some((url.as_str(), None)),
            Value::Object(fields) => fields
                .get("url")
                .and_then(Value::as_str)
                .map(|url| (url, fields.get("detail"))),
            _ => None,
        })
        .ok_or_else(|| ContentError::InvalidImageUrl {
            value: item.get("image_url").map_or_else(
                || "None".to_owned(),
                |value| match value {
                    Value::Null => "None".to_owned(),
                    Value::Object(_) => format_diagnostic_value(value),
                    other => display_text_value(other),
                },
            ),
        })?;
    let mut result = Map::new();
    result.insert("type".to_owned(), Value::String("input_image".to_owned()));
    result.insert(
        "image_url".to_owned(),
        Value::String(image_url.0.to_owned()),
    );
    let detail = image_url.1.filter(|detail| match detail {
        Value::Null => false,
        Value::String(text) => !text.is_empty(),
        _ => true,
    });
    result.insert(
        "detail".to_owned(),
        detail
            .cloned()
            .unwrap_or_else(|| Value::String("auto".to_owned())),
    );
    Ok(Value::Object(result))
}
