use super::settings_rules::JsonValue;

mod tests;

#[must_use]
pub fn coerce_bool(value: JsonValue) -> JsonValue {
    let normalized = super::coerce_bool(value);
    if normalized.is_null() {
        JsonValue::Null
    } else {
        normalized
    }
}
