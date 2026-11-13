use betterlite_core::message_utilities::validators::{
    StopSequences, cleanup_none_field_in_message, convert_list_message_to_dict,
    fix_openai_messages, fix_thinking_param, validate_and_fix_openai_tools,
    validate_openai_user_messages, validate_optional_params, validate_tool_choice,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;

use super::errors;
use crate::helpers::HelperResult;

const MEDIUM_THINKING_BUDGET: u32 = 2048;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Call {
    ConvertListMessageToDict(MessageListArgs),
    FixOpenaiMessages(MessageListArgs),
    ValidateAndFixOpenaiTools(OptionalToolsArgs),
    FixThinkingParam(ThinkingArgs),
    CleanupNoneFieldInMessage(MessageArgs),
    ValidateOpenaiUserMessages(MessageListArgs),
    ValidateToolChoice(ToolChoiceArgs),
    ValidateOptionalParams(OptionalParamsArgs),
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageListArgs {
    messages: Vec<Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OptionalToolsArgs {
    #[serde(default)]
    tools: Option<Vec<Json>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ThinkingArgs {
    thinking: Json,
    #[serde(default)]
    medium_budget: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageArgs {
    message: Json,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolChoiceArgs {
    tool_choice: Json,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OptionalParamsArgs {
    #[serde(default)]
    stop: Option<StopSequences>,
    #[serde(default)]
    disable_stop_sequence_limit: bool,
}

pub(super) fn run(call: &Call) -> HelperResult {
    match call {
        Call::ConvertListMessageToDict(args) => convert_list_message_to_dict(&args.messages)
            .map(|converted| json!(converted))
            .map_err(|error| errors::value_error(error.to_string())),
        Call::FixOpenaiMessages(args) => fix_openai_messages(&args.messages)
            .map(|fixed| json!(fixed))
            .map_err(|error| errors::value_error(error.to_string())),
        Call::ValidateAndFixOpenaiTools(args) => {
            Ok(json!(validate_and_fix_openai_tools(args.tools.as_deref())))
        }
        Call::FixThinkingParam(args) => Ok(json!(fix_thinking_param(
            &args.thinking,
            args.medium_budget.unwrap_or(MEDIUM_THINKING_BUDGET)
        ))),
        Call::CleanupNoneFieldInMessage(args) => cleanup_none_field_in_message(&args.message)
            .map(|cleaned| json!(cleaned))
            .map_err(|error| errors::value_error(error.to_string())),
        Call::ValidateOpenaiUserMessages(args) => validate_openai_user_messages(&args.messages)
            .map(|validated| json!(validated))
            .map_err(|error| errors::value_error(error.to_string())),
        Call::ValidateToolChoice(args) => validate_tool_choice(&args.tool_choice)
            .map(|choice| json!(choice))
            .map_err(|error| {
                error.class().map_or_else(
                    || errors::value_error(error.to_string()),
                    |class| {
                        errors::status_error(
                            "BadRequestError",
                            class.metadata().status_code,
                            error.to_string(),
                        )
                    },
                )
            }),
        Call::ValidateOptionalParams(args) => Ok(json!(validate_optional_params(
            args.stop.clone(),
            args.disable_stop_sequence_limit
        ))),
    }
}
