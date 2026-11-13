use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_translation::transform_user_key_metadata;

pub fn add_response_context(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    response: &GuardrailValue,
    user_api_key_dict: Option<&BTreeMap<String, GuardrailValue>>,
) {
    request_data
        .entry("response".to_owned())
        .or_insert_with(|| response.clone());
    if request_data.contains_key("litellm_metadata") {
        return;
    }
    let user_metadata = transform_user_key_metadata(user_api_key_dict);
    if !user_metadata.is_empty() {
        request_data.insert(
            "litellm_metadata".to_owned(),
            GuardrailValue::Object(user_metadata),
        );
    }
}

mod tests;
