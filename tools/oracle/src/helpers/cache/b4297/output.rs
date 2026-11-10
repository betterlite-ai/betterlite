use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::envelope::lookup::CacheLookup;
use serde_json::{Map, Value};

use super::{RecordedWrite, failure};

pub(crate) fn object_json(object: &KeyObject) -> Value {
    let mut fields = Map::new();
    for (name, value) in object {
        fields.insert(name.clone(), key_value_json(value));
    }
    Value::Object(fields)
}

pub(crate) fn key_value_json(value: &KeyValue) -> Value {
    match value {
        KeyValue::Null => Value::Null,
        KeyValue::Bool(flag) => Value::Bool(*flag),
        KeyValue::Number(number) => Value::Number(number.clone()),
        KeyValue::String(text) => Value::String(text.clone()),
        KeyValue::Array(items) => Value::Array(items.iter().map(key_value_json).collect()),
        KeyValue::Object(fields) => object_json(fields),
    }
}

pub(crate) fn lookup_json(lookup: &CacheLookup) -> Value {
    let mut fields = Map::new();
    if let Some(messages) = &lookup.messages {
        fields.insert("messages".to_owned(), key_value_json(messages));
    }
    if let Some(input) = &lookup.input {
        fields.insert("input".to_owned(), key_value_json(input));
    }
    if let Some(metadata) = &lookup.metadata {
        fields.insert("metadata".to_owned(), object_json(metadata));
    }
    Value::Object(fields)
}

pub(crate) fn write_json(record: Option<&RecordedWrite>) -> crate::helpers::HelperResult {
    let Some(found) = record else {
        return Ok(Value::Null);
    };
    Ok(serde_json::json!({
        "method": found.method,
        "key": found.key,
        "cached": serde_json::to_value(&found.value).map_err(failure)?,
        "ttl": found.ttl,
    }))
}
