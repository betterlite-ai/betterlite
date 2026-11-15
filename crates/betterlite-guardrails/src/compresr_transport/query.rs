use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::content_text::content_to_text;
use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_message_scope::Object;

mod tests;

#[must_use]
pub fn render_tool_intent(function: &GuardrailValue) -> String {
    match function {
        GuardrailValue::Object(fields) => render_intent_fields(fields),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => String::new(),
    }
}

#[must_use]
pub fn query_for_target(
    messages: &[GuardrailMessage],
    target_idx: usize,
    fallback: &str,
) -> String {
    let Some(message) = messages.get(target_idx) else {
        return fallback.to_owned();
    };
    if message.role != "tool" && message.role != "function" {
        return fallback.to_owned();
    }
    let tool_call_id = text_field(message, "tool_call_id");
    let function_name = text_field(message, "name");
    for previous in messages.get(..target_idx).unwrap_or_default().iter().rev() {
        if previous.role != "assistant" {
            continue;
        }
        if let Some(intent) = intent_from_tool_calls(previous, tool_call_id.as_deref()) {
            return intent;
        }
        if let Some(intent) = intent_from_function_call(previous, function_name.as_deref()) {
            return intent;
        }
    }
    fallback.to_owned()
}

#[must_use]
pub fn fallback_query(messages: &[GuardrailMessage]) -> (String, Option<usize>) {
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role == "user" {
            let text = message
                .extra
                .get("content")
                .map_or_else(String::new, content_to_text);
            return (text, Some(index));
        }
    }
    (String::new(), None)
}

fn render_intent_fields(fields: &Object) -> String {
    let name = match fields.get("name") {
        Some(GuardrailValue::Text(name)) => name.trim().to_owned(),
        _ => String::new(),
    };
    let arguments = match fields.get("arguments") {
        None | Some(GuardrailValue::Null) => String::new(),
        Some(GuardrailValue::Text(text)) => text.trim().to_owned(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
    };
    if name.is_empty() {
        return arguments;
    }
    if arguments.is_empty() {
        return name;
    }
    format!("{name}: {arguments}")
}

fn text_field(message: &GuardrailMessage, key: &str) -> Option<String> {
    let Some(GuardrailValue::Text(text)) = message.extra.get(key) else {
        return None;
    };
    Some(text.clone())
}

fn has_text(fields: &Object, key: &str, wanted: &str) -> bool {
    if let Some(GuardrailValue::Text(found)) = fields.get(key) {
        found == wanted
    } else {
        false
    }
}

fn intent_from_tool_calls(
    message: &GuardrailMessage,
    tool_call_id: Option<&str>,
) -> Option<String> {
    let wanted = tool_call_id?;
    let GuardrailValue::Array(calls) = message.extra.get("tool_calls")? else {
        return None;
    };
    calls.iter().find_map(|call| {
        let GuardrailValue::Object(fields) = call else {
            return None;
        };
        if !has_text(fields, "id", wanted) {
            return None;
        }
        let intent = fields
            .get("function")
            .map_or_else(String::new, render_tool_intent);
        (!intent.is_empty()).then_some(intent)
    })
}

// A legacy function_call has no id, so only a matching name may attach its intent to the output
fn intent_from_function_call(
    message: &GuardrailMessage,
    function_name: Option<&str>,
) -> Option<String> {
    let wanted = function_name?;
    let Some(GuardrailValue::Object(fields)) = message.extra.get("function_call") else {
        return None;
    };
    if !has_text(fields, "name", wanted) {
        return None;
    }
    let intent = render_intent_fields(fields);
    (!intent.is_empty()).then_some(intent)
}
