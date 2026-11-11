use betterlite_models::controls::json_value::JsonValue;

use super::Params;

#[must_use]
pub fn peek_reasoning_summary_aliases(optional_params: &Params) -> Option<&JsonValue> {
    let top = optional_params
        .get("reasoningSummary")
        .or_else(|| optional_params.get("reasoning_summary"));
    let nested = match optional_params.get("extra_body") {
        Some(JsonValue::Object(body)) => body
            .get("reasoningSummary")
            .or_else(|| body.get("reasoning_summary")),
        _ => None,
    };
    top.or(nested)
        .filter(|value| !matches!(value, JsonValue::Null))
}

#[must_use]
pub fn strip_reasoning_aliases(optional_params: &Params) -> (Params, Option<JsonValue>) {
    let mut stripped = optional_params.clone();
    let camel = stripped.remove("reasoningSummary");
    let snake = stripped.remove("reasoning_summary");
    let mut summary = camel
        .filter(|value| !matches!(value, JsonValue::Null))
        .or_else(|| snake.filter(|value| !matches!(value, JsonValue::Null)));
    let drop_extra_body = match stripped.get_mut("extra_body") {
        Some(JsonValue::Object(body)) => {
            let nested_camel = body.shift_remove("reasoningSummary");
            let nested_snake = body.shift_remove("reasoning_summary");
            summary = summary
                .or_else(|| nested_camel.filter(|value| !matches!(value, JsonValue::Null)))
                .or_else(|| nested_snake.filter(|value| !matches!(value, JsonValue::Null)));
            body.is_empty()
        }
        _ => false,
    };
    if drop_extra_body {
        stripped.remove("extra_body");
    }
    (stripped, summary)
}
