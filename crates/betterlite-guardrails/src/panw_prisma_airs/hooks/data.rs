use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::de::DeserializeOwned;

use super::error::HookDataError;

pub type Data = BTreeMap<String, GuardrailValue>;

pub const CALL_ID: &str = "litellm_call_id";
pub const LITELLM_METADATA: &str = "litellm_metadata";
pub const MESSAGES: &str = "messages";
pub const METADATA: &str = "metadata";
pub const PROMPT: &str = "prompt";

#[must_use]
pub fn text<'data>(data: &'data Data, key: &str) -> Option<&'data str> {
    match data.get(key) {
        Some(GuardrailValue::Text(text)) => Some(text),
        _ => None,
    }
}

#[must_use]
pub fn call_id(data: &Data) -> Option<&str> {
    match data.get(CALL_ID) {
        Some(GuardrailValue::Text(id)) if !id.is_empty() => Some(id),
        _ => None,
    }
}

#[must_use]
pub fn messages(data: &Data) -> Option<&[GuardrailValue]> {
    match data.get(MESSAGES) {
        Some(GuardrailValue::Array(messages)) => Some(messages),
        _ => None,
    }
}

// A value of the wrong type names its field instead of being read as a Python truthiness.
pub fn parse_items<T>(
    values: &[GuardrailValue],
    field: &'static str,
) -> Result<Vec<T>, HookDataError>
where
    T: DeserializeOwned,
{
    let mut parsed = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let encoded = serde_json::to_value(value).map_err(|source| HookDataError::Item {
            field,
            index,
            source,
        })?;
        let item = serde_json::from_value(encoded).map_err(|source| HookDataError::Item {
            field,
            index,
            source,
        })?;
        parsed.push(item);
    }
    Ok(parsed)
}
