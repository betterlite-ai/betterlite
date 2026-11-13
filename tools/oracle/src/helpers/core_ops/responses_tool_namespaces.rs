use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_codex_tools::records::AttributeValue;
use betterlite_core::api::responses_tool_namespaces::records::{
    CacheWrite, ChoiceRecord, FunctionRecord, ProviderFields, ToolCallRecord,
};
use betterlite_core::api::responses_tool_namespaces::responses_tools::{
    chat_tools_to_responses, responses_tools_to_chat,
};
use betterlite_core::api::responses_tool_namespaces::tool_outputs::tool_outputs;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::FunctionCallStatus;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum FieldsInput {
    Mapping { fields: IndexMap<String, Json> },
    Record { fields: IndexMap<String, Json> },
    Plain { value: Json },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallInput {
    id: String,
    r#type: String,
    name: String,
    arguments: Json,
    status: Option<FunctionCallStatus>,
    tool_fields: Option<FieldsInput>,
    function_fields: Option<FieldsInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChoiceInput {
    tool_calls: Vec<CallInput>,
    provider_specific_fields: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputsCase {
    tools: Vec<Json>,
    choices: Vec<ChoiceInput>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ToolsToChat { tools: Vec<Json> },
    ToolsToResponses { tools: Vec<Json> },
    ToolOutputs(OutputsCase),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::ToolsToChat { tools } => {
                let conversion = responses_tools_to_chat(tools).or_value_error()?;
                Ok(json!({
                    "chat_tools": conversion.chat_tools,
                    "web_search_options": conversion.web_search_options,
                    "warnings": conversion.warnings,
                }))
            }
            Input::ToolsToResponses { tools } => {
                Ok(json!(chat_tools_to_responses(tools).or_value_error()?))
            }
            Input::ToolOutputs(case) => outputs(case),
        }
    }
}

fn provider_fields(input: &FieldsInput) -> ProviderFields {
    match input {
        FieldsInput::Mapping { fields } => ProviderFields::Mapping(fields.clone()),
        FieldsInput::Record { fields } => {
            let pairs = fields
                .iter()
                .map(|(name, value)| Json::List(vec![Json::Text(name.clone()), value.clone()]))
                .collect();
            ProviderFields::Attributes(AttributeValue::Record(pairs))
        }
        FieldsInput::Plain { value } => {
            ProviderFields::Attributes(AttributeValue::Plain(value.clone()))
        }
    }
}

fn tool_call(call: &CallInput) -> ToolCallRecord {
    ToolCallRecord {
        id: call.id.clone(),
        kind: call.r#type.clone(),
        function: FunctionRecord {
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            status: call.status,
            provider_specific_fields: call.function_fields.as_ref().map(provider_fields),
        },
        provider_specific_fields: call.tool_fields.as_ref().map(provider_fields),
    }
}

fn outputs(case: &OutputsCase) -> crate::helpers::HelperResult {
    let choices: Vec<ChoiceRecord> = case
        .choices
        .iter()
        .map(|choice| ChoiceRecord {
            tool_calls: choice.tool_calls.iter().map(tool_call).collect(),
            provider_specific_fields: choice
                .provider_specific_fields
                .clone()
                .map_or(Json::Null, Json::Object),
        })
        .collect();
    let result = tool_outputs(&choices, &case.tools, ApiLimits::default()).or_value_error()?;
    let mut cached: BTreeMap<String, String> = BTreeMap::new();
    for CacheWrite { key, value } in result.cache_writes {
        cached.insert(key, value.function.name);
    }
    Ok(json!({"items": result.items, "cached": cached}))
}
