use opentelemetry::{Array, StringValue, Value};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use std::collections::BTreeMap;

use super::base::{AttributeMap, AttributeTable};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::payloads::tool_definition::ToolDefinition;
use crate::otel::model::utils::coercion::OtelModelValue;

pub const DEFAULT_SPAN_ATTRIBUTE_LIMIT: usize = 128;

pub const MAX_TOOL_ATTRS_PER_SPAN: usize = DEFAULT_SPAN_ATTRIBUTE_LIMIT.saturating_div(4);

#[must_use]
pub const fn tool_attr_budget(vocabularies: usize) -> usize {
    if vocabularies == 0 {
        return MAX_TOOL_ATTRS_PER_SPAN;
    }
    match MAX_TOOL_ATTRS_PER_SPAN.checked_div(vocabularies) {
        Some(share) => share,
        None => MAX_TOOL_ATTRS_PER_SPAN,
    }
}

#[must_use]
pub fn drop_none(values: &BTreeMap<String, Option<Value>>) -> AttributeMap {
    values
        .iter()
        .filter_map(|(key, value)| value.as_ref().map(|inner| (key.clone(), inner.clone())))
        .collect()
}

#[must_use]
pub fn drop_none_pairs<I>(pairs: I) -> AttributeMap
where
    I: Iterator<Item = (&'static str, Option<Value>)>,
{
    pairs
        .filter_map(|(key, value)| value.map(|inner| (key.to_owned(), inner)))
        .collect()
}

#[must_use]
pub fn collect<T>(table: AttributeTable<T>, source: &T) -> AttributeMap {
    drop_none_pairs(
        table
            .iter()
            .map(|(key, extract)| (*key, (*extract)(source))),
    )
}

#[must_use]
pub fn tool_definition_attrs(
    key_prefix: &str,
    tools: &[ToolDefinition],
    extractors: AttributeTable<ToolDefinition>,
    attr_budget: usize,
) -> AttributeMap {
    let fields = extractors.len().max(1);
    let max_tools = attr_budget
        .checked_div(fields)
        .map_or(1, |share| share.max(1));
    let mut attrs = AttributeMap::new();
    for (index, tool) in tools.iter().take(max_tools).enumerate() {
        for (suffix, extract) in extractors {
            if let Some(value) = (*extract)(tool) {
                attrs.insert(format!("{key_prefix}{index}.{suffix}"), value);
            }
        }
    }
    attrs
}

#[must_use]
pub fn string_array(values: &[String]) -> Value {
    let items: Vec<StringValue> = values
        .iter()
        .map(|value| StringValue::from(value.clone()))
        .collect();
    Value::Array(Array::String(items))
}

#[must_use]
pub fn decimal_attr(amount: Option<Decimal>) -> Option<Value> {
    let cost = amount?;
    let number = cost.to_f64()?;
    Some(Value::F64(number))
}

#[must_use]
pub fn json_if(payload: &MetadataMap) -> Option<String> {
    if payload.is_empty() {
        return None;
    }
    serde_json::to_string(payload).ok()
}

#[must_use]
pub fn json_or_none(value: &OtelModelValue) -> Option<String> {
    let Ok(text) = serde_json::to_string(value) else {
        return None;
    };
    Some(text)
}

#[must_use]
pub fn stringify_message(message: &MetadataMap) -> Option<String> {
    let Ok(text) = serde_json::to_string(message) else {
        return None;
    };
    Some(text)
}

#[must_use]
pub fn serialize_messages(messages: &[MetadataMap]) -> Option<String> {
    let mut serialized: Vec<OtelModelValue> = Vec::new();
    for message in messages {
        let Some(text) = stringify_message(message) else {
            continue;
        };
        if let Ok(parsed) = serde_json::from_str(&text) {
            serialized.push(parsed);
        }
    }
    if serialized.is_empty() {
        return None;
    }
    serde_json::to_string(&serialized).ok()
}

#[must_use]
pub fn message_content(message: &MetadataMap) -> Option<String> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }
    let parts = content.as_array()?;
    let mut joined = String::new();
    for part in parts {
        let Some(fields) = part.as_object() else {
            continue;
        };
        if fields.get("type").and_then(OtelModelValue::as_str) != Some("text") {
            continue;
        }
        if let Some(text) = fields.get("text").and_then(OtelModelValue::as_str) {
            joined.push_str(text);
        }
    }
    (!joined.is_empty()).then_some(joined)
}

#[must_use]
pub fn output_messages(data: &LLMCallSpanData) -> Vec<MetadataMap> {
    data.choices_out
        .iter()
        .filter_map(|choice| {
            choice
                .get("message")
                .and_then(OtelModelValue::as_object)
                .cloned()
        })
        .collect()
}

#[path = "utils/otel_mappers/mod.rs"]
mod otel_mappers;

#[path = "utils/otel_transforms/mod.rs"]
mod otel_transforms;
