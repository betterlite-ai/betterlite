use betterlite_bedrock::agentcore_stream::AgentCoreStreamError;
use betterlite_bedrock::bedrock_settings::ErrorHeaders;
use betterlite_openai::stream::chunk::Chunk;
use serde_json::{Value, json};

use super::Scenario;
use super::fixture::{Fixture, LoggedCall, Recorded};

pub(super) fn outcome(
    fixture: &Fixture,
    handler: &'static str,
    provider: Option<&'static str>,
    emitted: &Result<Vec<Chunk>, AgentCoreStreamError>,
    scenario: &Scenario,
) -> Value {
    let post = fixture
        .posts()
        .first()
        .map_or(Value::Null, |post| post_json(post, scenario));
    let post_call = fixture
        .logger
        .calls()
        .first()
        .map_or(Value::Null, call_json);
    match emitted {
        Ok(chunks) => json!({
            "handler": handler,
            "provider": provider,
            "failed": false,
            "chunks": chunks.iter().map(chunk_json).collect::<Vec<Value>>(),
            "post": post,
            "post_call": post_call,
            "error": Value::Null,
        }),
        Err(error) => json!({
            "handler": handler,
            "provider": Value::Null,
            "failed": true,
            "chunks": Vec::<Value>::new(),
            "post": post,
            "post_call": post_call,
            "error": error_json(error),
        }),
    }
}

fn chunk_json(chunk: &Chunk) -> Value {
    let usage = chunk.usage.as_ref().map_or(Value::Null, |usage| {
        json!({
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        })
    });
    json!({
        "id": &chunk.id,
        "object": &chunk.object,
        "created": chunk.created,
        "model": &chunk.model,
        "choices": chunk.choices.iter().map(|choice| json!({
            "index": choice.index,
            "finish_reason": &choice.finish_reason,
            "delta": {
                "content": &choice.delta.content,
                "role": &choice.delta.role,
            },
        })).collect::<Vec<Value>>(),
        "usage": usage,
    })
}

fn post_json(post: &Recorded, scenario: &Scenario) -> Value {
    let body = match &scenario.signed_json_body {
        Some(bytes) if !bytes.is_empty() => json!({ "signed": &post.body }),
        Some(_) | None => json!({
            "json": serde_json::from_slice::<Value>(&post.body).unwrap_or(Value::Null),
        }),
    };
    let mut headers = post.headers.clone();
    headers.sort();
    json!({
        "url": &post.url,
        "headers": headers,
        "body": body,
        "stream": post.stream,
    })
}

fn call_json(call: &LoggedCall) -> Value {
    json!({
        "input": &call.input,
        "api_key": &call.api_key,
        "original_response": &call.original_response,
        "complete_input_dict": &call.complete_input_dict,
    })
}

fn error_json(error: &AgentCoreStreamError) -> Value {
    match error {
        AgentCoreStreamError::Provider { status, cause } => {
            let headers = match &cause.cause.headers {
                Some(ErrorHeaders::Http(pairs)) => {
                    let mut sorted = pairs.clone();
                    sorted.sort();
                    json!(sorted)
                }
                Some(ErrorHeaders::Mapping(_)) | None => Value::Null,
            };
            json!({
                "kind": "provider",
                "status_code": status,
                "headers": headers,
                "response_status": cause.cause.error.response.status_code,
            })
        }
        AgentCoreStreamError::Line { .. } | AgentCoreStreamError::Reply { .. } => {
            json!({ "kind": "invalid" })
        }
        AgentCoreStreamError::Transport { .. }
        | AgentCoreStreamError::Body { .. }
        | AgentCoreStreamError::Url { .. }
        | AgentCoreStreamError::Encode { .. }
        | AgentCoreStreamError::Header(_) => json!({ "kind": "other" }),
    }
}
