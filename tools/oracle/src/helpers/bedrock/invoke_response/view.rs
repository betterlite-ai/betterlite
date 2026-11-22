use betterlite_bedrock::invoke_response::InvokeReply;
use serde::Serialize;
use serde_json::{Value, json};

pub(super) fn reply_json(reply: &InvokeReply) -> Value {
    let (id, created, model, usage, choices) = match reply {
        InvokeReply::Chat(response) => (
            response.id.as_str(),
            json!(response.created),
            json!(response.model),
            usage_json(
                response.usage.prompt_tokens,
                response.usage.completion_tokens,
                response.usage.total_tokens,
            ),
            response
                .choices
                .iter()
                .map(|choice| {
                    json!({
                        "index": choice.index,
                        "finish_reason": choice.finish_reason,
                        "content": choice.message.content,
                        "tool_calls": choice.message.tool_calls.as_ref().map(Vec::len),
                    })
                })
                .collect(),
        ),
        InvokeReply::Anthropic(envelope) => (
            envelope.id.as_str(),
            json!(envelope.created),
            json!(envelope.model),
            match &envelope.usage {
                Some(usage) => usage_json(
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens,
                ),
                None => Value::Null,
            },
            envelope
                .choices
                .iter()
                .map(|choice| {
                    json!({
                        "index": choice.index,
                        "finish_reason": choice.finish_reason,
                        "content": choice.message.content,
                        "tool_calls": choice.message.tool_calls.as_ref().map(Vec::len),
                    })
                })
                .collect(),
        ),
        InvokeReply::Nova(response) => (
            response.id.as_str(),
            json!(response.created),
            json!(response.model),
            match &response.usage {
                Some(usage) => usage_json(
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens,
                ),
                None => Value::Null,
            },
            response
                .choices
                .iter()
                .map(|choice| {
                    json!({
                        "index": choice.index,
                        "finish_reason": choice.finish_reason,
                        "content": choice.message.content,
                        "tool_calls": choice.message.tool_calls.as_ref().map(Vec::len),
                    })
                })
                .collect(),
        ),
        InvokeReply::Pegasus(response) => (
            response.id.as_str(),
            json!(response.created),
            json!(response.model),
            usage_json(
                response.usage.prompt_tokens,
                response.usage.completion_tokens,
                response.usage.total_tokens,
            ),
            response
                .choices
                .iter()
                .map(|choice| {
                    json!({
                        "index": choice.index,
                        "finish_reason": choice.finish_reason,
                        "content": choice.message.content,
                        "tool_calls": choice.message.tool_calls.as_ref().map(Vec::len),
                    })
                })
                .collect(),
        ),
    };
    json!({
        "id": id,
        "created": created,
        "model": model,
        "usage": usage,
        "choices": choices,
    })
}

fn usage_json<P, C, T>(prompt: P, completion: C, total: T) -> Value
where
    P: Serialize,
    C: Serialize,
    T: Serialize,
{
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    })
}
