use betterlite_models::decisions::Json;
use serde::Serialize;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(super) fn serialized<T>(value: &T) -> String
where
    T: Serialize,
{
    serde_json::to_string(value).expect("the record serializes")
}
