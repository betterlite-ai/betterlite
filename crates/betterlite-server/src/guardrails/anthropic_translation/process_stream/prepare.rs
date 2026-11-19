use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_translation::transform_user_key_metadata;

const LITELLM_METADATA: &str = "litellm_metadata";

// LiteLLM's _prepare_request_data: the response key only when missing, then the key's metadata when missing too.
pub(super) fn prepare_request_data(
    request_data: &mut Object,
    value: GuardrailValue,
    user_api_key_dict: Option<&Object>,
    key: &str,
) {
    request_data.entry(key.to_owned()).or_insert(value);
    if request_data.contains_key(LITELLM_METADATA) {
        return;
    }
    let metadata = transform_user_key_metadata(user_api_key_dict);
    if !metadata.is_empty() {
        request_data.insert(
            LITELLM_METADATA.to_owned(),
            GuardrailValue::Object(metadata),
        );
    }
}
