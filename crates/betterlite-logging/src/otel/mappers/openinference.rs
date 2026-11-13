use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Value;
use std::collections::{BTreeMap, BTreeSet};

use super::base::{AttributeMap, AttributeMapper, AttributeTable, SpanData};
use super::utils::{collect, message_content, tool_definition_attrs};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::payloads::tool_definition::ToolDefinition;
use crate::otel::model::utils::coercion::OtelModelValue;

#[derive(serde::Serialize)]
struct InvocationParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
}

#[derive(serde::Serialize)]
struct MessageBlob<'message> {
    role: Option<&'message str>,
    content: Option<&'message str>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    Input,
    Output,
}

pub struct OpenInferenceMapper {
    pub tool_attr_budget: usize,
}

// one attribute per line keeps a table readable; rustfmt stacks every closure vertically
#[rustfmt::skip]
const LLM_CALL_ATTRS: AttributeTable<LLMCallSpanData> = &[
    ("openinference.span.kind", |_data| Some(Value::from("LLM"))),
    ("llm.model_name", |data| (!data.request_model.is_empty()).then(|| Value::from(data.request_model.clone()))),
    ("llm.provider", |data| (!data.provider.is_empty()).then(|| Value::from(data.provider.clone()))),
    ("llm.token_count.prompt", |data| data.usage.input_tokens.map(Value::I64)),
    ("llm.token_count.completion", |data| data.usage.output_tokens.map(Value::I64)),
    ("llm.token_count.total", |data| data.usage.total_tokens.map(Value::I64)),
];

#[rustfmt::skip]
const TOOL_ATTRS: AttributeTable<ToolDefinition> = &[
    ("tool.name", |tool| Some(Value::from(tool.name.clone()))),
    ("tool.description", |tool| tool.description.clone().filter(|description| !description.is_empty()).map(Value::from)),
    ("tool.json_schema", |tool| tool.parameters_json.clone().filter(|parameters| !parameters.is_empty()).map(Value::from)),
];

impl AttributeMapper for OpenInferenceMapper {
    fn map(&self, data: &SpanData<'_>, _environment: &dyn EnvironmentLookup) -> AttributeMap {
        match data {
            SpanData::LLMCall(call) => self.llm_call(call),
            SpanData::MCPToolCall(_)
            | SpanData::MCPListTools(_)
            | SpanData::Guardrail(_)
            | SpanData::Service(_) => AttributeMap::new(),
        }
    }
}

impl OpenInferenceMapper {
    fn llm_call(&self, data: &LLMCallSpanData) -> AttributeMap {
        let mut attrs = collect(LLM_CALL_ATTRS, data);
        attrs.extend(invocation_parameters(data));
        attrs.extend(indexed_messages(
            "llm.input_messages",
            "input.value",
            &data.messages_in,
            |message| Some(message),
        ));
        attrs.extend(indexed_messages(
            "llm.output_messages",
            "output.value",
            &data.choices_out,
            |choice| choice.get("message").and_then(OtelModelValue::as_object),
        ));
        attrs.extend(tool_definition_attrs(
            "llm.tools.",
            &data.tools,
            TOOL_ATTRS,
            self.tool_attr_budget,
        ));
        attrs
    }
}

fn invocation_parameters(data: &LLMCallSpanData) -> AttributeMap {
    let parameters = InvocationParameters {
        temperature: data.request_params.temperature,
        top_p: data.request_params.top_p,
        top_k: data.request_params.top_k,
        max_tokens: data.request_params.max_tokens,
        frequency_penalty: data.request_params.frequency_penalty,
        presence_penalty: data.request_params.presence_penalty,
        seed: data.request_params.seed,
    };
    let Ok(text) = serde_json::to_string(&parameters) else {
        return AttributeMap::new();
    };
    if text == "{}" {
        return AttributeMap::new();
    }
    AttributeMap::from([("llm.invocation_parameters".to_owned(), Value::from(text))])
}

