use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{CompletionTokensDetails, PromptTokensDetails};
use betterlite_openai::api::responses_objects::{
    ResponseApiUsage, ResponseOutputItem, ResponsesApiResponse,
};
use serde_json::{Map, Value};
use tracing::warn;

use super::choices::{RawMessageMode, output_choices};
use super::error::{ResponseChatResultError as Error, serialized};
use super::reasoning::{empty_incomplete_choice, finish_text};
use super::records::{ChatChoice, ModelResponse};
use crate::api::api_boundaries::ApiLimits;
use crate::api::response_chat_history::metadata::{SharedOutputMetadata, shared_output_metadata};
use crate::api::responses_sse_recovery::recover_logged_output;
use crate::api::responses_usage::{Usage, UsageInput, normalize_usage};

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
    let mut choices = output_choices(&response.output, RawMessageMode::Convert)?;
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
        finish.clone_into(&mut choice.finish_reason);
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

// Usage.model_dump() prints every field the detail wrappers keep, so an absent wrapper is null
fn chat_usage(usage: Option<&ResponseApiUsage>) -> Result<Value, Error> {
    let Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details,
        completion_tokens_details,
        cost,
        extensions,
    } = normalize_usage(usage.map_or(UsageInput::Absent, UsageInput::Response))
        .map_err(|source| Error::Usage { source })?;
    let mut fields = extensions;
    fields.insert("prompt_tokens".to_owned(), Value::from(prompt_tokens));
    fields.insert(
        "completion_tokens".to_owned(),
        Value::from(completion_tokens),
    );
    fields.insert("total_tokens".to_owned(), Value::from(total_tokens));
    fields.insert(
        "prompt_tokens_details".to_owned(),
        prompt_tokens_details
            .as_ref()
            .map_or(Value::Null, prompt_details_value),
    );
    fields.insert(
        "completion_tokens_details".to_owned(),
        completion_tokens_details
            .as_ref()
            .map_or(Value::Null, completion_details_value),
    );
    if let Some(cost) = cost {
        // LiteLLM types cost as a Python float, so model_dump prints an integer cost as 2.0
        fields.insert("cost".to_owned(), Value::from(cost.as_f64()));
    }
    Ok(Value::Object(fields))
}

// the wrapper keeps five base counters, deletes every unset optional field, and keeps a nested cached_tokens_details
fn prompt_details_value(details: &PromptTokensDetails) -> Value {
    let mut fields = Map::new();
    for (key, counter) in [
        ("audio_tokens", details.audio_tokens),
        ("cached_tokens", details.cached_tokens),
        ("text_tokens", details.text_tokens),
        ("image_tokens", details.image_tokens),
        ("video_tokens", details.video_tokens),
    ] {
        fields.insert(key.to_owned(), counter.map_or(Value::Null, Value::from));
    }
    if let Some(cached) = &details.cached_tokens_details {
        let mut nested = Map::new();
        for (key, counter) in [
            ("text_tokens", cached.text_tokens),
            ("audio_tokens", cached.audio_tokens),
            ("image_tokens", cached.image_tokens),
        ] {
            nested.insert(key.to_owned(), counter.map_or(Value::Null, Value::from));
        }
        fields.insert("cached_tokens_details".to_owned(), Value::Object(nested));
    }
    for (key, counter) in [
        ("cache_write_tokens", details.cache_write_tokens),
        ("cache_creation_tokens", details.cache_creation_tokens),
        ("web_search_requests", details.web_search_requests),
        (
            "google_maps_grounding_requests",
            details.google_maps_grounding_requests,
        ),
    ] {
        if let Some(counter) = counter {
            fields.insert(key.to_owned(), Value::from(counter));
        }
    }
    Value::Object(fields)
}

// the completion wrapper keeps seven fields, and only the four the conversion passes can be set
fn completion_details_value(details: &CompletionTokensDetails) -> Value {
    let mut fields = Map::new();
    for (key, counter) in [
        ("accepted_prediction_tokens", None),
        ("audio_tokens", details.audio_tokens),
        ("reasoning_tokens", details.reasoning_tokens),
        ("rejected_prediction_tokens", None),
        ("text_tokens", details.text_tokens),
        ("image_tokens", details.image_tokens),
        ("video_tokens", None),
    ] {
        fields.insert(key.to_owned(), counter.map_or(Value::Null, Value::from));
    }
    Value::Object(fields)
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
