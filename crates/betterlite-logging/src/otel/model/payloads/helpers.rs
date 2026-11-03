use serde_json::Value;
use std::collections::BTreeMap;

pub fn cache_token_value(values: &[Value]) -> Option<i64> {
    let mut saw_zero = false;
    let mut invalid_before_zero = false;
    for value in values {
        let parsed = value.as_i64();
        match parsed {
            Some(value) if value > 0 => return Some(value),
            Some(0) => saw_zero = true,
            _ if !saw_zero => invalid_before_zero = true,
            _ => {}
        }
    }
    if saw_zero && !invalid_before_zero {
        Some(0)
    } else {
        None
    }
}

pub fn guardrail_mode_str(value: &Value) -> Option<String> {
    if let Value::Array(items) = value {
        let mut joined = String::new();
        for item in items {
            let Some(mode) = item.as_str() else {
                continue;
            };
            if !joined.is_empty() {
                joined.push(',');
            }
            joined.push_str(mode);
        }
        return (!joined.is_empty()).then_some(joined);
    }
    value.as_str().map(str::to_owned)
}

pub fn total_masked_entities(value: &Value) -> Option<i64> {
    if let Value::Object(items) = value {
        let total = items.values().fold(0_i64, |sum, value| {
            sum.saturating_add(value.as_i64().unwrap_or(0))
        });
        (total != 0).then_some(total)
    } else {
        value.as_i64().filter(|value| *value != 0)
    }
}

pub fn dicts(value: &Value) -> Vec<BTreeMap<String, Value>> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for item in items {
        if let Some(fields) = item.as_object() {
            let mut cloned = BTreeMap::new();
            for (key, value) in fields {
                cloned.insert(key.clone(), value.clone());
            }
            result.push(cloned);
        }
    }
    result
}

pub fn json_or_none(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| serde_json::to_string(value).ok())
}

#[path = "helpers/tests.rs"]
mod tests;
