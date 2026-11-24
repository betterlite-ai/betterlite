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

pub(crate) fn object(value: JsonValue) -> JsonMap {
    match value {
        JsonValue::Object(map) => map,
        other => panic!("the test fixture is not a JSON object: {other}"),
    }
}
