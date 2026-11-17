use betterlite_anthropic::response::wire::{ChatMessage, ResponseEnvelope};
use betterlite_anthropic::usage::AnthropicUsage;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;
use serde_json::{Value, json};

pub(super) fn reply(
    envelope: &ResponseEnvelope,
    details: &IndexMap<String, Json>,
    warned: bool,
) -> Value {
    let choice = envelope.choices.first();
    outcome_value(
        &json!({
            "kind": "reply",
            "id": envelope.id,
            "object": envelope.object,
            "created": envelope.created,
            "model": envelope.model,
            "finish_reason": choice.and_then(|chosen| chosen.finish_reason.clone()),
            "message": choice.map(|chosen| message_state(&chosen.message)),
            "usage": envelope.usage.as_ref().map(usage_state),
            "hidden": envelope.hidden_params,
        }),
        details,
        warned,
    )
}

pub(super) fn anthropic_error(
    status: u16,
    headers: &HeaderMap,
    details: &IndexMap<String, Json>,
    warned: bool,
) -> Value {
    outcome_value(
        &json!({
            "kind": "error",
            "status": status,
            "headers": headers_state(headers),
        }),
        details,
        warned,
    )
}

fn outcome_value(outcome: &Value, details: &IndexMap<String, Json>, warned: bool) -> Value {
    json!({
        "outcome": outcome,
        "logged": logged(details),
        "warned": warned,
    })
}

fn message_state(message: &ChatMessage) -> Value {
    json!({
        "role": message.role,
        "content": message.content,
        "tool_calls": message.tool_calls,
        "thinking_blocks": message.thinking_blocks,
        "reasoning_content": message.reasoning_content,
        "provider_specific_fields": message.provider_specific_fields,
    })
}

fn usage_state(usage: &AnthropicUsage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
        "cache_read_input_tokens": usage.cache_read_input_tokens,
        "reasoning_tokens": usage.completion_tokens_details.reasoning,
        "text_tokens": usage.completion_tokens_details.text,
        "speed": usage.speed,
    })
}

fn logged(details: &IndexMap<String, Json>) -> Value {
    let complete_input_dict = match details.get("additional_args") {
        Some(Json::Object(additional)) => additional
            .get("complete_input_dict")
            .cloned()
            .unwrap_or(Json::Null),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => Json::Null,
    };
    json!({
        "input": details.get("input").cloned().unwrap_or(Json::Null),
        "original_response": details.get("original_response").cloned().unwrap_or(Json::Null),
        "complete_input_dict": complete_input_dict,
    })
}

fn headers_state(headers: &HeaderMap) -> Json {
    let mut fields = IndexMap::new();
    for (name, value) in headers {
        // httpx decodes header bytes as latin-1
        let text = value.as_bytes().iter().copied().map(char::from).collect();
        fields.insert(name.as_str().to_owned(), Json::Text(text));
    }
    Json::Object(fields)
}
