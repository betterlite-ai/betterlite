use betterlite_openai::api::responses_objects::ResponseApiUsage;
use serde_json::{Map, Value};

pub(super) fn parse(fields: &Map<String, Value>) -> Result<ResponseApiUsage, serde_json::Error> {
    let mut normalized = fields.clone();
    fill_detail_alias(
        &mut normalized,
        "input_tokens_details",
        "input_token_details",
    );
    fill_detail_alias(
        &mut normalized,
        "output_tokens_details",
        "output_token_details",
    );
    normalize_output_bools(&mut normalized);
    fill_total_tokens(&mut normalized);
    serde_json::from_value(Value::Object(normalized))
}

fn fill_detail_alias(fields: &mut Map<String, Value>, plural: &str, singular: &str) {
    if fields.get(plural).is_none_or(Value::is_null)
        && let Some(alias) = fields.get(singular).cloned()
    {
        fields.insert(plural.to_owned(), alias);
    }
}

fn normalize_output_bools(fields: &mut Map<String, Value>) {
    let Some(Value::Object(details)) = fields.get_mut("output_tokens_details") else {
        return;
    };
    for name in ["reasoning_tokens", "audio_tokens", "text_tokens"] {
        let boolean = details.get(name).and_then(Value::as_bool);
        let integer = details
            .get(name)
            .is_some_and(|value| value.as_i64().is_some());
        match (boolean, integer, details.contains_key(name)) {
            (Some(value), _, _) => {
                details.insert(name.to_owned(), Value::from(i64::from(value)));
            }
            (None, false, true) => {
                details.remove(name);
            }
            _ => {}
        }
    }
}

fn fill_total_tokens(fields: &mut Map<String, Value>) {
    if fields
        .get("total_tokens")
        .is_some_and(|value| !value.is_null())
    {
        return;
    }
    let input = fields
        .get("input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output = fields
        .get("output_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    fields.insert(
        "total_tokens".to_owned(),
        Value::from(input.saturating_add(output)),
    );
}
