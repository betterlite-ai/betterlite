use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

mod tests;

// A response tool call is JSON here, so it is already the dict LiteLLM would build; anything that is not an object is invalid and dropped without a log.
#[must_use]
pub fn convert_tool_call_to_dict(tool_call: &GuardrailValue) -> Option<Object> {
    let GuardrailValue::Object(dict) = tool_call else {
        return None;
    };
    Some(dict.clone())
}
