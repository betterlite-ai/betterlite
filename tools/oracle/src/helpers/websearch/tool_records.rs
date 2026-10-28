pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
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

fn builder(function: &str) -> crate::helpers::HelperResult {
    if function == "get_litellm_web_search_tool" {
        return anthropic_builder();
    }
    if function == "get_litellm_web_search_tool_openai" {
        return openai_builder();
    }
    responses_builder()
}

fn anthropic_builder() -> crate::helpers::HelperResult {
    let tool = betterlite_websearch::tools::get_litellm_web_search_tool();
    serde_json::to_value(tool).map_err(|error| helper_error(error, "Serialization"))
}

fn openai_builder() -> crate::helpers::HelperResult {
    let tool = betterlite_websearch::tools::openai_web_search_tool();
    serde_json::to_value(tool).map_err(|error| helper_error(error, "Serialization"))
}

fn responses_builder() -> crate::helpers::HelperResult {
    let tool = betterlite_websearch::tools::responses_web_search_tool();
    serde_json::to_value(tool).map_err(|error| helper_error(error, "Serialization"))
}

fn helper_error<E>(error: E, class: &str) -> Box<crate::helpers::HelperError>
where
    E: std::fmt::Display,
{
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn detection(function: &str, tool: &betterlite_websearch::tools::Tool) -> serde_json::Value {
    match function {
        "is_web_search_tool_responses" => serde_json::json!(
            betterlite_websearch::tools::is_web_search_tool_responses(tool)
        ),
        "is_web_search_tool_chat_completion" => {
            serde_json::json!(betterlite_websearch::tools::is_chat_web_search_tool(tool))
        }
        _ => serde_json::json!(betterlite_websearch::tools::is_web_search_tool(tool)),
    }
}

fn outcome(function: &str, input: &Input) -> crate::helpers::HelperResult {
    if function == "search_outcome_succeeded" {
        let response = input.response.clone().map(decode).transpose()?;
        return Ok(serde_json::json!(
            betterlite_websearch::records::SearchOutcome::Succeeded(
                betterlite_websearch::records::SearchSucceeded {
                    text: input.text.clone().unwrap_or_default(),
                    response
                }
            )
        ));
    }
    if function == "search_outcome_failed" {
        let error_code = decode(input.error_code.clone().unwrap_or_default())?;
        return Ok(serde_json::json!(
            betterlite_websearch::records::SearchOutcome::Failed(
                betterlite_websearch::records::SearchFailed {
                    error_code,
                    message: input.message.clone().unwrap_or_default()
                }
            )
        ));
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

fn decode<T>(value: serde_json::Value) -> Result<T, Box<crate::helpers::HelperError>>
where
    T: serde::de::DeserializeOwned,
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
    tool: betterlite_websearch::tools::Tool,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    response: Option<serde_json::Value>,
    #[serde(default)]
    error_code: Option<serde_json::Value>,
    #[serde(default)]
    message: Option<String>,
}
