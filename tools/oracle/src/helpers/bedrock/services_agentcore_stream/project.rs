use betterlite_bedrock::agentcore_stream::AgentCoreStreamError;
use betterlite_bedrock::bedrock_settings::ErrorHeaders;
use betterlite_openai::stream::chunk::Chunk;
use serde_json::{Value, json};

pub(super) fn outcome(
    handler: &'static str,
    provider: Option<&'static str>,
    emitted: &Result<Vec<Chunk>, AgentCoreStreamError>,
) -> Value {
    match emitted {
        Ok(chunks) => json!({
            "handler": handler,
            "provider": provider,
            "failed": false,
            "chunks": chunks.iter().map(chunk_json).collect::<Vec<Value>>(),
            "error": Value::Null,
        }),
        Err(error) => json!({
            "handler": handler,
            "provider": Value::Null,
            "failed": true,
            "chunks": Vec::<Value>::new(),
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
