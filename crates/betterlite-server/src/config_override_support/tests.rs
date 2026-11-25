#![cfg(test)]

mod audit;
mod corpus;
mod decode;
mod environment;
mod masking;
mod models;
mod persist;
mod state;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

pub(crate) fn value(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn text(text: &str) -> JsonValue {
    JsonValue::String(text.to_owned())
}

pub(crate) fn object(text: &str) -> JsonMap {
    match value(text) {
        JsonValue::Object(map) => map,
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)) => panic!("the test fixture is not a JSON object: {other}"),
    }
}
