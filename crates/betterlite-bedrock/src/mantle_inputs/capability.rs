use crate::model_routes::{ParamMap, ParamValue};

use super::region::split_region_prefix;

const ENDPOINTS_FIELD: &str = "supported_endpoints";
const MODE_FIELD: &str = "mode";
const OPENAI_RESPONSES_PATH_FIELD: &str = "use_openai_responses_path";
const RESPONSES_ENDPOINT: &str = "/v1/responses";
const RESPONSES_MODE: &str = "responses";

#[must_use]
pub fn supports_responses(model: Option<&str>, model_cost: &ParamMap) -> bool {
    let Some(entry) = price_entry(model, model_cost) else {
        return false;
    };
    if endpoint_listed(entry) {
        return true;
    }
    matches!(mapping_field(entry, MODE_FIELD), Some(ParamValue::Text(mode)) if mode == RESPONSES_MODE)
}

#[must_use]
pub fn base_segment(model: Option<&str>, model_cost: &ParamMap) -> &'static str {
    let flag = price_entry(model, model_cost)
        .and_then(|entry| mapping_field(entry, OPENAI_RESPONSES_PATH_FIELD));
    match flag {
        Some(ParamValue::Bool(true)) => "openai/v1",
        _ => "v1",
    }
}

fn price_entry<'map>(model: Option<&str>, model_cost: &'map ParamMap) -> Option<&'map ParamValue> {
    let name = model.filter(|value| !value.is_empty())?;
    let (_, base_model) = split_region_prefix(name);
    model_cost.get(&format!("bedrock_mantle/{base_model}"))
}

fn mapping_field<'entry>(entry: &'entry ParamValue, field: &str) -> Option<&'entry ParamValue> {
    match entry {
        ParamValue::Mapping(fields) => fields.get(field),
        ParamValue::Null
        | ParamValue::Bool(_)
        | ParamValue::Number(_)
        | ParamValue::Text(_)
        | ParamValue::List(_) => None,
    }
}

fn endpoint_listed(entry: &ParamValue) -> bool {
    match mapping_field(entry, ENDPOINTS_FIELD) {
        Some(ParamValue::List(endpoints)) => endpoints
            .iter()
            .any(|item| matches!(item, ParamValue::Text(text) if text == RESPONSES_ENDPOINT)),
        // Python's `in` over a string is a substring test, and LiteLLM reaches it for a scalar list field.
        Some(ParamValue::Text(text)) => text.contains(RESPONSES_ENDPOINT),
        Some(
            ParamValue::Null | ParamValue::Bool(_) | ParamValue::Number(_) | ParamValue::Mapping(_),
        )
        | None => false,
    }
}
