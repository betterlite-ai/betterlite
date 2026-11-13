use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

// LiteLLM's get_session_id_from_request_data
#[must_use]
pub fn session_id_from_request(data: &BTreeMap<String, GuardrailValue>) -> Option<String> {
    let nested = |key: &str| match data.get(key) {
        Some(GuardrailValue::Object(fields)) => fields.get("session_id"),
        _ => None,
    };
    [
        data.get("litellm_session_id"),
        nested("metadata"),
        nested("litellm_metadata"),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| match value {
        GuardrailValue::Text(text) if !text.is_empty() => Some(text.clone()),
        GuardrailValue::SignedInteger(number) => Some(number.to_string()),
        GuardrailValue::UnsignedInteger(number) => Some(number.to_string()),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => None,
    })
}
