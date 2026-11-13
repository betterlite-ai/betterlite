use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};
use betterlite_models::usage_sum::Usage;
use betterlite_vertex::gemini_response_parts::projection::CallMode;
use betterlite_vertex::gemini_stream::chunk::{Choice, StreamChunk};
use betterlite_vertex::gemini_stream::error::StreamError;
use betterlite_vertex::gemini_stream::{GeminiStreamDecoder, StreamOptions};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;
mod tests;
pub(crate) struct Op;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    fragments: Vec<String>,
    legacy_function: bool,
    headers: BTreeMap<String, String>,
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
        let ids = Box::new(|| Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef));
        let mut decoder = GeminiStreamDecoder::new(options, &headers, ids);
        let mut chunks = Vec::new();
        for fragment in &input.fragments {
            match decoder.push(fragment.as_bytes()) {
                Ok(chunk) => chunks.push(chunk.as_ref().map(state)),
                Err(error) => return Err(stream_error(error)),
            }
        }
        let mut drained = Vec::new();
        while let Some(chunk) = decoder.finish().map_err(stream_error)? {
            drained.push(state(&chunk));
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
    let (status, message) = match &error {
        StreamError::Embedded { status, message } => (Some(*status), message.clone()),
        other => (None, other.to_string()),
    };
    Box::new(HelperError {
        class: "VertexAIError".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
fn state(chunk: &StreamChunk) -> Value {
    json!({
        "id": &chunk.id,
        "model": &chunk.model,
        "choices": chunk.choices.iter().map(choice_state).collect::<Vec<Value>>(),
        "usage": chunk.usage.as_ref().map(usage_state),
        "service_tier": &chunk.service_tier,
    })
}
fn choice_state(choice: &Choice) -> Value {
    match choice {
        Choice::Projected(projected) => json!({
            "index": projected.index,
            "finish_reason": projected.finish_reason,
            "delta": serde_json::to_value(&projected.delta).unwrap_or(Value::Null),
            "logprobs": serde_json::to_value(&projected.logprobs).unwrap_or(Value::Null),
        }),
        Choice::EmptyDelta { index } => json!({
            "index": index,
            "finish_reason": Value::Null,
            "delta": {
                "role": Value::Null, "content": Value::Null, "reasoning_content": Value::Null,
                "tool_calls": Value::Null, "function_call": Value::Null, "images": Value::Null,
                "annotations": Value::Null, "provider_specific_fields": Value::Null,
            },
            "logprobs": Value::Null,
        }),
    }
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
