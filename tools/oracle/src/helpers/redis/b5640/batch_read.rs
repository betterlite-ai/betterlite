use std::collections::{BTreeMap, HashMap};

use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::batch_read::{BatchKey, batch_keys, decoded_batch};
use serde_json::{Map, Value, json};

use super::helper_error;
use super::input::BatchReadCall;
use crate::helpers::HelperResult;

pub(crate) fn run(call: &BatchReadCall) -> HelperResult {
    let keys: Vec<Option<&str>> = call.keys.iter().map(Option::as_deref).collect();
    let entries: Vec<BatchKey> = batch_keys(&keys, call.namespace.as_deref());
    let replies: Vec<Option<Vec<u8>>> = call
        .replies
        .iter()
        .map(|reply| reply.as_ref().map(|text| text.clone().into_bytes()))
        .collect();
    let values =
        decoded_batch(&entries, &replies).map_err(|error| helper_error("DecodeError", error))?;
    Ok(json!({
        "keys_sent": entries.iter().map(|entry| entry.namespaced.clone()).collect::<Vec<String>>(),
        "values": batch_json(&values),
    }))
}

fn cached_json(value: &CachedValue) -> Value {
    match value {
        CachedValue::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, entry)| (key.clone(), cached_json(entry)))
                .collect(),
        ),
        CachedValue::Null => Value::Null,
        CachedValue::Boolean(flag) => Value::Bool(*flag),
        CachedValue::Number(number) => Value::Number(number.clone()),
        CachedValue::Text(text) => Value::String(text.clone()),
        CachedValue::Array(items) => Value::Array(items.iter().map(cached_json).collect()),
    }
}

fn batch_json(values: &HashMap<String, Option<CachedValue>>) -> Value {
    let ordered: BTreeMap<&String, &Option<CachedValue>> = values.iter().collect();
    let mut object = Map::new();
    for (key, value) in ordered {
        object.insert(key.clone(), value.as_ref().map_or(Value::Null, cached_json));
    }
    Value::Object(object)
}
