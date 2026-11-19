use serde_json::{Map, Value};

mod passthrough_extract_model;

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_)) => panic!("expected an object, got {other}"),
    }
}
