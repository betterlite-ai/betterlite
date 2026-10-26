use super::JsonValue;

#[must_use]
pub fn coerce_bool(value: JsonValue) -> JsonValue {
    super::settings_rules::coerce_bool(value)
}

#[cfg(test)]
mod tests;
