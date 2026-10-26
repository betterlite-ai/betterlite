use std::num::FpCategory;

use super::settings_rules::JsonValue;

mod tests;

#[must_use]
pub fn coerce_bool(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Null | JsonValue::Bool(_) => value,
        JsonValue::String(text) => JsonValue::Bool(text.to_lowercase() == "true"),
        JsonValue::Number(number) => JsonValue::Bool(
            number
                .as_f64()
                .is_some_and(|float| float.classify() != FpCategory::Zero),
        ),
        JsonValue::Array(items) => JsonValue::Bool(!items.is_empty()),
        JsonValue::Object(entries) => JsonValue::Bool(!entries.is_empty()),
    }
}
