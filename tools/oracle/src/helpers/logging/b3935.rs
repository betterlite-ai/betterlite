mod tests;

use betterlite_logging::datadog_llm_obs::arguments::to_dd_arguments;
use betterlite_logging::datadog_llm_obs::messages::to_dd_messages;
use betterlite_logging::datadog_llm_obs::response_messages::response_messages;
use betterlite_logging::datadog_llm_obs::tool_definitions::to_dd_tool_definitions;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function")]
pub(crate) enum Input {
    #[serde(rename = "messages")]
    Messages { messages: Option<Json> },
    #[serde(rename = "tool_arguments")]
    ToolArguments { arguments: Vec<ArgumentPiece> },
    #[serde(rename = "tool_definitions")]
    ToolDefinitions { model_parameters: Option<Json> },
    #[serde(rename = "response_messages")]
    ResponseMessages {
        response: Option<Json>,
        call_type: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum ArgumentPiece {
    Text(String),
    Repeated {
        text: String,
        prefix: Option<String>,
        suffix: Option<String>,
        times: usize,
    },
}

impl ArgumentPiece {
    fn expanded(&self) -> Json {
        match self {
            Self::Text(text) => Json::Text(text.clone()),
            Self::Repeated {
                text,
                prefix,
                suffix,
                times,
            } => Json::Text(format!(
                "{}{}{}",
                prefix.as_deref().unwrap_or(""),
                text.repeat(*times),
                suffix.as_deref().unwrap_or("")
            )),
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Messages { messages } => encode(&to_dd_messages(messages.as_ref())),
            Input::ToolArguments { arguments } => arguments
                .iter()
                .map(|piece| encode(&to_dd_arguments(&piece.expanded())))
                .collect::<Result<Vec<Value>, _>>()
                .map(Value::Array),
            Input::ToolDefinitions { model_parameters } => {
                encode(&to_dd_tool_definitions(model_parameters.as_ref()))
            }
            Input::ResponseMessages {
                response,
                call_type,
            } => encode(&response_messages(response.as_ref(), call_type.as_deref())),
        }
    }
}

fn encode<T>(value: &T) -> crate::helpers::HelperResult
where
    T: Serialize,
{
    match serde_json::to_value(value) {
        Ok(encoded) => Ok(encoded),
        Err(error) => Err(Box::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}
