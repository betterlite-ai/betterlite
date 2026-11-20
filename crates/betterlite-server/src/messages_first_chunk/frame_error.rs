use std::str::from_utf8;

use betterlite_bedrock::image_models::sizing::parse_python_int;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

const DATA_PREFIX: &str = "data: ";
const DONE_MARKER: &str = "[DONE]";
const LOWEST_CODE: i64 = 100;
const HIGHEST_CODE: i64 = 599;

// _parse_event_data_for_error: the HTTP status an error chunk carries, if it carries a usable one
#[must_use]
pub fn frame_error_code(frame: &[u8]) -> Option<u16> {
    let data = parsed_object(frame)?;
    let Some(Json::Object(error)) = data.get("error") else {
        return None;
    };
    let raw = error.get("code")?;
    let code = match raw {
        Json::Null => return None,
        Json::Text(text) => match parse_python_int(text) {
            Ok(value) => Some(value),
            Err(_error) => {
                tracing::warn!("Error code is a string but not a valid integer: {text}");
                None
            }
        },
        Json::Number(number) => number.as_i64(),
        Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    };
    if let Some(value) = code
        && (LOWEST_CODE..=HIGHEST_CODE).contains(&value)
    {
        return u16::try_from(value).ok();
    }
    tracing::warn!(
        "Error has invalid or non-convertible code: {}",
        raw_text(raw)
    );
    None
}

// _extract_error_from_sse_chunk: the chunk's own error object, or LiteLLM's default one in its key order
#[must_use]
pub fn frame_error_object(frame: &[u8]) -> JsonObject {
    let Some(data) = parsed_object(frame) else {
        return default_error_object();
    };
    match data.get("error") {
        Some(Json::Object(error)) => error.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => default_error_object(),
    }
}

fn parsed_object(frame: &[u8]) -> Option<JsonObject> {
    let text = from_utf8(frame).ok()?;
    let payload = text.strip_prefix(DATA_PREFIX)?;
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == DONE_MARKER {
        return None;
    }
    let Ok(Json::Object(data)) = serde_json::from_str::<Json>(trimmed) else {
        return None;
    };
    Some(data)
}

// Python's %s prints a str bare and every other JSON value as its own text; repr wording is not ported
fn raw_text(raw: &Json) -> String {
    match raw {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(raw).unwrap_or_default()
        }
    }
}

fn default_error_object() -> JsonObject {
    [
        ("message", Json::Text(String::from("Unknown error"))),
        ("type", Json::Text(String::from("internal_server_error"))),
        ("param", Json::Null),
        ("code", Json::Text(String::from("500"))),
    ]
    .into_iter()
    .map(|(key, value)| (String::from(key), value))
    .collect()
}
