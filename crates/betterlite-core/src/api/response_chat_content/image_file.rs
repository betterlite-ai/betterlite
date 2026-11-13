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
    let (url, detail) = image_url(item)?;
    Ok(image_block(url, detail))
}

fn image_url(item: &Map<String, Value>) -> Result<(&str, Option<&Value>), ContentError> {
    let image_source = item.get("image_url");
    let Some(image_value) = image_source else {
        return Err(invalid_url(image_source));
    };
    match image_value {
        Value::String(url) => Ok((url, None)),
        Value::Object(fields) => match fields.get("url") {
            Some(Value::String(url)) => Ok((url, fields.get("detail"))),
            Some(
                Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::Array(_)
                | Value::Object(_),
            )
            | None => Err(invalid_url(Some(image_value))),
        },
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => {
            Err(invalid_url(Some(image_value)))
        }
    }
}

fn invalid_url(image_url: Option<&Value>) -> ContentError {
    ContentError::InvalidImageUrl {
        value: image_url.map_or_else(
            || "None".to_owned(),
            |value| match value {
                Value::Null => "None".to_owned(),
                Value::Object(_) => format_diagnostic_value(value),
                other
                @ (Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
                    display_text_value(other)
                }
            },
        ),
    }
}

fn image_block(url: &str, detail: Option<&Value>) -> Value {
    let mut result = Map::new();
    result.insert("type".to_owned(), Value::String("input_image".to_owned()));
    result.insert("image_url".to_owned(), Value::String(url.to_owned()));
    let selected_detail = detail.filter(|detail_value| match detail_value {
        Value::Null => false,
        Value::String(text) => !text.is_empty(),
        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => true,
    });
    result.insert(
        "detail".to_owned(),
        selected_detail
            .cloned()
            .unwrap_or_else(|| Value::String("auto".to_owned())),
    );
    Value::Object(result)
}
