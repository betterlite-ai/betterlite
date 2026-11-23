use betterlite_bedrock::qwen_deepseek::reply::{Choice, Reply};
use serde_json::{Value, json};

pub(super) fn reply_view(reply: &Reply) -> Value {
    let choices = reply
        .choices
        .iter()
        .map(choice_view)
        .collect::<Vec<Value>>();
    json!({
        "id": reply.id,
        "object": reply.object,
        "created": reply.created,
        "model": reply.model,
        "usage": reply.usage.as_ref().map(|usage| json!({
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        })),
        "choices": choices,
    })
}

fn choice_view(choice: &Choice) -> Value {
    let fields = choice.message.provider_specific_fields.as_ref();
    let reasoning = fields.and_then(|fields| fields.get("reasoning_content"));
    json!({
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "content": choice.message.content,
        "reasoning_content": reasoning,
        "provider_specific_fields": fields,
    })
}
