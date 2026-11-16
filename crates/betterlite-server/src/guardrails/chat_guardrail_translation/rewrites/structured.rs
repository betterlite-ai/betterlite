use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::{GuardrailMessage, GuardrailToolParam};
use betterlite_guardrails::guardrail_message_scope::Object;

use super::error::RewriteError;

fn split_field(
    value: &GuardrailValue,
    key: &str,
    field: &'static str,
    index: usize,
) -> Result<(String, Object), RewriteError> {
    let GuardrailValue::Object(fields) = value else {
        return Err(RewriteError::NotAnObject { field, index });
    };
    let mut rest = fields.clone();
    let taken = match rest.remove(key) {
        Some(GuardrailValue::Text(text)) => text,
        Some(_) | None => String::new(),
    };
    Ok((taken, rest))
}

// A message without a role goes to the guardrail with an empty one.
pub fn guardrail_messages(
    messages: &[GuardrailValue],
    scoped: &[usize],
) -> Result<Vec<GuardrailMessage>, RewriteError> {
    let mut structured = Vec::new();
    for &index in scoped {
        let Some(message) = messages.get(index) else {
            return Err(RewriteError::NotAnObject {
                field: "messages",
                index,
            });
        };
        let (role, extra) = split_field(message, "role", "messages", index)?;
        structured.push(GuardrailMessage { role, extra });
    }
    Ok(structured)
}

#[must_use]
pub fn message_value(message: GuardrailMessage) -> GuardrailValue {
    let mut fields = message.extra;
    fields.insert("role".to_owned(), GuardrailValue::Text(message.role));
    GuardrailValue::Object(fields)
}

pub fn guardrail_tools(tools: &[GuardrailValue]) -> Result<Vec<GuardrailToolParam>, RewriteError> {
    tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let (tool_type, extra) = split_field(tool, "type", "tools", index)?;
            Ok(GuardrailToolParam { tool_type, extra })
        })
        .collect()
}

#[must_use]
pub fn tool_value(tool: GuardrailToolParam) -> GuardrailValue {
    let mut fields = tool.extra;
    fields.insert("type".to_owned(), GuardrailValue::Text(tool.tool_type));
    GuardrailValue::Object(fields)
}

pub fn tool_call_objects(values: Vec<GuardrailValue>) -> Result<Vec<Object>, RewriteError> {
    values
        .into_iter()
        .enumerate()
        .map(|(task_index, value)| match value {
            GuardrailValue::Object(object) => Ok(object),
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_) => Err(RewriteError::ToolCallNotObject { task_index }),
        })
        .collect()
}
