use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::{
    CustomToolCallOutputItem, CustomToolCallType, FunctionCallStatus,
};
use indexmap::IndexMap;
use serde::Serialize;

use super::error::NamespaceToolsError;
use super::hosted_search::{HostedSearchCall, calls_by_id};
use super::namespace_names::{NamespacedName, name_map, restore_name};
use super::records::{CacheWrite, ChoiceRecord, ProviderFields, ToolCallRecord};
use super::values::has_value;
use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_codex_tools::custom_tools::{
    extract_custom_tool_names, serialize_tool_call_arguments, tool_call_item_id,
    unwrap_custom_arguments,
};
use crate::api::responses_codex_tools::records::{AttributeValue, attribute_fields};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionCallType {
    FunctionCall,
}

// B902's OutputFunctionToolCall always writes a phase key, which the SDK's ResponseFunctionToolCall does not have
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionCallItem {
    pub name: String,
    pub arguments: String,
    pub call_id: String,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: FunctionCallType,
    pub status: FunctionCallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ToolOutputItem {
    Function(FunctionCallItem),
    Custom(CustomToolCallOutputItem),
    WebSearch(HostedSearchCall),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputs {
    pub items: Vec<ToolOutputItem>,
    pub cache_writes: Vec<CacheWrite>,
}

fn provider_fields(
    tool: &ToolCallRecord,
) -> Result<Option<IndexMap<String, Json>>, NamespaceToolsError> {
    let chosen = [
        &tool.provider_specific_fields,
        &tool.function.provider_specific_fields,
    ]
    .into_iter()
    .flatten()
    .find(|fields| match fields {
        ProviderFields::Mapping(mapping) => !mapping.is_empty(),
        ProviderFields::Attributes(AttributeValue::Plain(value)) => has_value(value),
        ProviderFields::Attributes(AttributeValue::Record(_)) => true,
    });
    let converted = match chosen {
        None => return Ok(None),
        Some(ProviderFields::Mapping(mapping)) => mapping.clone(),
        Some(ProviderFields::Attributes(attributes)) => attribute_fields(attributes)?,
    };
    Ok(Some(converted).filter(|fields| !fields.is_empty()))
}

struct RequestContext {
    custom_names: BTreeSet<String>,
    namespaces: BTreeMap<String, NamespacedName>,
    searches: BTreeMap<String, HostedSearchCall>,
    limits: ApiLimits,
}

fn output_item(
    call: &ToolCallRecord,
    context: &RequestContext,
) -> Result<ToolOutputItem, NamespaceToolsError> {
    let arguments = serialize_tool_call_arguments(&call.function.arguments, "");
    let status = call
        .function
        .status
        .unwrap_or(FunctionCallStatus::Completed);
    if let Some(search) = context.searches.get(&call.id) {
        return Ok(ToolOutputItem::WebSearch(search.clone()));
    }
    if context.custom_names.contains(&call.function.name) {
        return Ok(ToolOutputItem::Custom(CustomToolCallOutputItem {
            r#type: CustomToolCallType::CustomToolCall,
            call_id: call.id.clone(),
            id: Some(tool_call_item_id("custom_tool_call", &call.id)),
            name: call.function.name.clone(),
            input: unwrap_custom_arguments(arguments.as_bytes(), context.limits)?,
            status: Some(status),
            extra: BTreeMap::new(),
        }));
    }
    let (name, namespace) = restore_name(&call.function.name, &context.namespaces);
    Ok(ToolOutputItem::Function(FunctionCallItem {
        name,
        arguments,
        call_id: call.id.clone(),
        id: tool_call_item_id("function_call", &call.id),
        kind: FunctionCallType::FunctionCall,
        status,
        namespace: namespace.filter(|restored| !restored.is_empty()),
        provider_specific_fields: provider_fields(call)?,
    }))
}

// each call is cached under its id first, so a call dropped from the output still serves a later tool result
pub fn tool_outputs(
    choices: &[ChoiceRecord],
    request_tools: &[Json],
    limits: ApiLimits,
) -> Result<ToolOutputs, NamespaceToolsError> {
    let calls: Vec<&ToolCallRecord> = choices
        .iter()
        .flat_map(|choice| choice.tool_calls.iter())
        .collect();
    let cache_writes = calls
        .iter()
        .map(|call| CacheWrite {
            key: call.id.clone(),
            value: (*call).clone(),
        })
        .collect();
    let context = RequestContext {
        custom_names: extract_custom_tool_names(request_tools),
        namespaces: name_map(request_tools)?,
        searches: calls_by_id(choices),
        limits,
    };
    let items = calls
        .into_iter()
        .filter(|call| call.kind == "function")
        .map(|call| output_item(call, &context))
        .collect::<Result<_, _>>()?;
    Ok(ToolOutputs {
        items,
        cache_writes,
    })
}
