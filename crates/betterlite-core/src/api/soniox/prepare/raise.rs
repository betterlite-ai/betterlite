use betterlite_config::sections::settings::JsonValue;
use serde::Deserialize;
use serde_json::from_value;

use super::decode::SonioxReply;
use crate::api::soniox::records::error::SonioxException;

pub const ERROR_MESSAGE_FIELD: &str = "error_message";
pub const ERROR_FIELD: &str = "error";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ErrorBody {
    error_message: Option<JsonValue>,
    error: Option<JsonValue>,
}

pub fn raise_for_response(reply: &SonioxReply, action: &str) -> Result<(), SonioxException> {
    if reply.status < 400 {
        return Ok(());
    }
    let message = format!(
        "Soniox {action} failed (HTTP {}): {}",
        reply.status,
        provider_message(reply)
    );
    Err(SonioxException {
        message,
        status_code: reply.status,
        headers: Some(reply.headers.clone()),
    })
}

#[must_use]
pub fn provider_message(reply: &SonioxReply) -> String {
    let Ok(payload) = reply.object() else {
        return reply.text.clone();
    };
    let Ok(body) = from_value::<ErrorBody>(JsonValue::Object(payload.clone())) else {
        return reply.text.clone();
    };
    let chosen = body
        .error_message
        .as_ref()
        .filter(|value| counts_as_message(value))
        .or_else(|| body.error.as_ref().filter(|value| counts_as_message(value)));
    chosen.map_or_else(|| reply.text.clone(), message_text)
}

fn counts_as_message(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_f64().is_some_and(|number| number != 0.0_f64),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}

fn message_text(value: &JsonValue) -> String {
    if let JsonValue::String(text) = value {
        return text.clone();
    }
    value.to_string()
}
