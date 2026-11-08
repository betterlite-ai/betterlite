use betterlite_config::sections::settings::JsonValue;

use super::DeploymentKeepalive;
use crate::keepalive_interval::PingSetting;

pub(super) fn group_pair(deployment: &JsonValue) -> DeploymentKeepalive {
    let params = deployment
        .get("litellm_params")
        .and_then(JsonValue::as_object);
    let seconds = params
        .and_then(|fields| fields.get("keepalive_seconds"))
        .filter(|seconds| !seconds.is_null())
        .cloned();
    let allow_client_override = params
        .and_then(|fields| fields.get("allow_client_keepalive_override"))
        == Some(&JsonValue::Bool(true));
    DeploymentKeepalive {
        seconds,
        allow_client_override,
    }
}

pub(super) fn seconds_agree(left: Option<&JsonValue>, right: Option<&JsonValue>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(JsonValue::Number(left_number)), Some(JsonValue::Number(right_number))) => {
            left_number.as_f64() == right_number.as_f64()
        }
        (Some(JsonValue::String(left_text)), Some(JsonValue::String(right_text))) => {
            left_text == right_text
        }
        (Some(left_value), Some(right_value)) => left_value == right_value,
        _ => false,
    }
}

pub(super) fn value_seconds(raw: &JsonValue) -> f64 {
    match raw {
        JsonValue::Number(number) => number.as_f64().unwrap_or(0.0_f64),
        JsonValue::String(text) => parse_float_text(text).unwrap_or(0.0_f64),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            0.0_f64
        }
    }
}

pub(super) fn setting_seconds(setting: &PingSetting) -> f64 {
    match setting {
        PingSetting::Seconds(seconds) => *seconds,
        PingSetting::Text(text) => parse_float_text(text).unwrap_or(0.0_f64),
    }
}

// Python's float() ignores surrounding whitespace and accepts single underscores between digits
pub(super) fn parse_float_text(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let mut previous_is_digit = false;
    let mut characters = trimmed.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            let next_is_digit = characters.peek().is_some_and(char::is_ascii_digit);
            if !previous_is_digit || !next_is_digit {
                return None;
            }
        }
        previous_is_digit = character.is_ascii_digit();
    }
    trimmed.replace('_', "").parse::<f64>().ok()
}