fn indexed_messages(
    prefix: &str,
    value_key: &str,
    messages: &[MetadataMap],
    pick: fn(&MetadataMap) -> Option<&MetadataMap>,
) -> AttributeMap {
    let entries: Vec<(Option<&str>, Option<String>)> = messages
        .iter()
        .map(|message| {
            pick(message).map_or((None, None), |picked| {
                (
                    picked.get("role").and_then(OtelModelValue::as_str),
                    message_content(picked),
                )
            })
        })
        .collect();
    let mut attrs = AttributeMap::new();
    for (index, (role, content)) in entries.iter().enumerate() {
        if let Some(found_role) = role {
            attrs.insert(
                format!("{prefix}.{index}.message.role"),
                Value::from((*found_role).to_owned()),
            );
        }
        if let Some(found_content) = content {
            attrs.insert(
                format!("{prefix}.{index}.message.content"),
                Value::from(found_content.clone()),
            );
        }
    }
    if entries.is_empty() {
        return attrs;
    }
    let blob: Vec<MessageBlob<'_>> = entries
        .iter()
        .map(|(role, content)| MessageBlob {
            role: *role,
            content: content.as_deref(),
        })
        .collect();
    if let Ok(text) = serde_json::to_string(&blob) {
        attrs.insert(value_key.to_owned(), Value::from(text));
    }
    attrs
}

#[must_use]
pub fn fit_indexed_messages(attrs: AttributeMap, budget: Option<usize>) -> AttributeMap {
    let Some(limit) = budget else {
        return attrs;
    };
    if attrs.len() <= limit {
        return attrs;
    }
    let groups = message_key_groups(&attrs);
    let order = shed_order(&groups);
    let excess = attrs.len().saturating_sub(limit);
    let mut shed_count = order.len();
    let mut running: usize = 0;
    for (position, group) in order.iter().enumerate() {
        running = running.saturating_add(groups.get(group).map_or(0, Vec::len));
        if running >= excess {
            shed_count = position.saturating_add(1);
            break;
        }
    }
    let shed: BTreeSet<&str> = order
        .iter()
        .take(shed_count)
        .filter_map(|group| groups.get(group))
        .flatten()
        .map(String::as_str)
        .collect();
    attrs
        .into_iter()
        .filter(|(key, _value)| !shed.contains(key.as_str()))
        .collect()
}

fn message_key_groups(attrs: &AttributeMap) -> BTreeMap<(Family, usize), Vec<String>> {
    let mut groups: BTreeMap<(Family, usize), Vec<String>> = BTreeMap::new();
    for key in attrs.keys() {
        let family = if key.starts_with("llm.input_messages.") {
            Family::Input
        } else if key.starts_with("llm.output_messages.") {
            Family::Output
        } else {
            continue;
        };
        let Some(index) = key.split('.').nth(2).and_then(|index| index.parse().ok()) else {
            continue;
        };
        groups.entry((family, index)).or_default().push(key.clone());
    }
    groups
}

fn shed_order(groups: &BTreeMap<(Family, usize), Vec<String>>) -> Vec<(Family, usize)> {
    let indexes = |family: Family| -> Vec<usize> {
        groups
            .keys()
            .filter(|(kind, _index)| *kind == family)
            .map(|(_kind, index)| *index)
            .collect()
    };
    let inputs = indexes(Family::Input);
    let outputs = indexes(Family::Output);
    let first_input = inputs.first().copied();
    let mut order = Vec::new();
    order.extend(
        inputs
            .iter()
            .skip(1)
            .take(inputs.len().saturating_sub(2))
            .map(|index| (Family::Input, *index)),
    );
    order.extend(
        outputs
            .iter()
            .skip(1)
            .rev()
            .map(|index| (Family::Output, *index)),
    );
    if let Some(first) = first_input {
        order.push((Family::Input, first));
    }
    if let Some(last) = inputs.last().filter(|last| Some(**last) != first_input) {
        order.push((Family::Input, *last));
    }
    if let Some(first) = outputs.first() {
        order.push((Family::Output, *first));
    }
    order
}

#[path = "openinference/message_shedding/tests.rs"]
mod message_shedding;
#[path = "openinference/tests.rs"]
mod tests;
