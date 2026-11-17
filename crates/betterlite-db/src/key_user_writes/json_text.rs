use std::collections::BTreeMap;

use crate::budget_permission_rows::JsonData;

pub const FAILED_TO_SERIALIZE_JSON: &str = "failed-to-serialize-json";

pub trait JsonEncoder {
    fn encode(&self, value: &JsonData) -> Result<String, serde_json::Error>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SerdeJsonEncoder;

impl JsonEncoder for SerdeJsonEncoder {
    fn encode(&self, value: &JsonData) -> Result<String, serde_json::Error> {
        serde_json::to_string(value)
    }
}

// LiteLLM stringifies only top-level dict values and writes a marker when that dump fails; nothing nested changes
#[must_use]
pub fn jsonify_object(
    data: &BTreeMap<String, JsonData>,
    encoder: &dyn JsonEncoder,
) -> BTreeMap<String, JsonData> {
    let mut converted = data.clone();
    for value in converted.values_mut() {
        let JsonData::Object(_) = value else {
            continue;
        };
        *value = match encoder.encode(value) {
            Ok(encoded) => JsonData::String(encoded),
            Err(_) => JsonData::String(String::from(FAILED_TO_SERIALIZE_JSON)),
        };
    }
    converted
}
