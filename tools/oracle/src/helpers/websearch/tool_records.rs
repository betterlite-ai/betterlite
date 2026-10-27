pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input.function.as_str() {
            "get_litellm_web_search_tool" | "get_litellm_web_search_tool_openai" | "get_litellm_web_search_tool_responses" => builder(&input.function)?,
            "is_web_search_tool_responses" | "is_web_search_tool_chat_completion" | "is_web_search_tool" => detection(&input.function, &input.tool),
            "search_outcome_succeeded" | "search_outcome_failed" | "search_outcome_error_codes" => outcome(&input.function, input)?,
            other => return Self::not_built(other),
        };
        Ok(output)
    }
}

fn builder(function: &str) -> crate::helpers::HelperResult {
    use betterlite_websearch::tools;
    let value = match function { "get_litellm_web_search_tool" => serde_json::to_value(tools::get_litellm_web_search_tool()), "get_litellm_web_search_tool_openai" => serde_json::to_value(tools::get_litellm_web_search_tool_openai()), _ => serde_json::to_value(tools::get_litellm_web_search_tool_responses()) };
    value.map_err(|error| Box::new(crate::helpers::HelperError { class: "Serialization".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() }))
}

fn detection(function: &str, tool: &betterlite_websearch::tools::Tool) -> serde_json::Value {
    match function { "is_web_search_tool_responses" => serde_json::json!(betterlite_websearch::tools::is_web_search_tool_responses(tool)), "is_web_search_tool_chat_completion" => serde_json::json!(betterlite_websearch::tools::is_web_search_tool_chat_completion(tool)), _ => serde_json::json!(betterlite_websearch::tools::is_web_search_tool(tool)) }
}

fn outcome(function: &str, input: &Input) -> crate::helpers::HelperResult {
    let value = match function { "search_outcome_succeeded" => serde_json::json!(betterlite_websearch::records::SearchOutcome::Succeeded(betterlite_websearch::records::SearchSucceeded { text: input.text.clone().unwrap_or_default(), response: input.response.clone().map(serde_json::from_value).transpose().map_err(|error| crate::helpers::HelperError { class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() })? })), "search_outcome_failed" => serde_json::json!(betterlite_websearch::records::SearchOutcome::Failed(betterlite_websearch::records::SearchFailed { error_code: serde_json::from_value(input.error_code.clone().unwrap_or_default()).map_err(|error| crate::helpers::HelperError { class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() })?, message: input.message.clone().unwrap_or_default() })), _ => serde_json::json!(["invalid_tool_input", "unavailable", "max_uses_exceeded", "too_many_requests", "query_too_long", "request_too_large"]) };
    Ok(value)
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
