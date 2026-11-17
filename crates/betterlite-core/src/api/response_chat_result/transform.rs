use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{
    ResponseApiUsage, ResponseOutputItem, ResponsesApiResponse,
};
use serde_json::{Map, Value};
use tracing::warn;

use super::choices::output_choices;
use super::error::{ResponseChatResultError as Error, serialized};
use super::reasoning::{empty_incomplete_choice, finish_text};
use super::records::{ChatChoice, ModelResponse};
use crate::api::api_boundaries::ApiLimits;
use crate::api::response_chat_history::metadata::{SharedOutputMetadata, shared_output_metadata};
use crate::api::responses_sse_recovery::recover_logged_output;
use crate::api::responses_usage::{UsageInput, normalize_and_project};

pub struct ResponseSource<'source> {
    pub model: &'source str,
    pub response: &'source mut ResponsesApiResponse,
    pub hidden_params: &'source Map<String, Value>,
}

pub fn transform_response(
    source: ResponseSource<'_>,
    mut result: ModelResponse,
    logging: &LoggingState,
    limits: &ApiLimits,
) -> Result<ModelResponse, Error> {
    let ResponseSource {
        model,
        response,
        hidden_params,
    } = source;
    if let Some(error) = &response.error {
        return Err(Error::ResponseError {
            body: serialized(error)?.to_string(),
        });
    }
    recover_output(response, model, logging, limits)?;
    let mut choices = output_choices(&response.output)?;
    let shared = response_metadata(response)?;
    finish_choices(&mut choices, response, &shared)?;
    result.choices = choices;
    result.model = Some(model.to_owned());
    result.usage = Some(chat_usage(response.usage.as_ref())?);
    let response_id = response.id.clone();
    result.id = shared.upstream_response_id.unwrap_or(response_id);
    for (key, value) in shared.metadata {
        result.extra.insert(key, value);
    }
    if !hidden_params.is_empty() {
        merge_hidden(&mut result.hidden_params, hidden_params)?;
    }
    Ok(result)
}

// only an empty output is recovered, and the recovered items replace the response's own so conversion sees them
fn recover_output(
    response: &mut ResponsesApiResponse,
    model: &str,
    logging: &LoggingState,
    limits: &ApiLimits,
) -> Result<(), Error> {
    if !response.output.is_empty() {
        return Ok(());
    }
    let recorded =
        recover_logged_output(logging, limits).map_err(|source| Error::Recovery { source })?;
    if recorded.is_empty() {
        return Ok(());
    }
    response.output = recorded.into_iter().filter_map(recovered_item).collect();
    warn!("Recovered empty Responses API output from raw SSE for model={model}");
    Ok(())
}

fn recovered_item(value: Json) -> Option<ResponseOutputItem> {
    let Json::Object(fields) = value else {
        return None;
    };
    Some(ResponseOutputItem::Unknown(fields.into_iter().collect()))
}

fn finish_choices(
    choices: &mut Vec<ChatChoice>,
    response: &ResponsesApiResponse,
    shared: &SharedOutputMetadata,
) -> Result<(), Error> {
    let incomplete =
        response.status.as_deref() == Some("incomplete") || shared.incomplete_reason.is_some();
    if choices.is_empty() && !incomplete {
        return Err(Error::UnknownItems {
            items: serialized(&response.output)?.to_string(),
        });
    }
    if !incomplete {
        return Ok(());
    }
    if choices.is_empty() {
        let choice = empty_incomplete_choice(&response.output, shared.incomplete_finish)?;
        choices.push(choice);
        return Ok(());
    }
    let finish = finish_text(shared.incomplete_finish);
    for choice in choices.iter_mut() {
        choice.finish_reason = finish.to_owned();
    }
    Ok(())
}

fn response_metadata(response: &ResponsesApiResponse) -> Result<SharedOutputMetadata, Error> {
    let mut fields = Map::new();
    for (key, value) in &response.extra {
        fields.insert(key.clone(), serialized(value)?);
    }
    let mut payload = Map::new();
    if let Some(details) = &response.incomplete_details {
        payload.insert("incomplete_details".to_owned(), serialized(details)?);
    }
    Ok(shared_output_metadata(
        Some(&fields),
        Some(&response.id),
        &Value::Object(payload),
    ))
}

// Usage.model_dump() keeps its two declared detail fields, so an absent wrapper still prints as null
fn chat_usage(usage: Option<&ResponseApiUsage>) -> Result<Value, Error> {
    let mut projected =
        normalize_and_project(usage.map_or(UsageInput::Absent, UsageInput::Response))
            .map_err(|source| Error::Usage { source })?;
    if let Value::Object(fields) = &mut projected {
        fields
            .entry("prompt_tokens_details".to_owned())
            .or_insert(Value::Null);
        fields
            .entry("completion_tokens_details".to_owned())
            .or_insert(Value::Null);
    }
    Ok(projected)
}

fn merge_hidden(target: &mut Map<String, Value>, raw: &Map<String, Value>) -> Result<(), Error> {
    for (key, value) in raw {
        if key == "additional_headers"
            && let Some(existing) = target.get(key)
        {
            target.insert(key.clone(), merged_headers(value, existing)?);
            continue;
        }
        target.insert(key.clone(), value.clone());
    }
    Ok(())
}

// existing headers beat the raw response's headers, and the raw response only adds the keys it alone has
fn merged_headers(raw: &Value, existing: &Value) -> Result<Value, Error> {
    let (Value::Object(raw_fields), Value::Object(existing_fields)) = (raw, existing) else {
        return Err(Error::HiddenHeaders);
    };
    let mut merged = raw_fields.clone();
    for (key, value) in existing_fields {
        merged.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(merged))
}
