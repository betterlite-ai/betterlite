use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::payload::{ToolCallFunctionPayload, ToolCallPayload};
use super::text_value;

const DEFAULT_CALL_TYPE: &str = "function";

pub(super) fn convert(tool_call: &GuardrailValue) -> Option<ToolCallPayload> {
    let GuardrailValue::Object(fields) = tool_call else {
        return None;
    };
    let id = fields
        .get("id")
        .and_then(text_value)
        .filter(|id| !id.is_empty())?;
    let GuardrailValue::Object(function) = fields.get("function")? else {
        return None;
    };
    let name = function
        .get("name")
        .and_then(text_value)
        .filter(|name| !name.is_empty())?;
    let arguments = function.get("arguments")?;
    if matches!(arguments, GuardrailValue::Null) {
        return None;
    }
    let call_type = fields
        .get("type")
        .and_then(text_value)
        .filter(|call_type| !call_type.is_empty())
        .unwrap_or(DEFAULT_CALL_TYPE);
    Some(ToolCallPayload {
        id: id.to_owned(),
        call_type: call_type.to_owned(),
        function: ToolCallFunctionPayload {
            name: name.to_owned(),
            arguments: arguments_text(arguments),
        },
    })
}

// LiteLLM writes a non-string argument as its JSON text, since the payload field takes a string
fn arguments_text(arguments: &GuardrailValue) -> String {
    if let GuardrailValue::Text(text) = arguments {
        return text.clone();
    }
    serde_json::to_string(arguments).unwrap_or_default()
}
