use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_stream_primitives::decode::{
    JsonShape, decode_responses_stream_value,
};
use serde_json::{Value, json};

// both loaders are json.loads, so one decode proves what the object-named one does with []
pub(crate) fn answer(payloads: &[String]) -> Value {
    let mut rows = Vec::with_capacity(payloads.len());
    for payload in payloads {
        rows.push(row(payload));
    }
    json!({ "rows": rows })
}

fn row(payload: &str) -> Value {
    let Ok(value) = decode_responses_stream_value(payload.as_bytes(), &ApiLimits::default()) else {
        return json!({ "payload": payload, "decoded": false });
    };
    json!({
        "payload": payload,
        "decoded": true,
        "value": value,
        "is_object": value.is_json_object(),
        "is_array": value.is_json_array(),
        "optional_str": value.optional_str(),
        "array_or_empty": value.json_array_or_empty(),
        "str_mapping": value.is_str_mapping(),
    })
}
