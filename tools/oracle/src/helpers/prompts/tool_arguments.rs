use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::arguments::concatenated::{
    salvage_concatenated_tool_arguments, split_concatenated_json_objects,
};
use betterlite_prompts::arguments::parse::{ParseArgumentsError, parse_tool_call_arguments};
use betterlite_prompts::arguments::shapes::{
    check_is_function_call, convert_custom_tool_format_to_chat_shape,
    convert_custom_tool_format_to_responses_shape, get_tool_call_names, is_function_call,
    with_prompt_cache_breakpoint,
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
    SplitConcatenatedJsonObjects {
        raw: String,
    },
    SalvageConcatenatedToolArguments {
        raw: String,
    },
    GetToolCallNames {
        tools: Vec<Map<String, Value>>,
    },
    IsFunctionCall {
        optional_params: Map<String, Value>,
    },
    CheckIsFunctionCall {
        #[serde(default)]
        optional_params: Option<Map<String, Value>>,
    },
    ConvertCustomToolFormatToChatShape {
        format_obj: Map<String, Value>,
    },
    ConvertCustomToolFormatToResponsesShape {
        format_obj: Map<String, Value>,
    },
    WithPromptCacheBreakpoint {
        target: Map<String, Value>,
        #[serde(default)]
        marker: Option<Value>,
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
                .map_err(|error| failure(&error)),
            Input::SplitConcatenatedJsonObjects { raw } => {
                Ok(Value::from(split_concatenated_json_objects(raw)))
            }
            Input::SalvageConcatenatedToolArguments { raw } => {
                Ok(Value::from(salvage_concatenated_tool_arguments(raw)))
            }
            Input::GetToolCallNames { tools } => Ok(Value::from(get_tool_call_names(tools))),
            Input::IsFunctionCall { optional_params } => {
                Ok(Value::from(is_function_call(optional_params)))
            }
            Input::CheckIsFunctionCall { optional_params } => Ok(Value::from(
                check_is_function_call(optional_params.as_ref()),
            )),
            Input::ConvertCustomToolFormatToChatShape { format_obj } => Ok(Value::Object(
                convert_custom_tool_format_to_chat_shape(format_obj),
            )),
            Input::ConvertCustomToolFormatToResponsesShape { format_obj } => Ok(Value::Object(
                convert_custom_tool_format_to_responses_shape(format_obj),
            )),
            Input::WithPromptCacheBreakpoint { target, marker } => Ok(Value::Object(
                with_prompt_cache_breakpoint(target, marker.as_ref()),
            )),
        }
    }
}

fn failure(error: &ParseArgumentsError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;
