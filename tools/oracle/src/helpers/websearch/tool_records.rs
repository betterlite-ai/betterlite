use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_websearch::records::{SearchFailed, SearchOutcome, SearchSucceeded};
use betterlite_websearch::tools;
use betterlite_websearch::tools::Tool;
use serde::de::DeserializeOwned;
use std::fmt::Display;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = if input.function.starts_with("get_litellm") {
            builder(&input.function)?
        } else if input.function.starts_with("is_web_search") {
            detection(&input.function, &input.tool)
        } else if input.function.starts_with("search_outcome") {
            outcome(&input.function, input)?
        } else {
            return Self::not_built(&input.function);
        };
        Ok(output)
    }
}

fn builder(function: &str) -> HelperResult {
    let encoded = match function {
        "get_litellm_web_search_tool" => serde_json::to_value(tools::get_litellm_web_search_tool()),
        "get_litellm_web_search_tool_openai" => {
            serde_json::to_value(tools::openai_web_search_tool())
        }
        _ => serde_json::to_value(tools::responses_web_search_tool()),
    };
    encoded.map_err(|error| helper_error(error, "Serialization"))
}

fn helper_error<E>(error: E, class: &str) -> Box<HelperError>
where
    E: Display,
{
    let message = error.to_string();
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn detection(function: &str, tool: &Tool) -> serde_json::Value {
    match function {
        "is_web_search_tool_responses" => {
            serde_json::json!(tools::is_web_search_tool_responses(tool))
        }
        "is_web_search_tool_chat_completion" => {
            serde_json::json!(tools::is_chat_web_search_tool(tool))
        }
        _ => serde_json::json!(tools::is_web_search_tool(tool)),
    }
}

fn outcome(function: &str, input: &Input) -> HelperResult {
    if function == "search_outcome_succeeded" {
        let response = input.response.clone().map(decode).transpose()?;
        return Ok(serde_json::json!(SearchOutcome::Succeeded(
            SearchSucceeded {
                text: input.text.clone().unwrap_or_default(),
                response
            }
        )));
    }
    if function == "search_outcome_failed" {
        let error_code = decode(input.error_code.clone().unwrap_or_default())?;
        return Ok(serde_json::json!(SearchOutcome::Failed(SearchFailed {
            error_code,
            message: input.message.clone().unwrap_or_default()
        })));
    }
    Ok(serde_json::json!([
        "invalid_tool_input",
        "unavailable",
        "max_uses_exceeded",
        "too_many_requests",
        "query_too_long",
        "request_too_large"
    ]))
}

fn decode<T>(value: serde_json::Value) -> Result<T, Box<HelperError>>
where
    T: DeserializeOwned,
{
    match serde_json::from_value(value) {
        Ok(decoded) => Ok(decoded),
        Err(error) => Err(helper_error(error, "InvalidInput")),
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    function: String,
    #[serde(default)]
    tool: Tool,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    response: Option<serde_json::Value>,
    #[serde(default)]
    error_code: Option<serde_json::Value>,
    #[serde(default)]
    message: Option<String>,
}
