use betterlite_bedrock::agentcore_stream::AgentCoreStreamError;
use betterlite_bedrock::bedrock_settings::ErrorHeaders;
use betterlite_openai::stream::chunk::Chunk;
use serde_json::{Value, json};

use super::fixture::Records;

pub(super) fn outcome(
    provider: Option<&'static str>,
    chunks: &[Chunk],
    failure: Option<&AgentCoreStreamError>,
    records: &Records,
) -> Value {
    json!({
        "provider": provider.filter(|_| failure.is_none()),
        "failed": failure.is_some(),
        "chunks": chunks.iter().map(chunk_json).collect::<Vec<Value>>(),
        "error": failure.map_or(Value::Null, error_json),
        "posts": &records.posts,
        "post_calls": &records.post_calls,
    })
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
            "delta": {"content": &choice.delta.content, "role": &choice.delta.role},
        })).collect::<Vec<Value>>(),
        "usage": usage,
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
        AgentCoreStreamError::Line(_) | AgentCoreStreamError::Reply(_) => {
            json!({ "kind": "invalid" })
        }
        AgentCoreStreamError::Transport(_)
        | AgentCoreStreamError::Body(_)
        | AgentCoreStreamError::Url(_)
        | AgentCoreStreamError::Encode(_)
        | AgentCoreStreamError::Header(_) => json!({ "kind": "other" }),
    }
}
