use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::apply_error::ApplyError;

pub struct ChoiceParts<'choice> {
    pub content: Option<&'choice GuardrailValue>,
    pub tool_calls: &'choice [GuardrailValue],
}

#[must_use]
pub fn field<'value>(value: &'value GuardrailValue, name: &str) -> Option<&'value GuardrailValue> {
    let GuardrailValue::Object(fields) = value else {
        return None;
    };
    fields
        .get(name)
        .filter(|found| !matches!(found, GuardrailValue::Null))
}

pub fn field_mut<'value>(
    value: &'value mut GuardrailValue,
    name: &str,
) -> Option<&'value mut GuardrailValue> {
    let GuardrailValue::Object(fields) = value else {
        return None;
    };
    fields.get_mut(name)
}

// A choice with neither a message nor a delta is not a choice type LiteLLM reads.
#[must_use]
pub fn choice_parts(choice: &GuardrailValue) -> Option<ChoiceParts<'_>> {
    let payload = field(choice, "message").or_else(|| field(choice, "delta"))?;
    Some(ChoiceParts {
        content: field(payload, "content"),
        tool_calls: array_field(payload, "tool_calls"),
    })
}

#[must_use]
pub fn array_field<'value>(value: &'value GuardrailValue, name: &str) -> &'value [GuardrailValue] {
    match field(value, name) {
        Some(GuardrailValue::Array(items)) => items,
        _ => &[],
    }
}

#[must_use]
pub fn integer_value(value: Option<&GuardrailValue>) -> Option<i64> {
    match value {
        Some(GuardrailValue::SignedInteger(number)) => Some(*number),
        Some(GuardrailValue::UnsignedInteger(number)) => i64::try_from(*number).ok(),
        _ => None,
    }
}

#[must_use]
pub fn object_items<'object>(object: &'object Object, key: &str) -> &'object [GuardrailValue] {
    match object.get(key) {
        Some(GuardrailValue::Array(items)) => items,
        _ => &[],
    }
}

pub fn choice_mut(
    response: &mut Object,
    position: usize,
) -> Result<&mut GuardrailValue, ApplyError> {
    let Some(GuardrailValue::Array(choices)) = response.get_mut("choices") else {
        return Err(ApplyError::MissingEntry { index: position });
    };
    choices
        .get_mut(position)
        .ok_or(ApplyError::MissingEntry { index: position })
}

#[must_use]
pub fn image_object_url(image_url: &GuardrailValue) -> Option<&String> {
    match field(image_url, "url") {
        Some(GuardrailValue::Text(url)) if !url.is_empty() => Some(url),
        _ => None,
    }
}
