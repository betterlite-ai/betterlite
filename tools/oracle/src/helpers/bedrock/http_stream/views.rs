use betterlite_bedrock::bedrock_http_stream::error::HttpStreamError;
use betterlite_bedrock::converse_events::ChunkProjection;
use betterlite_bedrock::invoke_events::InvokeChunk;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use super::types::Case;

pub(super) fn error_view(error: &HttpStreamError, case: &Case) -> Value {
    let HttpStreamError::Provider { status, error } = error else {
        return raised_view(raised_category(error));
    };
    let mut headers = error.response_headers.clone();
    headers.sort();
    json!({
        "kind": "provider",
        "status": status,
        "message": if case.compare.message { json!(error.error.message) } else { Value::Null },
        "headers": if case.compare.headers { json!(headers) } else { Value::Null },
        "category": Value::Null,
    })
}

// Converse passes a transport failure through untouched, so only its timeout category is comparable.
const fn raised_category(error: &HttpStreamError) -> &'static str {
    match error {
        HttpStreamError::Transport(
            TransportError::ConnectTimeout(_)
            | TransportError::Deadline(_)
            | TransportError::Rejected(HttpError::Timeout),
        ) => "timeout",
        HttpStreamError::Transport(_)
        | HttpStreamError::Url(_)
        | HttpStreamError::Body(_)
        | HttpStreamError::Transform(_)
        | HttpStreamError::Header(_)
        | HttpStreamError::Provider { .. } => "other",
    }
}

fn raised_view(category: &str) -> Value {
    json!({
        "kind": "raised",
        "status": Value::Null,
        "message": Value::Null,
        "headers": Value::Null,
        "category": category,
    })
}

pub(super) fn chunk_view(chunk: InvokeChunk) -> Value {
    match chunk {
        InvokeChunk::Base(projection) => match *projection {
            ChunkProjection::Generic(generic) => json!({
                "kind": "generic",
                "text": generic.text,
                "tool_use": generic.tool_use,
                "is_finished": generic.is_finished,
                "finish_reason": generic.finish_reason,
                "usage": generic.usage.map_or(Value::Null, |usage| {
                    counts(usage.prompt_tokens, usage.completion_tokens, usage.total_tokens)
                }),
                "index": generic.index,
            }),
            ChunkProjection::Converse(converse) => json!({
                "kind": "stream",
                "model": converse.model,
                "content": converse.delta.content,
                "role": converse.delta.role,
                "finish_reason": converse.finish_reason,
                "usage": converse.usage.map_or(Value::Null, |usage| {
                    counts(usage.prompt_tokens, usage.completion_tokens, usage.total_tokens)
                }),
            }),
        },
        // Provider-specific chunks belong to their own decoders' oracle cases; every case here uses the base one.
        InvokeChunk::Anthropic(_) | InvokeChunk::DeepSeek(_) | InvokeChunk::OpenAiCompatible(_) => {
            Value::Null
        }
    }
}

fn counts(prompt: i64, completion: i64, total: i64) -> Value {
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    })
}

pub(super) fn log_view(details: &Json) -> Value {
    let fields = match details {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Value::Null;
        }
    };
    let input = fields.get("input").cloned().unwrap_or(Json::Null);
    let original = fields
        .get("original_response")
        .cloned()
        .unwrap_or(Json::Null);
    let data = fields
        .get("additional_args")
        .and_then(|args| match args {
            Json::Object(inner) => inner.get("complete_input_dict").cloned(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .unwrap_or(Json::Null);
    json!({
        "input": input,
        "original_response": original,
        "complete_input_dict": data,
    })
}
