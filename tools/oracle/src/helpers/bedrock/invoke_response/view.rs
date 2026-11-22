use betterlite_bedrock::invoke_response::InvokeReply;
use serde_json::{Value, json};

// The four reply wires carry the same choice fields, so one mapping builds every choice list.
macro_rules! choices_json {
    ($choices:expr) => {
        json!($choices
            .iter()
            .map(|choice| json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "content": choice.message.content,
                "tool_calls": choice.message.tool_calls.as_ref().map(Vec::len),
            }))
            .collect::<Vec<Value>>())
    };
}

// The four usage wires carry the same three token fields.
macro_rules! usage_json {
    ($usage:expr) => {
        json!({
            "prompt_tokens": $usage.prompt_tokens,
            "completion_tokens": $usage.completion_tokens,
            "total_tokens": $usage.total_tokens,
        })
    };
}

pub(super) fn reply_json(reply: &InvokeReply) -> Value {
    match reply {
        InvokeReply::Chat(response) => json!({
            "id": response.id,
            "created": response.created,
            "model": response.model,
            "usage": usage_json!(response.usage),
            "choices": choices_json!(response.choices),
        }),
        InvokeReply::Anthropic(envelope) => json!({
            "id": envelope.id,
            "created": envelope.created,
            "model": envelope.model,
            "usage": envelope.usage.as_ref().map_or(Value::Null, |usage| usage_json!(usage)),
            "choices": choices_json!(envelope.choices),
        }),
        InvokeReply::Nova(response) => json!({
            "id": response.id,
            "created": response.created,
            "model": response.model,
            "usage": response.usage.as_ref().map_or(Value::Null, |usage| usage_json!(usage)),
            "choices": choices_json!(response.choices),
        }),
        InvokeReply::Pegasus(response) => json!({
            "id": response.id,
            "created": response.created,
            "model": response.model,
            "usage": usage_json!(response.usage),
            "choices": choices_json!(response.choices),
        }),
    }
}
