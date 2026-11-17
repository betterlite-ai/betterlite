use std::collections::BTreeMap;

use betterlite_anthropic::batches::handler::BatchLogging;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::usage::AnthropicUsage;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_models::decisions::Json;
use http::header::HeaderMap;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::support::{AUTH_HEADERS, EXPLICIT_HEADERS, explicit_headers};

pub(crate) fn request_view(request: &OutboundRequest) -> Value {
    json!({
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "headers": explicit_headers(&request.headers),
        "auth_header": auth_header(&request.headers),
    })
}

fn auth_header(headers: &HeaderMap) -> Value {
    for name in AUTH_HEADERS {
        if headers.contains_key(name) {
            return json!(name);
        }
    }
    Value::Null
}

pub(crate) fn pre_call_view(details: &IndexMap<String, Json>) -> Value {
    let Some(Json::Object(additional)) = details.get("additional_args") else {
        return Value::Null;
    };
    let Some(Json::Text(input)) = details.get("input") else {
        return Value::Null;
    };
    let Some(Json::Object(headers)) = additional.get("headers") else {
        return Value::Null;
    };
    json!({
        "input": input,
        "api_base": additional.get("api_base").cloned().unwrap_or(Json::Null),
        "headers": recorded_headers(headers),
        "auth_header": recorded_auth_header(headers),
        "complete_input_dict": additional.get("complete_input_dict").cloned().unwrap_or(Json::Null),
    })
}

fn recorded_headers(headers: &IndexMap<String, Json>) -> Value {
    let mut fields = BTreeMap::new();
    for name in EXPLICIT_HEADERS {
        if let Some(value) = headers.get(name) {
            fields.insert(name.to_owned(), value.clone());
        }
    }
    json!(fields)
}

fn recorded_auth_header(headers: &IndexMap<String, Json>) -> Value {
    for name in AUTH_HEADERS {
        if headers.contains_key(name) {
            return json!(name);
        }
    }
    Value::Null
}

pub(crate) fn constructed_view(context: &BatchLogging) -> Value {
    json!({
        "model": context.model,
        "messages": context.messages,
        "stream": context.stream,
        "call_type": context.call_type,
        "start_time": context.start_time,
        "litellm_call_id": context.litellm_call_id,
        "function_id": context.function_id,
    })
}

pub(crate) fn response_view(envelope: &ResponseEnvelope) -> Value {
    let choice = envelope.choices.first();
    json!({
        "id": envelope.id,
        "object": envelope.object,
        "created": envelope.created,
        "model": envelope.model,
        "content": choice.and_then(|chosen| chosen.message.content.clone()),
        "finish_reason": choice.and_then(|chosen| chosen.finish_reason.clone()),
        "usage": envelope.usage.as_ref().map(usage_view),
    })
}

fn usage_view(usage: &AnthropicUsage) -> Value {
    let prompt = &usage.prompt_tokens_details;
    let completion = &usage.completion_tokens_details;
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_text_tokens": prompt.text,
        "cached_tokens": prompt.cached,
        "cache_creation_tokens": prompt.cache_creation,
        "cache_write_tokens": prompt.cache_write,
        "completion_text_tokens": completion.text,
        "reasoning_tokens": completion.reasoning,
        "accepted_prediction_tokens": completion.accepted_prediction,
    })
}

pub(crate) fn auth_of(headers: &HeaderMap, expected: Option<&str>) -> Value {
    for name in AUTH_HEADERS {
        if let Some(value) = headers.get(name) {
            let matches = value.to_str().ok() == expected;
            return json!({"name": name, "matches": matches});
        }
    }
    json!({"name": Value::Null, "matches": expected.is_none()})
}
