use betterlite_models::cost_usage::tool_projection::ToolUsage;
use betterlite_models::decisions::Json;
use betterlite_models::tool_signals::conversions::{
    extract_file_search_params, extract_token_counts, safe_convert_to_int,
};
use betterlite_models::tool_signals::counts::count_web_search_calls;
use betterlite_models::tool_signals::error::ToolSignalError;
use betterlite_models::tool_signals::kwargs::{get_file_search_tool_call, get_web_search_options};
use betterlite_models::tool_signals::response::{
    ToolBillingResponse, anthropic_search_usage, includes_file_search_call,
    includes_web_search_call,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) responses: Vec<ToolBillingResponse>,
    pub(crate) file_search_params: Vec<Json>,
    pub(crate) token_counts: Vec<Json>,
    pub(crate) values: Vec<Json>,
    pub(crate) kwargs: Vec<IndexMap<String, Json>>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let responses = input
            .responses
            .iter()
            .map(response_value)
            .collect::<Result<Vec<_>, _>>()?;
        let file_search_params = input
            .file_search_params
            .iter()
            .map(file_search_value)
            .collect::<Result<Vec<_>, _>>()?;
        let token_counts = input
            .token_counts
            .iter()
            .map(token_count_value)
            .collect::<Result<Vec<_>, _>>()?;
        let values = input
            .values
            .iter()
            .map(int_value)
            .collect::<Result<Vec<_>, _>>()?;
        let kwargs = input
            .kwargs
            .iter()
            .map(kwargs_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "responses": responses,
            "file_search_params": file_search_params,
            "token_counts": token_counts,
            "values": values,
            "kwargs": kwargs,
        }))
    }
}

fn response_value(response: &ToolBillingResponse) -> Result<Value, Box<HelperError>> {
    let count = count_web_search_calls(response).map_err(signal_error)?;
    let repaired = anthropic_search_usage(response);
    Ok(json!({
        "web_search": includes_web_search_call(response),
        "file_search": includes_file_search_call(response),
        "count": count,
        "usage": usage_value(repaired.as_ref()),
    }))
}

fn usage_value(supplied: Option<&ToolUsage>) -> Value {
    let Some(usage) = supplied else {
        return Value::Null;
    };
    json!({
        "prompt_tokens": usage.cost.prompt_tokens,
        "completion_tokens": usage.cost.completion_tokens,
        "total_tokens": usage.cost.total_tokens,
        "prompt_web_search_requests": usage
            .cost
            .prompt_details
            .and_then(|details| details.web_search_requests),
        "server_tool_use": usage
            .server_tool_use
            .and_then(|server_tool_use| server_tool_use.web_search_requests),
    })
}

fn file_search_value(value: &Json) -> Result<Value, Box<HelperError>> {
    let params = extract_file_search_params(Some(value)).map_err(signal_error)?;
    Ok(json!([
        params
            .storage_gb
            .map(|quantity| quantity.normalize().to_string()),
        params.days.map(|quantity| quantity.normalize().to_string()),
    ]))
}

fn token_count_value(value: &Json) -> Result<Value, Box<HelperError>> {
    extract_token_counts(Some(value))
        .map(|counts| json!([counts.input_tokens, counts.output_tokens]))
        .map_err(signal_error)
}

fn int_value(value: &Json) -> Result<Value, Box<HelperError>> {
    safe_convert_to_int(value)
        .map(|converted| json!(converted))
        .map_err(signal_error)
}

fn kwargs_value(kwargs: &IndexMap<String, Json>) -> Result<Value, Box<HelperError>> {
    let web = get_web_search_options(kwargs).map_err(signal_error)?;
    let file = get_file_search_tool_call(kwargs).map_err(signal_error)?;
    Ok(json!({
        "web_search_options": web.map(|options| options.fields),
        "file_search": file.map(|tool| tool.fields),
    }))
}

fn signal_error(error: ToolSignalError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ToolSignalError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;
