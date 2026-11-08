use betterlite_config::sections::settings::JsonValue;

use crate::keepalive_interval::PingSetting;

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
