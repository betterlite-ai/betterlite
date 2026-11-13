mod tests;

use betterlite_logging::datadog_llm_obs::arguments::to_dd_arguments;
use betterlite_logging::datadog_llm_obs::messages::to_dd_messages;
use betterlite_logging::datadog_llm_obs::response_messages::response_messages;
use betterlite_logging::datadog_llm_obs::tool_definitions::to_dd_tool_definitions;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function")]
pub(crate) enum Input {
    #[serde(rename = "messages")]
    Messages { messages: Option<Json> },
    #[serde(rename = "tool_arguments")]
    ToolArguments { arguments: Vec<Json> },
    #[serde(rename = "tool_definitions")]
    ToolDefinitions { model_parameters: Option<Json> },
    #[serde(rename = "response_messages")]
    ResponseMessages {
        response: Option<Json>,
        call_type: Option<String>,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::Messages { messages } => serde_json::to_value(to_dd_messages(messages.as_ref())),
            Input::ToolArguments { arguments } => Ok(Value::Array(
                arguments
                    .iter()
                    .map(|piece| serde_json::to_value(to_dd_arguments(piece)).unwrap_or_default())
                    .collect(),
            )),
            Input::ToolDefinitions { model_parameters } => {
                serde_json::to_value(to_dd_tool_definitions(model_parameters.as_ref()))
            }
            Input::ResponseMessages {
                response,
                call_type,
            } => serde_json::to_value(response_messages(response.as_ref(), call_type.as_deref())),
        };
        Ok(value.unwrap_or_default())
    }
}
