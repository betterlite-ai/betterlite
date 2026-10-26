use serde_json::Value;
use std::collections::BTreeSet;

pub fn message_has_cache_control(message: &Value) -> bool {
    let has_direct_control = message
        .get("cache_control")
        .is_some_and(|value| !value.is_null());
    if has_direct_control {
        return true;
    }
    message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|parts| {
            parts.iter().any(|part| {
                part.get("cache_control")
                    .is_some_and(|value| !value.is_null())
            })
        })
}

pub fn cached_prefix_indices(messages: &[Value]) -> Vec<usize> {
    let last = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message_has_cache_control(message))
        .map(|(index, _)| index)
        .next_back();
    last.map_or_else(Vec::new, |index| (0..=index).collect())
}

pub fn get_protected_indices(messages: &[Value]) -> Vec<usize> {
    let mut protected = BTreeSet::new();
    for (index, message) in messages.iter().enumerate() {
        if message.get("role") == Some(&Value::String("system".to_owned())) {
            protected.insert(index);
        }
    }
    if let Some(index) = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.get("role") == Some(&Value::String("user".to_owned())))
        .map(|(index, _)| index)
        .next_back()
    {
        protected.insert(index);
    }
    if let Some(index) = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.get("role") == Some(&Value::String("assistant".to_owned())))
        .map(|(index, _)| index)
        .next_back()
    {
        protected.insert(index);
    }
    protected.extend(cached_prefix_indices(messages));
    protected.into_iter().collect()
}

mod tests;
