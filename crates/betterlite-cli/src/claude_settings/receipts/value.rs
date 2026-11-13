use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::derive_partial_eq_without_eq,
    reason = "a JSON number is a float, which has PartialEq but not Eq"
)]
pub struct OwnedValue {
    pub present: bool,
    #[serde(default)]
    pub value: JsonValue,
}

pub(super) const ABSENT: OwnedValue = OwnedValue {
    present: false,
    value: JsonValue::Null,
};

#[must_use]
pub(super) fn fingerprint(owned: &OwnedValue) -> String {
    let object = [
        ("present".to_owned(), Json::Bool(owned.present)),
        ("value".to_owned(), sorted_json(&owned.value)),
    ]
    .into_iter()
    .collect();
    hex::encode(Sha256::digest(
        python_json::dumps_object(&object).as_bytes(),
    ))
}

// Python's sort_keys sorts every object level, so the port's ordered maps are filled in key order
fn sorted_json(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::String(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(sorted_json).collect()),
        JsonValue::Object(fields) => {
            let mut entries: Vec<(&String, &JsonValue)> = fields.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            Json::Object(
                entries
                    .into_iter()
                    .map(|(key, item)| (key.clone(), sorted_json(item)))
                    .collect(),
            )
        }
    }
}
