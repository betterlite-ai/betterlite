use betterlite_redis::redis_connection_settings::environment::EnvironmentLookup;
use serde_json::{Map, Value};

use super::fields::CacheSettingsField;
use super::overlay::overlay_environment;
use crate::cache_settings::precedence::resolve_cache_url_precedence;
use crate::cache_settings::redact::redact_credentials;

mod tests;

// LiteLLM reads a falsy value as absent: null, false, 0, "" and an empty list or object all count as unset.
pub(crate) fn carries_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|real| real != 0.0_f64),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(members) => !members.is_empty(),
    }
}

// The overlay, then url precedence, then redis_type and redaction, in LiteLLM's order.
#[must_use]
pub fn effective_values(
    stored: &Map<String, Value>,
    environment: &dyn EnvironmentLookup,
) -> Map<String, Value> {
    let mut effective = resolve_cache_url_precedence(&overlay_environment(stored, environment));
    if matches!(effective.get("type"), Some(Value::String(kind)) if kind == "redis") {
        let kind = redis_type(&effective);
        effective.insert("redis_type".to_owned(), Value::String(kind.to_owned()));
    }
    redact_credentials(&effective)
}

// LiteLLM copies field_value onto the catalog copy for every field name current_values holds.
pub fn fill_field_values(fields: &mut [CacheSettingsField], current_values: &Map<String, Value>) {
    for field in fields {
        if let Some(value) = current_values.get(field.field_name) {
            field.field_value = Some(value.clone());
        }
    }
}

fn redis_type(effective: &Map<String, Value>) -> &'static str {
    if effective
        .get("redis_startup_nodes")
        .is_some_and(carries_value)
    {
        return "cluster";
    }
    if effective.get("sentinel_nodes").is_some_and(carries_value) {
        return "sentinel";
    }
    "node"
}
