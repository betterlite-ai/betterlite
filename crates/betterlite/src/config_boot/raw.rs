use std::cmp::Ordering;

use betterlite_config::sections::settings::JsonValue;

// LiteLLM truth-tests these raw config values, so the same test decides here
#[must_use]
pub(crate) fn is_configured(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number
            .as_f64()
            .is_some_and(|number| number.partial_cmp(&0.0) != Some(Ordering::Equal)),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}

// environment_variables values reach str(); numbers and bools render as Python's str does
#[must_use]
pub(crate) fn scalar_text(value: &JsonValue) -> String {
    if value.is_null() {
        return "None".to_owned();
    }
    if let Some(flag) = value.as_bool() {
        return if flag { "True" } else { "False" }.to_owned();
    }
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    // numbers, lists and mappings take JSON text, since Python's repr is not reproduced
    value.to_string()
}
