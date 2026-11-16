use betterlite_models::decisions::{HiddenParams, Json as HiddenJson};
use serde_json::Value;

const CHOICE_FIELDS: [(&str, &str); 2] = [
    ("raw_output", "fireworks_raw_outputs"),
    ("token_ids", "fireworks_token_ids"),
];
const TOP_LEVEL_FIELDS: [(&str, &str); 2] = [
    ("perf_metrics", "fireworks_perf_metrics"),
    ("prompt_token_ids", "fireworks_prompt_token_ids"),
];

// _extract_fireworks_hidden_params: a key appears only when a payload field is present, so null counts
#[must_use]
pub fn extract_hidden_params(payload: &Value) -> HiddenParams {
    let mut hidden = HiddenParams::new();
    if let Some(fields) = payload.as_object() {
        for (field, destination) in TOP_LEVEL_FIELDS {
            if let Some(value) = fields.get(field) {
                hidden.insert(destination.to_owned(), hidden_json(value));
            }
        }
    }
    let choices: Vec<&Value> = payload
        .get("choices")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter(|entry| entry.is_object()).collect())
        .unwrap_or_default();
    for (field, destination) in CHOICE_FIELDS {
        let present: Vec<HiddenJson> = choices
            .iter()
            .filter_map(|choice| choice.get(field))
            .map(hidden_json)
            .collect();
        if !present.is_empty() {
            hidden.insert(destination.to_owned(), HiddenJson::List(present));
        }
    }
    hidden
}

fn hidden_json(value: &Value) -> HiddenJson {
    match value {
        Value::Null => HiddenJson::Null,
        Value::Bool(flag) => HiddenJson::Bool(*flag),
        Value::Number(number) => HiddenJson::Number(number.clone()),
        Value::String(text) => HiddenJson::Text(text.clone()),
        Value::Array(items) => HiddenJson::List(items.iter().map(hidden_json).collect()),
        Value::Object(fields) => HiddenJson::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), hidden_json(item)))
                .collect(),
        ),
    }
}
