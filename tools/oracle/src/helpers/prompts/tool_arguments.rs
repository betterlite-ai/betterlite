use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::arguments::concatenated::{
    salvage_concatenated_arguments, split_concatenated_json,
};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::arguments::parse::{ParseArgumentsError, parse_tool_call_arguments};
use betterlite_prompts::arguments::shapes::{
    check_is_function_call, custom_tool_to_chat_shape, custom_tool_to_responses_shape,
    get_tool_call_names, is_function_call, with_prompt_cache_breakpoint,
};
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    ParseToolCallArguments {
        arguments: String,
        #[serde(default)]
        tool_name: Option<String>,
        #[serde(default)]
        context: Option<String>,
    },
    #[serde(rename = "split_concatenated_json_objects")]
    SplitConcatenatedJson {
        raw: String,
    },
    #[serde(rename = "salvage_concatenated_tool_arguments")]
    SalvageConcatenatedArguments {
        raw: String,
    },
    GetToolCallNames {
        tools: Vec<Object>,
    },
    IsFunctionCall {
        optional_params: Object,
    },
    CheckIsFunctionCall {
        #[serde(default)]
        optional_params: Option<Object>,
    },
    #[serde(rename = "convert_custom_tool_format_to_chat_shape")]
    CustomToolToChatShape {
        format_obj: Object,
    },
    #[serde(rename = "convert_custom_tool_format_to_responses_shape")]
    CustomToolToResponsesShape {
        format_obj: Object,
    },
    WithPromptCacheBreakpoint {
        target: Object,
        #[serde(default)]
        marker: Option<Json>,
    },
}

pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ParseToolCallArguments {
                arguments,
                tool_name,
                context,
            } => parse_tool_call_arguments(arguments, tool_name.as_deref(), context.as_deref())
                .map(json_to_value)
                .map_err(|error| malformed_arguments_error(&error)),
            Input::SplitConcatenatedJson { raw } => Ok(Value::Array(
                split_concatenated_json(raw)
                    .into_iter()
                    .map(object_to_value)
                    .collect(),
            )),
            Input::SalvageConcatenatedArguments { raw } => Ok(Value::Array(
                salvage_concatenated_arguments(raw)
                    .into_iter()
                    .map(object_to_value)
                    .collect(),
            )),
            Input::GetToolCallNames { tools } => Ok(Value::from(get_tool_call_names(tools))),
            Input::IsFunctionCall { optional_params } => {
                Ok(Value::from(is_function_call(optional_params)))
            }
            Input::CheckIsFunctionCall { optional_params } => Ok(Value::from(
                check_is_function_call(optional_params.as_ref()),
            )),
            Input::CustomToolToChatShape { format_obj } => {
                Ok(object_to_value(custom_tool_to_chat_shape(format_obj)))
            }
            Input::CustomToolToResponsesShape { format_obj } => {
                Ok(object_to_value(custom_tool_to_responses_shape(format_obj)))
            }
            Input::WithPromptCacheBreakpoint { target, marker } => Ok(object_to_value(
                with_prompt_cache_breakpoint(target, marker.as_ref()),
            )),
        }
    }
}

fn json_to_value(json: Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(flag),
        Json::Number(number) => Value::Number(number),
        Json::Text(text) => Value::String(text),
        Json::List(items) => Value::Array(items.into_iter().map(json_to_value).collect()),
        Json::Object(fields) => object_to_value(fields),
    }
}

fn object_to_value(object: Object) -> Value {
    let entries: Map<String, Value> = object
        .into_iter()
        .map(|(key, value)| (key, json_to_value(value)))
        .collect();
    Value::Object(entries)
}

fn malformed_arguments_error(error: &ParseArgumentsError) -> Box<HelperError> {
    let message = error.to_string();
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
