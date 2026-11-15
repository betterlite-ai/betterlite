use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_input::{ResponsesInput, transform_responses_request};
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::ResponsesError;
use super::input::azure_input;

// LiteLLM's stripped model name drops these three routing prefixes and keeps "azure/" untouched
pub fn strip_model(model: &str) -> String {
    model
        .replace("responses/", "")
        .replace("o_series/", "")
        .replace("azure_ai/", "")
}

pub fn transform_request(
    model: &str,
    input: &ResponsesInput,
    options: &Object,
    model_info: Option<&Object>,
) -> Result<Object, ResponsesError> {
    let stripped = strip_model(model);
    let flattened = flattened_tools(options)?;
    let prepared = azure_input(input);
    Ok(transform_responses_request(
        &stripped,
        &prepared,
        &flattened,
        ProviderId::Azure,
        model_info,
    )?)
}

// Each tool that carries a nested function is copied with the nested keys laid over its own
fn flattened_tools(options: &Object) -> Result<Object, ResponsesError> {
    let mut flattened = options.clone();
    let tools = match flattened.get("tools") {
        Some(Json::List(tools)) => tools.clone(),
        Some(_) | None => return Ok(flattened),
    };
    let mut updated = Vec::with_capacity(tools.len());
    for tool in &tools {
        updated.push(flattened_tool(tool)?);
    }
    flattened.insert("tools".to_owned(), Json::List(updated));
    Ok(flattened)
}

fn flattened_tool(tool: &Json) -> Result<Json, ResponsesError> {
    let Json::Object(fields) = tool else {
        return Ok(tool.clone());
    };
    let mut fields = fields.clone();
    let Some(nested) = fields.shift_remove("function") else {
        return Ok(tool.clone());
    };
    let Json::Object(nested) = nested else {
        return Err(ResponsesError::InvalidToolFunction);
    };
    fields.extend(nested);
    Ok(Json::Object(fields))
}
