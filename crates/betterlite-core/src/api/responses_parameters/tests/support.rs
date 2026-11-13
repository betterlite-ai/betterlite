use betterlite_context::non_default_params::Params;
use betterlite_models::controls::json_value::JsonValue;

pub(super) fn json(source: &str) -> JsonValue {
    serde_json::from_str(source).expect("test JSON")
}

pub(super) fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("test params")
}
