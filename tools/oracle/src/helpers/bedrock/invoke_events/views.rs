use betterlite_bedrock::converse_events::ChunkProjection;
use betterlite_bedrock::invoke_events::InvokeChunk;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use serde_json::{Value, json, to_value};
fn counts(prompt: i64, completion: i64, total: i64) -> Value {
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    })
}
// A usage object counts only when all three totals are integers, as LiteLLM's Usage view requires.
fn usage_value(usage: Option<&Value>) -> Value {
    let keys = ["prompt_tokens", "completion_tokens", "total_tokens"];
    match keys.map(|key| {
        usage
            .and_then(|source| source.get(key))
            .and_then(Value::as_i64)
    }) {
        [Some(prompt), Some(completion), Some(total)] => counts(prompt, completion, total),
        _ => Value::Null,
    }
}
fn tool_value(call: &Value) -> Value {
    let function = call.get("function");
    json!({
        "id": call.get("id"),
        "name": function.and_then(|source| source.get("name")),
        "arguments": function.and_then(|source| source.get("arguments")),
        "index": call.get("index"),
    })
}
pub(super) fn generic_view(chunk: GenericChunk) -> Value {
    json!({
        "kind": "generic",
        "text": chunk.text,
        "tool_use": chunk.tool_use,
        "is_finished": chunk.is_finished,
        "finish_reason": chunk.finish_reason,
        "usage": chunk
            .usage
            .map(|usage| counts(usage.prompt_tokens, usage.completion_tokens, usage.total_tokens)),
        "index": chunk.index,
    })
}
// Every other projection is a stream chunk: the choice holds the delta and the finish reason.
pub(super) fn chunk_view(chunk: InvokeChunk) -> Result<Value, serde_json::Error> {
    let (value, choice_path) = match chunk {
        InvokeChunk::Base(projection) => match *projection {
            ChunkProjection::Generic(generic) => return Ok(generic_view(generic)),
            ChunkProjection::Converse(converse) => (to_value(converse)?, ""),
        },
        InvokeChunk::Anthropic(parsed) => (to_value(*parsed)?, "/choices/0"),
        InvokeChunk::DeepSeek(reply) | InvokeChunk::OpenAiCompatible(reply) => {
            (to_value(*reply)?, "/choices/0")
        }
    };
    let choice = value.pointer(choice_path);
    let delta = choice.and_then(|source| source.get("delta"));
    Ok(json!({
        "kind": "stream",
        "content": delta.and_then(|source| source.get("content")),
        "role": delta.and_then(|source| source.get("role")),
        "reasoning_content": delta.and_then(|source| source.get("reasoning_content")),
        "finish_reason": choice.and_then(|source| source.get("finish_reason")),
        "tool_calls": delta
            .and_then(|source| source.get("tool_calls"))
            .and_then(Value::as_array)
            .map(|calls| calls.iter().map(tool_value).collect::<Vec<_>>()),
        "usage": usage_value(value.get("usage")),
    }))
}
