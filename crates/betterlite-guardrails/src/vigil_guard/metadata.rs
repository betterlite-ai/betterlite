use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{METADATA_ALLOWLIST, METADATA_ARRAY_MAX_ITEMS, METADATA_STRING_MAX_CHARS};

pub trait CallIdSource {
    fn litellm_call_id(&self) -> Option<&str>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetadataScalar {
    Text(String),
    SignedInteger(i64),
    UnsignedInteger(u64),
    Decimal(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetadataValue {
    Text(String),
    SignedInteger(i64),
    UnsignedInteger(u64),
    Decimal(f64),
    List(Vec<MetadataScalar>),
}

#[must_use]
pub fn clamp_metadata_value(value: &GuardrailValue) -> Option<MetadataValue> {
    match value {
        GuardrailValue::Text(text) => Some(MetadataValue::Text(
            text.chars().take(METADATA_STRING_MAX_CHARS).collect(),
        )),
        GuardrailValue::SignedInteger(number) => Some(MetadataValue::SignedInteger(*number)),
        GuardrailValue::UnsignedInteger(number) => Some(MetadataValue::UnsignedInteger(*number)),
        GuardrailValue::Decimal(number) => Some(MetadataValue::Decimal(*number)),
        GuardrailValue::Array(items) => {
            let mut clamped = Vec::new();
            for item in items.iter().take(METADATA_ARRAY_MAX_ITEMS) {
                push_metadata_scalar(item, &mut clamped);
            }
            if clamped.is_empty() {
                return None;
            }
            Some(MetadataValue::List(clamped))
        }
        GuardrailValue::Null | GuardrailValue::Boolean(_) | GuardrailValue::Object(_) => None,
    }
}

fn push_metadata_scalar(item: &GuardrailValue, clamped: &mut Vec<MetadataScalar>) {
    match item {
        GuardrailValue::Text(text) => clamped.push(MetadataScalar::Text(
            text.chars().take(METADATA_STRING_MAX_CHARS).collect(),
        )),
        GuardrailValue::SignedInteger(number) => {
            clamped.push(MetadataScalar::SignedInteger(*number));
        }
        GuardrailValue::UnsignedInteger(number) => {
            clamped.push(MetadataScalar::UnsignedInteger(*number));
        }
        GuardrailValue::Decimal(number) => clamped.push(MetadataScalar::Decimal(*number)),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => {}
    }
}

#[must_use]
pub fn collect_metadata(
    request_data: &BTreeMap<String, GuardrailValue>,
    logging_obj: Option<&dyn CallIdSource>,
) -> IndexMap<String, MetadataValue> {
    let mut sources: Vec<&BTreeMap<String, GuardrailValue>> = vec![request_data];
    for nested_key in ["metadata", "litellm_metadata"] {
        if let Some(GuardrailValue::Object(nested)) = request_data.get(nested_key) {
            sources.push(nested);
        }
    }
    let mut collected = IndexMap::new();
    for field in METADATA_ALLOWLIST {
        collect_field(&sources, field, &mut collected);
    }
    if let Some(call_id) = extract_call_id(request_data, logging_obj) {
        collected.insert("litellm_call_id".to_owned(), MetadataValue::Text(call_id));
    }
    collected
}

fn collect_field(
    sources: &[&BTreeMap<String, GuardrailValue>],
    field: &str,
    collected: &mut IndexMap<String, MetadataValue>,
) {
    for source in sources {
        let Some(value) = source.get(field) else {
            continue;
        };
        if matches!(value, GuardrailValue::Null) {
            continue;
        }
        if let Some(clamped) = clamp_metadata_value(value) {
            collected.insert(field.to_owned(), clamped);
            return;
        }
    }
}

#[must_use]
pub fn extract_call_id(
    request_data: &BTreeMap<String, GuardrailValue>,
    logging_obj: Option<&dyn CallIdSource>,
) -> Option<String> {
    if let Some(logging) = logging_obj
        && let Some(call_id) = logging.litellm_call_id()
        && !call_id.is_empty()
    {
        return Some(call_id.to_owned());
    }
    if let Some(GuardrailValue::Text(call_id)) = request_data.get("litellm_call_id")
        && !call_id.is_empty()
    {
        return Some(call_id.clone());
    }
    if let Some(GuardrailValue::Object(metadata)) = request_data.get("metadata")
        && let Some(GuardrailValue::Text(call_id)) = metadata.get("litellm_call_id")
        && !call_id.is_empty()
    {
        return Some(call_id.clone());
    }
    None
}
