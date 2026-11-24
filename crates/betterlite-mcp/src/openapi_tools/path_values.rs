use betterlite_prompts::arguments::json::Json;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use thiserror::Error;

// Python's quote(value, safe="") leaves only letters, digits and "_.-~" unescaped
const VALUE_ENCODING: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathValueError {
    #[error("Path parameter '{name}' must not contain path separators")]
    Separator { name: String },
    #[error("Path parameter '{name}' cannot include '.' or '..' segments")]
    DotSegment { name: String },
}

// LiteLLM's _sanitize_path_parameter_value: no separators or "..", then percent-encoded whole
pub fn sanitize_path_value(value: &Json, name: &str) -> Result<String, PathValueError> {
    let text = value_text(value);
    if text.is_empty() {
        return Ok(text);
    }
    let normalized = text.replace('\\', "/");
    if normalized.contains('/') {
        return Err(PathValueError::Separator {
            name: name.to_owned(),
        });
    }
    if normalized == ".." {
        return Err(PathValueError::DotSegment {
            name: name.to_owned(),
        });
    }
    Ok(utf8_percent_encode(&text, VALUE_ENCODING).to_string())
}

// Python's str() for the scalar a path parameter carries; a container goes out as its JSON text
fn value_text(value: &Json) -> String {
    match value {
        Json::Null => String::new(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}
