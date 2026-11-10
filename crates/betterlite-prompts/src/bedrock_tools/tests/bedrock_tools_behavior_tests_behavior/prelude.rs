use crate::arguments::json::{Json, Object};

pub(super) fn json(raw: &str) -> Json {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn object(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn json_list(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).unwrap()
}
