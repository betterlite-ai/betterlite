use std::collections::BTreeSet;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_codex_tools::custom_tools::{
    ToolCall, custom_to_function_tool, extract_custom_tool_names, serialize_tool_call_arguments,
    tool_call_item, tool_call_item_id, unwrap_custom_arguments,
};
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
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    CustomNames {
        tools: Vec<Json>,
    },
    CustomSchema {
        custom_tools: Vec<IndexMap<String, Json>>,
    },
    ArgumentStrings {
        values: Vec<Json>,
        arguments: Vec<String>,
        padded: Vec<Padded>,
        ids: Vec<IdCase>,
        calls: Vec<CallCase>,
        custom_names: BTreeSet<String>,
    },
    ToolChoices {
        choices: Vec<Json>,
    },
    RecordHelpers {
        roles: Vec<IndexMap<String, Json>>,
        plain: Vec<Json>,
        records: Vec<IndexMap<String, Json>>,
    },
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
            Input::ArgumentStrings {
                values,
                arguments,
                padded,
                ids,
                calls,
                custom_names,
            } => argument_strings(values, arguments, padded, ids, calls, custom_names),
            Input::ToolChoices { choices } => Ok(json!(Choices {
                request: choices.iter().map(chat_tool_choice).collect(),
                response: choices.iter().map(responses_tool_choice).collect(),
            })),
            Input::RecordHelpers {
                roles,
                plain,
                records,
            } => record_helpers(roles, plain, records),
        }
    }
}

fn argument_strings(
    values: &[Json],
    arguments: &[String],
    padded: &[Padded],
    ids: &[IdCase],
    calls: &[CallCase],
    custom_names: &BTreeSet<String>,
) -> crate::helpers::HelperResult {
    let limits = ApiLimits::default();
    let serialized: Vec<String> = values
        .iter()
        .map(|value| serialize_tool_call_arguments(value, "fallback"))
        .collect();
    let unwrapped = arguments
        .iter()
        .map(|text| unwrap_custom_arguments(text.as_bytes(), limits))
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let padded = padded
        .iter()
        .map(|case| {
            let wrapped = format!(r#"{{"content":"{}"}}"#, case.fill.repeat(case.count));
            unwrap_custom_arguments(wrapped.as_bytes(), limits)
        })
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let ids: Vec<String> = ids
        .iter()
        .map(|case| tool_call_item_id(&case.item_type, &case.tool_id))
        .collect();
    let items = calls
        .iter()
        .map(|case| {
            let call = ToolCall {
                call_id: &case.call_id,
                name: &case.name,
                arguments_or_input: &case.arguments,
                status: &case.status,
            };
            tool_call_item(call, custom_names, limits)
        })
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    Ok(json!({
        "serialized": serialized,
        "unwrapped": unwrapped,
        "padded": padded,
        "ids": ids,
        "items": items,
    }))
}

fn record_helpers(
    roles: &[IndexMap<String, Json>],
    plain: &[Json],
    records: &[IndexMap<String, Json>],
) -> crate::helpers::HelperResult {
    let roles = roles
        .iter()
        .map(input_item_role)
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let plain = plain
        .iter()
        .map(|value| attribute_fields(&AttributeValue::Plain(value.clone())))
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let records = records
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
    Ok(json!({"roles": roles, "plain": plain, "records": records}))
}
