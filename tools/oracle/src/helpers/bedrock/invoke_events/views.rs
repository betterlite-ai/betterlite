use betterlite_anthropic::stream_events::chunk::StreamEventChunk;
use betterlite_bedrock::converse_events::ChunkProjection;
use betterlite_bedrock::converse_events::response::ConverseChunk;
use betterlite_bedrock::invoke_events::InvokeChunk;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use serde_json::{Value, json};
pub(super) struct ToolPart {
    pub(super) id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) arguments: Option<String>,
    pub(super) index: Option<i64>,
}
pub(super) struct StreamParts {
    pub(super) content: Option<String>,
    pub(super) reasoning_content: Option<String>,
    pub(super) finish_reason: Option<String>,
    pub(super) tool_calls: Option<Vec<ToolPart>>,
    pub(super) usage: Option<(i64, i64, i64)>,
}
pub(super) fn usage_value(usage: (i64, i64, i64)) -> Value {
    json!({
        "prompt_tokens": usage.0,
        "completion_tokens": usage.1,
        "total_tokens": usage.2,
    })
}
fn tool_value(tool: ToolPart) -> Value {
    json!({
        "id": tool.id,
        "name": tool.name,
        "arguments": tool.arguments,
        "index": tool.index,
    })
}
pub(super) fn stream_view(parts: StreamParts) -> Value {
    json!({
        "kind": "stream",
        "content": parts.content,
        "reasoning_content": parts.reasoning_content,
        "finish_reason": parts.finish_reason,
        "tool_calls": parts
            .tool_calls
            .map(|calls| calls.into_iter().map(tool_value).collect::<Vec<_>>()),
        "usage": parts.usage.map(usage_value),
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
            .map(|usage| usage_value((usage.prompt_tokens, usage.completion_tokens, usage.total_tokens))),
        "index": chunk.index,
    })
}
fn anthropic_parts(chunk: StreamEventChunk) -> StreamParts {
    let usage = chunk
        .usage
        .map(|usage| (usage.prompt_tokens, usage.completion_tokens, usage.total_tokens));
    match chunk.choices.into_iter().next() {
        Some(choice) => StreamParts {
            content: Some(choice.delta.content),
            reasoning_content: choice.delta.reasoning_content,
            finish_reason: choice.finish_reason,
            tool_calls: choice.delta.tool_calls.map(|calls| {
                calls
                    .into_iter()
                    .map(|call| ToolPart {
                        id: call.id,
                        name: call.function.name,
                        arguments: Some(call.function.arguments),
                        index: Some(call.index),
                    })
                    .collect()
            }),
            usage,
        },
        None => StreamParts {
            content: None,
            reasoning_content: None,
            finish_reason: None,
            tool_calls: None,
            usage,
        },
    }
}
fn converse_parts(chunk: ConverseChunk) -> StreamParts {
    StreamParts {
        content: chunk.delta.content,
        reasoning_content: chunk.delta.reasoning_content,
        finish_reason: chunk.finish_reason,
        tool_calls: chunk.delta.tool_calls.map(|calls| {
            calls
                .into_iter()
                .map(|call| ToolPart {
                    id: call.id,
                    name: call.function.name,
                    arguments: Some(call.function.arguments),
                    index: Some(call.index),
                })
                .collect()
        }),
        usage: chunk
            .usage
            .map(|usage| (usage.prompt_tokens, usage.completion_tokens, usage.total_tokens)),
    }
}
fn reply_parts(reply: StreamReply) -> StreamParts {
    let usage = reply.usage.and_then(reply_usage);
    match reply.choices.into_iter().next() {
        Some(choice) => StreamParts {
            content: choice.delta.content,
            reasoning_content: choice.delta.reasoning_content,
            finish_reason: choice.finish_reason,
            tool_calls: choice.delta.tool_calls.map(|calls| {
                calls
                    .into_iter()
                    .map(|call| {
                        let name = call
                            .function
                            .as_ref()
                            .and_then(|function| function.name.clone());
                        ToolPart {
                            id: call.id.flatten(),
                            name,
                            arguments: call.function.map(|function| function.arguments),
                            index: call.index,
                        }
                    })
                    .collect()
            }),
            usage,
        },
        None => StreamParts {
            content: None,
            reasoning_content: None,
            finish_reason: None,
            tool_calls: None,
            usage,
        },
    }
}
fn reply_usage(usage: Value) -> Option<(i64, i64, i64)> {
    let prompt = usage.get("prompt_tokens")?.as_i64()?;
    let completion = usage.get("completion_tokens")?.as_i64()?;
    let total = usage.get("total_tokens")?.as_i64()?;
    Some((prompt, completion, total))
}
pub(super) fn chunk_view(chunk: InvokeChunk) -> Value {
    match chunk {
        InvokeChunk::Base(projection) => match *projection {
            ChunkProjection::Generic(generic) => generic_view(generic),
            ChunkProjection::Converse(converse) => stream_view(converse_parts(converse)),
        },
        InvokeChunk::Anthropic(chunk) => stream_view(anthropic_parts(*chunk)),
        InvokeChunk::DeepSeek(reply) | InvokeChunk::OpenAiCompatible(reply) => {
            stream_view(reply_parts(*reply))
        }
    }
}
