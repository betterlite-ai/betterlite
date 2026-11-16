use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::inputs::GuardrailLogTarget;

const CALL_ID_KEY: &str = "litellm_call_id";
const METADATA_KEYS: [&str; 2] = ["metadata", "litellm_metadata"];

// The call id comes from the logging object only; a body or kwargs call id is client-settable and never consulted
#[must_use]
pub(super) fn call_id(target: Option<&GuardrailLogTarget>) -> Option<&str> {
    let logging = target?;
    [
        logging.model_call_params.as_ref(),
        logging.litellm_params.as_ref(),
    ]
    .into_iter()
    .flatten()
    .find_map(|params| match params.get(CALL_ID_KEY) {
        Some(GuardrailValue::Text(call_id)) if !call_id.is_empty() => Some(call_id.as_str()),
        _ => None,
    })
}

// The hash of the virtual key the proxy authenticated, empty without per-key auth; the bare user_api_key string is never trusted
#[must_use]
pub(super) fn caller_scope(target: Option<&GuardrailLogTarget>) -> String {
    let Some(logging) = target else {
        return String::new();
    };
    for container in [
        logging.litellm_params.as_ref(),
        logging.model_call_params.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        for metadata_key in METADATA_KEYS {
            let Some(GuardrailValue::Object(metadata)) = container.get(metadata_key) else {
                continue;
            };
            let Some(GuardrailValue::Object(auth)) = metadata.get("user_api_key_auth") else {
                continue;
            };
            if let Some(GuardrailValue::Text(api_key)) = auth.get("api_key")
                && !api_key.is_empty()
            {
                return api_key.clone();
            }
        }
    }
    String::new()
}
