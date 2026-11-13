use std::collections::BTreeMap;

use betterlite_models::usage_sum::Usage;
use betterlite_vertex::gemini_response_parts::projection::CallMode;
use betterlite_vertex::gemini_response_parts::tools::{FunctionChunk, ToolCallChunk};
use betterlite_vertex::gemini_stream::chunk::{Choice, StreamChunk};
use betterlite_vertex::gemini_stream::error::StreamError;
use betterlite_vertex::gemini_stream::{GeminiStreamDecoder, IdSource, StreamOptions};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    fragments: Vec<String>,
    legacy_function: bool,
    headers: BTreeMap<String, String>,
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef)
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let headers = header_map(&input.headers)?;
        let options = StreamOptions {
            now: 1_700_000_000,
            call_mode: if input.legacy_function {
                CallMode::LegacyFunction
            } else {
                CallMode::Tools
            },
        };
        let mut decoder = GeminiStreamDecoder::new(options, &headers, Box::new(FixedIds));
        let mut chunks = Vec::new();
        for fragment in &input.fragments {
            match decoder.push(fragment.as_bytes()) {
                Ok(chunk) => chunks.push(chunk.as_ref().map(state)),
                Err(error) => return Err(stream_error(error)),
            }
        }
        let mut drained = Vec::new();
        loop {
            match decoder.finish() {
                Ok(Some(chunk)) => drained.push(state(&chunk)),
                Ok(None) => break,
                Err(error) => return Err(stream_error(error)),
            }
        }
        Ok(json!({ "chunks": chunks, "drained": drained }))
    }
}

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).or_value_error()?,
            HeaderValue::from_str(value).or_value_error()?,
        );
    }
    Ok(map)
}

fn stream_error(error: StreamError) -> Box<HelperError> {
    let status = match &error {
        StreamError::Embedded { status, .. } => Some(*status),
        StreamError::Utf8 { .. }
        | StreamError::IncompleteUtf8
        | StreamError::NotAnObject
        | StreamError::TooLarge { .. }
        | StreamError::Decode(_)
        | StreamError::Candidates(_)
        | StreamError::Usage(_)
        | StreamError::Close => None,
    };
    Box::new(HelperError {
        class: "VertexAIError".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn state(chunk: &StreamChunk) -> Value {
    json!({
        "id": &chunk.id,
        "model": &chunk.model,
        "choices": chunk.choices.iter().map(choice_state).collect::<Vec<Value>>(),
        "usage": chunk.usage.as_ref().map(usage_state),
        "service_tier": &chunk.service_tier,
        "metadata": {
            "grounding_metadata": &chunk.metadata.grounding_metadata,
            "url_context_metadata": &chunk.metadata.url_context_metadata,
            "safety_ratings": &chunk.metadata.safety_ratings,
            "safety_results": &chunk.metadata.safety_results,
            "citation_metadata": &chunk.metadata.citation_metadata,
        },
    })
}

fn choice_state(choice: &Choice) -> Value {
    match choice {
        Choice::Projected(projected) => json!({
            "index": projected.index,
            "finish_reason": projected.finish_reason,
            "delta": {
                "role": serde_json::to_value(projected.delta.role).unwrap_or(Value::Null),
                "content": &projected.delta.content,
                "reasoning_content": &projected.delta.reasoning_content,
                "tool_calls": tool_calls(projected.delta.tool_calls.as_deref()),
                "function_call": function_call(projected.delta.function_call.as_ref()),
                "images": serde_json::to_value(&projected.delta.images).unwrap_or(Value::Null),
                "annotations": serde_json::to_value(&projected.delta.annotations).unwrap_or(Value::Null),
                "provider_specific_fields": serde_json::to_value(&projected.delta.provider_specific_fields).unwrap_or(Value::Null),
            },
            "logprobs": serde_json::to_value(&projected.logprobs).unwrap_or(Value::Null),
        }),
        Choice::EmptyDelta { index } => json!({
            "index": index,
            "finish_reason": Value::Null,
            "delta": {
                "role": Value::Null,
                "content": Value::Null,
                "reasoning_content": Value::Null,
                "tool_calls": Value::Null,
                "function_call": Value::Null,
                "images": Value::Null,
                "annotations": Value::Null,
                "provider_specific_fields": Value::Null,
            },
            "logprobs": Value::Null,
        }),
    }
}

fn tool_calls(calls: Option<&[ToolCallChunk]>) -> Value {
    let Some(calls) = calls else {
        return Value::Null;
    };
    Value::Array(calls.iter().map(tool_call).collect())
}

fn tool_call(call: &ToolCallChunk) -> Value {
    json!({
        "id": &call.id,
        "type": call.kind,
        "index": call.index,
        "function": {
            "name": &call.function.name,
            "arguments": &call.function.arguments,
        },
        "provider_specific_fields": serde_json::to_value(&call.provider_specific_fields).unwrap_or(Value::Null),
    })
}

fn function_call(function: Option<&FunctionChunk>) -> Value {
    let Some(function) = function else {
        return Value::Null;
    };
    json!({ "name": &function.name, "arguments": &function.arguments })
}

fn usage_state(usage: &Usage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_details": usage.prompt_tokens_details.is_some(),
        "completion_details": usage.completion_tokens_details.is_some(),
        "web_search_requests": usage.prompt_tokens_details.as_ref().and_then(|details| details.web_search_requests),
    })
}
