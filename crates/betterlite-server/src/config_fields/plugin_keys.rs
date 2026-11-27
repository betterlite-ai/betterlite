use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

pub const PLUGIN_KEY_REDACTED: &str = "***";

// _preserve_redacted_plugin_keys: a blank or "***" plugin_key keeps the stored credential, and one with nothing stored drops the key
#[must_use]
pub fn preserve_redacted_plugin_keys(
    incoming: &JsonValue,
    existing: Option<&JsonValue>,
) -> JsonValue {
    let Some(items) = incoming.as_array() else {
        return incoming.clone();
    };
    let stored = stored_plugin_keys(existing);
    JsonValue::Array(
        items
            .iter()
            .map(|item| resolve_plugin(item, &stored))
            .collect(),
    )
}

fn stored_plugin_keys(existing: Option<&JsonValue>) -> BTreeMap<String, JsonValue> {
    let Some(entries) = existing.and_then(JsonValue::as_array) else {
        return BTreeMap::new();
    };
    let mut stored = BTreeMap::new();
    for entry in entries {
        let Some(object) = entry.as_object() else {
            continue;
        };
        let Some(name) = object.get("name").filter(|name| kept_value(name)) else {
            continue;
        };
        let Some(key) = object.get("plugin_key").filter(|key| kept_value(key)) else {
            continue;
        };
        if let Some(name) = name.as_str() {
            stored.insert(name.to_owned(), key.clone());
        }
    }
    stored
}

fn resolve_plugin(plugin: &JsonValue, stored: &BTreeMap<String, JsonValue>) -> JsonValue {
    let Some(object) = plugin.as_object() else {
        return plugin.clone();
    };
    let placeholder = match object.get("plugin_key") {
        None | Some(JsonValue::Null) => true,
        Some(JsonValue::String(text)) => text.is_empty() || text == PLUGIN_KEY_REDACTED,
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => false,
    };
    if !placeholder {
        return plugin.clone();
    }
    let mut resolved = object.clone();
    match object.get("name").and_then(JsonValue::as_str) {
        Some(name) => match stored.get(name) {
            Some(key) => {
                resolved.insert("plugin_key".to_owned(), key.clone());
            }
            None => {
                resolved.remove("plugin_key");
            }
        },
        None => {
            resolved.remove("plugin_key");
        }
    }
    JsonValue::Object(resolved)
}

// Python's truthiness, which decides whether a stored name or key is worth keeping
fn kept_value(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_f64().is_some_and(|numeric| numeric != 0.0),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(entries) => !entries.is_empty(),
    }
}
