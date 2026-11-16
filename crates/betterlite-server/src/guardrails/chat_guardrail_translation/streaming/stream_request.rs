use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_translation::merge_user_key_metadata;

use crate::guardrails::chat_guardrail_translation::rewrites::principal::user_key_dict;

mod tests;

// A fresh dict holding the buffered chunks and the key's metadata, as LiteLLM builds it when the caller passes no request data.
#[must_use]
pub fn stream_request_data(responses_so_far: &[Object], principal: &Principal) -> Object {
    let chunks = responses_so_far
        .iter()
        .cloned()
        .map(GuardrailValue::Object)
        .collect();
    let mut request_data = Object::new();
    request_data.insert("responses".to_owned(), GuardrailValue::Array(chunks));
    merge_user_key_metadata(&mut request_data, Some(&user_key_dict(principal)));
    request_data
}

// The model comes from the first buffered chunk, and only when it names one.
#[must_use]
pub fn stream_inputs(texts: Vec<String>, responses_so_far: &[Object]) -> GuardrailApiInputs {
    let model = match responses_so_far
        .first()
        .and_then(|chunk| chunk.get("model"))
    {
        Some(GuardrailValue::Text(name)) if !name.is_empty() => Some(name.clone()),
        Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        )
        | None => None,
    };
    GuardrailApiInputs {
        texts: Some(texts),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model,
        stream_holdback_chars: None,
    }
}
