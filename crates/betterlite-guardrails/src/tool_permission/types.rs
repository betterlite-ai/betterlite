use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionError {
    pub tool_name: String,
    pub rule_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    #[serde(rename = "type", default = "default_type")]
    pub kind: String,
    pub tool_use_id: String,
    pub content: String,
    #[serde(default = "default_is_error")]
    pub is_error: bool,
}

fn default_type() -> String {
    "tool_result".to_owned()
}

fn default_is_error() -> bool {
    true
}

// LiteLLM's _object_mapping: an opaque value is a mapping only when it really is one
#[must_use]
pub const fn object_mapping(value: &GuardrailValue) -> Option<&BTreeMap<String, GuardrailValue>> {
    match value {
        GuardrailValue::Object(mapping) => Some(mapping),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => None,
    }
}

// LiteLLM's _object_list: a sequence only when the value really is a list
#[must_use]
pub fn object_list(value: &GuardrailValue) -> Option<&[GuardrailValue]> {
    match value {
        GuardrailValue::Array(items) => Some(items),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Object(_) => None,
    }
}

// LiteLLM's _get_mapping_value: a key read off a mapping, absent for a missing key or a non-mapping value
#[must_use]
pub fn mapping_value<'value>(
    value: &'value GuardrailValue,
    key: &str,
) -> Option<&'value GuardrailValue> {
    match value {
        GuardrailValue::Object(mapping) => mapping.get(key),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => None,
    }
}

// LiteLLM's _is_tool_use_block: an Anthropic content block that dispatches a tool
#[must_use]
pub fn is_tool_use_block(block: &GuardrailValue) -> bool {
    match mapping_value(block, "type") {
        Some(GuardrailValue::Text(kind)) => kind == "tool_use",
        Some(_other) | None => false,
    }
}
