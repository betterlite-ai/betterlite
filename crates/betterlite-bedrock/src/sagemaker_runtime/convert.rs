use betterlite_models::decisions::Json as Plain;
use betterlite_prompts::arguments::json::Json as Prompted;
use indexmap::IndexMap;
use serde_json::{Map, Value};

// the body the signer serializes keeps its keys in the order the transform wrote them
pub fn completion_body(body: &IndexMap<String, Value>) -> IndexMap<String, Plain> {
    body.iter()
        .map(|(name, value)| (name.clone(), plain(value)))
        .collect()
}

pub fn prompt_body(body: &IndexMap<String, Prompted>) -> IndexMap<String, Plain> {
    body.iter()
        .map(|(name, value)| (name.clone(), prompted(value)))
        .collect()
}

pub fn plain(value: &Value) -> Plain {
    match value {
        Value::Null => Plain::Null,
        Value::Bool(flag) => Plain::Bool(*flag),
        Value::Number(number) => Plain::Number(number.clone()),
        Value::String(text) => Plain::Text(text.clone()),
        Value::Array(items) => Plain::List(items.iter().map(plain).collect()),
        Value::Object(fields) => Plain::Object(object(fields)),
    }
}

fn object(fields: &Map<String, Value>) -> IndexMap<String, Plain> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), plain(value)))
        .collect()
}

fn prompted(value: &Prompted) -> Plain {
    match value {
        Prompted::Null => Plain::Null,
        Prompted::Bool(flag) => Plain::Bool(*flag),
        Prompted::Number(number) => Plain::Number(number.clone()),
        Prompted::Text(text) => Plain::Text(text.clone()),
        Prompted::List(items) => Plain::List(items.iter().map(prompted).collect()),
        Prompted::Object(fields) => Plain::Object(prompted_object(fields)),
    }
}

fn prompted_object(fields: &IndexMap<String, Prompted>) -> IndexMap<String, Plain> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), prompted(value)))
        .collect()
}
