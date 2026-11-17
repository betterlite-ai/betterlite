use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

// What a streaming guardrail is handed: a decoded chunk object, or the raw bytes of an SSE frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    Json(GuardrailValue),
    Bytes(Vec<u8>),
}

const fn object(value: &GuardrailValue) -> Option<&BTreeMap<String, GuardrailValue>> {
    if let GuardrailValue::Object(fields) = value {
        return Some(fields);
    }
    None
}

#[must_use]
pub fn field<'value>(value: &'value GuardrailValue, name: &str) -> Option<&'value GuardrailValue> {
    object(value)?
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

#[must_use]
pub fn text_field(value: Option<&GuardrailValue>) -> Option<&str> {
    let GuardrailValue::Text(text) = value? else {
        return None;
    };
    Some(text.as_str())
}

impl Chunk {
    #[must_use]
    pub const fn payload(&self) -> Option<&GuardrailValue> {
        match self {
            Self::Json(value) => Some(value),
            Self::Bytes(_) => None,
        }
    }
}
