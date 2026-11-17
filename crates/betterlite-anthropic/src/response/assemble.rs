use std::error::Error as StdError;

use betterlite_logging::python_json;
use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::headers::response::process_anthropic_headers;
use crate::response_content::wire::{AnthropicCompletionResponse, AnthropicToolCall, JsonFields};
use crate::response_content::{AnthropicContentReply, project_response};
use crate::usage::{AnthropicUsage, ReasoningTokenCounter, calculate_usage};

use super::error::ResponseError;
use super::json_mode::{Selection, select};
use super::params::{apply_reverse_map, reverse_map_of, speed_of};
use super::wire::{AnthropicReply, ChatMessage, ResponseEnvelope};
use super::{RawResponse, ResponseRequest, get_error_class, get_prefix_prompt};

pub(super) fn transform_parsed_response<Counter>(
    envelope: &mut ResponseEnvelope,
    raw: &RawResponse<'_>,
    request: &ResponseRequest<'_>,
    reply: AnthropicReply,
    counter: &Counter,
) -> Result<(), ResponseError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let prefix = get_prefix_prompt(request.messages)?;
    let speed = speed_of(request.optional_params)?;
    // B570 prepares the additional headers before LiteLLM looks at the body's error key
    let additional_headers = headers_of(process_anthropic_headers(raw.headers));
    let mut body = body_or_error(reply, raw)?;
    let parsed = project_response(&body, None)?;
    let completion = completion_response(&body);
    let message = build_message(&mut body, &parsed, request, prefix.as_deref())?;
    let choice = envelope
        .choices
        .first_mut()
        .ok_or(ResponseError::MissingChoice)?;
    choice.message = message;
    choice.finish_reason = Some(finish_reason(&body.fields)?);
    envelope.usage = Some(usage_of(&body, &parsed, &completion, speed, counter)?);
    envelope.created = request.now;
    envelope.model = model_of(&body.fields)?;
    envelope.hidden_params = hidden_params(additional_headers, parsed.provider_specific_fields);
    Ok(())
}

fn body_or_error<CounterFailure>(
    reply: AnthropicReply,
    raw: &RawResponse<'_>,
) -> Result<AnthropicCompletionResponse, ResponseError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    match reply {
        AnthropicReply::Success(body) => Ok(body),
        AnthropicReply::Error(error) => Err(get_error_class(
            python_json::dumps(&error),
            raw.status,
            raw.headers.clone(),
        )),
    }
}

fn headers_of(headers: IndexMap<String, String>) -> JsonFields {
    headers
        .into_iter()
        .map(|(name, value)| (name, Json::Text(value)))
        .collect()
}

fn hidden_params(
    additional_headers: JsonFields,
    provider_specific_fields: JsonFields,
) -> JsonFields {
    IndexMap::from([
        (
            "additional_headers".to_owned(),
            Json::Object(additional_headers),
        ),
        (
            "provider_specific_fields".to_owned(),
            Json::Object(provider_specific_fields),
        ),
    ])
}

fn build_message<CounterFailure>(
    body: &mut AnthropicCompletionResponse,
    parsed: &AnthropicContentReply,
    request: &ResponseRequest<'_>,
    prefix: Option<&str>,
) -> Result<ChatMessage, ResponseError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let reverse = reverse_map_of(request.litellm_params)?;
    let mut calls = parsed.tool_calls.clone();
    if let Some(renames) = reverse.as_ref() {
        apply_reverse_map(&mut calls, renames);
    }
    let selection = select(body, request.json_mode, &calls, reverse.as_ref())?;
    if selection.message.is_some() {
        // LiteLLM overwrites the required stop_reason before mapping it, so a missing one cannot fail there
        body.fields
            .insert("stop_reason".to_owned(), Json::Text("stop".to_owned()));
    }
    let Selection {
        message,
        tool_calls,
        extra,
    } = selection;
    Ok(message.map_or_else(
        || {
            composed_message(
                parsed,
                prefix,
                request.disable_add_prefix_to_prompt,
                extra.as_deref(),
                &tool_calls,
            )
        },
        // the JSON-mode message LiteLLM builds from the tool arguments carries no provider fields or thinking
        |content| ChatMessage {
            role: "assistant".to_owned(),
            content: Some(content),
            tool_calls: None,
            thinking_blocks: None,
            reasoning_content: None,
            provider_specific_fields: None,
        },
    ))
}

fn composed_message(
    parsed: &AnthropicContentReply,
    prefix: Option<&str>,
    disabled: bool,
    extra: Option<&str>,
    tool_calls: &[AnthropicToolCall],
) -> ChatMessage {
    let mut text = parsed.content.clone();
    if let Some(requested) = prefix
        && !text.starts_with(requested)
        && !disabled
    {
        text = format!("{requested}{text}");
    }
    if let Some(tail) = extra {
        text.push_str(tail);
    }
    ChatMessage {
        role: "assistant".to_owned(),
        content: (!text.is_empty()).then_some(text),
        tool_calls: (!tool_calls.is_empty()).then(|| tool_calls.to_vec()),
        thinking_blocks: parsed.thinking_blocks.clone(),
        reasoning_content: parsed.reasoning_content.clone(),
        provider_specific_fields: Some(parsed.provider_specific_fields.clone()),
    }
}

fn finish_reason<CounterFailure>(
    fields: &JsonFields,
) -> Result<String, ResponseError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let Some(value) = fields.get("stop_reason") else {
        return Err(ResponseError::MissingField("stop_reason"));
    };
    if let Json::Text(raw) = value {
        return Ok(mapped_finish_reason(raw));
    }
    if matches!(value, Json::List(_) | Json::Object(_)) {
        return Err(ResponseError::FieldType {
            field: "stop_reason",
        });
    }
    // LiteLLM looks the value up, misses, warns with it and falls back to stop
    tracing::warn!(
        "Unmapped finish_reason '{}', defaulting to 'stop'",
        python_json::dumps(value)
    );
    Ok("stop".to_owned())
}

fn mapped_finish_reason(raw: &str) -> String {
    let mapping = map_finish_reason(raw);
    if !mapping.recognized {
        tracing::warn!("Unmapped finish_reason '{raw}', defaulting to 'stop'");
    }
    mapping.reason.as_str().to_owned()
}

fn model_of<CounterFailure>(
    fields: &JsonFields,
) -> Result<Option<String>, ResponseError<CounterFailure>>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    match fields.get("model") {
        Some(Json::Text(model)) => Ok(Some(model.clone())),
        Some(Json::Null) => Ok(None),
        Some(_) => Err(ResponseError::FieldType { field: "model" }),
        None => Err(ResponseError::MissingField("model")),
    }
}

// LiteLLM passes the parsed body to calculate_usage, so the content blocks keep their provider-written names
fn completion_response(body: &AnthropicCompletionResponse) -> Json {
    let mut fields = body.fields.clone();
    fields.insert("content".to_owned(), Json::List(body.content.clone()));
    Json::Object(fields)
}

fn usage_of<Counter>(
    body: &AnthropicCompletionResponse,
    parsed: &AnthropicContentReply,
    completion: &Json,
    speed: Option<&str>,
    counter: &Counter,
) -> Result<AnthropicUsage, ResponseError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let usage = body
        .fields
        .get("usage")
        .ok_or(ResponseError::MissingField("usage"))?;
    Ok(calculate_usage(
        usage,
        parsed.reasoning_content.as_deref(),
        Some(completion),
        speed,
        counter,
    )?)
}
