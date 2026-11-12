use std::collections::BTreeSet;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_codex_tools::custom_tools::{
    ToolCall, ToolCallItem, custom_to_function_tool, extract_custom_tool_names,
    serialize_tool_call_arguments, tool_call_item, tool_call_item_id, unwrap_custom_arguments,
};
use betterlite_core::api::responses_codex_tools::error::CustomToolsError;
use betterlite_core::api::responses_codex_tools::records::{
    AttributeValue, attribute_fields, input_item_role,
};
use betterlite_core::api::responses_codex_tools::tool_choices::{
    chat_tool_choice, responses_tool_choice,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
pub(crate) struct Padded {
    fill: String,
    count: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IdCase {
    item_type: String,
    tool_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CallCase {
    call_id: String,
    name: String,
    arguments: String,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StringsCase {
    values: Vec<Json>,
    arguments: Vec<String>,
    padded: Vec<Padded>,
    ids: Vec<IdCase>,
    calls: Vec<CallCase>,
    custom_names: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordsCase {
    roles: Vec<IndexMap<String, Json>>,
    plain: Vec<Json>,
    records: Vec<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    CustomNames {
        tools: Vec<Json>,
    },
    CustomSchema {
        custom_tools: Vec<IndexMap<String, Json>>,
    },
    ArgumentStrings(StringsCase),
    ToolChoices {
        choices: Vec<Json>,
    },
    RecordHelpers(RecordsCase),
}

#[derive(Serialize)]
struct Choices {
    request: Vec<Json>,
    response: Vec<Json>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::CustomNames { tools } => Ok(json!(extract_custom_tool_names(tools))),
            Input::CustomSchema { custom_tools } => {
                let converted = custom_tools
                    .iter()
                    .map(custom_to_function_tool)
                    .collect::<Result<Vec<_>, _>>()
                    .or_value_error()?;
                Ok(json!(converted))
            }
            Input::ArgumentStrings(case) => argument_strings(case),
            Input::ToolChoices { choices } => Ok(json!(Choices {
                request: choices.iter().map(chat_tool_choice).collect(),
                response: choices.iter().map(responses_tool_choice).collect(),
            })),
            Input::RecordHelpers(case) => record_helpers(case),
        }
    }
}

fn call_items(
    case: &StringsCase,
    limits: ApiLimits,
) -> Result<Vec<ToolCallItem>, CustomToolsError> {
    case.calls
        .iter()
        .map(|call| {
            let request = ToolCall {
                call_id: &call.call_id,
                name: &call.name,
                arguments_or_input: &call.arguments,
                status: &call.status,
            };
            tool_call_item(request, &case.custom_names, limits)
        })
        .collect()
}

fn argument_strings(case: &StringsCase) -> crate::helpers::HelperResult {
    let limits = ApiLimits::default();
    let serialized: Vec<String> = case
        .values
        .iter()
        .map(|value| serialize_tool_call_arguments(value, "fallback"))
        .collect();
    let unwrapped = case
        .arguments
        .iter()
        .map(|text| unwrap_custom_arguments(text.as_bytes(), limits))
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let padded = case
        .padded
        .iter()
        .map(|pad| {
            let wrapped = format!(r#"{{"content":"{}"}}"#, pad.fill.repeat(pad.count));
            unwrap_custom_arguments(wrapped.as_bytes(), limits)
        })
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let item_ids: Vec<String> = case
        .ids
        .iter()
        .map(|id| tool_call_item_id(&id.item_type, &id.tool_id))
        .collect();
    let items = call_items(case, limits).or_value_error()?;
    Ok(json!({
        "serialized": serialized,
        "unwrapped": unwrapped,
        "padded": padded,
        "ids": item_ids,
        "items": items,
    }))
}

fn record_helpers(case: &RecordsCase) -> crate::helpers::HelperResult {
    let role_names = case
        .roles
        .iter()
        .map(input_item_role)
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let plain_fields = case
        .plain
        .iter()
        .map(|value| attribute_fields(&AttributeValue::Plain(value.clone())))
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let record_fields = case
        .records
        .iter()
        .map(|fields| {
            let pairs = fields
                .iter()
                .map(|(name, value)| Json::List(vec![Json::Text(name.clone()), value.clone()]))
                .collect();
            attribute_fields(&AttributeValue::Record(pairs))
        })
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    Ok(json!({"roles": role_names, "plain": plain_fields, "records": record_fields}))
}
