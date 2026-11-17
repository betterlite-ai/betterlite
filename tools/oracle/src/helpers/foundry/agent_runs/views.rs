use betterlite_foundry::agent_results::reply::{AgentReply, THREAD_ID_KEY};
use betterlite_foundry::agent_results::request::SSL_VERIFY_KEY;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::{Value, json};

// the same request the handler's _build_model_response answers, shaped as betterlite's AgentReply
pub(super) fn reply_view(reply: &AgentReply) -> Value {
    let choice = reply.choices.first();
    let message = choice.map(|picked| &picked.message);
    let thread = reply
        .hidden_params
        .as_ref()
        .and_then(|params| params.get(THREAD_ID_KEY));
    json!({
        "model": reply.model,
        "choices": [{
            "index": choice.map(|picked| picked.index),
            "finish_reason": choice.map(|picked| picked.finish_reason.clone()),
            "message": {
                "role": message.map(|picked| picked.role.clone()),
                "content": message.map(|picked| picked.content.clone()),
                "annotations": message.and_then(|picked| picked.annotations.clone()),
            },
        }],
        "hidden_params": {"thread_id": thread},
        "usage": reply.usage.map(|usage| json!({
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        })),
    })
}

pub(super) fn ssl_verify(litellm_params: &Object) -> Json {
    match litellm_params.get(SSL_VERIFY_KEY) {
        None | Some(Json::Null) => Json::Null,
        Some(value) => value.clone(),
    }
}
