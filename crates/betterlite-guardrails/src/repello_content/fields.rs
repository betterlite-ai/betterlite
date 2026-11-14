use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

pub(super) fn get_field<'value>(
    value: Option<&'value GuardrailValue>,
    key: &str,
) -> Option<&'value GuardrailValue> {
    match value {
        Some(GuardrailValue::Object(object)) => object.get(key),
        _ => None,
    }
}

pub(super) fn non_empty_text(value: Option<&GuardrailValue>) -> Option<&str> {
    let Some(GuardrailValue::Text(text)) = value else {
        return None;
    };
    if text.is_empty() { None } else { Some(text) }
}

pub(super) fn non_blank_text(value: Option<&GuardrailValue>) -> Option<&str> {
    let Some(GuardrailValue::Text(text)) = value else {
        return None;
    };
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

pub(super) const fn object_of(value: &GuardrailValue) -> Option<&BTreeMap<String, GuardrailValue>> {
    if let GuardrailValue::Object(object) = value {
        Some(object)
    } else {
        None
    }
}
